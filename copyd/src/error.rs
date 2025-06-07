use std::path::PathBuf;
use thiserror::Error;

/// Comprehensive error types for copyd operations
#[derive(Error, Debug)]
pub enum CopydError {
    // File system errors
    #[error("File not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("Permission denied accessing: {path}")]
    PermissionDenied { path: PathBuf },

    #[error("Invalid file path: {path}")]
    InvalidPath { path: PathBuf },

    #[error("Destination already exists: {path}")]
    DestinationExists { path: PathBuf },

    #[error("Source and destination are the same: {path}")]
    SameSourceDestination { path: PathBuf },

    #[error("Cross-device operation not supported: {source} -> {destination}")]
    CrossDevice { source: PathBuf, destination: PathBuf },

    #[error("Insufficient disk space: need {required} bytes, available {available} bytes")]
    InsufficientSpace { required: u64, available: u64 },

    // Copy engine errors
    #[error("Copy engine '{engine}' failed: {reason}")]
    CopyEngineFailed { engine: String, reason: String },

    #[error("No suitable copy engine available")]
    NoSuitableCopyEngine,

    #[error("io_uring operation failed: {operation} - {reason}")]
    IoUringFailed { operation: String, reason: String },

    #[error("Copy verification failed: expected {expected}, got {actual}")]
    VerificationFailed { expected: String, actual: String },

    // Job management errors
    #[error("Job '{job_id}' not found")]
    JobNotFound { job_id: String },

    #[error("Job already exists: {job_id}")]
    JobAlreadyExists { job_id: String },

    #[error("Job is not in a valid state for this operation: {job_id} (current: {current_state})")]
    InvalidJobState { job_id: String, current_state: String },

    #[error("Maximum concurrent jobs reached: {max_jobs}")]
    MaxJobsReached { max_jobs: usize },

    #[error("Job queue is full")]
    JobQueueFull,

    // Checkpoint errors
    #[error("Checkpoint corrupted: {job_id} - {reason}")]
    CheckpointCorrupted { job_id: String, reason: String },

    #[error("Failed to save checkpoint: {job_id} - {reason}")]
    CheckpointSaveFailed { job_id: String, reason: String },

    #[error("Failed to load checkpoint: {job_id} - {reason}")]
    CheckpointLoadFailed { job_id: String, reason: String },

    #[error("Checkpoint version mismatch: expected {expected}, got {actual}")]
    CheckpointVersionMismatch { expected: String, actual: String },

    // Configuration errors
    #[error("Invalid configuration: {field} - {reason}")]
    InvalidConfiguration { field: String, reason: String },

    #[error("Configuration file not found: {path}")]
    ConfigurationNotFound { path: PathBuf },

    #[error("Failed to parse configuration: {reason}")]
    ConfigurationParseError { reason: String },

    // Network/IPC errors
    #[error("Failed to connect to daemon: {reason}")]
    DaemonConnectionFailed { reason: String },

    #[error("Daemon not running")]
    DaemonNotRunning,

    #[error("Protocol error: {message}")]
    ProtocolError { message: String },

    #[error("Authentication failed: {reason}")]
    AuthenticationFailed { reason: String },

    #[error("Request timeout after {timeout_ms}ms")]
    RequestTimeout { timeout_ms: u64 },

    // Regex errors
    #[error("Invalid regex pattern: {pattern} - {reason}")]
    InvalidRegexPattern { pattern: String, reason: String },

    #[error("Unsafe regex replacement: {replacement} - {reason}")]
    UnsafeRegexReplacement { replacement: String, reason: String },

    // Resource errors
    #[error("Resource limit exceeded: {resource} (limit: {limit}, current: {current})")]
    ResourceLimitExceeded { resource: String, limit: u64, current: u64 },

    #[error("Memory allocation failed: requested {size} bytes")]
    MemoryAllocationFailed { size: usize },

    #[error("File descriptor limit reached")]
    FileDescriptorLimitReached,

    // System errors
    #[error("System call failed: {syscall} - {errno}")]
    SystemCallFailed { syscall: String, errno: i32 },

