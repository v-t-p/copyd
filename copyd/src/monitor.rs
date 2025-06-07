use crate::error::{CopydError, CopydResult};
use prometheus::{
    Counter, Gauge, Histogram, IntCounter, IntGauge, Registry, 
    exponential_buckets, linear_buckets
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn, error};
use tokio::sync::RwLock;

/// Enhanced monitoring system with Prometheus metrics
pub struct EnhancedMonitor {
    registry: Registry,
    metrics: MonitoringMetrics,
    alerts: AlertManager,
    start_time: Instant,
}

/// Prometheus metrics collection
#[derive(Clone)]
pub struct MonitoringMetrics {
    // Job metrics
    pub jobs_total: IntCounter,
    pub jobs_active: IntGauge,
    pub jobs_completed: IntCounter,
    pub jobs_failed: IntCounter,
    pub job_duration: Histogram,
    
    // Transfer metrics
    pub bytes_transferred: Counter,
    pub transfer_rate: Gauge,
    pub files_processed: IntCounter,
    
    // Engine metrics
    pub engine_operations: IntCounter,
    pub engine_errors: IntCounter,
    pub engine_throughput: Gauge,
    
    // System metrics
    pub memory_usage: Gauge,
    pub cpu_usage: Gauge,
    pub disk_io_ops: IntCounter,
    pub file_descriptors: IntGauge,
    
    // Error metrics
    pub errors_total: IntCounter,
    pub errors_by_type: IntCounter,
    
    // Performance metrics
    pub checkpoint_operations: IntCounter,
    pub verification_operations: IntCounter,
    pub retry_operations: IntCounter,
}

impl EnhancedMonitor {
    pub fn new() -> CopydResult<Self> {
        let registry = Registry::new();
        let metrics = MonitoringMetrics::new(&registry)?;
        let alerts = AlertManager::new();

        Ok(Self {
            registry,
            metrics,
            alerts,
            start_time: Instant::now(),
        })
    }

    /// Get Prometheus metrics registry
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Record job start
    pub fn job_started(&self, job_id: &str) {
        self.metrics.jobs_total.inc();
        self.metrics.jobs_active.inc();
        info!("Job started: {}", job_id);
    }

    /// Record job completion
    pub fn job_completed(&self, job_id: &str, duration: Duration, bytes_transferred: u64) {
        self.metrics.jobs_active.dec();
        self.metrics.jobs_completed.inc();
        self.metrics.job_duration.observe(duration.as_secs_f64());
        self.metrics.bytes_transferred.inc_by(bytes_transferred as f64);
        
        // Calculate transfer rate
        let rate_mbps = if duration.as_secs_f64() > 0.0 {
            (bytes_transferred as f64) / duration.as_secs_f64() / (1024.0 * 1024.0)
        } else {
            0.0
        };
        self.metrics.transfer_rate.set(rate_mbps);

        info!("Job completed: {} ({:.2} MB/s)", job_id, rate_mbps);
    }

    /// Record job failure
    pub fn job_failed(&self, job_id: &str, error: &CopydError) {
        self.metrics.jobs_active.dec();
        self.metrics.jobs_failed.inc();
        self.record_error(error);
        
        // Check for critical failure patterns
        self.alerts.check_job_failure_rate(&self.metrics);
        
        warn!("Job failed: {} - {}", job_id, error);
    }

    /// Record engine performance
    pub fn engine_operation(&self, engine: &str, success: bool, throughput_mbps: f64) {
        self.metrics.engine_operations.inc();
        
        if !success {
            self.metrics.engine_errors.inc();
        }
        
        self.metrics.engine_throughput.set(throughput_mbps);
        
        // Check engine performance thresholds
        self.alerts.check_engine_performance(engine, success, throughput_mbps);
    }

    /// Record system metrics
    pub fn update_system_metrics(&self, memory_mb: f64, cpu_percent: f64, fd_count: i64) {
        self.metrics.memory_usage.set(memory_mb);
        self.metrics.cpu_usage.set(cpu_percent);
        self.metrics.file_descriptors.set(fd_count);
        
        // Check resource thresholds
        self.alerts.check_resource_usage(memory_mb, cpu_percent, fd_count);
    }

    /// Record error occurrence
    pub fn record_error(&self, error: &CopydError) {
        self.metrics.errors_total.inc();
        
        let error_type = match error {
            CopydError::FileNotFound { .. } => "file_not_found",
            CopydError::PermissionDenied { .. } => "permission_denied",
            CopydError::InsufficientSpace { .. } => "insufficient_space",
            CopydError::VerificationFailed { .. } => "verification_failed",
            CopydError::CopyEngineFailed { .. } => "engine_failed",
            CopydError::CheckpointCorrupted { .. } => "checkpoint_corrupted",
            _ => "other",
        };
        
        // Record by error type (would need proper label support)
        self.metrics.errors_by_type.inc();
        
        // Trigger alerts for critical errors
        if matches!(error.severity(), crate::error::ErrorSeverity::Critical) {
            self.alerts.trigger_critical_error_alert(error);
        }
    }

