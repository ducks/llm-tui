//! OpenAI provider implementation

use super::{LlmEvent, LlmProvider, ModelInfo, ProviderMessage, ToolDef, ToolResult};
use anyhow::{anyhow, Result};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, ChatCompletionTool, ChatCompletionToolArgs,
        ChatCompletionToolType, CreateChatCompletionRequestArgs, FunctionObjectArgs,
    },
    Client,
};
use futures::StreamExt;
use std::sync::mpsc::{channel, Receiver, Sender};

pub struct OpenAIProvider {
    api_key: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    fn convert_tools(tools: Option<Vec<ToolDef>>) -> Vec<ChatCompletionTool> {
        tools
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| {
                ChatCompletionToolArgs::default()
                    .r#type(ChatCompletionToolType::Function)
                    .function(
                        FunctionObjectArgs::default()
                            .name(&t.name)
                            .description(&t.description)
                            .parameters(t.input_schema)
                            .build()
                            .ok()?,
                    )
                    .build()
                    .ok()
            })
            .collect()
    }

    fn convert_messages(messages: Vec<ProviderMessage>) -> Vec<ChatCompletionRequestMessage> {
        messages
            .into_iter()
            .filter_map(|m| match m.role.as_str() {
                "system" => ChatCompletionRequestSystemMessageArgs::default()
                    .content(&m.content)
                    .build()
                    .ok()
                    .map(ChatCompletionRequestMessage::System),
                "user" | "assistant" => ChatCompletionRequestUserMessageArgs::default()
                    .content(&m.content)
                    .build()
                    .ok()
                    .map(ChatCompletionRequestMessage::User),
                _ => None,
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
        tools: Option<Vec<ToolDef>>,
        max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>> {
        let (tx, rx) = channel();
        let api_key = self.api_key.clone();
        let model = model.to_string();
        let converted_messages = Self::convert_messages(messages);
        let converted_tools = Self::convert_tools(tools);

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let config = OpenAIConfig::new().with_api_key(&api_key);
                let client = Client::with_config(config);

                let mut request = CreateChatCompletionRequestArgs::default();
                request
                    .model(&model)
                    .messages(converted_messages)
                    .max_tokens(max_tokens);

                if !converted_tools.is_empty() {
                    request.tools(converted_tools);
                }

                let request = match request.build() {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = tx.send(LlmEvent::Error(format!("Request build error: {}", e)));
                        return;
                    }
                };

                let mut stream = match client.chat().create_stream(request).await {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = tx.send(LlmEvent::Error(format!("Stream error: {}", e)));
                        return;
                    }
                };

                while let Some(result) = stream.next().await {
                    match result {
                        Ok(response) => {
                            for choice in response.choices {
                                if let Some(content) = choice.delta.content {
                                    let _ = tx.send(LlmEvent::Text(content));
                                }

                                if let Some(tool_calls) = choice.delta.tool_calls {
                                    for tool_call in tool_calls {
                                        if let Some(function) = tool_call.function {
                                            if let (Some(name), Some(args)) = (function.name, function.arguments) {
                                                if let Ok(input) = serde_json::from_str(&args) {
                                                    let _ = tx.send(LlmEvent::ToolUse {
                                                        id: tool_call.id.unwrap_or_default(),
                                                        name,
                                                        input,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(LlmEvent::Error(format!("Stream error: {}", e)));
                            break;
                        }
                    }
                }

                let _ = tx.send(LlmEvent::Done {
                    input_tokens: None,
                    output_tokens: None,
                });
            });
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
        // For OpenAI, tool results are added to messages
        self.chat(model, messages, tools, max_tokens)
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![
            ModelInfo {
                id: "gpt-4".to_string(),
                name: "GPT-4".to_string(),
                provider: "openai".to_string(),
            },
            ModelInfo {
                id: "gpt-4-turbo".to_string(),
                name: "GPT-4 Turbo".to_string(),
                provider: "openai".to_string(),
            },
            ModelInfo {
                id: "gpt-4o".to_string(),
                name: "GPT-4o".to_string(),
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
