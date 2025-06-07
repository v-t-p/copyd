use crate::metrics::PerformanceMetrics;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex};
use tracing::debug;

#[cfg(target_os = "linux")]
use procfs::process::Process;

/// Performance metrics collector
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    /// CPU usage samples
    pub cpu_samples: Vec<f64>,
    /// Memory usage samples (in bytes)
    pub memory_samples: Vec<u64>,
    /// I/O operation times
    pub io_operations: HashMap<String, Vec<Duration>>,
    /// Copy engine performance
    pub engine_performance: HashMap<String, EngineMetrics>,
    /// System resource usage
    pub system_metrics: SystemMetrics,
}

#[derive(Debug, Clone)]
pub struct EngineMetrics {
    pub operations: u64,
    pub bytes_copied: u64,
    pub total_time: Duration,
    pub average_throughput: f64, // MB/s
    pub error_count: u64,
}

#[derive(Debug, Clone)]
pub struct SystemMetrics {
    pub peak_memory_usage: u64,
    pub current_memory_usage: u64,
    pub file_descriptors_used: u32,
    pub total_jobs_processed: u64,
    pub uptime: Duration,
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            cpu_samples: Vec::new(),
            memory_samples: Vec::new(),
            io_operations: HashMap::new(),
            engine_performance: HashMap::new(),
            system_metrics: SystemMetrics {
                peak_memory_usage: 0,
                current_memory_usage: 0,
                file_descriptors_used: 0,
                total_jobs_processed: 0,
                uptime: Duration::new(0, 0),
            },
        }
    }
}

/// Performance profiler for tracking and optimizing operations
pub struct PerformanceProfiler {
    metrics: Arc<Mutex<PerformanceMetrics>>,
    start_time: Instant,
    sampling_interval: Duration,
}

