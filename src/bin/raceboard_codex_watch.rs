use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use futures::future::BoxFuture;
use log::{debug, error, info, warn};
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use regex::Regex;
use reqwest::Client;
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
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};
use uuid::Uuid;

// Race, RaceState, RaceUpdate, and Event are now imported from adapter_common

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Watch Codex logs for automatic race tracking"
)]
struct Args {
    /// Raceboard server URL
    #[arg(short, long, default_value = "http://localhost:7777")]
    server: String,

    /// Path to Codex log file (overrides default)
    #[arg(long)]
    log_path: Option<PathBuf>,

    // follow/new-only behavior is the default: we always tail and start at EOF
    /// Enable debug output
    #[arg(short = 'd', long)]
    debug: bool,

    /// Poll interval in milliseconds (in addition to FS events)
    #[arg(long, default_value_t = 500)]
    poll_ms: u64,

    /// Disable filesystem watcher; rely on polling only
    #[arg(long)]
    no_watcher: bool,

    /// Disable auto-starting races on FunctionCall lines; start only on prompt submissions
    #[arg(long, default_value_t = true)]
    only_submission_starts: bool,

    /// Minimum seconds a turn must run before we honor a completion signal
    #[arg(long, default_value_t = 2)]
    min_turn_secs: u64,
}

#[derive(Debug)]
#[allow(dead_code)]
struct PlanProgress {
    total_steps: usize,
    completed_steps: usize,
    #[allow(dead_code)]
    current_step: Option<String>,
}

impl PlanProgress {
    fn calculate_percentage(&self) -> i32 {
        if self.total_steps == 0 {
            return 0;
        }
        ((self.completed_steps as f64 / self.total_steps as f64) * 100.0) as i32
    }
}

#[derive(Debug)]
struct ActivityTracker {
    start_time: Instant,
    function_call_count: u32,
    last_activity: Instant,
    initial_eta: i64,
}

impl ActivityTracker {
    fn new(eta: i64) -> Self {
        let now = Instant::now();
        Self {
            start_time: now,
            function_call_count: 0,
            last_activity: now,
            initial_eta: eta,
        }
    }

    fn estimate_progress(&self) -> i32 {
        // Time-based estimation
        let elapsed = self.start_time.elapsed().as_secs();
        let time_progress = ((elapsed as f64 / self.initial_eta as f64) * 100.0).min(95.0);

        // Activity-based estimation
        let idle_time = self.last_activity.elapsed().as_secs();
        if idle_time > 10 {
            return 90; // Likely finishing up
        }

        // Function call count heuristic
        let call_progress = match self.function_call_count {
            0..=5 => 10,
            6..=10 => 25,
            11..=20 => 50,
            21..=30 => 75,
            31..=40 => 85,
            _ => 90,
        };

        // Return the higher of the two estimates
        time_progress.max(call_progress as f64) as i32
    }
}

#[derive(Debug)]
enum PromptComplexity {
    Simple,
    Moderate,
    Complex,
}

struct SessionTracker {
    plan_progress: Option<PlanProgress>,
    activity_tracker: ActivityTracker,
    #[allow(dead_code)]
    prompt_complexity: PromptComplexity,
    race_id: String,
    last_progress_bucket: Option<i32>,
}

impl SessionTracker {
    fn new(prompt: &str, race_id: String) -> Self {
        let complexity = Self::analyze_prompt(prompt);
        let eta = match complexity {
            PromptComplexity::Simple => 10,
            PromptComplexity::Moderate => 30,
            PromptComplexity::Complex => 120,
        };

        Self {
            plan_progress: None,
            activity_tracker: ActivityTracker::new(eta),
            prompt_complexity: complexity,
            race_id,
            last_progress_bucket: None,
        }
    }

    fn analyze_prompt(prompt: &str) -> PromptComplexity {
        let word_count = prompt.split_whitespace().count();
        let action_words = [
            "implement",
            "refactor",
            "design",
            "create",
            "build",
            "fix",
            "debug",
        ];
        let has_complex_action = action_words
            .iter()
            .any(|w| prompt.to_lowercase().contains(w));

        match (word_count, has_complex_action) {
            (0..=10, false) => PromptComplexity::Simple,
            (_, true) | (30.., _) => PromptComplexity::Complex,
            _ => PromptComplexity::Moderate,
        }
    }

