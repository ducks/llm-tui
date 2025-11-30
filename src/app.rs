use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rusqlite::Connection;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::config::{AutosaveMode, Config};
use crate::db;
use crate::ollama::{ChatMessage, LlmEvent, OllamaClient};
use crate::session::Session;
use crate::tree::SessionTree;
use crate::tools::Tools;
use crate::claude::{ClaudeClient, ClaudeEvent};
use crate::bedrock::{BedrockClient, BedrockEvent};
use vim_navigator::{InputMode, ListNavigator, VimNavigator};

#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    SessionList,
    Chat,
    Models,
    Browser,
    Settings,
}

pub struct App {
    pub screen: AppScreen,
    pub vim_nav: VimNavigator,
    pub sessions: Vec<Session>,
    pub session_tree: SessionTree,
    pub session_nav: ListNavigator,
    pub current_session: Option<Session>,
    pub message_buffer: String,
    pub current_project: Option<String>,
    pub input_scroll: u16,
    pub message_scroll: u16,
    pub message_scroll_manual: bool, // true if user is manually scrolling
    pub conn: Connection,
    pub config: Config,
    pub last_autosave: Instant,
    pub needs_save: bool,
    pub ollama: OllamaClient,
    pub llm_receiver: Option<Receiver<LlmEvent>>,
    pub waiting_for_response: bool,
    pub assistant_buffer: String,
    pub models: Vec<crate::ollama::OllamaModel>,
    pub model_nav: ListNavigator,
    pub pull_status: Option<String>,
    pub pull_receiver: Option<Receiver<String>>,
    pub browse_models: Vec<crate::ollama::OllamaModel>,
    pub browse_nav: ListNavigator,
    pub tools: Tools,
    pub claude: Option<ClaudeClient>,
    pub claude_receiver: Option<Receiver<ClaudeEvent>>,
    pub bedrock: Option<BedrockClient>,
    pub bedrock_receiver: Option<Receiver<BedrockEvent>>,
    pub tool_status: Option<String>,
    pub pending_tool_results: Vec<(String, String)>, // (tool_name, result)
    pub pending_tool_call: Option<(String, serde_json::Value)>, // (tool_name, arguments) waiting for confirmation
    pub awaiting_tool_confirmation: bool,
}


impl App {
    pub fn new() -> Result<Self> {
        let config = Config::load()?;
        let conn = db::init_db()?;
        let sessions = db::list_sessions(&conn)?;

        let mut session_tree = SessionTree::new();
        session_tree.build_from_sessions(sessions.clone());

        let mut ollama = OllamaClient::new(config.ollama_url.clone());

        // Auto-start Ollama if configured
        if config.ollama_auto_start {
            let _ = ollama.start_server();
        }

        // Initialize Claude client if API key is configured
        let claude = config.claude_api_key.as_ref().map(|api_key| {
            ClaudeClient::new(api_key.clone())
        });

        // Initialize Bedrock client (uses AWS credentials from environment)
        let bedrock = Some(BedrockClient::new(config.bedrock_model.clone()));

        Ok(Self {
            screen: AppScreen::SessionList,
            vim_nav: VimNavigator::new(),
            sessions,
            session_tree,
            session_nav: ListNavigator::new(),
            current_session: None,
            message_buffer: String::new(),
            current_project: None,
            input_scroll: 0,
            message_scroll: 0,
            message_scroll_manual: false,
            conn,
            config,
            last_autosave: Instant::now(),
            needs_save: false,
            ollama,
            llm_receiver: None,
            waiting_for_response: false,
            assistant_buffer: String::new(),
            models: Vec::new(),
            model_nav: ListNavigator::new(),
            pull_status: None,
            pull_receiver: None,
            browse_models: Vec::new(),
            browse_nav: ListNavigator::new(),
            tools: Tools::new(),
            claude,
            claude_receiver: None,
            bedrock,
            bedrock_receiver: None,
            tool_status: None,
            pending_tool_results: Vec::new(),
            pending_tool_call: None,
            awaiting_tool_confirmation: false,
        })
    }

    pub fn rebuild_tree(&mut self) {
        self.session_tree.build_from_sessions(self.sessions.clone());
    }

    pub fn update_message_scroll(&mut self, visible_height: u16) {
        // Only auto-scroll if user hasn't manually scrolled
        // When user presses 'G' or new content arrives while at bottom, resume auto-scroll

        if self.message_scroll_manual {
            // User is manually scrolling, don't override
            return;
        }

        if let Some(ref session) = self.current_session {
            // Count total lines in all messages
            let mut total_lines = 0u16;
            for msg in &session.messages {
                let lines = msg.content.lines().count();
                total_lines = total_lines.saturating_add(lines.max(1) as u16);
            }

            // Add streaming buffer lines if waiting
            if self.waiting_for_response && !self.assistant_buffer.is_empty() {
                let buffer_lines = self.assistant_buffer.lines().count();
                total_lines = total_lines.saturating_add(buffer_lines.max(1) as u16);
            }

            // Scroll to show the bottom
            // Add some padding to account for message wrapping and role prefixes
            let padded_lines = total_lines.saturating_add(6);
            let new_scroll = padded_lines.saturating_sub(visible_height);
            crate::debug_log!("DEBUG scroll: total_lines={}, visible_height={}, padded_lines={}, setting scroll to {}", total_lines, visible_height, padded_lines, new_scroll);
            self.message_scroll = new_scroll;
        }
    }

    pub fn get_total_message_lines(&self) -> u16 {
        let mut total_lines = 0u16;
        if let Some(ref session) = self.current_session {
            for msg in &session.messages {
                let lines = msg.content.lines().count();
                total_lines = total_lines.saturating_add(lines.max(1) as u16);
            }
            if self.waiting_for_response && !self.assistant_buffer.is_empty() {
                let buffer_lines = self.assistant_buffer.lines().count();
                total_lines = total_lines.saturating_add(buffer_lines.max(1) as u16);
            }
        }
        total_lines
    }