impl PerformanceProfiler {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(Mutex::new(PerformanceMetrics::default())),
            start_time: Instant::now(),
            sampling_interval: Duration::from_secs(5),
        }
    }

    /// Start timing an operation
    pub fn start_timer(&self, operation: &str) -> OperationTimer {
        OperationTimer::new(operation.to_string(), self.metrics.clone())
    }

    /// Record engine performance
    pub fn record_engine_performance(
        &self,
        engine_name: &str,
        bytes_copied: u64,
        duration: Duration,
        success: bool,
    ) {
        if let Ok(mut metrics) = self.metrics.lock() {
            let entry = metrics.engine_performance
                .entry(engine_name.to_string())
                .or_insert_with(|| EngineMetrics {
                    operations: 0,
                    bytes_copied: 0,
                    total_time: Duration::new(0, 0),
                    average_throughput: 0.0,
                    error_count: 0,
                });

            entry.operations += 1;
            entry.bytes_copied += bytes_copied;
            entry.total_time += duration;

            if !success {
                entry.error_count += 1;
            }

            // Calculate throughput in MB/s
            if entry.total_time.as_secs_f64() > 0.0 {
                entry.average_throughput = (entry.bytes_copied as f64) 
                    / entry.total_time.as_secs_f64() 
                    / (1024.0 * 1024.0);
            }

            debug!("Engine '{}' performance: {:.2} MB/s, {} operations, {:.1}% error rate",
                   engine_name,
                   entry.average_throughput,
                   entry.operations,
                   (entry.error_count as f64 / entry.operations as f64) * 100.0);
        }
    }

    /// Sample system performance
    pub fn sample_system_performance(&self) {
        if let Ok(mut metrics) = self.metrics.lock() {
            // Sample memory usage (simplified - in production use proper system calls)
            let current_memory = self.get_current_memory_usage();
            metrics.memory_samples.push(current_memory);
            metrics.system_metrics.current_memory_usage = current_memory;
            
            if current_memory > metrics.system_metrics.peak_memory_usage {
                metrics.system_metrics.peak_memory_usage = current_memory;
            }

            // Sample CPU usage (simplified)
            let cpu_usage = self.get_current_cpu_usage();
            metrics.cpu_samples.push(cpu_usage);

            // Update uptime
            metrics.system_metrics.uptime = self.start_time.elapsed();

            // Keep only recent samples (last 100)
            if metrics.memory_samples.len() > 100 {
                let to_drain = metrics.memory_samples.len() - 100;
                metrics.memory_samples.drain(..to_drain);
            }
            if metrics.cpu_samples.len() > 100 {
                let to_drain = metrics.cpu_samples.len() - 100;
                metrics.cpu_samples.drain(..to_drain);
            }

            debug!("System metrics: {}MB memory, {:.1}% CPU",
                   current_memory / (1024 * 1024),
                   cpu_usage);
        }
    }

    /// Get performance report
    pub fn get_performance_report(&self) -> PerformanceReport {
        let metrics = self.metrics.lock().unwrap();
        
        let average_memory = if metrics.memory_samples.is_empty() {
            0
        } else {
            metrics.memory_samples.iter().sum::<u64>() / metrics.memory_samples.len() as u64
        };

        let average_cpu = if metrics.cpu_samples.is_empty() {
            0.0
        } else {
            metrics.cpu_samples.iter().sum::<f64>() / metrics.cpu_samples.len() as f64
        };

        let mut engine_reports = Vec::new();
        for (name, engine_metrics) in &metrics.engine_performance {
            engine_reports.push(EngineReport {
                name: name.clone(),
                throughput_mbps: engine_metrics.average_throughput,
                operations: engine_metrics.operations,
                error_rate: if engine_metrics.operations > 0 {
                    (engine_metrics.error_count as f64 / engine_metrics.operations as f64) * 100.0
                } else {
                    0.0
                },
                total_bytes: engine_metrics.bytes_copied,
            });
        }

        PerformanceReport {
            uptime: metrics.system_metrics.uptime,
            peak_memory_mb: metrics.system_metrics.peak_memory_usage / (1024 * 1024),
            average_memory_mb: average_memory / (1024 * 1024),
            average_cpu_percent: average_cpu,
            total_jobs: metrics.system_metrics.total_jobs_processed,
            engine_reports,
        }
    }

    /// Detect performance issues and suggest optimizations
    pub fn analyze_performance(&self) -> Vec<PerformanceRecommendation> {
        let mut recommendations = Vec::new();
        let metrics = self.metrics.lock().unwrap();

        // Check memory usage
        if metrics.system_metrics.peak_memory_usage > 500 * 1024 * 1024 { // 500MB
            recommendations.push(PerformanceRecommendation {
                category: "Memory".to_string(),
                issue: "High memory usage detected".to_string(),
                suggestion: "Consider reducing buffer sizes or concurrent operations".to_string(),
                severity: RecommendationSeverity::High,
            });
        }

        // Check engine performance
        for (name, engine_metrics) in &metrics.engine_performance {
            if engine_metrics.error_rate() > 10.0 {
                recommendations.push(PerformanceRecommendation {
                    category: "Reliability".to_string(),
                    issue: format!("High error rate for engine '{}'", name),
                    suggestion: "Check engine configuration or consider using different engine".to_string(),
                    severity: RecommendationSeverity::Medium,
                });
            }

            if engine_metrics.average_throughput < 50.0 { // Less than 50 MB/s
                recommendations.push(PerformanceRecommendation {
                    category: "Performance".to_string(),
                    issue: format!("Low throughput for engine '{}'", name),
                    suggestion: "Consider tuning buffer sizes or using different copy engine".to_string(),
                    severity: RecommendationSeverity::Low,
                });
            }
        }

        recommendations
    }

    /// Start background performance monitoring
    pub fn start_monitoring(&self) -> tokio::task::JoinHandle<()> {
        let profiler = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(profiler.sampling_interval);
            loop {
                interval.tick().await;
                profiler.sample_system_performance();
            }
        })
    }

    // Helper methods (simplified implementations)
    fn get_current_memory_usage(&self) -> u64 {
        // In production, use proper system calls like getrusage or /proc/self/status
        // This is a simplified placeholder
        50 * 1024 * 1024 // 50MB placeholder
    }

    fn get_current_cpu_usage(&self) -> f64 {
        // In production, calculate CPU usage from /proc/stat or similar
        // This is a simplified placeholder
        15.0 // 15% placeholder
    }
}

