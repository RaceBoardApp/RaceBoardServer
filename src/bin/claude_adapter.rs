use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};
use RaceboardServer::adapter_common::{RaceboardClient, Race, RaceState, RaceUpdate, Event, ServerConfig};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::io::{self};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(author, version, about = "Claude Code adapter for Raceboard", long_about = None)]
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
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start a race when Claude Code prompt is submitted
    Start {
        /// Title for the race
        #[arg(short, long)]
        title: Option<String>,

        /// Prompt text (if not provided, reads from stdin)
        #[arg(short, long)]
        prompt: Option<String>,

        /// Race ID (if updating existing race)
        #[arg(short = 'i', long)]
        race_id: Option<String>,

        /// Estimated time in seconds
        #[arg(short, long)]
        eta: Option<i64>,

        /// Additional metadata as key=value pairs
        #[arg(short = 'm', long, value_parser = parse_metadata)]
        metadata: Vec<(String, String)>,
    },

    /// Complete a race when Claude Code responds
    Complete {
        /// Race ID to complete
        race_id: String,

        /// Whether the response was successful
        #[arg(long, default_value = "true")]
        success: bool,

        /// Response text (if not provided, reads from stdin)
        #[arg(short, long)]
        response: Option<String>,

        /// Additional metadata as key=value pairs
        #[arg(short = 'm', long, value_parser = parse_metadata)]
        metadata: Vec<(String, String)>,
    },

    /// Update race progress
    Update {
        /// Race ID to update
        race_id: String,

        /// Progress percentage (0-100)
        #[arg(short, long)]
        progress: Option<i32>,

        /// Update state
        #[arg(short, long)]
        state: Option<String>,

        /// Additional metadata as key=value pairs
        #[arg(short = 'm', long, value_parser = parse_metadata)]
        metadata: Vec<(String, String)>,
    },

    /// Hook mode - handles both prompt and response in one process
    Hook {
        /// Disable progress tracking
        #[arg(long = "no-progress")]
        no_progress: bool,

        /// Progress update interval in seconds
        #[arg(short = 'i', long, default_value = "2")]
        interval: u64,
    },

    /// Install Claude Code hooks
    Install {
        /// Claude Code config directory
        #[arg(short = 'd', long, default_value = "~/.config/claude")]
        config_dir: String,
    },

    /// Background progress updater (internal use)
    UpdateProgress {
        /// Race ID to update
        #[arg(long)]
        race_id: String,

        /// Original ETA in seconds
        #[arg(long)]
        eta: i64,

        /// Update interval in seconds
        #[arg(long, default_value = "2")]
        interval: u64,
    },
}

