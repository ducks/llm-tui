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
    let default_model = match app.config.default_llm_provider.as_str() {
        "bedrock" => &app.config.bedrock_model,
        "claude" => &app.config.claude_model,
        _ => &app.config.ollama_model,
    };
    let title = if let Some(ref project) = app.current_project {
        format!("LLM TUI - Project: {} [{} - {}]", project, app.config.default_llm_provider, default_model)
    } else {
        format!("LLM TUI - Sessions [{} - {}]", app.config.default_llm_provider, default_model)
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
    // Split screen into fixed header + scrollable content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Fixed header
            Constraint::Min(1),     // Scrollable content
        ])
        .split(f.area());

    // Fixed header with session info and token count
    let header_text = if let Some(ref session) = app.current_session {
        let provider = &session.llm_provider;
        let model = session.model.as_ref().map(|m| m.as_str()).unwrap_or("unknown");
        let total_tokens = session.total_tokens();
        let context_window = match provider.as_str() {
            "bedrock" => app.config.bedrock_context_window,
            "claude" => app.config.claude_context_window,
            _ => app.config.ollama_context_window,
        };
        let percent = (total_tokens as f64 / context_window as f64 * 100.0) as i32;
        format!("Chat: {} [{} - {}] | Tokens: {}/{} ({}%)",
            session.display_name(), provider, model, total_tokens, context_window, percent)
    } else {
        "Chat: No Session".to_string()
    };
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Build scrollable content
    let mut all_lines = Vec::new();
    let viewport_width = (chunks[1].width.saturating_sub(4)) as usize; // Subtract borders and padding

    // Helper to wrap a single line to viewport width
    let wrap_line = |text: &str| -> Vec<String> {
        if text.is_empty() {
            return vec![String::new()];
        }
        let mut wrapped = Vec::new();
        let mut current = String::new();
        for word in text.split_whitespace() {
            if current.is_empty() {
                current = word.to_string();
            } else if current.len() + 1 + word.len() <= viewport_width {
                current.push(' ');
                current.push_str(word);
            } else {
                wrapped.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            wrapped.push(current);
        }
        if wrapped.is_empty() {
            wrapped.push(String::new());
        }
        wrapped
    };

    // Messages
    if let Some(ref session) = app.current_session {
        if session.messages.is_empty() {
            all_lines.push(Line::from("No messages yet. Press 'i' to start typing."));
        } else {
            for msg in &session.messages {
                for (i, line) in msg.content.lines().enumerate() {
                    let wrapped = wrap_line(line);
                    for (j, wrapped_line) in wrapped.iter().enumerate() {
                        if i == 0 && j == 0 {
                            // First line gets role prefix
                            all_lines.push(Line::from(vec![
                                Span::styled(
                                    format!("[{}] ", msg.role),
                                    Style::default().fg(Color::Yellow),
                                ),
                                Span::raw(wrapped_line.clone()),
                            ]));
                        } else {
                            all_lines.push(Line::from(wrapped_line.clone()));
                        }
                    }
                }
                all_lines.push(Line::from("")); // Blank line between messages
            }
        }
    } else {
        all_lines.push(Line::from("No session loaded."));
    }

    // Show assistant's streaming response if waiting
    if app.waiting_for_response && !app.assistant_buffer.is_empty() {
        for (i, line) in app.assistant_buffer.lines().enumerate() {
            let wrapped = wrap_line(line);
            for (j, wrapped_line) in wrapped.iter().enumerate() {
                if i == 0 && j == 0 {
                    all_lines.push(Line::from(vec![
                        Span::styled(
                            "[assistant] ",
                            Style::default().fg(Color::Yellow),
                        ),
                        Span::raw(wrapped_line.clone()),
                        Span::styled(" ●", Style::default().fg(Color::Green)),
                    ]));
                } else {
                    all_lines.push(Line::from(wrapped_line.clone()));
                }
            }
        }
        all_lines.push(Line::from(""));
    } else if app.waiting_for_response {
        all_lines.push(Line::from(vec![
            Span::styled(
                "[assistant] ",
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("Thinking...", Style::default().fg(Color::Gray)),
        ]));
        all_lines.push(Line::from(""));
    }

    // Separator before input
    all_lines.push(Line::from("─".repeat(viewport_width)));

    // Input area OR tool confirmation
    if app.awaiting_tool_confirmation {
        if let Some((ref tool_name, ref args)) = app.pending_tool_call {
            all_lines.push(Line::from(Span::styled("Tool Execution Confirmation", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
            all_lines.push(Line::from(""));
            all_lines.push(Line::from(format!("Tool: {} - Args: {}", tool_name, serde_json::to_string_pretty(args).unwrap_or_else(|_| "{}".to_string()))));
            all_lines.push(Line::from(""));
            all_lines.push(Line::from(Span::styled("[Y]es  [N]o  [Q]uit", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))));
        }
    } else {
        let input_title = if app.vim_nav.mode == InputMode::Insert {
            "Input (INSERT)".to_string()
        } else if app.vim_nav.mode == InputMode::Command {
            format!(":{}", app.vim_nav.command_buffer)
        } else {
            "Input (press 'i' to start typing)".to_string()
        };
        all_lines.push(Line::from(Span::styled(input_title, Style::default().add_modifier(Modifier::BOLD))));
        all_lines.push(Line::from(""));

        if app.message_buffer.is_empty() {
            all_lines.push(Line::from(""));
        } else {
            for line in app.message_buffer.lines() {
                for wrapped_line in wrap_line(line) {
                    all_lines.push(Line::from(wrapped_line));
                }
            }
        }
    }

    // Footer at bottom
    all_lines.push(Line::from(""));
    all_lines.push(Line::from("─".repeat(viewport_width)));
    let footer_text = if app.vim_nav.mode == InputMode::Command {
        "Command mode".to_string()
    } else if app.vim_nav.mode == InputMode::Insert {
        "INSERT | Esc: normal | Enter: newline | Ctrl+Space: send".to_string()
    } else {
        "i: insert | j/k: scroll | G: bottom | Enter: send | :w :q".to_string()
    };
    all_lines.push(Line::from(footer_text));

    // Calculate scroll - we now know EXACTLY how many lines we have
    let total_lines = all_lines.len() as u16;
    let visible_height = chunks[1].height.saturating_sub(2); // Subtract borders from content area

    let scroll_offset = if !app.message_scroll_manual {
        // Auto-scroll to bottom
        let offset = total_lines.saturating_sub(visible_height);
        // Update app.message_scroll so j/k continue from here
        app.message_scroll = offset;
        offset
    } else {
        // Manual scroll - clamp to valid range
        let max_scroll = total_lines.saturating_sub(visible_height);
        let offset = app.message_scroll.min(max_scroll);
        // Update app.message_scroll to the clamped value
        app.message_scroll = offset;
        offset
    };

    // Render everything as one scrollable paragraph - NO WRAPPING since we pre-wrapped
    let paragraph = Paragraph::new(all_lines)
        .block(Block::default().borders(Borders::ALL).title("Messages"))
        .scroll((scroll_offset, 0));
    f.render_widget(paragraph, chunks[1]);
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
    let header = Paragraph::new("Models & Providers")
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Provider models list
    if app.provider_models.is_empty() {
        let empty_msg = Paragraph::new(vec![
            Line::from("Loading models..."),
            Line::from(""),
            Line::from("Press 3 to refresh"),
        ])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Models"));
        f.render_widget(empty_msg, chunks[1]);
    } else {
        let mut items: Vec<ListItem> = Vec::new();
        let mut current_provider = "";
        let mut item_index = 0; // Track position in rendered list

        for (i, model) in app.provider_models.iter().enumerate() {
            // Add provider header when switching to a new provider
            if model.provider != current_provider {
                current_provider = &model.provider;
                let provider_header = format!("=== {} ===", model.provider.to_uppercase());
                items.push(ListItem::new(provider_header).style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                ));
                item_index += 1; // Headers also take up space
            }

            // Build model display with indicators
            let mut markers = Vec::new();
            if model.is_current {
                markers.push("current");
            }
            if model.installed {
                markers.push("installed");
            }

            let marker_str = if !markers.is_empty() {
                format!(" [{}]", markers.join(", "))
            } else {
                String::new()
            };

            let display = format!("  {}{}", model.model_id, marker_str);

            // Check against item_index (rendered position), not i (provider_models index)
            let style = if item_index == app.model_nav.selected_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if model.is_current {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };

            items.push(ListItem::new(display).style(style));
            item_index += 1;
        }

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
            Line::from(Span::styled("Select any model to switch provider and model", Style::default().add_modifier(Modifier::BOLD))),
            Line::from("Ollama models must be pulled first (use :pull <model>)"),
            Line::from("Claude and Bedrock models are available instantly"),
        ]
    };
    let info = Paragraph::new(info_text)
        .block(Block::default().borders(Borders::ALL).title("Info"));
    f.render_widget(info, chunks[2]);

    // Footer with keybinds
    let footer_text = if app.vim_nav.mode == InputMode::Command {
        "Command mode".to_string()
    } else {
        "j/k: navigate | Enter: select model+provider | :pull <model>: download Ollama model | 3: refresh | 1/2: sessions/chat".to_string()
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
