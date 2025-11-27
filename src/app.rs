use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::session::{list_sessions, Session};

#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    SessionList,
    Chat,
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
}

impl App {
    pub fn new() -> Result<Self> {
        let sessions = list_sessions()?;
        let selected_index = if sessions.is_empty() { 0 } else { 0 };

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
        })
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
            KeyCode::Char('i') if self.screen == AppScreen::Chat => {
                self.mode = InputMode::Insert;
            }
            KeyCode::Enter if self.screen == AppScreen::Chat => {
                // Send message in normal mode
                if !self.message_buffer.is_empty() {
                    // TODO: Send message to LLM
                    self.message_buffer.clear();
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.screen == AppScreen::SessionList && !self.sessions.is_empty() {
                    self.selected_session_index =
                        (self.selected_session_index + 1).min(self.sessions.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.screen == AppScreen::SessionList {
                    self.selected_session_index = self.selected_session_index.saturating_sub(1);
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
                    let session = self.sessions[self.selected_session_index].clone();
                    self.current_session = Some(session);
                    self.screen = AppScreen::Chat;
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
                if !self.message_buffer.is_empty() {
                    // TODO: Send message to LLM
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
                session.save()?;
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

            let session = Session::new(name, self.current_project.clone());
            session.save()?;
            self.current_session = Some(session);
            self.screen = AppScreen::Chat;
            self.sessions = list_sessions()?;
            return Ok(false);
        }

        if cmd.starts_with("project") {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() > 1 {
                self.current_project = Some(parts[1].to_string());
            }
            return Ok(false);
        }

        Ok(false)
    }
}