    pub fn check_autosave(&mut self) {
        if self.config.autosave_mode != AutosaveMode::Timer {
            return;
        }

        if !self.needs_save {
            return;
        }

        let elapsed = self.last_autosave.elapsed();
        let interval = Duration::from_secs(self.config.autosave_interval_seconds);

        if elapsed >= interval {
            if let Some(ref session) = self.current_session {
                let _ = db::save_session(&self.conn, session);
                self.last_autosave = Instant::now();
                self.needs_save = false;
            }
        }
    }

    pub fn check_llm_response(&mut self) {
        if let Some(ref receiver) = self.llm_receiver {
            match receiver.try_recv() {
                Ok(LlmEvent::Token(token)) => {
                    crate::debug_log!("DEBUG: Received token: {:?}", token);
                    self.assistant_buffer.push_str(&token);
                }
                Ok(LlmEvent::ToolUse { name, arguments }) => {
                    crate::debug_log!("DEBUG: Received ToolUse - name: {}, args: {:?}", name, arguments);

                    // Store tool call for confirmation
                    self.pending_tool_call = Some((name.clone(), arguments));
                    self.awaiting_tool_confirmation = true;
                    self.tool_status = Some(format!("Waiting for confirmation: {} - Press y/n/q", name));
                }
                Ok(LlmEvent::Done) => {
                    crate::debug_log!("DEBUG: Received Done event, pending_tool_results: {}, awaiting_confirmation: {}",
                        self.pending_tool_results.len(), self.awaiting_tool_confirmation);

                    // If we're awaiting tool confirmation, don't process Done yet - wait for user response
                    if self.awaiting_tool_confirmation {
                        crate::debug_log!("DEBUG: Waiting for tool confirmation, not processing Done yet");
                        // Don't do anything - user needs to confirm/reject first
                    }
                    // If we have pending tool results, send them back to continue the conversation
                    else if !self.pending_tool_results.is_empty() {
                        crate::debug_log!("DEBUG: Continuing conversation with tool results");

                        // Save the assistant's tool call message and the tool results to history
                        if let Some(ref mut session) = self.current_session {
                            // Save assistant message with tool calls (marked as executed)
                            session.add_message_with_flag(
                                "assistant".to_string(),
                                self.assistant_buffer.clone(),
                                Some(self.config.ollama_model.clone()),
                                true, // tools_executed flag
                            );

                            // Save tool results as system message (also marked as executed)
                            let tool_results_text = self.pending_tool_results
                                .iter()
                                .map(|(name, result)| format!("[Tool {} result]:\n{}", name, result))
                                .collect::<Vec<_>>()
                                .join("\n\n");

                            session.add_message_with_flag(
                                "system".to_string(),
                                tool_results_text,
                                None,
                                true, // tools_executed flag
                            );
                        }

                        // Clear the buffer before continuing so we don't duplicate output
                        self.assistant_buffer.clear();

                        self.continue_with_tool_results();
                    } else {
                        // No more tool calls, save the final response
                        crate::debug_log!("DEBUG: No tool results, saving final response");
                        if let Some(ref mut session) = self.current_session {
                            session.add_message("assistant".to_string(), self.assistant_buffer.clone(), Some(self.config.ollama_model.clone()));
                            match self.config.autosave_mode {
                                AutosaveMode::OnSend => self.save_current_message(),
                                AutosaveMode::Timer => self.needs_save = true,
                                AutosaveMode::Disabled => {}
                            }
                        }
                        self.assistant_buffer.clear();
                        self.waiting_for_response = false;
                        self.llm_receiver = None;
                    }
                }
                Ok(LlmEvent::Error(err)) => {
                    crate::debug_log!("DEBUG: Received Error event: {}", err);
                    if let Some(ref mut session) = self.current_session {
                        session.add_message(
                            "system".to_string(),
                            format!("Error: {}", err),
                            None,
                        );
                    }
                    self.assistant_buffer.clear();
                    self.waiting_for_response = false;
                    self.llm_receiver = None;
                    self.pending_tool_results.clear();
                }
                Err(_) => {} // No message available yet
            }
        }
    }

    pub fn check_claude_response(&mut self) {
        if let Some(ref receiver) = self.claude_receiver {
            match receiver.try_recv() {
                Ok(ClaudeEvent::Text(text)) => {
                    crate::debug_log!("DEBUG CLAUDE: Received text: {:?}", text);
                    self.assistant_buffer.push_str(&text);
                }
                Ok(ClaudeEvent::ToolUse { id: _, name, input }) => {
                    crate::debug_log!("DEBUG CLAUDE: Received ToolUse - name: {}, input: {:?}", name, input);

                    // Store tool call for confirmation (same as Ollama flow)
                    self.pending_tool_call = Some((name.clone(), input));
                    self.awaiting_tool_confirmation = true;
                    self.tool_status = Some(format!("Waiting for confirmation: {} - Press y/n/q", name));
                }
                Ok(ClaudeEvent::Done) => {
                    crate::debug_log!("DEBUG CLAUDE: Received Done event, pending_tool_results: {}, awaiting_confirmation: {}",
                        self.pending_tool_results.len(), self.awaiting_tool_confirmation);

                    // If we're awaiting tool confirmation, don't process Done yet - wait for user response
                    if self.awaiting_tool_confirmation {
                        crate::debug_log!("DEBUG CLAUDE: Waiting for tool confirmation, not processing Done yet");
                        // Don't do anything - user needs to confirm/reject first
                    }
                    // If we have pending tool results, send them back to continue the conversation
                    else if !self.pending_tool_results.is_empty() {
                        crate::debug_log!("DEBUG CLAUDE: Continuing conversation with tool results");
                        // Note: Claude continuation needs proper implementation
                        // For now, just finish the response
                        self.pending_tool_results.clear();
                        self.waiting_for_response = false;
                        self.claude_receiver = None;
                    } else {
                        // No more tool calls, save the final response
                        crate::debug_log!("DEBUG CLAUDE: No tool results, saving final response");
                        if let Some(ref mut session) = self.current_session {
                            session.add_message("assistant".to_string(), self.assistant_buffer.clone(), Some(self.config.claude_model.clone()));
                            match self.config.autosave_mode {
                                AutosaveMode::OnSend => self.save_current_message(),
                                AutosaveMode::Timer => self.needs_save = true,
                                AutosaveMode::Disabled => {}
                            }
                        }
                        self.assistant_buffer.clear();
                        self.waiting_for_response = false;
                        self.claude_receiver = None;
                    }
                }
                Ok(ClaudeEvent::Error(err)) => {
                    crate::debug_log!("DEBUG CLAUDE: Received Error: {}", err);
                    if let Some(ref mut session) = self.current_session {
                        session.add_message(
                            "system".to_string(),
                            format!("Error: {}", err),
                            None,
                        );
                    }
                    self.assistant_buffer.clear();
                    self.waiting_for_response = false;
                    self.claude_receiver = None;
                    self.pending_tool_results.clear();
                }
                Err(_) => {} // No message available yet
            }
        }
    }

