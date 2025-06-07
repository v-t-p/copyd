use anyhow::{Result, Context};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd};
use tracing::{info, debug, warn};
#[cfg(unix)]
use nix::fcntl::copy_file_range;
#[cfg(unix)]
use nix::sys::sendfile::sendfile;
#[cfg(unix)]
use nix::unistd;
use std::time::SystemTime;
use crate::verify::{FileVerifier};
use copyd_protocol::VerifyMode;
use crate::sparse::SparseFileHandler;
use copyd_protocol::{CopyEngine, ExistsAction};

#[derive(Debug, Clone)]
pub struct CopyOptions {
    pub preserve_metadata: bool,
    pub preserve_links: bool,
    pub preserve_sparse: bool,
    pub verify: VerifyMode,
    pub exists_action: ExistsAction,
    pub max_rate_bps: Option<u64>,
    pub block_size: Option<u64>,
    pub dry_run: bool,
    pub compress: bool,
    pub encrypt: bool,
}

pub struct FileCopyEngine {
    engine_type: CopyEngine,
}

impl FileCopyEngine {
    pub fn new(engine_type: CopyEngine) -> Self {
        Self { engine_type }
    }

    pub async fn copy_file(
        &self,
        source: &Path,
        destination: &Path,
        options: &CopyOptions,
    ) -> Result<u64> {
        info!("Copying {:?} to {:?} with engine {:?}", source, destination, self.engine_type);

        if options.dry_run {
            return self.perform_dry_run(source, destination, options).await;
        }

        // Check if this is a sparse file and we should preserve sparse regions
        let is_sparse = if options.preserve_sparse {
            SparseFileHandler::is_sparse_file(source).await.unwrap_or(false)
        } else {
            false
        };

        // Perform the actual copy
        let bytes_copied = if is_sparse && options.preserve_sparse {
            info!("Detected sparse file, using sparse-aware copy");
            SparseFileHandler::copy_sparse_file(source, destination, options.block_size).await?
        } else {
            match self.engine_type {
                CopyEngine::Auto => self.auto_copy(source, destination, options).await?,
                CopyEngine::IoUring => self.auto_copy(source, destination, options).await?,
                CopyEngine::CopyFileRange => self.copy_file_range_copy(source, destination, options).await?,
                CopyEngine::Sendfile => self.sendfile_copy(source, destination, options).await?,
                CopyEngine::Reflink => self.reflink_copy(source, destination, options).await?,
                CopyEngine::ReadWrite => self.read_write_copy(source, destination, options).await?,
            }
        };

        // Copy metadata if requested (but only after the file content is copied)
        if options.preserve_metadata {
            self.copy_metadata(source, destination).await?;
        }

        // Verify the copy if requested
        if matches!(options.verify, VerifyMode::Size | VerifyMode::Md5 | VerifyMode::Sha256) {
            info!("Verifying copied file with {:?}", options.verify);
            let verification_start = std::time::Instant::now();
            
            let verify_mode_local = crate::verify::VerifyMode::from(options.verify);

            match FileVerifier::verify_copy(source, destination, verify_mode_local).await {
                Ok(true) => {
                    let verification_time = verification_start.elapsed();
                    info!("Verification completed successfully in {:.2}s", verification_time.as_secs_f64());
                }
                Ok(false) => {
                    return Err(anyhow::anyhow!("File verification failed for {:?}", destination));
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("Verification error for {:?}", destination));
                }
            }
        }