    #[error("Kernel feature not supported: {feature}")]
    KernelFeatureNotSupported { feature: String },

    #[error("Systemd operation failed: {operation} - {reason}")]
    SystemdOperationFailed { operation: String, reason: String },

    // Validation errors
    #[error("Invalid input: {field} - {reason}")]
    InvalidInput { field: String, reason: String },

    #[error("Rate limit exceeded: {limit} operations per {window} seconds")]
    RateLimitExceeded { limit: u64, window: u64 },

    #[error("Operation cancelled by user")]
    OperationCancelled,

    #[error("Operation timed out after {seconds} seconds")]
    OperationTimeout { seconds: u64 },

    // Generic errors
    #[error("Internal error: {message}")]
    InternalError { message: String },

    #[error("Feature not implemented: {feature}")]
    NotImplemented { feature: String },

    #[error("Temporary failure, retry recommended: {reason}")]
    TemporaryFailure { reason: String },

    #[error("Monitoring error: {reason}")]
    MonitoringError { reason: String },

    #[error("Configuration error: {0}")]
    Config(#[from] toml::de::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Filesystem error on path {path:?}: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Verification failed for {0}: {1}")]
    Verification(PathBuf, String),

    #[error("Checkpoint error: {0}")]
    Checkpoint(String),

    #[error("Security violation: {0}")]
    Security(String),

    #[error("Daemon connection error: {0}")]
    DaemonConnection(String),

    #[error("RPC error: {0}")]
    Rpc(String),
    
    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Feature not supported: {0}")]
    Unsupported(String),
}

impl From<prometheus::Error> for CopydError {
    fn from(e: prometheus::Error) -> Self {
        CopydError::MonitoringError {
            reason: e.to_string(),
        }
    }
}

impl CopydError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            CopydError::TemporaryFailure { .. }
                | CopydError::InsufficientSpace { .. }
                | CopydError::ResourceLimitExceeded { .. }
                | CopydError::RequestTimeout { .. }
                | CopydError::DaemonConnectionFailed { .. }
                | CopydError::IoUringFailed { .. }
        )
    }

    /// Get the error severity level
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            CopydError::InternalError { .. } | CopydError::SystemCallFailed { .. } => {
                ErrorSeverity::Critical
            }
            CopydError::VerificationFailed { .. }
            | CopydError::CheckpointCorrupted { .. }
            | CopydError::AuthenticationFailed { .. }
            | CopydError::PermissionDenied { .. } => ErrorSeverity::High,
            CopydError::FileNotFound { .. }
            | CopydError::InvalidPath { .. }
            | CopydError::DestinationExists { .. }
            | CopydError::InvalidConfiguration { .. }
            | CopydError::InvalidInput { .. } => ErrorSeverity::Medium,
            CopydError::OperationCancelled
            | CopydError::NotImplemented { .. }
            | CopydError::TemporaryFailure { .. } => ErrorSeverity::Low,
            _ => ErrorSeverity::Medium,
        }
    }

    /// Get suggested user action
    pub fn suggested_action(&self) -> &'static str {
        match self {
            CopydError::FileNotFound { .. } => "Check that the source file exists and is accessible",
            CopydError::PermissionDenied { .. } => {
                "Check file permissions or run with appropriate privileges"
            }
            CopydError::DestinationExists { .. } => {
                "Use --overwrite, --skip, or --serial to handle existing files"
            }
            CopydError::InsufficientSpace { .. } => "Free up disk space on the destination",
            CopydError::DaemonNotRunning => "Start the copyd daemon: systemctl start copyd.socket",
            CopydError::InvalidRegexPattern { .. } => "Check regex pattern syntax",
            CopydError::RateLimitExceeded { .. } => "Wait before retrying the operation",
            CopydError::OperationTimeout { .. } => "Try again or increase timeout settings",
            CopydError::TemporaryFailure { .. } => "Retry the operation after a short delay",
            _ => "Check the error details and consult documentation",
        }
    }

    /// Convert to exit code for CLI applications
    pub fn exit_code(&self) -> i32 {
        match self.severity() {
            ErrorSeverity::Critical => 2,
            ErrorSeverity::High => 1,
            ErrorSeverity::Medium => 1,
            ErrorSeverity::Low => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    Critical,
    High,
    Medium,
    Low,
}

/// Result type alias for copyd operations
pub type CopydResult<T> = Result<T, CopydError>;

/// Helper trait for converting common errors to CopydError
pub trait ToCopydError<T> {
    fn to_copyd_error(self) -> CopydResult<T>;
}

impl<T> ToCopydError<T> for std::io::Result<T> {
    fn to_copyd_error(self) -> CopydResult<T> {
        self.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => CopydError::FileNotFound {
                path: PathBuf::from("<unknown>"),
            },
            std::io::ErrorKind::PermissionDenied => CopydError::PermissionDenied {
                path: PathBuf::from("<unknown>"),
            },
            std::io::ErrorKind::AlreadyExists => CopydError::DestinationExists {
                path: PathBuf::from("<unknown>"),
            },
            _ => CopydError::InternalError {
                message: e.to_string(),
            },
        })
    }
}

