use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use env_logger;
use log::{debug, info, warn};
use RaceboardServer::adapter_common::{
    RaceboardClient, Race, RaceState, RaceUpdate, Event, ServerConfig,
    AdapterType, AdapterHealthMonitor
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

// Race, RaceState, RaceUpdate, and Event are now imported from adapter_common

#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "Gemini CLI telemetry adapter for Raceboard")]
struct Args {
    /// Raceboard server URL
    #[arg(short, long, default_value = "http://localhost:7777")]
    server: String,

    /// Path to Gemini telemetry outfile to tail
    #[arg(long)]
    telemetry_file: Option<PathBuf>,

    /// Provide an ETA hint (seconds)
    #[arg(long)]
    eta: Option<i64>,

    /// Start reading from beginning (default tails from end)
    #[arg(long)]
    from_start: bool,

    /// Disable posting per-event payloads (only lifecycle updates)
    #[arg(long)]
    no_events: bool,

    /// Verbose adapter logs
    #[arg(long)]
    debug: bool,

    /// Poll interval in milliseconds
    #[arg(long, default_value_t = 400)]
    poll_ms: u64,
}

fn init_logging(debug_on: bool) {
    let default_level = if debug_on { "debug" } else { "info" };
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_level));
    builder.format_timestamp_secs();
    let _ = builder.try_init();
}

#[derive(Debug)]
struct Session {
    race_id: String,
    session_id: String,
    prompt_id: Option<String>,
    start: Instant,
    tool_calls: u32,
    last_activity: Instant,
    waiting_for_prompt_id: bool,
    prompt_text: Option<String>,
}

impl Session {
    fn new(race_id: String, session_id: String, prompt_text: Option<String>) -> Self {
        let now = Instant::now();
        Self {
            race_id,
            session_id,
            prompt_id: None,
            start: now,
            tool_calls: 0,
            last_activity: now,
            waiting_for_prompt_id: true,
            prompt_text,
        }
    }
}

pub struct GeminiWatcher {
    client: RaceboardClient,
    telemetry_path: PathBuf,
    last_position: u64,
    current: Option<Session>,
    races_by_prompt_id: HashMap<String, String>, // prompt_id -> race_id mapping
    json_buffer: String,                         // Buffer for accumulating multi-line JSON
    no_events: bool,
    eta_hint: Option<i64>,
    debug: bool,
}

impl GeminiWatcher {
    pub fn new(
        server_config: ServerConfig,
        telemetry_path: PathBuf,
        no_events: bool,
        eta_hint: Option<i64>,
        debug: bool,
    ) -> Result<Self> {
        Ok(Self {
            client: RaceboardClient::new(server_config)?,
            telemetry_path,
            last_position: 0,
            current: None,
            races_by_prompt_id: HashMap::new(),
            json_buffer: String::new(),
            no_events,
            eta_hint,
            debug,
        })
    }

    pub fn seek_to_end(&mut self) -> Result<()> {
        if self.telemetry_path.exists() {
            let file = File::open(&self.telemetry_path)?;
            self.last_position = file.metadata()?.len();
            info!("Starting from end of telemetry file");
        }
        Ok(())
    }

    async fn create_race(
        &self,
        title: String,
        meta: HashMap<String, String>,
    ) -> Result<Race> {
        let race = Race {
            id: Uuid::new_v4().to_string(),
            source: "gemini-cli".to_string(),
            title,
            state: RaceState::Running,
            started_at: Utc::now(),
            eta_sec: self.eta_hint,
            progress: Some(0),
            deeplink: None,
            metadata: Some(meta),
        };

        debug!("Creating race: {}", race.id);
        self.client.create_race(&race).await.context("Failed to create race")
    }

    async fn update_progress(
        &self,
        race_id: &str,
        progress: i32,
        extra: Option<HashMap<String, String>>,
    ) -> Result<()> {
        let update = RaceUpdate {
            state: None,
            progress: Some(progress),
            eta_sec: None,
            metadata: extra,
            deeplink: None,
        };
        debug!("Updating race {} progress={}", race_id, progress);
        self.client.update_race(race_id, &update).await.context("Failed to update race progress")
    }

    async fn complete(
        &self,
        race_id: &str,
        success: bool,
        extra: Option<HashMap<String, String>>,
    ) -> Result<()> {
        let update = RaceUpdate {
            state: Some(if success {
                RaceState::Passed
            } else {
                RaceState::Failed
            }),
            progress: Some(100),
            eta_sec: Some(0),
            metadata: extra,
            deeplink: None,
        };
        debug!("Completing race {} success={}", race_id, success);
        self.client.update_race(race_id, &update).await.context("Failed to complete race")
    }

