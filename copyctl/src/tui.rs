use crate::client::CopyClient;
use anyhow::Result;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
    Terminal,
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;
use tokio::time::{interval, Duration};

pub async fn run_monitor(client: CopyClient) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = monitor_loop(&mut terminal, client).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn monitor_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    client: CopyClient,
) -> Result<()> {
    let mut last_tick = std::time::Instant::now();
    let tick_rate = Duration::from_millis(250);

    loop {
        // Draw UI
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(0),
                ].as_ref())
                .split(f.size());

            // Title
            let title = Paragraph::new("Copy Monitor - Press 'q' to quit")
                .block(Block::default().borders(Borders::ALL).title("copyd Monitor"));
            f.render_widget(title, chunks[0]);

            // Job list placeholder
            let jobs = List::new(vec![
                ListItem::new("No active jobs"),
            ])
            .block(Block::default().borders(Borders::ALL).title("Active Jobs"));
            f.render_widget(jobs, chunks[1]);
        })?;

        // Handle events
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => break,
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            // Update data here
            last_tick = std::time::Instant::now();
        }
    }

    Ok(())
}

pub async fn run_navigator(client: CopyClient) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = navigator_loop(&mut terminal, client).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn navigator_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    client: CopyClient,
) -> Result<()> {
    let mut last_tick = std::time::Instant::now();
    let tick_rate = Duration::from_millis(250);

    loop {
        // Draw UI
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .margin(1)
                .constraints([
                    Constraint::Percentage(50),
                    Constraint::Percentage(50),
                ].as_ref())
                .split(f.size());

            // Left pane
            let left_items = vec![
                ListItem::new(".."),
                ListItem::new("Documents/"),
                ListItem::new("Downloads/"),
                ListItem::new("file1.txt"),
                ListItem::new("file2.txt"),
            ];
            let left_list = List::new(left_items)
                .block(Block::default().borders(Borders::ALL).title("Source"))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            f.render_widget(left_list, chunks[0]);

            // Right pane
            let right_items = vec![
                ListItem::new(".."),
                ListItem::new("backup/"),
                ListItem::new("temp/"),
            ];
            let right_list = List::new(right_items)
                .block(Block::default().borders(Borders::ALL).title("Destination"));
            f.render_widget(right_list, chunks[1]);
        })?;

        // Handle events
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => break,
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
        }
    }

    Ok(())
} 