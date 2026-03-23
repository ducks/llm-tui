//! OpenAI-compatible provider implementation with tool support
//!
//! Works with OpenAI's API and any OpenAI-compatible endpoint
//! (hosted LLMs, OpenRouter, vLLM, llama.cpp, etc.)

use super::{LlmEvent, LlmProvider, ModelInfo, ProviderMessage, ToolDef};
use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use std::io::BufRead;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[derive(Debug, Serialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OpenAITool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAIFunction,
}

#[derive(Debug, Serialize)]
struct OpenAIFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAITool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<serde_json::Value>,
}

pub struct OpenAIProvider {
    api_key: String,
    base_url: String,
    provider_name: String,
}

impl OpenAIProvider {
    #[allow(dead_code)]
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
            provider_name: "openai".to_string(),
        }
    }

    pub fn with_base_url(api_key: String, base_url: String, provider_name: String) -> Self {
        Self {
            api_key,
            base_url,
            provider_name,
        }
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

    fn convert_tools(tools: Option<Vec<ToolDef>>) -> Option<Vec<OpenAITool>> {
        tools.map(|ts| {
            ts.into_iter()
                .map(|t| OpenAITool {
                    tool_type: "function".to_string(),
                    function: OpenAIFunction {
                        name: t.name,
                        description: t.description,
                        parameters: t.input_schema,
                    },
                })
                .collect()
        })
    }

    fn stream_chat(
        api_key: String,
        base_url: String,
        model: String,
        messages: Vec<OpenAIMessage>,
        tools: Option<Vec<OpenAITool>>,
        max_tokens: u32,
        tx: Sender<LlmEvent>,
    ) -> Result<()> {
        let client = reqwest::blocking::Client::new();

        let request = OpenAIRequest {
            model,
            messages,
            max_tokens,
            stream: true,
            tools,
            stream_options: Some(json!({"include_usage": true})),
        };

        let url = format!("{}/chat/completions", base_url);

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()?;

        if !response.status().is_success() {
            let error_text = response.text()?;
            tx.send(LlmEvent::Error(format!("API error: {}", error_text)))?;
            return Ok(());
        }

        let reader = std::io::BufReader::new(response);

        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_args = String::new();
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;

        for line in reader.lines() {
            let line = line?;

            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    break;
                }

                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(data) {
                    // Extract usage from the final chunk
                    if let Some(usage) = json_val.get("usage") {
                        input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0) as u32;
                        output_tokens = usage["completion_tokens"].as_u64().unwrap_or(0) as u32;
                    }

                    if let Some(choices) = json_val["choices"].as_array() {
                        for choice in choices {
                            let delta = &choice["delta"];

                            // Stream text content
                            if let Some(content) = delta["content"].as_str() {
                                tx.send(LlmEvent::Text(content.to_string()))?;
                            }

                            // Stream tool calls (accumulated across chunks)
                            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                                for tc in tool_calls {
                                    if let Some(id) = tc["id"].as_str() {
                                        // New tool call starting
                                        if !current_tool_name.is_empty() {
                                            // Flush previous tool call
                                            let input = serde_json::from_str(&current_tool_args)
                                                .unwrap_or(json!({}));
                                            tx.send(LlmEvent::ToolUse {
                                                id: current_tool_id.clone(),
                                                name: current_tool_name.clone(),
                                                input,
                                            })?;
                                            current_tool_args.clear();
                                        }
                                        current_tool_id = id.to_string();
                                    }
                                    if let Some(func) = tc.get("function") {
                                        if let Some(name) = func["name"].as_str() {
                                            current_tool_name = name.to_string();
                                        }
                                        if let Some(args) = func["arguments"].as_str() {
                                            current_tool_args.push_str(args);
                                        }
                                    }
                                }
                            }

                            // Check finish reason
                            if let Some(reason) = choice["finish_reason"].as_str() {
                                if reason == "tool_calls" && !current_tool_name.is_empty() {
                                    let input = serde_json::from_str(&current_tool_args)
                                        .unwrap_or(json!({}));
                                    tx.send(LlmEvent::ToolUse {
                                        id: current_tool_id.clone(),
                                        name: current_tool_name.clone(),
                                        input,
                                    })?;
                                    current_tool_id.clear();
                                    current_tool_name.clear();
                                    current_tool_args.clear();
                                }
                            }
                        }
                    }
                }
            }
        }

        tx.send(LlmEvent::Done {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
        })?;

        Ok(())
    }
}

impl LlmProvider for OpenAIProvider {
    fn name(&self) -> &str {
        &self.provider_name
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
        let base_url = self.base_url.clone();
        let model = model.to_string();
        let messages = Self::convert_messages(messages);
        let tools = Self::convert_tools(tools);

        thread::spawn(move || {
            if let Err(e) =
                Self::stream_chat(api_key, base_url, model, messages, tools, max_tokens, tx)
            {
                eprintln!("OpenAI chat error: {}", e);
            }
        });

        Ok(rx)
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                name: "GPT-4o".to_string(),
                provider: self.provider_name.clone(),
            },
            ModelInfo {
                id: "gpt-4o-mini".to_string(),
                name: "GPT-4o Mini".to_string(),
                provider: self.provider_name.clone(),
            },
            ModelInfo {
                id: "o3-mini".to_string(),
                name: "o3 Mini".to_string(),
                provider: self.provider_name.clone(),
            },
            ModelInfo {
                id: "gpt-4-turbo".to_string(),
                name: "GPT-4 Turbo".to_string(),
                provider: self.provider_name.clone(),
            },
        ])
    }
}
