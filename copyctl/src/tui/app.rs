use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs, Clear},
    Frame,
};
use std::time::{Duration, Instant};

use super::{
    file_browser::FileBrowser,
    job_monitor::JobMonitor,
    help_screen::HelpScreen,
    config_editor::ConfigEditor,
};
use crate::client::CopyClient;

#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    FileBrowser,
    JobMonitor,
    Config,
    Help,
}

impl AppScreen {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppScreen::FileBrowser => "File Browser",
            AppScreen::JobMonitor => "Job Monitor",
            AppScreen::Config => "Configuration",
            AppScreen::Help => "Help",
        }
    }
}

pub struct App {
    pub current_screen: AppScreen,
    pub file_browser: FileBrowser,
    pub job_monitor: JobMonitor,
    pub help_screen: HelpScreen,
    pub config_editor: ConfigEditor,
    pub client: CopyClient,
    pub last_update: Instant,
    pub status_message: Option<(String, Instant, bool)>, // (message, timestamp, is_error)
    pub show_popup: bool,
    pub popup_content: String,
}

impl App {
    pub async fn new(client: CopyClient) -> Result<Self> {
        Ok(Self {
            current_screen: AppScreen::FileBrowser,
            file_browser: FileBrowser::new()?,
            job_monitor: JobMonitor::new(),
            help_screen: HelpScreen::new(),
            config_editor: ConfigEditor::new()?,
            client,
            last_update: Instant::now(),
            status_message: None,
            show_popup: false,
            popup_content: String::new(),
        })
    }

