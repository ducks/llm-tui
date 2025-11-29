use crate::app::{App, AppScreen};
use vim_navigator::InputMode;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub fn draw(f: &mut Frame, app: &mut App) {
    match app.screen {
        AppScreen::SessionList => draw_session_list(f, app),
        AppScreen::Chat => draw_chat(f, app),
        AppScreen::Models => draw_models(f, app),
        AppScreen::Browser => draw_browser(f, app),
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
        format!("LLM TUI - Project: {} [Model: {}]", project, app.config.ollama_model)
    } else {
        format!("LLM TUI - Sessions [Model: {}]", app.config.ollama_model)
    };
    let header = Paragraph::new(title)
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Session tree
    if app.session_tree.items.is_empty() {
        let empty_msg = Paragraph::new(vec![
            Line::from("No sessions found."),
            Line::from(""),
            Line::from("Use :new [name] to create a new session."),
            Line::from("Use n to create session in current project."),
        ])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Sessions"));
        f.render_widget(empty_msg, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .session_tree
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                use crate::tree::TreeItem;

                let (display, style) = match item {
                    TreeItem::Project { name, expanded } => {
                        let icon = if *expanded { "▼" } else { "▶" };
                        let display = format!("{} {}", icon, name);
                        let style = if i == app.session_nav.selected_index {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Cyan)
                        };
                        (display, style)
                    }
                    TreeItem::Session { session, .. } => {
                        let model_str = session.model.as_ref().map(|m| format!(" ({})", m)).unwrap_or_default();
                        let display = format!(
                            "  {} - {}{}",
                            session.display_name(),
                            session.updated_at.format("%Y-%m-%d %H:%M"),
                            model_str
                        );
                        let style = if i == app.session_nav.selected_index {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        (display, style)
                    }
                };

                ListItem::new(display).style(style)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Sessions"));
        f.render_widget(list, chunks[1]);
    }

    // Footer with keybinds
    let footer_text = if app.vim_nav.mode == InputMode::Command {
        "Command mode".to_string()
    } else {
        "j/k: navigate | Enter: open | Space: toggle | n: new in project | d: delete | :new [name] --project <proj> | 1: sessions | q: quit".to_string()
    };
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);

    // Command line
    let cmd_line = if app.vim_nav.mode == InputMode::Command {
        Paragraph::new(format!(":{}", app.vim_nav.command_buffer))
            .style(Style::default().fg(Color::Green))
    } else {
        Paragraph::new("")
    };
    f.render_widget(cmd_line, chunks[3]);
}

