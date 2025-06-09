use anyhow::Result;
use copyd::{Config, Daemon, JobManager, CopyEngine, FileCopyEngine, CheckpointManager, DirectoryHandler};
use std::path::{Path, PathBuf};
use tempfile::{TempDir, NamedTempFile};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use std::time::Duration;

#[tokio::test]
async fn test_copy_engine_basic_file_copy() -> Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create source file
    let source_path = temp_dir.path().join("source.txt");
    let mut source_file = fs::File::create(&source_path).await?;
    source_file.write_all(b"Hello, World!").await?;
    source_file.sync_all().await?;
    drop(source_file);
    
    // Create destination path
    let dest_path = temp_dir.path().join("dest.txt");
    
    // Test read_write copy engine
    let copy_engine = FileCopyEngine::new(CopyEngine::ReadWrite);
    let options = copyd::CopyOptions {
        preserve_metadata: true,
        preserve_links: false,
        preserve_sparse: false,
        verify: copyd::protocol::VerifyMode::None,
        exists_action: copyd::protocol::ExistsAction::Overwrite,
        max_rate_bps: None,
        block_size: Some(4096),
        dry_run: false,
        compress: false,
        encrypt: false,
    };
    
    let bytes_copied = copy_engine.copy_file(&source_path, &dest_path, &options).await?;
    
    // Verify copy
    assert_eq!(bytes_copied, 13);
    assert!(dest_path.exists());
    
    let dest_content = fs::read_to_string(&dest_path).await?;
    assert_eq!(dest_content, "Hello, World!");
    
    Ok(())
}

#[tokio::test]
async fn test_directory_traversal() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let source_dir = temp_dir.path().join("source");
    fs::create_dir(&source_dir).await?;
    
    // Create test directory structure
    let subdir = source_dir.join("subdir");
    fs::create_dir(&subdir).await?;
    
    let file1 = source_dir.join("file1.txt");
    fs::write(&file1, b"file1 content").await?;
    
    let file2 = subdir.join("file2.txt");
    fs::write(&file2, b"file2 content").await?;
    
    let dest_dir = temp_dir.path().join("dest");
    
    // Test directory analysis
    let traversal = DirectoryHandler::analyze_sources(
        &[source_dir.clone()],
        &dest_dir,
        true, // recursive
        false, // preserve_links
    ).await?;
    
    assert_eq!(traversal.total_files, 2);
    assert_eq!(traversal.total_size, 26); // "file1 content" + "file2 content"
    assert_eq!(traversal.directories.len(), 2); // source and subdir
    
    Ok(())
}

#[tokio::test]
async fn test_checkpoint_system() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let checkpoint_manager = CheckpointManager::new(temp_dir.path().to_path_buf())?;
    
    // Create test checkpoint
    let mut checkpoint = copyd::JobCheckpoint::new("test-job".to_string(), "copy".to_string());
    
    let file_checkpoint = copyd::FileCheckpoint {
        source_path: PathBuf::from("/tmp/source.txt"),
        destination_path: PathBuf::from("/tmp/dest.txt"),
        bytes_copied: 512,
        total_size: 1024,
        last_modified: 1234567890,
        checksum_partial: Some("abc123".to_string()),
        chunk_size: 4096,
        created_at: 1234567890,
        updated_at: 1234567890,
    };
    
    checkpoint.add_file("file1".to_string(), file_checkpoint);
    
    // Save checkpoint
    checkpoint_manager.save_checkpoint(&checkpoint).await?;
    
    // Load checkpoint
    let loaded = checkpoint_manager.load_checkpoint("test-job").await?;
    assert!(loaded.is_some());
    
    let loaded_checkpoint = loaded.unwrap();
    assert_eq!(loaded_checkpoint.job_id, "test-job");
    assert_eq!(loaded_checkpoint.total_files, 1);
    assert_eq!(loaded_checkpoint.total_bytes, 1024);
    assert!(loaded_checkpoint.files.contains_key("file1"));
    
    // Test cleanup
    checkpoint_manager.delete_checkpoint("test-job").await?;
    let deleted = checkpoint_manager.load_checkpoint("test-job").await?;
    assert!(deleted.is_none());
    
    Ok(())
}

