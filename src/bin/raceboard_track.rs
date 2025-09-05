use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RaceState {
    Queued,
    Running,
    Passed,
    Failed,
    Canceled,
}

#[derive(Debug, Serialize, Deserialize)]
struct Race {
    id: String,
    source: String,
    title: String,
    state: RaceState,
    started_at: String,
    eta_sec: Option<i64>,
    progress: Option<i32>,
    deeplink: Option<String>,
    metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct RaceUpdate {
    state: Option<RaceState>,
    progress: Option<i32>,
    eta_sec: Option<i64>,
    metadata: Option<HashMap<String, String>>,
}

#[derive(Parser, Debug)]
#[command(author, version, about = "Raceboard tracking CLI for AI agents", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,

    /// Raceboard server URL
    #[arg(
        short = 's',
        long,
        default_value = "http://localhost:7777",
        global = true
    )]
    server: String,

    /// Quiet mode (no output)
    #[arg(short = 'q', long, global = true)]
    quiet: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start tracking a new task
    Start {
        /// Description of the task
        #[arg(trailing_var_arg = true)]
        description: Vec<String>,

        /// Estimated time in seconds
        #[arg(short = 'e', long)]
        eta: Option<i64>,
    },

    /// Update progress on current task
    Progress {
        /// What you're currently doing
        #[arg(trailing_var_arg = true)]
        action: Vec<String>,

        /// Progress percentage (0-100)
        #[arg(short = 'p', long)]
        percent: Option<i32>,
    },

    /// Mark current task as complete
    Complete {
        /// Optional completion message
        #[arg(trailing_var_arg = true)]
        message: Vec<String>,
    },

    /// Mark current task as failed
    Error {
        /// Error description
        #[arg(trailing_var_arg = true)]
        message: Vec<String>,
    },

    /// Cancel current task
    Cancel,

    /// Show current active race
    Status,

    /// Clear all completed races
    Clear,
}

struct RaceTracker {
    client: Client,
    server_url: String,
    state_file: PathBuf,
    quiet: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct TrackerState {
    active_race_id: Option<String>,
    last_races: Vec<String>,
}

impl RaceTracker {
    fn new(server_url: String, quiet: bool) -> Self {
        let state_file = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("raceboard-track-state.json");

        Self {
            client: Client::new(),
            server_url,
            state_file,
            quiet,
        }
    }

    fn load_state(&self) -> TrackerState {
        if self.state_file.exists() {
            if let Ok(contents) = fs::read_to_string(&self.state_file) {
                if let Ok(state) = serde_json::from_str(&contents) {
                    return state;
                }
            }
        }
        TrackerState::default()
    }

    fn save_state(&self, state: &TrackerState) -> Result<()> {
        let json = serde_json::to_string_pretty(state)?;
        fs::write(&self.state_file, json)?;
        Ok(())
    }

    fn print(&self, message: &str) {
        if !self.quiet {
            println!("{}", message);
        }
    }

    async fn create_race(&self, race: Race) -> Result<Race> {
        let response = self
            .client
            .post(format!("{}/race", self.server_url))
            .json(&race)
            .send()
            .await
            .context("Failed to create race")?;

        let created_race = response
            .json::<Race>()
            .await
            .context("Failed to parse race response")?;

        Ok(created_race)
    }

    async fn update_race(&self, race_id: &str, update: RaceUpdate) -> Result<()> {
        self.client
            .patch(format!("{}/race/{}", self.server_url, race_id))
            .json(&update)
            .send()
            .await
            .context("Failed to update race")?;

        Ok(())
    }

    async fn handle_start(&self, description: Vec<String>, eta: Option<i64>) -> Result<()> {
        let title = if description.is_empty() {
            "AI Task".to_string()
        } else {
            description.join(" ")
        };

        let race = Race {
            id: Uuid::new_v4().to_string(),
            source: "ai-agent".to_string(),
            title: title.clone(),
            state: RaceState::Running,
            started_at: Utc::now().to_rfc3339(),
            eta_sec: eta.or(Some(60)),
            progress: Some(0),
            deeplink: None,
            metadata: Some({
                let mut m = HashMap::new();
                m.insert("triggered_by".to_string(), "agent".to_string());
                m.insert("cwd".to_string(), env::current_dir()?.display().to_string());
                if let Ok(user) = env::var("USER") {
                    m.insert("user".to_string(), user);
                }
                m
            }),
        };

        let created = self.create_race(race).await?;

        // Save state
        let mut state = self.load_state();
        state.active_race_id = Some(created.id.clone());
        state.last_races.insert(0, created.id.clone());
        state.last_races.truncate(10);
        self.save_state(&state)?;

        self.print(&format!("🏁 Started: {} [{}]", title, created.id));
        Ok(())
    }

