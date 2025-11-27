use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rusqlite::Connection;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::config::{AutosaveMode, Config};
use crate::db;
use crate::ollama::{ChatMessage, LlmEvent, OllamaClient};
use crate::session::Session;

#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    SessionList,
    Chat,
    Models,
    Browser,
    Settings,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    Command,
    Insert,
}

pub struct App {
    pub screen: AppScreen,
    pub mode: InputMode,
    pub sessions: Vec<Session>,
    pub selected_session_index: usize,
    pub current_session: Option<Session>,
    pub command_buffer: String,
    pub message_buffer: String,
    pub current_project: Option<String>,
    pub input_scroll: u16,
    pub conn: Connection,
    pub config: Config,
    pub last_autosave: Instant,
    pub needs_save: bool,
    pub ollama: OllamaClient,
    pub llm_receiver: Option<Receiver<LlmEvent>>,
    pub waiting_for_response: bool,
    pub assistant_buffer: String,
    pub models: Vec<crate::ollama::OllamaModel>,
    pub selected_model_index: usize,
    pub pull_status: Option<String>,
    pub pull_receiver: Option<Receiver<String>>,
    pub browse_models: Vec<crate::ollama::OllamaModel>,
    pub selected_browse_index: usize,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = Config::load()?;
        let conn = db::init_db()?;
        let sessions = db::list_sessions(&conn)?;
        let selected_index = if sessions.is_empty() { 0 } else { 0 };

        let mut ollama = OllamaClient::new(config.ollama_url.clone());

        // Auto-start Ollama if configured
        if config.ollama_auto_start {
            let _ = ollama.start_server();
        }