    pub fn draw(&mut self, f: &mut Frame) {
        let size = f.size();

        // Create main layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Tab bar
                Constraint::Min(0),    // Main content
                Constraint::Length(1), // Status bar
            ])
            .split(size);

        // Draw tab bar
        self.draw_tab_bar(f, chunks[0]);

        // Draw main content based on current screen
        match self.current_screen {
            AppScreen::FileBrowser => self.file_browser.draw(f, chunks[1]),
            AppScreen::JobMonitor => self.job_monitor.draw(f, chunks[1]),
            AppScreen::Config => self.config_editor.draw(f, chunks[1]),
            AppScreen::Help => self.help_screen.draw(f, chunks[1]),
        }

        // Draw status bar
        self.draw_status_bar(f, chunks[2]);

        // Draw popup if needed
        if self.show_popup {
            self.draw_popup(f, size);
        }
    }

    fn draw_tab_bar(&self, f: &mut Frame, area: Rect) {
        let titles = vec![
            AppScreen::FileBrowser.as_str(),
            AppScreen::JobMonitor.as_str(),
            AppScreen::Config.as_str(),
            AppScreen::Help.as_str(),
        ];

        let selected_index = match self.current_screen {
            AppScreen::FileBrowser => 0,
            AppScreen::JobMonitor => 1,
            AppScreen::Config => 2,
            AppScreen::Help => 3,
        };

        let tabs = Tabs::new(titles)
            .block(Block::default().borders(Borders::ALL).title("copyctl - Modern File Operations"))
            .style(Style::default().fg(Color::White))
            .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            .select(selected_index);

        f.render_widget(tabs, area);
    }

    fn draw_status_bar(&self, f: &mut Frame, area: Rect) {
        let mut status_text = Vec::new();

        // Show current time
        let now = chrono::Local::now();
        status_text.push(Span::styled(
            format!("{}", now.format("%H:%M:%S")),
            Style::default().fg(Color::Cyan),
        ));

        status_text.push(Span::raw(" | "));

        // Show status message or default help
        if let Some((ref message, timestamp, is_error)) = self.status_message {
            if timestamp.elapsed() < Duration::from_secs(5) {
                let style = if is_error {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::Green)
                };
                status_text.push(Span::styled(message.clone(), style));
            } else {
                status_text.push(Span::styled("Press '?' for help, 'q' to quit", Style::default().fg(Color::Gray)));
            }
        } else {
            status_text.push(Span::styled("Press '?' for help, 'q' to quit", Style::default().fg(Color::Gray)));
        }

        // Add connection status
        status_text.push(Span::raw(" | "));
        status_text.push(Span::styled(
            "Connected to copyd",
            Style::default().fg(Color::Green),
        ));

        let status_paragraph = Paragraph::new(Line::from(status_text))
            .style(Style::default().bg(Color::DarkGray));

        f.render_widget(status_paragraph, area);
    }

    fn draw_popup(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(60, 20, area);
        
        f.render_widget(Clear, popup_area);
        
        let popup_block = Block::default()
            .title("Information")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));

        let popup_content = Paragraph::new(self.popup_content.as_str())
            .block(popup_block)
            .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(popup_content, popup_area);
    }

    pub async fn handle_key_event(&mut self, key: KeyEvent) -> Result<bool> {
        // Global key bindings
        match key.code {
            KeyCode::Char('q') => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(true); // Quit
                }
            }
            KeyCode::Char('?') => {
                self.current_screen = AppScreen::Help;
                return Ok(false);
            }
            KeyCode::Tab => {
                self.switch_to_next_tab();
                return Ok(false);
            }
            KeyCode::BackTab => {
                self.switch_to_prev_tab();
                return Ok(false);
            }
            KeyCode::F(1) => {
                self.current_screen = AppScreen::FileBrowser;
                return Ok(false);
            }
            KeyCode::F(2) => {
                self.current_screen = AppScreen::JobMonitor;
                return Ok(false);
            }
            KeyCode::F(3) => {
                self.current_screen = AppScreen::Config;
                return Ok(false);
            }
            KeyCode::F(4) => {
                self.current_screen = AppScreen::Help;
                return Ok(false);
            }
            KeyCode::Esc => {
                if self.show_popup {
                    self.show_popup = false;
                    return Ok(false);
                }
            }
            _ => {}
        }

        // Handle popup keys
        if self.show_popup {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.show_popup = false;
                }
                _ => {}
            }
            return Ok(false);
        }

        // Screen-specific key handling
        match self.current_screen {
            AppScreen::FileBrowser => {
                if self.file_browser.handle_key_event(key, &mut self.client).await? {
                    self.set_status_message("File operation completed", false);
                }
            }
            AppScreen::JobMonitor => {
                self.job_monitor.handle_key_event(key, &mut self.client).await?;
            }
            AppScreen::Config => {
                if self.config_editor.handle_key_event(key).await? {
                    self.set_status_message("Configuration saved", false);
                }
            }
            AppScreen::Help => {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        self.current_screen = AppScreen::FileBrowser;
                    }
                    _ => {
                        self.help_screen.handle_key_event(key);
                    }
                }
            }
        }

        Ok(false)
    }

    pub async fn update(&mut self) -> Result<()> {
        let now = Instant::now();
        
        // Update every 500ms
        if now.duration_since(self.last_update) > Duration::from_millis(500) {
            match self.current_screen {
                AppScreen::FileBrowser => {
                    self.file_browser.update().await?;
                }
                AppScreen::JobMonitor => {
                    self.job_monitor.update(&mut self.client).await?;
                }
                _ => {}
            }
            
            self.last_update = now;
        }

        Ok(())
    }

    fn switch_to_next_tab(&mut self) {
        self.current_screen = match self.current_screen {
            AppScreen::FileBrowser => AppScreen::JobMonitor,
            AppScreen::JobMonitor => AppScreen::Config,
            AppScreen::Config => AppScreen::Help,
            AppScreen::Help => AppScreen::FileBrowser,
        };
    }

    fn switch_to_prev_tab(&mut self) {
        self.current_screen = match self.current_screen {
            AppScreen::FileBrowser => AppScreen::Help,
            AppScreen::JobMonitor => AppScreen::FileBrowser,
            AppScreen::Config => AppScreen::JobMonitor,
            AppScreen::Help => AppScreen::Config,
        };
    }

    pub fn set_status_message(&mut self, message: &str, is_error: bool) {
        self.status_message = Some((message.to_string(), Instant::now(), is_error));
    }

    pub fn show_popup(&mut self, content: &str) {
        self.popup_content = content.to_string();
        self.show_popup = true;
    }
}

// Helper function to create a centered rectangle
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
} 