fn draw_chat(f: &mut Frame, app: &mut App) {
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
    let header_text = if let Some(ref session) = app.current_session {
        let model_str = session.model.as_ref().map(|m| format!(" [{}]", m)).unwrap_or_default();
        format!("Chat: {}{}", session.display_name(), model_str)
    } else {
        "Chat: No Session".to_string()
    };
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Calculate scroll position first
    let visible_height = chunks[1].height.saturating_sub(2); // Subtract borders
    app.update_message_scroll(visible_height);

    // Messages
    let mut messages_text = if let Some(ref session) = app.current_session {
        if session.messages.is_empty() {
            vec![Line::from("No messages yet. Press 'i' to start typing.")]
        } else {
            let mut lines = Vec::new();
            for msg in &session.messages {
                // Split message content by newlines
                let content_lines: Vec<&str> = msg.content.lines().collect();

                if content_lines.is_empty() {
                    // Empty message, just show role
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("[{}] ", msg.role),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]));
                } else {
                    // First line includes the role prefix
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("[{}] ", msg.role),
                            Style::default().fg(Color::Yellow),
                        ),
                        Span::raw(content_lines[0]),
                    ]));

                    // Subsequent lines are indented
                    for line in &content_lines[1..] {
                        lines.push(Line::from(Span::raw(*line)));
                    }
                }
            }
            lines
        }
    } else {
        vec![Line::from("No session loaded.")]
    };

    // Show assistant's streaming response if waiting
    if app.waiting_for_response && !app.assistant_buffer.is_empty() {
        let buffer_lines: Vec<&str> = app.assistant_buffer.lines().collect();
        if buffer_lines.is_empty() {
            // Empty buffer, just show role
            messages_text.push(Line::from(vec![
                Span::styled(
                    "[assistant] ",
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(" ●", Style::default().fg(Color::Green)),
            ]));
        } else {
            // First line includes the role prefix and indicator
            messages_text.push(Line::from(vec![
                Span::styled(
                    "[assistant] ",
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(buffer_lines[0]),
                Span::styled(" ●", Style::default().fg(Color::Green)),
            ]));

            // Subsequent lines without prefix
            for line in &buffer_lines[1..] {
                messages_text.push(Line::from(Span::raw(*line)));
            }
        }
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
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((app.message_scroll, 0));
    f.render_widget(messages, chunks[1]);

    // Input area OR tool confirmation
    if app.awaiting_tool_confirmation {
        if let Some((ref tool_name, ref args)) = app.pending_tool_call {
            let confirmation_text = vec![
                Line::from(Span::styled("Tool Execution Confirmation", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from(Span::styled(format!("Tool: {}", tool_name), Style::default().fg(Color::Cyan))),
                Line::from(""),
                Line::from(Span::styled("Arguments:", Style::default().fg(Color::White))),
                Line::from(format!("{}", serde_json::to_string_pretty(args).unwrap_or_else(|_| "{}".to_string()))),
                Line::from(""),
                Line::from(Span::styled("Allow this tool to execute?  [Y]es  [N]o  [Q]uit", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
            ];

            let confirmation_widget = Paragraph::new(confirmation_text)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(" Confirm Tool Use "))
                .wrap(ratatui::widgets::Wrap { trim: false });

            f.render_widget(confirmation_widget, chunks[2]);
        }
    } else {
        let input_title = if app.vim_nav.mode == InputMode::Insert {
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
    }

    // Footer with keybinds
    let footer_text = if app.vim_nav.mode == InputMode::Command {
        "Command mode".to_string()
    } else if app.vim_nav.mode == InputMode::Insert {
        "INSERT mode | Esc: normal mode | Enter: newline | Ctrl+Space: send".to_string()
    } else {
        "i: insert mode | j/k: scroll messages | G: jump to bottom | Enter: send | 1: sessions | 2: chat | :w: save | :q: quit".to_string()
    };
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[3]);

    // Command line
    let cmd_line = if app.vim_nav.mode == InputMode::Command {
        Paragraph::new(format!(":{}", app.vim_nav.command_buffer))
            .style(Style::default().fg(Color::Green))
    } else {
        Paragraph::new("")
    };
    f.render_widget(cmd_line, chunks[4]);
}

fn draw_models(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(1),     // Model list
            Constraint::Length(5),  // Info/recommendations
            Constraint::Length(3),  // Footer with keybinds
            Constraint::Length(1),  // Command line
        ])
        .split(f.area());

    // Header
    let header = Paragraph::new("Installed Models")
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Installed Models
    if app.models.is_empty() {
        let empty_msg = Paragraph::new(vec![
            Line::from("No models installed."),
            Line::from(""),
            Line::from("Press 4 to browse available models"),
            Line::from("or use :pull <model>"),
        ])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Models"));
        f.render_widget(empty_msg, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .models
            .iter()
            .enumerate()
            .map(|(i, model)| {
                let size_mb = model.size / (1024 * 1024);
                let is_active = model.name == app.config.ollama_model;
                let active_marker = if is_active { " [active]" } else { "" };
                let display = format!("{} ({}MB){}", model.name, size_mb, active_marker);
                let style = if i == app.model_nav.selected_index {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else if is_active {
                    Style::default()
                        .fg(Color::Green)
                } else {
                    Style::default()
                };
                ListItem::new(display).style(style)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Models"));
        f.render_widget(list, chunks[1]);
    }

    // Info/recommendations or pull status
    let info_text = if let Some(ref status) = app.pull_status {
        vec![
            Line::from(Span::styled("Downloading Model:", Style::default().add_modifier(Modifier::BOLD))),
            Line::from(Span::styled(status, Style::default().fg(Color::Green))),
            Line::from(""),
        ]
    } else {
        vec![
            Line::from(Span::styled("Recommendations:", Style::default().add_modifier(Modifier::BOLD))),
            Line::from("Chat: mistral, llama3.2, phi3, qwen2.5"),
            Line::from("Code: codellama, deepseek-coder, starcoder2"),
        ]
    };
    let info = Paragraph::new(info_text)
        .block(Block::default().borders(Borders::ALL).title("Info"));
    f.render_widget(info, chunks[2]);

    // Footer with keybinds
    let footer_text = if app.vim_nav.mode == InputMode::Command {
        "Command mode".to_string()
    } else {
        "j/k: navigate | Enter: select model | :pull <model>: download | 3: models | 4: browse library | 1/2: sessions/chat".to_string()
    };
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[3]);

    // Command line
    let cmd_line = if app.vim_nav.mode == InputMode::Command {
        Paragraph::new(format!(":{}", app.vim_nav.command_buffer))
            .style(Style::default().fg(Color::Green))
    } else {
        Paragraph::new("")
    };
    f.render_widget(cmd_line, chunks[4]);
}

fn draw_browser(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(1),     // Model browser
            Constraint::Length(5),  // Info/recommendations
            Constraint::Length(3),  // Footer with keybinds
            Constraint::Length(1),  // Command line
        ])
        .split(f.area());

    // Header
    let header = Paragraph::new("Browse Model Library")
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Model browser list
    if app.browse_models.is_empty() {
        let empty_msg = Paragraph::new(vec![
            Line::from("Loading model library..."),
            Line::from(""),
            Line::from("Use :models to refresh"),
        ])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Available Models"));
        f.render_widget(empty_msg, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .browse_models
            .iter()
            .enumerate()
            .take(100) // Limit to first 100 for performance
            .map(|(i, model)| {
                let size_gb = model.size as f64 / (1024.0 * 1024.0 * 1024.0);
                let display = format!("{} ({:.1}GB)", model.name, size_gb);
                let style = if i == app.browse_nav.selected_index {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(display).style(style)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Available Models"));
        f.render_widget(list, chunks[1]);
    }

    // Info section
    let info_text = if let Some(ref status) = app.pull_status {
        vec![
            Line::from(Span::styled("Downloading Model:", Style::default().add_modifier(Modifier::BOLD))),
            Line::from(Span::styled(status, Style::default().fg(Color::Green))),
            Line::from(""),
        ]
    } else {
        vec![
            Line::from(Span::styled("Browse hundreds of models from Ollama library", Style::default().add_modifier(Modifier::BOLD))),
            Line::from("Press Enter to download a model"),
            Line::from(""),
        ]
    };
    let info = Paragraph::new(info_text)
        .block(Block::default().borders(Borders::ALL).title("Info"));
    f.render_widget(info, chunks[2]);

    // Footer with keybinds
    let footer_text = if app.vim_nav.mode == InputMode::Command {
        "Command mode".to_string()
    } else {
        "j/k: navigate | Enter: download model | 3: installed models | 4: browser | 1/2: sessions/chat".to_string()
    };
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[3]);

    // Command line
    let cmd_line = if app.vim_nav.mode == InputMode::Command {
        Paragraph::new(format!(":{}", app.vim_nav.command_buffer))
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