    pub fn check_bedrock_response(&mut self) {
        if let Some(ref receiver) = self.bedrock_receiver {
            match receiver.try_recv() {
                Ok(BedrockEvent::Text(text)) => {
                    crate::debug_log!("DEBUG BEDROCK: Received text: {:?}", text);
                    self.assistant_buffer.push_str(&text);
                }
                Ok(BedrockEvent::ToolUse { id: _, name, input }) => {
                    crate::debug_log!("DEBUG BEDROCK: Received ToolUse - name: {}, input: {:?}", name, input);

                    // Store tool call for confirmation (same as Ollama/Claude flow)
                    self.pending_tool_call = Some((name.clone(), input));
                    self.awaiting_tool_confirmation = true;
                    self.tool_status = Some(format!("Waiting for confirmation: {} - Press y/n/q", name));
                }
                Ok(BedrockEvent::Done) => {
                    crate::debug_log!("DEBUG BEDROCK: Received Done event, pending_tool_results: {}, awaiting_confirmation: {}",
                        self.pending_tool_results.len(), self.awaiting_tool_confirmation);

                    // If we're awaiting tool confirmation, don't process Done yet - wait for user response
                    if self.awaiting_tool_confirmation {
                        crate::debug_log!("DEBUG BEDROCK: Waiting for tool confirmation, not processing Done yet");
                        // Don't do anything - user needs to confirm/reject first
                    }
                    // If we have pending tool results, send them back to continue the conversation
                    else if !self.pending_tool_results.is_empty() {
                        crate::debug_log!("DEBUG BEDROCK: Continuing conversation with tool results");
                        // Note: Bedrock continuation needs proper implementation
                        // For now, just finish the response
                        self.pending_tool_results.clear();
                        self.waiting_for_response = false;
                        self.bedrock_receiver = None;
                    } else {
                        // No more tool calls, save the final response
                        crate::debug_log!("DEBUG BEDROCK: No tool results, saving final response");
                        if let Some(ref mut session) = self.current_session {
                            session.add_message("assistant".to_string(), self.assistant_buffer.clone(), Some(self.config.bedrock_model.clone()));
                            match self.config.autosave_mode {
                                AutosaveMode::OnSend => self.save_current_message(),
                                AutosaveMode::Timer => self.needs_save = true,
                                AutosaveMode::Disabled => {}
                            }
                        }
                        self.assistant_buffer.clear();
                        self.waiting_for_response = false;
                        self.bedrock_receiver = None;
                        self.message_scroll_manual = false; // Reset scroll to auto-scroll to new message
                    }
                }
                Ok(BedrockEvent::Error(err)) => {
                    crate::debug_log!("DEBUG BEDROCK: Received Error: {}", err);
                    if let Some(ref mut session) = self.current_session {
                        session.add_message(
                            "system".to_string(),
                            format!("Error: {}", err),
                            None,
                        );
                    }
                    self.assistant_buffer.clear();
                    self.waiting_for_response = false;
                    self.bedrock_receiver = None;
                    self.pending_tool_results.clear();
                }
                Err(_) => {} // No message available yet
            }
        }
    }

    pub fn confirm_tool_execution(&mut self) {
        if let Some((name, arguments)) = self.pending_tool_call.take() {
            crate::debug_log!("DEBUG: Executing confirmed tool: {}", name);

            // Execute tool and collect result
            let result = self.execute_tool(&name, arguments);
            let result_str = match result {
                Ok(output) => output,
                Err(e) => format!("Error: {}", e),
            };

            // Store tool result for later
            self.pending_tool_results.push((name.clone(), result_str.clone()));

            // Show in UI with better formatting
            self.assistant_buffer.push_str(&format!(
                "\n\n─────────────────────────────────────────\n[Tool: {}]\n─────────────────────────────────────────\n{}\n─────────────────────────────────────────\n",
                name,
                result_str
            ));
        }
        self.awaiting_tool_confirmation = false;
        self.tool_status = None;

        // Now process the pending tool results (trigger the Done logic)
        self.process_tool_completion();
    }

    pub fn reject_tool_execution(&mut self) {
        if let Some((name, _)) = self.pending_tool_call.take() {
            crate::debug_log!("DEBUG: Rejected tool execution: {}", name);
            self.pending_tool_results.push((name.clone(), "Tool execution rejected by user".to_string()));
        }
        self.awaiting_tool_confirmation = false;
        self.tool_status = None;

        // Process the rejection (trigger the Done logic)
        self.process_tool_completion();
    }

    fn process_tool_completion(&mut self) {
        // This is the logic from the Done event handler
        if !self.pending_tool_results.is_empty() {
            crate::debug_log!("DEBUG: Processing tool completion with {} results", self.pending_tool_results.len());

            // Save the assistant's tool call message and the tool results to history
            if let Some(ref mut session) = self.current_session {
                // Get the current provider's model name
                let model_name = match session.llm_provider.as_str() {
                    "bedrock" => Some(self.config.bedrock_model.clone()),
                    "claude" => Some(self.config.claude_model.clone()),
                    _ => Some(self.config.ollama_model.clone()),
                };

                // Save assistant message with tool calls (marked as executed)
                session.add_message_with_flag(
                    "assistant".to_string(),
                    self.assistant_buffer.clone(),
                    model_name,
                    true, // tools_executed flag
                );

                // Save tool results as system message (also marked as executed)
                let tool_results_text = self.pending_tool_results
                    .iter()
                    .map(|(name, result)| format!("[Tool {} result]:\n{}", name, result))
                    .collect::<Vec<_>>()
                    .join("\n\n");

                session.add_message_with_flag(
                    "system".to_string(),
                    tool_results_text,
                    None,
                    true, // tools_executed flag
                );
            }

            // Clear the buffer before continuing so we don't duplicate output
            self.assistant_buffer.clear();

            self.continue_with_tool_results();
        } else {
            // No tool results, just finish
            crate::debug_log!("DEBUG: No tool results after confirmation");
            if let Some(ref mut session) = self.current_session {
                session.add_message("assistant".to_string(), self.assistant_buffer.clone(), Some(self.config.ollama_model.clone()));
            }
            self.assistant_buffer.clear();
            self.waiting_for_response = false;
            self.llm_receiver = None;
        }
    }