impl Clone for PerformanceProfiler {
    fn clone(&self) -> Self {
        Self {
            metrics: self.metrics.clone(),
            start_time: self.start_time,
            sampling_interval: self.sampling_interval,
        }
    }
}

impl EngineMetrics {
    fn error_rate(&self) -> f64 {
        if self.operations > 0 {
            (self.error_count as f64 / self.operations as f64) * 100.0
        } else {
            0.0
        }
    }
}

/// Timer for measuring operation duration
pub struct OperationTimer {
    operation: String,
    start_time: Instant,
    metrics: Arc<Mutex<PerformanceMetrics>>,
}

impl OperationTimer {
    fn new(operation: String, metrics: Arc<Mutex<PerformanceMetrics>>) -> Self {
        Self {
            operation,
            start_time: Instant::now(),
            metrics,
        }
    }

    /// Complete the timer and record the duration
    pub fn finish(self) -> Duration {
        let duration = self.start_time.elapsed();
        
        if let Ok(mut metrics) = self.metrics.lock() {
            metrics.io_operations
                .entry(self.operation.clone())
                .or_insert_with(Vec::new)
                .push(duration);
        }

        debug!("Operation '{}' completed in {:?}", self.operation, duration);
        duration
    }
}

/// Performance analysis report
#[derive(Debug)]
pub struct PerformanceReport {
    pub uptime: Duration,
    pub peak_memory_mb: u64,
    pub average_memory_mb: u64,
    pub average_cpu_percent: f64,
    pub total_jobs: u64,
    pub engine_reports: Vec<EngineReport>,
}

#[derive(Debug)]
pub struct EngineReport {
    pub name: String,
    pub throughput_mbps: f64,
    pub operations: u64,
    pub error_rate: f64,
    pub total_bytes: u64,
}

#[derive(Debug)]
pub struct PerformanceRecommendation {
    pub category: String,
    pub issue: String,
    pub suggestion: String,
    pub severity: RecommendationSeverity,
}

#[derive(Debug, PartialEq)]
pub enum RecommendationSeverity {
    High,
    Medium,
    Low,
}

impl PerformanceReport {
    /// Generate a human-readable performance summary
    pub fn summary(&self) -> String {
        format!(
            "Performance Summary:\n\
             Uptime: {:.1} hours\n\
             Peak Memory: {} MB\n\
             Average Memory: {} MB\n\
             Average CPU: {:.1}%\n\
             Total Jobs: {}\n\
             Engines: {}",
            self.uptime.as_secs_f64() / 3600.0,
            self.peak_memory_mb,
            self.average_memory_mb,
            self.average_cpu_percent,
            self.total_jobs,
            self.engine_reports.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_profiler() {
        let profiler = PerformanceProfiler::new();
        
        // Record some engine performance
        profiler.record_engine_performance(
            "test_engine",
            1024 * 1024, // 1MB
            Duration::from_secs(1),
            true,
        );

        let report = profiler.get_performance_report();
        assert_eq!(report.engine_reports.len(), 1);
        assert_eq!(report.engine_reports[0].name, "test_engine");
    }

    #[test]
    fn test_operation_timer() {
        let profiler = PerformanceProfiler::new();
        let timer = profiler.start_timer("test_operation");
        
        std::thread::sleep(Duration::from_millis(10));
        let duration = timer.finish();
        
        assert!(duration >= Duration::from_millis(10));
    }

    #[test]
    fn test_performance_analysis() {
        let profiler = PerformanceProfiler::new();
        
        // Record poor engine performance
        profiler.record_engine_performance(
            "slow_engine",
            1024, // 1KB
            Duration::from_secs(1), // Very slow
            false, // Error
        );

        let recommendations = profiler.analyze_performance();
        assert!(!recommendations.is_empty());
    }
} 