#[tokio::test]
async fn test_job_manager_basic_operations() -> Result<()> {
    let (job_manager, mut event_receiver) = JobManager::new(2);
    
    // Create test job
    let request = copyd::protocol::CreateJobRequest {
        sources: vec!["/tmp/test.txt".to_string()],
        destination: "/tmp/dest.txt".to_string(),
        recursive: false,
        preserve_metadata: true,
        preserve_links: false,
        preserve_sparse: false,
        verify: copyd::protocol::VerifyMode::None.into(),
        exists_action: copyd::protocol::ExistsAction::Overwrite.into(),
        priority: 100,
        max_rate_bps: 0,
        engine: 0,
        dry_run: false,
        regex_rename_match: String::new(),
        regex_rename_replace: String::new(),
        block_size: 0,
        compress: false,
        encrypt: false,
    };
    
    let job_id = job_manager.create_job(request).await?;
    assert!(!job_id.is_empty());
    
    // Get job
    let job = job_manager.get_job(&job_id).await;
    assert!(job.is_some());
    
    let job = job.unwrap();
    assert_eq!(job.id, job_id);
    assert_eq!(job.sources.len(), 1);
    assert_eq!(job.destination, PathBuf::from("/tmp/dest.txt"));
    
    // List jobs
    let jobs = job_manager.list_jobs(false).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, job_id);
    
    Ok(())
}

#[tokio::test]
async fn test_copy_engines_fallback() -> Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create test file
    let source_path = temp_dir.path().join("source.txt");
    let test_data = b"Test data for copy engine fallback";
    fs::write(&source_path, test_data).await?;
    
    let dest_path = temp_dir.path().join("dest.txt");
    
    let options = copyd::CopyOptions {
        preserve_metadata: false,
        preserve_links: false,
        preserve_sparse: false,
        verify: copyd::protocol::VerifyMode::None,
        exists_action: copyd::protocol::ExistsAction::Overwrite,
        max_rate_bps: None,
        block_size: Some(1024),
        dry_run: false,
        compress: false,
        encrypt: false,
    };
    
    // Test auto engine (should fall back to available engine)
    let auto_engine = FileCopyEngine::new(CopyEngine::Auto);
    let bytes_copied = auto_engine.copy_file(&source_path, &dest_path, &options).await?;
    
    assert_eq!(bytes_copied, test_data.len() as u64);
    assert!(dest_path.exists());
    
    let copied_data = fs::read(&dest_path).await?;
    assert_eq!(copied_data, test_data);
    
    Ok(())
}

#[tokio::test]
async fn test_sparse_file_detection() -> Result<()> {
    // This test requires Linux sparse file support
    if !cfg!(target_os = "linux") {
        return Ok(());
    }
    
    let temp_dir = TempDir::new()?;
    let sparse_file = temp_dir.path().join("sparse.txt");
    
    // Create a sparse file using standard library
    use std::fs::OpenOptions;
    use std::io::{Seek, SeekFrom, Write};
    
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&sparse_file)?;
    
    // Write some data at the beginning
    file.write_all(b"start")?;
    
    // Seek to create a hole
    file.seek(SeekFrom::Start(1024 * 1024))?; // 1MB hole
    
    // Write some data at the end
    file.write_all(b"end")?;
    
    drop(file);
    
    // Test sparse file detection
    let is_sparse = copyd::sparse::SparseFileHandler::is_sparse_file(&sparse_file).await.unwrap_or(false);
    
    // Note: This may not always detect as sparse depending on filesystem
    // but the test verifies the detection logic runs without error
    println!("Sparse file detected: {}", is_sparse);
    
    Ok(())
}

#[tokio::test]
async fn test_verification_system() -> Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create test file
    let test_file = temp_dir.path().join("test.txt");
    let test_data = b"Test data for verification";
    fs::write(&test_file, test_data).await?;
    
    // Test different verification modes
    let verifier = copyd::verify::FileVerifier::new();
    
    // Size verification
    let size_result = verifier.verify_file(&test_file, copyd::verify::VerifyMode::Size).await?;
    assert!(size_result.verified);
    assert_eq!(size_result.calculated_checksum, test_data.len().to_string());
    
    // SHA256 verification
    let sha256_result = verifier.verify_file(&test_file, copyd::verify::VerifyMode::Sha256).await?;
    assert!(sha256_result.verified);
    assert!(!sha256_result.calculated_checksum.is_empty());
    assert_eq!(sha256_result.calculated_checksum.len(), 64); // SHA256 hex length
    
    Ok(())
}

