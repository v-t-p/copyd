use copyd_protocol::*;
use crate::copy_engine::{CopyOptions, FileCopyEngine};
use crate::directory::{DirectoryHandler, DirectoryTraversal};
use crate::checkpoint::{CheckpointManager, JobCheckpoint};
use anyhow::{Result, Context};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{RwLock, mpsc, Semaphore};
use tokio::time::Duration;
use tracing::{info, error};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
    pub options: JobOptions,
    pub progress: Progress,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub priority: u32,
    pub log_entries: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct JobOptions {
    pub recursive: bool,
    pub preserve_metadata: bool,
    pub preserve_links: bool,
    pub preserve_sparse: bool,
    pub verify: VerifyMode,
    pub exists_action: ExistsAction,
    pub max_rate_bps: Option<u64>,
    pub engine: CopyEngine,
    pub dry_run: bool,
    pub regex_rename_match: Option<String>,
    pub regex_rename_replace: Option<String>,
    pub block_size: Option<u64>,
    pub compress: bool,
    pub encrypt: bool,
}

impl Job {
    pub fn new(request: CreateJobRequest) -> Self {
        let id = Uuid::new_v4().to_string();
        let sources = request.sources.into_iter().map(PathBuf::from).collect();
        let destination = PathBuf::from(request.destination);
        
        let options = JobOptions {
            recursive: request.recursive,
            preserve_metadata: request.preserve_metadata,
            preserve_links: request.preserve_links,
            preserve_sparse: request.preserve_sparse,
            verify: VerifyMode::try_from(request.verify).unwrap_or(VerifyMode::None),
            exists_action: ExistsAction::try_from(request.exists_action).unwrap_or(ExistsAction::Overwrite),
            max_rate_bps: if request.max_rate_bps > 0 { Some(request.max_rate_bps) } else { None },
            engine: CopyEngine::try_from(request.engine).unwrap_or(CopyEngine::Auto),
            dry_run: request.dry_run,
            regex_rename_match: if request.regex_rename_match.is_empty() { None } else { Some(request.regex_rename_match) },
            regex_rename_replace: if request.regex_rename_replace.is_empty() { None } else { Some(request.regex_rename_replace) },
            block_size: if request.block_size > 0 { Some(request.block_size) } else { None },
            compress: request.compress,
            encrypt: request.encrypt,
        };

        Self {
            id,
            sources,
            destination,
            options,
            progress: Progress {
                bytes_copied: 0,
                total_bytes: 0,
                files_copied: 0,
                total_files: 0,
                throughput_mbps: 0.0,
                eta_seconds: 0,
                status: JobStatus::Pending.into(),
            },
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            priority: request.priority,
            log_entries: Vec::new(),
        }
    }

    pub fn add_log(&mut self, message: String) {
        self.log_entries.push(format!("{}: {}", Utc::now().format("%Y-%m-%d %H:%M:%S"), message));
        
        // Keep only the last 100 log entries
        if self.log_entries.len() > 100 {
            self.log_entries.drain(..self.log_entries.len() - 100);
        }
    }

    pub fn get_status(&self) -> JobStatus {
        JobStatus::try_from(self.progress.status).unwrap_or(JobStatus::Pending)
    }

    pub fn set_status(&mut self, status: JobStatus) {
        self.progress.status = status.into();
        match status {
            JobStatus::Running => {
                if self.started_at.is_none() {
                    self.started_at = Some(Utc::now());
                }
            }
            JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled => {
                if self.completed_at.is_none() {
                    self.completed_at = Some(Utc::now());
                }
            }
            _ => {}
        }
    }
}

pub struct JobManager {
    jobs: Arc<RwLock<HashMap<String, Job>>>,
    job_queue: Arc<RwLock<VecDeque<String>>>,
    active_jobs: Arc<RwLock<HashMap<String, tokio::task::JoinHandle<()>>>>,
    max_concurrent: usize,
    semaphore: Arc<Semaphore>,
    event_sender: mpsc::UnboundedSender<JobEvent>,
    checkpoint_manager: Arc<CheckpointManager>,
}

