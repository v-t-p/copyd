use anyhow::{Result, Context};
use std::path::Path;
use std::os::unix::io::{AsRawFd, RawFd};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, AsyncSeekExt};
use tracing::{info, debug};

#[derive(Debug, Clone)]
pub struct SparseRegion {
    pub offset: u64,
    pub length: u64,
    pub is_hole: bool,
}

pub struct SparseFileHandler;

impl SparseFileHandler {
    /// Copy a sparse file while preserving holes
    pub async fn copy_sparse_file(
        source: &Path,
        destination: &Path,
        block_size: Option<u64>,
    ) -> Result<u64> {
        info!("Copying sparse file: {:?} -> {:?}", source, destination);
        
        let mut source_file = File::open(source).await
            .with_context(|| format!("Failed to open source sparse file: {:?}", source))?;
        
        let mut dest_file = File::create(destination).await
            .with_context(|| format!("Failed to create destination sparse file: {:?}", destination))?;

        // Get file size
        let source_metadata = source_file.metadata().await?;
        let file_size = source_metadata.len();
        
        if file_size == 0 {
            return Ok(0);
        }

        // Detect sparse regions
        let regions = Self::detect_sparse_regions(source, file_size).await?;
        debug!("Detected {} sparse regions", regions.len());

        let block_size = block_size.unwrap_or(64 * 1024) as usize; // 64KB default
        let mut buffer = vec![0u8; block_size];
        let mut total_copied = 0u64;

        for region in regions {
            if region.is_hole {
                // Create hole by seeking past it
                dest_file.seek(std::io::SeekFrom::Start(region.offset + region.length)).await?;
                debug!("Created hole: offset={}, length={}", region.offset, region.length);
            } else {
                // Copy data region
                source_file.seek(std::io::SeekFrom::Start(region.offset)).await?;
                dest_file.seek(std::io::SeekFrom::Start(region.offset)).await?;
                
                let mut remaining = region.length;
                while remaining > 0 {
                    let to_read = std::cmp::min(remaining, block_size as u64) as usize;
                    
                    let bytes_read = source_file.read(&mut buffer[..to_read]).await?;
                    if bytes_read == 0 {
                        break;
                    }
                    
                    dest_file.write_all(&buffer[..bytes_read]).await?;
                    remaining -= bytes_read as u64;
                    total_copied += bytes_read as u64;
                }
                
                debug!("Copied data region: offset={}, length={}", region.offset, region.length);
            }
        }

        // Ensure the file has the correct size
        dest_file.set_len(file_size).await?;
        dest_file.flush().await?;

        info!("Sparse file copy completed: {} bytes data in {} total", total_copied, file_size);
        Ok(total_copied)
    }

    /// Detect sparse regions in a file using SEEK_HOLE/SEEK_DATA
    async fn detect_sparse_regions(source: &Path, file_size: u64) -> Result<Vec<SparseRegion>> {
        let file = std::fs::File::open(source)?;
        let fd = file.as_raw_fd();
        
        let mut regions = Vec::new();
        let mut current_offset = 0u64;

        while current_offset < file_size {
            // Find next data region
            let data_start = match Self::seek_data(fd, current_offset) {
                Ok(offset) if offset < file_size => offset,
                _ => break, // No more data
            };

            // If there's a gap before data, it's a hole
            if data_start > current_offset {
                regions.push(SparseRegion {
                    offset: current_offset,
                    length: data_start - current_offset,
                    is_hole: true,
                });
            }

            // Find end of data region (start of next hole)
            let data_end = match Self::seek_hole(fd, data_start) {
                Ok(offset) => std::cmp::min(offset, file_size),
                Err(_) => file_size, // Data extends to end of file
            };

            // Add data region
            if data_end > data_start {
                regions.push(SparseRegion {
                    offset: data_start,
                    length: data_end - data_start,
                    is_hole: false,
                });
            }

            current_offset = data_end;
        }

        // Handle any remaining hole at the end
        if current_offset < file_size {
            regions.push(SparseRegion {
                offset: current_offset,
                length: file_size - current_offset,
                is_hole: true,
            });
        }

        Ok(regions)
    }

