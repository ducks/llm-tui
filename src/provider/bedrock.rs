//! Bedrock provider implementation

use super::{LlmEvent, LlmProvider, ModelInfo, ProviderMessage, ToolDef, ToolResult};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Tool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

pub struct BedrockProvider {}

impl BedrockProvider {
    pub fn new() -> Self {
        Self {}
    }

    fn convert_messages(messages: Vec<ProviderMessage>) -> Vec<Message> {
        messages
            .into_iter()
            .filter(|m| m.role != "system") // Bedrock Claude doesn't support system in messages
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

    fn chat_impl(
        model_id: String,
        messages: Vec<Message>,
        tools: Vec<Tool>,
        max_tokens: u32,
        tx: Sender<LlmEvent>,
    ) -> Result<()> {
        let rt = tokio::runtime::Runtime::new()?;

        rt.block_on(async {
            let config = aws_config::load_from_env().await;
            let client = aws_sdk_bedrockruntime::Client::new(&config);

            let request_body = json!({
                "anthropic_version": "bedrock-2023-05-31",
                "max_tokens": max_tokens,
                "messages": messages,
                "tools": tools,
            });

            let response = client
                .invoke_model()
                .model_id(&model_id)
                .content_type("application/json")
                .body(aws_sdk_bedrockruntime::primitives::Blob::new(
                    serde_json::to_vec(&request_body)?,
                ))
                .send()
                .await?;

            let response_body: serde_json::Value =
                serde_json::from_slice(response.body().as_ref())?;

            if let Some(content) = response_body["content"].as_array() {
                for block in content {
                    let block_type = block["type"].as_str().unwrap_or("");

                    match block_type {
                        "text" => {
                            if let Some(text) = block["text"].as_str() {
                                tx.send(LlmEvent::Text(text.to_string()))?;
                            }
                        }
                        "tool_use" => {
                            let id = block["id"].as_str().unwrap_or("").to_string();
                            let name = block["name"].as_str().unwrap_or("").to_string();
                            if let Some(input) = block.get("input") {
                                tx.send(LlmEvent::ToolUse {
                                    id,
                                    name,
                                    input: input.clone(),
                                })?;
                            }
                        }
                        _ => {}
                    }
                }
            }

            let input_tokens = response_body["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
            let output_tokens = response_body["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

            tx.send(LlmEvent::Done {
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
            })?;

            Ok::<(), anyhow::Error>(())
        })?;

        Ok(())
    }
}

impl Default for BedrockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmProvider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    fn is_available(&self) -> bool {
        // Check if AWS credentials are available
        std::env::var("AWS_PROFILE").is_ok()
            || std::env::var("AWS_ACCESS_KEY_ID").is_ok()
    }

    fn chat(
        &self,
        model: &str,
        messages: Vec<ProviderMessage>,
        tools: Option<Vec<ToolDef>>,
        max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>> {
        let (tx, rx) = channel();
        let model_id = model.to_string();
        let messages = Self::convert_messages(messages);
        let tools = Self::convert_tools(tools);

        thread::spawn(move || {
            if let Err(e) = Self::chat_impl(model_id, messages, tools, max_tokens, tx.clone()) {
                let _ = tx.send(LlmEvent::Error(format!("Bedrock error: {:?}", e)));
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
        let rt = tokio::runtime::Runtime::new()?;

        rt.block_on(async {
            let bedrock_config = aws_config::load_from_env().await;
            let bedrock_client = aws_sdk_bedrock::Client::new(&bedrock_config);

            let response = bedrock_client.list_inference_profiles().send().await?;

            let models: Vec<ModelInfo> = response
                .inference_profile_summaries()
                .iter()
                .filter_map(|profile| {
                    let profile_id = profile.inference_profile_id();
                    if profile_id.contains("anthropic.claude")
                        || profile_id.contains("us.anthropic.claude")
                    {
                        Some(ModelInfo {
                            id: profile_id.to_string(),
                            name: profile_id.to_string(),
                            provider: "bedrock".to_string(),
                        })
                    } else {
                        None
                    }
                })
                .collect();

            Ok(models)
        })
    }
}
