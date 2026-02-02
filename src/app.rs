use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rusqlite::Connection;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::config::{AutosaveMode, Config};
use crate::db;
use crate::provider::{LlmEvent, LlmProvider, OllamaProvider, ProviderMessage};
use crate::session::Session;
use crate::tools::Tools;
use crate::tree::SessionTree;
use vim_navigator::{InputMode, ListNavigator, VimNavigator};

#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    SessionList,
    Chat,
    Models,
    Settings,
    Help,
    Setup,
}

#[derive(Debug, Clone)]
pub struct ProviderModel {
    pub provider: String, // "ollama", "claude", "bedrock"
    pub model_id: String,
    pub installed: bool,  // For Ollama models
    pub is_current: bool, // Currently selected for this session
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
    // Provider for Ollama-specific operations (pull, delete, browse models)
    pub ollama: OllamaProvider,
    // Unified response receiver for all providers
    pub response_receiver: Option<Receiver<LlmEvent>>,
    pub waiting_for_response: bool,
    pub assistant_buffer: String,
    pub models: Vec<crate::provider::ollama::OllamaModel>,
    pub model_nav: ListNavigator,
    pub pull_status: Option<String>,
    pub pull_receiver: Option<Receiver<String>>,
    pub provider_models: Vec<ProviderModel>,
    pub tools: Tools,
    pub tool_status: Option<String>,
    pub pending_tool_results: Vec<(String, String)>, // (tool_name, result)
    pub pending_tool_call: Option<(String, serde_json::Value)>, // (tool_name, arguments) waiting for confirmation
    pub awaiting_tool_confirmation: bool,
    pub setup_step: usize, // Current step in setup wizard (0=welcome, 1=ollama, 2=claude, 3=bedrock, 4=complete)
    pub setup_message: String, // Status message for setup wizard
    pub ollama_status: Option<bool>,
    pub claude_status: Option<bool>,
    pub bedrock_status: Option<bool>,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = Config::load()?;
        let conn = db::init_db()?;
        let sessions = db::list_sessions(&conn)?;

        let mut session_tree = SessionTree::new();
        session_tree.build_from_sessions(sessions.clone());

        let mut ollama = OllamaProvider::new(&config.ollama_url);

