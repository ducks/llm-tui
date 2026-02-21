//! Unified LLM provider abstraction
//!
//! This module provides a common interface for all LLM providers (Ollama, Claude, Bedrock).
//! Each provider implements the `LlmProvider` trait, allowing the application to work with
//! any provider through a single, unified API.

pub mod bedrock;
pub mod claude;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod registry;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::mpsc::Receiver;

// Re-export provider implementations
pub use bedrock::BedrockProvider;
pub use claude::ClaudeProvider;
pub use gemini::GeminiProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAIProvider;
pub use registry::ProviderRegistry;

/// Unified event type for all providers
#[derive(Debug, Clone)]
pub enum LlmEvent {
    /// Streaming text content
    Text(String),
    /// Tool use request from the model
    ToolUse {
        #[allow(dead_code)]
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Response complete
    Done {
        #[allow(dead_code)]
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    /// Error occurred
    Error(String),
}

/// Common message format for providers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMessage {
    pub role: String,
    pub content: String,
}

/// Tool definition in unified format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Result of a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
}

/// Model information
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub provider: String,
}

/// Trait that all LLM providers must implement
#[allow(dead_code)]
pub trait LlmProvider: Send + Sync {
    /// Provider name (e.g., "ollama", "claude", "bedrock")
    fn name(&self) -> &str;

    /// Check if the provider is available and configured
    fn is_available(&self) -> bool;

    /// Start a chat, returns a receiver for streaming events
    fn chat(
        &self,
        model: &str,
        messages: Vec<ProviderMessage>,
        tools: Option<Vec<ToolDef>>,
        max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>>;

    /// Continue conversation after tool execution
    fn continue_with_tools(
        &self,
        model: &str,
        messages: Vec<ProviderMessage>,
        tools: Option<Vec<ToolDef>>,
        tool_results: Vec<ToolResult>,
        max_tokens: u32,
    ) -> Result<Receiver<LlmEvent>>;

    /// List available models for this provider
    fn list_models(&self) -> Result<Vec<ModelInfo>>;
}

/// Get the standard tool definitions used by all providers
pub fn get_tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "read".to_string(),
            description: "Read a file from the filesystem with line numbers.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file to read"
                    },
                    "offset": {
                        "type": "number",
                        "description": "Optional: Line number to start reading from (1-indexed)"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Optional: Number of lines to read"
                    }
                },
                "required": ["file_path"]
            }),
        },
        ToolDef {
            name: "write".to_string(),
            description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write to the file"
                    }
                },
                "required": ["file_path", "content"]
            }),
        },
        ToolDef {
            name: "edit".to_string(),
            description: "Edit a file by replacing old_string with new_string. The old_string must match exactly.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file to edit"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact string to find and replace"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The string to replace with"
                    }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
        },
        ToolDef {
            name: "glob".to_string(),
            description: "Find files matching a glob pattern.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern (e.g., '**/*.rs', 'src/**/*.ts')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional: Base directory to search in"
                    }
                },
                "required": ["pattern"]
            }),
        },
        ToolDef {
            name: "grep".to_string(),
            description: "Search for a pattern in files.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regular expression pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional: Directory or file to search in"
                    },
                    "include": {
                        "type": "string",
                        "description": "Optional: Glob pattern to filter files (e.g., '*.rs')"
                    }
                },
                "required": ["pattern"]
            }),
        },
        ToolDef {
            name: "bash".to_string(),
            description: "Execute a bash command. Use for git, build tools, or other CLI operations.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to execute"
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Optional: Timeout in seconds (default: 30)"
                    }
                },
                "required": ["command"]
            }),
        },
    ]
}
