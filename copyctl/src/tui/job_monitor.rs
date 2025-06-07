use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{
    backend::Backend,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
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

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Job Monitor")
            .borders(Borders::ALL);
        
        let items: Vec<ListItem> = self.jobs.iter()
            .map(|job| ListItem::new(Line::from(Span::raw(job))))
            .collect();
        
        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }

    pub async fn handle_key_event(&mut self, _key: KeyEvent, _client: &mut CopyClient) -> Result<()> {
        // TODO: Implement job control
        Ok(())
    }

    pub async fn update(&mut self, _client: &mut CopyClient) -> Result<()> {
        // TODO: Refresh job list from daemon
        Ok(())
    }
} 