    fn calculate_progress(&self) -> i32 {
        // Priority 1: Explicit plan progress
        if let Some(ref plan) = self.plan_progress {
            return plan.calculate_percentage();
        }

        // Priority 2: Activity-based estimation
        self.activity_tracker.estimate_progress()
    }

    #[allow(dead_code)]
    fn get_eta(&self) -> i64 {
        match self.prompt_complexity {
            PromptComplexity::Simple => 10,
            PromptComplexity::Moderate => 30,
            PromptComplexity::Complex => 120,
        }
    }
}

trait RaceboardApi: Send + Sync {
    fn create_race(&self, race: Race) -> BoxFuture<'static, Result<Race>>;
    fn update_race_progress(
        &self,
        race_id: String,
        progress: i32,
    ) -> BoxFuture<'static, Result<()>>;
    fn complete_race(&self, race_id: String) -> BoxFuture<'static, Result<()>>;
}

struct RealRaceboardApi {
    client: RaceboardClient,
}

impl RealRaceboardApi {
    fn new(server_config: ServerConfig) -> Result<Self> {
        Ok(Self {
            client: RaceboardClient::new(server_config)?,
        })
    }
}

impl RaceboardApi for RealRaceboardApi {
    fn create_race(&self, race: Race) -> BoxFuture<'static, Result<Race>> {
        let client = self.client.clone();
        Box::pin(async move {
            debug!("Creating race: {}", race.id);
            client.create_race(&race).await.context("Failed to create race")
        })
    }

    fn update_race_progress(
        &self,
        race_id: String,
        progress: i32,
    ) -> BoxFuture<'static, Result<()>> {
        let client = self.client.clone();
        Box::pin(async move {
            debug!("Updating race {} progress={}", race_id, progress);
            let update = RaceUpdate {
                state: None,
                progress: Some(progress),
                eta_sec: None,
                metadata: None,
                deeplink: None,
            };
            client.update_race(&race_id, &update).await.context("Failed to update race progress")
        })
    }

    fn complete_race(&self, race_id: String) -> BoxFuture<'static, Result<()>> {
        let client = self.client.clone();
        Box::pin(async move {
            debug!("Completing race: {}", race_id);
            let update = RaceUpdate {
                state: Some(RaceState::Passed),
                progress: Some(100),
                eta_sec: Some(0),
                metadata: None,
                deeplink: None,
            };
            client.update_race(&race_id, &update).await.context("Failed to complete race")
        })
    }
}

pub struct CodexLogWatcher {
    api: Box<dyn RaceboardApi>,
    log_path: PathBuf,
    last_position: u64,
    current_session: Option<SessionTracker>,
    last_completion: Option<Instant>,
    last_autostart: Option<(String, Instant)>,
    only_submission_starts: bool,
    min_turn_secs: u64,
    debug: bool,
}

impl CodexLogWatcher {
    pub fn new(
        server_config: ServerConfig,
        debug: bool,
        log_path_override: Option<PathBuf>,
        only_submission_starts: bool,
        min_turn_secs: u64,
    ) -> Result<Self> {
        let default_path = dirs::home_dir()
            .expect("Could not find home directory")
            .join(".codex/log/codex-tui.log");

        let log_path = log_path_override.unwrap_or(default_path);

        Ok(Self {
            api: Box::new(RealRaceboardApi::new(server_config)?),
            log_path,
            last_position: 0,
            current_session: None,
            last_completion: None,
            last_autostart: None,
            only_submission_starts,
            min_turn_secs,
            debug,
        })
    }

    pub fn skip_to_end(&mut self) -> Result<()> {
        if self.log_path.exists() {
            let file = File::open(&self.log_path)?;
            self.last_position = file.metadata()?.len();
            println!("📍 Skipping to end of log file");
        }
        Ok(())
    }

