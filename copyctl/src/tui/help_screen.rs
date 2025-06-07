use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    backend::Backend,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

pub struct HelpScreen;

impl HelpScreen {
    pub fn new() -> Self {
        Self
    }

    pub fn draw<B: Backend>(&mut self, f: &mut Frame<B>, area: Rect) {
        let help_text = vec![
            Line::from(vec![
                Span::styled("copyctl - Modern File Operations TUI", 
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Global Keys:", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            ]),
            Line::from("  Tab / Shift+Tab  - Switch between tabs"),
            Line::from("  F1-F4           - Direct tab navigation"),
            Line::from("  Ctrl+Q          - Quit application"),
            Line::from("  ?               - Show this help"),
            Line::from(""),
            Line::from(vec![
                Span::styled("File Browser:", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            ]),
            Line::from("  ↑/↓             - Navigate files"),
            Line::from("  Enter           - Enter directory"),
            Line::from("  Tab             - Switch panes"),
            Line::from("  F5              - Copy selected files"),
            Line::from("  F6              - Move selected files"),
            Line::from("  Delete          - Delete selected files"),
            Line::from("  F7              - Create directory"),
            Line::from("  H               - Go to home directory"),
            Line::from("  R               - Refresh current pane"),
            Line::from(""),
            Line::from(vec![
                Span::styled("Press any key to return", 
                    Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC))
            ]),
        ];

        let help_paragraph = Paragraph::new(help_text)
            .block(Block::default().title("Help - copyctl TUI").borders(Borders::ALL))
            .wrap(Wrap { trim: true });

        f.render_widget(help_paragraph, area);
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) {
        // Any key closes help (handled in main app)
    }
} 