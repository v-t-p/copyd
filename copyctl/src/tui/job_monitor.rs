use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use crate::client::CopyClient;

pub struct JobMonitor {
    pub jobs: Vec<String>,
}

impl JobMonitor {
    pub fn new() -> Self {
        Self {
            jobs: vec!["Job monitoring coming soon...".to_string()],
        }
    }

    pub fn draw<B: Backend>(&mut self, f: &mut Frame<B>, area: Rect) {
        let block = Block::default()
            .title("Job Monitor")
            .borders(Borders::ALL);
        
        let items: Vec<ListItem> = self.jobs.iter()
            .map(|job| ListItem::new(Line::from(Span::raw(job))))
            .collect();
        
        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }

    pub async fn handle_key_event(&mut self, key: KeyEvent, client: &mut CopyClient) -> Result<()> {
        // TODO: Implement job control
        Ok(())
    }

    pub async fn update(&mut self, client: &mut CopyClient) -> Result<()> {
        // TODO: Refresh job list from daemon
        Ok(())
    }
} 