    /// Get health status
    pub fn health_status(&self) -> HealthStatus {
        let uptime = self.start_time.elapsed();
        let active_jobs = self.metrics.jobs_active.get();
        let total_errors = self.metrics.errors_total.get();
        let memory_usage = self.metrics.memory_usage.get();
        let cpu_usage = self.metrics.cpu_usage.get();

        // Determine overall health
        let status = if memory_usage > 1000.0 || cpu_usage > 90.0 || total_errors > 100 {
            HealthLevel::Warning
        } else if memory_usage > 2000.0 || cpu_usage > 95.0 || total_errors > 500 {
            HealthLevel::Critical
        } else {
            HealthLevel::Healthy
        };

        HealthStatus {
            level: status,
            uptime,
            active_jobs: active_jobs as u64,
            total_errors: total_errors as u64,
            memory_usage_mb: memory_usage,
            cpu_usage_percent: cpu_usage,
            alerts: self.alerts.get_active_alerts(),
        }
    }

    /// Export metrics in Prometheus format
    pub fn export_metrics(&self) -> String {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder.encode_to_string(&metric_families).unwrap_or_default()
    }
}

impl MonitoringMetrics {
    fn new(registry: &Registry) -> CopydResult<Self> {
        // Job metrics
        let jobs_total = IntCounter::new("copyd_jobs_total", "Total number of jobs processed")?;
        let jobs_active = IntGauge::new("copyd_jobs_active", "Number of currently active jobs")?;
        let jobs_completed = IntCounter::new("copyd_jobs_completed", "Number of completed jobs")?;
        let jobs_failed = IntCounter::new("copyd_jobs_failed", "Number of failed jobs")?;
        let job_duration = Histogram::with_opts(
            prometheus::HistogramOpts::new("copyd_job_duration_seconds", "Job duration in seconds")
                .buckets(exponential_buckets(0.1, 2.0, 10)?),
        )?;

        // Transfer metrics
        let bytes_transferred = Counter::new("copyd_bytes_transferred_total", "Total bytes transferred")?;
        let transfer_rate = Gauge::new("copyd_transfer_rate_mbps", "Current transfer rate in MB/s")?;
        let files_processed = IntCounter::new("copyd_files_processed_total", "Total files processed")?;

        // Engine metrics
        let engine_operations = IntCounter::new("copyd_engine_operations_total", "Total engine operations")?;
        let engine_errors = IntCounter::new("copyd_engine_errors_total", "Total engine errors")?;
        let engine_throughput = Gauge::new("copyd_engine_throughput_mbps", "Engine throughput in MB/s")?;

        // System metrics
        let memory_usage = Gauge::new("copyd_memory_usage_mb", "Memory usage in MB")?;
        let cpu_usage = Gauge::new("copyd_cpu_usage_percent", "CPU usage percentage")?;
        let disk_io_ops = IntCounter::new("copyd_disk_io_operations_total", "Total disk I/O operations")?;
        let file_descriptors = IntGauge::new("copyd_file_descriptors", "Number of open file descriptors")?;

        // Error metrics
        let errors_total = IntCounter::new("copyd_errors_total", "Total number of errors")?;
        let errors_by_type = IntCounter::new("copyd_errors_by_type_total", "Errors by type")?;

        // Performance metrics
        let checkpoint_operations = IntCounter::new("copyd_checkpoint_operations_total", "Checkpoint operations")?;
        let verification_operations = IntCounter::new("copyd_verification_operations_total", "Verification operations")?;
        let retry_operations = IntCounter::new("copyd_retry_operations_total", "Retry operations")?;

        // Register all metrics
        registry.register(Box::new(jobs_total.clone()))?;
        registry.register(Box::new(jobs_active.clone()))?;
        registry.register(Box::new(jobs_completed.clone()))?;
        registry.register(Box::new(jobs_failed.clone()))?;
        registry.register(Box::new(job_duration.clone()))?;
        registry.register(Box::new(bytes_transferred.clone()))?;
        registry.register(Box::new(transfer_rate.clone()))?;
        registry.register(Box::new(files_processed.clone()))?;
        registry.register(Box::new(engine_operations.clone()))?;
        registry.register(Box::new(engine_errors.clone()))?;
        registry.register(Box::new(engine_throughput.clone()))?;
        registry.register(Box::new(memory_usage.clone()))?;
        registry.register(Box::new(cpu_usage.clone()))?;
        registry.register(Box::new(disk_io_ops.clone()))?;
        registry.register(Box::new(file_descriptors.clone()))?;
        registry.register(Box::new(errors_total.clone()))?;
        registry.register(Box::new(errors_by_type.clone()))?;
        registry.register(Box::new(checkpoint_operations.clone()))?;
        registry.register(Box::new(verification_operations.clone()))?;
        registry.register(Box::new(retry_operations.clone()))?;

        Ok(Self {
            jobs_total,
            jobs_active,
            jobs_completed,
            jobs_failed,
            job_duration,
            bytes_transferred,
            transfer_rate,
            files_processed,
            engine_operations,
            engine_errors,
            engine_throughput,
            memory_usage,
            cpu_usage,
            disk_io_ops,
            file_descriptors,
            errors_total,
            errors_by_type,
            checkpoint_operations,
            verification_operations,
            retry_operations,
        })
    }
}

