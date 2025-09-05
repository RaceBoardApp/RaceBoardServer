use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::sleep;
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

#[derive(Debug, Serialize)]
struct Event {
    #[serde(rename = "type")]
    event_type: String,
    data: Option<serde_json::Value>,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Command to execute
    #[arg(required = true, last = true)]
    command: Vec<String>,

    /// Title for the race (defaults to command)
    #[arg(short, long)]
    title: Option<String>,

    /// Raceboard server URL
    #[arg(short = 's', long, default_value = "http://localhost:7777")]
    server: String,

    /// Estimated time in seconds
    #[arg(short, long)]
    eta: Option<i64>,

    /// Working directory
    #[arg(short = 'd', long)]
    working_dir: Option<String>,

    /// Show command output
    #[arg(short = 'o', long)]
    output: bool,

    /// Add metadata key=value pairs
    #[arg(short = 'm', long, value_parser = parse_metadata)]
    metadata: Vec<(String, String)>,

    /// Deep link URL
    #[arg(short = 'l', long)]
    deeplink: Option<String>,
}

fn parse_metadata(s: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid metadata format: {}", s));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

struct CommandRunner {
    client: Client,
    args: Args,
    race_id: String,
}

impl CommandRunner {
    fn new(args: Args) -> Self {
        Self {
            client: Client::new(),
            args,
            race_id: Uuid::new_v4().to_string(),
        }
    }

    async fn create_race(&self) -> Result<()> {
        let command_str = self.args.command.join(" ");
        let title = self
            .args
            .title
            .clone()
            .unwrap_or_else(|| command_str.clone());

        let mut metadata = HashMap::new();
        metadata.insert("command".to_string(), command_str.clone());

        if let Some(dir) = &self.args.working_dir {
            metadata.insert("working_dir".to_string(), dir.clone());
        }

        for (key, value) in &self.args.metadata {
            metadata.insert(key.clone(), value.clone());
        }

        let race = Race {
            id: self.race_id.clone(),
            source: "cmd".to_string(),
            title,
            state: RaceState::Queued,
            started_at: Utc::now().to_rfc3339(),
            eta_sec: self.args.eta,
            progress: Some(0),
            deeplink: self.args.deeplink.clone(),
            metadata: Some(metadata),
        };

        self.client
            .post(format!("{}/race", self.args.server))
            .json(&race)
            .send()
            .await
            .context("Failed to create race")?;

        Ok(())
    }

    async fn update_race(&self, update: RaceUpdate) -> Result<()> {
        self.client
            .patch(format!("{}/race/{}", self.args.server, self.race_id))
            .json(&update)
            .send()
            .await
            .context("Failed to update race")?;

        Ok(())
    }

    async fn add_event(&self, event_type: &str, data: serde_json::Value) -> Result<()> {
        let event = Event {
            event_type: event_type.to_string(),
            data: Some(data),
        };

        self.client
            .post(format!("{}/race/{}/event", self.args.server, self.race_id))
            .json(&event)
            .send()
            .await
            .context("Failed to add event")?;

        Ok(())
    }

    async fn run_command(&self) -> Result<i32> {
        let program = &self.args.command[0];
        let args = &self.args.command[1..];

        let mut cmd = Command::new(program);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(dir) = &self.args.working_dir {
            cmd.current_dir(dir);
        }

        // Update race to running
        self.update_race(RaceUpdate {
            state: Some(RaceState::Running),
            progress: Some(0),
            eta_sec: self.args.eta,
            metadata: None,
        })
        .await?;

        // Start the command
        let mut child = cmd.spawn().context("Failed to spawn command")?;

        let stdout = child.stdout.take().expect("Failed to capture stdout");
        let stderr = child.stderr.take().expect("Failed to capture stderr");

        // Create readers for stdout and stderr
        let stdout_reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);

        let start_time = Instant::now();

        // Spawn tasks to read stdout and stderr
        let output_enabled = self.args.output;
        let client = self.client.clone();
        let server = self.args.server.clone();
        let race_id = self.race_id.clone();

        let stdout_task = tokio::spawn(async move {
            let mut lines = stdout_reader.lines();
            let mut buffer = Vec::new();

            while let Ok(Some(line)) = lines.next_line().await {
                if output_enabled {
                    println!("{}", line);
                }
                buffer.push(line.clone());

                // Send batch of lines as event every 10 lines
                if buffer.len() >= 10 {
                    let event = Event {
                        event_type: "stdout".to_string(),
                        data: Some(json!({
                            "lines": buffer.clone()
                        })),
                    };

                    let _ = client
                        .post(format!("{}/race/{}/event", server, race_id))
                        .json(&event)
                        .send()
                        .await;

                    buffer.clear();
                }
            }

            // Send remaining lines
            if !buffer.is_empty() {
                let event = Event {
                    event_type: "stdout".to_string(),
                    data: Some(json!({
                        "lines": buffer
                    })),
                };

                let _ = client
                    .post(format!("{}/race/{}/event", server, race_id))
                    .json(&event)
                    .send()
                    .await;
            }
        });

        let stderr_task = tokio::spawn(async move {
            let mut lines = stderr_reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if output_enabled {
                    eprintln!("{}", line);
                }
            }
        });

        // Update progress periodically
        let progress_task = tokio::spawn({
            let client = self.client.clone();
            let server = self.args.server.clone();
            let race_id = self.race_id.clone();
            let eta_sec = self.args.eta;

            async move {
                let mut elapsed = 0;
                loop {
                    sleep(Duration::from_secs(2)).await;
                    elapsed += 2;

                    let progress = if let Some(eta) = eta_sec {
                        let progress = ((elapsed as f32 / eta as f32) * 100.0).min(99.0) as i32;
                        Some(progress)
                    } else {
                        None
                    };

                    let update = RaceUpdate {
                        state: None,
                        progress,
                        eta_sec: eta_sec.map(|e| (e - elapsed).max(1)),
                        metadata: None,
                    };

                    let _ = client
                        .patch(format!("{}/race/{}", server, race_id))
                        .json(&update)
                        .send()
                        .await;
                }
            }
        });

        // Wait for the command to complete
        let status = child.wait().await.context("Failed to wait for command")?;

        // Cancel the progress update task
        progress_task.abort();

        // Wait for output tasks to complete
        let _ = stdout_task.await;
        let _ = stderr_task.await;

        let exit_code = status.code().unwrap_or(-1);
        let duration = start_time.elapsed();

        // Send completion event
        self.add_event(
            "completed",
            json!({
                "exit_code": exit_code,
                "duration_sec": duration.as_secs(),
                "success": status.success()
            }),
        )
        .await?;

        // Update final state
        let final_state = if status.success() {
            RaceState::Passed
        } else {
            RaceState::Failed
        };

        self.update_race(RaceUpdate {
            state: Some(final_state),
            progress: Some(100),
            eta_sec: Some(0),
            metadata: Some({
                let mut meta = HashMap::new();
                meta.insert("exit_code".to_string(), exit_code.to_string());
                meta.insert("duration_sec".to_string(), duration.as_secs().to_string());
                meta
            }),
        })
        .await?;

        Ok(exit_code)
    }

    async fn run(&self) -> Result<i32> {
        // Create the race
        self.create_race().await?;

        println!("🏁 Race created: {}", self.race_id);
        println!(
            "📊 Track progress at: {}/race/{}",
            self.args.server, self.race_id
        );
        println!("🚀 Running command: {}", self.args.command.join(" "));

        if let Some(eta) = self.args.eta {
            println!("⏱️  Estimated time: {}s", eta);
        }

        println!("---");

        // Run the command
        match self.run_command().await {
            Ok(exit_code) => {
                if exit_code == 0 {
                    println!("---");
                    println!("✅ Command completed successfully!");
                } else {
                    println!("---");
                    println!("❌ Command failed with exit code: {}", exit_code);
                }
                Ok(exit_code)
            }
            Err(e) => {
                println!("---");
                println!("❌ Error: {}", e);

                // Update race to failed state
                let _ = self
                    .update_race(RaceUpdate {
                        state: Some(RaceState::Failed),
                        progress: Some(100),
                        eta_sec: Some(0),
                        metadata: Some({
                            let mut meta = HashMap::new();
                            meta.insert("error".to_string(), e.to_string());
                            meta
                        }),
                    })
                    .await;

                Err(e)
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("warn"));

    let args = Args::parse();
    let runner = CommandRunner::new(args);

    let exit_code = runner.run().await?;
    std::process::exit(exit_code);
}
