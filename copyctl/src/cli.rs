use crate::client::CopyClient;
use copyd_protocol::*;
use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use console::style;
use tokio::time::{interval, Duration};

pub async fn handle_copy(
    client: CopyClient,
    args: crate::CopyMoveArgs,
    format: &str,
) -> Result<()> {
    let request = CreateJobRequest {
        sources: args.sources.iter().map(|p| p.to_string_lossy().to_string()).collect(),
        destination: args.destination.to_string_lossy().to_string(),
        recursive: args.recursive,
        preserve_metadata: args.preserve,
        preserve_links: args.preserve_links,
        preserve_sparse: args.preserve_sparse,
        verify: args.verify as i32,
        exists_action: args.exists as i32,
        priority: args.priority,
        max_rate_bps: match args.max_rate {
            Some(r) => r.checked_mul(1024 * 1024)
                            .ok_or_else(|| anyhow::anyhow!("--max-rate is too large"))?,
            None => 0,
        },
        engine: args.engine as i32,
        dry_run: args.dry_run,
        regex_rename_match: args.regex_rename_match.unwrap_or_default(),
        regex_rename_replace: args.regex_rename_replace.unwrap_or_default(),
        block_size: args.block_size.unwrap_or(0),
        compress: args.compress,
        encrypt: args.encrypt,
    };

    let job_id = client.create_job(request).await?;

    if format == "json" {
        println!("{}", serde_json::json!({
            "job_id": job_id,
            "status": "created"
        }));
    } else {
        println!("{} Created copy job: {}", 
            style("✓").green(), 
            style(&job_id).cyan()
        );
    }

    if args.monitor {
        monitor_job(&client, &job_id, format).await?;
    }

    Ok(())
}

pub async fn handle_move(
    client: CopyClient,
    args: crate::CopyMoveArgs,
    format: &str,
) -> Result<()> {
    println!("{} Move operation will copy then delete source files", style("⚠").yellow());
    
    handle_copy(client, args, format).await
}

pub async fn handle_list(
    client: CopyClient,
    completed: bool,
    format: &str,
) -> Result<()> {
    let jobs = client.list_jobs(completed).await?;

    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&jobs)?);
    } else {
        if jobs.is_empty() {
            println!("{} No jobs found", style("ℹ").blue());
            return Ok(());
        }

        println!("{:<36} {:<8} {:<20} {:<20} {:<10}", "Job ID", "Status", "Source", "Destination", "Progress");
        println!("{}", "-".repeat(100));

        for job in jobs {
            let job_id = job.job_id.map(|j| j.uuid).unwrap_or_default();
            let status = styled_job_status(job.progress.as_ref().map(|p| p.status).unwrap_or(0));

            let source = job.sources.first().map(|s| {
                if s.len() > 18 {
                    format!("{}...", &s[..15])
                } else {
                    s.clone()
                }
            }).unwrap_or_default();

            let destination = if job.destination.len() > 18 {
                format!("{}...", &job.destination[..15])
            } else {
                job.destination
            };

            let progress = if let Some(p) = job.progress {
                if p.total_bytes > 0 {
                    format!("{:.1}%", (p.bytes_copied as f64 / p.total_bytes as f64) * 100.0)
                } else {
                    "N/A".to_string()
                }
            } else {
                "N/A".to_string()
            };

            let short_id = job_id.get(..8).unwrap_or(&job_id);
            println!("{:<36} {:<8} {:<20} {:<20} {:<10}",
                style(short_id).dim(),
                status,
                source,
                destination,
                progress
            );
        }
    }

    Ok(())
}

pub async fn handle_status(
    client: CopyClient,
    job_id: String,
    monitor: bool,
    format: &str,
) -> Result<()> {
    if monitor {
        monitor_job(&client, &job_id, format).await?;
    } else {
        let status = client.get_job_status(&job_id).await?;

        if format == "json" {
            println!("{}", serde_json::to_string_pretty(&status)?);
        } else {
            print_job_status(&status);
        }
    }

    Ok(())
}