    async fn post_event(&self, race_id: &str, event_type: &str, data: Option<Value>) -> Result<()> {
        if self.no_events {
            return Ok(());
        }
        let evt = Event {
            event_type: event_type.to_string(),
            timestamp: Utc::now(),
            data,
        };
        debug!("Adding event to race {} type={}", race_id, event_type);
        self.client.add_event(race_id, &evt).await.context("Failed to post event")
    }

    pub async fn watch(&mut self, poll_ms: u64) -> Result<()> {
        if !self.telemetry_path.exists() {
            anyhow::bail!(
                "Telemetry file not found: {}",
                self.telemetry_path.display()
            );
        }

        info!(
            "👀 Tailing Gemini telemetry: {}",
            self.telemetry_path.display()
        );
        // Server info is logged by the RaceboardClient

        loop {
            self.read_new_lines().await?;
            tokio::time::sleep(Duration::from_millis(poll_ms)).await;
        }
    }

    async fn read_new_lines(&mut self) -> Result<()> {
        let file = File::open(&self.telemetry_path)?;
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(self.last_position))?;

        let mut bytes_read: u64 = 0;
        let mut line = String::new();
        let mut in_object = false;
        let mut brace_count = 0;

        loop {
            line.clear();
            let len = reader.read_line(&mut line)?;
            if len == 0 {
                break;
            }
            bytes_read += len as u64;

            // Count braces to track JSON object boundaries
            for ch in line.chars() {
                match ch {
                    '{' => {
                        brace_count += 1;
                        in_object = true;
                    }
                    '}' => {
                        brace_count -= 1;
                    }
                    _ => {}
                }
            }

            // Add line to buffer if we're in an object
            if in_object {
                self.json_buffer.push_str(&line);
            }

            // Check if we have a complete JSON object (brace count back to 0)
            if in_object && brace_count == 0 {
                // Try to parse the accumulated buffer
                if let Err(e) = self.handle_complete_json().await {
                    if self.debug {
                        eprintln!("Error handling JSON: {}", e);
                    }
                }
                // Clear the buffer for the next object
                self.json_buffer.clear();
                in_object = false;
            }
        }

        // Process any remaining complete JSON object in buffer when we reach EOF
        if in_object && brace_count == 0 && !self.json_buffer.is_empty() {
            if let Err(e) = self.handle_complete_json().await {
                if self.debug {
                    eprintln!("Error handling final JSON: {}", e);
                }
            }
            self.json_buffer.clear();
        }

