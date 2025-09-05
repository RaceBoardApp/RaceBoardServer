use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tokio::signal;

#[derive(Debug, Deserialize)]
struct Config {
    gitlab: GitLabConfig,
    raceboard: RaceboardConfig,
    sync: SyncConfig,
    #[serde(default)]
    webhook: WebhookConfig,
}

#[derive(Debug, Deserialize)]
struct GitLabConfig {
    url: String,
    api_token: String,
    user_id: u64,
    #[serde(default)]
    project_ids: Vec<u64>,  // Additional specific projects to monitor
}

#[derive(Debug, Clone, Deserialize)]
struct RaceboardConfig {
    server_url: String,
}

#[derive(Debug, Deserialize)]
struct SyncConfig {
    interval_seconds: u64,
    max_pipelines: usize,
    lookback_hours: i64,
}

#[derive(Debug, Deserialize, Clone)]
struct WebhookConfig {
    enabled: bool,
    port: u16,
    secret: String,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8082,
            secret: String::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitLabPipeline {
    id: u64,
    project_id: u64,
    status: String,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    sha: String,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    web_url: String,
}

#[derive(Debug, Deserialize)]
struct GitLabProject {
    id: u64,
    name: String,
    path_with_namespace: String,
}

#[derive(Debug, Deserialize)]
struct GitLabJob {
    id: u64,
    status: String,
    name: String,
}

// Webhook event structures
#[derive(Debug, Deserialize)]
struct WebhookEvent {
    object_kind: String,
    project: WebhookProject,
    #[serde(flatten)]
    data: WebhookData,
}

#[derive(Debug, Deserialize)]
struct WebhookProject {
    id: u64,
    name: String,
    path_with_namespace: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WebhookData {
    Pipeline {
        object_attributes: PipelineAttributes,
    },
    Job {
        build_id: u64,
        build_status: String,
        pipeline_id: u64,
    },
}

#[derive(Debug, Deserialize)]
struct PipelineAttributes {
    id: u64,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    status: String,
    sha: String,
    created_at: String,
    finished_at: Option<String>,
    user: Option<WebhookUser>,
}

#[derive(Debug, Deserialize)]
struct WebhookUser {
    id: u64,
}

#[derive(Debug, Serialize)]
struct Race {
    id: String,
    source: String,
    title: String,
    state: String,
    started_at: DateTime<Utc>,
    eta_sec: Option<i64>,
    progress: Option<u8>,
    deeplink: Option<String>,
    metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AdapterState {
    last_sync: DateTime<Utc>,
    last_pipeline_ids: HashSet<u64>,
}

struct GitLabClient {
    client: Client,
    base_url: String,
    headers: HeaderMap,
    additional_project_ids: Vec<u64>,
    username: Option<String>,
}

impl GitLabClient {
    fn new(config: &GitLabConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Private-Token",
            HeaderValue::from_str(&config.api_token)
                .context("Invalid API token")?,
        );

        Ok(Self {
            client: Client::new(),
            base_url: config.url.trim_end_matches('/').to_string(),
            headers,
            additional_project_ids: config.project_ids.clone(),
            username: None,
        })
    }
    
    async fn verify_token(&mut self) -> Result<()> {
        let url = format!("{}/api/v4/user", self.base_url);
        log::info!("Verifying API token with: {}", url);
        
        let response = self.client
            .get(&url)
            .headers(self.headers.clone())
            .send()
            .await?;
            
        let status = response.status();
        
        if !status.is_success() {
            let body = response.text().await?;
            
            if status == StatusCode::UNAUTHORIZED {
                return Err(anyhow::anyhow!(
                    "Invalid API token (401): The token is not recognized. Body: {}",
                    body
                ));
            } else if status == StatusCode::FORBIDDEN {
                return Err(anyhow::anyhow!(
                    "Forbidden (403): Token is valid but lacks permissions. Body: {}",
                    body
                ));
            } else {
                return Err(anyhow::anyhow!(
                    "Failed to verify token: Status={}, Body={}",
                    status, body
                ));
            }
        }
        
        let user_info: serde_json::Value = response.json().await?;
        let username = user_info["username"].as_str().unwrap_or("unknown").to_string();
        log::info!("API token verified. User: {} (ID: {})", 
            username,
            user_info["id"].as_u64().unwrap_or(0)
        );
        
        // Store the username for use in filtering pipelines
        self.username = Some(username);
        
        Ok(())
    }