    pub async fn watch(&mut self, poll_ms: u64, no_watcher: bool) -> Result<()> {
        if !self.log_path.exists() {
            eprintln!("❌ Log file not found: {}", self.log_path.display());
            eprintln!("   Make sure Codex is installed and has been run at least once");
            eprintln!("   Enable debug logs: RUST_LOG=codex_core=debug,codex_tui=debug codex");
            debug!("Log file missing at path: {:?}", self.log_path);
            return Ok(());
        }

        // Process existing content first if not skipping
        if self.last_position == 0 {
            self.process_existing().await?;
        }

        println!("👀 Watching Codex logs for activity...");
        println!("   Log file: {}", self.log_path.display());
        // Server is logged in main; API is injected here
        debug!(
            "Watcher started with state: last_position={}, current_session_present={}",
            self.last_position,
            self.current_session.is_some()
        );

        // Set up file watcher (optional)
        let (tx, rx) = channel();
        let mut _watcher_opt: Option<RecommendedWatcher> = None;
        if !no_watcher {
            match Watcher::new(tx, Duration::from_millis(100)) {
                Ok(w) => {
                    let mut w: RecommendedWatcher = w;
                    if let Err(e) =
                        Watcher::watch(&mut w, &self.log_path, RecursiveMode::NonRecursive)
                    {
                        warn!("Failed to watch file: {} — falling back to polling only", e);
                    } else {
                        _watcher_opt = Some(w);
                    }
                }
                Err(e) => warn!("Watcher init failed: {} — using polling only", e),
            }
        }

        let timeout = Duration::from_millis(poll_ms);
        loop {
            // Try to receive FS events, but time out to poll
            match rx.recv_timeout(timeout) {
                Ok(DebouncedEvent::Write(path)) | Ok(DebouncedEvent::NoticeWrite(path)) => {
                    debug!("Filesystem event: write to {:?}", path);
                    self.process_new_lines().await?;
                }
                Ok(other) => {
                    debug!("Filesystem event ignored: {:?}", other);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Polling tick
                    self.process_new_lines().await?;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    warn!("Watcher channel disconnected; continuing with polling only");
                    // From now on, rely on polling
                    loop {
                        tokio::time::sleep(timeout).await;
                        self.process_new_lines().await?;
                    }
                }
            }

            // No idle-based completion; finish only on explicit marker in logs
        }

        // unreachable
        // Ok(())
    }

    pub async fn process_existing(&mut self) -> Result<()> {
        println!("📖 Processing existing log entries...");
        self.process_new_lines().await
    }

    async fn process_new_lines(&mut self) -> Result<()> {
        let mut file = File::open(&self.log_path)?;
        let file_len = file.metadata()?.len();
        if file_len < self.last_position {
            // File was truncated or rotated
            debug!(
                "File size shrank from {} to {}; assuming rotation/truncate and resetting position",
                self.last_position, file_len
            );
            self.last_position = 0;
            file.seek(SeekFrom::Start(0))?;
        } else {
            file.seek(SeekFrom::Start(self.last_position))?;
        }

        let reader = BufReader::new(file);
        for line in reader.lines() {
            if let Ok(line) = line {
                if let Err(e) = self.parse_log_line(&line).await {
                    if self.debug {
                        eprintln!("Error parsing line: {}", e);
                        // keep errors minimal; no line echo
                    }
                }
                // avoid per-line debug spam
            }
        }

        // Update position
        let file = File::open(&self.log_path)?;
        let end_pos = file.metadata()?.len();
        self.last_position = end_pos;

        Ok(())
    }

