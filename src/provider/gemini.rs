//! Google Gemini provider implementation

use super::{LlmEvent, LlmProvider, ModelInfo, ProviderMessage, ToolDef, ToolResult};
use anyhow::Result;
use serde::Serialize;
use std::io::BufRead;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

#[derive(Debug, Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
}

pub struct GeminiProvider {
    api_key: String,
}

impl GeminiProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    fn convert_messages(messages: Vec<ProviderMessage>) -> Vec<GeminiContent> {
        messages
            .into_iter()
            .filter(|m| m.role != "system") // Skip system messages for now
            .map(|m| GeminiContent {
                role: match m.role.as_str() {
                    "assistant" => "model".to_string(),
                    _ => "user".to_string(),
                },
                parts: vec![GeminiPart { text: m.content }],
            })
            .collect()
    }
}

impl LlmProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
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

            let request = GeminiRequest {
                contents: converted_messages,
                generation_config: Some(GeminiGenerationConfig {
                    max_output_tokens: max_tokens,
                }),
            };

            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?key={}",
                model, api_key
            );

            let client = reqwest::blocking::Client::new();

            match client
                .post(&url)
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
                            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(data) {
                                if let Some(candidates) = json_val["candidates"].as_array() {
                                    for candidate in candidates {
                                        if let Some(content) = candidate["content"].as_object() {
                                            if let Some(parts) = content["parts"].as_array() {
                                                for part in parts {
                                                    if let Some(text) = part["text"].as_str() {
                                                        let _ = tx
                                                            .send(LlmEvent::Text(text.to_string()));
                                                    }
                                                }
                                            }
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
        // For Gemini, tool results are added to messages
        self.chat(model, messages, tools, max_tokens)
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![
            ModelInfo {
                id: "gemini-2.0-flash-exp".to_string(),
                name: "Gemini 2.0 Flash Experimental".to_string(),
                provider: "gemini".to_string(),
            },
            ModelInfo {
                id: "gemini-1.5-pro".to_string(),
                name: "Gemini 1.5 Pro".to_string(),
                provider: "gemini".to_string(),
            },
            ModelInfo {
                id: "gemini-1.5-flash".to_string(),
                name: "Gemini 1.5 Flash".to_string(),
                provider: "gemini".to_string(),
            },
        ])
    }
}
