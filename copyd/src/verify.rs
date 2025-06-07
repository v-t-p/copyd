use anyhow::{Result, Context};
use std::path::Path;
use sha2::{Sha256, Digest};
use md5::Md5 as Md5Hasher;
use tokio::io::AsyncReadExt;
use tracing::{info, debug};

#[derive(Debug, Clone, Copy)]
pub enum VerifyMode {
    None = 0,
    Size = 1,
    Md5 = 2,
    Sha256 = 3,
}

impl From<i32> for VerifyMode {
    fn from(value: i32) -> Self {
        match value {
            0 => VerifyMode::None,
            1 => VerifyMode::Size,
            2 => VerifyMode::Md5,
            3 => VerifyMode::Sha256,
            _ => VerifyMode::None,
        }
    }
}

impl From<copyd_protocol::VerifyMode> for VerifyMode {
    fn from(value: copyd_protocol::VerifyMode) -> Self {
        match value {
            copyd_protocol::VerifyMode::None => VerifyMode::None,
            copyd_protocol::VerifyMode::Size => VerifyMode::Size,
            copyd_protocol::VerifyMode::Md5 => VerifyMode::Md5,
            copyd_protocol::VerifyMode::Sha256 => VerifyMode::Sha256,
        }
    }
}

pub struct FileVerifier;

impl FileVerifier {
    pub async fn verify_copy(
        source: &Path,
        destination: &Path,
        mode: VerifyMode,
    ) -> Result<bool> {
        match mode {
            VerifyMode::None => {
                debug!("No verification requested");
                Ok(true)
            }
            VerifyMode::Size => {
                Self::verify_size(source, destination).await
            }
            VerifyMode::Md5 => {
                Self::verify_md5(source, destination).await
            }
            VerifyMode::Sha256 => {
                Self::verify_sha256(source, destination).await
            }
        }
    }

    async fn verify_size(source: &Path, destination: &Path) -> Result<bool> {
        info!("Verifying file sizes");
        
        let source_metadata = tokio::fs::metadata(source).await
            .with_context(|| format!("Failed to get source metadata: {:?}", source))?;
        
        let dest_metadata = tokio::fs::metadata(destination).await
            .with_context(|| format!("Failed to get destination metadata: {:?}", destination))?;
        
        let sizes_match = source_metadata.len() == dest_metadata.len();
        
        if sizes_match {
            info!("Size verification passed: {} bytes", source_metadata.len());
        } else {
            info!("Size verification failed: source {} bytes, dest {} bytes", 
                  source_metadata.len(), dest_metadata.len());
        }
        
        Ok(sizes_match)
    }

    async fn verify_md5(source: &Path, destination: &Path) -> Result<bool> {
        info!("Verifying with MD5 checksums");
        
        let source_hash = Self::calculate_md5(source).await?;
        let dest_hash = Self::calculate_md5(destination).await?;
        
        let hashes_match = source_hash == dest_hash;
        
        if hashes_match {
            info!("MD5 verification passed: {}", source_hash);
        } else {
            info!("MD5 verification failed: source {}, dest {}", source_hash, dest_hash);
        }
        
        Ok(hashes_match)
    }

    async fn verify_sha256(source: &Path, destination: &Path) -> Result<bool> {
        info!("Verifying with SHA256 checksums");
        
        let source_hash = Self::calculate_sha256(source).await?;
        let dest_hash = Self::calculate_sha256(destination).await?;
        
        let hashes_match = source_hash == dest_hash;
        
        if hashes_match {
            info!("SHA256 verification passed: {}", source_hash);
        } else {
            info!("SHA256 verification failed: source {}, dest {}", source_hash, dest_hash);
        }
        
        Ok(hashes_match)
    }

    async fn calculate_md5(file_path: &Path) -> Result<String> {
        let mut file = tokio::fs::File::open(file_path).await
            .with_context(|| format!("Failed to open file for MD5: {:?}", file_path))?;
        
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).await?;
        
        let digest = md5::compute(&contents);
        Ok(format!("{:x}", digest))
    }

    async fn calculate_sha256(file_path: &Path) -> Result<String> {
        let mut file = tokio::fs::File::open(file_path).await
            .with_context(|| format!("Failed to open file for SHA256: {:?}", file_path))?;
        
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 8192];
        
        loop {
            let bytes_read = file.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }
        
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub async fn calculate_checksum(file_path: &Path, mode: VerifyMode) -> Result<String> {
        match mode {
            VerifyMode::Md5 => Self::calculate_md5(file_path).await,
            VerifyMode::Sha256 => Self::calculate_sha256(file_path).await,
            VerifyMode::Size => {
                let metadata = tokio::fs::metadata(file_path).await?;
                Ok(metadata.len().to_string())
            }
            VerifyMode::None => Ok(String::new()),
        }
    }
} 