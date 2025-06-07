use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

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
        }
    }
}

impl Config {
    pub async fn load() -> Result<Self> {
        let config_paths = [
            PathBuf::from("/etc/copyd/copyd.toml"),
            PathBuf::from("/usr/local/etc/copyd/copyd.toml"),
            dirs::config_dir().map(|p| p.join("copyd/copyd.toml")),
        ];

        for path in config_paths.iter().filter_map(|p| p.as_ref()) {
            if path.exists() {
                let content = fs::read_to_string(path).await?;
                let config: Config = toml::from_str(&content)?;
                return Ok(config);
            }
        }

        // Use default config if no file found
        Ok(Config::default())
    }

    pub async fn ensure_directories(&self) -> Result<()> {
        // Create socket directory
        if let Some(socket_dir) = self.socket_path.parent() {
            fs::create_dir_all(socket_dir).await?;
        }

        // Create temp directory
        fs::create_dir_all(&self.temp_dir).await?;

        Ok(())
    }
} 