    async fn get_user_pipelines(
        &self,
        user_id: u64,
        lookback_hours: i64,
    ) -> Result<Vec<GitLabPipeline>> {
        let mut all_pipelines = Vec::new();
        let cutoff = Utc::now() - chrono::Duration::hours(lookback_hours);
        
        // Try to get pipelines directly by user first
        log::info!("Attempting to fetch pipelines directly for user {}", user_id);
        
        // First try: Get recent pipelines across all projects the user can see
        let url = format!(
            "{}/api/v4/projects/pipelines?updated_after={}&per_page=100",
            self.base_url,
            cutoff.to_rfc3339()
        );
        
        if let Ok(response) = self.client
            .get(&url)
            .headers(self.headers.clone())
            .send()
            .await
        {
            if response.status().is_success() {
                if let Ok(pipelines) = response.json::<Vec<GitLabPipeline>>().await {
                    log::info!("Found {} pipelines via direct API", pipelines.len());
                    return Ok(pipelines);
                }
            }
        }
        
        // Fallback: Get all accessible projects first
        log::info!("Falling back to per-project pipeline fetching");
        let projects = self.get_user_projects(user_id).await?;
        log::info!("Found {} accessible projects", projects.len());
        
        for project in projects {
            let mut page = 1;
            let mut pages_fetched = 0;
            
            loop {
                if pages_fetched >= 1 {
                    break; // Only fetch 1 page (100 pipelines) sorted by newest
                }
                
                // Use created_after to only get new pipelines since last check
                // Also filter by username to only get your pipelines
                let created_after = (Utc::now() - chrono::Duration::hours(lookback_hours))
                    .to_rfc3339();
                let mut url = format!(
                    "{}/api/v4/projects/{}/pipelines?page={}&per_page=100&sort=desc&created_after={}",
                    self.base_url, project.id, page, created_after
                );
                
                // Add username filter if we have one
                if let Some(ref username) = self.username {
                    url.push_str(&format!("&username={}", username));
                }
                
                log::info!("Fetching pipelines for project '{}' (ID: {})", project.name, project.id);
                log::debug!("Fetching URL: {}", url);
                
                let response = self.client
                    .get(&url)
                    .headers(self.headers.clone())
                    .send()
                    .await?;
                
                let status = response.status();
                
                // Check for rate limiting first
                if status == StatusCode::TOO_MANY_REQUESTS {
                    let retry_after = response.headers()
                        .get("Retry-After")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(60);
                    log::warn!("Rate limited, waiting {} seconds", retry_after);
                    sleep(Duration::from_secs(retry_after)).await;
                    continue;
                }
                
                // Check for other error responses
                if !status.is_success() {
                    let body = response.text().await?;
                    
                    if status == StatusCode::UNAUTHORIZED {
                        return Err(anyhow::anyhow!("Unauthorized: Check your API token"));
                    } else if status == StatusCode::FORBIDDEN {
                        log::warn!("Access denied to project '{}' (ID: {}) - skipping", project.name, project.id);
                        break; // Skip this project
                    } else if status == StatusCode::NOT_FOUND {
                        log::warn!("Project '{}' (ID: {}) not found - skipping", project.name, project.id);
                        break; // Skip this project
                    } else {
                        log::error!("Error fetching pipelines for project '{}': Status={}, Body={}", 
                            project.name, status, body);
                        break; // Skip this project instead of failing entirely
                    }
                }
                
                // Try to parse the response body
                let body_text = response.text().await?;
                let pipelines: Vec<GitLabPipeline> = match serde_json::from_str(&body_text) {
                    Ok(p) => p,
                    Err(e) => {
                        log::error!("Failed to parse pipelines response: {}", e);
                        log::debug!("Response body: {}", body_text);
                        Vec::new() // Skip this project's pipelines
                    }
                };
                
                if pipelines.is_empty() {
                    break;
                }
                
                // Filter by user and time
                for mut pipeline in pipelines {
                    // Skip if too old
                    if pipeline.created_at < cutoff {
                        continue;
                    }
                    
                    // Store project_id for later use
                    pipeline.project_id = project.id;
                    all_pipelines.push(pipeline);
                }
                
                page += 1;
                pages_fetched += 1;
            }
        }
        
        Ok(all_pipelines)
    }

