use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::{sleep, timeout};
use tracing::{info, warn};

// ============================================================================
// Common Race Models (shared by all adapters)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Race {
    pub id: String,
    pub source: String,
    pub title: String,
    pub state: RaceState,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_sec: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deeplink: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaceUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<RaceState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_sec: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deeplink: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RaceState {
    Queued,
    Running,
    Passed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ============================================================================
// Configuration Base Types
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_server_url")]
    pub url: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_retry_count")]
    pub max_retries: u32,
}

fn default_server_url() -> String {
    "http://127.0.0.1:7777".to_string()
}

fn default_timeout() -> u64 {
    30
}

fn default_retry_count() -> u32 {
    3
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            url: default_server_url(),
            timeout_seconds: default_timeout(),
            max_retries: default_retry_count(),
        }
    }
}

// ============================================================================
// Raceboard API Client
// ============================================================================

#[derive(Clone)]
pub struct RaceboardClient {
    client: Client,
    server_url: String,
    timeout: Duration,
    max_retries: u32,
}

impl RaceboardClient {
    pub fn new(config: ServerConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            server_url: config.url,
            timeout: Duration::from_secs(config.timeout_seconds),
            max_retries: config.max_retries,
        })
    }

    pub async fn create_race(&self, race: &Race) -> Result<Race> {
        let url = format!("{}/race", self.server_url);
        
        let response = self
            .execute_with_retry(|| async {
                self.client
                    .post(&url)
                    .json(race)
                    .send()
                    .await
            })
            .await
            .context("Failed to create race")?;

        response
            .json()
            .await
            .context("Failed to parse race response")
    }

    pub async fn update_race(&self, race_id: &str, update: &RaceUpdate) -> Result<()> {
        let url = format!("{}/race/{}", self.server_url, race_id);
        
        self.execute_with_retry(|| async {
            self.client
                .patch(&url)
                .json(update)
                .send()
                .await
        })
        .await
        .context("Failed to update race")?;

        Ok(())
    }

    pub async fn add_event(&self, race_id: &str, event: &Event) -> Result<()> {
        let url = format!("{}/race/{}/event", self.server_url, race_id);
        
        self.execute_with_retry(|| async {
            self.client
                .post(&url)
                .json(event)
                .send()
                .await
        })
        .await
        .context("Failed to add event")?;

        Ok(())
    }

    pub async fn get_races(&self) -> Result<Vec<Race>> {
        let url = format!("{}/races", self.server_url);
        
        let response = self
            .execute_with_retry(|| async {
                self.client
                    .get(&url)
                    .send()
                    .await
            })
            .await
            .context("Failed to get races")?;

        response
            .json()
            .await
            .context("Failed to parse races response")
    }

    pub async fn delete_race(&self, race_id: &str) -> Result<()> {
        let url = format!("{}/race/{}", self.server_url, race_id);
        
        self.execute_with_retry(|| async {
            self.client
                .delete(&url)
                .send()
                .await
        })
        .await
        .context("Failed to delete race")?;

        Ok(())
    }

    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/health", self.server_url);
        
        match timeout(Duration::from_secs(5), self.client.get(&url).send()).await {
            Ok(Ok(response)) => Ok(response.status() == StatusCode::OK),
            _ => Ok(false),
        }
    }

    async fn execute_with_retry<F, Fut>(&self, f: F) -> Result<Response>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = reqwest::Result<Response>>,
    {
        let mut retries = 0;
        let mut last_error = None;

        while retries <= self.max_retries {
            match timeout(self.timeout, f()).await {
                Ok(Ok(response)) => {
                    if response.status().is_success() {
                        return Ok(response);
                    } else if response.status().is_server_error() && retries < self.max_retries {
                        warn!(
                            "Server error ({}), retrying... ({}/{})",
                            response.status(),
                            retries + 1,
                            self.max_retries
                        );
                    } else {
                        return Err(anyhow::anyhow!(
                            "Request failed with status: {}",
                            response.status()
                        ));
                    }
                }
                Ok(Err(e)) => {
                    last_error = Some(e.to_string());
                    if retries < self.max_retries {
                        let backoff = Duration::from_secs(2u64.pow(retries));
                        warn!(
                            "Request failed: {}. Retrying in {:?}... ({}/{})",
                            e,
                            backoff,
                            retries + 1,
                            self.max_retries
                        );
                        sleep(backoff).await;
                    }
                }
                Err(_) => {
                    last_error = Some("Request timed out".to_string());
                    if retries < self.max_retries {
                        warn!(
                            "Request timed out. Retrying... ({}/{})",
                            retries + 1,
                            self.max_retries
                        );
                    }
                }
            }
            retries += 1;
        }

        Err(anyhow::anyhow!(
            "All retries exhausted. Last error: {}",
            last_error.unwrap_or_else(|| "Unknown error".to_string())
        ))
    }
}

