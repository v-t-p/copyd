use anyhow::{Result, Context};
use std::path::Path;
use std::os::unix::io::{AsRawFd, RawFd};
use std::time::Instant;
use tokio::fs::File;
use tracing::{info, debug, warn, error};
use io_uring::{IoUring, opcode, types};
use std::io::{IoSlice, IoSliceMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct IoUringCopyEngine {
    ring: IoUring,
    max_concurrent_ops: usize,
    buffer_size: usize,
}

#[derive(Debug)]
pub struct IoUringCopyStats {
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub read_ops: u64,
    pub write_ops: u64,
    pub avg_read_latency_us: f64,
    pub avg_write_latency_us: f64,
    pub queue_depth: u32,
}

impl IoUringCopyEngine {
    pub fn new(queue_depth: u32, buffer_size: Option<usize>) -> Result<Self> {
        // Check if io_uring is available
        if !Self::is_io_uring_available() {
            return Err(anyhow::anyhow!("io_uring not available on this system"));
        }

        let ring = IoUring::new(queue_depth)
            .with_context(|| "Failed to create io_uring instance")?;

        info!("Created io_uring with queue depth: {}", queue_depth);

        Ok(Self {
            ring,
            max_concurrent_ops: queue_depth as usize,
            buffer_size: buffer_size.unwrap_or(1024 * 1024), // 1MB default
        })
    }

    pub fn is_io_uring_available() -> bool {
        // Try to create a minimal io_uring to test availability
        IoUring::new(1).is_ok()
    }

    pub async fn copy_file_async(
        &mut self,
        source: &Path,
        destination: &Path,
        max_rate_bps: Option<u64>,
    ) -> Result<IoUringCopyStats> {
        info!("Starting io_uring copy: {:?} -> {:?}", source, destination);
        
        let start_time = Instant::now();
        let source_file = std::fs::File::open(source)
            .with_context(|| format!("Failed to open source file: {:?}", source))?;
        let dest_file = std::fs::File::create(destination)
            .with_context(|| format!("Failed to create destination file: {:?}", destination))?;

        let source_fd = source_file.as_raw_fd();
        let dest_fd = dest_file.as_raw_fd();

        // Get file size for progress tracking
        let file_size = source_file.metadata()?.len();
        info!("File size: {} bytes", file_size);

        let mut stats = IoUringCopyStats {
            bytes_read: 0,
            bytes_written: 0,
            read_ops: 0,
            write_ops: 0,
            avg_read_latency_us: 0.0,
            avg_write_latency_us: 0.0,
            queue_depth: self.ring.params().sq_entries(),
        };

        // Use multiple buffers for better parallelism
        let num_buffers = std::cmp::min(self.max_concurrent_ops, 8);
        let mut buffers: Vec<Vec<u8>> = (0..num_buffers)
            .map(|_| vec![0u8; self.buffer_size])
            .collect();

        let mut offset = 0u64;
        let mut pending_ops = 0;
        let mut buffer_index = 0;
        
        let total_read_latency = Arc::new(AtomicU64::new(0));
        let total_write_latency = Arc::new(AtomicU64::new(0));

        while offset < file_size || pending_ops > 0 {
            // Submit read operations
            while offset < file_size && pending_ops < self.max_concurrent_ops && buffer_index < buffers.len() {
                let read_size = std::cmp::min(self.buffer_size as u64, file_size - offset);
                
                let read_entry = opcode::Read::new(
                    types::Fd(source_fd),
                    buffers[buffer_index].as_mut_ptr(),
                    read_size as u32,
                )
                .offset(offset)
                .build()
                .user_data(Self::encode_user_data(true, buffer_index as u64, offset));

                unsafe {
                    self.ring.submission()
                        .push(&read_entry)
                        .with_context(|| "Failed to push read operation")?;
                }

                offset += read_size;
                buffer_index = (buffer_index + 1) % buffers.len();
                pending_ops += 1;
            }

            // Submit the operations
            let submitted = self.ring.submit()?;
            debug!("Submitted {} operations", submitted);

            // Wait for at least one completion
            self.ring.submit_and_wait(1)?;

            // Process completions
            for cqe in &mut self.ring.completion() {
                let user_data = cqe.user_data();
                let (is_read, buf_idx, file_offset) = Self::decode_user_data(user_data);
                let result = cqe.result();

                if result < 0 {
                    return Err(anyhow::anyhow!("io_uring operation failed: {}", result));
                }

                let bytes_transferred = result as u64;
                
                if is_read {
                    stats.bytes_read += bytes_transferred;
                    stats.read_ops += 1;

                    // Now submit corresponding write operation
                    if bytes_transferred > 0 {
                        let write_entry = opcode::Write::new(
                            types::Fd(dest_fd),
                            buffers[buf_idx as usize].as_ptr(),
                            bytes_transferred as u32,
                        )
                        .offset(file_offset)
                        .build()
                        .user_data(Self::encode_user_data(false, buf_idx, file_offset));

                        unsafe {
                            self.ring.submission()
                                .push(&write_entry)
                                .with_context(|| "Failed to push write operation")?;
                        }
                    }
                } else {
                    stats.bytes_written += bytes_transferred;
                    stats.write_ops += 1;
                    pending_ops -= 1;
                }
            }

            // Rate limiting
            if let Some(max_rate) = max_rate_bps {
                let elapsed = start_time.elapsed();
                let expected_time = stats.bytes_written as f64 / max_rate as f64;
                let actual_time = elapsed.as_secs_f64();
                
                if actual_time < expected_time {
                    let sleep_time = expected_time - actual_time;
                    tokio::time::sleep(std::time::Duration::from_secs_f64(sleep_time)).await;
                }
            }
        }

        // Ensure all data is written to disk
        let fsync_entry = opcode::Fsync::new(types::Fd(dest_fd))
            .build()
            .user_data(u64::MAX); // Special marker for fsync

        unsafe {
            self.ring.submission()
                .push(&fsync_entry)
                .with_context(|| "Failed to push fsync operation")?;
        }
        
        self.ring.submit_and_wait(1)?;

        // Process fsync completion
        for cqe in &mut self.ring.completion() {
            if cqe.result() < 0 {
                return Err(anyhow::anyhow!("fsync failed: {}", cqe.result()));
            }
        }

        let total_time = start_time.elapsed();
        let throughput = stats.bytes_read as f64 / total_time.as_secs_f64() / 1024.0 / 1024.0;

        info!("io_uring copy completed: {} bytes in {:.2}s ({:.2} MB/s)",
              stats.bytes_read, total_time.as_secs_f64(), throughput);

        // Calculate average latencies
        if stats.read_ops > 0 {
            stats.avg_read_latency_us = total_read_latency.load(Ordering::Relaxed) as f64 / stats.read_ops as f64;
        }
        if stats.write_ops > 0 {
            stats.avg_write_latency_us = total_write_latency.load(Ordering::Relaxed) as f64 / stats.write_ops as f64;
        }

        Ok(stats)
    }

    // Enhanced copy with features like vectored I/O
    pub async fn copy_file_vectored(
        &mut self,
        source: &Path,
        destination: &Path,
        vector_size: usize,
        max_rate_bps: Option<u64>,
    ) -> Result<IoUringCopyStats> {
        info!("Starting vectored io_uring copy: {:?} -> {:?}", source, destination);

        let source_file = std::fs::File::open(source)?;
        let dest_file = std::fs::File::create(destination)?;
        let source_fd = source_file.as_raw_fd();
        let dest_fd = dest_file.as_raw_fd();

        let file_size = source_file.metadata()?.len();
        let chunk_size = self.buffer_size / vector_size;

        let mut stats = IoUringCopyStats {
            bytes_read: 0,
            bytes_written: 0,
            read_ops: 0,
            write_ops: 0,
            avg_read_latency_us: 0.0,
            avg_write_latency_us: 0.0,
            queue_depth: self.ring.params().sq_entries(),
        };

        // Create vectored buffers
        let mut buffer_vecs: Vec<Vec<Vec<u8>>> = (0..2)
            .map(|_| (0..vector_size).map(|_| vec![0u8; chunk_size]).collect())
            .collect();

        let mut offset = 0u64;
        let mut current_buffer_set = 0;

        while offset < file_size {
            let remaining = file_size - offset;
            let total_read_size = std::cmp::min(remaining, (chunk_size * vector_size) as u64);
            
            // Prepare iovec for vectored read
            let mut iovecs: Vec<IoSliceMut> = buffer_vecs[current_buffer_set]
                .iter_mut()
                .take(vector_size)
                .map(|buf| IoSliceMut::new(buf))
                .collect();

            // Submit vectored read
            let readv_entry = opcode::Readv::new(
                types::Fd(source_fd),
                iovecs.as_ptr() as *const libc::iovec,
                iovecs.len() as u32,
            )
            .offset(offset)
            .build()
            .user_data(1); // Mark as read operation

            unsafe {
                self.ring.submission().push(&readv_entry)?;
            }
            self.ring.submit_and_wait(1)?;

            // Process read completion
            for cqe in &mut self.ring.completion() {
                let bytes_read = cqe.result() as u64;
                if cqe.result() < 0 {
                    return Err(anyhow::anyhow!("Vectored read failed: {}", cqe.result()));
                }

                stats.bytes_read += bytes_read;
                stats.read_ops += 1;

                // Submit vectored write
                let write_iovecs: Vec<IoSlice> = buffer_vecs[current_buffer_set]
                    .iter()
                    .take(vector_size)
                    .map(|buf| IoSlice::new(buf))
                    .collect();

                let writev_entry = opcode::Writev::new(
                    types::Fd(dest_fd),
                    write_iovecs.as_ptr() as *const libc::iovec,
                    write_iovecs.len() as u32,
                )
                .offset(offset)
                .build()
                .user_data(2); // Mark as write operation

                unsafe {
                    self.ring.submission().push(&writev_entry)?;
                }
                self.ring.submit_and_wait(1)?;

                // Process write completion
                for write_cqe in &mut self.ring.completion() {
                    if write_cqe.result() < 0 {
                        return Err(anyhow::anyhow!("Vectored write failed: {}", write_cqe.result()));
                    }
                    stats.bytes_written += write_cqe.result() as u64;
                    stats.write_ops += 1;
                }
            }

            offset += total_read_size;
            current_buffer_set = 1 - current_buffer_set; // Alternate buffer sets
            
            // Rate limiting
            if let Some(max_rate) = max_rate_bps {
                let elapsed_time = std::time::Instant::now().elapsed();
                let expected_time = stats.bytes_written as f64 / max_rate as f64;
                if elapsed_time.as_secs_f64() < expected_time {
                    let sleep_duration = expected_time - elapsed_time.as_secs_f64();
                    tokio::time::sleep(std::time::Duration::from_secs_f64(sleep_duration)).await;
                }
            }
        }

        info!("Vectored io_uring copy completed: {} bytes", stats.bytes_read);
        Ok(stats)
    }

    // Helper functions for encoding/decoding user data
    fn encode_user_data(is_read: bool, buffer_index: u64, offset: u64) -> u64 {
        let operation_bit = if is_read { 1u64 << 63 } else { 0 };
        let buffer_bits = (buffer_index & 0xFF) << 48;
        let offset_bits = offset & 0xFFFFFFFFFFFF;
        operation_bit | buffer_bits | offset_bits
    }

    fn decode_user_data(user_data: u64) -> (bool, u64, u64) {
        let is_read = (user_data & (1u64 << 63)) != 0;
        let buffer_index = (user_data >> 48) & 0xFF;
        let offset = user_data & 0xFFFFFFFFFFFF;
        (is_read, buffer_index, offset)
    }

    pub fn get_ring_stats(&self) -> (u32, u32, u32) {
        let params = self.ring.params();
        (params.sq_entries(), params.cq_entries(), params.features())
    }
}

impl std::fmt::Display for IoUringCopyStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IoUringStats {{ read: {} bytes ({} ops), write: {} bytes ({} ops), queue_depth: {}, avg_latency: {:.2}μs read, {:.2}μs write }}",
               self.bytes_read, self.read_ops,
               self.bytes_written, self.write_ops,
               self.queue_depth,
               self.avg_read_latency_us, self.avg_write_latency_us)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[tokio::test]
    async fn test_io_uring_availability() {
        // This test will only pass on systems with io_uring support
        if IoUringCopyEngine::is_io_uring_available() {
            println!("io_uring is available");
            let engine = IoUringCopyEngine::new(32, Some(64 * 1024));
            assert!(engine.is_ok());
        } else {
            println!("io_uring is not available on this system");
        }
    }

    #[tokio::test]
    async fn test_io_uring_copy() {
        if !IoUringCopyEngine::is_io_uring_available() {
            return; // Skip test if io_uring not available
        }

        let mut engine = IoUringCopyEngine::new(32, Some(64 * 1024)).unwrap();
        
        // Create test file
        let mut source_file = NamedTempFile::new().unwrap();
        let test_data = b"Hello, io_uring world!".repeat(1000);
        source_file.write_all(&test_data).unwrap();
        
        let dest_file = NamedTempFile::new().unwrap();
        
        let stats = engine.copy_file_async(
            source_file.path(),
            dest_file.path(),
            None
        ).await.unwrap();
        
        assert_eq!(stats.bytes_read, test_data.len() as u64);
        assert_eq!(stats.bytes_written, test_data.len() as u64);
        assert!(stats.read_ops > 0);
        assert!(stats.write_ops > 0);
    }
} 