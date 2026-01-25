//! Ollama provider implementation

use super::{LlmEvent, LlmProvider, ModelInfo, ProviderMessage, ToolDef, ToolResult};
use anyhow::Result;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub modified_at: String,
    pub size: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OllamaFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolCall {
    function: FunctionCall,
}

#[derive(Debug, Clone, Deserialize)]
struct FunctionCall {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OllamaTool>>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: Option<MessageWithTools>,
    done: bool,
}

#[derive(Debug, Deserialize)]
struct MessageWithTools {
    #[allow(dead_code)]
    role: String,
    content: String,
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Serialize)]
struct PullRequest {
    name: String,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct PullResponse {
    status: String,
    #[serde(default)]
    completed: Option<i64>,
    #[serde(default)]
    total: Option<i64>,
}

pub struct OllamaProvider {
    base_url: String,
    client: Client,
    process: Option<Child>,
}

impl OllamaProvider {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            client: Client::new(),
            process: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.client
            .get(&format!("{}/api/tags", self.base_url))
            .timeout(Duration::from_secs(2))
            .send()
            .is_ok()
    }

    pub fn start_server(&mut self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }

        let child = Command::new("ollama")
            .arg("serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        self.process = Some(child);

        for _ in 0..30 {
            thread::sleep(Duration::from_millis(100));
            if self.is_running() {
                return Ok(());
            }
        }

        Err(anyhow::anyhow!("Ollama server failed to start"))
    }

    pub fn list_ollama_models(&self) -> Result<Vec<OllamaModel>> {
        let response: ModelsResponse = self
            .client
            .get(&format!("{}/api/tags", self.base_url))
            .send()?
            .json()?;
        Ok(response.models)
    }

    pub fn pull_model(&self, name: &str) -> Result<Receiver<String>> {
        let (tx, rx) = channel();
        let client = self.client.clone();
        let url = format!("{}/api/pull", self.base_url);
        let name = name.to_string();

        thread::spawn(move || {
            let request = PullRequest {
                name,
                stream: true,
            };

            let response = match client.post(&url).json(&request).send() {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(format!("Error: {}", e));
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().unwrap_or_else(|_| "Unknown error".to_string());
                let _ = tx.send(format!("Error {}: {}", status, error_text));
                return;
            }

            let reader = BufReader::new(response);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if let Ok(response) = serde_json::from_str::<PullResponse>(&line) {
                        let status = if let (Some(completed), Some(total)) =
                            (response.completed, response.total)
                        {
                            format!(
                                "{}: {:.1}%",
                                response.status,
                                (completed as f64 / total as f64) * 100.0
                            )
                        } else {
                            response.status
                        };
                        if tx.send(status).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    pub fn delete_model(&self, name: &str) -> Result<()> {
        #[derive(Serialize)]
        struct DeleteRequest {
            name: String,
        }

        self.client
            .delete(&format!("{}/api/delete", self.base_url))
            .json(&DeleteRequest {
                name: name.to_string(),
            })
            .send()?;
        Ok(())
    }

    pub fn browse_library(&self) -> Result<Vec<OllamaModel>> {
        #[derive(Deserialize)]
        struct LibraryResponse {
            models: Vec<OllamaModel>,
        }

        let response: LibraryResponse = self
            .client
            .get("https://ollama.com/api/tags")
            .timeout(Duration::from_secs(10))
            .send()?
            .json()?;

        Ok(response.models)
    }

    pub fn unload_model(&self, model: &str) -> Result<()> {
        #[derive(Serialize)]
        struct GenerateRequest {
            model: String,
            keep_alive: i32,
        }

        let _ = self
            .client
            .post(&format!("{}/api/generate", self.base_url))
            .json(&GenerateRequest {
                model: model.to_string(),
                keep_alive: 0,
            })
            .send();

        Ok(())
    }

    fn convert_messages(messages: Vec<ProviderMessage>) -> Vec<ChatMessage> {
        messages
            .into_iter()
            .map(|m| ChatMessage {
                role: m.role,
                content: m.content,
            })
            .collect()
    }

    fn convert_tools(tools: Option<Vec<ToolDef>>) -> Option<Vec<OllamaTool>> {
        tools.map(|ts| {
            ts.into_iter()
                .map(|t| OllamaTool {
                    tool_type: "function".to_string(),
                    function: OllamaFunction {
                        name: t.name,
                        description: t.description,
                        parameters: t.input_schema,
                    },
                })
                .collect()
        })
    }

    fn stream_chat(
        client: Client,
        url: String,
        request: ChatRequest,
        tx: Sender<LlmEvent>,
    ) {
        let response = match client
            .post(&url)
            .json(&request)
            .timeout(Duration::from_secs(300))
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(LlmEvent::Error(format!("Request failed: {}", e)));
                return;
            }
        };

        let reader = BufReader::new(response);
        let mut tool_id_counter = 0;

        for line in reader.lines() {
            if let Ok(line) = line {
                match serde_json::from_str::<ChatResponse>(&line) {
                    Ok(response) => {
                        if let Some(message) = response.message {
                            if let Some(tool_calls) = message.tool_calls {
                                for tool_call in tool_calls {
                                    tool_id_counter += 1;
                                    if tx
                                        .send(LlmEvent::ToolUse {
                                            id: format!("ollama-tool-{}", tool_id_counter),
                                            name: tool_call.function.name,
                                            input: tool_call.function.arguments,
                                        })
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                            }
                            if !message.content.is_empty() {
                                if tx.send(LlmEvent::Text(message.content)).is_err() {
                                    break;
                                }
                            }
                        }

                        if response.done {
                            let _ = tx.send(LlmEvent::Done {
                                input_tokens: None,
                                output_tokens: None,
                            });
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(LlmEvent::Error(format!("Parse error: {}", e)));
                        break;
                    }
                }
            }
        }
    }
}

impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn is_available(&self) -> bool {
        self.is_running()
    }

    fn chat(
        &self,
        model: &str,
        messages: Vec<ProviderMessage>,
        tools: Option<Vec<ToolDef>>,
        _max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>> {
        let (tx, rx) = channel();
        let client = self.client.clone();
        let url = format!("{}/api/chat", self.base_url);

        let request = ChatRequest {
            model: model.to_string(),
            messages: Self::convert_messages(messages),
            stream: true,
            tools: Self::convert_tools(tools),
        };

        thread::spawn(move || {
            Self::stream_chat(client, url, request, tx);
        });

        Ok(rx)
    }

    fn continue_with_tools(
        &self,
        model: &str,
        mut messages: Vec<ProviderMessage>,
        tools: Option<Vec<ToolDef>>,
        tool_results: Vec<ToolResult>,
        max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>> {
        // Add tool results as a user message
        let results_text: Vec<String> = tool_results
            .into_iter()
            .map(|r| format!("[Tool result for {}]:\n{}", r.tool_use_id, r.content))
            .collect();

        messages.push(ProviderMessage {
            role: "user".to_string(),
            content: results_text.join("\n\n"),
        });

        self.chat(model, messages, tools, max_tokens)
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let models = self.list_ollama_models()?;
        Ok(models
            .into_iter()
            .map(|m| ModelInfo {
                id: m.name.clone(),
                name: m.name,
                provider: "ollama".to_string(),
            })
            .collect())
    }
}

impl Drop for OllamaProvider {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
        }
    }
}
