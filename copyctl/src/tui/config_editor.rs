use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub struct ConfigEditor;

impl ConfigEditor {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let config_text = "Configuration editor coming soon...";
        let paragraph = Paragraph::new(config_text)
            .block(Block::default().title("Configuration").borders(Borders::ALL));
        f.render_widget(paragraph, area);
    }

    pub async fn handle_key_event(&mut self, _key: KeyEvent) -> Result<bool> {
        // TODO: Implement config editing
        Ok(false)
    }
} 