    /// Use lseek with SEEK_DATA to find next data region
    fn seek_data(fd: RawFd, offset: u64) -> Result<u64> {
        const SEEK_DATA: i32 = 3; // Linux SEEK_DATA constant
        
        let result = unsafe {
            libc::lseek(fd, offset as libc::off_t, SEEK_DATA)
        };
        
        if result < 0 {
            let errno = unsafe { *libc::__errno_location() };
            if errno == libc::ENXIO {
                // No more data - return file end
                return Err(anyhow::anyhow!("No more data regions"));
            }
            return Err(anyhow::anyhow!("lseek SEEK_DATA failed: errno {}", errno));
        }
        
        Ok(result as u64)
    }

    /// Use lseek with SEEK_HOLE to find next hole
    fn seek_hole(fd: RawFd, offset: u64) -> Result<u64> {
        const SEEK_HOLE: i32 = 4; // Linux SEEK_HOLE constant
        
        let result = unsafe {
            libc::lseek(fd, offset as libc::off_t, SEEK_HOLE)
        };
        
        if result < 0 {
            let errno = unsafe { *libc::__errno_location() };
            return Err(anyhow::anyhow!("lseek SEEK_HOLE failed: errno {}", errno));
        }
        
        Ok(result as u64)
    }

    /// Check if a file is sparse by comparing allocated blocks vs file size
    pub async fn is_sparse_file(path: &Path) -> Result<bool> {
        let metadata = tokio::fs::metadata(path).await?;
        let file_size = metadata.len();
        
        if file_size == 0 {
            return Ok(false);
        }

        // Get allocated blocks (Unix-specific)
        use std::os::unix::fs::MetadataExt;
        let blocks = metadata.blocks();
        let allocated_bytes = blocks * 512; // stat.st_blocks is in 512-byte units
        
        // If allocated size is significantly less than file size, it's sparse
        let sparseness_threshold = 0.95; // 95% efficiency threshold
        let is_sparse = (allocated_bytes as f64) < (file_size as f64 * sparseness_threshold);
        
        if is_sparse {
            let sparseness = 100.0 * (1.0 - allocated_bytes as f64 / file_size as f64);
            debug!("Detected sparse file: {:?} ({:.1}% sparse, {} allocated / {} total)", 
                   path, sparseness, allocated_bytes, file_size);
        }
        
        Ok(is_sparse)
    }

    /// Get sparse file statistics
    pub async fn get_sparse_stats(path: &Path) -> Result<SparseStats> {
        let metadata = tokio::fs::metadata(path).await?;
        let file_size = metadata.len();
        
        if file_size == 0 {
            return Ok(SparseStats {
                file_size: 0,
                allocated_bytes: 0,
                sparse_ratio: 0.0,
                hole_count: 0,
                data_regions: 0,
            });
        }

        use std::os::unix::fs::MetadataExt;
        let allocated_bytes = metadata.blocks() * 512;
        let sparse_ratio = if file_size > 0 {
            1.0 - (allocated_bytes as f64 / file_size as f64)
        } else {
            0.0
        };

        // Count holes and data regions
        let regions = Self::detect_sparse_regions(path, file_size).await?;
        let hole_count = regions.iter().filter(|r| r.is_hole).count();
        let data_regions = regions.iter().filter(|r| !r.is_hole).count();

        Ok(SparseStats {
            file_size,
            allocated_bytes,
            sparse_ratio,
            hole_count,
            data_regions,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SparseStats {
    pub file_size: u64,
    pub allocated_bytes: u64,
    pub sparse_ratio: f64, // 0.0 = not sparse, 1.0 = completely sparse
    pub hole_count: usize,
    pub data_regions: usize,
}

impl std::fmt::Display for SparseStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SparseStats {{ size: {}, allocated: {}, sparse: {:.1}%, holes: {}, data_regions: {} }}", 
               self.file_size, 
               self.allocated_bytes, 
               self.sparse_ratio * 100.0,
               self.hole_count,
               self.data_regions)
    }
} 