    async fn handle_progress(&self, action: Vec<String>, percent: Option<i32>) -> Result<()> {
        let state = self.load_state();
        let race_id = state
            .active_race_id
            .ok_or_else(|| anyhow::anyhow!("No active race"))?;

        let action_text = action.join(" ");

        let update = RaceUpdate {
            state: None,
            progress: percent,
            eta_sec: None,
            metadata: if !action_text.is_empty() {
                Some({
                    let mut m = HashMap::new();
                    m.insert("last_action".to_string(), action_text.clone());
                    m.insert("updated_at".to_string(), Utc::now().to_rfc3339());
                    m
                })
            } else {
                None
            },
        };

        self.update_race(&race_id, update).await?;

        if !action_text.is_empty() {
            self.print(&format!("📊 Progress: {}", action_text));
        } else if let Some(p) = percent {
            self.print(&format!("📊 Progress: {}%", p));
        }

        Ok(())
    }

    async fn handle_complete(&self, message: Vec<String>) -> Result<()> {
        let mut state = self.load_state();
        let race_id = state
            .active_race_id
            .ok_or_else(|| anyhow::anyhow!("No active race"))?;

        let message_text = message.join(" ");

        let update = RaceUpdate {
            state: Some(RaceState::Passed),
            progress: Some(100),
            eta_sec: Some(0),
            metadata: Some({
                let mut m = HashMap::new();
                m.insert("completed_at".to_string(), Utc::now().to_rfc3339());
                if !message_text.is_empty() {
                    m.insert("completion_message".to_string(), message_text.clone());
                }
                m
            }),
        };

        self.update_race(&race_id, update).await?;

        // Clear active race
        state.active_race_id = None;
        self.save_state(&state)?;

        if !message_text.is_empty() {
            self.print(&format!("✅ Complete: {}", message_text));
        } else {
            self.print("✅ Task completed");
        }

        Ok(())
    }

    async fn handle_error(&self, message: Vec<String>) -> Result<()> {
        let mut state = self.load_state();
        let race_id = state
            .active_race_id
            .ok_or_else(|| anyhow::anyhow!("No active race"))?;

        let error_text = message.join(" ");

        let update = RaceUpdate {
            state: Some(RaceState::Failed),
            progress: None,
            eta_sec: Some(0),
            metadata: Some({
                let mut m = HashMap::new();
                m.insert("failed_at".to_string(), Utc::now().to_rfc3339());
                m.insert("error".to_string(), error_text.clone());
                m
            }),
        };

        self.update_race(&race_id, update).await?;

        // Clear active race
        state.active_race_id = None;
        self.save_state(&state)?;

        self.print(&format!("❌ Error: {}", error_text));
        Ok(())
    }

    async fn handle_cancel(&self) -> Result<()> {
        let mut state = self.load_state();
        let race_id = state
            .active_race_id
            .ok_or_else(|| anyhow::anyhow!("No active race"))?;

        let update = RaceUpdate {
            state: Some(RaceState::Canceled),
            progress: None,
            eta_sec: Some(0),
            metadata: Some({
                let mut m = HashMap::new();
                m.insert("canceled_at".to_string(), Utc::now().to_rfc3339());
                m
            }),
        };

        self.update_race(&race_id, update).await?;

        // Clear active race
        state.active_race_id = None;
        self.save_state(&state)?;

        self.print("🚫 Task canceled");
        Ok(())
    }

    fn handle_status(&self) -> Result<()> {
        let state = self.load_state();
        if let Some(race_id) = state.active_race_id {
            self.print(&format!("📍 Active race: {}", race_id));
        } else {
            self.print("📍 No active race");
        }

        if !state.last_races.is_empty() {
            self.print("\nRecent races:");
            for (i, race_id) in state.last_races.iter().take(5).enumerate() {
                self.print(&format!("  {}. {}", i + 1, race_id));
            }
        }

        Ok(())
    }

    fn handle_clear(&self) -> Result<()> {
        let state = TrackerState::default();
        self.save_state(&state)?;
        self.print("🧹 Cleared tracker state");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let tracker = RaceTracker::new(args.server.clone(), args.quiet);

    match args.command {
        Commands::Start { description, eta } => {
            tracker.handle_start(description, eta).await?;
        }
        Commands::Progress { action, percent } => {
            tracker.handle_progress(action, percent).await?;
        }
        Commands::Complete { message } => {
            tracker.handle_complete(message).await?;
        }
        Commands::Error { message } => {
            tracker.handle_error(message).await?;
        }
        Commands::Cancel => {
            tracker.handle_cancel().await?;
        }
        Commands::Status => {
            tracker.handle_status()?;
        }
        Commands::Clear => {
            tracker.handle_clear()?;
        }
    }

    Ok(())
}
