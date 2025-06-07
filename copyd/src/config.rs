use anyhow::{Result};
use serde::{Deserialize, Serialize};
use std::path::{PathBuf};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub socket_path: PathBuf,
    pub max_concurrent_jobs: usize,
    pub max_job_queue_size: usize,
    pub default_block_size: u64,
    pub max_rate_mbps: Option<u64>,
    pub metrics_bind_addr: Option<String>,
    pub log_level: String,
    pub job_history_days: u32,
    pub checkpoint_interval_secs: u64,
    pub temp_dir: PathBuf,
    pub enable_compression: bool,
    pub enable_encryption: bool,
    pub io_uring_entries: u32,
    pub watchdog_enabled: bool,
    pub checkpoint_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/run/copyd/copyd.sock"),
            max_concurrent_jobs: num_cpus::get(),
            max_job_queue_size: 1000,
            default_block_size: 1024 * 1024, // 1MB
            max_rate_mbps: None,
            metrics_bind_addr: Some("127.0.0.1:9090".to_string()),
            log_level: "info".to_string(),
            job_history_days: 30,
            checkpoint_interval_secs: 5,
            temp_dir: PathBuf::from("/tmp/copyd"),
            enable_compression: false,
            enable_encryption: false,
            io_uring_entries: 256,
            watchdog_enabled: true,
            checkpoint_dir: PathBuf::from("/var/lib/copyd/checkpoints"),
        }
    }
}

impl Config {
    pub async fn load() -> Result<Self> {
        let config_path = std::env::var("COPYD_CONFIG_PATH")
            .unwrap_or_else(|_| "/etc/copyd/config.toml".to_string());
        
        match tokio::fs::read_to_string(&config_path).await {
            Ok(content) => {
                let config: Config = toml::from_str(&content)?;
                Ok(config)
            }
            Err(_) => {
                // If config file doesn't exist or fails to load, use defaults
                warn!("Configuration file not found at {}. Using default settings.", config_path);
                Ok(Config::default())
            }
        }
    }

    pub async fn ensure_directories(&self) -> Result<()> {
        if let Some(parent) = self.socket_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::create_dir_all(&self.temp_dir).await?;
        tokio::fs::create_dir_all(&self.checkpoint_dir).await?;
        Ok(())
    }
} 