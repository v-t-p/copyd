use crate::error::{CopydError, CopydResult};
use std::path::{Path, PathBuf};
use nix::unistd::{getuid, Uid};
use tracing::{warn, info};

/// Security configuration for copyd operations
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    pub max_file_size: u64,
    pub max_path_length: usize,
    pub blocked_extensions: Vec<String>,
    pub system_paths: Vec<PathBuf>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            max_file_size: 100 * 1024 * 1024 * 1024, // 100GB
            max_path_length: 4096,
            blocked_extensions: vec![
                ".exe".to_string(), ".bat".to_string(), ".cmd".to_string(),
            ],
            system_paths: vec![
                PathBuf::from("/proc"),
                PathBuf::from("/sys"),
                PathBuf::from("/dev"),
            ],
        }
    }
}

/// Security validator for file operations
pub struct SecurityValidator {
    config: SecurityConfig,
}

impl SecurityValidator {
    pub fn new(config: SecurityConfig) -> Self {
        Self { config }
    }

    /// Validate a file path for security issues
    pub fn validate_path(&self, path: &Path) -> CopydResult<()> {
        let path_str = path.to_string_lossy();

        // Check path length
        if path_str.len() > self.config.max_path_length {
            return Err(CopydError::InvalidPath {
                path: path.to_path_buf(),
            });
        }

        // Check for path traversal
        if path_str.contains("..") {
            warn!("Path traversal attempt detected: {}", path_str);
            return Err(CopydError::InvalidPath {
                path: path.to_path_buf(),
            });
        }

        // Check system paths
        for sys_path in &self.config.system_paths {
            if path.starts_with(sys_path) {
                return Err(CopydError::PermissionDenied {
                    path: path.to_path_buf(),
                });
            }
        }

        Ok(())
    }

    /// Validate file extension
    pub fn validate_extension(&self, path: &Path) -> CopydResult<()> {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            let ext_lower = format!(".{}", ext.to_lowercase());
            if self.config.blocked_extensions.contains(&ext_lower) {
                return Err(CopydError::InvalidInput {
                    field: "extension".to_string(),
                    reason: format!("Extension '{}' not allowed", ext),
                });
            }
        }
        Ok(())
    }

    /// Validate file size
    pub fn validate_size(&self, size: u64) -> CopydResult<()> {
        if size > self.config.max_file_size {
            return Err(CopydError::ResourceLimitExceeded {
                resource: "file_size".to_string(),
                limit: self.config.max_file_size,
                current: size,
            });
        }
        Ok(())
    }

    /// Check if user is privileged
    pub fn is_privileged(&self) -> bool {
        getuid() == Uid::from_raw(0)
    }

    /// Validate complete operation
    pub fn validate_operation(&self, sources: &[PathBuf], dest: &Path) -> CopydResult<()> {
        // Validate destination
        self.validate_path(dest)?;

        // Validate sources
        for source in sources {
            self.validate_path(source)?;
            self.validate_extension(source)?;
            
            // Check for same source/dest
            if source == dest {
                return Err(CopydError::SameSourceDestination {
                    path: source.clone(),
                });
            }
        }

        info!("Security validation passed for {} sources", sources.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_validation() {
        let validator = SecurityValidator::new(SecurityConfig::default());
        
        assert!(validator.validate_path(Path::new("/tmp/test.txt")).is_ok());
        assert!(validator.validate_path(Path::new("/tmp/../etc")).is_err());
        assert!(validator.validate_path(Path::new("/proc/version")).is_err());
    }

    #[test]
    fn test_extension_validation() {
        let validator = SecurityValidator::new(SecurityConfig::default());
        
        assert!(validator.validate_extension(Path::new("test.txt")).is_ok());
        assert!(validator.validate_extension(Path::new("malware.exe")).is_err());
    }
} 