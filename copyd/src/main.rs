use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod daemon;
mod job;
mod copy_engine;
mod io_uring_engine;
mod directory;
mod sparse;
mod verify;
mod metrics;
mod config;
mod utils;
mod checkpoint;

use daemon::Daemon;
use config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "copyd=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting copyd daemon");

    // Load configuration
    let config = Config::load().await?;
    
    // Create daemon instance
    let daemon = Arc::new(Daemon::new(config).await?);

    // Handle systemd notifications
    if let Ok(_) = std::env::var("NOTIFY_SOCKET") {
        // Send ready notification to systemd
        systemd::daemon::notify(false, [(systemd::daemon::STATE_READY, "1")].iter())?;
        info!("Notified systemd that daemon is ready");

        // Start watchdog if enabled
        if let Some(watchdog_usec) = systemd::daemon::watchdog_enabled(false) {
            let daemon_clone = daemon.clone();
            tokio::spawn(async move {
                let interval = std::time::Duration::from_micros(watchdog_usec / 2);
                let mut ticker = tokio::time::interval(interval);
                loop {
                    ticker.tick().await;
                    if daemon_clone.is_healthy().await {
                        let _ = systemd::daemon::notify(false, [(systemd::daemon::STATE_WATCHDOG, "1")].iter());
                    }
                }
            });
            info!("Started systemd watchdog with interval {:?}", std::time::Duration::from_micros(watchdog_usec));
        }
    }

    // Run the daemon
    if let Err(e) = daemon.run().await {
        error!("Daemon error: {}", e);
        // Notify systemd of failure
        if let Ok(_) = std::env::var("NOTIFY_SOCKET") {
            let _ = systemd::daemon::notify(false, [
                (systemd::daemon::STATE_STATUS, &format!("Failed: {}", e)),
                (systemd::daemon::STATE_ERRNO, &format!("{}", e as &dyn std::error::Error as *const _ as usize))
            ].iter());
        }
        return Err(e);
    }

    Ok(())
} 