    async fn get_user_projects(&self, user_id: u64) -> Result<Vec<GitLabProject>> {
        let mut all_projects = Vec::new();
        let mut page = 1;
        
        // Try different API endpoints based on what works
        // First try: user's projects directly
        let mut try_user_endpoint = true;
        
        loop {
            let url = if try_user_endpoint && page == 1 {
                // First attempt: try getting user's projects directly
                format!(
                    "{}/api/v4/users/{}/projects?page={}&per_page=100",
                    self.base_url, user_id, page
                )
            } else {
                // Fallback: projects with membership
                format!(
                    "{}/api/v4/projects?membership=true&page={}&per_page=100",
                    self.base_url, page
                )
            };
            
            log::debug!("Fetching projects from: {}", url);
            
            let response = self.client
                .get(&url)
                .headers(self.headers.clone())
                .send()
                .await?;
            
            let status = response.status();
            
            if !status.is_success() {
                let body = response.text().await?;
                
                // If first attempt with user endpoint failed, try fallback
                if try_user_endpoint && page == 1 && status == StatusCode::FORBIDDEN {
                    log::warn!("User projects endpoint failed with 403, trying membership endpoint");
                    try_user_endpoint = false;
                    continue;
                }
                
                log::error!("Failed to fetch projects from {}: Status={}, Body={}", url, status, body);
                
                if status == StatusCode::UNAUTHORIZED {
                    return Err(anyhow::anyhow!("Unauthorized (401): Check your API token"));
                } else if status == StatusCode::FORBIDDEN {
                    return Err(anyhow::anyhow!("Forbidden (403): API token lacks required permissions. Ensure the token has 'read_api' scope"));
                }
                break;
            }
            
            let body_text = response.text().await?;
            let projects: Vec<GitLabProject> = match serde_json::from_str(&body_text) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("Failed to parse projects response: {}", e);
                    log::debug!("Response body: {}", body_text);
                    break;
                }
            };
            
            if projects.is_empty() {
                break;
            }
            
            all_projects.extend(projects);
            page += 1;
            
            if page > 5 { // Max 5 pages of projects
                break;
            }
        }
        
        // Add additional specified project IDs
        for project_id in &self.additional_project_ids {
            log::info!("Fetching additional project ID: {}", project_id);
            match self.get_project(*project_id).await {
                Ok(project) => {
                    log::info!("Added project: {} (ID: {})", project.name, project.id);
                    all_projects.push(project);
                }
                Err(e) => {
                    log::warn!("Failed to fetch project {}: {}", project_id, e);
                }
            }
        }
        
        Ok(all_projects)
    }

    async fn get_project(&self, project_id: u64) -> Result<GitLabProject> {
        let url = format!("{}/api/v4/projects/{}", self.base_url, project_id);
        
        let response = self.client
            .get(&url)
            .headers(self.headers.clone())
            .send()
            .await?;
        
        Ok(response.json().await?)
    }

    async fn get_pipeline_jobs(&self, project_id: u64, pipeline_id: u64) -> Result<Vec<GitLabJob>> {
        let url = format!(
            "{}/api/v4/projects/{}/pipelines/{}/jobs",
            self.base_url, project_id, pipeline_id
        );
        
        let response = self.client
            .get(&url)
            .headers(self.headers.clone())
            .send()
            .await?;
        
        Ok(response.json().await?)
    }
}

fn map_gitlab_state(state: &str) -> String {
    match state {
        "created" | "waiting_for_resource" | "pending" | "scheduled" | "manual" => "queued",
        "preparing" | "running" => "running",
        "success" => "passed",
        "failed" => "failed",
        "canceled" | "skipped" => "canceled",
        _ => "queued",
    }.to_string()
}

