mod app;
mod config;
mod db;
mod ollama;
mod session;
mod tree;
mod ui;
mod input;
mod tools;
mod claude;

use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{stdout, Write};
use std::fs::OpenOptions;
use std::sync::Mutex;

// Global logger for debugging
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {{
        use std::io::Write;
        if let Ok(mut guard) = crate::LOG_FILE.lock() {
            if let Some(ref mut file) = *guard {
                let _ = writeln!(file, $($arg)*);
                let _ = file.flush();
            }
        }
    }};
}

fn main() -> Result<()> {
    // Initialize log file in current directory
    let log_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open("llm-tui-debug.log")?;

    *LOG_FILE.lock().unwrap() = Some(log_file);
    debug_log!("=== llm-tui started ===");

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = app::App::new()?;

    let mut needs_redraw = true;

    loop {
        // Only redraw if something changed
        if needs_redraw {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            needs_redraw = false;
        }

        // Check for timer-based autosave
        app.check_autosave();

        // Check for LLM response tokens (triggers redraw if we got data)
        let had_llm_data = app.llm_receiver.is_some();
        app.check_llm_response();
        if had_llm_data {
            needs_redraw = true;
        }

        // Check for Claude response tokens
        let had_claude_data = app.claude_receiver.is_some();
        app.check_claude_response();
        if had_claude_data {
            needs_redraw = true;
        }

        // Check for model pull progress
        let had_pull_data = app.pull_receiver.is_some();
        app.check_pull_progress();
        if had_pull_data {
            needs_redraw = true;
        }

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if app.handle_input(key)? {
                    break;
                }
                needs_redraw = true;
            }
        }
    }

    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}
