use crate::app::{App, AppScreen, InputMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    match app.screen {
        AppScreen::SessionList => draw_session_list(f, app),
        AppScreen::Chat => draw_chat(f, app),
        AppScreen::Settings => draw_settings(f, app),
    }
}

fn draw_session_list(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(1),     // Session list
            Constraint::Length(3),  // Footer with keybinds
            Constraint::Length(1),  // Command line
        ])
        .split(f.area());

    // Header
    let title = if let Some(ref project) = app.current_project {
        format!("LLM TUI - Project: {}", project)
    } else {
        "LLM TUI - Sessions".to_string()
    };
    let header = Paragraph::new(title)
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Session list
    if app.sessions.is_empty() {
        let empty_msg = Paragraph::new(vec![
            Line::from("No sessions found."),
            Line::from(""),
            Line::from("Use :new [name] to create a new session."),
            Line::from("Use :project <name> to set the current project."),
        ])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Sessions"));
        f.render_widget(empty_msg, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let display = if let Some(ref project) = session.project {
                    format!(
                        "[{}] {} - {}",
                        project,
                        session.display_name(),
                        session.updated_at.format("%Y-%m-%d %H:%M")
                    )
                } else {
                    format!(
                        "{} - {}",
                        session.display_name(),
                        session.updated_at.format("%Y-%m-%d %H:%M")
                    )
                };

                let style = if i == app.selected_session_index {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                ListItem::new(display).style(style)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Sessions"));
        f.render_widget(list, chunks[1]);
    }

    // Footer with keybinds
    let footer_text = if app.mode == InputMode::Command {
        "Command mode".to_string()
    } else {
        "j/k: navigate | Enter: open | :new [name]: new session | :project <name>: set project | 1: sessions | q: quit".to_string()
    };
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);

    // Command line
    let cmd_line = if app.mode == InputMode::Command {
        Paragraph::new(format!(":{}", app.command_buffer))
            .style(Style::default().fg(Color::Green))
    } else {
        Paragraph::new("")
    };
    f.render_widget(cmd_line, chunks[3]);
}

fn draw_chat(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),   // Header
            Constraint::Min(1),      // Chat messages
            Constraint::Length(10),  // Input area (larger for multiline)
            Constraint::Length(3),   // Footer with keybinds
            Constraint::Length(1),   // Command line
        ])
        .split(f.area());

    // Header
    let session_name = app
        .current_session
        .as_ref()
        .map(|s| s.display_name())
        .unwrap_or_else(|| "No Session".to_string());
    let header = Paragraph::new(format!("Chat: {}", session_name))
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Messages
    let mut messages_text = if let Some(ref session) = app.current_session {
        if session.messages.is_empty() {
            vec![Line::from("No messages yet. Press 'i' to start typing.")]
        } else {
            session
                .messages
                .iter()
                .map(|msg| {
                    Line::from(vec![
                        Span::styled(
                            format!("[{}] ", msg.role),
                            Style::default().fg(Color::Yellow),
                        ),
                        Span::raw(&msg.content),
                    ])
                })
                .collect()
        }
    } else {
        vec![Line::from("No session loaded.")]
    };

    // Show assistant's streaming response if waiting
    if app.waiting_for_response && !app.assistant_buffer.is_empty() {
        messages_text.push(Line::from(vec![
            Span::styled(
                "[assistant] ",
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(&app.assistant_buffer),
            Span::styled(" ●", Style::default().fg(Color::Green)),
        ]));
    } else if app.waiting_for_response {
        messages_text.push(Line::from(vec![
            Span::styled(
                "[assistant] ",
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("Thinking...", Style::default().fg(Color::Gray)),
        ]));
    }

    let messages = Paragraph::new(messages_text)
        .block(Block::default().borders(Borders::ALL).title("Messages"))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(messages, chunks[1]);

    // Input area
    let input_title = if app.mode == InputMode::Insert {
        "Input (INSERT)"
    } else {
        "Input (press 'i' to start typing)"
    };

    // Split input into lines for scrolling
    let input_lines: Vec<Line> = app.message_buffer
        .lines()
        .map(|line| Line::from(line.to_string()))
        .collect();

    let input = Paragraph::new(input_lines)
        .block(Block::default().borders(Borders::ALL).title(input_title))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((app.input_scroll, 0));
    f.render_widget(input, chunks[2]);

    // Footer with keybinds
    let footer_text = if app.mode == InputMode::Command {
        "Command mode".to_string()
    } else if app.mode == InputMode::Insert {
        "INSERT mode | Esc: normal mode | Enter: newline | Ctrl+Space: send".to_string()
    } else {
        "i: insert mode | Enter: send message | 1: sessions | 2: chat | :w: save | :q: quit".to_string()
    };
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[3]);

    // Command line
    let cmd_line = if app.mode == InputMode::Command {
        Paragraph::new(format!(":{}", app.command_buffer))
            .style(Style::default().fg(Color::Green))
    } else {
        Paragraph::new("")
    };
    f.render_widget(cmd_line, chunks[4]);
}

fn draw_settings(f: &mut Frame, _app: &App) {
    let block = Block::default()
        .title("Settings (TODO)")
        .borders(Borders::ALL);
    f.render_widget(block, f.area());
}