/// Error context builder for better error reporting
pub struct ErrorContext {
    operation: String,
    path: Option<PathBuf>,
    job_id: Option<String>,
    additional_info: Vec<(String, String)>,
}

impl ErrorContext {
    pub fn new(operation: &str) -> Self {
        Self {
            operation: operation.to_string(),
            path: None,
            job_id: None,
            additional_info: Vec::new(),
        }
    }

    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_job_id(mut self, job_id: impl Into<String>) -> Self {
        self.job_id = Some(job_id.into());
        self
    }

    pub fn with_info(mut self, key: &str, value: &str) -> Self {
        self.additional_info.push((key.to_string(), value.to_string()));
        self
    }

    pub fn build(self, error: CopydError) -> anyhow::Error {
        let mut context = anyhow::Error::new(error);
        
        context = context.context(format!("Operation: {}", self.operation));
        
        if let Some(path) = self.path {
            context = context.context(format!("Path: {}", path.display()));
        }
        
        if let Some(job_id) = self.job_id {
            context = context.context(format!("Job ID: {}", job_id));
        }
        
        for (key, value) in self.additional_info {
            context = context.context(format!("{}: {}", key, value));
        }
        
        context
    }
}

/// Macro for creating error contexts
#[macro_export]
macro_rules! error_context {
    ($op:expr) => {
        $crate::error::ErrorContext::new($op)
    };
    ($op:expr, path = $path:expr) => {
        $crate::error::ErrorContext::new($op).with_path($path)
    };
    ($op:expr, job_id = $job_id:expr) => {
        $crate::error::ErrorContext::new($op).with_job_id($job_id)
    };
    ($op:expr, path = $path:expr, job_id = $job_id:expr) => {
        $crate::error::ErrorContext::new($op).with_path($path).with_job_id($job_id)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_severity() {
        let error = CopydError::FileNotFound {
            path: PathBuf::from("/test"),
        };
        assert_eq!(error.severity(), ErrorSeverity::Medium);
        assert!(error.exit_code() == 1);
    }

    #[test]
    fn test_retryable_errors() {
        let retryable = CopydError::TemporaryFailure {
            reason: "test".to_string(),
        };
        assert!(retryable.is_retryable());

        let non_retryable = CopydError::FileNotFound {
            path: PathBuf::from("/test"),
        };
        assert!(!non_retryable.is_retryable());
    }

    #[test]
    fn test_error_context() {
        let error = CopydError::FileNotFound {
            path: PathBuf::from("/test"),
        };
        
        let context = error_context!("copy operation", path = "/source", job_id = "job123");
        let full_error = context.build(error);
        
        let error_string = format!("{:#}", full_error);
        assert!(error_string.contains("copy operation"));
        assert!(error_string.contains("/source"));
        assert!(error_string.contains("job123"));
    }
} 