#[tokio::test]
async fn test_concurrent_job_execution() -> Result<()> {
    let (job_manager, mut event_receiver) = JobManager::new(2);
    let temp_dir = TempDir::new()?;
    
    // Create multiple test files
    let mut job_ids = Vec::new();
    
    for i in 0..3 {
        let source_file = temp_dir.path().join(format!("source{}.txt", i));
        fs::write(&source_file, format!("test data {}", i)).await?;
        
        let request = copyd::protocol::CreateJobRequest {
            sources: vec![source_file.to_string_lossy().to_string()],
            destination: temp_dir.path().join(format!("dest{}.txt", i)).to_string_lossy().to_string(),
            recursive: false,
            preserve_metadata: false,
            preserve_links: false,
            preserve_sparse: false,
            verify: copyd::protocol::VerifyMode::None.into(),
            exists_action: copyd::protocol::ExistsAction::Overwrite.into(),
            priority: 100,
            max_rate_bps: 0,
            engine: 0,
            dry_run: false,
            regex_rename_match: String::new(),
            regex_rename_replace: String::new(),
            block_size: 0,
            compress: false,
            encrypt: false,
        };
        
        let job_id = job_manager.create_job(request).await?;
        job_ids.push(job_id);
    }
    
    // Start queue processor
    job_manager.start_queue_processor().await;
    
    // Wait for jobs to be processed
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Check that jobs were created
    assert_eq!(job_ids.len(), 3);
    
    let jobs = job_manager.list_jobs(true).await;
    assert_eq!(jobs.len(), 3);
    
    Ok(())
}

#[tokio::test]
async fn test_rate_limiting() -> Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create a larger test file
    let source_path = temp_dir.path().join("large_source.txt");
    let test_data = vec![b'A'; 1024 * 1024]; // 1MB of data
    fs::write(&source_path, &test_data).await?;
    
    let dest_path = temp_dir.path().join("large_dest.txt");
    
    let options = copyd::CopyOptions {
        preserve_metadata: false,
        preserve_links: false,
        preserve_sparse: false,
        verify: copyd::protocol::VerifyMode::None,
        exists_action: copyd::protocol::ExistsAction::Overwrite,
        max_rate_bps: Some(1024 * 1024), // 1MB/s limit
        block_size: Some(64 * 1024),     // 64KB blocks
        dry_run: false,
        compress: false,
        encrypt: false,
    };
    
    let copy_engine = FileCopyEngine::new(CopyEngine::ReadWrite);
    
    let start_time = std::time::Instant::now();
    let bytes_copied = copy_engine.copy_file(&source_path, &dest_path, &options).await?;
    let elapsed = start_time.elapsed();
    
    assert_eq!(bytes_copied, test_data.len() as u64);
    
    // Rate limiting should make this take at least close to 1 second
    // (though we allow some variance for test reliability)
    assert!(elapsed >= Duration::from_millis(500)); // At least 0.5 seconds
    
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_io_uring_availability() -> Result<()> {
    use copyd::io_uring_engine::IoUringCopyEngine;
    
    let is_available = IoUringCopyEngine::is_io_uring_available();
    println!("io_uring available: {}", is_available);
    
    // If available, test basic functionality
    if is_available {
        let temp_dir = TempDir::new()?;
        let source_path = temp_dir.path().join("io_uring_source.txt");
        let test_data = b"io_uring test data";
        fs::write(&source_path, test_data).await?;
        
        let dest_path = temp_dir.path().join("io_uring_dest.txt");
        
        let mut engine = IoUringCopyEngine::new(32, Some(64 * 1024))?;
        let stats = engine.copy_file_async(&source_path, &dest_path, None).await?;
        
        assert_eq!(stats.bytes_read, test_data.len() as u64);
        assert_eq!(stats.bytes_written, test_data.len() as u64);
        assert!(dest_path.exists());
        
        let copied_data = fs::read(&dest_path).await?;
        assert_eq!(copied_data, test_data);
    }
    
    Ok(())
}

// Benchmark test for performance regression detection
#[tokio::test]
async fn test_copy_performance_benchmark() -> Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create a 10MB test file
    let source_path = temp_dir.path().join("benchmark_source.bin");
    let test_data = vec![0u8; 10 * 1024 * 1024]; // 10MB
    fs::write(&source_path, &test_data).await?;
    
    let dest_path = temp_dir.path().join("benchmark_dest.bin");
    
    let options = copyd::CopyOptions {
        preserve_metadata: false,
        preserve_links: false,
        preserve_sparse: false,
        verify: copyd::protocol::VerifyMode::None,
        exists_action: copyd::protocol::ExistsAction::Overwrite,
        max_rate_bps: None,
        block_size: Some(1024 * 1024), // 1MB blocks
        dry_run: false,
        compress: false,
        encrypt: false,
    };
    
    let copy_engine = FileCopyEngine::new(CopyEngine::Auto);
    
    let start_time = std::time::Instant::now();
    let bytes_copied = copy_engine.copy_file(&source_path, &dest_path, &options).await?;
    let elapsed = start_time.elapsed();
    
    assert_eq!(bytes_copied, test_data.len() as u64);
    
    let throughput_mbps = bytes_copied as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0);
    println!("Copy throughput: {:.2} MB/s", throughput_mbps);
    
    // Expect at least 50 MB/s on reasonable hardware
    // (this is a conservative threshold for CI environments)
    assert!(throughput_mbps > 50.0, "Copy performance too low: {:.2} MB/s", throughput_mbps);
    
    Ok(())
} 