fn calculate_progress(jobs: &[GitLabJob]) -> Option<u8> {
    if jobs.is_empty() {
        return None;
    }
    
    let completed = jobs.iter()
        .filter(|j| matches!(j.status.as_str(), "success" | "skipped" | "manual"))
        .count();
    
    Some((completed * 100 / jobs.len()) as u8)
}

async fn pipeline_to_race(
    pipeline: &GitLabPipeline,
    gitlab: &GitLabClient,
) -> Result<Race> {
    // Get project details for the title
    let project = gitlab.get_project(pipeline.project_id).await?;
    
    // Try to get jobs for progress calculation
    let jobs = gitlab.get_pipeline_jobs(pipeline.project_id, pipeline.id)
        .await
        .unwrap_or_default();
    
    let branch = pipeline.ref_name.clone().unwrap_or_else(|| "unknown".to_string());
    let title = format!("{} - {}", project.name, branch);
    
    let mut metadata = HashMap::new();
    metadata.insert("project_name".to_string(), project.name);
    metadata.insert("branch".to_string(), branch);
    metadata.insert("commit_sha".to_string(), pipeline.sha.chars().take(8).collect());
    metadata.insert("pipeline_url".to_string(), pipeline.web_url.clone());
    
    Ok(Race {
        id: format!("gitlab-{}-{}", pipeline.project_id, pipeline.id),
        source: "gitlab".to_string(),
        title,
        state: map_gitlab_state(&pipeline.status),
        started_at: pipeline.started_at.unwrap_or(pipeline.created_at),
        eta_sec: None, // Server will calculate
        progress: calculate_progress(&jobs),
        deeplink: Some(pipeline.web_url.clone()),
        metadata,
    })
}

async fn upsert_race(race: &Race, config: &RaceboardConfig, is_new: bool) -> Result<()> {
    let client = Client::new();
    
    // Use create (POST) for new pipelines, update (PATCH) for existing ones
    let (url, method) = if is_new {
        (format!("{}/race", config.server_url), "POST")
    } else {
        (format!("{}/race/{}", config.server_url, race.id), "PATCH")
    };
    
    let request = if method == "POST" {
        client.post(&url)
    } else {
        client.patch(&url)
    };
    
    let response = request
        .json(race)
        .send()
        .await?;
    
    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to {} race: {} - {}",
            if is_new { "create" } else { "update" },
            response.status(),
            response.text().await?
        );
    }
    
    Ok(())
}

// Note: delete_race function removed - adapters should never delete races
// Races should only be marked as finished with appropriate state (passed/failed/canceled)

async fn call_with_retry<T, F, Fut>(mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut retries = 3;
    let mut backoff = 2;
    
    loop {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) if retries > 0 => {
                eprintln!("Error: {}. Retrying in {} seconds...", e, backoff);
                sleep(Duration::from_secs(backoff)).await;
                retries -= 1;
                backoff *= 2; // Exponential backoff
            }
            Err(e) => return Err(e),
        }
    }
}

fn load_config() -> Result<Config> {
    // Check for config file path from command line arguments
    let args: Vec<String> = std::env::args().collect();
    let config_path = if args.len() > 2 && args[1] == "--config" {
        PathBuf::from(&args[2])
    } else {
        PathBuf::from("gitlab_config.toml")
    };
    
    if !config_path.exists() {
        anyhow::bail!("Configuration file '{}' not found", config_path.display());
    }
    
    log::info!("Loading configuration from: {}", config_path.display());
    let config_str = std::fs::read_to_string(&config_path)?;
    toml::from_str(&config_str).context("Failed to parse configuration")
}

fn load_state() -> AdapterState {
    let state_path = PathBuf::from(".gitlab_adapter_state.json");
    if state_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&state_path) {
            if let Ok(state) = serde_json::from_str(&content) {
                return state;
            }
        }
    }
    
    AdapterState {
        last_sync: Utc::now() - chrono::Duration::hours(24),
        last_pipeline_ids: HashSet::new(),
    }
}

fn save_state(state: &AdapterState) -> Result<()> {
    let state_path = PathBuf::from(".gitlab_adapter_state.json");
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(state_path, content)?;
    Ok(())
}

