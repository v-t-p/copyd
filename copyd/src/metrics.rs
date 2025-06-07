use prometheus::{Counter, Gauge, Histogram, Registry, Encoder, TextEncoder};
use std::sync::Arc;
use anyhow::Result;

#[derive(Clone)]
pub struct Metrics {
    registry: Arc<Registry>,
    pub jobs_total: Counter,
    pub jobs_active: Gauge,
    pub jobs_completed: Counter,
    pub jobs_failed: Counter,
    pub bytes_copied_total: Counter,
    pub copy_duration: Histogram,
    pub throughput_mbps: Gauge,
}

impl Metrics {
    pub fn new() -> Result<Self> {
        let registry = Arc::new(Registry::new());
        
        let jobs_total = Counter::new("copyd_jobs_total", "Total number of copy jobs created")?;
        let jobs_active = Gauge::new("copyd_jobs_active", "Number of currently active jobs")?;
        let jobs_completed = Counter::new("copyd_jobs_completed_total", "Total number of completed jobs")?;
        let jobs_failed = Counter::new("copyd_jobs_failed_total", "Total number of failed jobs")?;
        let bytes_copied_total = Counter::new("copyd_bytes_copied_total", "Total bytes copied")?;
        let copy_duration = Histogram::with_opts(
            prometheus::HistogramOpts::new("copyd_copy_duration_seconds", "Time taken to copy files")
                .buckets(vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0, 600.0])
        )?;
        let throughput_mbps = Gauge::new("copyd_throughput_mbps", "Current throughput in MB/s")?;

        registry.register(Box::new(jobs_total.clone()))?;
        registry.register(Box::new(jobs_active.clone()))?;
        registry.register(Box::new(jobs_completed.clone()))?;
        registry.register(Box::new(jobs_failed.clone()))?;
        registry.register(Box::new(bytes_copied_total.clone()))?;
        registry.register(Box::new(copy_duration.clone()))?;
        registry.register(Box::new(throughput_mbps.clone()))?;

        Ok(Self {
            registry,
            jobs_total,
            jobs_active,
            jobs_completed,
            jobs_failed,
            bytes_copied_total,
            copy_duration,
            throughput_mbps,
        })
    }

    pub fn export(&self) -> Result<String> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer)?;
        Ok(String::from_utf8(buffer)?)
    }

    pub fn record_job_created(&self) {
        self.jobs_total.inc();
        self.jobs_active.inc();
    }

    pub fn record_file_copied(&self, bytes_copied: u64, duration_secs: f64) {
        self.bytes_copied_total.inc_by(bytes_copied as f64);
        if duration_secs > 0.0 {
            let throughput = bytes_copied as f64 / duration_secs;
            self.throughput_mbps.set(throughput);
        }
    }

    pub fn record_job_completed(&self, bytes_copied: u64, duration_secs: f64) {
        self.jobs_completed.inc();
        self.jobs_active.dec();
        self.bytes_copied_total.inc_by(bytes_copied as f64);
        self.copy_duration.observe(duration_secs);
    }

    pub fn record_job_failed(&self) {
        self.jobs_failed.inc();
        self.jobs_active.dec();
    }

    pub fn update_throughput(&self, mbps: f64) {
        self.throughput_mbps.set(mbps);
    }
} 