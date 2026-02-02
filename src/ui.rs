use crate::app::{App, AppScreen};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use vim_navigator::InputMode;

// Gruvbox fg2 - softer than white, easier on the eyes
const FG2: Color = Color::Rgb(213, 196, 161);

pub fn draw(f: &mut Frame, app: &mut App) {
    match app.screen {
        AppScreen::SessionList => draw_session_list(f, app),
        AppScreen::Chat => draw_chat(f, app),
        AppScreen::Models => draw_models(f, app),
        AppScreen::Settings => draw_settings(f, app),
        AppScreen::Help => draw_help(f, app),
        AppScreen::Setup => draw_setup(f, app),
    }
}

fn draw_session_list(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(1),    // Session list
            Constraint::Length(3), // Footer with keybinds
            Constraint::Length(1), // Command line
        ])
        .split(f.area());

    // Header
    let default_model = match app.config.default_llm_provider.as_str() {
        "bedrock" => &app.config.bedrock_model,
        "claude" => &app.config.claude_model,
        _ => &app.config.ollama_model,
    };
    let title = if let Some(ref project) = app.current_project {
        format!(
            "LLM TUI - Project: {} [{} - {}]",
            project, app.config.default_llm_provider, default_model
        )
    } else {
        format!(
            "LLM TUI - Sessions [{} - {}]",
            app.config.default_llm_provider, default_model
        )
    };
    let header = Paragraph::new(title)
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG2)),
        );
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
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Sessions")
                .border_style(Style::default().fg(FG2))
                .title_style(Style::default().fg(FG2)),
        );
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
                        let model_str = session
                            .model
                            .as_ref()
                            .map(|m| format!(" ({})", m))
                            .unwrap_or_default();
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

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Sessions")
                .border_style(Style::default().fg(FG2))
                .title_style(Style::default().fg(FG2)),
        );
        f.render_widget(list, chunks[1]);
    }

    // Footer with keybinds
    let footer_text = if app.vim_nav.mode == InputMode::Command {
        "Command mode".to_string()
    } else {
        "j/k: navigate | Enter: open | Space: toggle | n: new in project | d: delete | :new [name] --project <proj> | 1: sessions | q: quit".to_string()
    };
    let footer = Paragraph::new(footer_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(FG2)),
    );
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
            Constraint::Min(1),    // Scrollable content
        ])
        .split(f.area());

    // Fixed header with session info and token count
    let header_text = if let Some(ref session) = app.current_session {
        let provider = &session.llm_provider;
        let model = session
            .model
            .as_ref()
            .map(|m| m.as_str())
            .unwrap_or("unknown");
        let total_tokens = session.total_tokens();
        let context_window = match provider.as_str() {
            "bedrock" => app.config.bedrock_context_window,
            "claude" => app.config.claude_context_window,
            _ => app.config.ollama_context_window,
        };
        let percent = (total_tokens as f64 / context_window as f64 * 100.0) as i32;
        format!(
            "Chat: {} [{} - {}] | Tokens: {}/{} ({}%)",
            session.display_name(),
            provider,
            model,
            total_tokens,
            context_window,
            percent
        )
    } else {
        "Chat: No Session".to_string()
    };
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG2)),
        );
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
                // Determine styling based on role
                let (prefix, prefix_style, content_style) = match msg.role.as_str() {
                    "user" => (
                        "> ",
                        Style::default().fg(Color::White).bg(Color::DarkGray),
                        Style::default().fg(Color::White).bg(Color::DarkGray),
                    ),
                    "assistant" => (
                        "● ",
                        Style::default().fg(FG2), // gruvbox fg2
                        Style::default().fg(FG2), // gruvbox fg2
                    ),
                    "system" => {
                        // Check if system message indicates error or failure
                        let content_lower = msg.content.to_lowercase();
                        let is_tool_result = msg.content.starts_with("[Tool ");
                        let is_error = content_lower.contains("error")
                            || content_lower.contains("rejected")
                            || content_lower.contains("failed")
                            || content_lower.contains("not found")
                            || content_lower.contains("access denied");

                        // Tool results get colored bullets, other system messages stay gray
                        let bullet_color = if is_tool_result {
                            if is_error {
                                Color::Red
                            } else {
                                Color::Green
                            }
                        } else {
                            Color::Gray
                        };

                        (
                            "● ",
                            Style::default().fg(bullet_color),
                            Style::default().fg(Color::Gray),
                        )
                    }
                    _ => (
                        "● ",
                        Style::default().fg(Color::Gray),
                        Style::default().fg(Color::Gray),
                    ),
                };

                for (i, line) in msg.content.lines().enumerate() {
                    let wrapped = wrap_line(line);
                    for (j, wrapped_line) in wrapped.iter().enumerate() {
                        if i == 0 && j == 0 {
                            // First line gets prefix
                            all_lines.push(Line::from(vec![
                                Span::styled(prefix, prefix_style),
                                Span::styled(wrapped_line.clone(), content_style),
                            ]));
                        } else {
                            // Continuation lines (indent to align with content after prefix)
                            let indent = if msg.role == "user" { "  " } else { "  " };
                            all_lines.push(Line::from(vec![
                                Span::styled(indent, content_style),
                                Span::styled(wrapped_line.clone(), content_style),
                            ]));
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
                        Span::styled("● ", Style::default().fg(Color::Gray)),
                        Span::styled(wrapped_line.clone(), Style::default().fg(Color::Gray)),
                        Span::styled(" ●", Style::default().fg(Color::Green)),
                    ]));
                } else {
                    all_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(wrapped_line.clone(), Style::default().fg(Color::Gray)),
                    ]));
                }
            }
        }
        all_lines.push(Line::from(""));
    } else if app.waiting_for_response {
        all_lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(Color::Gray)),
            Span::styled("Thinking...", Style::default().fg(Color::DarkGray)),
        ]));
        all_lines.push(Line::from(""));
    }

    // Separator before input
    all_lines.push(Line::from("─".repeat(viewport_width)));

    // Input area OR tool confirmation
    if app.awaiting_tool_confirmation {
        if let Some((ref tool_name, ref args)) = app.pending_tool_call {
            all_lines.push(Line::from(Span::styled(
                "Tool Execution Confirmation",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            all_lines.push(Line::from(""));
            all_lines.push(Line::from(format!(
                "Tool: {} - Args: {}",
                tool_name,
                serde_json::to_string_pretty(args).unwrap_or_else(|_| "{}".to_string())
            )));
            all_lines.push(Line::from(""));
            all_lines.push(Line::from(Span::styled(
                "[Y]es  [N]o  [Q]uit",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    } else {
        let input_title = if app.vim_nav.mode == InputMode::Insert {
            "Input (INSERT)".to_string()
        } else if app.vim_nav.mode == InputMode::Command {
            format!(":{}", app.vim_nav.command_buffer)
        } else {
            "Input (press 'i' to start typing)".to_string()
        };
        all_lines.push(Line::from(Span::styled(
            input_title,
            Style::default().fg(FG2).add_modifier(Modifier::BOLD),
        )));
        all_lines.push(Line::from(""));

        if app.message_buffer.is_empty() {
            all_lines.push(Line::from(Span::styled("> ", Style::default().fg(FG2))));
        } else {
            for (i, line) in app.message_buffer.lines().enumerate() {
                let prefix = if i == 0 { "> " } else { "  " };
                for (j, wrapped_line) in wrap_line(line).iter().enumerate() {
                    let line_prefix = if i == 0 && j == 0 { prefix } else { "  " };
                    all_lines.push(Line::from(Span::styled(
                        format!("{}{}", line_prefix, wrapped_line),
                        Style::default().fg(FG2),
                    )));
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
    all_lines.push(Line::from(Span::styled(
        footer_text,
        Style::default().fg(FG2),
    )));

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
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Messages")
                .border_style(Style::default().fg(FG2))
                .title_style(Style::default().fg(FG2)),
        )
        .scroll((scroll_offset, 0));
    f.render_widget(paragraph, chunks[1]);
}

fn draw_models(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(1),    // Model list
            Constraint::Length(5), // Info/recommendations
            Constraint::Length(3), // Footer with keybinds
            Constraint::Length(1), // Command line
        ])
        .split(f.area());

    // Header
    let header = Paragraph::new("Models & Providers")
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG2)),
        );
    f.render_widget(header, chunks[0]);

    // Provider models list
    if app.provider_models.is_empty() {
        let empty_msg = Paragraph::new(vec![
            Line::from("Loading models..."),
            Line::from(""),
            Line::from("Press 3 to refresh"),
        ])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Models")
                .border_style(Style::default().fg(FG2))
                .title_style(Style::default().fg(FG2)),
        );
        f.render_widget(empty_msg, chunks[1]);
    } else {
        let mut items: Vec<ListItem> = Vec::new();
        let mut current_provider = "";
        let mut item_index = 0; // Track position in rendered list

        for (_i, model) in app.provider_models.iter().enumerate() {
            // Add provider header when switching to a new provider
            if model.provider != current_provider {
                current_provider = &model.provider;
                let provider_header = format!("=== {} ===", model.provider.to_uppercase());
                items.push(
                    ListItem::new(provider_header).style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
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

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Models")
                .border_style(Style::default().fg(FG2))
                .title_style(Style::default().fg(FG2)),
        );
        f.render_widget(list, chunks[1]);
    }

    // Info/recommendations or pull status
    let info_text = if let Some(ref status) = app.pull_status {
        vec![
            Line::from(Span::styled(
                "Downloading Model:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(status, Style::default().fg(Color::Green))),
            Line::from(""),
        ]
    } else {
        vec![
            Line::from(Span::styled(
                "Select any model to switch provider and model",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Ollama models must be pulled first (use :pull <model>)"),
            Line::from("Claude and Bedrock models are available instantly"),
        ]
    };
    let info = Paragraph::new(info_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Info")
            .border_style(Style::default().fg(FG2))
            .title_style(Style::default().fg(FG2)),
    );
    f.render_widget(info, chunks[2]);

    // Footer with keybinds
    let footer_text = if app.vim_nav.mode == InputMode::Command {
        "Command mode".to_string()
    } else {
        "j/k: navigate | Enter: select model+provider | :pull <model>: download Ollama model | 3: refresh | 1/2: sessions/chat".to_string()
    };
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(FG2))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG2)),
        );
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
        .borders(Borders::ALL)
        .border_style(Style::default().fg(FG2))
        .title_style(Style::default().fg(FG2));
    f.render_widget(block, f.area());
}

fn draw_help(f: &mut Frame, _app: &App) {
    let help_text = vec![
        Line::from(Span::styled(
            "LLM TUI - Help",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Navigation",
            Style::default().fg(FG2).add_modifier(Modifier::BOLD),
        )),
        Line::from("  1          - Sessions screen"),
        Line::from("  2          - Chat screen (if session open)"),
        Line::from("  3          - Models screen"),
        Line::from("  ?          - Help screen (this screen)"),
        Line::from("  q          - Quit application"),
        Line::from(""),
        Line::from(Span::styled(
            "Session List Screen",
            Style::default().fg(FG2).add_modifier(Modifier::BOLD),
        )),
        Line::from("  j/k        - Navigate sessions"),
        Line::from("  g/G        - Jump to top/bottom"),
        Line::from("  Enter      - Open selected session"),
        Line::from("  Space      - Toggle project expand/collapse"),
        Line::from("  n          - New session in current project"),
        Line::from("  d          - Delete selected session"),
        Line::from(""),
        Line::from(Span::styled(
            "Chat Screen",
            Style::default().fg(FG2).add_modifier(Modifier::BOLD),
        )),
        Line::from("  i          - Enter insert mode"),
        Line::from("  Esc        - Return to normal mode"),
        Line::from("  Enter      - Send message (normal mode)"),
        Line::from("  Ctrl+Space - Send message (insert mode)"),
        Line::from("  j/k        - Scroll up/down (normal mode)"),
        Line::from("  G          - Jump to bottom and resume auto-scroll"),
        Line::from(""),
        Line::from(Span::styled(
            "Models Screen",
            Style::default().fg(FG2).add_modifier(Modifier::BOLD),
        )),
        Line::from("  j/k        - Navigate models"),
        Line::from("  Enter      - Select model (downloads if not installed)"),
        Line::from(""),
        Line::from(Span::styled(
            "Commands (type : to enter command mode)",
            Style::default().fg(FG2).add_modifier(Modifier::BOLD),
        )),
        Line::from("  :new [name]              - Create new session"),
        Line::from("  :new project <name>      - Create/switch to project"),
        Line::from("  :rename <name>           - Rename current session"),
        Line::from("  :delete-session / :ds    - Delete current session"),
        Line::from("  :project <name>          - Set current project"),
        Line::from("  :provider <name>         - Switch provider (ollama/claude/bedrock)"),
        Line::from("  :load <file|session>     - Load context from file or session"),
        Line::from("  :compact                 - Manually compact conversation"),
        Line::from("  :pull <model>            - Download Ollama model"),
        Line::from("  :setup                   - Run setup wizard"),
        Line::from("  :w / :save               - Save current session"),
        Line::from("  :q / :quit               - Quit application"),
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to return",
            Style::default().fg(Color::Green),
        )),
    ];

    let paragraph = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG2))
                .title("Help")
                .title_style(Style::default().fg(FG2)),
        )
        .alignment(Alignment::Left);

    f.render_widget(paragraph, f.area());
}

