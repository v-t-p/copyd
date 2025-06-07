use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod client;
mod tui;
mod cli;
mod protocol;

use client::CopyClient;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(name = "copyctl")]
struct Cli {
    /// Socket path to connect to copyd daemon
    #[arg(short, long, default_value = "/run/copyd/copyd.sock")]
    socket: PathBuf,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Output format (text, json)
    #[arg(short, long, default_value = "text")]
    format: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Copy files or directories
    Copy {
        /// Source files or directories
        sources: Vec<PathBuf>,
        /// Destination
        destination: PathBuf,
        /// Copy directories recursively
        #[arg(short, long)]
        recursive: bool,
        /// Preserve metadata (permissions, timestamps, etc.)
        #[arg(short, long)]
        preserve: bool,
        /// Preserve hard links
        #[arg(long)]
        preserve_links: bool,
        /// Preserve sparse file regions
        #[arg(long)]
        preserve_sparse: bool,
        /// Verification method (none, size, md5, sha256)
        #[arg(long, default_value = "none")]
        verify: String,
        /// What to do if destination exists (overwrite, skip, serial)
        #[arg(long, default_value = "overwrite")]
        exists: String,
        /// Job priority (higher = processed first)
        #[arg(long, default_value = "100")]
        priority: u32,
        /// Maximum transfer rate in MB/s
        #[arg(long)]
        max_rate: Option<u64>,
        /// Copy engine to use (auto, io_uring, copy_file_range, sendfile, reflink, read_write)
        #[arg(long, default_value = "auto")]
        engine: String,
        /// Dry run - don't actually copy files
        #[arg(long)]
        dry_run: bool,
        /// Regex pattern for renaming files
        #[arg(long)]
        regex_rename_match: Option<String>,
        /// Replacement pattern for renaming files
        #[arg(long)]
        regex_rename_replace: Option<String>,
        /// Block size for I/O operations
        #[arg(long)]
        block_size: Option<u64>,
        /// Enable compression
        #[arg(long)]
        compress: bool,
        /// Enable encryption
        #[arg(long)]
        encrypt: bool,
        /// Monitor job progress
        #[arg(short, long)]
        monitor: bool,
    },
    /// Move files or directories
    Move {
        /// Source files or directories
        sources: Vec<PathBuf>,
        /// Destination
        destination: PathBuf,
        /// Copy directories recursively
        #[arg(short, long)]
        recursive: bool,
        /// Preserve metadata (permissions, timestamps, etc.)
        #[arg(short, long)]
        preserve: bool,
        /// Preserve hard links
        #[arg(long)]
        preserve_links: bool,
        /// Preserve sparse file regions
        #[arg(long)]
        preserve_sparse: bool,
        /// Verification method (none, size, md5, sha256)
        #[arg(long, default_value = "none")]
        verify: String,
        /// What to do if destination exists (overwrite, skip, serial)
        #[arg(long, default_value = "overwrite")]
        exists: String,
        /// Job priority (higher = processed first)
        #[arg(long, default_value = "100")]
        priority: u32,
        /// Maximum transfer rate in MB/s
        #[arg(long)]
        max_rate: Option<u64>,
        /// Copy engine to use (auto, io_uring, copy_file_range, sendfile, reflink, read_write)
        #[arg(long, default_value = "auto")]
        engine: String,
        /// Dry run - don't actually move files
        #[arg(long)]
        dry_run: bool,
        /// Regex pattern for renaming files
        #[arg(long)]
        regex_rename_match: Option<String>,
        /// Replacement pattern for renaming files
        #[arg(long)]
        regex_rename_replace: Option<String>,
        /// Block size for I/O operations
        #[arg(long)]
        block_size: Option<u64>,
        /// Enable compression
        #[arg(long)]
        compress: bool,
        /// Enable encryption
        #[arg(long)]
        encrypt: bool,
        /// Monitor job progress
        #[arg(short, long)]
        monitor: bool,
    },
    /// List jobs
    List {
        /// Include completed jobs
        #[arg(short, long)]
        completed: bool,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },
    /// Show job status
    Status {
        /// Job ID
        job_id: String,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Monitor job progress
        #[arg(short, long)]
        monitor: bool,
    },
    /// Cancel a job
    Cancel {
        /// Job ID
        job_id: String,
    },
    /// Pause a job
    Pause {
        /// Job ID
        job_id: String,
    },
    /// Resume a job
    Resume {
        /// Job ID
        job_id: String,
    },
    /// Show daemon statistics
    Stats {
        /// Number of days to include
        #[arg(short, long, default_value = "7")]
        days: i32,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },
    /// TUI monitor mode
    Monitor,
    /// Navigator mode (dual-pane file browser)
    Navigator,
    /// Health check
    Health,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    let filter = if cli.verbose {
        "copyctl=debug"
    } else {
        "copyctl=info"
    };

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(filter))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Create client
    let client = CopyClient::new(cli.socket).await?;

    // Execute command
    match cli.command {
        Commands::Copy { 
            sources, destination, recursive, preserve, preserve_links, preserve_sparse,
            verify, exists, priority, max_rate, engine, dry_run, 
            regex_rename_match, regex_rename_replace, block_size, compress, encrypt, monitor 
        } => {
            cli::handle_copy(
                client, sources, destination, recursive, preserve, preserve_links, 
                preserve_sparse, verify, exists, priority, max_rate, engine, dry_run,
                regex_rename_match, regex_rename_replace, block_size, compress, encrypt, 
                monitor, &cli.format
            ).await?;
        }
        Commands::Move { 
            sources, destination, recursive, preserve, preserve_links, preserve_sparse,
            verify, exists, priority, max_rate, engine, dry_run, 
            regex_rename_match, regex_rename_replace, block_size, compress, encrypt, monitor 
        } => {
            // For move, we'll copy then delete the originals
            cli::handle_move(
                client, sources, destination, recursive, preserve, preserve_links, 
                preserve_sparse, verify, exists, priority, max_rate, engine, dry_run,
                regex_rename_match, regex_rename_replace, block_size, compress, encrypt, 
                monitor, &cli.format
            ).await?;
        }
        Commands::List { completed, json } => {
            cli::handle_list(client, completed, json, &cli.format).await?;
        }
        Commands::Status { job_id, json, monitor } => {
            cli::handle_status(client, job_id, json, monitor, &cli.format).await?;
        }
        Commands::Cancel { job_id } => {
            cli::handle_cancel(client, job_id, &cli.format).await?;
        }
        Commands::Pause { job_id } => {
            cli::handle_pause(client, job_id, &cli.format).await?;
        }
        Commands::Resume { job_id } => {
            cli::handle_resume(client, job_id, &cli.format).await?;
        }
        Commands::Stats { days, json } => {
            cli::handle_stats(client, days, json, &cli.format).await?;
        }
        Commands::Monitor => {
            tui::run_monitor(client).await?;
        }
        Commands::Navigator => {
            tui::run_navigator(client).await?;
        }
        Commands::Health => {
            cli::handle_health(client, &cli.format).await?;
        }
    }

    Ok(())
} 