fn parse_metadata(s: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid metadata format: {}", s));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

// Helper to build title from an optional explicit title and a prompt string.
// - If explicit title is provided, it's used as-is.
// - Otherwise, if the prompt starts with an 8-char alphanumeric session id like "ABC12345: ",
//   the prefix is stripped and the rest is trimmed.
// - Else, the trimmed prompt is returned.
pub(crate) fn derive_title_from_prompt(prompt_text: &str, explicit_title: Option<String>) -> String {
    if let Some(t) = explicit_title { return t; }
    if let Some(colon_pos) = prompt_text.find(": ") {
        let prefix = &prompt_text[..colon_pos];
        if prefix.len() == 8 && prefix.chars().all(|c| c.is_alphanumeric()) {
            return prompt_text[colon_pos + 2..].trim().to_string();
        }
    }
    prompt_text.trim().to_string()
}

struct ClaudeAdapter {
    client: RaceboardClient,
}

impl ClaudeAdapter {
    fn new(server_url: String) -> Result<Self> {
        let config = ServerConfig {
            url: server_url,
            timeout_seconds: 30,
            max_retries: 3,
        };
        let client = RaceboardClient::new(config)?;
        Ok(Self { client })
    }

    async fn create_race(&self, race: &Race) -> Result<Race> {
        self.client.create_race(race).await
    }

    async fn update_race(&self, race_id: &str, update: &RaceUpdate) -> Result<()> {
        self.client.update_race(race_id, update).await
    }

    async fn add_event(&self, race_id: &str, event: &Event) -> Result<()> {
        self.client.add_event(race_id, event).await
    }

    async fn handle_start(&self, args: Commands) -> Result<()> {
        if let Commands::Start {
            title,
            prompt,
            race_id,
            eta,
            metadata,
        } = args
        {
            let prompt_text = if let Some(p) = prompt {
                p
            } else {
                // Read from stdin
                let mut buffer = String::new();
                io::stdin().read_line(&mut buffer)?;
                buffer.trim().to_string()
            };

            // Clean up title - use provided title or cleaned prompt
            let race_title = derive_title_from_prompt(&prompt_text, title);

            // Build rich metadata for better clustering
            let mut race_metadata = HashMap::new();
            // Full prompt (truncate to 8KB for safety) and hash (over full text)
            let max_prompt = 8 * 1024;
            let prompt_trunc = if prompt_text.len() > max_prompt {
                prompt_text[..max_prompt].to_string()
            } else {
                prompt_text.clone()
            };
            let prompt_hash = blake3::hash(prompt_text.as_bytes()).to_hex().to_string();
            race_metadata.insert("prompt".to_string(), prompt_trunc);
            race_metadata.insert("prompt_hash".to_string(), prompt_hash);
            // Contextual metadata
            race_metadata.insert("editor".to_string(), "claude-code".to_string());
            race_metadata.insert(
                "model".to_string(),
                std::env::var("CLAUDE_MODEL").unwrap_or_else(|_| "claude".to_string()),
            );
            race_metadata.insert(
                "session_id".to_string(),
                metadata
                    .iter()
                    .find(|(k, _)| k == "session_id")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| "auto".to_string()),
            );
            // Simple task classification
            let lower = prompt_text.to_lowercase();
            let task = if lower.contains("refactor") {
                "refactor"
            } else if lower.contains("debug") || lower.contains("fix") {
                "debug"
            } else if lower.contains("explain") || lower.contains("review") {
                "analysis"
            } else {
                "code_generation"
            };
            race_metadata.insert("task_type".to_string(), task.to_string());
            // Estimated complexity bucket 1-5 based on length
            let est = match prompt_text.len() {
                0..=200 => 1,
                201..=600 => 2,
                601..=1200 => 3,
                1201..=2400 => 4,
                _ => 5,
            };
            race_metadata.insert("estimated_complexity".to_string(), est.to_string());
            // Apply user-supplied metadata (do not overwrite computed keys)
            for (key, value) in metadata {
                race_metadata.entry(key).or_insert(value);
            }

            let race = Race {
                id: race_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
                source: "claude-code".to_string(),
                title: race_title,
                state: RaceState::Running,
                started_at: Utc::now(),
                eta_sec: eta,
                progress: Some(0),
                deeplink: None,
                metadata: Some(race_metadata),
            };

            let created = self.create_race(&race).await?;

            // Output the race ID for the hook to capture
            println!("{}", created.id);
        }

        Ok(())
    }

    async fn handle_complete(&self, args: Commands) -> Result<()> {
        if let Commands::Complete {
            race_id,
            success,
            response,
            metadata,
        } = args
        {
            let response_text = if let Some(r) = response {
                r
            } else {
                // Read from stdin
                let mut buffer = String::new();
                io::stdin().read_line(&mut buffer)?;
                buffer.trim().to_string()
            };

            let mut update_metadata = HashMap::new();
            update_metadata.insert(
                "response_length".to_string(),
                response_text.len().to_string(),
            );
            update_metadata.insert("completed_at".to_string(), Utc::now().to_rfc3339());

            for (key, value) in metadata {
                update_metadata.insert(key, value);
            }

            // Add completion event (include response preview and length)
            let event = Event {
                event_type: "response".to_string(),
                timestamp: Utc::now(),
                data: Some(json!({
                    "success": success,
                    "response_preview": response_text.chars().take(200).collect::<String>(),
                    "response_length": response_text.len()
                })),
            };
            self.add_event(&race_id, &event).await?;

            // Update race state
            let update = RaceUpdate {
                state: Some(if success {
                    RaceState::Passed
                } else {
                    RaceState::Failed
                }),
                progress: Some(100),
                eta_sec: Some(0),
                deeplink: None,
                // Preserve original metadata set on creation; do not overwrite here
                metadata: None,
            };
            self.update_race(&race_id, &update).await?;
        }

        Ok(())
    }

    async fn handle_update(&self, args: Commands) -> Result<()> {
        if let Commands::Update {
            race_id,
            progress,
            state,
            metadata,
        } = args
        {
            let race_state = state.and_then(|s| match s.to_lowercase().as_str() {
                "queued" => Some(RaceState::Queued),
                "running" => Some(RaceState::Running),
                "passed" => Some(RaceState::Passed),
                "failed" => Some(RaceState::Failed),
                "canceled" => Some(RaceState::Canceled),
                _ => None,
            });

            let update_metadata = if metadata.is_empty() {
                None
            } else {
                Some(metadata.into_iter().collect())
            };

            let update = RaceUpdate {
                state: race_state,
                progress,
                eta_sec: None,
                deeplink: None,
                metadata: update_metadata,
            };
            self.update_race(&race_id, &update).await?;
        }

        Ok(())
    }

    async fn handle_hook(&self, progress_tracking: bool, interval: u64) -> Result<()> {
        use std::fs;
        use std::path::Path;
        use std::process::Command;

        // Check if there's already a race in progress
        let race_file = "/tmp/claude_race_current";
        if Path::new(race_file).exists() {
            // Check if the race is still valid (less than 5 minutes old)
            if let Ok(metadata) = fs::metadata(race_file) {
                if let Ok(modified) = metadata.modified() {
                    let age = std::time::SystemTime::now()
                        .duration_since(modified)
                        .unwrap_or(std::time::Duration::from_secs(0));

                    if age.as_secs() < 300 {
                        // Recent race exists, don't create a new one
                        if let Ok(existing_id) = fs::read_to_string(race_file) {
                            let existing_id = existing_id.trim();
                            eprintln!("⚠️  Race already in progress: {}", existing_id);
                            println!("{}", existing_id);
                            return Ok(());
                        }
                    }
                }
            }
        }

        // Read input from stdin
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        // Try to parse as JSON from Claude hook
        let (session_id, prompt_text, eta) =
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&input) {
                // Extract session_id and prompt from JSON
                let session_id = data["session_id"]
                    .as_str()
                    .map(|s| s.chars().take(8).collect::<String>())
                    .unwrap_or_else(|| "unknown".to_string());

                let prompt = data["prompt"].as_str().unwrap_or(&input).to_string();

                // Estimate ETA based on prompt complexity
                let eta = self.estimate_eta(&prompt);

                (session_id, prompt, eta)
            } else {
                // Fallback to plain text
                let prompt = input.trim().to_string();
                let eta = self.estimate_eta(&prompt);
                ("unknown".to_string(), prompt, eta)
            };

        // Clean up the prompt for display as title
        // Remove any duplicate session IDs or metadata that might be in the prompt
        let title = if let Some(colon_pos) = prompt_text.find(": ") {
            // Check if it starts with a session ID pattern (8 chars followed by colon)
            let prefix = &prompt_text[..colon_pos];
            if prefix.len() == 8 && prefix.chars().all(|c| c.is_alphanumeric()) {
                // Skip the session ID prefix
                prompt_text[colon_pos + 2..].trim().to_string()
            } else {
                prompt_text.trim().to_string()
            }
        } else {
            prompt_text.trim().to_string()
        };

        // Create race
        let race_id = Uuid::new_v4().to_string();
        let race = Race {
            id: race_id.clone(),
            source: "claude-code".to_string(),
            title,
            state: RaceState::Running,
            started_at: Utc::now(),
            eta_sec: None, // Let server predict ETA using clustering
            progress: Some(0),
            deeplink: None,
            metadata: Some({
                let mut m = HashMap::new();
                // Prompt and hash
                let max_prompt = 8 * 1024;
                let prompt_trunc = if prompt_text.len() > max_prompt {
                    prompt_text[..max_prompt].to_string()
                } else {
                    prompt_text.clone()
                };
                let prompt_hash = blake3::hash(prompt_text.as_bytes()).to_hex().to_string();
                m.insert("prompt".to_string(), prompt_trunc);
                m.insert("prompt_hash".to_string(), prompt_hash);
                // Context
                m.insert("editor".to_string(), "claude-code".to_string());
                m.insert(
                    "model".to_string(),
                    std::env::var("CLAUDE_MODEL").unwrap_or_else(|_| "claude".to_string()),
                );
                m.insert("session_id".to_string(), session_id);
                // Simple classification
                let lower = prompt_text.to_lowercase();
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
                let est = match prompt_text.len() {
                    0..=200 => 1,
                    201..=600 => 2,
                    601..=1200 => 3,
                    1201..=2400 => 4,
                    _ => 5,
                };
                m.insert("estimated_complexity".to_string(), est.to_string());
                m
            }),
        };

        let created = self.create_race(&race).await?;

        // Write race ID to temp file for response hook
        fs::write(race_file, &created.id)?;

        // Output race ID
        println!("{}", created.id);
        let server_eta = created.eta_sec.unwrap_or(eta);
        eprintln!(
            "🏁 Race started: {} (ETA: {}s from server prediction)",
            created.id, server_eta
        );

        // Start progress tracking in background if enabled
        if progress_tracking {
            // Spawn a background process to handle progress updates
            let update_interval = if interval > 0 { interval } else { (server_eta / 10).max(2).min(10) as u64 }; // Use provided interval or calculate one
                                                             // Use nohup to detach the process
            let binary_path = env::var("RACEBOARD_CLAUDE").unwrap_or_else(|_| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "raceboard-claude".to_string())
            });

            let cmd = format!(
                "nohup {} update-progress --race-id {} --eta {} --interval {} --server {} >/dev/null 2>&1 &",
                binary_path, created.id, server_eta, update_interval, "http://localhost:7777"
            );

            Command::new("sh").arg("-c").arg(&cmd).spawn()?;
        }

        Ok(())
    }

    fn estimate_eta(&self, prompt: &str) -> i64 {
        let length = prompt.len();
        let mut eta = 5; // Base time

        // Add time based on length
        eta += (length / 100) * 2;

        // Check for complexity indicators
        if prompt.contains("implement") || prompt.contains("create") || prompt.contains("write") {
            eta += 10;
        }
        if prompt.contains("analyze") || prompt.contains("debug") || prompt.contains("review") {
            eta += 8;
        }
        if prompt.contains("[Image") || prompt.contains("screenshot") {
            eta += 5;
        }

        // Cap at 60 seconds
        eta.min(60) as i64
    }

    fn install_hooks(&self, config_dir: String) -> Result<()> {
        let config_dir = shellexpand::tilde(&config_dir).to_string();
        let hooks_dir = format!("{}/hooks", config_dir);

        // Create hooks directory if it doesn't exist
        std::fs::create_dir_all(&hooks_dir)?;

        // Create prompt-submit hook script
        let prompt_hook_path = format!("{}/prompt-submit", hooks_dir);
        let prompt_hook_content = r#"#!/bin/bash
# Claude Code Raceboard adapter - prompt submission hook

# Get the prompt from stdin or args
if [ -t 0 ]; then
  PROMPT="$*"
else
  PROMPT=$(cat)
fi

RACE_ID=$(raceboard-claude start --prompt "$PROMPT" 2>/dev/null)
echo "$RACE_ID" > /tmp/claude_race_current
echo "🏁 Started race: $RACE_ID" >&2

# Pass through the prompt
echo "$PROMPT"
"#;

        std::fs::write(&prompt_hook_path, prompt_hook_content)?;

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&prompt_hook_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&prompt_hook_path, perms)?;
        }

        // Create response-received hook script
        let response_hook_path = format!("{}/response-received", hooks_dir);
        let response_hook_content = r#"#!/bin/bash