fn draw_setup(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(10),   // Content
            Constraint::Length(3), // Status message
        ])
        .split(f.area());

    // Title
    let title = Paragraph::new("LLM TUI - Setup Wizard")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG2)),
        );
    f.render_widget(title, chunks[0]);

    // Content based on step
    let content = match app.setup_step {
        0 => draw_setup_welcome(),
        1 => draw_setup_ollama(app),
        2 => draw_setup_claude(app),
        3 => draw_setup_bedrock(app),
        4 => draw_setup_complete(app),
        _ => vec![],
    };

    let content_widget = Paragraph::new(content)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG2)),
        )
        .alignment(Alignment::Left);
    f.render_widget(content_widget, chunks[1]);

    // Status message
    if !app.setup_message.is_empty() {
        let status = Paragraph::new(app.setup_message.clone())
            .style(Style::default().fg(FG2))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(FG2)),
            );
        f.render_widget(status, chunks[2]);
    }
}

fn draw_setup_welcome() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from("Welcome to LLM TUI!"),
        Line::from(""),
        Line::from("This wizard will help you configure providers for:"),
        Line::from("  • Ollama (local models)"),
        Line::from("  • Claude API (Anthropic)"),
        Line::from("  • AWS Bedrock (Claude on AWS)"),
        Line::from(""),
        Line::from("Would you like to run the setup wizard?"),
        Line::from(""),
        Line::from(Span::styled(
            "  [y] Yes, configure providers",
            Style::default().fg(Color::Green),
        )),
        Line::from(Span::styled(
            "  [n] No, skip setup",
            Style::default().fg(Color::Red),
        )),
    ]
}