        self.last_position += bytes_read;
        Ok(())
    }

    async fn handle_complete_json(&mut self) -> Result<()> {
        // Parse the complete JSON object
        let parsed: Result<Value, _> = serde_json::from_str(&self.json_buffer);
        match parsed {
            Ok(v) => {
                // The attributes field is at the top level
                if let Some(attributes) = v.get("attributes") {
                    // Debug log to see what we're getting
                    if self.debug {
                        if let Some(event_name) = attributes.get("event.name") {
                            debug!("Processing event: {}", event_name);
                        }
                    }
                    self.handle_telemetry_event(attributes).await
                } else {
                    // No attributes field, skip
                    Ok(())
                }
            }
            Err(e) => {
                if self.debug {
                    debug!("Failed to parse JSON object: {}", e);
                }
                Ok(())
            }
        }
    }

    async fn handle_telemetry_event(&mut self, attributes: &Value) -> Result<()> {
        let event_name = attributes
            .get("event.name")
            .and_then(|n| n.as_str())
            .unwrap_or("");

        // Log for debugging
        info!(
            "Event: {} | Has prompt_id: {}",
            event_name,
            attributes.get("prompt_id").is_some()
        );

        match event_name {
            "gemini_cli.user_prompt" => {
                // Start a new race for this prompt
                let session_id = attributes
                    .get("session.id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let prompt = attributes
                    .get("prompt")
                    .and_then(|p| p.as_str())
                    .map(|s| s.to_string());

                info!(
                    "Starting new race for prompt: {:?}",
                    prompt.as_ref().map(|p| truncate(p, 30))
                );
                self.start_turn_with_prompt(session_id, prompt).await?;
            }
            "gemini_cli.next_speaker_check" => {
                // Check if this event has a prompt_id and finish_reason
                if let Some(prompt_id) = attributes.get("prompt_id").and_then(|p| p.as_str()) {
                    // First, try to associate prompt_id if we haven't yet
                    if let Some(ref mut session) = self.current {
                        if session.waiting_for_prompt_id && session.prompt_id.is_none() {
                            session.prompt_id = Some(prompt_id.to_string());
                            session.waiting_for_prompt_id = false;
                            self.races_by_prompt_id
                                .insert(prompt_id.to_string(), session.race_id.clone());
                            info!(
                                "Associated prompt_id {} with race {}",
                                prompt_id, session.race_id
                            );
                        }
                    }

                    if let Some(finish_reason) =
                        attributes.get("finish_reason").and_then(|f| f.as_str())
                    {
                        info!(
                            "Next speaker check: prompt_id={}, finish_reason={}",
                            prompt_id, finish_reason
                        );

                        if finish_reason == "STOP" {
                            // Complete the race that matches this prompt_id
                            if let Some(ref session) = self.current {
                                if session.prompt_id.as_deref() == Some(prompt_id) {
                                    info!("Completing current race for prompt_id {}", prompt_id);
                                    self.finish(true, None).await?;
                                } else {
                                    info!("Current race has different prompt_id (current: {:?}, event: {})", session.prompt_id, prompt_id);
                                }
                            } else {
                                info!("No current race to complete for prompt_id {}", prompt_id);
                            }
                        }
                    }
                }
            }
            _ => {
                // For any other event with a prompt_id, associate it with the current race
                if let Some(prompt_id) = attributes.get("prompt_id").and_then(|p| p.as_str()) {
                    if let Some(ref mut session) = self.current {
                        if session.waiting_for_prompt_id && session.prompt_id.is_none() {
                            session.prompt_id = Some(prompt_id.to_string());
                            session.waiting_for_prompt_id = false;
                            self.races_by_prompt_id
                                .insert(prompt_id.to_string(), session.race_id.clone());
                            info!(
                                "Associated prompt_id {} with race {} via event {}",
                                prompt_id, session.race_id, event_name
                            );
                        }
                    }

                    // Handle progress events
                    match event_name {
                        "gemini_cli.tool_call" | "gemini_cli.tool_execution" => {
                            self.bump_tool().await?;
                        }
                        "gemini_cli.api_request" => {
                            self.bump_progress(30).await?;
                        }
                        "gemini_cli.api_response" => {
                            self.bump_progress(90).await?;
                        }
                        _ => {
                            // Post other events
                            if let Some(race_id) = self.races_by_prompt_id.get(prompt_id) {
                                self.post_event(race_id, event_name, Some(attributes.clone()))
                                    .await?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn start_turn(&mut self, title_from_prompt: Option<String>) -> Result<()> {
        if self.current.is_some() {
            return Ok(()); // Ignore if already in a turn
        }

        let title = format!(
            "Gemini: {}",
            title_from_prompt
                .as_deref()
                .map(|s| truncate(s, 50))
                .unwrap_or("turn".to_string())
        );

        let mut meta = HashMap::new();
        meta.insert(
            "cwd".to_string(),
            std::env::current_dir()
                .unwrap_or_default()
                .display()
                .to_string(),
        );
        if let Some(ref t) = self.telemetry_path.to_str() {
            meta.insert("telemetry_file".to_string(), t.to_string());
        }
        if let Some(ref src) = title_from_prompt {
            meta.insert("prompt_len".to_string(), src.len().to_string());
        }

        let created = self.create_race(title, meta).await?;
        self.current = Some(Session::new(
            created.id.clone(),
            "unknown".to_string(),
            title_from_prompt,
        ));

        println!("🏁 Started: {}", created.id);
        Ok(())
    }

    async fn start_turn_with_prompt(
        &mut self,
        session_id: String,
        prompt: Option<String>,
    ) -> Result<()> {
        // Each user_prompt is a NEW race - finish any existing race first
        if self.current.is_some() {
            info!("Finishing previous race before starting new one");
            self.finish(true, None).await?;
        }

        let title = format!(
            "Gemini: {}",
            prompt
                .as_deref()
                .map(|s| truncate(s, 50))
                .unwrap_or_else(|| "New prompt".to_string())
        );

        let mut meta = HashMap::new();
        meta.insert("session_id".to_string(), session_id.clone());
        meta.insert(
            "cwd".to_string(),
            std::env::current_dir()
                .unwrap_or_default()
                .display()
                .to_string(),
        );
        if let Some(ref t) = self.telemetry_path.to_str() {
            meta.insert("telemetry_file".to_string(), t.to_string());
        }
        if let Some(ref src) = prompt {
            meta.insert("prompt".to_string(), src.clone());
            meta.insert("prompt_len".to_string(), src.len().to_string());
        }

        let created = self.create_race(title, meta).await?;
        self.current = Some(Session::new(created.id.clone(), session_id, prompt));

        println!("🏁 Started: {} (waiting for prompt_id)", created.id);
        Ok(())
    }

    async fn bump_progress(&mut self, at_least: i32) -> Result<()> {
        if let Some(ref mut s) = self.current {
            s.last_activity = Instant::now();
            let progress = at_least.max(estimate_progress(
                s.start.elapsed(),
                s.tool_calls,
                self.eta_hint,
            ));
            let race_id = s.race_id.clone();
            let _ = s; // release borrow
            self.update_progress(&race_id, progress, None).await?;
        }
        Ok(())
    }

    async fn bump_tool(&mut self) -> Result<()> {
        if let Some(ref mut s) = self.current {
            s.tool_calls += 1;
            s.last_activity = Instant::now();
            let progress = (60 + (s.tool_calls as i32 * 3)).min(90);
            let race_id = s.race_id.clone();
            let _ = s; // release borrow
            self.update_progress(&race_id, progress, None).await?;
        }
        Ok(())
    }

    async fn finish(&mut self, success: bool, reason: Option<String>) -> Result<()> {
        if let Some(s) = self.current.take() {
            let mut meta = HashMap::new();
            meta.insert(
                "duration_ms".to_string(),
                s.start.elapsed().as_millis().to_string(),
            );
            meta.insert("tool_calls".to_string(), s.tool_calls.to_string());
            if let Some(prompt_id) = &s.prompt_id {
                meta.insert("prompt_id".to_string(), prompt_id.clone());
                // Clean up the mapping
                self.races_by_prompt_id.remove(prompt_id);
            }
            if let Some(r) = reason {
                meta.insert("reason".to_string(), r);
            }
            self.complete(&s.race_id, success, Some(meta)).await?;

            let id_display = s
                .prompt_id
                .as_deref()
                .unwrap_or(&s.race_id)
                .chars()
                .take(16)
                .collect::<String>();
            println!("✅ Completed: {}", id_display);
        }
        Ok(())
    }
}

fn estimate_progress(elapsed: Duration, tool_calls: u32, eta_hint: Option<i64>) -> i32 {
    let time_based = if let Some(eta) = eta_hint {
        ((elapsed.as_secs_f64() / eta as f64) * 95.0) as i32
    } else {
        0
    };
    let tool_based = match tool_calls {
        0..=1 => 10,
        2..=5 => 35,
        6..=10 => 60,
        11..=20 => 80,
        _ => 90,
    };
    time_based.max(tool_based).min(95)
}

fn truncate(s: &str, max: usize) -> String {
    // If the byte length is less than or equal to max, return as-is
    if s.len() <= max {
        return s.to_string();
    }

    // We need to truncate - find a valid UTF-8 boundary
    let mut byte_count = 0;
    let mut last_valid_boundary = 0;

    // Reserve 3 bytes for "..."
    let target = max.saturating_sub(3);

    for (idx, _char) in s.char_indices() {
        if idx <= target {
            last_valid_boundary = idx;
            byte_count = idx;
        } else {
            // We've exceeded the target, use the last valid boundary
            break;
        }
    }

    // If we couldn't fit any characters, just return "..."
    if last_valid_boundary == 0 && byte_count > target {
        return "...".to_string();
    }

    format!("{}...", &s[..last_valid_boundary])
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_logging(args.debug);

    info!("raceboard-gemini-watch starting");
    info!(
        "server={} telemetry_file={:?} from_start={} eta_hint={:?}",
        args.server, args.telemetry_file, args.from_start, args.eta
    );

    // Resolve telemetry path
    let telemetry_path = if let Some(p) = args.telemetry_file {
        p
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".gemini/telemetry.log")
    };

    let server_config = ServerConfig {
        url: args.server.clone(),
        ..Default::default()
    };
    
    // Create raceboard client for health monitor
    let raceboard_client = RaceboardClient::new(server_config.clone())
        .context("Failed to create raceboard client")?;
    
    // Create and register health monitor
    let instance_id = format!("gemini-watch-{}", std::process::id());
    let mut health_monitor = AdapterHealthMonitor::new(
        raceboard_client.clone(),
        AdapterType::GeminiWatch,
        instance_id.clone(),
        60, // Report health every 60 seconds
    ).await.context("Failed to create health monitor")?;
    
    info!("Registering gemini-watch adapter with Raceboard server...");
    health_monitor.register().await
        .context("Failed to register adapter with server")?;
    info!("Adapter registered successfully as: adapter:gemini-watch:{}", instance_id);
    
    // Start automatic health reporting
    let _health_report_handle = tokio::spawn(async move {
        health_monitor.clone().start_health_reporting().await;
    });
    
    let mut watcher = GeminiWatcher::new(
        server_config,
        telemetry_path,
        args.no_events,
        args.eta,
        args.debug,
    )?;
    
    // Basic health check (non-fatal)
    match watcher.client.health_check().await {
        Ok(true) => debug!("/health status: OK"),
        Ok(false) | Err(_) => warn!("Health check failed"),
    }
    if !args.from_start {
        watcher.seek_to_end()?;
    }
    watcher.watch(args.poll_ms).await?;

    Ok(())
}
