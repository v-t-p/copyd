pub mod app;
pub mod file_browser;
pub mod job_monitor;
pub mod help_screen;
pub mod config_editor;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io;
use tracing::{info, error};

use crate::client::CopyClient;
pub use app::App;

pub async fn run_tui(client: CopyClient) -> Result<()> {
    info!("Starting copyctl Terminal UI");

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run it
    let res = run_app(&mut terminal, client).await;

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
    client: CopyClient,
) -> Result<()> {
    let mut app = App::new(client).await?;
    loop {
        terminal.draw(|f| app.draw(f))?;

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.handle_key_event(key).await? {
                        true => break, // Quit requested
                        false => {}, // Continue
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
    run_tui(client).await
}

pub async fn run_navigator(client: crate::client::CopyClient) -> Result<()> {
    info!("Starting file navigator TUI");
    run_tui(client).await
} 