        Ok(bytes_copied)
    }

    async fn auto_copy(&self, source: &Path, destination: &Path, options: &CopyOptions) -> Result<u64> {
        // Auto mode: intelligently choose the best copy method
        debug!("Auto-selecting best copy engine for {:?} -> {:?}", source, destination);
        
        // Check if source and destination are on the same filesystem
        let source_metadata = tokio::fs::metadata(source).await?;
        let dest_parent = destination.parent().unwrap_or(destination);
        
        // Try to get destination filesystem info
        let same_filesystem = if let Ok(dest_metadata) = tokio::fs::metadata(dest_parent).await {
            source_metadata.dev() == dest_metadata.dev()
        } else {
            false
        };
        
        // Decision tree for best copy method:
        if same_filesystem {
            // Same filesystem - try reflink first (instant COW copy)
            info!("Same filesystem detected, trying reflink (COW) first");
            match self.reflink_copy(source, destination, options).await {
                Ok(bytes) => return Ok(bytes),
                Err(e) => {
                    debug!("Reflink failed: {}, trying copy_file_range", e);
                    // Reflink failed, try copy_file_range
                    match self.copy_file_range_copy(source, destination, options).await {
                        Ok(bytes) => return Ok(bytes),
                        Err(e) => {
                            debug!("copy_file_range failed: {}, falling back to read/write", e);
                        }
                    }
                }
            }
        } else {
            // Cross-filesystem - use copy_file_range or sendfile
            info!("Cross-filesystem copy detected, using copy_file_range");
            match self.copy_file_range_copy(source, destination, options).await {
                Ok(bytes) => return Ok(bytes),
                Err(e) => {
                    debug!("copy_file_range failed: {}, trying sendfile", e);
                    match self.sendfile_copy(source, destination, options).await {
                        Ok(bytes) => return Ok(bytes),
                        Err(e) => {
                            debug!("sendfile failed: {}, falling back to read/write", e);
                        }
                    }
                }
            }
        }
        
        // Final fallback to simple read/write
        info!("Using read/write fallback");
        self.read_write_copy(source, destination, options).await
    }

    #[cfg(unix)]
    async fn copy_file_range_copy(&self, source: &Path, destination: &Path, options: &CopyOptions) -> Result<u64> {
        info!("Using copy_file_range for high-performance copying");
        
        let source_file = std::fs::File::open(source)
            .with_context(|| format!("Failed to open source file: {:?}", source))?;
        
        let dest_file = std::fs::File::create(destination)
            .with_context(|| format!("Failed to create destination file: {:?}", destination))?;
        
        // Get source file size
        let source_metadata = source_file.metadata()?;
        let file_size = source_metadata.len();
        
        let mut total_copied = 0u64;
        let chunk_size = options.block_size.unwrap_or(4 * 1024 * 1024) as usize; // Default 4MB chunks
        
        while total_copied < file_size {
            let remaining = file_size - total_copied;
            let copy_size = std::cmp::min(remaining, chunk_size as u64) as usize;
            
            let mut source_offset = total_copied as i64;
            let mut dest_offset = total_copied as i64;

            // Use copy_file_range system call
            match copy_file_range(
                &source_file,
                Some(&mut source_offset),
                &dest_file, 
                Some(&mut dest_offset),
                copy_size
            ) {
                Ok(bytes_copied) => {
                    if bytes_copied == 0 {
                        break; // EOF reached
                    }
                    total_copied += bytes_copied as u64;
                    
                    // Apply rate limiting if specified
                    if let Some(max_rate) = options.max_rate_bps {
                        let elapsed = std::time::Duration::from_nanos(
                            (bytes_copied as f64 / max_rate as f64 * 1_000_000_000.0) as u64
                        );
                        if elapsed > std::time::Duration::from_millis(1) {
                            tokio::time::sleep(elapsed).await;
                        }
                    }
                }
                Err(e) => {
                    warn!("copy_file_range failed: {}, falling back to read/write", e);
                    drop(source_file);
                    drop(dest_file);
                    return self.read_write_copy(source, destination, options).await;
                }
            }
        }

        info!("copy_file_range completed: {} bytes", total_copied);
        Ok(total_copied)
    }

    #[cfg(not(unix))]
    async fn copy_file_range_copy(&self, source: &Path, destination: &Path, options: &CopyOptions) -> Result<u64> {
        warn!("copy_file_range is not supported on this platform, falling back to read/write");
        self.read_write_copy(source, destination, options).await
    }

    #[cfg(unix)]
    async fn sendfile_copy(&self, source: &Path, destination: &Path, options: &CopyOptions) -> Result<u64> {
        info!("Using sendfile for zero-copy transfer");
        
        let source_file = std::fs::File::open(source)
            .with_context(|| format!("Failed to open source file: {:?}", source))?;
        
        let dest_file = std::fs::File::create(destination)
            .with_context(|| format!("Failed to create destination file: {:?}", destination))?;
        
        // Get source file size
        let source_metadata = source_file.metadata()?;
        let file_size = source_metadata.len();
        
        let mut total_copied = 0u64;
        let mut offset = 0i64;
        let chunk_size = options.block_size.unwrap_or(1024 * 1024) as usize; // Default 1MB chunks
        
        while total_copied < file_size {
            let remaining = file_size - total_copied;
            let copy_size = std::cmp::min(remaining, chunk_size as u64) as usize;
            
            // Use sendfile system call
            match sendfile(&dest_file, &source_file, Some(&mut offset), copy_size) {
                Ok(bytes_copied) => {
                    if bytes_copied == 0 {
                        break; // EOF reached
                    }
                    total_copied += bytes_copied as u64;
                    
                    // Apply rate limiting if specified
                    if let Some(max_rate) = options.max_rate_bps {
                        let elapsed = std::time::Duration::from_nanos(
                            (bytes_copied as f64 / max_rate as f64 * 1_000_000_000.0) as u64
                        );
                        if elapsed > std::time::Duration::from_millis(1) {
                            tokio::time::sleep(elapsed).await;
                        }
                    }
                }
                Err(e) => {
                    warn!("sendfile failed: {}, falling back to read/write", e);
                    drop(source_file);
                    drop(dest_file);
                    return self.read_write_copy(source, destination, options).await;
                }
            }
        }

        info!("sendfile completed: {} bytes", total_copied);
        Ok(total_copied)
    }

    #[cfg(not(unix))]
    async fn sendfile_copy(&self, source: &Path, destination: &Path, options: &CopyOptions) -> Result<u64> {
        warn!("sendfile is not supported on this platform, falling back to read/write");
        self.read_write_copy(source, destination, options).await
    }

    #[cfg(unix)]
    async fn reflink_copy(&self, source: &Path, destination: &Path, options: &CopyOptions) -> Result<u64> {
        info!("Attempting reflink (COW) copy");
        
        let source_file = std::fs::File::open(source)
            .with_context(|| format!("Failed to open source file: {:?}", source))?;
        
        let dest_file = std::fs::File::create(destination)
            .with_context(|| format!("Failed to create destination file: {:?}", destination))?;

        let source_fd = source_file.as_raw_fd();
        let dest_fd = dest_file.as_raw_fd();
        
        // Try to use FICLONE ioctl for reflink (COW) copy
        // This is supported on Btrfs, XFS, and OCFS2
        const FICLONE: libc::c_ulong = 0x40049409;
        
        let result = unsafe {
            libc::ioctl(dest_fd, FICLONE, source_fd)
        };
        
        if result == 0 {
            // Reflink succeeded - instant copy!
            let source_metadata = source_file.metadata()?;
            let file_size = source_metadata.len();
            
            info!("Reflink completed successfully: {} bytes (instant COW copy)", file_size);
            Ok(file_size)
        } else {
            let errno = unsafe { *libc::__errno_location() };
            match errno {
                libc::EOPNOTSUPP => {
                    info!("Reflink not supported on this filesystem, falling back to copy_file_range");
                    drop(source_file);
                    drop(dest_file);
                    self.copy_file_range_copy(source, destination, options).await
                }
                libc::EXDEV => {
                    info!("Cross-device reflink not supported, falling back to copy_file_range");
                    drop(source_file);
                    drop(dest_file);
                    self.copy_file_range_copy(source, destination, options).await
                }
                libc::EINVAL => {
                    warn!("Invalid reflink operation, falling back to copy_file_range");
                    drop(source_file);
                    drop(dest_file);
                    self.copy_file_range_copy(source, destination, options).await
                }
                _ => {
                    warn!("Reflink failed with errno {}, falling back to copy_file_range", errno);
                    drop(source_file);
                    drop(dest_file);
                    self.copy_file_range_copy(source, destination, options).await
                }
            }
        }
    }

    #[cfg(not(unix))]
    async fn reflink_copy(&self, source: &Path, destination: &Path, options: &CopyOptions) -> Result<u64> {
        warn!("reflink is not supported on this platform, falling back to read/write");
        self.read_write_copy(source, destination, options).await
    }

    async fn read_write_copy(&self, source: &Path, destination: &Path, options: &CopyOptions) -> Result<u64> {
        info!("Using read/write copy with optimized buffering");
        
        let block_size = options.block_size.unwrap_or(1024 * 1024) as usize; // Default 1MB for better performance
        
        let mut source_file = tokio::fs::File::open(source).await
            .with_context(|| format!("Failed to open source file: {:?}", source))?;
        
        let mut dest_file = tokio::fs::File::create(destination).await
            .with_context(|| format!("Failed to create destination file: {:?}", destination))?;

        // Use multiple buffers for better I/O parallelism
        let mut buffer1 = vec![0u8; block_size];
        let mut buffer2 = vec![0u8; block_size];
        let mut use_buffer1 = true;
        
        let mut total_bytes = 0u64;
        let start_time = std::time::Instant::now();
        let mut last_report = start_time;

        loop {
            let buffer = if use_buffer1 { &mut buffer1 } else { &mut buffer2 };
            
            let bytes_read = tokio::io::AsyncReadExt::read(&mut source_file, buffer).await?;
            if bytes_read == 0 {
                break;
            }

            tokio::io::AsyncWriteExt::write_all(&mut dest_file, &buffer[..bytes_read]).await?;
            total_bytes += bytes_read as u64;
            
            // Apply rate limiting if specified
            if let Some(max_rate) = options.max_rate_bps {
                let elapsed = start_time.elapsed();
                let expected_time = std::time::Duration::from_secs_f64(total_bytes as f64 / max_rate as f64);
                if elapsed < expected_time {
                    tokio::time::sleep(expected_time - elapsed).await;
                }
            }
            
            // Log progress periodically
            let now = std::time::Instant::now();
            if now.duration_since(last_report) > std::time::Duration::from_secs(5) {
                let throughput = total_bytes as f64 / start_time.elapsed().as_secs_f64() / 1024.0 / 1024.0;
                debug!("Copy progress: {} bytes, {:.2} MB/s", total_bytes, throughput);
                last_report = now;
            }
            
            use_buffer1 = !use_buffer1;
        }

        tokio::io::AsyncWriteExt::flush(&mut dest_file).await?;

        let elapsed = start_time.elapsed();
        let throughput = total_bytes as f64 / elapsed.as_secs_f64() / 1024.0 / 1024.0;
        info!("Read/write copy completed: {} bytes in {:.2}s ({:.2} MB/s)", 
              total_bytes, elapsed.as_secs_f64(), throughput);
        
        Ok(total_bytes)
    }

    #[cfg(unix)]
    async fn copy_metadata(&self, source: &Path, destination: &Path) -> Result<()> {
        let metadata = tokio::fs::metadata(source).await?;
        
        // Copy permissions
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(metadata.permissions().mode());
            tokio::fs::set_permissions(destination, permissions).await?;
        }

        // Copy ownership (requires appropriate privileges)
        {
            let uid = metadata.uid();
            let gid = metadata.gid();
            
            if let Err(e) = unistd::chown(destination, Some(unistd::Uid::from_raw(uid)), Some(unistd::Gid::from_raw(gid))) {
                // Don't fail if we can't change ownership (common when not root)
                debug!("Could not change ownership of {:?}: {}", destination, e);
            }
        }

        // Copy timestamps using utimensat system call
        {
            use nix::sys::stat::{utimensat, UtimensatFlags};
            use nix::sys::time::{TimeSpec};
            
            let atime = metadata.accessed().unwrap_or_else(|_| metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
            let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            
            let atime_spec = TimeSpec::from(atime.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default());
            let mtime_spec = TimeSpec::from(mtime.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default());
            
            if let Err(e) = utimensat(None, destination, &atime_spec, &mtime_spec, UtimensatFlags::empty()) {
                warn!("Could not set timestamps for {:?}: {}", destination, e);
            }
        }

        // Copy extended attributes (xattrs)
        {
            if let Err(e) = self.copy_xattrs(source, destination).await {
                debug!("Could not copy extended attributes: {}", e);
            }
        }
        
        Ok(())
    }

    #[cfg(not(unix))]
    async fn copy_metadata(&self, source: &Path, destination: &Path) -> Result<()> {
        warn!("Metadata preservation is not fully supported on this platform");
        let source_metadata = tokio::fs::metadata(source).await?;
        let dest_file = tokio::fs::File::open(destination).await?;
        dest_file.set_permissions(source_metadata.permissions()).await?;
        Ok(())
    }

    #[cfg(unix)]
    async fn copy_xattrs(&self, source: &Path, destination: &Path) -> Result<()> {
        use std::ffi::CString;
        
        // Get list of extended attributes
        let source_cstr = CString::new(source.to_string_lossy().as_bytes())?;
        let dest_cstr = CString::new(destination.to_string_lossy().as_bytes())?;
        
        // Buffer to hold attribute names
        let mut names_buf = vec![0u8; 1024];
        let names_len = unsafe {
            libc::listxattr(
                source_cstr.as_ptr(),
                names_buf.as_mut_ptr() as *mut libc::c_char,
                names_buf.len(),
            )
        };
        
        if names_len < 0 {
            let errno = unsafe { *libc::__errno_location() };
            if errno == libc::ENOTSUP || errno == libc::EOPNOTSUPP {
                debug!("Extended attributes not supported");
                return Ok(());
            }
            return Err(anyhow::anyhow!("Failed to list xattrs: errno {}", errno));
        }
        
        if names_len == 0 {
            return Ok(()); // No extended attributes
        }
        
        names_buf.truncate(names_len as usize);
        
        // Parse attribute names (null-separated)
        let mut pos = 0;
        while pos < names_buf.len() {
            let name_end = names_buf[pos..].iter().position(|&b| b == 0).unwrap_or(names_buf.len() - pos);
            if name_end == 0 {
                break;
            }
            
            let name = &names_buf[pos..pos + name_end];
            if let Ok(name_cstr) = CString::new(name) {
                // Get attribute value
                let mut value_buf = vec![0u8; 4096];
                let value_len = unsafe {
                    libc::getxattr(
                        source_cstr.as_ptr(),
                        name_cstr.as_ptr(),
                        value_buf.as_mut_ptr() as *mut libc::c_void,
                        value_buf.len(),
                    )
                };
                
                if value_len >= 0 {
                    value_buf.truncate(value_len as usize);
                    
                    // Set attribute on destination
                    let result = unsafe {
                        libc::setxattr(
                            dest_cstr.as_ptr(),
                            name_cstr.as_ptr(),
                            value_buf.as_ptr() as *const libc::c_void,
                            value_buf.len(),
                            0,
                        )
                    };
                    
                    if result < 0 {
                        debug!("Failed to set xattr {:?}", name_cstr);
                    }
                }
            }
            
            pos += name_end + 1;
        }
        
        Ok(())
    }

    #[cfg(not(unix))]
    async fn copy_xattrs(&self, source: &Path, destination: &Path) -> Result<()> {
        warn!("Extended attributes are not supported on this platform");
        Ok(())
    }

    async fn perform_dry_run(&self, source: &Path, destination: &Path, options: &CopyOptions) -> Result<u64> {
        info!("=== DRY RUN MODE ===");
        info!("Source: {:?}", source);
        info!("Destination: {:?}", destination);
        info!("Engine: {:?}", self.engine_type);

        // Check source file
        let source_metadata = tokio::fs::metadata(source).await
            .with_context(|| format!("Failed to read source: {:?}", source))?;
        
        let file_size = source_metadata.len();
        info!("Source size: {} bytes ({:.2} MB)", file_size, file_size as f64 / 1024.0 / 1024.0);

        // Check if destination directory exists
        if let Some(parent) = destination.parent() {
            if !parent.exists() {
                info!("Would create directory: {:?}", parent);
            } else {
                info!("Destination directory exists: {:?}", parent);
            }
        }

        // Check destination existence and handle according to policy
        if destination.exists() {
            let dest_metadata = tokio::fs::metadata(destination).await?;
            let dest_size = dest_metadata.len();
            
            match options.exists_action {
                ExistsAction::Overwrite => info!("Would OVERWRITE existing file ({} bytes)", dest_size),
                ExistsAction::Skip => {
                    info!("Would SKIP existing file ({} bytes)", dest_size);
                    return Ok(0); // Skip in dry run
                }
                ExistsAction::Serial => {
                    let serial_name = self.generate_serial_name(destination);
                    info!("Would create SERIAL copy: {:?}", serial_name);
                }
            }
        } else {
            info!("Destination does not exist, would create new file");
        }

        // Report copy operations that would be performed
        info!("Copy engine: {:?}", self.engine_type);
        
        if options.preserve_sparse {
            let is_sparse = SparseFileHandler::is_sparse_file(source).await.unwrap_or(false);
            if is_sparse {
                info!("Would preserve sparse file holes");
            } else {
                info!("Source is not sparse (or detection failed)");
            }
        }

        if options.preserve_metadata {
            info!("Would preserve metadata:");
            info!("  - File permissions");
            info!("  - Timestamps (atime, mtime)");
            info!("  - Ownership (uid, gid)");
            info!("  - Extended attributes");
        }

        if options.verify != VerifyMode::None {
            let verify_type = match options.verify {
                VerifyMode::Size => "size check",
                VerifyMode::Md5 => "MD5 checksum",
                VerifyMode::Sha256 => "SHA256 checksum",
                _ => "size check (default)",
            };
            info!("Would verify integrity with: {}", verify_type);
        }

        if let Some(rate_limit) = options.max_rate_bps {
            info!("Would apply rate limit: {} bytes/sec ({:.2} MB/s)", 
                  rate_limit, rate_limit as f64 / 1024.0 / 1024.0);
            
            let estimated_time = file_size as f64 / rate_limit as f64;
            info!("Estimated transfer time: {:.1} seconds", estimated_time);
        }

        if let Some(block_size) = options.block_size {
            info!("Would use block size: {} bytes", block_size);
        }

        info!("=== END DRY RUN ===");
        Ok(file_size) // Return size that would be copied
    }

    fn generate_serial_name(&self, original: &Path) -> PathBuf {
        let parent = original.parent().unwrap_or(Path::new(""));
        let stem = original.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let extension = original.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        // Find first available serial number
        for i in 1..=9999 {
            let new_name = if extension.is_empty() {
                format!("{}.{}", stem, i)
            } else {
                format!("{}.{}.{}", stem, i, extension)
            };
            
            let new_path = parent.join(new_name);
            if !new_path.exists() {
                return new_path;
            }
        }

        // Fallback if all serial numbers are taken
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let fallback_name = if extension.is_empty() {
            format!("{}.{}", stem, timestamp)
        } else {
            format!("{}.{}.{}", stem, timestamp, extension)
        };
        
        parent.join(fallback_name)
    }

    async fn handle_destination_exists(&self, destination: &Path, options: &CopyOptions) -> Result<PathBuf> {
        if !destination.exists() {
            return Ok(destination.to_path_buf());
        }

        match options.exists_action {
            ExistsAction::Overwrite => {
                // Overwrite
                info!("Overwriting existing file: {:?}", destination);
                Ok(destination.to_path_buf())
            }
            ExistsAction::Skip => {
                // Skip
                info!("Skipping existing file: {:?}", destination);
                Err(anyhow::anyhow!("File exists and skip policy is enabled"))
            }
            ExistsAction::Serial => {
                // Serial (create numbered copy)
                let serial_path = self.generate_serial_name(destination);
                info!("Creating serial copy: {:?}", serial_path);
                Ok(serial_path)
            }
        }
    }
}