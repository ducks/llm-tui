//! Google Gemini provider implementation with tool support

use super::{LlmEvent, LlmProvider, ModelInfo, ProviderMessage, ToolDef};
use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use std::io::BufRead;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[derive(Debug, Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum GeminiPart {
    Text { text: String },
}

#[derive(Debug, Serialize)]
struct GeminiToolDeclaration {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Debug, Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiToolDeclaration>>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
            .filter(|m| m.role != "system")
            .map(|m| GeminiContent {
                role: match m.role.as_str() {
                    "assistant" => "model".to_string(),
                    _ => "user".to_string(),
                },
                parts: vec![GeminiPart::Text { text: m.content }],
            })
            .collect()
    }

    fn convert_tools(tools: Option<Vec<ToolDef>>) -> Option<Vec<GeminiToolDeclaration>> {
        tools.map(|ts| {
            vec![GeminiToolDeclaration {
                function_declarations: ts
                    .into_iter()
                    .map(|t| GeminiFunctionDeclaration {
                        name: t.name,
                        description: t.description,
                        parameters: t.input_schema,
                    })
                    .collect(),
            }]
        })
    }

    fn stream_chat(
        api_key: String,
        model: String,
        messages: Vec<GeminiContent>,
        tools: Option<Vec<GeminiToolDeclaration>>,
        max_tokens: u32,
        tx: Sender<LlmEvent>,
    ) -> Result<()> {
        let client = reqwest::blocking::Client::new();

        let request = GeminiRequest {
            contents: messages,
            tools,
            generation_config: Some(GeminiGenerationConfig {
                max_output_tokens: max_tokens,
            }),
        };

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?key={}&alt=sse",
            model, api_key
        );

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()?;

        if !response.status().is_success() {
            let error_text = response.text()?;
            tx.send(LlmEvent::Error(format!("API error: {}", error_text)))?;
            return Ok(());
        }

        let reader = std::io::BufReader::new(response);

        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;

        for line in reader.lines() {
            let line = line?;

            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(data) {
                    // Extract usage metadata
                    if let Some(usage) = json_val.get("usageMetadata") {
                        input_tokens = usage["promptTokenCount"].as_u64().unwrap_or(0) as u32;
                        output_tokens = usage["candidatesTokenCount"].as_u64().unwrap_or(0) as u32;
                    }

                    if let Some(candidates) = json_val["candidates"].as_array() {
                        for candidate in candidates {
                            if let Some(parts) = candidate["content"]["parts"].as_array() {
                                for part in parts {
                                    // Text response
                                    if let Some(text) = part["text"].as_str() {
                                        tx.send(LlmEvent::Text(text.to_string()))?;
                                    }

                                    // Function call response
                                    if let Some(fc) = part.get("functionCall") {
                                        let name = fc["name"].as_str().unwrap_or("").to_string();
                                        let args = fc.get("args").cloned().unwrap_or(json!({}));
                                        tx.send(LlmEvent::ToolUse {
                                            id: format!("gemini-tool-{}", name),
                                            name,
                                            input: args,
                                        })?;
                                    }
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
        tools: Option<Vec<ToolDef>>,
        max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>> {
        let (tx, rx) = channel();
        let api_key = self.api_key.clone();
        let model = model.to_string();
        let messages = Self::convert_messages(messages);
        let tools = Self::convert_tools(tools);

        thread::spawn(move || {
            if let Err(e) = Self::stream_chat(api_key, model, messages, tools, max_tokens, tx) {
                eprintln!("Gemini chat error: {}", e);
            }
        });

        Ok(rx)
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![
            ModelInfo {
                id: "gemini-2.5-flash".to_string(),
                name: "Gemini 2.5 Flash".to_string(),
                provider: "gemini".to_string(),
            },
            ModelInfo {
                id: "gemini-2.0-flash".to_string(),
                name: "Gemini 2.0 Flash".to_string(),
                provider: "gemini".to_string(),
            },
            ModelInfo {
                id: "gemini-1.5-pro".to_string(),
                name: "Gemini 1.5 Pro".to_string(),
                provider: "gemini".to_string(),
            },
        ])
    }
}
