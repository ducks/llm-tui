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
    pub tool_status: Option<String>,
    pub pending_tool_results: Vec<(String, String)>, // (tool_name, result)
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
            tool_status: None,
            pending_tool_results: Vec::new(),
        })
    }

    pub fn rebuild_tree(&mut self) {
        self.session_tree.build_from_sessions(self.sessions.clone());
    }

    pub fn update_message_scroll(&mut self, visible_height: u16) {
        // Always auto-scroll to bottom when content arrives
        // User can scroll up manually with j/k if they want to read history

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

            // Scroll to show the bottom (add padding to ensure we see everything)
            self.message_scroll = total_lines.saturating_sub(visible_height).saturating_add(10);
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
                    self.tool_status = Some(format!("Using tool: {}", name));

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
                    self.tool_status = None;
                }
                Ok(LlmEvent::Done) => {
                    crate::debug_log!("DEBUG: Received Done event, pending_tool_results: {}", self.pending_tool_results.len());

                    // If we have pending tool results, send them back to continue the conversation
                    if !self.pending_tool_results.is_empty() {
                        crate::debug_log!("DEBUG: Continuing conversation with tool results");
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
                    self.assistant_buffer.push_str(&text);
                }
                Ok(ClaudeEvent::ToolUse { id, name, input }) => {
                    self.tool_status = Some(format!("Using tool: {}", name));

                    // Execute tool
                    let result = self.execute_tool(&name, input);

                    // For now, just add tool result to assistant buffer
                    // In full implementation, we'd send result back to Claude and continue
                    match result {
                        Ok(output) => {
                            self.assistant_buffer.push_str(&format!(
                                "\n\n─────────────────────────────────────────\n[Tool: {}]\n─────────────────────────────────────────\n{}\n─────────────────────────────────────────\n",
                                name,
                                output
                            ));
                        }
                        Err(e) => {
                            self.assistant_buffer.push_str(&format!(
                                "\n\n─────────────────────────────────────────\n[Tool Error: {}]\n─────────────────────────────────────────\n{}\n─────────────────────────────────────────\n",
                                name,
                                e
                            ));
                        }
                    }

                    self.tool_status = None;
                }
                Ok(ClaudeEvent::Done) => {
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
                Ok(ClaudeEvent::Error(err)) => {
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
                }
                Err(_) => {} // No message available yet
            }
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

        match provider.as_str() {
            "claude" => {
                if let Some(ref claude) = self.claude {
                    // Convert messages to Claude format
                    let messages: Vec<crate::claude::Message> = session
                        .messages
                        .iter()
                        .filter(|m| m.role != "system") // Claude doesn't support system messages in messages array
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

                messages.extend(session.messages.iter().map(|m| ChatMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                }));

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
            "claude" => {
                // TODO: Implement Claude tool result continuation
                // For now, just clear and finish
                self.pending_tool_results.clear();
                self.waiting_for_response = false;
                self.llm_receiver = None;
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

                // Add all previous messages
                messages.extend(session.messages.iter().map(|m| ChatMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                }));

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
        match self.vim_nav.mode {
            InputMode::Normal => self.handle_normal_mode(key),
            InputMode::Command => self.handle_command_mode(key),
            InputMode::Insert => self.handle_insert_mode(key),
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
                    // Jump to bottom (scroll will auto-update on next render)
                    self.message_scroll = u16::MAX;
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
                if provider == "claude" || provider == "ollama" {
                    if let Some(ref mut session) = self.current_session {
                        session.llm_provider = provider.clone();
                        let _ = db::save_session(&self.conn, session);
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