    fn execute_tool(&mut self, name: &str, input: serde_json::Value) -> Result<String> {
        match name {
            "read" => {
                let params: crate::tools::ReadParams = serde_json::from_value(input)?;
                self.tools.read(params)
            }
            "write" => {
                let params: crate::tools::WriteParams = serde_json::from_value(input)?;
                self.tools.write(params)
            }
            "edit" => {
                let params: crate::tools::EditParams = serde_json::from_value(input)?;
                self.tools.edit(params)
            }
            "glob" => {
                let params: crate::tools::GlobParams = serde_json::from_value(input)?;
                self.tools.glob(params)
            }
            "grep" => {
                let params: crate::tools::GrepParams = serde_json::from_value(input)?;
                self.tools.grep(params)
            }
            "bash" => {
                let params: crate::tools::BashParams = serde_json::from_value(input)?;
                self.tools.bash(params)
            }
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
        }
    }

    fn send_llm_message(&mut self) -> Result<()> {
        let session = match self.current_session {
            Some(ref mut s) => s,
            None => return Ok(()),
        };

        let provider = &session.llm_provider;
        crate::debug_log!("DEBUG send_llm_message: provider = {}", provider);

        match provider.as_str() {
            "bedrock" => {
                if let Some(ref bedrock) = self.bedrock {
                    // Build a summary of previously executed tools
                    let tool_summary: Vec<String> = session
                        .messages
                        .iter()
                        .filter(|m| m.tools_executed && m.role == "system")
                        .map(|m| {
                            // Extract just the tool names from "[Tool xxx result]:" lines
                            m.content.lines()
                                .filter(|line| line.starts_with("[Tool "))
                                .map(|line| line.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .filter(|s| !s.is_empty())
                        .collect();

                    let context_note = if !tool_summary.is_empty() {
                        format!("\n\nNote: You have already executed these tools in this conversation: {}. You have access to their results in your context, so you don't need to re-run them.", tool_summary.join("; "))
                    } else {
                        String::new()
                    };

                    // Convert messages to Bedrock format (same as Claude)
                    let mut messages: Vec<crate::bedrock::Message> = vec![
                        crate::bedrock::Message {
                            role: "user".to_string(),
                            content: format!("You are a helpful AI assistant with access to tools for reading files, editing code, and searching the codebase.{}", context_note),
                        }
                    ];

                    messages.extend(
                        session
                            .messages
                            .iter()
                            .filter(|m| m.role != "system") // Bedrock doesn't support system messages in messages array
                            .filter(|m| !m.content.trim().is_empty()) // Skip empty messages
                            // NOTE: We DON'T filter tools_executed for Bedrock because we send tool results as plain user messages,
                            // so the model needs to see them to know what tools were already run
                            .map(|m| crate::bedrock::Message {
                                role: m.role.clone(),
                                content: m.content.clone(),
                            })
                    );

                    let tools = crate::bedrock::get_tool_definitions();

                    if let Ok(receiver) = bedrock.chat(messages, tools, 4096) {
                        self.bedrock_receiver = Some(receiver);
                        self.waiting_for_response = true;
                    }
                } else {
                    session.add_message(
                        "system".to_string(),
                        "Error: Bedrock client not initialized".to_string(),
                        None,
                    );
                }
            }
            "claude" => {
                if let Some(ref claude) = self.claude {
                    // Convert messages to Claude format
                    let messages: Vec<crate::claude::Message> = session
                        .messages
                        .iter()
                        .filter(|m| m.role != "system") // Claude doesn't support system messages in messages array
                        .filter(|m| !m.tools_executed) // Skip already-executed tool messages
                        .map(|m| crate::claude::Message {
                            role: m.role.clone(),
                            content: m.content.clone(),
                        })
                        .collect();

                    let tools = crate::claude::get_tool_definitions();

                    if let Ok(receiver) = claude.chat(&self.config.claude_model, messages, tools, 4096) {
                        self.claude_receiver = Some(receiver);
                        self.waiting_for_response = true;
                    }
                } else {
                    session.add_message(
                        "system".to_string(),
                        "Error: Claude API key not configured. Add it to ~/.config/llm-tui/config.toml".to_string(),
                        None,
                    );
                }
            }
            "ollama" | _ => {
                // Convert session messages to chat format
                let cwd = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "/".to_string());

                let mut messages: Vec<ChatMessage> = vec![ChatMessage {
                    role: "system".to_string(),
                    content: format!(
                        "You are a helpful assistant with access to tools for reading files, editing code, and searching the codebase.\n\nONLY use tools when the user explicitly asks you to work with files or code. Do NOT use tools for casual conversation.\n\nCurrent working directory: {}\n\nWhen using file paths, use absolute paths or paths relative to the current working directory.",
                        cwd
                    ),
                }];

                // Add all previous messages, but skip ones that have already-executed tools
                let total_messages = session.messages.len();
                let filtered_messages: Vec<_> = session.messages.iter()
                    .filter(|m| {
                        let keep = !m.tools_executed;
                        if !keep {
                            crate::debug_log!("DEBUG send_llm_message: Filtering out message with tools_executed=true: role={}, content_preview={}",
                                m.role,
                                m.content.chars().take(50).collect::<String>());
                        }
                        keep
                    })
                    .collect();

                crate::debug_log!("DEBUG send_llm_message: Total messages: {}, After filtering: {}", total_messages, filtered_messages.len());

                messages.extend(
                    filtered_messages.iter()
                        .map(|m| ChatMessage {
                            role: m.role.clone(),
                            content: m.content.clone(),
                        })
                );

                // Get tool definitions and convert to Ollama format
                let claude_tools = crate::claude::get_tool_definitions();
                let ollama_tools = crate::ollama::claude_tools_to_ollama(claude_tools);

                // Start LLM chat with tools
                if let Ok(receiver) = self.ollama.chat_with_tools(&self.config.ollama_model, messages, Some(ollama_tools)) {
                    self.llm_receiver = Some(receiver);
                    self.waiting_for_response = true;
                }
            }
        }

        Ok(())
    }

    fn continue_with_tool_results(&mut self) {
        let session = match self.current_session {
            Some(ref mut s) => s,
            None => return,
        };

        let provider = &session.llm_provider.clone();

        // Build tool result messages
        let tool_results: Vec<String> = self.pending_tool_results
            .iter()
            .map(|(name, result)| format!("[Tool {} result]:\n{}", name, result))
            .collect();

        crate::debug_log!("DEBUG: Sending {} tool results back to model", tool_results.len());

        match provider.as_str() {
            "bedrock" => {
                if let Some(ref bedrock) = self.bedrock {
                    // Convert messages to Bedrock format, adding tool results
                    let total_messages = session.messages.len();
                    let mut messages: Vec<crate::bedrock::Message> = session
                        .messages
                        .iter()
                        .filter(|m| m.role != "system") // Bedrock doesn't support system messages
                        .filter(|m| !m.content.trim().is_empty()) // Skip empty messages
                        // NOTE: We DON'T filter tools_executed because tool results need to stay in context
                        .map(|m| crate::bedrock::Message {
                            role: m.role.clone(),
                            content: m.content.clone(),
                        })
                        .collect();

                    crate::debug_log!("DEBUG continue_with_tool_results (bedrock): Total messages: {}, After filtering: {}", total_messages, messages.len());

                    // Add tool results as user message
                    messages.push(crate::bedrock::Message {
                        role: "user".to_string(),
                        content: tool_results.join("\n\n"),
                    });

                    // Clear pending results since we're sending them now
                    self.pending_tool_results.clear();

                    let tools = crate::bedrock::get_tool_definitions();

                    // Continue conversation
                    if let Ok(receiver) = bedrock.chat(messages, tools, 4096) {
                        self.bedrock_receiver = Some(receiver);
                        // Keep waiting_for_response = true
                    }
                } else {
                    self.pending_tool_results.clear();
                    self.waiting_for_response = false;
                    self.bedrock_receiver = None;
                }
            }
            "claude" => {
                // TODO: Implement Claude tool result continuation
                // For now, just clear and finish
                self.pending_tool_results.clear();
                self.waiting_for_response = false;
                self.claude_receiver = None;
            }
            "ollama" | _ => {
                let cwd = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "/".to_string());

                let mut messages: Vec<ChatMessage> = vec![ChatMessage {
                    role: "system".to_string(),
                    content: format!(
                        "You are a helpful assistant with access to tools for reading files, editing code, and searching the codebase.\n\nONLY use tools when the user explicitly asks you to work with files or code. Do NOT use tools for casual conversation.\n\nCurrent working directory: {}\n\nWhen using file paths, use absolute paths or paths relative to the current working directory.",
                        cwd
                    ),
                }];

                // Add all previous messages, but skip ones that have already-executed tools
                let total_messages = session.messages.len();
                let filtered_messages: Vec<_> = session.messages.iter()
                    .filter(|m| {
                        let keep = !m.tools_executed;
                        if !keep {
                            crate::debug_log!("DEBUG continue_with_tool_results: Filtering out message with tools_executed=true: role={}, content_preview={}",
                                m.role,
                                m.content.chars().take(50).collect::<String>());
                        }
                        keep
                    })
                    .collect();

                crate::debug_log!("DEBUG continue_with_tool_results: Total messages: {}, After filtering: {}", total_messages, filtered_messages.len());

                messages.extend(
                    filtered_messages.iter()
                        .map(|m| ChatMessage {
                            role: m.role.clone(),
                            content: m.content.clone(),
                        })
                );

                // Add tool results as a user message
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: tool_results.join("\n\n"),
                });

                // Clear pending results since we're sending them now
                self.pending_tool_results.clear();

                // Get tool definitions
                let claude_tools = crate::claude::get_tool_definitions();
                let ollama_tools = crate::ollama::claude_tools_to_ollama(claude_tools);

                // Continue conversation
                if let Ok(receiver) = self.ollama.chat_with_tools(&self.config.ollama_model, messages, Some(ollama_tools)) {
                    self.llm_receiver = Some(receiver);
                    // Keep waiting_for_response = true
                }
            }
        }
    }

    pub fn check_pull_progress(&mut self) {
        if let Some(ref receiver) = self.pull_receiver {
            match receiver.try_recv() {
                Ok(status) => {
                    if status.contains("success") || status.contains("complete") {
                        self.pull_status = None;
                        self.pull_receiver = None;
                        // Refresh model list
                        if let Ok(models) = self.ollama.list_models() {
                            self.models = models;
                        }
                    } else {
                        self.pull_status = Some(status);
                    }
                }
                Err(_) => {} // No update yet
            }
        }
    }

    fn save_current_message(&mut self) {
        if let Some(ref mut session) = self.current_session {
            if let Some(last_msg) = session.messages.last() {
                let _ = db::save_message(&self.conn, &session.id, last_msg);
            }
            let _ = db::save_session(&self.conn, session);
            self.last_autosave = Instant::now();
            self.needs_save = false;
        }
    }

    pub fn handle_input(&mut self, key: KeyEvent) -> Result<bool> {
        // If awaiting tool confirmation, handle y/n/q regardless of vim mode
        if self.awaiting_tool_confirmation {
            return self.handle_tool_confirmation(key);
        }

        match self.vim_nav.mode {
            InputMode::Normal => self.handle_normal_mode(key),
            InputMode::Command => self.handle_command_mode(key),
            InputMode::Insert => self.handle_insert_mode(key),
        }
    }

    fn handle_tool_confirmation(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirm_tool_execution();
                Ok(false)
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                self.reject_tool_execution();
                Ok(false)
            }
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                // Quit/cancel - reject and stop waiting for response
                self.reject_tool_execution();
                self.waiting_for_response = false;
                self.llm_receiver = None;
                self.claude_receiver = None;
                self.bedrock_receiver = None;
                Ok(false)
            }
            _ => Ok(false), // Ignore other keys while waiting for confirmation
        }
    }

    fn handle_normal_mode(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => return Ok(true), // Quit
            KeyCode::Char(':') => {
                self.vim_nav.mode = InputMode::Command;
                self.vim_nav.command_buffer.clear();
            }
            KeyCode::Char('1') => {
                self.screen = AppScreen::SessionList;
            }
            KeyCode::Char('2') => {
                if self.current_session.is_some() {
                    self.screen = AppScreen::Chat;
                }
            }
            KeyCode::Char('3') => {
                self.screen = AppScreen::Models;
                if let Ok(models) = self.ollama.list_models() {
                    self.models = models;
                }
            }
            KeyCode::Char('4') => {
                self.screen = AppScreen::Browser;
                if let Ok(browse) = self.ollama.browse_library() {
                    self.browse_models = browse;
                }
            }
            KeyCode::Char('i') if self.screen == AppScreen::Chat => {
                self.vim_nav.mode = InputMode::Insert;
            }
            KeyCode::Enter if self.screen == AppScreen::Chat => {
                // Send message in normal mode
                if !self.message_buffer.is_empty() && !self.waiting_for_response {
                    if let Some(ref mut session) = self.current_session {
                        session.add_message("user".to_string(), self.message_buffer.clone(), None);

                        let _ = self.send_llm_message();

                        match self.config.autosave_mode {
                            AutosaveMode::OnSend => self.save_current_message(),
                            AutosaveMode::Timer => self.needs_save = true,
                            AutosaveMode::Disabled => {}
                        }
                    }
                    self.message_buffer.clear();
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.screen == AppScreen::Chat {
                    // Scroll down (increase scroll offset)
                    self.message_scroll = self.message_scroll.saturating_add(1);
                    self.message_scroll_manual = true; // User is manually scrolling
                } else if self.screen == AppScreen::SessionList && !self.session_tree.items.is_empty() {
                    self.session_nav.selected_index =
                        (self.session_nav.selected_index + 1).min(self.session_tree.items.len() - 1);
                } else if self.screen == AppScreen::Models && !self.models.is_empty() {
                    self.model_nav.selected_index =
                        (self.model_nav.selected_index + 1).min(self.models.len() - 1);
                } else if self.screen == AppScreen::Browser && !self.browse_models.is_empty() {
                    self.browse_nav.selected_index =
                        (self.browse_nav.selected_index + 1).min(self.browse_models.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.screen == AppScreen::Chat {
                    // Scroll up (decrease scroll offset)
                    self.message_scroll = self.message_scroll.saturating_sub(1);
                    self.message_scroll_manual = true; // User is manually scrolling
                } else if self.screen == AppScreen::SessionList {
                    self.session_nav.selected_index = self.session_nav.selected_index.saturating_sub(1);
                } else if self.screen == AppScreen::Models {
                    self.model_nav.selected_index = self.model_nav.selected_index.saturating_sub(1);
                } else if self.screen == AppScreen::Browser {
                    self.browse_nav.selected_index = self.browse_nav.selected_index.saturating_sub(1);
                }
            }
            KeyCode::Char('g') => {
                if self.screen == AppScreen::SessionList {
                    self.session_nav.selected_index = 0;
                }
            }
            KeyCode::Char('G') => {
                if self.screen == AppScreen::Chat {
                    // Jump to bottom and resume auto-scroll
                    // Just reset the flag and let update_message_scroll() handle it on next render
                    self.message_scroll_manual = false;
                } else if self.screen == AppScreen::SessionList && !self.session_tree.items.is_empty() {
                    self.session_nav.selected_index = self.session_tree.items.len() - 1;
                }
            }
            KeyCode::Char('n') => {
                if self.screen == AppScreen::SessionList && !self.session_tree.items.is_empty() {
                    // Get parent project of currently selected item
                    let project = self.session_tree.get_parent_project(self.session_nav.selected_index);
                    let session = Session::new(None, project, Some(self.config.ollama_model.clone()));
                    if let Ok(_) = db::save_session(&self.conn, &session) {
                        self.sessions = db::list_sessions(&self.conn).unwrap_or_default();
                        self.rebuild_tree();
                        self.current_session = Some(session);
                        self.screen = AppScreen::Chat;
                    }
                }
            }
            KeyCode::Char('d') => {
                if self.screen == AppScreen::SessionList && !self.session_tree.items.is_empty() {
                    // Get the currently selected item
                    let selected_idx = self.session_nav.selected_index;
                    if selected_idx < self.session_tree.items.len() {
                        if let Some(session) = self.session_tree.items[selected_idx].session() {
                            let session_id = session.id.clone();
                            let _ = db::delete_session(&self.conn, &session_id);
                            self.sessions = db::list_sessions(&self.conn).unwrap_or_default();
                            self.rebuild_tree();
                            // Adjust selected index if needed
                            if self.session_nav.selected_index >= self.session_tree.items.len() && self.session_tree.items.len() > 0 {
                                self.session_nav.selected_index = self.session_tree.items.len() - 1;
                            }
                        }
                    }
                }
            }
            KeyCode::Char(' ') => {
                // Space bar toggles project expand/collapse
                if self.screen == AppScreen::SessionList && !self.session_tree.items.is_empty() {
                    let selected_idx = self.session_nav.selected_index;
                    if selected_idx < self.session_tree.items.len() {
                        let item = &self.session_tree.items[selected_idx];
                        if item.is_project() {
                            self.session_tree.toggle_project(selected_idx);
                            self.rebuild_tree();
                        }
                    }
                }
            }
            KeyCode::Enter => {
                if self.screen == AppScreen::SessionList && !self.session_tree.items.is_empty() {
                    let selected_idx = self.session_nav.selected_index;
                    if selected_idx < self.session_tree.items.len() {
                        let item = &self.session_tree.items[selected_idx];

                        if let Some(session) = item.session() {
                            // Open session
                            let mut session = session.clone();
                            if let Ok(messages) = db::load_messages(&self.conn, &session.id) {
                                session.messages = messages;
                            }
                            
                            // Set session context for tools (for saving files)
                            // Note: We can't share the connection directly, so we'll handle saving in execute_tool
                            
                            // Load session files and add them to context
                            if let Ok(session_files) = db::load_session_files(&self.conn, &session.id) {
                                for file in session_files {
                                    // Check if file has changed on disk
                                    let current_content = if db::should_reload_file(&file.file_path, &file.content_hash).unwrap_or(true) {
                                        // File changed or missing, use cached content but re-read if possible
                                        match std::fs::read_to_string(&file.file_path) {
                                            Ok(new_content) => {
                                                crate::debug_log!("File {} changed, using updated content", file.file_path);
                                                new_content
                                            }
                                            Err(_) => {
                                                crate::debug_log!("File {} not found, using cached content", file.file_path);
                                                file.content
                                            }
                                        }
                                    } else {
                                        // File unchanged, use cached content
                                        file.content
                                    };
                                    
                                    // Add file contents as system message for context
                                    let context_message = crate::session::Message {
                                        role: "system".to_string(),
                                        content: format!("[File: {}]\n\n{}", file.file_path, current_content),
                                        timestamp: chrono::Utc::now(),
                                        model: None,
                                        tools_executed: false,
                                    };
                                    session.messages.push(context_message);
                                }
                            }
                            
                            self.current_session = Some(session);
                            self.screen = AppScreen::Chat;
                        }
                    }
                } else if self.screen == AppScreen::Models && !self.models.is_empty() {
                    // Select model and update config
                    let model_name = self.models[self.model_nav.selected_index].name.clone();
                    self.config.ollama_model = model_name;
                    let _ = self.config.save();
                } else if self.screen == AppScreen::Browser && !self.browse_models.is_empty() {
                    // Pull model from browse list
                    let model_name = self.browse_models[self.browse_nav.selected_index].name.clone();
                    self.pull_status = Some(format!("Starting download: {}", model_name));
                    if let Ok(receiver) = self.ollama.pull_model(&model_name) {
                        self.pull_receiver = Some(receiver);
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_command_mode(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.vim_nav.mode = InputMode::Normal;
                self.vim_nav.command_buffer.clear();
            }
            KeyCode::Enter => {
                let should_quit = self.execute_command()?;
                self.vim_nav.mode = InputMode::Normal;
                self.vim_nav.command_buffer.clear();
                if should_quit {
                    return Ok(true);
                }
            }
            KeyCode::Backspace => {
                self.vim_nav.command_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.vim_nav.command_buffer.push(c);
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_insert_mode(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.vim_nav.mode = InputMode::Normal;
            }
            KeyCode::Enter => {
                // Plain Enter adds newline in insert mode
                self.message_buffer.push('\n');
                // Auto-scroll down only if we have more than 8 lines (input area is 10 lines, keep some buffer)
                let line_count = self.message_buffer.lines().count();
                if line_count > 8 {
                    self.input_scroll = (line_count as u16).saturating_sub(7);
                }
            }
            KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+Space sends message in insert mode
                if !self.message_buffer.is_empty() && !self.waiting_for_response {
                    if let Some(ref mut session) = self.current_session {
                        session.add_message("user".to_string(), self.message_buffer.clone(), None);

                        let _ = self.send_llm_message();

                        match self.config.autosave_mode {
                            AutosaveMode::OnSend => self.save_current_message(),
                            AutosaveMode::Timer => self.needs_save = true,
                            AutosaveMode::Disabled => {}
                        }
                    }
                    self.message_buffer.clear();
                    self.input_scroll = 0;
                }
            }
            KeyCode::Backspace => {
                self.message_buffer.pop();
                // Adjust scroll based on line count
                let line_count = self.message_buffer.lines().count();
                if line_count > 8 {
                    self.input_scroll = (line_count as u16).saturating_sub(7);
                } else {
                    self.input_scroll = 0;
                }
            }
            KeyCode::Char(c) => {
                self.message_buffer.push(c);
            }
            _ => {}
        }
        Ok(false)
    }

    fn execute_command(&mut self) -> Result<bool> {
        let cmd = self.vim_nav.command_buffer.trim();

        if cmd == "q" || cmd == "quit" {
            return Ok(true);
        }

        if cmd == "w" || cmd == "save" {
            if let Some(ref session) = self.current_session {
                db::save_session(&self.conn, session)?;
            }
            return Ok(false);
        }

        // :provider <name> - switch LLM provider for current session
        if cmd.starts_with("provider") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 1 {
                let provider = parts[1].to_lowercase();
                if provider == "claude" || provider == "ollama" || provider == "bedrock" {
                    if let Some(ref mut session) = self.current_session {
                        crate::debug_log!("DEBUG: Changing provider from '{}' to '{}'", session.llm_provider, provider);

                        // Unload Ollama model if switching away from Ollama
                        if session.llm_provider == "ollama" && provider != "ollama" {
                            let _ = self.ollama.unload_model(&self.config.ollama_model);
                        }

                        session.llm_provider = provider.clone();
                        let _ = db::save_session(&self.conn, session);

                        // Clear any active receivers from previous provider
                        self.llm_receiver = None;
                        self.claude_receiver = None;
                        self.bedrock_receiver = None;
                        self.waiting_for_response = false;

                        session.add_message(
                            "system".to_string(),
                            format!("Provider switched to: {}", provider),
                            None,
                        );
                    }
                }
            }
            return Ok(false);
        }

        // :session new [name] - create new session
        if cmd.starts_with("session new") || cmd.starts_with("session create") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            let name = if parts.len() > 2 {
                Some(parts[2..].join(" "))
            } else {
                None
            };

            let session = Session::new(name, self.current_project.clone(), Some(self.config.ollama_model.clone()));
            db::save_session(&self.conn, &session)?;
            self.current_session = Some(session);
            self.screen = AppScreen::Chat;
            self.sessions = db::list_sessions(&self.conn)?;
            self.rebuild_tree();
            return Ok(false);
        }

        // :project new <name> - create project with initial session
        if cmd.starts_with("project new") || cmd.starts_with("project create") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 2 {
                let project_name = parts[2..].join(" ");
                self.current_project = Some(project_name.clone());

                // Create initial session in the new project
                let session = Session::new(None, Some(project_name), Some(self.config.ollama_model.clone()));
                db::save_session(&self.conn, &session)?;
                self.current_session = Some(session);
                self.screen = AppScreen::Chat;
                self.sessions = db::list_sessions(&self.conn)?;
                self.rebuild_tree();
            }
            return Ok(false);
        }

        // :project <name> - switch to project
        if cmd.starts_with("project") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 1 && parts[1] != "new" && parts[1] != "create" {
                self.current_project = Some(parts[1..].join(" "));
            }
            return Ok(false);
        }

        // Legacy :new command (kept for backward compatibility)
        if cmd.starts_with("new") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();

            // Parse --project flag
            let mut project = self.current_project.clone();
            let mut name_parts = Vec::new();
            let mut i = 1;
            while i < parts.len() {
                if parts[i] == "--project" && i + 1 < parts.len() {
                    project = Some(parts[i + 1].to_string());
                    i += 2;
                } else {
                    name_parts.push(parts[i]);
                    i += 1;
                }
            }

            let name = if !name_parts.is_empty() {
                Some(name_parts.join(" "))
            } else {
                None
            };

            let session = Session::new(name, project, Some(self.config.ollama_model.clone()));
            db::save_session(&self.conn, &session)?;
            self.current_session = Some(session);
            self.screen = AppScreen::Chat;
            self.sessions = db::list_sessions(&self.conn)?;
            self.rebuild_tree();
            return Ok(false);
        }

        if cmd == "models" {
            self.screen = AppScreen::Models;
            if let Ok(models) = self.ollama.list_models() {
                self.models = models;
            }
            if let Ok(browse) = self.ollama.browse_library() {
                self.browse_models = browse;
            }
            return Ok(false);
        }

        if cmd.starts_with("pull") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 1 {
                let model_name = parts[1].to_string();
                self.pull_status = Some(format!("Starting download: {}", model_name));
                if let Ok(receiver) = self.ollama.pull_model(&model_name) {
                    self.pull_receiver = Some(receiver);
                }
            }
            return Ok(false);
        }

        if cmd.starts_with("delete") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 1 {
                let model_name = parts[1].to_string();
                let _ = self.ollama.delete_model(&model_name);
                // Refresh model list
                if let Ok(models) = self.ollama.list_models() {
                    self.models = models;
                }
            }
            return Ok(false);
        }

        if cmd == "delete-session" || cmd == "ds" {
            if let Some(ref session) = self.current_session {
                let session_id = session.id.clone();
                db::delete_session(&self.conn, &session_id)?;
                self.current_session = None;
                self.sessions = db::list_sessions(&self.conn)?;
                self.rebuild_tree();
                self.screen = AppScreen::SessionList;
            }
            return Ok(false);
        }

        if cmd.starts_with("rename") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 1 && self.current_session.is_some() {
                let new_name = parts[1..].join(" ");
                if let Some(ref mut session) = self.current_session {
                    session.name = Some(new_name.clone());
                    db::rename_session(&self.conn, &session.id, &new_name)?;
                    self.sessions = db::list_sessions(&self.conn)?;
                    self.rebuild_tree();
                }
            }
            return Ok(false);
        }

        if cmd.starts_with("load") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 1 && self.current_session.is_some() {
                let target = parts[1..].join(" ");

                // Try to load as file first
                if let Ok(content) = std::fs::read_to_string(&target) {
                    if let Some(ref mut session) = self.current_session {
                        session.add_message(
                            "system".to_string(),
                            format!("Context loaded from file '{}':\n\n{}", target, content),
                            None,
                        );
                        match self.config.autosave_mode {
                            AutosaveMode::OnSend => {
                                if let Some(last_msg) = session.messages.last() {
                                    let _ = db::save_message(&self.conn, &session.id, last_msg);
                                }
                                let _ = db::save_session(&self.conn, session);
                            }
                            AutosaveMode::Timer => self.needs_save = true,
                            AutosaveMode::Disabled => {}
                        }
                    }
                } else {
                    // Try to find session by name or ID (but not the current session)
                    let current_id = self.current_session.as_ref().map(|s| s.id.as_str());

                    // Try exact ID match first
                    let mut found_session = self.sessions.iter()
                        .find(|s| Some(s.id.as_str()) != current_id && s.id == target);

                    // If no exact ID, try exact name match (case insensitive)
                    if found_session.is_none() {
                        found_session = self.sessions.iter()
                            .find(|s| {
                                Some(s.id.as_str()) != current_id &&
                                s.name.as_ref().map(|n| n.to_lowercase() == target.to_lowercase()).unwrap_or(false)
                            });
                    }

                    // If still no match, try partial name match (contains)
                    if found_session.is_none() {
                        found_session = self.sessions.iter()
                            .find(|s| {
                                Some(s.id.as_str()) != current_id &&
                                s.name.as_ref().map(|n| n.to_lowercase().contains(&target.to_lowercase())).unwrap_or(false)
                            });
                    }

                    if let Some(found_session) = found_session {
                        if let Ok(messages) = db::load_messages(&self.conn, &found_session.id) {
                            if let Some(ref mut session) = self.current_session {
                                // Format all messages from the loaded session
                                let context: Vec<String> = messages.iter().map(|m| {
                                    format!("[{}]: {}", m.role, m.content)
                                }).collect();

                                session.add_message(
                                    "system".to_string(),
                                    format!("Context loaded from session '{}':\n\n{}",
                                        found_session.display_name(),
                                        context.join("\n\n")
                                    ),
                                    None,
                                );

                                match self.config.autosave_mode {
                                    AutosaveMode::OnSend => {
                                        if let Some(last_msg) = session.messages.last() {
                                            let _ = db::save_message(&self.conn, &session.id, last_msg);
                                        }
                                        let _ = db::save_session(&self.conn, session);
                                    }
                                    AutosaveMode::Timer => self.needs_save = true,
                                    AutosaveMode::Disabled => {}
                                }
                            }
                        }
                    }
                }
            }
            return Ok(false);
        }

        Ok(false)
    }
}