// Global metrics
struct Metrics {
    api_calls: AtomicU64,
    races_created: AtomicU64,
    races_updated: AtomicU64,
    races_finished: AtomicU64,  // Track races marked as finished (passed/failed/canceled)
    errors: AtomicU64,
    last_sync: Arc<tokio::sync::RwLock<DateTime<Utc>>>,
    is_healthy: AtomicBool,
}

impl Metrics {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            api_calls: AtomicU64::new(0),
            races_created: AtomicU64::new(0),
            races_updated: AtomicU64::new(0),
            races_finished: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            last_sync: Arc::new(tokio::sync::RwLock::new(Utc::now())),
            is_healthy: AtomicBool::new(true),
        })
    }
}

fn verify_webhook_signature(secret: &str, signature: &str, body: &[u8]) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    
    if secret.is_empty() {
        return true; // No secret configured, accept all
    }
    
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    
    let expected = mac.finalize();
    let expected_hex = hex::encode(expected.into_bytes());
    
    signature == expected_hex
}

async fn handle_webhook_event(
    event: WebhookEvent,
    config: &RaceboardConfig,
    user_id: u64,
) -> Result<()> {
    match event.data {
        WebhookData::Pipeline { object_attributes } => {
            // Check if this pipeline is for our user
            if let Some(user) = object_attributes.user {
                if user.id != user_id {
                    log::debug!("Ignoring pipeline for different user: {}", user.id);
                    return Ok(());
                }
            } else {
                log::debug!("Ignoring pipeline with no user");
                return Ok(());
            }
            
            // Create race from webhook data
            let race = Race {
                id: format!("gitlab-{}-{}", event.project.id, object_attributes.id),
                source: "gitlab".to_string(),
                title: format!("{} - {}", 
                    event.project.name, 
                    object_attributes.ref_name.as_deref().unwrap_or("unknown")),
                state: map_gitlab_state(&object_attributes.status),
                started_at: DateTime::parse_from_rfc3339(&object_attributes.created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                eta_sec: None,
                progress: None, // Would need to fetch jobs to calculate
                deeplink: None, // Not provided in webhook
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("project_name".to_string(), event.project.name);
                    m.insert("branch".to_string(), 
                        object_attributes.ref_name.unwrap_or_else(|| "unknown".to_string()));
                    m.insert("commit_sha".to_string(), 
                        object_attributes.sha.chars().take(8).collect());
                    m
                },
            };
            
            let is_new = matches!(object_attributes.status.as_str(), "created" | "pending");
            upsert_race(&race, config, is_new).await?;
            
            log::info!("Processed webhook for pipeline {}", object_attributes.id);
        }
        WebhookData::Job { pipeline_id, .. } => {
            log::debug!("Received job webhook for pipeline {}", pipeline_id);
            // Jobs are used for progress calculation, but we'd need to fetch
            // all jobs for the pipeline to calculate progress
        }
    }
    
    Ok(())
}

async fn start_webhook_server(
    config: WebhookConfig,
    raceboard_config: RaceboardConfig,
    user_id: u64,
    metrics: Arc<Metrics>,
) {
    use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
    
    let port = config.port;
    HttpServer::new(move || {
        let config = config.clone();
        let raceboard_config = raceboard_config.clone();
        let metrics = metrics.clone();
        
        App::new()
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(raceboard_config.clone()))
            .app_data(web::Data::new(metrics.clone()))
            .route("/webhooks/gitlab", web::post().to(move |
                req: HttpRequest,
                body: web::Bytes,
                webhook_config: web::Data<WebhookConfig>,
                raceboard_cfg: web::Data<RaceboardConfig>,
                metrics: web::Data<Arc<Metrics>>,
            | {
                let user_id = user_id;
                async move {
                    // Verify signature
                    if let Some(signature) = req.headers().get("X-Gitlab-Token") {
                        let sig_str = signature.to_str().unwrap_or("");
                        if sig_str != webhook_config.secret && !webhook_config.secret.is_empty() {
                            log::warn!("Invalid webhook signature");
                            return HttpResponse::Unauthorized().finish();
                        }
                    } else if !webhook_config.secret.is_empty() {
                        log::warn!("Missing webhook signature");
                        return HttpResponse::Unauthorized().finish();
                    }
                    
                    // Parse event
                    match serde_json::from_slice::<WebhookEvent>(&body) {
                        Ok(event) => {
                            log::info!("Received {} webhook", event.object_kind);
                            
                            // Process in background to respond quickly
                            let raceboard_cfg = (*raceboard_cfg).clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_webhook_event(event, &raceboard_cfg, user_id).await {
                                    log::error!("Failed to handle webhook: {}", e);
                                }
                            });
                            
                            metrics.api_calls.fetch_add(1, Ordering::Relaxed);
                            HttpResponse::Ok().finish()
                        }
                        Err(e) => {
                            log::error!("Failed to parse webhook: {}", e);
                            metrics.errors.fetch_add(1, Ordering::Relaxed);
                            HttpResponse::BadRequest().finish()
                        }
                    }
                }
            }))
    })
    .bind(("0.0.0.0", port))
    .unwrap()
    .run()
    .await
    .unwrap();
}