    async fn parse_log_line(&mut self, line: &str) -> Result<()> {
        // Detect turn completion (robust: ignore ANSI, case-insensitive)
        if is_turn_completed_line(line) {
            // Guard against early false positives by requiring minimal runtime
            if let Some(ref session) = self.current_session {
                let elapsed = session.activity_tracker.start_time.elapsed().as_secs();
                if elapsed < self.min_turn_secs {
                    debug!(
                        "Ignoring early completion signal ({}s < min {})",
                        elapsed, self.min_turn_secs
                    );
                    return Ok(());
                }
            }
            if let Some(session) = self.current_session.take() {
                self.complete_race(&session.race_id).await?;
                println!("✅ Response completed");
                debug!("Turn completed; race {} marked complete", session.race_id);
                self.last_completion = Some(Instant::now());
            }
            return Ok(());
        }

        // Detect user input (start of new turn) — robust match
        let plain = strip_ansi(line);
        if plain.contains("Submission")
            && (plain.contains("op: UserInput")
                || plain.contains("op=UserInput")
                || plain.contains("UserInput {"))
            && (plain.contains("text:") || plain.contains("Text {"))
        {
            // Avoid starting multiple races for the same turn
            if self.current_session.is_some() {
                return Ok(());
            }

            let prompt = self.extract_user_input(&plain);
            if prompt == "Unknown prompt" || prompt.trim().is_empty() {
                // Skip noisy submissions we can't parse
                return Ok(());
            }
            let title = format!("Codex: {}", self.truncate_title(&prompt, 50));
            debug!("Detected new submission; title='{}'", title);

            let race = Race {
                id: Uuid::new_v4().to_string(),
                source: "codex-session".to_string(),
                title: title.clone(),
                state: RaceState::Running,
                started_at: Utc::now(),
                eta_sec: None, // Will be updated based on complexity
                progress: Some(0),
                deeplink: None,
                metadata: Some({
                    let mut m = HashMap::new();
                    // Full prompt and hash (truncate prompt to 8KB for safety)
                    let max_prompt = 8 * 1024;
                    let prompt_trunc = if prompt.len() > max_prompt {
                        prompt[..max_prompt].to_string()
                    } else {
                        prompt.clone()
                    };
                    let hash = blake3::hash(prompt.as_bytes()).to_hex().to_string();
                    m.insert("prompt".to_string(), prompt_trunc);
                    m.insert("prompt_hash".to_string(), hash);
                    // Contextual metadata for clustering
                    m.insert("editor".to_string(), "codex".to_string());
                    m.insert("model".to_string(), "codex".to_string());
                    m.insert("session_id".to_string(), "auto".to_string());
                    let lower = prompt.to_lowercase();
                    let task = if lower.contains("refactor") {
                        "refactor"
                    } else if lower.contains("debug") || lower.contains("fix") {
                        "debug"
                    } else if lower.contains("explain") || lower.contains("review") {
                        "analysis"
                    } else {
                        "code_generation"
                    };
                    m.insert("task_type".to_string(), task.to_string());
                    let est = match prompt.len() {
                        0..=200 => 1,
                        201..=600 => 2,
                        601..=1200 => 3,
                        1201..=2400 => 4,
                        _ => 5,
                    };
                    m.insert("estimated_complexity".to_string(), est.to_string());
                    m.insert("log_file".to_string(), self.log_path.display().to_string());
                    m
                }),
            };

            let created = self.api.create_race(race).await?;
            self.current_session = Some(SessionTracker::new(&prompt, created.id.clone()));

            println!("🏁 Started: {}", title);
            debug!("Race created with id={}", created.id);
            return Ok(());
        }

        // Track function calls
        if strip_ansi(line).contains("FunctionCall:") {
            // Parse plan updates first (needs mutable self)
            if line.contains("update_plan") {
                debug!("update_plan detected in FunctionCall line");
                self.parse_plan_update(line)?;
            }

            // Then update session tracking
            if let Some(ref mut session) = self.current_session {
                session.activity_tracker.function_call_count += 1;
                session.activity_tracker.last_activity = Instant::now();

                let progress = session.calculate_progress();
                let race_id = session.race_id.clone();
                let call_count = session.activity_tracker.function_call_count;

                // Throttle progress prints by 10% buckets
                let bucket = (progress / 10).clamp(0, 10);
                let should_print = session
                    .plan_progress
                    .as_ref()
                    .map(|_| true) // if using explicit plan, allow prints
                    .unwrap_or_else(|| {
                        session
                            .last_progress_bucket
                            .map(|b| b != bucket)
                            .unwrap_or(true)
                    });

                session.last_progress_bucket = Some(bucket);

                // Update progress (release mutable borrow first)
                let _ = session; // end borrow
                self.api
                    .update_race_progress(race_id.clone(), progress)
                    .await?;

                if self.debug && should_print {
                    println!("   📊 Progress: {}% (calls: {})", progress, call_count);
                    debug!(
                        "Progress update sent: race_id={}, progress={} (calls={})",
                        race_id, progress, call_count
                    );
                }
            } else {
                // No active session; eagerly start one based on the function call
                // Debounce shortly after a completion to avoid double-starts on trailing logs
                if self.only_submission_starts {
                    return Ok(());
                }
                if let Some(t) = self.last_completion {
                    if t.elapsed() < Duration::from_secs(3) {
                        return Ok(());
                    }
                }
                let title = if line.contains("shell(") {
                    "shell command"
                } else if line.contains("apply_patch(") {
                    "editing files"
                } else if line.contains("update_plan(") {
                    "planning"
                } else {
                    "activity"
                };

                // Suppress duplicate auto-starts with same title within 10s
                if let Some((ref last_title, when)) = self.last_autostart {
                    if last_title == title && when.elapsed() < Duration::from_secs(10) {
                        debug!("Skipped duplicate autostart '{}' within 10s", title);
                        return Ok(());
                    }
                }

                let race = Race {
                    id: Uuid::new_v4().to_string(),
                    source: "codex-session".to_string(),
                    title: title.to_string(),
                    state: RaceState::Running,
                    started_at: Utc::now(),
                    eta_sec: None,
                    progress: Some(0),
                    deeplink: None,
                    metadata: Some({
                        let mut m = HashMap::new();
                        m.insert("trigger".to_string(), "function_call".to_string());
                        m.insert("log_file".to_string(), self.log_path.display().to_string());
                        m
                    }),
                };

                let created = self.api.create_race(race).await?;
                self.current_session = Some(SessionTracker::new("auto", created.id.clone()));
                println!("🏁 Started: {}", title);
                debug!(
                    "Auto-started session on FunctionCall; race id={}",
                    created.id
                );
                self.last_autostart = Some((title.to_string(), Instant::now()));
            }
        }

        Ok(())
    }

