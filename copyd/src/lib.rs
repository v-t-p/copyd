#![allow(dead_code)]

pub mod checkpoint;
pub mod config;
pub mod copy_engine;
pub mod daemon;
pub mod directory;
pub mod error;
pub mod io_uring_engine;
pub mod job;
pub mod metrics;
pub mod monitor;
pub mod profiler;
pub mod regex_rename;
pub mod sparse;
pub mod verify;
// pub mod scheduler;
pub mod security;
// pub mod transfer_manager;

// Re-export commonly used types
pub use error::{CopydError, CopydResult, ErrorContext};
pub use security::{SecurityConfig, SecurityValidator};
pub use profiler::{PerformanceProfiler, PerformanceReport};
pub use config::Config;
pub use job::{Job};
pub use copyd_protocol::{JobStatus, CopyEngine};
pub use regex_rename::RegexRenamer;
// Additional re-exports to simplify external usage and keep integration tests working
pub use daemon::Daemon;
pub use job::JobManager;
pub use copy_engine::{FileCopyEngine, CopyOptions};
pub use checkpoint::{CheckpointManager, JobCheckpoint, FileCheckpoint};
pub use directory::DirectoryHandler;
pub use sparse::SparseFileHandler;
pub use verify::{FileVerifier, VerifyMode};

// Expose the protocol crate directly for convenience (e.g., copyd::protocol::CreateJobRequest)
pub use copyd_protocol as protocol; 