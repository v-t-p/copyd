use anyhow::{Result, Context};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, debug, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCheckpoint {
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    pub bytes_copied: u64,
    pub total_size: u64,
    pub last_modified: u64, // Unix timestamp
    pub checksum_partial: Option<String>, // Partial checksum for verification
    pub chunk_size: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobCheckpoint {
    pub job_id: String,
    pub operation_type: String, // "copy" or "move"
    pub files: HashMap<String, FileCheckpoint>, // file_id -> checkpoint
    pub completed_files: Vec<String>,
    pub failed_files: Vec<String>,
    pub total_files: usize,
    pub total_bytes: u64,
    pub bytes_completed: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub resume_count: u32,
}

impl JobCheckpoint {
    pub fn new(job_id: String, operation_type: String) -> Self {
        let now = now_unix_secs();

        Self {
            job_id,
            operation_type,
            files: HashMap::new(),
            completed_files: Vec::new(),
            failed_files: Vec::new(),
            total_files: 0,
            total_bytes: 0,
            bytes_completed: 0,
            created_at: now,
            updated_at: now,
            resume_count: 0,
        }
    }

    pub fn add_file(&mut self, file_id: String, checkpoint: FileCheckpoint) {
        self.total_bytes += checkpoint.total_size;
        self.total_files += 1;
        self.files.insert(file_id, checkpoint);
        self.update_timestamp();
    }

    pub fn update_file_progress(&mut self, file_id: &str, bytes_copied: u64, checksum: Option<String>) {
        if let Some(checkpoint) = self.files.get_mut(file_id) {
            let old_bytes = checkpoint.bytes_copied;
            checkpoint.bytes_copied = bytes_copied;
            checkpoint.checksum_partial = checksum;
            checkpoint.updated_at = now_unix_secs();

            // Update total progress
            self.bytes_completed = self.bytes_completed.saturating_sub(old_bytes) + bytes_copied;
            self.update_timestamp();
        }
    }

    pub fn complete_file(&mut self, file_id: String) {
        if let Some(checkpoint) = self.files.remove(&file_id) {
            self.completed_files.push(file_id);
            // Ensure bytes_completed accounts for this file
            if checkpoint.bytes_copied < checkpoint.total_size {
                self.bytes_completed += checkpoint.total_size - checkpoint.bytes_copied;
            }
            self.update_timestamp();
        }
    }

    pub fn fail_file(&mut self, file_id: String) {
        if let Some(_) = self.files.remove(&file_id) {
            self.failed_files.push(file_id);
            self.update_timestamp();
        }
    }

    pub fn get_progress(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.bytes_completed as f64 / self.total_bytes as f64) * 100.0
    }

    pub fn is_resumable(&self) -> bool {
        !self.files.is_empty() || !self.failed_files.is_empty()
    }

    pub fn increment_resume_count(&mut self) {
        self.resume_count += 1;
        self.update_timestamp();
    }

    fn update_timestamp(&mut self) {
        self.updated_at = now_unix_secs();
    }
}

pub struct CheckpointManager {
    checkpoint_dir: PathBuf,
}

impl CheckpointManager {
    pub fn new(checkpoint_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&checkpoint_dir)
            .with_context(|| format!("Failed to create checkpoint directory: {:?}", checkpoint_dir))?;

        Ok(Self { checkpoint_dir })
    }

    pub async fn save_checkpoint(&self, checkpoint: &JobCheckpoint) -> Result<()> {
        let checkpoint_file = self.checkpoint_dir.join(format!("{}.json", checkpoint.job_id));
        
        let json_data = serde_json::to_string_pretty(checkpoint)
            .with_context(|| "Failed to serialize checkpoint")?;

        let mut file = fs::File::create(&checkpoint_file).await
            .with_context(|| format!("Failed to create checkpoint file: {:?}", checkpoint_file))?;

        file.write_all(json_data.as_bytes()).await
            .with_context(|| "Failed to write checkpoint data")?;

        file.sync_all().await
            .with_context(|| "Failed to sync checkpoint file")?;

        debug!("Saved checkpoint for job {}", checkpoint.job_id);
        Ok(())
    }

    pub async fn load_checkpoint(&self, job_id: &str) -> Result<Option<JobCheckpoint>> {
        let checkpoint_file = self.checkpoint_dir.join(format!("{}.json", job_id));
        
        if !checkpoint_file.exists() {
            return Ok(None);
        }

        let mut file = fs::File::open(&checkpoint_file).await
            .with_context(|| format!("Failed to open checkpoint file: {:?}", checkpoint_file))?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).await
            .with_context(|| "Failed to read checkpoint file")?;

        let checkpoint: JobCheckpoint = serde_json::from_str(&contents)
            .with_context(|| "Failed to deserialize checkpoint")?;

        info!("Loaded checkpoint for job {} (resume count: {})", job_id, checkpoint.resume_count);
        Ok(Some(checkpoint))
    }

    pub async fn delete_checkpoint(&self, job_id: &str) -> Result<()> {
        let checkpoint_file = self.checkpoint_dir.join(format!("{}.json", job_id));
        
        if checkpoint_file.exists() {
            fs::remove_file(&checkpoint_file).await
                .with_context(|| format!("Failed to delete checkpoint file: {:?}", checkpoint_file))?;
            info!("Deleted checkpoint for job {}", job_id);
        }

        Ok(())
    }

    pub async fn list_resumable_jobs(&self) -> Result<Vec<String>> {
        let mut resumable_jobs = Vec::new();
        let mut entries = fs::read_dir(&self.checkpoint_dir).await
            .with_context(|| format!("Failed to read checkpoint directory: {:?}", self.checkpoint_dir))?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    // Verify the checkpoint is actually resumable
                    if let Ok(Some(checkpoint)) = self.load_checkpoint(stem).await {
                        if checkpoint.is_resumable() {
                            resumable_jobs.push(stem.to_string());
                        }
                    }
                }
            }
        }

        Ok(resumable_jobs)
    }

    pub async fn cleanup_old_checkpoints(&self, max_age_days: u64) -> Result<usize> {
        let cutoff_time = now_unix_secs().saturating_sub(max_age_days * 24 * 60 * 60);

        let mut cleaned_count = 0;
        let mut entries = fs::read_dir(&self.checkpoint_dir).await
            .with_context(|| format!("Failed to read checkpoint directory: {:?}", self.checkpoint_dir))?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(Some(checkpoint)) = self.load_checkpoint(stem).await {
                        if checkpoint.updated_at < cutoff_time {
                            self.delete_checkpoint(stem).await?;
                            cleaned_count += 1;
                        }
                    }
                }
            }
        }

        if cleaned_count > 0 {
            info!("Cleaned up {} old checkpoints", cleaned_count);
        }

        Ok(cleaned_count)
    }

    pub async fn get_checkpoint_stats(&self) -> Result<CheckpointStats> {
        let mut stats = CheckpointStats::default();
        let mut entries = fs::read_dir(&self.checkpoint_dir).await
            .with_context(|| format!("Failed to read checkpoint directory: {:?}", self.checkpoint_dir))?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(Some(checkpoint)) = self.load_checkpoint(stem).await {
                        stats.total_checkpoints += 1;
                        stats.total_bytes += checkpoint.total_bytes;
                        stats.completed_bytes += checkpoint.bytes_completed;
                        
                        if checkpoint.is_resumable() {
                            stats.resumable_jobs += 1;
                        }
                        
                        if checkpoint.resume_count > 0 {
                            stats.resumed_jobs += 1;
                        }
                    }
                }
            }
        }

        Ok(stats)
    }
}