    // Removed idle-based completion; rely on explicit Codex completion marker

    fn extract_user_input(&self, line: &str) -> String {
        // Extract text from: UserInput { items: [Text { text: "..." }] }
        // Be forgiving about whitespace and formatting
        let re = Regex::new(r#"Text\s*\{\s*text:\s*\"([^\"]+)\"\s*\}"#).unwrap();
        if let Some(caps) = re.captures(line) {
            return caps
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        }
        // Fallback: text:"..."
        let re2 = Regex::new(r#"text:\s*\"([^\"]+)\""#).unwrap();
        if let Some(caps) = re2.captures(line) {
            return caps
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        }
        "Unknown prompt".to_string()
    }

    fn parse_plan_update(&mut self, line: &str) -> Result<()> {
        // Extract JSON from FunctionCall: update_plan({...})
        let re = Regex::new(r"update_plan\((.*)\)").unwrap();
        if let Some(caps) = re.captures(line) {
            let json_str = &caps[1];
            if let Ok(data) = serde_json::from_str::<Value>(json_str) {
                if let Some(plan) = data["plan"].as_array() {
                    let total = plan.len();
                    let completed = plan
                        .iter()
                        .filter(|step| step["status"].as_str() == Some("completed"))
                        .count();
                    let current = plan
                        .iter()
                        .find(|step| step["status"].as_str() == Some("in_progress"))
                        .and_then(|s| s["step"].as_str().map(String::from));

                    if let Some(ref mut session) = self.current_session {
                        session.plan_progress = Some(PlanProgress {
                            total_steps: total,
                            completed_steps: completed,
                            current_step: current.clone(),
                        });

                        if self.debug {
                            if let Some(step) = current {
                                println!("   📋 Plan: {}/{} - {}", completed, total, step);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn truncate_title(&self, text: &str, max_len: usize) -> String {
        if text.len() <= max_len {
            text.to_string()
        } else {
            format!("{}...", &text[..max_len - 3])
        }
    }

    async fn complete_race(&self, race_id: &str) -> Result<()> {
        self.api.complete_race(race_id.to_string()).await
    }
}

fn init_logging(debug_on: bool) {
    // If RUST_LOG is set, honor it; otherwise choose based on flag
    let default_level = if debug_on { "debug" } else { "info" };
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_level));
    // Compact log format
    builder.format_timestamp_secs();
    let _ = builder.try_init();
}

fn strip_ansi(s: &str) -> String {
    // Remove ANSI escape sequences commonly used for coloring logs
    let re = Regex::new("\u{1B}\\[[0-9;]*[A-Za-z]").unwrap();
    re.replace_all(s, "").to_string()
}

fn is_turn_completed_line(line: &str) -> bool {
    // Strictly match Codex log line indicating turn completion, e.g.:
    // 2025-09-02T07:20:27.875854Z DEBUG Turn completed
    let plain = strip_ansi(line);
    // Case-insensitive: require DEBUG level and exact phrase
    let re = Regex::new(r"(?i)\bdebug\b.*\bturn completed\b").unwrap();
    re.is_match(&plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    struct MockApiState {
        created: Vec<Race>,
        progress: Vec<(String, i32)>,
        completed: Vec<String>,
    }

    struct MockApi {
        state: Arc<Mutex<MockApiState>>,
    }

    impl MockApi {
        fn new() -> (Self, Arc<Mutex<MockApiState>>) {
            let state = Arc::new(Mutex::new(MockApiState::default()));
            (
                Self {
                    state: state.clone(),
                },
                state,
            )
        }
    }

    impl RaceboardApi for MockApi {
        fn create_race(&self, race: Race) -> BoxFuture<'static, Result<Race>> {
            let st = self.state.clone();
            Box::pin(async move {
                st.lock().unwrap().created.push(race.clone());
                Ok(race)
            })
        }

        fn update_race_progress(
            &self,
            race_id: String,
            progress: i32,
        ) -> BoxFuture<'static, Result<()>> {
            let st = self.state.clone();
            Box::pin(async move {
                st.lock().unwrap().progress.push((race_id, progress));
                Ok(())
            })
        }

        fn complete_race(&self, race_id: String) -> BoxFuture<'static, Result<()>> {
            let st = self.state.clone();
            Box::pin(async move {
                st.lock().unwrap().completed.push(race_id);
                Ok(())
            })
        }
    }

    // Helper to build a watcher with the mock API
    fn make_watcher_with_api(api: Box<dyn RaceboardApi>) -> CodexLogWatcher {
        CodexLogWatcher {
            api,
            log_path: PathBuf::from("/dev/null"),
            last_position: 0,
            current_session: None,
            last_completion: None,
            last_autostart: None,
            only_submission_starts: true,
            min_turn_secs: 0,
            debug: true,
        }
    }

    #[tokio::test]
    async fn test_replay_real_log_completes_on_turn_completed_only() -> Result<()> {
        // Load the provided real log file
        let path = PathBuf::from("codex.2025-08-29T18.log");
        assert!(path.exists(), "expected real log file at {:?}", path);

        let (mock, state) = MockApi::new();
        let mut watcher = make_watcher_with_api(Box::new(mock));

        let content = std::fs::read_to_string(&path)?;
        let mut completed_before_marker = false;

        for line in content.lines() {
            if is_turn_completed_line(line) {
                // Before feeding the completion line, ensure not completed yet
                let s = state.lock().unwrap();
                completed_before_marker = !s.completed.is_empty();
            }
            watcher.parse_log_line(line).await?;
        }

        let s = state.lock().unwrap();
        // One race created
        assert_eq!(s.created.len(), 1, "expected exactly one race created");
        // Completed exactly once, and not before the marker
        assert_eq!(s.completed.len(), 1, "expected exactly one completion");
        assert!(
            !completed_before_marker,
            "race completed before 'Turn completed' marker"
        );

        // Title should be derived from the first Submission text
        let title = &s.created[0].title;
        assert!(
            title.starts_with("Codex: "),
            "title should start with 'Codex: '"
        );

        // No auto-start from FunctionCall lines due to only_submission_starts=true
        // (already implied by created.len()==1)
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_logging(args.debug);

    info!("raceboard-codex-watch starting");
    info!(
        "server={} mode=follow+new_only log_path={}",
        args.server,
        args.log_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<default> ~/.codex/log/codex-tui.log".to_string())
    );

    // Create server config and test connection
    let server_config = ServerConfig {
        url: args.server.clone(),
        ..Default::default()
    };
    
    let raceboard_client = RaceboardClient::new(server_config.clone())
        .context("Failed to create raceboard client")?;
        
    match raceboard_client.health_check().await {
        Ok(true) => {
            println!("✅ Connected to Raceboard server");
            debug!("/health status: OK");
        }
        Ok(false) | Err(_) => {
            eprintln!(
                "⚠️  Warning: Cannot reach Raceboard server at {}",
                args.server
            );
            eprintln!("   Make sure the server is running: cargo run --bin raceboard-server");
            warn!("Health check failed");
        }
    }
    
    // Create and register health monitor
    let instance_id = format!("codex-watch-{}", std::process::id());
    let mut health_monitor = AdapterHealthMonitor::new(
        raceboard_client.clone(),
        AdapterType::CodexWatch,
        instance_id.clone(),
        60, // Report health every 60 seconds
    ).await.context("Failed to create health monitor")?;
    
    info!("Registering codex-watch adapter with Raceboard server...");
    health_monitor.register().await
        .context("Failed to register adapter with server")?;
    info!("Adapter registered successfully as: adapter:codex-watch:{}", instance_id);
    
    // Start automatic health reporting
    let _health_report_handle = tokio::spawn(async move {
        health_monitor.clone().start_health_reporting().await;
    });

    let mut watcher = CodexLogWatcher::new(
        server_config,
        args.debug,
        args.log_path,
        args.only_submission_starts,
        args.min_turn_secs,
    )?;

    // Always behave as: new_only + follow
    watcher.skip_to_end()?;
    watcher.watch(args.poll_ms, args.no_watcher).await?;

    Ok(())
}