        Ok(Self {
            screen: AppScreen::SessionList,
            mode: InputMode::Normal,
            sessions,
            selected_session_index: selected_index,
            current_session: None,
            command_buffer: String::new(),
            message_buffer: String::new(),
            current_project: None,
            input_scroll: 0,
            conn,
            config,
            last_autosave: Instant::now(),
            needs_save: false,
            ollama,
            llm_receiver: None,
            waiting_for_response: false,
            assistant_buffer: String::new(),
            models: Vec::new(),
            selected_model_index: 0,
            pull_status: None,
            pull_receiver: None,
            browse_models: Vec::new(),
            selected_browse_index: 0,
        })
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
                    self.assistant_buffer.push_str(&token);
                }
                Ok(LlmEvent::Done) => {
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
                Ok(LlmEvent::Error(err)) => {
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
                }
                Err(_) => {} // No message available yet
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
        match self.mode {
            InputMode::Normal => self.handle_normal_mode(key),
            InputMode::Command => self.handle_command_mode(key),
            InputMode::Insert => self.handle_insert_mode(key),
        }
    }

    fn handle_normal_mode(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => return Ok(true), // Quit
            KeyCode::Char(':') => {
                self.mode = InputMode::Command;
                self.command_buffer.clear();
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
                self.mode = InputMode::Insert;
            }
            KeyCode::Enter if self.screen == AppScreen::Chat => {
                // Send message in normal mode
                if !self.message_buffer.is_empty() && !self.waiting_for_response {
                    if let Some(ref mut session) = self.current_session {
                        session.add_message("user".to_string(), self.message_buffer.clone(), None);

                        // Convert session messages to chat format
                        let mut messages: Vec<ChatMessage> = vec![ChatMessage {
                            role: "system".to_string(),
                            content: "You are a helpful assistant. Respond directly to the user's message. Do not generate both sides of a conversation.".to_string(),
                        }];

                        messages.extend(session
                            .messages
                            .iter()
                            .map(|m| ChatMessage {
                                role: m.role.clone(),
                                content: m.content.clone(),
                            }));

                        // Start LLM chat
                        if let Ok(receiver) = self.ollama.chat(&self.config.ollama_model, messages) {
                            self.llm_receiver = Some(receiver);
                            self.waiting_for_response = true;
                        }

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
                if self.screen == AppScreen::SessionList && !self.sessions.is_empty() {
                    self.selected_session_index =
                        (self.selected_session_index + 1).min(self.sessions.len() - 1);
                } else if self.screen == AppScreen::Models && !self.models.is_empty() {
                    self.selected_model_index =
                        (self.selected_model_index + 1).min(self.models.len() - 1);
                } else if self.screen == AppScreen::Browser && !self.browse_models.is_empty() {
                    self.selected_browse_index =
                        (self.selected_browse_index + 1).min(self.browse_models.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.screen == AppScreen::SessionList {
                    self.selected_session_index = self.selected_session_index.saturating_sub(1);
                } else if self.screen == AppScreen::Models {
                    self.selected_model_index = self.selected_model_index.saturating_sub(1);
                } else if self.screen == AppScreen::Browser {
                    self.selected_browse_index = self.selected_browse_index.saturating_sub(1);
                }
            }
            KeyCode::Char('g') => {
                if self.screen == AppScreen::SessionList {
                    self.selected_session_index = 0;
                }
            }
            KeyCode::Char('G') => {
                if self.screen == AppScreen::SessionList && !self.sessions.is_empty() {
                    self.selected_session_index = self.sessions.len() - 1;
                }
            }
            KeyCode::Enter => {
                if self.screen == AppScreen::SessionList && !self.sessions.is_empty() {
                    let mut session = self.sessions[self.selected_session_index].clone();
                    if let Ok(messages) = db::load_messages(&self.conn, &session.id) {
                        session.messages = messages;
                    }
                    self.current_session = Some(session);
                    self.screen = AppScreen::Chat;
                } else if self.screen == AppScreen::Models && !self.models.is_empty() {
                    // Select model and update config
                    let model_name = self.models[self.selected_model_index].name.clone();
                    self.config.ollama_model = model_name;
                    let _ = self.config.save();
                } else if self.screen == AppScreen::Browser && !self.browse_models.is_empty() {
                    // Pull model from browse list
                    let model_name = self.browse_models[self.selected_browse_index].name.clone();
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
                self.mode = InputMode::Normal;
                self.command_buffer.clear();
            }
            KeyCode::Enter => {
                let should_quit = self.execute_command()?;
                self.mode = InputMode::Normal;
                self.command_buffer.clear();
                if should_quit {
                    return Ok(true);
                }
            }
            KeyCode::Backspace => {
                self.command_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.command_buffer.push(c);
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_insert_mode(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.mode = InputMode::Normal;
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

                        // Convert session messages to chat format
                        let mut messages: Vec<ChatMessage> = vec![ChatMessage {
                            role: "system".to_string(),
                            content: "You are a helpful assistant. Respond directly to the user's message. Do not generate both sides of a conversation.".to_string(),
                        }];

                        messages.extend(session
                            .messages
                            .iter()
                            .map(|m| ChatMessage {
                                role: m.role.clone(),
                                content: m.content.clone(),
                            }));

                        // Start LLM chat
                        if let Ok(receiver) = self.ollama.chat(&self.config.ollama_model, messages) {
                            self.llm_receiver = Some(receiver);
                            self.waiting_for_response = true;
                        }

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
        let cmd = self.command_buffer.trim();

        if cmd == "q" || cmd == "quit" {
            return Ok(true);
        }

        if cmd == "w" || cmd == "save" {
            if let Some(ref session) = self.current_session {
                db::save_session(&self.conn, session)?;
            }
            return Ok(false);
        }

        if cmd.starts_with("new") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            let name = if parts.len() > 1 {
                Some(parts[1..].join(" "))
            } else {
                None
            };

            let session = Session::new(name, self.current_project.clone(), Some(self.config.ollama_model.clone()));
            db::save_session(&self.conn, &session)?;
            self.current_session = Some(session);
            self.screen = AppScreen::Chat;
            self.sessions = db::list_sessions(&self.conn)?;
            return Ok(false);
        }

        if cmd.starts_with("project") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 1 {
                self.current_project = Some(parts[1].to_string());
            }
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

        Ok(false)
    }
}