/// Alert management system
pub struct AlertManager {
    active_alerts: Arc<RwLock<Vec<Alert>>>,
}

impl AlertManager {
    fn new() -> Self {
        Self {
            active_alerts: Arc::new(RwLock::new(Vec::new())),
        }
    }

    async fn add_alert(&self, alert: Alert) {
        let mut alerts = self.active_alerts.write().await;
        alerts.push(alert);
        
        // Keep only recent alerts (last 100)
        if alerts.len() > 100 {
            alerts.drain(..alerts.len() - 100);
        }
    }

    fn check_job_failure_rate(&self, metrics: &MonitoringMetrics) {
        let total_jobs = metrics.jobs_total.get();
        let failed_jobs = metrics.jobs_failed.get();
        
        if total_jobs > 10 {
            let failure_rate = (failed_jobs as f64 / total_jobs as f64) * 100.0;
            if failure_rate > 20.0 {
                let alert = Alert {
                    id: uuid::Uuid::new_v4().to_string(),
                    severity: AlertSeverity::High,
                    message: format!("High job failure rate: {:.1}%", failure_rate),
                    timestamp: chrono::Utc::now(),
                    category: "Jobs".to_string(),
                };
                tokio::spawn(async move {
                    // Would send alert in real implementation
                });
            }
        }
    }

    fn check_engine_performance(&self, engine: &str, success: bool, throughput: f64) {
        if !success {
            warn!("Engine {} operation failed", engine);
        }
        
        if throughput < 10.0 && throughput > 0.0 {
            warn!("Low engine {} throughput: {:.2} MB/s", engine, throughput);
        }
    }

    fn check_resource_usage(&self, memory_mb: f64, cpu_percent: f64, fd_count: i64) {
        if memory_mb > 1000.0 {
            warn!("High memory usage: {:.1} MB", memory_mb);
        }
        
        if cpu_percent > 90.0 {
            warn!("High CPU usage: {:.1}%", cpu_percent);
        }
        
        if fd_count > 1000 {
            warn!("High file descriptor usage: {}", fd_count);
        }
    }

    fn trigger_critical_error_alert(&self, error: &CopydError) {
        error!("Critical error: {}", error);
        // In production, would send notifications via email, Slack, etc.
    }

    async fn get_active_alerts(&self) -> Vec<Alert> {
        self.active_alerts.read().await.clone()
    }
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub id: String,
    pub severity: AlertSeverity,
    pub message: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlertSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug)]
pub struct HealthStatus {
    pub level: HealthLevel,
    pub uptime: Duration,
    pub active_jobs: u64,
    pub total_errors: u64,
    pub memory_usage_mb: f64,
    pub cpu_usage_percent: f64,
    pub alerts: Vec<Alert>,
}

#[derive(Debug, PartialEq)]
pub enum HealthLevel {
    Healthy,
    Warning,
    Critical,
}

impl HealthStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(self.level, HealthLevel::Healthy)
    }

    pub fn summary(&self) -> String {
        format!(
            "Health: {:?} | Uptime: {:.1}h | Active Jobs: {} | Errors: {} | Memory: {:.1}MB | CPU: {:.1}%",
            self.level,
            self.uptime.as_secs_f64() / 3600.0,
            self.active_jobs,
            self.total_errors,
            self.memory_usage_mb,
            self.cpu_usage_percent
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhanced_monitor_creation() {
        let monitor = EnhancedMonitor::new().unwrap();
        assert!(monitor.registry().gather().len() > 0);
    }

    #[test]
    fn test_job_metrics() {
        let monitor = EnhancedMonitor::new().unwrap();
        
        monitor.job_started("test_job");
        assert_eq!(monitor.metrics.jobs_total.get(), 1);
        assert_eq!(monitor.metrics.jobs_active.get(), 1);
        
        monitor.job_completed("test_job", Duration::from_secs(10), 1024);
        assert_eq!(monitor.metrics.jobs_completed.get(), 1);
        assert_eq!(monitor.metrics.jobs_active.get(), 0);
    }

    #[test]
    fn test_health_status() {
        let monitor = EnhancedMonitor::new().unwrap();
        let health = monitor.health_status();
        assert_eq!(health.level, HealthLevel::Healthy);
        assert!(health.is_healthy());
    }
} 