// ============================================================================
// Configuration Loading
// ============================================================================

pub fn load_config_file<T: for<'de> Deserialize<'de>>(path: Option<PathBuf>) -> Result<T> {
    let config_path = resolve_config_path(path)?;
    
    info!("Loading configuration from: {}", config_path.display());
    
    let contents = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
    
    toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", config_path.display()))
}

pub fn resolve_config_path(path: Option<PathBuf>) -> Result<PathBuf> {
    // Priority order:
    // 1. Command line argument (--config flag)
    // 2. Environment variable
    // 3. Default locations

    if let Some(path) = path {
        return Ok(path);
    }

    // Check for environment variable (adapter-specific)
    let exe_name = std::env::current_exe()?
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("adapter")
        .to_string();
    
    let env_var = format!("{}_CONFIG", exe_name.to_uppercase().replace('-', "_"));
    
    if let Ok(path) = std::env::var(&env_var) {
        return Ok(PathBuf::from(path));
    }

    // Check default locations
    let default_paths = vec![
        PathBuf::from(format!("./{}.toml", exe_name)),
        PathBuf::from(format!("./config/{}.toml", exe_name)),
        dirs::config_dir()
            .map(|d| d.join("raceboard").join(format!("{}.toml", exe_name)))
            .unwrap_or_default(),
    ];

    for path in default_paths {
        if path.exists() {
            return Ok(path);
        }
    }

    Err(anyhow::anyhow!(
        "No configuration file found. Please provide one via --config flag or {} environment variable",
        env_var
    ))
}

// ============================================================================
// Common CLI Arguments
// ============================================================================

use clap::Parser;

#[derive(Parser, Debug)]
pub struct CommonArgs {
    /// Path to configuration file
    #[arg(short, long)]
    pub config: Option<PathBuf>,

    /// Server URL (overrides config)
    #[arg(short, long)]
    pub server: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    pub log_level: String,

    /// Enable verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

// ============================================================================
// Signal Handling
// ============================================================================

pub async fn shutdown_signal() {
    use tokio::signal;
    
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received shutdown signal (Ctrl+C)");
        },
        _ = terminate => {
            info!("Received terminate signal");
        },
    }
}

// ============================================================================
// Progress Tracking
// ============================================================================

pub struct ProgressTracker {
    start_time: DateTime<Utc>,
    total_steps: Option<u32>,
    current_step: u32,
}

impl ProgressTracker {
    pub fn new(total_steps: Option<u32>) -> Self {
        Self {
            start_time: Utc::now(),
            total_steps,
            current_step: 0,
        }
    }

    pub fn increment(&mut self) -> (Option<i32>, Option<i64>) {
        self.current_step += 1;
        
        let progress = self.total_steps.map(|total| {
            ((self.current_step as f32 / total as f32) * 100.0) as i32
        });
        
        let eta_sec = self.estimate_eta();
        
        (progress, eta_sec)
    }

    pub fn set_step(&mut self, step: u32) -> (Option<i32>, Option<i64>) {
        self.current_step = step;
        
        let progress = self.total_steps.map(|total| {
            ((self.current_step as f32 / total as f32) * 100.0) as i32
        });
        
        let eta_sec = self.estimate_eta();
        
        (progress, eta_sec)
    }

    fn estimate_eta(&self) -> Option<i64> {
        if self.current_step == 0 {
            return None;
        }
        
        self.total_steps.map(|total| {
            let elapsed = (Utc::now() - self.start_time).num_seconds();
            let rate = elapsed as f64 / self.current_step as f64;
            let remaining = (total - self.current_step) as f64;
            (remaining * rate) as i64
        })
    }
}

// ============================================================================
// Health Check Server
// ============================================================================

use actix_web::{web, App, HttpResponse, HttpServer, middleware};

pub async fn health_handler() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "timestamp": Utc::now().to_rfc3339(),
    }))
}

pub async fn run_health_server(port: u16) -> Result<()> {
    info!("Starting health check server on port {}", port);
    
    HttpServer::new(|| {
        App::new()
            .wrap(middleware::Logger::default())
            .route("/health", web::get().to(health_handler))
    })
    .bind(format!("127.0.0.1:{}", port))?
    .run()
    .await?;
    
    Ok(())
}

// ============================================================================
// Testing Utilities
// ============================================================================

#[cfg(test)]
pub mod test_utils {
    use super::*;
    use mockito;

    pub fn mock_server_url() -> String {
        mockito::server_url()
    }

    pub fn create_test_race(id: &str) -> Race {
        Race {
            id: id.to_string(),
            source: "test".to_string(),
            title: "Test Race".to_string(),
            state: RaceState::Running,
            started_at: Utc::now(),
            eta_sec: Some(60),
            progress: Some(50),
            deeplink: None,
            metadata: None,
        }
    }
}