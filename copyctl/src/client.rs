use copyd_protocol::*;
use anyhow::{Result, Context};
use std::path::Path;
use tokio::net::UnixStream;
use tracing::debug;

pub struct CopyClient {
    socket_path: std::path::PathBuf,
}

impl CopyClient {
    pub async fn new(socket_path: impl AsRef<Path>) -> Result<Self> {
        let socket_path = socket_path.as_ref().to_path_buf();
        
        // Test connection
        let mut stream = UnixStream::connect(&socket_path).await
            .with_context(|| format!("Failed to connect to daemon at {:?}", socket_path))?;
        
        // Send a health check to verify the daemon is working
        let health_request = Request {
            request_type: Some(request::RequestType::HealthCheck(HealthCheckRequest {})),
        };
        
        send_request(&mut stream, &health_request).await?;
        let response = receive_response(&mut stream).await?;
        
        match response.response_type {
            Some(response::ResponseType::HealthCheck(health)) => {
                if !health.healthy {
                    anyhow::bail!("Daemon reports unhealthy status");
                }
                debug!("Connected to daemon version {}", health.version);
            }
            _ => anyhow::bail!("Unexpected response to health check"),
        }
        
        Ok(Self { socket_path })
    }

    async fn send_request(&self, request: Request) -> Result<Response> {
        let mut stream = UnixStream::connect(&self.socket_path).await
            .with_context(|| format!("Failed to connect to daemon at {:?}", self.socket_path))?;
        
        send_request(&mut stream, &request).await?;
        let response = receive_response(&mut stream).await?;
        
        Ok(response)
    }

    pub async fn create_job(&self, request: CreateJobRequest) -> Result<String> {
        let request = Request {
            request_type: Some(request::RequestType::CreateJob(request)),
        };
        
        let response = self.send_request(request).await?;
        
        match response.response_type {
            Some(response::ResponseType::CreateJob(create_response)) => {
                if !create_response.error.is_empty() {
                    anyhow::bail!("Failed to create job: {}", create_response.error);
                }
                
                match create_response.job_id {
                    Some(job_id) => Ok(job_id.uuid),
                    None => anyhow::bail!("No job ID returned"),
                }
            }
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    pub async fn get_job_status(&self, job_id: &str) -> Result<JobStatusResponse> {
        let request = Request {
            request_type: Some(request::RequestType::JobStatus(JobStatusRequest {
                job_id: Some(JobId { uuid: job_id.to_string() }),
            })),
        };
        
        let response = self.send_request(request).await?;
        
        match response.response_type {
            Some(response::ResponseType::JobStatus(status_response)) => {
                Ok(status_response)
            }
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    pub async fn list_jobs(&self, include_completed: bool) -> Result<Vec<JobInfo>> {
        let request = Request {
            request_type: Some(request::RequestType::ListJobs(ListJobsRequest {
                include_completed,
            })),
        };
        
        let response = self.send_request(request).await?;
        
        match response.response_type {
            Some(response::ResponseType::ListJobs(list_response)) => {
                Ok(list_response.jobs)
            }
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    pub async fn cancel_job(&self, job_id: &str) -> Result<()> {
        let request = Request {
            request_type: Some(request::RequestType::CancelJob(CancelJobRequest {
                job_id: Some(JobId { uuid: job_id.to_string() }),
            })),
        };
        
        let response = self.send_request(request).await?;
        
        match response.response_type {
            Some(response::ResponseType::CancelJob(cancel_response)) => {
                if !cancel_response.success {
                    anyhow::bail!("Failed to cancel job: {}", cancel_response.error);
                }
                Ok(())
            }
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    pub async fn pause_job(&self, job_id: &str) -> Result<()> {
        let request = Request {
            request_type: Some(request::RequestType::PauseJob(PauseJobRequest {
                job_id: Some(JobId { uuid: job_id.to_string() }),
            })),
        };
        
        let response = self.send_request(request).await?;
        
        match response.response_type {
            Some(response::ResponseType::PauseJob(pause_response)) => {
                if !pause_response.success {
                    anyhow::bail!("Failed to pause job: {}", pause_response.error);
                }
                Ok(())
            }
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    pub async fn resume_job(&self, job_id: &str) -> Result<()> {
        let request = Request {
            request_type: Some(request::RequestType::ResumeJob(ResumeJobRequest {
                job_id: Some(JobId { uuid: job_id.to_string() }),
            })),
        };
        
        let response = self.send_request(request).await?;
        
        match response.response_type {
            Some(response::ResponseType::ResumeJob(resume_response)) => {
                if !resume_response.success {
                    anyhow::bail!("Failed to resume job: {}", resume_response.error);
                }
                Ok(())
            }
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    pub async fn get_stats(&self, days_back: i32) -> Result<StatsResponse> {
        let request = Request {
            request_type: Some(request::RequestType::GetStats(GetStatsRequest {
                days_back,
            })),
        };
        
        let response = self.send_request(request).await?;
        
        match response.response_type {
            Some(response::ResponseType::GetStats(stats_response)) => {
                Ok(stats_response)
            }
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    pub async fn health_check(&self) -> Result<HealthCheckResponse> {
        let request = Request {
            request_type: Some(request::RequestType::HealthCheck(HealthCheckRequest {})),
        };
        
        let response = self.send_request(request).await?;
        
        match response.response_type {
            Some(response::ResponseType::HealthCheck(health_response)) => {
                Ok(health_response)
            }
            _ => anyhow::bail!("Unexpected response type"),
        }
    }
} 