pub async fn handle_cancel(
    client: CopyClient,
    job_id: String,
    format: &str,
) -> Result<()> {
    client.cancel_job(&job_id).await?;

    if format == "json" {
        println!("{}", serde_json::json!({
            "job_id": job_id,
            "action": "cancelled"
        }));
    } else {
        println!("{} Cancelled job: {}", 
            style("✓").green(), 
            style(&job_id).cyan()
        );
    }

    Ok(())
}

pub async fn handle_pause(
    client: CopyClient,
    job_id: String,
    format: &str,
) -> Result<()> {
    client.pause_job(&job_id).await?;

    if format == "json" {
        println!("{}", serde_json::json!({
            "job_id": job_id,
            "action": "paused"
        }));
    } else {
        println!("{} Paused job: {}", 
            style("⏸").yellow(), 
            style(&job_id).cyan()
        );
    }

    Ok(())
}

pub async fn handle_resume(
    client: CopyClient,
    job_id: String,
    format: &str,
) -> Result<()> {
    client.resume_job(&job_id).await?;

    if format == "json" {
        println!("{}", serde_json::json!({
            "job_id": job_id,
            "action": "resumed"
        }));
    } else {
        println!("{} Resumed job: {}", 
            style("▶").green(), 
            style(&job_id).cyan()
        );
    }

    Ok(())
}

pub async fn handle_stats(
    client: CopyClient,
    days: i32,
    format: &str,
) -> Result<()> {
    let stats = client.get_stats(days).await?;

    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("{} Statistics for the last {} days:", style("📊").blue(), days);
        println!("  Total bytes copied: {}", format_bytes(stats.total_bytes_copied));
        println!("  Total files copied: {}", stats.total_files_copied);
        println!("  Total jobs: {}", stats.total_jobs);
        
        if !stats.daily_stats.is_empty() {
            println!("\n{} Daily breakdown:", style("📅").blue());
            for daily in stats.daily_stats {
                println!("  {}: {} bytes, {} files, {} jobs",
                    daily.date,
                    format_bytes(daily.bytes_copied),
                    daily.files_copied,
                    daily.jobs_completed
                );
            }
        }

        if !stats.slow_paths.is_empty() {
            println!("\n{} Slowest paths:", style("🐌").yellow());
            for slow in stats.slow_paths {
                println!("  {}: {:.2} MB/s (copied {} times)",
                    slow.path,
                    slow.avg_throughput_mbps,
                    slow.copy_count
                );
            }
        }
    }

    Ok(())
}

pub async fn handle_health(
    client: CopyClient,
    format: &str,
) -> Result<()> {
    let health = client.health_check().await?;

    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&health)?);
    } else {
        let status_icon = if health.healthy {
            style("✓").green()
        } else {
            style("✗").red()
        };

        println!("{} Daemon status: {}", 
            status_icon,
            if health.healthy { style("HEALTHY").green() } else { style("UNHEALTHY").red() }
        );
        println!("  Version: {}", health.version);
        println!("  Uptime: {}", format_duration(health.uptime_seconds));
        println!("  Active jobs: {}", health.active_jobs);
        println!("  Queued jobs: {}", health.queued_jobs);
        println!("  Memory usage: {}", format_bytes(health.memory_usage_bytes));
        println!("  CPU usage: {:.1}%", health.cpu_usage_percent);
    }

    Ok(())
}

