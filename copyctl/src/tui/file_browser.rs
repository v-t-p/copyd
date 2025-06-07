use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::path::{Path, PathBuf};
use std::fs;
use tokio::fs as async_fs;
use tracing::{info, warn, error};
use std::os::unix::fs::PermissionsExt;
use dirs;

use crate::client::CopyClient;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub permissions: String,
}

impl FileEntry {
    pub fn from_path(path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path)?;
        let name = path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        
        let permissions = format!("{:o}", metadata.permissions().mode() & 0o777);
        
        Ok(Self {
            name,
            path: path.to_path_buf(),
            is_dir: metadata.is_dir(),
            size: metadata.len(),
            permissions,
        })
    }

    pub fn display_size(&self) -> String {
        if self.is_dir {
            "<DIR>".to_string()
        } else {
            format_size(self.size)
        }
    }
}

pub struct FilePane {
    pub current_dir: PathBuf,
    pub entries: Vec<FileEntry>,
    pub selected_index: usize,
    pub list_state: ListState,
    pub is_active: bool,
}

impl FilePane {
    pub fn new(path: PathBuf) -> Result<Self> {
        let mut pane = Self {
            current_dir: path,
            entries: Vec::new(),
            selected_index: 0,
            list_state: ListState::default(),
            is_active: false,
        };
        pane.refresh()?;
        Ok(pane)
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.entries.clear();
        
        // Add parent directory entry if not at root
        if let Some(parent) = self.current_dir.parent() {
            self.entries.push(FileEntry {
                name: "..".to_string(),
                path: parent.to_path_buf(),
                is_dir: true,
                size: 0,
                permissions: "755".to_string(),
            });
        }

        // Read directory entries
        let entries = fs::read_dir(&self.current_dir)?;
        for entry in entries {
            let entry = entry?;
            if let Ok(file_entry) = FileEntry::from_path(&entry.path()) {
                self.entries.push(file_entry);
            }
        }

        // Sort entries: directories first, then by name
        self.entries.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            }
        });

        // Ensure selected index is valid
        if self.selected_index >= self.entries.len() && !self.entries.is_empty() {
            self.selected_index = self.entries.len() - 1;
        }
        
        self.list_state.select(Some(self.selected_index));
        Ok(())
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.list_state.select(Some(self.selected_index));
        }
    }

    pub fn move_down(&mut self) {
        if self.selected_index < self.entries.len().saturating_sub(1) {
            self.selected_index += 1;
            self.list_state.select(Some(self.selected_index));
        }
    }

    pub fn enter_directory(&mut self) -> Result<()> {
        if let Some(entry) = self.entries.get(self.selected_index) {
            if entry.is_dir {
                self.current_dir = entry.path.clone();
                self.selected_index = 0;
                self.refresh()?;
            }
        }
        Ok(())
    }

    pub fn get_selected_entry(&self) -> Option<&FileEntry> {
        self.entries.get(self.selected_index)
    }

    pub fn get_selected_files(&self) -> Vec<&FileEntry> {
        // For now, just return the selected file
        // TODO: Implement multi-selection
        if let Some(entry) = self.get_selected_entry() {
            vec![entry]
        } else {
            vec![]
        }
    }
}

pub struct FileBrowser {
    pub left_pane: FilePane,
    pub right_pane: FilePane,
    pub active_pane: usize, // 0 = left, 1 = right
}

impl FileBrowser {
    pub fn new() -> Result<Self> {
        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let current_dir = std::env::current_dir().unwrap_or(home_dir.clone());
        
        let mut left_pane = FilePane::new(current_dir)?;
        let right_pane = FilePane::new(home_dir)?;
        
        left_pane.is_active = true;
        
        Ok(Self {
            left_pane,
            right_pane,
            active_pane: 0,
        })
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Draw left pane
        Self::draw_pane(f, chunks[0], &mut self.left_pane, self.active_pane == 0);
        
        // Draw right pane
        Self::draw_pane(f, chunks[1], &mut self.right_pane, self.active_pane == 1);
    }