fn draw_setup_ollama(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Ollama Setup",
            Style::default().fg(FG2).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if let Some(status) = app.ollama_status {
        if status {
            lines.push(Line::from(Span::styled(
                "✓ Ollama is running",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from("You can now use local models from Ollama."));
        } else {
            lines.push(Line::from(Span::styled(
                "✗ Ollama is not running",
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from("To use Ollama:"));
            lines.push(Line::from("  1. Install from https://ollama.ai"));
            lines.push(Line::from("  2. Run 'ollama serve' in another terminal"));
            lines.push(Line::from("  3. Pull a model: 'ollama pull mistral'"));
        }
    } else {
        lines.push(Line::from("Checking Ollama connection..."));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press Enter to continue",
        Style::default().fg(Color::Green),
    )));

    lines
}

fn draw_setup_claude(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Claude API Setup",
            Style::default().fg(FG2).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if let Some(status) = app.claude_status {
        if status {
            lines.push(Line::from(Span::styled(
                "✓ Claude API key configured",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "You can now use Claude models via Anthropic API.",
            ));
        } else {
            lines.push(Line::from(Span::styled(
                "✗ No Claude API key found",
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from("To use Claude API:"));
            lines.push(Line::from(
                "  1. Get an API key from https://console.anthropic.com",
            ));
            lines.push(Line::from("  2. Set environment variable:"));
            lines.push(Line::from("     export ANTHROPIC_API_KEY=\"sk-ant-...\""));
            lines.push(Line::from(
                "  3. Add to your ~/.bashrc or ~/.zshrc to persist",
            ));
            lines.push(Line::from("  4. Restart the terminal and llm-tui"));
        }
    } else {
        lines.push(Line::from("Checking Claude API configuration..."));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press Enter to continue  [s] Skip",
        Style::default().fg(Color::Green),
    )));

    lines
}

fn draw_setup_bedrock(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "AWS Bedrock Setup",
            Style::default().fg(FG2).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if let Some(status) = app.bedrock_status {
        if status {
            lines.push(Line::from(Span::styled(
                "✓ AWS credentials configured",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from("You can now use Claude models via AWS Bedrock."));
        } else {
            lines.push(Line::from(Span::styled(
                "✗ No AWS credentials found",
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from("To use AWS Bedrock:"));
            lines.push(Line::from(
                "  1. Install AWS CLI: https://aws.amazon.com/cli",
            ));
            lines.push(Line::from("  2. Configure credentials:"));
            lines.push(Line::from("     aws configure"));
            lines.push(Line::from("  3. Or set environment variables:"));
            lines.push(Line::from("     export AWS_ACCESS_KEY_ID=\"...\""));
            lines.push(Line::from("     export AWS_SECRET_ACCESS_KEY=\"...\""));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "Note: AWS credentials should be stored securely.",
            ));
            lines.push(Line::from(
                "      The 'aws configure' command creates ~/.aws/credentials",
            ));
            lines.push(Line::from(
                "      with proper 0600 permissions automatically.",
            ));
        }
    } else {
        lines.push(Line::from("Checking AWS credentials..."));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press Enter to continue  [s] Skip",
        Style::default().fg(Color::Green),
    )));

    lines
}

fn draw_setup_complete(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Setup Complete!",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Provider Status:"),
        Line::from(""),
    ];

    // Ollama status
    if let Some(status) = app.ollama_status {
        let icon = if status { "✓" } else { "✗" };
        let color = if status { Color::Green } else { Color::Red };
        lines.push(Line::from(Span::styled(
            format!("  {} Ollama", icon),
            Style::default().fg(color),
        )));
    }

    // Claude status
    if let Some(status) = app.claude_status {
        let icon = if status { "✓" } else { "✗" };
        let color = if status { Color::Green } else { Color::Red };
        lines.push(Line::from(Span::styled(
            format!("  {} Claude API", icon),
            Style::default().fg(color),
        )));
    }

    // Bedrock status
    if let Some(status) = app.bedrock_status {
        let icon = if status { "✓" } else { "✗" };
        let color = if status { Color::Green } else { Color::Red };
        lines.push(Line::from(Span::styled(
            format!("  {} AWS Bedrock", icon),
            Style::default().fg(color),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(
        "You can reconfigure providers anytime with the :setup command.",
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press Enter to start using LLM TUI",
        Style::default().fg(Color::Green),
    )));

    lines
}