#[derive(Debug, Default)]
pub struct CheckpointStats {
    pub total_checkpoints: usize,
    pub resumable_jobs: usize,
    pub resumed_jobs: usize,
    pub total_bytes: u64,
    pub completed_bytes: u64,
}

impl CheckpointStats {
    pub fn completion_rate(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.completed_bytes as f64 / self.total_bytes as f64) * 100.0
    }
}

// Helper function to create a file ID from source and destination paths
pub fn create_file_id(source: &Path, destination: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    destination.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

// Helper function to verify if a partial file can be resumed
pub async fn can_resume_file(checkpoint: &FileCheckpoint) -> Result<bool> {
    let dest_path = &checkpoint.destination_path;
    
    // Check if destination file exists
    if !dest_path.exists() {
        return Ok(false);
    }

    // Check if the file size matches our checkpoint
    let metadata = fs::metadata(dest_path).await
        .with_context(|| format!("Failed to get metadata for: {:?}", dest_path))?;

    if metadata.len() != checkpoint.bytes_copied {
        warn!("Destination file size mismatch: expected {}, found {}", 
              checkpoint.bytes_copied, metadata.len());
        return Ok(false);
    }

    // Check if source file still exists and hasn't changed
    let source_metadata = fs::metadata(&checkpoint.source_path).await
        .with_context(|| format!("Source file not found: {:?}", checkpoint.source_path))?;

    let source_modified = source_metadata.modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if source_modified != checkpoint.last_modified {
        warn!("Source file has been modified since checkpoint");
        return Ok(false);
    }

    if source_metadata.len() != checkpoint.total_size {
        warn!("Source file size has changed since checkpoint");
        return Ok(false);
    }

    Ok(true)
}

/// Return the current UNIX epoch seconds, falling back to 0 on clock error (pre-1970).
fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_checkpoint_creation() {
        let checkpoint = JobCheckpoint::new("test-job".to_string(), "copy".to_string());
        assert_eq!(checkpoint.job_id, "test-job");
        assert_eq!(checkpoint.operation_type, "copy");
        assert_eq!(checkpoint.total_files, 0);
        assert_eq!(checkpoint.total_bytes, 0);
    }

    #[tokio::test]
    async fn test_checkpoint_manager() {
        let temp_dir = TempDir::new().unwrap();
        let manager = CheckpointManager::new(temp_dir.path().to_path_buf()).unwrap();

        let mut checkpoint = JobCheckpoint::new("test-job".to_string(), "copy".to_string());
        checkpoint.total_bytes = 1000;
        checkpoint.bytes_completed = 500;

        // Save checkpoint
        manager.save_checkpoint(&checkpoint).await.unwrap();

        // Load checkpoint
        let loaded = manager.load_checkpoint("test-job").await.unwrap().unwrap();
        assert_eq!(loaded.job_id, "test-job");
        assert_eq!(loaded.total_bytes, 1000);
        assert_eq!(loaded.bytes_completed, 500);

        // Delete checkpoint
        manager.delete_checkpoint("test-job").await.unwrap();
        let deleted = manager.load_checkpoint("test-job").await.unwrap();
        assert!(deleted.is_none());
    }

    #[test]
    fn test_file_id_creation() {
        let source = Path::new("/tmp/source.txt");
        let dest = Path::new("/tmp/dest.txt");
        let id1 = create_file_id(source, dest);
        let id2 = create_file_id(source, dest);
        assert_eq!(id1, id2);

        let dest2 = Path::new("/tmp/dest2.txt");
        let id3 = create_file_id(source, dest2);
        assert_ne!(id1, id3);
    }
} 