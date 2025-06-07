use anyhow::{Result, Context};
use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use tokio::fs;
use tracing::{info, debug, warn};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub source_path: PathBuf,
    pub dest_path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_sparse: bool,
    pub hard_links: Option<(u64, u64)>, // (device, inode) for hard link detection
}

#[derive(Debug, Clone)]
pub struct DirectoryTraversal {
    pub files: Vec<FileEntry>,
    pub total_size: u64,
    pub total_files: u64,
    pub directories: Vec<PathBuf>,
    pub symlinks: Vec<FileEntry>,
    pub hard_link_map: HashMap<(u64, u64), PathBuf>, // Track hard links
}

pub struct DirectoryHandler;

impl DirectoryHandler {
    pub async fn analyze_sources(
        sources: &[PathBuf], 
        destination: &Path, 
        recursive: bool,
        preserve_links: bool,
    ) -> Result<DirectoryTraversal> {
        let mut traversal = DirectoryTraversal {
            files: Vec::new(),
            total_size: 0,
            total_files: 0,
            directories: Vec::new(),
            symlinks: Vec::new(),
            hard_link_map: HashMap::new(),
        };

        // Determine if destination is a directory
        let dest_is_dir = if let Ok(metadata) = fs::metadata(destination).await {
            metadata.is_dir()
        } else {
            // If destination doesn't exist, assume it's a directory if multiple sources
            sources.len() > 1
        };

        for source in sources {
            if let Ok(metadata) = fs::metadata(source).await {
                if metadata.is_dir() {
                    if recursive {
                        let dest_dir = if dest_is_dir {
                            destination.join(source.file_name().unwrap_or_default())
                        } else {
                            destination.to_path_buf()
                        };
                        
                        Self::traverse_directory(
                            source, 
                            &dest_dir, 
                            &mut traversal,
                            preserve_links
                        ).await?;
                    } else {
                        warn!("Skipping directory {:?} (recursive not enabled)", source);
                    }
                } else {
                    // Single file
                    let dest_path = if dest_is_dir {
                        destination.join(source.file_name().unwrap_or_default())
                    } else {
                        destination.to_path_buf()
                    };

                    let entry = Self::create_file_entry(
                        source, 
                        &dest_path, 
                        &metadata, 
                        &mut traversal.hard_link_map,
                        preserve_links
                    ).await?;

                    if entry.is_symlink {
                        traversal.symlinks.push(entry);
                    } else {
                        traversal.total_size += entry.size;
                        traversal.total_files += 1;
                        traversal.files.push(entry);
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Source not found: {:?}", source));
            }
        }

        info!("Directory analysis complete: {} files, {} bytes, {} directories", 
              traversal.total_files, traversal.total_size, traversal.directories.len());

        Ok(traversal)
    }

    async fn traverse_directory(
        source_dir: &Path,
        dest_dir: &Path,
        traversal: &mut DirectoryTraversal,
        preserve_links: bool,
    ) -> Result<()> {
        let mut entries = fs::read_dir(source_dir).await
            .with_context(|| format!("Failed to read directory: {:?}", source_dir))?;

        // Add directory to create list
        traversal.directories.push(dest_dir.to_path_buf());

        while let Some(entry) = entries.next_entry().await? {
            let source_path = entry.path();
            let dest_path = dest_dir.join(entry.file_name());
            
            let metadata = entry.metadata().await?;

            if metadata.is_dir() {
                // Recursively traverse subdirectory
                Self::traverse_directory(
                    &source_path, 
                    &dest_path, 
                    traversal,
                    preserve_links
                ).await?;
            } else {
                let file_entry = Self::create_file_entry(
                    &source_path, 
                    &dest_path, 
                    &metadata,
                    &mut traversal.hard_link_map,
                    preserve_links
                ).await?;

                if file_entry.is_symlink {
                    traversal.symlinks.push(file_entry);
                } else {
                    traversal.total_size += file_entry.size;
                    traversal.total_files += 1;
                    traversal.files.push(file_entry);
                }
            }
        }

        Ok(())
    }

    async fn create_file_entry(
        source_path: &Path,
        dest_path: &Path,
        metadata: &fs::Metadata,
        hard_link_map: &mut HashMap<(u64, u64), PathBuf>,
        preserve_links: bool,
    ) -> Result<FileEntry> {
        let is_symlink = metadata.file_type().is_symlink();
        let size = if is_symlink { 0 } else { metadata.len() };
        
        // Check for sparse files (simplified detection)
        let is_sparse = if !is_symlink && size > 0 {
            Self::is_sparse_file(source_path, metadata).await.unwrap_or(false)
        } else {
            false
        };

        // Handle hard links
        let hard_links = if preserve_links && !is_symlink && metadata.nlink() > 1 {
            let key = (metadata.dev(), metadata.ino());
            
            if let Some(existing_path) = hard_link_map.get(&key) {
                // This is a hard link to an already processed file
                debug!("Found hard link: {:?} -> {:?}", source_path, existing_path);
                Some(key)
            } else {
                // First occurrence of this inode
                hard_link_map.insert(key, source_path.to_path_buf());
                Some(key)
            }
        } else {
            None
        };

        Ok(FileEntry {
            source_path: source_path.to_path_buf(),
            dest_path: dest_path.to_path_buf(),
            size,
            is_dir: metadata.is_dir(),
            is_symlink,
            is_sparse,
            hard_links,
        })
    }

    async fn is_sparse_file(path: &Path, metadata: &fs::Metadata) -> Result<bool> {
        // Simple sparse file detection: compare allocated blocks vs file size
        // This is a heuristic - actual sparse detection would use FIEMAP ioctl
        let file_size = metadata.len();
        let block_size = 4096u64; // Typical block size
        let blocks = metadata.blocks();
        let allocated_size = blocks * 512; // stat.st_blocks is in 512-byte units
        
        // If allocated size is significantly less than file size, it's likely sparse
        let sparseness_threshold = 0.9; // 90% of file size
        let is_sparse = allocated_size < (file_size as f64 * sparseness_threshold) as u64;
        
        if is_sparse {
            debug!("Detected sparse file: {:?} (size: {}, allocated: {})", 
                   path, file_size, allocated_size);
        }
        
        Ok(is_sparse)
    }

    pub async fn create_directories(directories: &[PathBuf]) -> Result<()> {
        for dir_path in directories {
            if let Err(e) = fs::create_dir_all(dir_path).await {
                if e.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(anyhow::anyhow!("Failed to create directory {:?}: {}", dir_path, e));
                }
            }
            debug!("Created directory: {:?}", dir_path);
        }
        Ok(())
    }

    pub async fn create_symlinks(symlinks: &[FileEntry]) -> Result<()> {
        for entry in symlinks {
            // Read the symlink target
            let target = fs::read_link(&entry.source_path).await
                .with_context(|| format!("Failed to read symlink: {:?}", entry.source_path))?;
            
            // Create the symlink
            if let Err(e) = std::os::unix::fs::symlink(&target, &entry.dest_path) {
                warn!("Failed to create symlink {:?} -> {:?}: {}", 
                      entry.dest_path, target, e);
            } else {
                debug!("Created symlink: {:?} -> {:?}", entry.dest_path, target);
            }
        }
        Ok(())
    }

    pub async fn create_hard_links(
        files: &[FileEntry], 
        hard_link_map: &HashMap<(u64, u64), PathBuf>
    ) -> Result<()> {
        let mut processed_inodes: HashSet<(u64, u64)> = HashSet::new();
        
        for entry in files {
            if let Some(hard_link_key) = entry.hard_links {
                if processed_inodes.contains(&hard_link_key) {
                    // This is a subsequent hard link - create link instead of copying
                    if let Some(original_source) = hard_link_map.get(&hard_link_key) {
                        // Find the corresponding destination path for the original
                        if let Some(original_entry) = files.iter()
                            .find(|f| f.source_path == *original_source) {
                            
                            if let Err(e) = std::fs::hard_link(&original_entry.dest_path, &entry.dest_path) {
                                warn!("Failed to create hard link {:?} -> {:?}: {}", 
                                      entry.dest_path, original_entry.dest_path, e);
                            } else {
                                debug!("Created hard link: {:?} -> {:?}", 
                                       entry.dest_path, original_entry.dest_path);
                            }
                        }
                    }
                } else {
                    processed_inodes.insert(hard_link_key);
                }
            }
        }
        Ok(())
    }

    pub fn estimate_completion_time(
        transferred_bytes: u64,
        total_bytes: u64,
        elapsed_time: std::time::Duration,
    ) -> Option<std::time::Duration> {
        if transferred_bytes == 0 || total_bytes == 0 {
            return None;
        }

        let transfer_rate = transferred_bytes as f64 / elapsed_time.as_secs_f64();
        let remaining_bytes = total_bytes.saturating_sub(transferred_bytes);
        let eta_seconds = remaining_bytes as f64 / transfer_rate;
        
        if eta_seconds.is_finite() && eta_seconds > 0.0 {
            Some(std::time::Duration::from_secs_f64(eta_seconds))
        } else {
            None
        }
    }
} 