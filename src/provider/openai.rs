//! OpenAI provider implementation (basic HTTP streaming)

use super::{LlmEvent, LlmProvider, ModelInfo, ProviderMessage, ToolDef, ToolResult};
use anyhow::Result;
use serde::Serialize;
use std::io::BufRead;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

#[derive(Debug, Serialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    max_tokens: u32,
    stream: bool,
}

pub struct OpenAIProvider {
    api_key: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    fn convert_messages(messages: Vec<ProviderMessage>) -> Vec<OpenAIMessage> {
        messages
            .into_iter()
            .map(|m| OpenAIMessage {
                role: m.role,
                content: m.content,
            })
            .collect()
    }
}

impl LlmProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    fn chat(
        &self,
        model: &str,
        messages: Vec<ProviderMessage>,
        _tools: Option<Vec<ToolDef>>,
        max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>> {
        let (tx, rx) = channel();
        let api_key = self.api_key.clone();
        let model = model.to_string();

        thread::spawn(move || {
            let converted_messages = Self::convert_messages(messages);

            let request = OpenAIRequest {
                model,
                messages: converted_messages,
                max_tokens,
                stream: true,
            };

            let client = reqwest::blocking::Client::new();

            match client
                .post("https://api.openai.com/v1/chat/completions")
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
            {
                Ok(response) => {
                    if !response.status().is_success() {
                        let error_text = response
                            .text()
                            .unwrap_or_else(|_| "Unknown error".to_string());
                        let _ = tx.send(LlmEvent::Error(format!("API error: {}", error_text)));
                        return;
                    }

                    let reader = std::io::BufReader::new(response);

                    for line in reader.lines().map_while(Result::ok) {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                break;
                            }

                            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(data) {
                                if let Some(choices) = json_val["choices"].as_array() {
                                    for choice in choices {
                                        if let Some(content) = choice["delta"]["content"].as_str() {
                                            let _ = tx.send(LlmEvent::Text(content.to_string()));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let _ = tx.send(LlmEvent::Done {
                        input_tokens: None,
                        output_tokens: None,
                    });
                }
                Err(e) => {
                    let _ = tx.send(LlmEvent::Error(format!("Request error: {}", e)));
                }
            }
        });

        Ok(rx)
    }

    fn continue_with_tools(
        &self,
        model: &str,
        messages: Vec<ProviderMessage>,
        tools: Option<Vec<ToolDef>>,
        _tool_results: Vec<ToolResult>,
        max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>> {
        self.chat(model, messages, tools, max_tokens)
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                name: "GPT-4o".to_string(),
                provider: "openai".to_string(),
            },
            ModelInfo {
                id: "gpt-4-turbo".to_string(),
                name: "GPT-4 Turbo".to_string(),
                provider: "openai".to_string(),
            },
            ModelInfo {
                id: "gpt-4".to_string(),
                name: "GPT-4".to_string(),
                provider: "openai".to_string(),
            },
            ModelInfo {
                id: "gpt-3.5-turbo".to_string(),
                name: "GPT-3.5 Turbo".to_string(),
                provider: "openai".to_string(),
            },
        ])
    }
}
