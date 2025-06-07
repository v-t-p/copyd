pub mod app;
pub mod file_browser;
pub mod job_monitor;
pub mod help_screen;
pub mod config_editor;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io;
use tracing::{info, error};

pub use app::App;

pub async fn run_tui() -> Result<()> {
    info!("Starting copyctl Terminal UI");

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run it
    let app = App::new().await?;
    let res = run_app(&mut terminal, app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        error!("Terminal UI error: {}", err);
        return Err(err);
    }

    info!("Terminal UI closed");
    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
) -> Result<()> {
    loop {
        terminal.draw(|f| app.draw(f))?;

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.handle_key_event(key).await? {
                        true => break, // Quit requested
                        false => continue,
                    }
                }
            }
        }

        // Update app state periodically
        app.update().await?;
    }

    Ok(())
}

pub async fn run_monitor(client: crate::client::CopyClient) -> Result<()> {
    info!("Starting job monitor TUI");
    run_tui().await
}

pub async fn run_navigator(client: crate::client::CopyClient) -> Result<()> {
    info!("Starting file navigator TUI");
    run_tui().await
} 