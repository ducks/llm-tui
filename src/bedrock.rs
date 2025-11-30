use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[derive(Debug, Clone)]
pub enum BedrockEvent {
    Text(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
    Done { input_tokens: i64, output_tokens: i64 },
    Error(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

pub struct BedrockClient {}

impl BedrockClient {
    pub fn new() -> Self {
        Self {}
    }

    pub fn list_models() -> Result<Vec<String>> {
        // Use tokio runtime to call async AWS SDK
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            let bedrock_config = aws_config::load_from_env().await;
            let bedrock_client = aws_sdk_bedrock::Client::new(&bedrock_config);

            // List inference profiles instead of foundation models
            // Inference profiles are the correct way to invoke Bedrock models
            let response = bedrock_client
                .list_inference_profiles()
                .send()
                .await?;

            let models: Vec<String> = response
                .inference_profile_summaries()
                .iter()
                .filter_map(|profile| {
                    // Get the inference profile ID
                    let profile_id = profile.inference_profile_id();
                    // Only show Claude profiles
                    if profile_id.contains("anthropic.claude") || profile_id.contains("us.anthropic.claude") {
                        Some(profile_id.to_string())
                    } else {
                        None
                    }
                })
                .collect();

            Ok(models)
        })
    }

    pub fn chat(
        &self,
        model_id: String,
        messages: Vec<Message>,
        tools: Vec<Tool>,
        max_tokens: u32,
    ) -> Result<Receiver<BedrockEvent>> {
        let (tx, rx) = channel();

        thread::spawn(move || {
            if let Err(e) = Self::chat_impl(model_id, messages, tools, max_tokens, tx.clone()) {
                let _ = tx.send(BedrockEvent::Error(format!("Bedrock error: {:?}", e)));
            }
        });

        Ok(rx)
    }

    fn chat_impl(
        model_id: String,
        messages: Vec<Message>,
        tools: Vec<Tool>,
        max_tokens: u32,
        tx: Sender<BedrockEvent>,
    ) -> Result<()> {
        // Need to use tokio runtime for async AWS SDK
        let rt = tokio::runtime::Runtime::new()?;

        rt.block_on(async {
            // Load AWS config from environment
            let config = aws_config::load_from_env().await;
            let client = aws_sdk_bedrockruntime::Client::new(&config);

            // Build request body in Claude format (Bedrock uses same format)
            let request_body = json!({
                "anthropic_version": "bedrock-2023-05-31",
                "max_tokens": max_tokens,
                "messages": messages,
                "tools": tools,
            });

            // Invoke model (non-streaming for now)
            let response = client
                .invoke_model()
                .model_id(&model_id)
                .content_type("application/json")
                .body(aws_sdk_bedrockruntime::primitives::Blob::new(
                    serde_json::to_vec(&request_body)?
                ))
                .send()
                .await?;

            // Parse response body
            let response_body: serde_json::Value = serde_json::from_slice(response.body().as_ref())?;

            // Process content blocks
            if let Some(content) = response_body["content"].as_array() {
                for block in content {
                    let block_type = block["type"].as_str().unwrap_or("");

                    match block_type {
                        "text" => {
                            if let Some(text) = block["text"].as_str() {
                                tx.send(BedrockEvent::Text(text.to_string()))?;
                            }
                        }
                        "tool_use" => {
                            let id = block["id"].as_str().unwrap_or("").to_string();
                            let name = block["name"].as_str().unwrap_or("").to_string();
                            if let Some(input) = block.get("input") {
                                tx.send(BedrockEvent::ToolUse {
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

            // Extract token usage
            let input_tokens = response_body["usage"]["input_tokens"].as_i64().unwrap_or(0);
            let output_tokens = response_body["usage"]["output_tokens"].as_i64().unwrap_or(0);

            tx.send(BedrockEvent::Done { input_tokens, output_tokens })?;

            Ok::<(), anyhow::Error>(())
        })?;

        Ok(())
    }
}

/// Get tool definitions in Claude/Bedrock format
pub fn get_tool_definitions() -> Vec<Tool> {
    crate::claude::get_tool_definitions()
        .into_iter()
        .map(|claude_tool| Tool {
            name: claude_tool.name,
            description: claude_tool.description,
            input_schema: claude_tool.input_schema,
        })
        .collect()
}
