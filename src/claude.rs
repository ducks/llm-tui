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
    Done { input_tokens: i64, output_tokens: i64 },
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
            tx.send(ClaudeEvent::Error(format!(
                "API request failed: {}",
                error_text
            )))?;
            return Ok(());
        }

        // Parse SSE stream
        use std::io::BufRead;
        let reader = std::io::BufReader::new(response);

        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_input = String::new();
        let mut input_tokens: i64 = 0;
        let mut output_tokens: i64 = 0;

        for line in reader.lines() {
            let line = line?;

            // SSE lines starting with "data: " contain JSON
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    break;
                }

                if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                    let event_type = event["type"].as_str().unwrap_or("");

                    match event_type {
                        "message_start" => {
                            // Extract initial usage info
                            if let Some(message) = event.get("message") {
                                if let Some(usage) = message.get("usage") {
                                    input_tokens = usage["input_tokens"].as_i64().unwrap_or(0);
                                    output_tokens = usage["output_tokens"].as_i64().unwrap_or(0);
                                }
                            }
                        }
                        "message_delta" => {
                            // Update usage with final counts
                            if let Some(usage) = event["usage"].as_object() {
                                output_tokens = usage.get("output_tokens")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(output_tokens);
                            }
                        }
                        "content_block_start" => {
                            // Check if it's a tool_use block
                            if let Some(content_block) = event.get("content_block") {
                                if content_block["type"] == "tool_use" {
                                    current_tool_id = content_block["id"].as_str().unwrap_or("").to_string();
                                    current_tool_name = content_block["name"].as_str().unwrap_or("").to_string();
                                    current_tool_input.clear();
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = event.get("delta") {
                                let delta_type = delta["type"].as_str().unwrap_or("");

                                if delta_type == "text_delta" {
                                    // Streaming text
                                    if let Some(text) = delta["text"].as_str() {
                                        tx.send(ClaudeEvent::Text(text.to_string()))?;
                                    }
                                } else if delta_type == "input_json_delta" {
                                    // Streaming tool input JSON
                                    if let Some(partial_json) = delta["partial_json"].as_str() {
                                        current_tool_input.push_str(partial_json);
                                    }
                                }
                            }
                        }
                        "content_block_stop" => {
                            // If we accumulated tool input, send the tool_use event
                            if !current_tool_name.is_empty() && !current_tool_input.is_empty() {
                                if let Ok(input) = serde_json::from_str(&current_tool_input) {
                                    tx.send(ClaudeEvent::ToolUse {
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
                            tx.send(ClaudeEvent::Done { input_tokens, output_tokens })?;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

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
            description: r#"A powerful search tool built on ripgrep.

Usage:
- ALWAYS use Grep for search tasks. NEVER invoke grep or rg as a Bash command. The Grep tool has been optimized for correct permissions and access.
- Supports full regex syntax (e.g., "log.*Error", "function\\s+\\w+")
- Filter files with glob parameter (e.g., "*.js", "**/*.tsx") or type parameter (e.g., "js", "py", "rust")
- Output modes: "content" shows matching lines, "files_with_matches" shows only file paths (default), "count" shows match counts
- Pattern syntax: Uses ripgrep (not grep) - literal braces need escaping (use interface\\{\\} to find interface{} in Go code)
- Multiline matching: By default patterns match within single lines only. For cross-line patterns like struct \\{[\\s\\S]*?field, use multiline: true"#.to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The regular expression pattern to search for in file contents"
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search in. Defaults to current working directory."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Glob pattern to filter files (e.g. \"*.js\", \"*.{ts,tsx}\")"
                    },
                    "type": {
                        "type": "string",
                        "description": "File type to search. Common types: js, py, rust, go, java, etc. More efficient than glob for standard file types."
                    },
                    "output_mode": {
                        "type": "string",
                        "description": "Output mode: \"content\" shows matching lines (supports -A/-B/-C context, -n line numbers), \"files_with_matches\" shows file paths (default), \"count\" shows match counts.",
                        "enum": ["content", "files_with_matches", "count"]
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "Case insensitive search"
                    },
                    "line_numbers": {
                        "type": "boolean",
                        "description": "Show line numbers in output. Requires output_mode: \"content\", ignored otherwise."
                    },
                    "context_before": {
                        "type": "number",
                        "description": "Number of lines to show before each match. Requires output_mode: \"content\", ignored otherwise."
                    },
                    "context_after": {
                        "type": "number",
                        "description": "Number of lines to show after each match. Requires output_mode: \"content\", ignored otherwise."
                    },
                    "multiline": {
                        "type": "boolean",
                        "description": "Enable multiline mode where . matches newlines and patterns can span lines. Default: false."
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