# Claude Code Raceboard adapter - response received hook

# Get the response from stdin or args
if [ -t 0 ]; then
  RESPONSE="$*"
else
  RESPONSE=$(cat)
fi

RACE_ID=$(cat /tmp/claude_race_current 2>/dev/null)
if [ -n "$RACE_ID" ]; then
  raceboard-claude complete "$RACE_ID" --response "$RESPONSE" 2>/dev/null
  echo "✅ Completed race: $RACE_ID" >&2
  rm -f /tmp/claude_race_current
fi

# Pass through the response
echo "$RESPONSE"
"#;

        std::fs::write(&response_hook_path, response_hook_content)?;

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&response_hook_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&response_hook_path, perms)?;
        }

        println!("✅ Claude Code hooks installed successfully!");
        println!("📁 Hook scripts created in: {}", hooks_dir);
        println!();
        println!("To use these hooks:");
        println!("1. Ensure raceboard-claude is in your PATH");
        println!("2. Start the Raceboard server: cargo run --bin raceboard-server");
        println!("3. Configure Claude Code to use these hooks");
        println!();
        println!("Hook files created:");
        println!("  - {}/prompt-submit", hooks_dir);
        println!("  - {}/response-received", hooks_dir);

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let adapter = ClaudeAdapter::new(args.server.clone())?;

    match args.command {
        Commands::Start { .. } => adapter.handle_start(args.command).await?,
        Commands::Complete { .. } => adapter.handle_complete(args.command).await?,
        Commands::Update { .. } => adapter.handle_update(args.command).await?,
        Commands::Hook {
            no_progress,
            interval,
        } => adapter.handle_hook(!no_progress, interval).await?,
        Commands::Install { config_dir } => adapter.install_hooks(config_dir)?,
        Commands::UpdateProgress {
            race_id,
            eta,
            interval,
        } => {
            // Background progress updater
            use std::path::Path;
            use std::time::Instant;

            let start_time = Instant::now();
            let eta_duration = Duration::from_secs(eta as u64);
            let race_file = "/tmp/claude_race_current";

            loop {
                // Check if race file still exists
                if !Path::new(race_file).exists() {
                    break;
                }

                // Check if file contains our race ID
                if let Ok(current_id) = std::fs::read_to_string(race_file) {
                    if current_id.trim() != race_id {
                        break; // Different race now
                    }
                }

                let elapsed = start_time.elapsed();
                let progress = if elapsed >= eta_duration {
                    95 // Stay at 95% if exceeded ETA
                } else {
                    ((elapsed.as_secs() as f64 / eta as f64) * 95.0) as i32
                }
                .min(95);

                // Update progress
                let update = RaceUpdate {
                    state: None,
                    progress: Some(progress),
                    eta_sec: None,
                    deeplink: None,
                    metadata: None,
                };
                let _ = adapter.update_race(&race_id, &update).await;

                // Sleep before next update
                sleep(Duration::from_secs(interval)).await;

                // Safety timeout after 5 minutes
                if elapsed.as_secs() > 300 {
                    break;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server};

    #[test]
    fn test_parse_metadata() {
        let (k, v) = parse_metadata("key=value").unwrap();
        assert_eq!(k, "key");
        assert_eq!(v, "value");
        assert!(parse_metadata("novalue").is_err());
        assert!(parse_metadata("=").is_ok());
    }

    #[test]
    fn test_derive_title_from_prompt() {
        // Uses explicit title when provided
        let t = derive_title_from_prompt("ABCDEFGH: Do the thing", Some("My Title".to_string()));
        assert_eq!(t, "My Title");
        // Strips 8-char session id prefix
        let t = derive_title_from_prompt("ABCDEFGH: Do the thing", None);
        assert_eq!(t, "Do the thing");
        // Keeps when prefix not 8 alnum
        let t = derive_title_from_prompt("ABCD: Not a session id", None);
        assert_eq!(t, "ABCD: Not a session id");
        // Trims spaces
        let t = derive_title_from_prompt("  hello world  ", None);
        assert_eq!(t, "hello world");
    }

    #[tokio::test]
    async fn test_create_race_posts_to_server() -> Result<()> {
        let mut server = Server::new();
        let expected_id = "r123";
        let _m = server
            .mock("POST", "/race")
            .match_header("content-type", Matcher::Regex("application/json".into()))
            .match_body(Matcher::PartialJson(serde_json::json!({
                "source": "claude-code",
                "title": "My Task",
                "state": "running"
            })))
            .with_status(200)
            .with_body(serde_json::json!({
                "id": expected_id,
                "source": "claude-code",
                "title": "My Task",
                "state": "running",
                "started_at": Utc::now(),
                "progress": 0
            }).to_string())
            .create();

        let adapter = ClaudeAdapter::new(server.url())?;
        let race = Race {
            id: Uuid::new_v4().to_string(),
            source: "claude-code".to_string(),
            title: "My Task".to_string(),
            state: RaceState::Running,
            started_at: Utc::now(),
            eta_sec: None,
            progress: Some(0),
            deeplink: None,
            metadata: Some(HashMap::new()),
        };

        let created = adapter.create_race(&race).await?;
        assert_eq!(created.id, expected_id);
        Ok(())
    }
}
