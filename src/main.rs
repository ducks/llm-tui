mod app;
mod config;
mod db;
mod provider;
mod session;
mod tree;
mod ui;
mod tools;

use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::stdout;
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

    // Check if this is first run (no config file) and show setup wizard
    let config_path = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
        .join("llm-tui")
        .join("config.toml");

    let is_first_run = !config_path.exists();
    if is_first_run {
        app.start_setup_wizard();
    }

    let mut needs_redraw = true;

    loop {
        // Only redraw if something changed
        if needs_redraw {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            needs_redraw = false;
        }

        // Check for timer-based autosave
        app.check_autosave();

        // Check for LLM response tokens (unified handler for all providers)
        let had_response_data = app.response_receiver.is_some();
        app.check_response();
        if had_response_data {
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