async fn monitor_job(client: &CopyClient, job_id: &str, format: &str) -> Result<()> {
    if format == "json" {
        // For JSON format, just poll and output status updates
        let mut interval = interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            
            match client.get_job_status(job_id).await {
                Ok(status) => {
                    println!("{}", serde_json::to_string_pretty(&status)?);
                    
                    if let Some(progress) = &status.progress {
                        if let Ok(status) = JobStatus::try_from(progress.status) {
                            match status {
                                JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled => break,
                                _ => {}
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("{}", serde_json::json!({
                        "error": format!("Error getting job status: {}", e)
                    }));
                    break;
                }
            }
        }
    } else {
        // For text format, show a nice progress bar
        let pb = ProgressBar::new(100);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {percent}% ({bytes}/{total_bytes}) {msg}")
                .expect("valid indicatif progress bar template")
                .progress_chars("#>-")
        );
        pb.set_length(100);

        let mut interval = interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            
            match client.get_job_status(job_id).await {
                Ok(status) => {
                    if let Some(progress) = &status.progress {
                        let percent = if progress.total_bytes > 0 {
                            (progress.bytes_copied as f64 / progress.total_bytes as f64 * 100.0) as u64
                        } else {
                            0
                        };

                        pb.set_position(percent);
                        
                        let msg = if progress.throughput_mbps > 0.0 {
                            format!("{:.1} MB/s, ETA: {}s", 
                                progress.throughput_mbps, 
                                progress.eta_seconds)
                        } else {
                            "Calculating...".to_string()
                        };
                        pb.set_message(msg);

                        if let Ok(status) = JobStatus::try_from(progress.status) {
                            match status {
                                JobStatus::Completed => {
                                    pb.finish_with_message("Completed!");
                                    break;
                                }
                                JobStatus::Failed => {
                                    pb.finish_with_message("Failed!");
                                    break;
                                }
                                JobStatus::Cancelled => {
                                    pb.finish_with_message("Cancelled!");
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Err(e) => {
                    pb.finish_with_message(&format!("Error: {}", e));
                    break;
                }
            }
        }
    }

    Ok(())
}

fn print_job_status(status: &JobStatusResponse) {
    let job_id = status.job_id.as_ref()
        .map(|j| j.uuid.clone())
        .unwrap_or_default();

    println!("{} Job Status: {}", style("📋").blue(), style(&job_id).cyan());

    if let Some(progress) = &status.progress {
        let status_text = styled_job_status(progress.status);

        println!("  Status: {}", status_text);
        
        if progress.total_bytes > 0 {
            let percent = (progress.bytes_copied as f64 / progress.total_bytes as f64) * 100.0;
            println!("  Progress: {:.1}% ({} / {})",
                percent,
                format_bytes(progress.bytes_copied),
                format_bytes(progress.total_bytes)
            );
        }

        if progress.throughput_mbps > 0.0 {
            println!("  Throughput: {:.1} MB/s", progress.throughput_mbps);
        }

        if progress.eta_seconds > 0 {
            println!("  ETA: {}", format_duration(progress.eta_seconds));
        }

        if progress.total_files > 0 {
            println!("  Files: {} / {}", progress.files_copied, progress.total_files);
        }
    }

    if !status.log_entries.is_empty() {
        println!("\n{} Recent log entries:", style("📝").blue());
        for entry in status.log_entries.iter().rev().take(5) {
            println!("  {}", entry);
        }
    }

    if !status.error.is_empty() {
        println!("\n{} Error: {}", style("❌").red(), status.error);
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

fn format_duration(seconds: i64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m {}s", seconds / 3600, (seconds % 3600) / 60, seconds % 60)
    }
}

/// Convert a numeric `JobStatus` code into a coloured, human-readable string.
fn styled_job_status(code: i32) -> console::StyledObject<&'static str> {
    match JobStatus::try_from(code) {
        Ok(JobStatus::Pending) => style("PENDING").yellow(),
        Ok(JobStatus::Running) => style("RUNNING").green(),
        Ok(JobStatus::Paused) => style("PAUSED").blue(),
        Ok(JobStatus::Completed) => style("COMPLETED").green(),
        Ok(JobStatus::Failed) => style("FAILED").red(),
        Ok(JobStatus::Cancelled) => style("CANCELLED").red(),
        _ => style("UNKNOWN").dim(),
    }
} 