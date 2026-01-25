//! Claude provider implementation

use super::{LlmEvent, LlmProvider, ModelInfo, ProviderMessage, ToolDef, ToolResult};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::BufRead;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

pub struct ClaudeProvider {
    api_key: String,
    api_url: String,
}

impl ClaudeProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            api_url: "https://api.anthropic.com/v1/messages".to_string(),
        }
    }

    fn convert_messages(messages: Vec<ProviderMessage>) -> Vec<Message> {
        messages
            .into_iter()
            .filter(|m| m.role != "system") // Claude doesn't support system in messages array
            .map(|m| Message {
                role: m.role,
                content: m.content,
            })
            .collect()
    }

    fn convert_tools(tools: Option<Vec<ToolDef>>) -> Vec<Tool> {
        tools
            .unwrap_or_default()
            .into_iter()
            .map(|t| Tool {
                name: t.name,
                description: t.description,
                input_schema: t.input_schema,
            })
            .collect()
    }

    fn stream_chat(
        api_key: String,
        api_url: String,
        model: String,
        messages: Vec<Message>,
        tools: Vec<Tool>,
        max_tokens: u32,
        tx: Sender<LlmEvent>,
    ) -> Result<()> {
        let client = reqwest::blocking::Client::new();

        let body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
            "tools": tools,
            "stream": true,
        });

        let response = client
            .post(&api_url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()?;

        if !response.status().is_success() {
            let error_text = response.text()?;
            tx.send(LlmEvent::Error(format!("API request failed: {}", error_text)))?;
            return Ok(());
        }

        let reader = std::io::BufReader::new(response);

        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_input = String::new();
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;

        for line in reader.lines() {
            let line = line?;

            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    break;
                }

                if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                    let event_type = event["type"].as_str().unwrap_or("");

                    match event_type {
                        "message_start" => {
                            if let Some(message) = event.get("message") {
                                if let Some(usage) = message.get("usage") {
                                    input_tokens = usage["input_tokens"].as_u64().unwrap_or(0) as u32;
                                    output_tokens = usage["output_tokens"].as_u64().unwrap_or(0) as u32;
                                }
                            }
                        }
                        "message_delta" => {
                            if let Some(usage) = event["usage"].as_object() {
                                output_tokens = usage
                                    .get("output_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(output_tokens as u64) as u32;
                            }
                        }
                        "content_block_start" => {
                            if let Some(content_block) = event.get("content_block") {
                                if content_block["type"] == "tool_use" {
                                    current_tool_id =
                                        content_block["id"].as_str().unwrap_or("").to_string();
                                    current_tool_name =
                                        content_block["name"].as_str().unwrap_or("").to_string();
                                    current_tool_input.clear();
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = event.get("delta") {
                                let delta_type = delta["type"].as_str().unwrap_or("");

                                if delta_type == "text_delta" {
                                    if let Some(text) = delta["text"].as_str() {
                                        tx.send(LlmEvent::Text(text.to_string()))?;
                                    }
                                } else if delta_type == "input_json_delta" {
                                    if let Some(partial_json) = delta["partial_json"].as_str() {
                                        current_tool_input.push_str(partial_json);
                                    }
                                }
                            }
                        }
                        "content_block_stop" => {
                            if !current_tool_name.is_empty() && !current_tool_input.is_empty() {
                                if let Ok(input) = serde_json::from_str(&current_tool_input) {
                                    tx.send(LlmEvent::ToolUse {
                                        id: current_tool_id.clone(),
                                        name: current_tool_name.clone(),
                                        input,
                                    })?;
                                }
                                current_tool_name.clear();
                                current_tool_input.clear();
                                current_tool_id.clear();
                            }
                        }
                        "message_stop" => {
                            tx.send(LlmEvent::Done {
                                input_tokens: Some(input_tokens),
                                output_tokens: Some(output_tokens),
                            })?;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }
}

impl LlmProvider for ClaudeProvider {
    fn name(&self) -> &str {
        "claude"
    }

    fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    fn chat(
        &self,
        model: &str,
        messages: Vec<ProviderMessage>,
        tools: Option<Vec<ToolDef>>,
        max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>> {
        let (tx, rx) = channel();
        let api_key = self.api_key.clone();
        let api_url = self.api_url.clone();
        let model = model.to_string();
        let messages = Self::convert_messages(messages);
        let tools = Self::convert_tools(tools);

        thread::spawn(move || {
            if let Err(e) = Self::stream_chat(api_key, api_url, model, messages, tools, max_tokens, tx) {
                eprintln!("Claude chat error: {}", e);
            }
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
        // Claude doesn't have a list models API, return static list
        Ok(vec![
            ModelInfo {
                id: "claude-sonnet-4-20250514".to_string(),
                name: "Claude Sonnet 4".to_string(),
                provider: "claude".to_string(),
            },
            ModelInfo {
                id: "claude-3-5-sonnet-20241022".to_string(),
                name: "Claude 3.5 Sonnet".to_string(),
                provider: "claude".to_string(),
            },
            ModelInfo {
                id: "claude-3-opus-20240229".to_string(),
                name: "Claude 3 Opus".to_string(),
                provider: "claude".to_string(),
            },
            ModelInfo {
                id: "claude-3-haiku-20240307".to_string(),
                name: "Claude 3 Haiku".to_string(),
                provider: "claude".to_string(),
            },
        ])
    }
}