        // Auto-start Ollama if configured
        if config.ollama_auto_start {
            let _ = ollama.start_server();
        }

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
            response_receiver: None,
            waiting_for_response: false,
            assistant_buffer: String::new(),
            models: Vec::new(),
            model_nav: ListNavigator::new(),
            pull_status: None,
            pull_receiver: None,
            provider_models: Vec::new(),
            tools: Tools::new(),
            tool_status: None,
            pending_tool_results: Vec::new(),
            pending_tool_call: None,
            awaiting_tool_confirmation: false,
            setup_step: 0,
            setup_message: String::new(),
            ollama_status: None,
            claude_status: None,
            bedrock_status: None,
        })
    }

    pub fn rebuild_tree(&mut self) {
        self.session_tree.build_from_sessions(self.sessions.clone());
    }

    pub fn refresh_provider_models(&mut self) {
        self.refresh_provider_models_full();
    }

    fn refresh_provider_models_full(&mut self) {
        let mut provider_models = Vec::new();

        // Get current provider and model from session if available, otherwise use config
        let (current_provider, current_model) = if let Some(ref session) = self.current_session {
            (
                session.llm_provider.as_str(),
                session.model.as_ref().map(|m| m.as_str()),
            )
        } else {
            let model = match self.config.default_llm_provider.as_str() {
                "claude" => Some(self.config.claude_model.as_str()),
                "bedrock" => Some(self.config.bedrock_model.as_str()),
                _ => Some(self.config.ollama_model.as_str()),
            };
            (self.config.default_llm_provider.as_str(), model)
        };

        // Ollama models - merge installed and browseable
        let installed_models = self.ollama.list_ollama_models().unwrap_or_default();
        let browseable_models = self.ollama.browse_library().unwrap_or_default();

        // Create a set of installed model names for quick lookup
        let installed_names: std::collections::HashSet<String> =
            installed_models.iter().map(|m| m.name.clone()).collect();

        // Add all browseable models (marking installed ones)
        for model in browseable_models {
            let is_installed = installed_names.contains(&model.name);
            let is_current =
                current_provider == "ollama" && current_model == Some(model.name.as_str());
            provider_models.push(ProviderModel {
                provider: "ollama".to_string(),
                model_id: model.name,
                installed: is_installed,
                is_current,
            });
        }

        // Add any installed models that aren't in the browse list
        for model in installed_models {
            // Check if we already added this model from browse list
            if !provider_models
                .iter()
                .any(|pm| pm.provider == "ollama" && pm.model_id == model.name)
            {
                let is_current =
                    current_provider == "ollama" && current_model == Some(model.name.as_str());
                provider_models.push(ProviderModel {
                    provider: "ollama".to_string(),
                    model_id: model.name,
                    installed: true,
                    is_current,
                });
            }
        }

        // Claude API models - use the provider's list_models
        let claude_provider = crate::provider::ClaudeProvider::new(
            self.config.claude_api_key.clone().unwrap_or_default(),
        );
        if let Ok(claude_models) = claude_provider.list_models() {
            for model in claude_models {
                let is_current =
                    current_provider == "claude" && current_model == Some(model.id.as_str());
                provider_models.push(ProviderModel {
                    provider: "claude".to_string(),
                    model_id: model.id,
                    installed: false, // Claude API models don't need installation
                    is_current,
                });
            }
        }

        // Bedrock models - use the provider's list_models
        let bedrock_provider = crate::provider::BedrockProvider::new();
        if let Ok(bedrock_models) = bedrock_provider.list_models() {
            for model in bedrock_models {
                let is_current =
                    current_provider == "bedrock" && current_model == Some(model.id.as_str());
                provider_models.push(ProviderModel {
                    provider: "bedrock".to_string(),
                    model_id: model.id,
                    installed: false, // Bedrock models don't need installation
                    is_current,
                });
            }
        }

        self.provider_models = provider_models;
    }

    fn update_current_model_flags(&mut self) {
        // Lightweight update: just update is_current flags without fetching from APIs
        let (current_provider, current_model) = if let Some(ref session) = self.current_session {
            (
                session.llm_provider.as_str(),
                session.model.as_ref().map(|m| m.as_str()),
            )
        } else {
            let model = match self.config.default_llm_provider.as_str() {
                "claude" => Some(self.config.claude_model.as_str()),
                "bedrock" => Some(self.config.bedrock_model.as_str()),
                _ => Some(self.config.ollama_model.as_str()),
            };
            (self.config.default_llm_provider.as_str(), model)
        };

        for model in &mut self.provider_models {
            model.is_current = model.provider == current_provider
                && current_model == Some(model.model_id.as_str());
        }
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

    /// Unified response handler for all LLM providers
    pub fn check_response(&mut self) {
        if let Some(ref receiver) = self.response_receiver {
            match receiver.try_recv() {
                Ok(LlmEvent::Text(text)) => {
                    crate::debug_log!("DEBUG: Received text: {:?}", text);
                    self.assistant_buffer.push_str(&text);
                }
                Ok(LlmEvent::ToolUse { id: _, name, input }) => {
                    crate::debug_log!(
                        "DEBUG: Received ToolUse - name: {}, input: {:?}",
                        name,
                        input
                    );

                    // Store tool call for confirmation
                    self.pending_tool_call = Some((name.clone(), input));
                    self.awaiting_tool_confirmation = true;
                    self.tool_status =
                        Some(format!("Waiting for confirmation: {} - Press y/n/q", name));
                }
                Ok(LlmEvent::Done {
                    input_tokens: _,
                    output_tokens,
                }) => {
                    crate::debug_log!("DEBUG: Received Done event, pending_tool_results: {}, awaiting_confirmation: {}",
                        self.pending_tool_results.len(), self.awaiting_tool_confirmation);

                    // If we're awaiting tool confirmation, don't process Done yet - wait for user response
                    if self.awaiting_tool_confirmation {
                        crate::debug_log!(
                            "DEBUG: Waiting for tool confirmation, not processing Done yet"
                        );
                        // Don't do anything - user needs to confirm/reject first
                    }
                    // If we have pending tool results, send them back to continue the conversation
                    else if !self.pending_tool_results.is_empty() {
                        crate::debug_log!("DEBUG: Continuing conversation with tool results");

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
                            let mut tool_results_text = String::new();
                            for (i, (name, result)) in self.pending_tool_results.iter().enumerate()
                            {
                                if i > 0 {
                                    tool_results_text.push_str("\n\n");
                                }
                                tool_results_text
                                    .push_str(&format!("[Tool {} result]:\n{}", name, result));
                            }

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
                            // Get the current provider's model name
                            let model_name = match session.llm_provider.as_str() {
                                "bedrock" => Some(self.config.bedrock_model.clone()),
                                "claude" => Some(self.config.claude_model.clone()),
                                _ => Some(self.config.ollama_model.clone()),
                            };

                            let token_count = output_tokens.map(|t| t as i64);
                            session.add_message_full(
                                "assistant".to_string(),
                                self.assistant_buffer.clone(),
                                model_name,
                                false, // tools_executed
                                false, // is_summary
                                token_count,
                            );
                            match self.config.autosave_mode {
                                AutosaveMode::OnSend => self.save_current_message(),
                                AutosaveMode::Timer => self.needs_save = true,
                                AutosaveMode::Disabled => {}
                            }
                        }
                        self.assistant_buffer.clear();
                        self.waiting_for_response = false;
                        self.response_receiver = None;
                        self.message_scroll_manual = false; // Reset scroll to auto-scroll to new message
                    }
                }
                Ok(LlmEvent::Error(err)) => {
                    crate::debug_log!("DEBUG: Received Error event: {}", err);
                    if let Some(ref mut session) = self.current_session {
                        session.add_message("system".to_string(), format!("Error: {}", err), None);
                    }
                    self.assistant_buffer.clear();
                    self.waiting_for_response = false;
                    self.response_receiver = None;
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
            self.pending_tool_results
                .push((name.clone(), result_str.clone()));

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
            self.pending_tool_results
                .push((name.clone(), "Tool execution rejected by user".to_string()));
        }
        self.awaiting_tool_confirmation = false;
        self.tool_status = None;

        // Process the rejection (trigger the Done logic)
        self.process_tool_completion();
    }

    fn process_tool_completion(&mut self) {
        // This is the logic from the Done event handler
        if !self.pending_tool_results.is_empty() {
            crate::debug_log!(
                "DEBUG: Processing tool completion with {} results",
                self.pending_tool_results.len()
            );

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
                    model_name.clone(),
                    true, // tools_executed flag
                );

                // Save tool results as system message (also marked as executed)
                let mut tool_results_text = String::new();
                for (i, (name, result)) in self.pending_tool_results.iter().enumerate() {
                    if i > 0 {
                        tool_results_text.push_str("\n\n");
                    }
                    tool_results_text.push_str(&format!("[Tool {} result]:\n{}", name, result));
                }

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
                // Get the current provider's model name
                let model_name = match session.llm_provider.as_str() {
                    "bedrock" => Some(self.config.bedrock_model.clone()),
                    "claude" => Some(self.config.claude_model.clone()),
                    _ => Some(self.config.ollama_model.clone()),
                };
                session.add_message(
                    "assistant".to_string(),
                    self.assistant_buffer.clone(),
                    model_name,
                );
            }
            self.assistant_buffer.clear();
            self.waiting_for_response = false;
            self.response_receiver = None;
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

    fn compact_conversation(&mut self) -> Result<()> {
        let session = match self.current_session {
            Some(ref mut s) => s,
            None => return Ok(()),
        };

        // Check if we have messages to compact
        let range = match session.get_compactable_range(self.config.autocompact_keep_recent) {
            Some(r) => r,
            None => {
                session.add_message(
                    "system".to_string(),
                    "Not enough messages to compact (need more than configured keep_recent threshold)".to_string(),
                    None,
                );
                return Ok(());
            }
        };

        // Extract messages to compact
        let filtered_messages: Vec<_> = session.messages[range.0..=range.1]
            .iter()
            .filter(|m| !m.is_summary && !m.tools_executed)
            .collect();

        if filtered_messages.is_empty() {
            return Ok(());
        }

        let mut messages_to_compact = String::new();
        for (i, m) in filtered_messages.iter().enumerate() {
            if i > 0 {
                messages_to_compact.push_str("\n\n");
            }
            messages_to_compact.push_str(&format!("[{}]: {}", m.role, m.content));
        }

        let compact_prompt = format!(
            "Please provide a concise summary of the following conversation segment. \
            Focus on key information, decisions made, and important context that should be preserved. \
            Keep the summary under 500 tokens.\n\n{}",
            messages_to_compact
        );

        // Send compact request to LLM (using current provider)
        let provider_name = &session.llm_provider.clone();

        // Create a temporary message list for the summary request
        let summary_messages = vec![ProviderMessage {
            role: "user".to_string(),
            content: compact_prompt.clone(),
        }];

        // Get summary synchronously using unified provider
        let summary_text = match provider_name.as_str() {
            "bedrock" => {
                let provider = crate::provider::BedrockProvider::new();
                let model_id = session
                    .model
                    .clone()
                    .unwrap_or_else(|| self.config.bedrock_model.clone());

                if let Ok(receiver) = provider.chat(&model_id, summary_messages, None, 2048) {
                    let mut result = String::new();
                    loop {
                        match receiver.recv() {
                            Ok(LlmEvent::Text(text)) => result.push_str(&text),
                            Ok(LlmEvent::Done { .. }) => break,
                            Ok(LlmEvent::Error(e)) => {
                                return Err(anyhow::anyhow!("Bedrock error: {}", e))
                            }
                            _ => {}
                        }
                    }
                    result
                } else {
                    return Err(anyhow::anyhow!("Failed to start Bedrock chat"));
                }
            }
            "claude" => {
                let provider = crate::provider::ClaudeProvider::new(
                    self.config.claude_api_key.clone().unwrap_or_default(),
                );

                if let Ok(receiver) =
                    provider.chat(&self.config.claude_model, summary_messages, None, 2048)
                {
                    let mut result = String::new();
                    loop {
                        match receiver.recv() {
                            Ok(LlmEvent::Text(text)) => result.push_str(&text),
                            Ok(LlmEvent::Done { .. }) => break,
                            Ok(LlmEvent::Error(e)) => {
                                return Err(anyhow::anyhow!("Claude error: {}", e))
                            }
                            _ => {}
                        }
                    }
                    result
                } else {
                    return Err(anyhow::anyhow!("Failed to start Claude chat"));
                }
            }
            "ollama" | _ => {
                if let Ok(receiver) =
                    self.ollama
                        .chat(&self.config.ollama_model, summary_messages, None, 2048)
                {
                    let mut result = String::new();
                    loop {
                        match receiver.recv() {
                            Ok(LlmEvent::Text(text)) => result.push_str(&text),
                            Ok(LlmEvent::Done { .. }) => break,
                            Ok(LlmEvent::Error(e)) => {
                                return Err(anyhow::anyhow!("Ollama error: {}", e))
                            }
                            _ => {}
                        }
                    }
                    result
                } else {
                    return Err(anyhow::anyhow!("Failed to start Ollama chat"));
                }
            }
        };

        // Mark old messages as tools_executed (so they get filtered out)
        for i in range.0..=range.1 {
            if !session.messages[i].is_summary && !session.messages[i].tools_executed {
                session.messages[i].tools_executed = true;
            }
        }

        // Add summary as new message
        session.add_message_full(
            "system".to_string(),
            format!(
                "[Summary of {} messages]: {}",
                range.1 - range.0 + 1,
                summary_text
            ),
            session.model.clone(),
            false, // tools_executed
            true,  // is_summary
            None,  // token_count will be auto-calculated
        );

        // Save session
        let _ = db::save_session(&self.conn, session);

        // Update the compacted messages in database (already marked as tools_executed)
        for i in range.0..=range.1 {
            if session.messages[i].tools_executed {
                let _ = db::update_message(&self.conn, &session.id, &session.messages[i]);
            }
        }

        // Save only the new summary message
        if let Some(summary_msg) = session.messages.last() {
            let _ = db::save_message(&self.conn, &session.id, summary_msg);
        }

        Ok(())
    }

    fn send_llm_message(&mut self) -> Result<()> {
        // Check if autocompact should trigger BEFORE accessing session
        let should_compact = if let Some(ref session) = self.current_session {
            let context_window = match session.llm_provider.as_str() {
                "bedrock" => self.config.bedrock_context_window,
                "claude" => self.config.claude_context_window,
                _ => self.config.ollama_context_window,
            };
            session.should_autocompact(context_window, self.config.autocompact_threshold)
        } else {
            false
        };

        // Trigger compact if needed
        if should_compact {
            if let Some(ref session) = self.current_session {
                let total = session.total_tokens();
                let context_window = match session.llm_provider.as_str() {
                    "bedrock" => self.config.bedrock_context_window,
                    "claude" => self.config.claude_context_window,
                    _ => self.config.ollama_context_window,
                };
                crate::debug_log!(
                    "DEBUG: Auto-compacting conversation ({}/{} tokens, {}% full)",
                    total,
                    context_window,
                    (total as f64 / context_window as f64 * 100.0) as i32
                );
            }
            self.compact_conversation()?;
        }

        let session = match self.current_session {
            Some(ref mut s) => s,
            None => return Ok(()),
        };

        let provider_name = session.llm_provider.clone();

        // Build messages for the provider
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "/".to_string());

        // Build a summary of previously executed tools
        let tool_messages: Vec<_> = session
            .messages
            .iter()
            .filter(|m| m.tools_executed && m.role == "system")
            .collect();

        let mut tool_summary = String::new();
        let mut first_summary = true;
        for m in &tool_messages {
            let mut tools_in_msg = String::new();
            let mut first_tool = true;
            for line in m.content.lines().filter(|line| line.starts_with("[Tool ")) {
                if !first_tool {
                    tools_in_msg.push_str(", ");
                }
                tools_in_msg.push_str(line);
                first_tool = false;
            }
            if !tools_in_msg.is_empty() {
                if !first_summary {
                    tool_summary.push_str("; ");
                }
                tool_summary.push_str(&tools_in_msg);
                first_summary = false;
            }
        }

        let context_note = if !tool_summary.is_empty() {
            format!("\n\nNote: You have already executed these tools in this conversation: {}. You have access to their results in your context, so you don't need to re-run them.", tool_summary)
        } else {
            String::new()
        };

        // Build system prompt
        let system_prompt = format!(
            "You are a helpful assistant with access to tools for reading files, editing code, and searching the codebase.\n\nONLY use tools when the user explicitly asks you to work with files or code. Do NOT use tools for casual conversation.\n\nCurrent working directory: {}{}",
            cwd, context_note
        );

        // Convert session messages to ProviderMessage format
        let mut messages: Vec<ProviderMessage> = vec![ProviderMessage {
            role: "system".to_string(),
            content: system_prompt,
        }];

        // Add conversation messages, filtering based on provider behavior
        let total_messages = session.messages.len();
        let filtered_messages: Vec<_> = session
            .messages
            .iter()
            .filter(|m| {
                // For Bedrock: don't filter tools_executed (model needs context)
                // For others: filter out already-executed tools
                if provider_name == "bedrock" {
                    m.role != "system" && !m.content.trim().is_empty()
                } else {
                    !m.tools_executed
                }
            })
            .collect();

        crate::debug_log!(
            "DEBUG send_llm_message: Total messages: {}, After filtering: {}",
            total_messages,
            filtered_messages.len()
        );

        messages.extend(filtered_messages.iter().map(|m| ProviderMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        }));

        // Get tool definitions
        let tools = Some(crate::provider::get_tool_definitions());

        // Create provider and start chat based on provider name
        match provider_name.as_str() {
            "bedrock" => {
                let provider = crate::provider::BedrockProvider::new();
                let model_id = session
                    .model
                    .clone()
                    .unwrap_or_else(|| self.config.bedrock_model.clone());

                if let Ok(receiver) = provider.chat(&model_id, messages, tools, 4096) {
                    self.response_receiver = Some(receiver);
                    self.waiting_for_response = true;
                } else {
                    session.add_message(
                        "system".to_string(),
                        "Error: Failed to start Bedrock chat".to_string(),
                        None,
                    );
                }
            }
            "claude" => {
                let api_key = self.config.claude_api_key.clone().unwrap_or_default();
                if api_key.is_empty() {
                    session.add_message(
                        "system".to_string(),
                        "Error: Claude API key not configured. Add it to ~/.config/llm-tui/config.toml".to_string(),
                        None,
                    );
                } else {
                    let provider = crate::provider::ClaudeProvider::new(api_key);

                    if let Ok(receiver) =
                        provider.chat(&self.config.claude_model, messages, tools, 4096)
                    {
                        self.response_receiver = Some(receiver);
                        self.waiting_for_response = true;
                    }
                }
            }
            "ollama" | _ => {
                if let Ok(receiver) =
                    self.ollama
                        .chat(&self.config.ollama_model, messages, tools, 4096)
                {
                    self.response_receiver = Some(receiver);
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

        let provider_name = session.llm_provider.clone();

        // Build tool result messages
        let tool_results: Vec<String> = self
            .pending_tool_results
            .iter()
            .map(|(name, result)| format!("[Tool {} result]:\n{}", name, result))
            .collect();

        crate::debug_log!(
            "DEBUG: Sending {} tool results back to model",
            tool_results.len()
        );

        // Convert tool results to ToolResult format for the provider
        let tool_result_structs: Vec<crate::provider::ToolResult> = self
            .pending_tool_results
            .iter()
            .map(|(name, result)| crate::provider::ToolResult {
                tool_use_id: name.clone(),
                content: result.clone(),
            })
            .collect();

        // Build messages for continuation
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "/".to_string());

        let system_prompt = format!(
            "You are a helpful assistant with access to tools for reading files, editing code, and searching the codebase.\n\nONLY use tools when the user explicitly asks you to work with files or code. Do NOT use tools for casual conversation.\n\nCurrent working directory: {}",
            cwd
        );

        let mut messages: Vec<ProviderMessage> = vec![ProviderMessage {
            role: "system".to_string(),
            content: system_prompt,
        }];

        // Add conversation messages
        let total_messages = session.messages.len();
        let filtered_messages: Vec<_> = session
            .messages
            .iter()
            .filter(|m| {
                if provider_name == "bedrock" {
                    m.role != "system" && !m.content.trim().is_empty()
                } else {
                    !m.tools_executed
                }
            })
            .collect();

        crate::debug_log!(
            "DEBUG continue_with_tool_results: Total messages: {}, After filtering: {}",
            total_messages,
            filtered_messages.len()
        );

        messages.extend(filtered_messages.iter().map(|m| ProviderMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        }));

        // Clear pending results since we're sending them now
        self.pending_tool_results.clear();

        // Get tool definitions
        let tools = Some(crate::provider::get_tool_definitions());

        // Continue conversation with tool results
        match provider_name.as_str() {
            "bedrock" => {
                let provider = crate::provider::BedrockProvider::new();
                let model_id = session
                    .model
                    .clone()
                    .unwrap_or_else(|| self.config.bedrock_model.clone());

                if let Ok(receiver) = provider.continue_with_tools(
                    &model_id,
                    messages,
                    tools,
                    tool_result_structs,
                    4096,
                ) {
                    self.response_receiver = Some(receiver);
                }
            }
            "claude" => {
                let api_key = self.config.claude_api_key.clone().unwrap_or_default();
                if !api_key.is_empty() {
                    let provider = crate::provider::ClaudeProvider::new(api_key);

                    if let Ok(receiver) = provider.continue_with_tools(
                        &self.config.claude_model,
                        messages,
                        tools,
                        tool_result_structs,
                        4096,
                    ) {
                        self.response_receiver = Some(receiver);
                    }
                } else {
                    self.waiting_for_response = false;
                    self.response_receiver = None;
                }
            }
            "ollama" | _ => {
                if let Ok(receiver) = self.ollama.continue_with_tools(
                    &self.config.ollama_model,
                    messages,
                    tools,
                    tool_result_structs,
                    4096,
                ) {
                    self.response_receiver = Some(receiver);
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
                        if let Ok(models) = self.ollama.list_ollama_models() {
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
                self.response_receiver = None;
                Ok(false)
            }
            _ => Ok(false), // Ignore other keys while waiting for confirmation
        }
    }

    fn handle_normal_mode(&mut self, key: KeyEvent) -> Result<bool> {
        // If on help screen, any key returns to session list
        if self.screen == AppScreen::Help {
            self.screen = AppScreen::SessionList;
            return Ok(false);
        }

        // Handle setup wizard navigation
        if self.screen == AppScreen::Setup {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    self.advance_setup_step();
                }
                KeyCode::Char('n') if self.setup_step == 0 => {
                    // Skip entire wizard from welcome screen
                    self.screen = AppScreen::SessionList;
                }
                KeyCode::Char('n') | KeyCode::Char('s') if self.setup_step > 0 => {
                    // Skip current step and advance to next
                    self.advance_setup_step();
                }
                KeyCode::Char('q') => {
                    // Quit wizard and return to session list
                    self.screen = AppScreen::SessionList;
                }
                _ => {}
            }
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('q') => return Ok(true), // Quit
            KeyCode::Char('?') => {
                self.screen = AppScreen::Help;
            }
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
                if let Ok(models) = self.ollama.list_ollama_models() {
                    self.models = models;
                }
                self.refresh_provider_models();
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
                } else if self.screen == AppScreen::SessionList
                    && !self.session_tree.items.is_empty()
                {
                    self.session_nav.selected_index = (self.session_nav.selected_index + 1)
                        .min(self.session_tree.items.len() - 1);
                } else if self.screen == AppScreen::Models && !self.provider_models.is_empty() {
                    // Count total items (models + headers)
                    let mut total_items = self.provider_models.len();
                    let mut prev_provider = "";
                    for model in &self.provider_models {
                        if model.provider != prev_provider {
                            prev_provider = &model.provider;
                            total_items += 1; // Add 1 for each provider header
                        }
                    }
                    self.model_nav.selected_index =
                        (self.model_nav.selected_index + 1).min(total_items - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.screen == AppScreen::Chat {
                    // Scroll up (decrease scroll offset)
                    self.message_scroll = self.message_scroll.saturating_sub(1);
                    self.message_scroll_manual = true; // User is manually scrolling
                } else if self.screen == AppScreen::SessionList {
                    self.session_nav.selected_index =
                        self.session_nav.selected_index.saturating_sub(1);
                } else if self.screen == AppScreen::Models {
                    self.model_nav.selected_index = self.model_nav.selected_index.saturating_sub(1);
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
                } else if self.screen == AppScreen::SessionList
                    && !self.session_tree.items.is_empty()
                {
                    self.session_nav.selected_index = self.session_tree.items.len() - 1;
                }
            }
            KeyCode::Char('n') => {
                if self.screen == AppScreen::SessionList && !self.session_tree.items.is_empty() {
                    // Get parent project of currently selected item
                    let project = self
                        .session_tree
                        .get_parent_project(self.session_nav.selected_index);
                    let model = match self.config.default_llm_provider.as_str() {
                        "claude" => Some(self.config.claude_model.clone()),
                        "bedrock" => Some(self.config.bedrock_model.clone()),
                        _ => Some(self.config.ollama_model.clone()),
                    };
                    let session = Session::new(
                        None,
                        project,
                        self.config.default_llm_provider.clone(),
                        model,
                    );
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
                            if self.session_nav.selected_index >= self.session_tree.items.len()
                                && self.session_tree.items.len() > 0
                            {
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
                            if let Ok(session_files) =
                                db::load_session_files(&self.conn, &session.id)
                            {
                                for file in session_files {
                                    // Check if file has changed on disk
                                    let current_content = if db::should_reload_file(
                                        &file.file_path,
                                        &file.content_hash,
                                    )
                                    .unwrap_or(true)
                                    {
                                        // File changed or missing, use cached content but re-read if possible
                                        match std::fs::read_to_string(&file.file_path) {
                                            Ok(new_content) => {
                                                crate::debug_log!(
                                                    "File {} changed, using updated content",
                                                    file.file_path
                                                );
                                                new_content
                                            }
                                            Err(_) => {
                                                crate::debug_log!(
                                                    "File {} not found, using cached content",
                                                    file.file_path
                                                );
                                                file.content
                                            }
                                        }
                                    } else {
                                        // File unchanged, use cached content
                                        file.content
                                    };

                                    // Add file contents as system message for context
                                    let content = format!(
                                        "[File: {}]\n\n{}",
                                        file.file_path, current_content
                                    );
                                    let token_count =
                                        Some(crate::session::estimate_tokens(&content));
                                    let context_message = crate::session::Message {
                                        role: "system".to_string(),
                                        content,
                                        timestamp: chrono::Utc::now(),
                                        model: None,
                                        tools_executed: false,
                                        is_summary: false,
                                        token_count,
                                    };
                                    session.messages.push(context_message);
                                }
                            }

                            self.current_session = Some(session);
                            self.screen = AppScreen::Chat;
                        }
                    }
                } else if self.screen == AppScreen::Models && !self.provider_models.is_empty() {
                    // Map visual index (with headers) to provider_models index
                    let mut visual_index = 0;
                    let mut model_index = None;
                    let mut current_provider = "";

                    for (i, model) in self.provider_models.iter().enumerate() {
                        // Account for provider headers
                        if model.provider != current_provider {
                            current_provider = &model.provider;
                            visual_index += 1; // Header takes up a slot
                        }

                        if visual_index == self.model_nav.selected_index {
                            model_index = Some(i);
                            break;
                        }
                        visual_index += 1;
                    }

                    // Only process if we found a valid model (not a header)
                    if let Some(model_index) = model_index {
                        // Clone the selected model data before any mutations
                        let selected_provider = self.provider_models[model_index].provider.clone();
                        let selected_model_id = self.provider_models[model_index].model_id.clone();
                        let is_installed = self.provider_models[model_index].installed;

                        // If Ollama model is not installed, pull it instead of switching
                        if selected_provider == "ollama" && !is_installed {
                            self.pull_status =
                                Some(format!("Starting download: {}", selected_model_id));
                            if let Ok(receiver) = self.ollama.pull_model(&selected_model_id) {
                                self.pull_receiver = Some(receiver);
                            }
                            return Ok(false);
                        }

                        // Update config based on provider (always do this)
                        self.config.default_llm_provider = selected_provider.clone();
                        match selected_provider.as_str() {
                            "ollama" => self.config.ollama_model = selected_model_id.clone(),
                            "claude" => self.config.claude_model = selected_model_id.clone(),
                            "bedrock" => self.config.bedrock_model = selected_model_id.clone(),
                            _ => {}
                        }
                        let _ = self.config.save();

                        // Update [current] indicator (lightweight, no API calls)
                        self.update_current_model_flags();

                        // Update session provider and model if we have one
                        if let Some(ref mut session) = self.current_session {
                            // Unload Ollama model if switching away from Ollama
                            if session.llm_provider == "ollama" && selected_provider != "ollama" {
                                let _ = self.ollama.unload_model(&self.config.ollama_model);
                            }

                            session.llm_provider = selected_provider.clone();
                            session.model = Some(selected_model_id.clone());
                            let _ = db::save_session(&self.conn, session);

                            // Clear any active receiver
                            self.response_receiver = None;
                            self.waiting_for_response = false;

                            session.add_message(
                                "system".to_string(),
                                format!(
                                    "Switched to {} - {}",
                                    selected_provider, selected_model_id
                                ),
                                None,
                            );
                        }
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

        if cmd == "compact" {
            self.compact_conversation()?;
            return Ok(false);
        }

        if cmd == "setup" {
            self.start_setup_wizard();
            return Ok(false);
        }

        // :provider <name> - switch LLM provider for current session
        if cmd.starts_with("provider") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 1 {
                let provider = parts[1].to_lowercase();
                if provider == "claude" || provider == "ollama" || provider == "bedrock" {
                    if let Some(ref mut session) = self.current_session {
                        crate::debug_log!(
                            "DEBUG: Changing provider from '{}' to '{}'",
                            session.llm_provider,
                            provider
                        );

                        // Unload Ollama model if switching away from Ollama
                        if session.llm_provider == "ollama" && provider != "ollama" {
                            let _ = self.ollama.unload_model(&self.config.ollama_model);
                        }

                        session.llm_provider = provider.clone();

                        // Update session model to match provider
                        session.model = Some(match provider.as_str() {
                            "bedrock" => self.config.bedrock_model.clone(),
                            "claude" => self.config.claude_model.clone(),
                            _ => self.config.ollama_model.clone(),
                        });

                        let _ = db::save_session(&self.conn, session);

                        // Clear any active receiver from previous provider
                        self.response_receiver = None;
                        self.waiting_for_response = false;

                        session.add_message(
                            "system".to_string(),
                            format!(
                                "Provider switched to: {} ({})",
                                provider,
                                session.model.as_ref().unwrap()
                            ),
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

            let model = match self.config.default_llm_provider.as_str() {
                "claude" => Some(self.config.claude_model.clone()),
                "bedrock" => Some(self.config.bedrock_model.clone()),
                _ => Some(self.config.ollama_model.clone()),
            };
            let session = Session::new(
                name,
                self.current_project.clone(),
                self.config.default_llm_provider.clone(),
                model,
            );
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
                let model = match self.config.default_llm_provider.as_str() {
                    "claude" => Some(self.config.claude_model.clone()),
                    "bedrock" => Some(self.config.bedrock_model.clone()),
                    _ => Some(self.config.ollama_model.clone()),
                };
                let session = Session::new(
                    None,
                    Some(project_name),
                    self.config.default_llm_provider.clone(),
                    model,
                );
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

        // :new project <name> - create/switch to project (doesn't create session)
        if cmd.starts_with("new project") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 2 {
                let project_name = parts[2..].join(" ");
                self.current_project = Some(project_name);
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

            let model = match self.config.default_llm_provider.as_str() {
                "claude" => Some(self.config.claude_model.clone()),
                "bedrock" => Some(self.config.bedrock_model.clone()),
                _ => Some(self.config.ollama_model.clone()),
            };
            let session = Session::new(
                name,
                project,
                self.config.default_llm_provider.clone(),
                model,
            );
            db::save_session(&self.conn, &session)?;
            self.current_session = Some(session);
            self.screen = AppScreen::Chat;
            self.sessions = db::list_sessions(&self.conn)?;
            self.rebuild_tree();
            return Ok(false);
        }

        if cmd == "models" {
            self.screen = AppScreen::Models;
            if let Ok(models) = self.ollama.list_ollama_models() {
                self.models = models;
            }
            self.refresh_provider_models();
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
                if let Ok(models) = self.ollama.list_ollama_models() {
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
                    let mut found_session = self
                        .sessions
                        .iter()
                        .find(|s| Some(s.id.as_str()) != current_id && s.id == target);

                    // If no exact ID, try exact name match (case insensitive)
                    if found_session.is_none() {
                        found_session = self.sessions.iter().find(|s| {
                            Some(s.id.as_str()) != current_id
                                && s.name
                                    .as_ref()
                                    .map(|n| n.to_lowercase() == target.to_lowercase())
                                    .unwrap_or(false)
                        });
                    }

                    // If still no match, try partial name match (contains)
                    if found_session.is_none() {
                        found_session = self.sessions.iter().find(|s| {
                            Some(s.id.as_str()) != current_id
                                && s.name
                                    .as_ref()
                                    .map(|n| n.to_lowercase().contains(&target.to_lowercase()))
                                    .unwrap_or(false)
                        });
                    }

                    if let Some(found_session) = found_session {
                        if let Ok(messages) = db::load_messages(&self.conn, &found_session.id) {
                            if let Some(ref mut session) = self.current_session {
                                // Format all messages from the loaded session
                                let mut context = String::new();
                                for (i, m) in messages.iter().enumerate() {
                                    if i > 0 {
                                        context.push_str("\n\n");
                                    }
                                    context.push_str(&format!("[{}]: {}", m.role, m.content));
                                }

                                session.add_message(
                                    "system".to_string(),
                                    format!(
                                        "Context loaded from session '{}':\n\n{}",
                                        found_session.display_name(),
                                        context
                                    ),
                                    None,
                                );

                                match self.config.autosave_mode {
                                    AutosaveMode::OnSend => {
                                        if let Some(last_msg) = session.messages.last() {
                                            let _ =
                                                db::save_message(&self.conn, &session.id, last_msg);
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

    pub fn start_setup_wizard(&mut self) {
        self.screen = AppScreen::Setup;
        self.setup_step = 0;
        self.setup_message.clear();
        self.ollama_status = None;
        self.claude_status = None;
        self.bedrock_status = None;
    }

    pub fn advance_setup_step(&mut self) {
        match self.setup_step {
            0 => {
                // Welcome -> Check Ollama
                self.setup_step = 1;
                self.check_ollama_status();
            }
            1 => {
                // Ollama -> Check Claude
                self.setup_step = 2;
                self.check_claude_status();
            }
            2 => {
                // Claude -> Check Bedrock
                self.setup_step = 3;
                self.check_bedrock_status();
            }
            3 => {
                // Bedrock -> Complete
                self.setup_step = 4;
            }
            4 => {
                // Complete -> Exit to session list
                self.screen = AppScreen::SessionList;
                self.setup_step = 0;
            }
            _ => {}
        }
    }

    fn check_ollama_status(&mut self) {
        // Try to connect to Ollama
        let client = reqwest::blocking::Client::new();
        match client
            .get(&format!("{}/api/tags", self.config.ollama_url))
            .send()
        {
            Ok(resp) if resp.status().is_success() => {
                self.ollama_status = Some(true);
                self.setup_message = format!("✓ Connected to Ollama at {}", self.config.ollama_url);
            }
            _ => {
                self.ollama_status = Some(false);
                self.setup_message = format!(
                    "✗ Could not connect to Ollama at {}",
                    self.config.ollama_url
                );
            }
        }
    }

    fn check_claude_status(&mut self) {
        // Check for ANTHROPIC_API_KEY env var
        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            if !api_key.is_empty() {
                self.claude_status = Some(true);
                self.setup_message = "✓ ANTHROPIC_API_KEY found in environment".to_string();
            } else {
                self.claude_status = Some(false);
                self.setup_message = "✗ ANTHROPIC_API_KEY is empty".to_string();
            }
        } else if self.config.claude_api_key.is_some() {
            self.claude_status = Some(true);
            self.setup_message = "✓ Claude API key found in config".to_string();
        } else {
            self.claude_status = Some(false);
            self.setup_message = "✗ No Claude API key configured".to_string();
        }
    }

    fn check_bedrock_status(&mut self) {
        // Try to detect AWS credentials
        // Check environment variables first
        if std::env::var("AWS_ACCESS_KEY_ID").is_ok()
            && std::env::var("AWS_SECRET_ACCESS_KEY").is_ok()
        {
            self.bedrock_status = Some(true);
            self.setup_message = "✓ AWS credentials found in environment variables".to_string();
            return;
        }

        // Check for AWS credentials file
        if let Ok(home) = std::env::var("HOME") {
            let creds_path = std::path::PathBuf::from(home)
                .join(".aws")
                .join("credentials");

            if creds_path.exists() {
                // Check file permissions
                if let Ok(metadata) = std::fs::metadata(&creds_path) {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mode = metadata.permissions().mode();
                        let perms = mode & 0o777;

                        if perms == 0o600 {
                            self.bedrock_status = Some(true);
                            self.setup_message =
                                "✓ AWS credentials file found (secure permissions: 0600)"
                                    .to_string();
                        } else {
                            self.bedrock_status = Some(false);
                            self.setup_message = format!("⚠ AWS credentials file found but permissions are {:o} (should be 0600)", perms);
                        }
                        return;
                    }

                    #[cfg(not(unix))]
                    {
                        self.bedrock_status = Some(true);
                        self.setup_message = "✓ AWS credentials file found".to_string();
                        return;
                    }
                }
            }
        }

        self.bedrock_status = Some(false);
        self.setup_message = "✗ No AWS credentials found".to_string();
    }
}
