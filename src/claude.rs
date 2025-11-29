use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    #[serde(rename = "type")]
    pub result_type: String, // "tool_result"
    pub tool_use_id: String,
    pub content: String,
}

pub enum ClaudeEvent {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    Done,
    Error(String),
}

pub struct ClaudeClient {
    api_key: String,
    api_url: String,
}

impl ClaudeClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            api_url: "https://api.anthropic.com/v1/messages".to_string(),
        }
    }

    /// Send a chat request with tool support
    pub fn chat(
        &self,
        model: &str,
        messages: Vec<Message>,
        tools: Vec<Tool>,
        max_tokens: u32,
    ) -> Result<Receiver<ClaudeEvent>> {
        let (tx, rx) = channel();
        let api_key = self.api_key.clone();
        let api_url = self.api_url.clone();
        let model = model.to_string();

        thread::spawn(move || {
            if let Err(e) = Self::stream_chat(api_key, api_url, model, messages, tools, max_tokens, tx) {
                eprintln!("Claude chat error: {}", e);
            }
        });

        Ok(rx)
    }

    fn stream_chat(
        api_key: String,
        api_url: String,
        model: String,
        messages: Vec<Message>,
        tools: Vec<Tool>,
        max_tokens: u32,
        tx: Sender<ClaudeEvent>,
    ) -> Result<()> {
        let client = reqwest::blocking::Client::new();

        let body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
            "tools": tools,
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
            tx.send(ClaudeEvent::Error(format!(
                "API request failed: {}",
                error_text
            )))?;
            return Ok(());
        }

        let response_json: serde_json::Value = response.json()?;

        // Parse content blocks
        if let Some(content) = response_json["content"].as_array() {
            for block in content {
                if let Ok(content_block) = serde_json::from_value::<ContentBlock>(block.clone()) {
                    match content_block {
                        ContentBlock::Text { text } => {
                            tx.send(ClaudeEvent::Text(text))?;
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            tx.send(ClaudeEvent::ToolUse { id, name, input })?;
                        }
                    }
                }
            }
        }

        tx.send(ClaudeEvent::Done)?;
        Ok(())
    }

    /// Continue conversation with tool results
    pub fn continue_with_tools(
        &self,
        model: &str,
        mut messages: Vec<Message>,
        tools: Vec<Tool>,
        tool_results: Vec<ToolResult>,
        max_tokens: u32,
    ) -> Result<Receiver<ClaudeEvent>> {
        // Add tool results as a new assistant message
        // This is simplified - in reality we need to track the conversation properly
        let (tx, rx) = channel();
        let api_key = self.api_key.clone();
        let api_url = self.api_url.clone();
        let model = model.to_string();

        thread::spawn(move || {
            if let Err(e) = Self::stream_chat(api_key, api_url, model, messages, tools, max_tokens, tx) {
                eprintln!("Claude chat error: {}", e);
            }
        });

        Ok(rx)
    }
}

/// Define available tools for Claude
pub fn get_tool_definitions() -> Vec<Tool> {
    vec![
        Tool {
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
        Tool {
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
        Tool {
            name: "edit".to_string(),
            description: "Edit a file by replacing old_string with new_string. old_string must be unique in the file unless replace_all is true.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file to edit"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact string to replace (must match exactly including indentation)"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The string to replace it with"
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "If true, replace all occurrences. If false, old_string must be unique."
                    }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
        },
        Tool {
            name: "glob".to_string(),
            description: "Find files matching a glob pattern (e.g., '**/*.rs', 'src/*.js').".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The glob pattern to match files (e.g., '**/*.rs')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional: Base directory to search in (defaults to current directory)"
                    }
                },
                "required": ["pattern"]
            }),
        },
        Tool {
            name: "grep".to_string(),
            description: "Search for a pattern in files.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional: Directory to search in"
                    },
                    "glob": {
                        "type": "string",
                        "description": "Optional: Glob pattern to filter files (e.g., '*.rs')"
                    },
                    "output_mode": {
                        "type": "string",
                        "description": "Output mode: 'files_with_matches', 'content', or 'count'"
                    }
                },
                "required": ["pattern"]
            }),
        },
        Tool {
            name: "bash".to_string(),
            description: "Execute a bash command and return its output. Use this to run build commands, git operations, system utilities, etc.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to execute (e.g., 'ls -la', 'git status', 'cargo build')"
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Optional: Timeout in milliseconds (default: 120000, max: 600000)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional: Clear, concise description of what this command does"
                    }
                },
                "required": ["command"]
            }),
        },
    ]
}