    fn draw_pane(f: &mut Frame, area: Rect, pane: &mut FilePane, is_active: bool) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);

        // Draw path header
        let path_text = format!(" {} ", pane.current_dir.display());
        let path_style = if is_active {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let path_block = Block::default()
            .borders(Borders::ALL)
            .border_style(if is_active {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Gray)
            });

        let path_paragraph = Paragraph::new(path_text)
            .block(path_block)
            .style(path_style);
        
        f.render_widget(path_paragraph, layout[0]);

        // Draw file list
        let items: Vec<ListItem> = pane.entries.iter().enumerate().map(|(_, entry)| {
            let mut spans = Vec::new();
            
            // Icon based on file type
            let icon = if entry.name == ".." {
                "📁"
            } else if entry.is_dir {
                "📂"
            } else {
                match entry.path.extension().and_then(|s| s.to_str()) {
                    Some("txt") | Some("md") | Some("rst") => "📄",
                    Some("rs") | Some("py") | Some("js") | Some("c") | Some("cpp") => "📝",
                    Some("jpg") | Some("png") | Some("gif") | Some("bmp") => "🖼️",
                    Some("mp3") | Some("wav") | Some("flac") => "🎵",
                    Some("mp4") | Some("avi") | Some("mkv") => "🎬",
                    Some("zip") | Some("tar") | Some("gz") => "📦",
                    _ => "📄",
                }
            };

            spans.push(Span::raw(format!("{} ", icon)));
            
            // File name
            let name_style = if entry.is_dir {
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            spans.push(Span::styled(format!("{:<30}", entry.name), name_style));
            
            // File size
            spans.push(Span::styled(
                format!(" {:<10}", entry.display_size()),
                Style::default().fg(Color::Cyan),
            ));
            
            // Permissions
            spans.push(Span::styled(
                format!(" {}", entry.permissions),
                Style::default().fg(Color::Green),
            ));

            ListItem::new(Line::from(spans))
        }).collect();

        let list_block = Block::default()
            .borders(Borders::ALL)
            .border_style(if is_active {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Gray)
            });

        let list = List::new(items)
            .block(list_block)
            .highlight_style(
                Style::default()
                    .bg(if is_active { Color::Yellow } else { Color::DarkGray })
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            );

        f.render_stateful_widget(list, layout[1], &mut pane.list_state);
    }

    pub async fn handle_key_event(&mut self, key: KeyEvent, client: &mut CopyClient) -> Result<bool> {
        match key.code {
            KeyCode::Up => {
                self.get_active_pane_mut().move_up();
            }
            KeyCode::Down => {
                self.get_active_pane_mut().move_down();
            }
            KeyCode::Enter => {
                self.get_active_pane_mut().enter_directory()?;
            }
            KeyCode::Tab => {
                self.switch_pane();
            }
            KeyCode::Char('r') => {
                self.get_active_pane_mut().refresh()?;
            }
            KeyCode::F(5) => {
                // Copy operation
                return self.copy_selected_files(client).await;
            }
            KeyCode::F(6) => {
                // Move operation
                return self.move_selected_files(client).await;
            }
            KeyCode::Delete => {
                // Delete operation
                return self.delete_selected_files().await;
            }
            KeyCode::F(7) => {
                // Create directory
                // TODO: Implement directory creation dialog
                info!("Create directory not yet implemented");
            }
            KeyCode::Char('h') => {
                // Go to home directory
                let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                self.get_active_pane_mut().current_dir = home_dir;
                self.get_active_pane_mut().refresh()?;
            }
            _ => {}
        }
        Ok(false)
    }

    pub async fn update(&mut self) -> Result<()> {
        // Refresh active pane if needed
        // This could be extended to watch for file system changes
        Ok(())
    }

    fn get_active_pane_mut(&mut self) -> &mut FilePane {
        match self.active_pane {
            0 => &mut self.left_pane,
            _ => &mut self.right_pane,
        }
    }

    fn get_inactive_pane(&self) -> &FilePane {
        match self.active_pane {
            0 => &self.right_pane,
            _ => &self.left_pane,
        }
    }

    fn switch_pane(&mut self) {
        self.left_pane.is_active = !self.left_pane.is_active;
        self.right_pane.is_active = !self.right_pane.is_active;
        self.active_pane = 1 - self.active_pane;
    }

    async fn copy_selected_files(&mut self, client: &mut CopyClient) -> Result<bool> {
        let destination_dir = self.get_inactive_pane().current_dir.clone();
        let source_files = self.get_active_pane_mut().get_selected_files();

        if source_files.is_empty() {
            warn!("No files selected for copy");
            return Ok(false);
        }

        info!("Copying {} files to {:?}", source_files.len(), destination_dir);

        for file in source_files {
            if file.name == ".." {
                continue; // Skip parent directory
            }

            let destination = destination_dir.join(&file.name);
            info!("Copying {:?} to {:?}", file.path, destination);

            // Create copy job via daemon
            let request = copyd_protocol::CreateJobRequest {
                sources: vec![file.path.to_string_lossy().to_string()],
                destination: destination.to_string_lossy().to_string(),
                recursive: file.is_dir,
                preserve_metadata: true,
                ..Default::default()
            };
            let result = client.create_job(request).await;

            match result {
                Ok(job_id) => {
                    info!("Created copy job: {}", job_id);
                }
                Err(e) => {
                    error!("Failed to create copy job: {}", e);
                    return Ok(false);
                }
            }
        }

        // Refresh both panes to show updated file lists
        self.left_pane.refresh()?;
        self.right_pane.refresh()?;

        Ok(true)
    }

    async fn move_selected_files(&mut self, client: &mut CopyClient) -> Result<bool> {
        let destination_dir = self.get_inactive_pane().current_dir.clone();
        let source_files = self.get_active_pane_mut().get_selected_files();

        if source_files.is_empty() {
            warn!("No files selected for move");
            return Ok(false);
        }

        info!("Moving {} files to {:?}", source_files.len(), destination_dir);

        for file in source_files {
            if file.name == ".." {
                continue; // Skip parent directory
            }

            let destination = destination_dir.join(&file.name);
            info!("Moving {:?} to {:?}", file.path, destination);

            // For now, implement move as copy + delete
            // TODO: Implement proper move operation in daemon
            let request = copyd_protocol::CreateJobRequest {
                sources: vec![file.path.to_string_lossy().to_string()],
                destination: destination.to_string_lossy().to_string(),
                recursive: file.is_dir,
                preserve_metadata: true,
                ..Default::default()
            };
            let result = client.create_job(request).await;

            match result {
                Ok(job_id) => {
                    info!("Created move job: {}", job_id);
                    // TODO: Delete source after successful copy
                }
                Err(e) => {
                    error!("Failed to create move job: {}", e);
                    return Ok(false);
                }
            }
        }

        // Refresh both panes
        self.left_pane.refresh()?;
        self.right_pane.refresh()?;

        Ok(true)
    }

    async fn delete_selected_files(&mut self) -> Result<bool> {
        let source_files = self.get_active_pane_mut().get_selected_files();

        if source_files.is_empty() {
            warn!("No files selected for deletion");
            return Ok(false);
        }

        info!("Deleting {} files", source_files.len());

        for file in source_files {
            if file.name == ".." {
                continue; // Skip parent directory
            }

            info!("Deleting {:?}", file.path);

            let result = if file.is_dir {
                async_fs::remove_dir_all(&file.path).await
            } else {
                async_fs::remove_file(&file.path).await
            };

            match result {
                Ok(_) => {
                    info!("Deleted {:?}", file.path);
                }
                Err(e) => {
                    error!("Failed to delete {:?}: {}", file.path, e);
                    return Ok(false);
                }
            }
        }

        // Refresh active pane
        self.get_active_pane_mut().refresh()?;

        Ok(true)
    }
}

fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = size as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
} 