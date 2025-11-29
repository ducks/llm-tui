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
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaTool {
    #[serde(rename = "type")]
    pub tool_type: String, // "function"
    pub function: OllamaFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: serde_json::Value,
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
    role: String,
    content: String,
    tool_calls: Option<Vec<ToolCall>>,
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

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    models: Vec<OllamaModel>,
}

pub enum LlmEvent {
    Token(String),
    ToolUse {
        name: String,
        arguments: serde_json::Value,
    },
    Done,
    Error(String),
}

pub struct OllamaClient {
    base_url: String,
    client: Client,
    process: Option<Child>,
}

impl OllamaClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
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

        // Wait for server to be ready
        for _ in 0..30 {
            thread::sleep(Duration::from_millis(100));
            if self.is_running() {
                return Ok(());
            }
        }

        Err(anyhow::anyhow!("Ollama server failed to start"))
    }

    pub fn list_models(&self) -> Result<Vec<OllamaModel>> {
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

            // Check for HTTP errors
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

    pub fn chat(&self, model: &str, messages: Vec<ChatMessage>) -> Result<Receiver<LlmEvent>> {
        self.chat_with_tools(model, messages, None)
    }

    pub fn chat_with_tools(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<OllamaTool>>,
    ) -> Result<Receiver<LlmEvent>> {
        let (tx, rx) = channel();
        let client = self.client.clone();
        let url = format!("{}/api/chat", self.base_url);
        let request = ChatRequest {
            model: model.to_string(),
            messages,
            stream: true,
            tools,
        };

        thread::spawn(move || {
            let response = match client
                .post(&url)
                .json(&request)
                .timeout(Duration::from_secs(300)) // 5 minute timeout for LLM responses
                .send()
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(LlmEvent::Error(format!("Request failed: {}", e)));
                    return;
                }
            };

            let reader = BufReader::new(response);
            for line in reader.lines() {
                if let Ok(line) = line {
                    crate::debug_log!("DEBUG OLLAMA: Raw line: {}", line);
                    match serde_json::from_str::<ChatResponse>(&line) {
                        Ok(response) => {
                            crate::debug_log!("DEBUG OLLAMA: Parsed response - done: {}, message: {:?}", response.done, response.message);

                            // Process message first (can have tool_calls even when done=true)
                            if let Some(message) = response.message {
                                // Check for tool calls first
                                if let Some(tool_calls) = message.tool_calls {
                                    crate::debug_log!("DEBUG OLLAMA: Found {} tool calls", tool_calls.len());
                                    for tool_call in tool_calls {
                                        if tx
                                            .send(LlmEvent::ToolUse {
                                                name: tool_call.function.name,
                                                arguments: tool_call.function.arguments,
                                            })
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                }
                                // Then send content if any
                                if !message.content.is_empty() {
                                    if tx.send(LlmEvent::Token(message.content)).is_err() {
                                        break;
                                    }
                                }
                            }

                            // Check done after processing message
                            if response.done {
                                let _ = tx.send(LlmEvent::Done);
                                break;
                            }
                        }
                        Err(e) => {
                            crate::debug_log!("DEBUG OLLAMA: Parse error: {}", e);
                            let _ = tx.send(LlmEvent::Error(format!("Parse error: {}", e)));
                            break;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

/// Convert Claude tool definitions to Ollama format
pub fn claude_tools_to_ollama(claude_tools: Vec<crate::claude::Tool>) -> Vec<OllamaTool> {
    claude_tools
        .into_iter()
        .map(|t| OllamaTool {
            tool_type: "function".to_string(),
            function: OllamaFunction {
                name: t.name,
                description: t.description,
                parameters: t.input_schema,
            },
        })
        .collect()
}

impl Drop for OllamaClient {
    fn drop(&mut self) {
        // Don't kill the Ollama server on drop - let it keep running
        // for other applications
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
        }
    }
}
