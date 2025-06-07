use crate::config::Config;
use crate::job::{JobManager, Job};
use crate::metrics::Metrics;
use crate::protocol::*;
use anyhow::{Result, Context};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{info, error, debug, warn};

pub struct Daemon {
    config: Config,
    job_manager: JobManager,
    metrics: Metrics,
    start_time: Instant,
}

impl Daemon {
    pub async fn new(config: Config) -> Result<Self> {
        // Ensure required directories exist
        config.ensure_directories().await?;

        // Initialize job manager
        let (job_manager, _event_receiver) = JobManager::new(
            config.max_concurrent_jobs,
            config.checkpoint_dir.clone()
        );
        
        // Initialize metrics
        let metrics = Metrics::new()?;

        Ok(Self {
            config,
            job_manager,
            metrics,
            start_time: Instant::now(),
        })
    }

    pub async fn run(&self) -> Result<()> {
        info!("Starting copyd daemon on socket: {:?}", self.config.socket_path);

        // Remove existing socket if it exists
        if self.config.socket_path.exists() {
            tokio::fs::remove_file(&self.config.socket_path).await?;
        }

        // Create Unix domain socket listener
        let listener = UnixListener::bind(&self.config.socket_path)
            .with_context(|| format!("Failed to bind to socket: {:?}", self.config.socket_path))?;

        info!("Daemon listening on socket: {:?}", self.config.socket_path);

        // Resume jobs from checkpoints
        match self.job_manager.resume_jobs_from_checkpoints().await {
            Ok(count) => {
                if count > 0 {
                    info!("Resumed {} jobs from checkpoints", count);
                }
            }
            Err(e) => {
                warn!("Failed to resume jobs from checkpoints: {}", e);
            }
        }

        // Start job queue processor
        self.job_manager.start_queue_processor().await;

        // Start metrics server if configured
        if let Some(metrics_addr) = &self.config.metrics_bind_addr {
            let metrics = self.metrics.clone();
            let addr = metrics_addr.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::run_metrics_server(metrics, addr).await {
                    error!("Metrics server error: {}", e);
                }
            });
        }

        // Accept connections
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let daemon = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = daemon.handle_client(stream).await {
                            error!("Client handler error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    async fn handle_client(&self, mut stream: UnixStream) -> Result<()> {
        debug!("New client connected");

        loop {
            // Read request from client
            let request = match receive_request(&mut stream).await {
                Ok(req) => req,
                Err(e) => {
                    debug!("Client disconnected or error reading request: {}", e);
                    break;
                }
            };

            debug!("Received request: {:?}", request);

            // Process request and send response
            let response = self.process_request(request).await;
            
            if let Err(e) = send_response(&mut stream, &response).await {
                error!("Failed to send response: {}", e);
                break;
            }
        }

        Ok(())
    }

    async fn process_request(&self, request: Request) -> Response {
        use request::RequestType;
        use response::ResponseType;

        let response_type = match request.request_type {
            Some(RequestType::CreateJob(req)) => {
                ResponseType::CreateJob(self.handle_create_job(req).await)
            }
            Some(RequestType::JobStatus(req)) => {
                ResponseType::JobStatus(self.handle_job_status(req).await)
            }
            Some(RequestType::ListJobs(req)) => {
                ResponseType::ListJobs(self.handle_list_jobs(req).await)
            }
            Some(RequestType::CancelJob(req)) => {
                ResponseType::CancelJob(self.handle_cancel_job(req).await)
            }
            Some(RequestType::PauseJob(req)) => {
                ResponseType::PauseJob(self.handle_pause_job(req).await)
            }
            Some(RequestType::ResumeJob(req)) => {
                ResponseType::ResumeJob(self.handle_resume_job(req).await)
            }
            Some(RequestType::GetStats(req)) => {
                ResponseType::GetStats(self.handle_get_stats(req).await)
            }
            Some(RequestType::HealthCheck(req)) => {
                ResponseType::HealthCheck(self.handle_health_check(req).await)
            }
            None => {
                ResponseType::CreateJob(CreateJobResponse {
                    job_id: None,
                    error: "Invalid request".to_string(),
                })
            }
        };

        Response {
            response_type: Some(response_type),
        }
    }

    async fn handle_create_job(&self, request: CreateJobRequest) -> CreateJobResponse {
        match self.job_manager.create_job(request).await {
            Ok(job_id) => {
                self.metrics.record_job_created();
                CreateJobResponse {
                    job_id: Some(JobId { uuid: job_id }),
                    error: String::new(),
                }
            }
            Err(e) => CreateJobResponse {
                job_id: None,
                error: format!("Failed to create job: {}", e),
            },
        }
    }

    async fn handle_job_status(&self, request: JobStatusRequest) -> JobStatusResponse {
        let job_id = match request.job_id {
            Some(id) => id.uuid,
            None => {
                return JobStatusResponse {
                    job_id: None,
                    progress: None,
                    error: "Missing job_id".to_string(),
                    log_entries: vec![],
                }
            }
        };
        
        match self.job_manager.get_job(&job_id).await {
            Some(job) => JobStatusResponse {
                job_id: Some(JobId { uuid: job_id }),
                progress: Some(job.progress),
                error: String::new(),
                log_entries: job.log_entries,
            },
            None => JobStatusResponse {
                job_id: Some(JobId { uuid: job_id }),
                progress: None,
                error: "Job not found".to_string(),
                log_entries: vec![],
            },
        }
    }

    async fn handle_list_jobs(&self, request: ListJobsRequest) -> ListJobsResponse {
        let jobs = self.job_manager.list_jobs(request.include_completed).await;
        
        let job_infos = jobs.into_iter().map(|job| JobInfo {
            job_id: Some(JobId { uuid: job.id }),
            sources: job.sources.into_iter().map(|p| p.to_string_lossy().to_string()).collect(),
            destination: job.destination.to_string_lossy().to_string(),
            progress: Some(job.progress),
            created_at: job.created_at.timestamp(),
            started_at: job.started_at.map(|t| t.timestamp()).unwrap_or(0),
            completed_at: job.completed_at.map(|t| t.timestamp()).unwrap_or(0),
            priority: job.priority,
        }).collect();

        ListJobsResponse { jobs: job_infos }
    }

    async fn handle_cancel_job(&self, request: CancelJobRequest) -> CancelJobResponse {
        let job_id = request.job_id.map(|id| id.uuid).unwrap_or_default();
        
        match self.job_manager.cancel_job(&job_id).await {
            Ok(()) => CancelJobResponse {
                success: true,
                error: String::new(),
            },
            Err(e) => CancelJobResponse {
                success: false,
                error: format!("Failed to cancel job: {}", e),
            },
        }
    }

    async fn handle_pause_job(&self, request: PauseJobRequest) -> PauseJobResponse {
        let job_id = request.job_id.map(|id| id.uuid).unwrap_or_default();
        
        match self.job_manager.pause_job(&job_id).await {
            Ok(()) => PauseJobResponse {
                success: true,
                error: String::new(),
            },
            Err(e) => PauseJobResponse {
                success: false,
                error: format!("Failed to pause job: {}", e),
            },
        }
    }

    async fn handle_resume_job(&self, request: ResumeJobRequest) -> ResumeJobResponse {
        let job_id = request.job_id.map(|id| id.uuid).unwrap_or_default();
        
        match self.job_manager.resume_job(&job_id).await {
            Ok(()) => ResumeJobResponse {
                success: true,
                error: String::new(),
            },
            Err(e) => ResumeJobResponse {
                success: false,
                error: format!("Failed to resume job: {}", e),
            },
        }
    }

    async fn handle_get_stats(&self, _request: GetStatsRequest) -> StatsResponse {
        // TODO: Implement proper statistics gathering
        StatsResponse {
            total_bytes_copied: 0,
            total_files_copied: 0,
            total_jobs: 0,
            daily_stats: vec![],
            slow_paths: vec![],
        }
    }

    async fn handle_health_check(&self, _request: HealthCheckRequest) -> HealthCheckResponse {
        // TODO: Implement proper health checks
        HealthCheckResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_seconds: self.start_time.elapsed().as_secs() as i64,
            active_jobs: 0, // TODO: Get from job manager
            queued_jobs: 0, // TODO: Get from job manager
            memory_usage_bytes: 0, // TODO: Get actual memory usage
            cpu_usage_percent: 0.0, // TODO: Get actual CPU usage
        }
    }

    async fn run_metrics_server(metrics: Metrics, addr: String) -> Result<()> {
        use std::convert::Infallible;
        use std::net::SocketAddr;

        let make_svc = hyper::service::make_service_fn(move |_conn| {
            let metrics = metrics.clone();
            async move {
                Ok::<_, Infallible>(hyper::service::service_fn(move |req| {
                    let metrics = metrics.clone();
                    async move {
                        if req.uri().path() == "/metrics" {
                            match metrics.export() {
                                Ok(body) => Ok(hyper::Response::new(hyper::Body::from(body))),
                                Err(e) => {
                                    error!("Failed to export metrics: {}", e);
                                    Ok(hyper::Response::builder()
                                        .status(500)
                                        .body(hyper::Body::from("Internal Server Error"))
                                        .unwrap())
                                }
                            }
                        } else {
                            Ok(hyper::Response::builder()
                                .status(404)
                                .body(hyper::Body::from("Not Found"))
                                .unwrap())
                        }
                    }
                }))
            }
        });

        let addr: SocketAddr = addr.parse()?;
        let server = hyper::Server::bind(&addr).serve(make_svc);

        info!("Metrics server listening on http://{}/metrics", addr);

        if let Err(e) = server.await {
            error!("Metrics server error: {}", e);
        }

        Ok(())
    }

    pub async fn is_healthy(&self) -> bool {
        // TODO: Implement comprehensive health checks
        true
    }
}

impl Clone for Daemon {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            job_manager: self.job_manager.clone(),
            metrics: self.metrics.clone(),
            start_time: self.start_time,
        }
    }
} 