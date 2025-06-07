use std::path::Path;
use anyhow::Result;

pub fn format_bytes(bytes: u64) -> String {
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

pub fn format_duration(seconds: i64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m {}s", seconds / 3600, (seconds % 3600) / 60, seconds % 60)
    }
}

pub fn validate_path(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("Path does not exist: {:?}", path);
    }
    Ok(())
}

pub fn calculate_eta(bytes_done: u64, total_bytes: u64, throughput_mbps: f64) -> i64 {
    if throughput_mbps <= 0.0 || bytes_done >= total_bytes {
        return 0;
    }
    
    let remaining_bytes = total_bytes - bytes_done;
    let remaining_mb = remaining_bytes as f64 / (1024.0 * 1024.0);
    (remaining_mb / throughput_mbps) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30), "30s");
        assert_eq!(format_duration(90), "1m 30s");
        assert_eq!(format_duration(3661), "1h 1m 1s");
    }

    #[test]
    fn test_calculate_eta() {
        assert_eq!(calculate_eta(0, 1024*1024*100, 10.0), 10); // 100MB at 10MB/s = 10s
        assert_eq!(calculate_eta(1024*1024*50, 1024*1024*100, 10.0), 5); // 50MB remaining at 10MB/s = 5s
        assert_eq!(calculate_eta(1024*1024*100, 1024*1024*100, 10.0), 0); // Already done
    }
} 