impl JobManager {
    pub fn new(max_concurrent: usize, checkpoint_dir: PathBuf) -> (Self, mpsc::UnboundedReceiver<JobEvent>) {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        
        let checkpoint_manager = Arc::new(
            CheckpointManager::new(checkpoint_dir)
                .expect("Failed to create checkpoint manager")
        );
        
        let manager = Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            job_queue: Arc::new(RwLock::new(VecDeque::new())),
            active_jobs: Arc::new(RwLock::new(HashMap::new())),
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            event_sender,
            checkpoint_manager,
        };

        (manager, event_receiver)
    }

    pub async fn create_job(&self, request: CreateJobRequest) -> Result<String> {
        let job = Job::new(request);
        let job_id = job.id.clone();
        
        info!("Created job {}: {:?} -> {:?}", job_id, job.sources, job.destination);
        
        // Add to jobs map
        {
            let mut jobs = self.jobs.write().await;
            jobs.insert(job_id.clone(), job);
        }

        // Add to queue based on priority
        {
            let mut queue = self.job_queue.write().await;
            queue.push_back(job_id.clone());
        }

        // Try to start the job immediately if capacity allows
        self.try_start_next_job().await;

        Ok(job_id)
    }

    pub async fn get_job(&self, job_id: &str) -> Option<Job> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).cloned()
    }

    pub async fn list_jobs(&self, include_completed: bool) -> Vec<Job> {
        let jobs = self.jobs.read().await;
        jobs.values()
            .filter(|job| include_completed || matches!(job.get_status(), JobStatus::Pending | JobStatus::Running | JobStatus::Paused))
            .cloned()
            .collect()
    }

    pub async fn cancel_job(&self, job_id: &str) -> Result<()> {
        // Remove from queue
        {
            let mut queue = self.job_queue.write().await;
            queue.retain(|id| id != job_id);
        }

        // Cancel active job
        {
            let mut active = self.active_jobs.write().await;
            if let Some(handle) = active.remove(job_id) {
                handle.abort();
            }
        }

        // Update job status
        {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(job_id) {
                job.set_status(JobStatus::Cancelled);
                job.add_log("Job cancelled by user".to_string());
            }
        }

        info!("Cancelled job {}", job_id);
        Ok(())
    }

    pub async fn pause_job(&self, job_id: &str) -> Result<()> {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            if job.get_status() == JobStatus::Running {
                job.set_status(JobStatus::Paused);
                job.add_log("Job paused".to_string());
                info!("Paused job {}", job_id);
            }
        }
        Ok(())
    }

    pub async fn resume_job(&self, job_id: &str) -> Result<()> {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            if job.get_status() == JobStatus::Paused {
                job.set_status(JobStatus::Pending);
                job.add_log("Job resumed".to_string());
                info!("Resumed job {}", job_id);
                
                // Add back to queue
                drop(jobs);
                
                let mut queue = self.job_queue.write().await;
                queue.push_back(job_id.to_string());
            }
        }
        
        // Try to start the job
        self.try_start_next_job().await;
        Ok(())
    }

    async fn try_start_next_job(&self) {
        if self.semaphore.available_permits() == 0 {
            return;
        }

        let job_id = {
            let mut queue = self.job_queue.write().await;
            queue.pop_front()
        };

        if let Some(job_id) = job_id {
            if let Ok(permit) = self.semaphore.clone().try_acquire_owned() {
                let jobs = self.jobs.clone();
                let event_sender = self.event_sender.clone();
                let active_jobs = self.active_jobs.clone();
                let job_id_clone = job_id.clone();
                
                let handle = tokio::spawn(async move {
                    let _permit = permit; // Hold permit for duration of job
                    
                    // Execute the job
                    if let Err(e) = Self::execute_job(&job_id_clone, jobs.clone(), event_sender).await {
                        error!("Job {} failed: {}", job_id_clone, e);
                        
                        // Update job status to failed
                        let mut jobs_guard = jobs.write().await;
                        if let Some(job) = jobs_guard.get_mut(&job_id_clone) {
                            job.set_status(JobStatus::Failed);
                            job.add_log(format!("Job failed: {}", e));
                        }
                    }
                    
                    // Remove from active jobs
                    let mut active = active_jobs.write().await;
                    active.remove(&job_id_clone);
                });

                let mut active = self.active_jobs.write().await;
                active.insert(job_id, handle);
            }
        }
    }

    async fn execute_job(
        job_id: &str,
        jobs: Arc<RwLock<HashMap<String, Job>>>,
        event_sender: mpsc::UnboundedSender<JobEvent>,
    ) -> Result<()> {
        info!("Starting execution of job {}", job_id);
        
        let start_time = Instant::now();
        
        // Get job details and mark as running
        let (sources, destination, options) = {
            let mut jobs_guard = jobs.write().await;
            let job = jobs_guard.get_mut(job_id)
                .context("Job not found")?;
            
            job.set_status(JobStatus::Running);
            job.add_log("Job started".to_string());
            
            (job.sources.clone(), job.destination.clone(), job.options.clone())
        };

        // Send status update event
        let _ = event_sender.send(JobEvent {
            job_id: Some(JobId { uuid: job_id.to_string() }),
            event_type: Some(job_event::EventType::StatusChange(JobStatus::Running.into())),
        });

        // Execute the copy operation
        let result = Self::execute_copy_operation(
            job_id, 
            &sources, 
            &destination, 
            &options, 
            jobs.clone(), 
            event_sender
        ).await;

        // Update final job status
        let duration = start_time.elapsed();
        {
            let mut jobs_guard = jobs.write().await;
            if let Some(job) = jobs_guard.get_mut(job_id) {
                match result {
                    Ok(_) => {
                        job.set_status(JobStatus::Completed);
                        let throughput = if duration.as_secs_f64() > 0.0 {
                            job.progress.bytes_copied as f64 / duration.as_secs_f64() / 1024.0 / 1024.0
                        } else {
                            0.0
                        };
                        
                        let message = format!("Job completed successfully in {:.2}s ({:.2} MB/s)", 
                                            duration.as_secs_f64(), throughput);
                        job.add_log(message);
                        info!("Completed job {} in {:.2}s", job_id, duration.as_secs_f64());
                    }
                    Err(ref e) => {
                        job.set_status(JobStatus::Failed);
                        let error_msg = format!("Job failed: {}", e);
                        job.add_log(error_msg);
                        error!("Job {} failed: {}", job_id, e);
                    }
                }
            }
        }

        result
    }

    async fn execute_copy_operation(
        job_id: &str,
        sources: &[PathBuf],
        destination: &Path,
        options: &JobOptions,
        _jobs: Arc<RwLock<HashMap<String, Job>>>,
        event_sender: &mpsc::UnboundedSender<JobEvent>,
    ) -> Result<()> {
        let copy_options = CopyOptions {
            preserve_metadata: options.preserve_metadata,
            preserve_links: options.preserve_links,
            preserve_sparse: options.preserve_sparse,
            verify: options.verify,
            exists_action: options.exists_action,
            max_rate_bps: options.max_rate_bps,
            block_size: options.block_size,
            dry_run: options.dry_run,
            compress: options.compress,
            encrypt: options.encrypt,
        };

        let copy_engine = FileCopyEngine::new(options.engine);

        // 1. Analyze sources to get a plan of action
        let traversal = DirectoryHandler::analyze_sources(sources, destination, options.recursive, options.preserve_links).await?;

        // 2. Create all directories first
        DirectoryHandler::create_directories(&traversal.directories).await?;

        // 3. Copy all regular files
        for file_entry in traversal.files {
            let dest_path = file_entry.dest_path.clone();
            match copy_engine.copy_file(&file_entry.source_path, &dest_path, &copy_options).await {
                Ok(bytes_copied) => {
                    let _ = event_sender.send(JobEvent {
                        job_id: Some(JobId { uuid: job_id.to_string() }),
                        event_type: Some(job_event::EventType::FileCompleted(FileCompletedEvent {
                            file_path: dest_path.to_string_lossy().to_string(),
                            bytes_copied,
                        })),
                    });
                }
                Err(e) => {
                     let _ = event_sender.send(JobEvent {
                        job_id: Some(JobId { uuid: job_id.to_string() }),
                        event_type: Some(job_event::EventType::FileError(FileErrorEvent {
                            file_path: dest_path.to_string_lossy().to_string(),
                            error: e.to_string(),
                        })),
                    });
                }
            }
        }
        
        // 4. Create symlinks if needed
        if options.preserve_links {
            DirectoryHandler::create_symlinks(&traversal.symlinks).await?;
        }

        Ok(())
    }

    async fn add_job_log(jobs: Arc<RwLock<HashMap<String, Job>>>, job_id: &str, message: String) {
        let mut jobs_guard = jobs.write().await;
        if let Some(job) = jobs_guard.get_mut(job_id) {
            job.add_log(message);
        }
    }

    async fn calculate_total_size(sources: &[PathBuf], recursive: bool) -> Result<u64> {
        let mut total = 0;
        
        for source in sources {
            if let Ok(metadata) = tokio::fs::metadata(source).await {
                if metadata.is_file() {
                    total += metadata.len();
                } else if metadata.is_dir() && recursive {
                    // TODO: Recursively calculate directory size
                    total += 1024; // Placeholder
                }
            }
        }
        
        Ok(total)
    }

    pub async fn resume_jobs_from_checkpoints(&self) -> Result<usize> {
        info!("Scanning for resumable jobs...");
        
        let resumable_jobs = self.checkpoint_manager.list_resumable_jobs().await?;
        let mut resumed_count = 0;

        for job_id in resumable_jobs {
            if let Some(mut checkpoint) = self.checkpoint_manager.load_checkpoint(&job_id).await? {
                info!("Resuming job {} (resume count: {})", job_id, checkpoint.resume_count);
                
                checkpoint.increment_resume_count();
                self.checkpoint_manager.save_checkpoint(&checkpoint).await?;

                // Create a new job from the checkpoint
                let job = self.create_job_from_checkpoint(checkpoint).await?;
                
                // Add to jobs map
                {
                    let mut jobs = self.jobs.write().await;
                    jobs.insert(job_id.clone(), job);
                }

                // Add to queue
                {
                    let mut queue = self.job_queue.write().await;
                    queue.push_front(job_id); // Prioritize resumed jobs
                }

                resumed_count += 1;
            }
        }

        if resumed_count > 0 {
            info!("Resumed {} jobs from checkpoints", resumed_count);
            self.try_start_next_job().await;
        }

        Ok(resumed_count)
    }

    async fn create_job_from_checkpoint(&self, checkpoint: JobCheckpoint) -> Result<Job> {
        // Reconstruct job from checkpoint data
        let mut job = Job {
            id: checkpoint.job_id.clone(),
            sources: Vec::new(), // Will be populated from checkpoint files
            destination: PathBuf::new(), // Will be set from first file
            options: JobOptions {
                recursive: true,
                preserve_metadata: true,
                preserve_links: false,
                preserve_sparse: false,
                verify: VerifyMode::None,
                exists_action: ExistsAction::Overwrite,
                max_rate_bps: None,
                engine: CopyEngine::Auto,
                dry_run: false,
                regex_rename_match: None,
                regex_rename_replace: None,
                block_size: None,
                compress: false,
                encrypt: false,
            },
            progress: Progress {
                bytes_copied: checkpoint.bytes_completed,
                total_bytes: checkpoint.total_bytes,
                files_copied: checkpoint.completed_files.len() as u64,
                total_files: checkpoint.total_files as u64,
                throughput_mbps: 0.0,
                eta_seconds: 0,
                status: JobStatus::Pending.into(),
            },
            created_at: DateTime::from_timestamp(checkpoint.created_at as i64, 0).unwrap_or(Utc::now()),
            started_at: None,
            completed_at: None,
            priority: 100, // Default priority for resumed jobs
            log_entries: vec![format!("Job resumed from checkpoint (resume count: {})", checkpoint.resume_count)],
        };

        // Extract source and destination from checkpoint files
        if let Some((_, file_checkpoint)) = checkpoint.files.iter().next() {
            job.destination = file_checkpoint.destination_path.parent()
                .unwrap_or(&file_checkpoint.destination_path)
                .to_path_buf();
            job.sources.push(file_checkpoint.source_path.clone());
        }

        Ok(job)
    }

    pub async fn start_queue_processor(&self) {
        let manager = self.clone();
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(100));
            loop {
                interval.tick().await;
                manager.try_start_next_job().await;
            }
        });
    }
}

impl Clone for JobManager {
    fn clone(&self) -> Self {
        Self {
            jobs: self.jobs.clone(),
            job_queue: self.job_queue.clone(),
            active_jobs: self.active_jobs.clone(),
            max_concurrent: self.max_concurrent,
            semaphore: self.semaphore.clone(),
            event_sender: self.event_sender.clone(),
            checkpoint_manager: self.checkpoint_manager.clone(),
        }
    }
} 