async fn start_health_server(metrics: Arc<Metrics>) -> actix_web::dev::Server {
    use actix_web::{web, App, HttpResponse, HttpServer};
    
    let metrics_clone = metrics.clone();
    
    let server = HttpServer::new(move || {
        let metrics = metrics_clone.clone();
        App::new()
            .app_data(web::Data::new(metrics.clone()))
            .route("/health", web::get().to(move |data: web::Data<Arc<Metrics>>| async move {
                let is_healthy = data.is_healthy.load(Ordering::Relaxed);
                let last_sync = *data.last_sync.read().await;
                let age = Utc::now() - last_sync;
                
                // Unhealthy if last sync was more than 5 minutes ago
                let json_response = serde_json::json!({
                    "status": if is_healthy && age.num_seconds() < 300 { "healthy" } else { "unhealthy" },
                    "last_sync": last_sync.to_rfc3339(),
                    "metrics": {
                        "api_calls": data.api_calls.load(Ordering::Relaxed),
                        "races_created": data.races_created.load(Ordering::Relaxed),
                        "races_updated": data.races_updated.load(Ordering::Relaxed),
                        "races_finished": data.races_finished.load(Ordering::Relaxed),
                        "errors": data.errors.load(Ordering::Relaxed),
                    }
                });
                
                if is_healthy && age.num_seconds() < 300 {
                    HttpResponse::Ok().json(json_response)
                } else {
                    HttpResponse::ServiceUnavailable().json(json_response)
                }
            }))
    })
    .disable_signals()  // Important: disable actix's own signal handling
    .bind("127.0.0.1:8081")
    .unwrap()
    .run();
    
    log::info!("Health check server started on http://127.0.0.1:8081/health");
    server
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    
    log::info!("Starting GitLab Pipeline Adapter for Raceboard");
    
    let config = load_config()?;
    log::info!("Configuration loaded successfully");
    log::info!("GitLab URL from config: {}", config.gitlab.url);
    log::info!("GitLab User ID: {}", config.gitlab.user_id);
    
    let mut gitlab = GitLabClient::new(&config.gitlab)?;
    log::info!("GitLab client initialized for {}", gitlab.base_url);
    
    // Verify the API token works
    gitlab.verify_token().await?;
    
    let mut state = load_state();
    log::info!("State loaded. Last sync: {}", state.last_sync);
    
    // Initialize metrics
    let metrics = Metrics::new();
    
    // Start health check server
    let metrics_clone = metrics.clone();
    let health_server = start_health_server(metrics_clone).await;
    let health_handle = tokio::spawn(health_server);
    
    // Start webhook server if enabled
    if config.webhook.enabled {
        let webhook_config = config.webhook.clone();
        let raceboard_config = config.raceboard.clone();
        let user_id = config.gitlab.user_id;
        let metrics_clone = metrics.clone();
        
        tokio::spawn(async move {
            log::info!("Starting webhook server on http://0.0.0.0:{}/webhooks/gitlab", 
                webhook_config.port);
            start_webhook_server(webhook_config, raceboard_config, user_id, metrics_clone).await;
        });
    } else {
        log::info!("Webhook server disabled");
    }
    
    // Set up shutdown signal handler
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                log::info!("Received shutdown signal, stopping gracefully...");
                shutdown_clone.store(true, Ordering::Relaxed);
            }
            Err(err) => {
                log::error!("Unable to listen for shutdown signal: {}", err);
            }
        }
    });
    
    while !shutdown.load(Ordering::Relaxed) {
        log::info!("Starting sync cycle...");
        
        // Get pipelines with retry
        let pipelines_result = call_with_retry(|| async {
            metrics.api_calls.fetch_add(1, Ordering::Relaxed);
            gitlab.get_user_pipelines(
                config.gitlab.user_id,
                config.sync.lookback_hours,
            ).await
        }).await;
        
        match pipelines_result {
            Ok(pipelines) => {
                log::info!("Found {} pipelines for user {}", pipelines.len(), config.gitlab.user_id);
                
                let mut processed = 0;
                let mut new_pipeline_ids = HashSet::new();
                
                for pipeline in pipelines.iter().take(config.sync.max_pipelines) {
                    // Check if this is a new pipeline or an update
                    let was_tracking = state.last_pipeline_ids.contains(&pipeline.id);
                    let is_running = matches!(pipeline.status.as_str(), 
                        "created" | "waiting_for_resource" | "preparing" | "pending" | "running" | "manual");
                    
                    // Track active pipelines for next sync
                    if is_running {
                        new_pipeline_ids.insert(pipeline.id);
                    }
                    
                    // Process all pipelines to ensure we update final states
                    // We process:
                    // 1. All running pipelines (new or existing)
                    // 2. All pipelines we were tracking (to update final status)
                    if is_running || was_tracking {
                        match pipeline_to_race(pipeline, &gitlab).await {
                            Ok(race) => {
                                let is_new = !was_tracking;
                                
                                log::info!("Processing pipeline {} (status: {}) -> race {} ({})", 
                                    pipeline.id, pipeline.status, race.id, if is_new { "new" } else { "update" });
                                
                                if let Err(e) = upsert_race(&race, &config.raceboard, is_new).await {
                                    log::error!("Failed to upsert race {}: {}", race.id, e);
                                    metrics.errors.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    processed += 1;
                                    if is_new {
                                        metrics.races_created.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        metrics.races_updated.fetch_add(1, Ordering::Relaxed);
                                        // Track if this update marks the race as finished
                                        if matches!(race.state.as_str(), "passed" | "failed" | "canceled") && was_tracking {
                                            metrics.races_finished.fetch_add(1, Ordering::Relaxed);
                                            log::info!("Pipeline {} finished with state: {}", pipeline.id, race.state);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to convert pipeline {}: {}", pipeline.id, e);
                                metrics.errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    } else {
                        log::debug!("Skipping finished pipeline {} (status: {})", pipeline.id, pipeline.status);
                    }
                }
                
                // Note: We no longer delete races for finished pipelines
                // Finished pipelines should remain in the history as completed races
                // Only truly deleted pipelines (404 from GitLab) should be removed
                
                state.last_sync = Utc::now();
                state.last_pipeline_ids = new_pipeline_ids;
                if let Err(e) = save_state(&state) {
                    log::error!("Failed to save state: {}", e);
                }
                
                // Update metrics
                *metrics.last_sync.write().await = Utc::now();
                metrics.is_healthy.store(true, Ordering::Relaxed);
                
                log::info!("Sync completed. Processed {} races", processed);
            }
            Err(e) => {
                log::error!("Failed to fetch pipelines: {}", e);
                metrics.errors.fetch_add(1, Ordering::Relaxed);
                metrics.is_healthy.store(false, Ordering::Relaxed);
            }
        }
        
        // Check if shutdown was requested
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        
        // Sleep with periodic checks for shutdown
        log::info!("Sleeping for {} seconds...", config.sync.interval_seconds);
        for _ in 0..config.sync.interval_seconds {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            sleep(Duration::from_secs(1)).await;
        }
    }
    
    log::info!("GitLab adapter shutting down gracefully");
    
    // Stop the health check server
    health_handle.abort();
    log::info!("Health check server stopped");
    
    // Save final state
    if let Err(e) = save_state(&state) {
        log::error!("Failed to save state: {}", e);
    } else {
        log::info!("State saved. Goodbye!");
    }
    
    // Give servers a moment to shut down
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    Ok(())
}