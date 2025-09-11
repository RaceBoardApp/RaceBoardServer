use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use serde_json::json;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::sleep;
use uuid::Uuid;

// Use shared library for common functionality
use RaceboardServer::adapter_common::{
    Event, Race, RaceState, RaceUpdate, RaceboardClient, ServerConfig,
};

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
    client: RaceboardClient,
    args: Args,
    race_id: String,
}

impl CommandRunner {
    fn new(args: Args) -> Result<Self> {
        let config = ServerConfig {
            url: args.server.clone(),
            timeout_seconds: 30,
            max_retries: 3,
        };
        
        let client = RaceboardClient::new(config)?;
        let race_id = Uuid::new_v4().to_string();
        
        Ok(Self {
            client,
            args,
            race_id,
        })
    }

    async fn create_race(&self) -> Result<()> {
        let command_str = self.args.command.join(" ");
        let title = self
            .args
            .title
            .clone()
            .unwrap_or_else(|| command_str.clone());

        let mut metadata = HashMap::new();
        metadata.insert("command".to_string(), command_str);
        
        if let Some(ref dir) = self.args.working_dir {
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
            started_at: Utc::now(),
            eta_sec: self.args.eta,
            progress: Some(0),
            deeplink: self.args.deeplink.clone(),
            metadata: Some(metadata),
        };

        self.client.create_race(&race).await?;
        println!("🏁 Race started: {}/race/{}", self.args.server, self.race_id);
        
        Ok(())
    }

    async fn update_race(&self, update: RaceUpdate) -> Result<()> {
        self.client.update_race(&self.race_id, &update).await
    }

    async fn add_event(&self, event_type: &str, data: serde_json::Value) -> Result<()> {
        let event = Event {
            event_type: event_type.to_string(),
            timestamp: Utc::now(),
            data: Some(data),
        };
        
        self.client.add_event(&self.race_id, &event).await
    }

    async fn run_command(&self) -> Result<i32> {
        let mut cmd = Command::new(&self.args.command[0]);
        
        if self.args.command.len() > 1 {
            cmd.args(&self.args.command[1..]);
        }
        
        if let Some(ref dir) = self.args.working_dir {
            cmd.current_dir(dir);
        }
        
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let start = Instant::now();
        let mut child = cmd.spawn().context("Failed to spawn command")?;

        // Update race to running
        let _ = self.update_race(RaceUpdate {
            state: Some(RaceState::Running),
            progress: Some(0),
            eta_sec: self.args.eta,
            deeplink: None,
            metadata: None,
        }).await;

        // Spawn task to capture stdout
        let stdout = child.stdout.take().unwrap();
        let stdout_reader = BufReader::new(stdout);
        let race_id = self.race_id.clone();
        let show_output = self.args.output;
        let client = self.client.clone();
        
        let stdout_task = tokio::spawn(async move {
            let mut lines = stdout_reader.lines();
            let mut buffer = Vec::new();
            
            while let Ok(Some(line)) = lines.next_line().await {
                if show_output {
                    println!("{}", line);
                }
                buffer.push(line);
                
                // Send batch of lines as event
                if buffer.len() >= 10 {
                    let _ = client.add_event(
                        &race_id,
                        &Event {
                            event_type: "stdout".to_string(),
                            timestamp: Utc::now(),
                            data: Some(json!({ "lines": buffer.clone() })),
                        }
                    ).await;
                    buffer.clear();
                }
            }
            
            // Send remaining lines
            if !buffer.is_empty() {
                let _ = client.add_event(
                    &race_id,
                    &Event {
                        event_type: "stdout".to_string(),
                        timestamp: Utc::now(),
                        data: Some(json!({ "lines": buffer })),
                    }
                ).await;
            }
        });

        // Spawn task to capture stderr
        let stderr = child.stderr.take().unwrap();
        let stderr_reader = BufReader::new(stderr);
        let show_output = self.args.output;
        
        let stderr_task = tokio::spawn(async move {
            let mut lines = stderr_reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if show_output {
                    eprintln!("{}", line);
                }
            }
        });

        // Spawn task for progress updates
        let eta_sec = self.args.eta;
        let race_id_clone = self.race_id.clone();
        let client_clone = self.client.clone();
        
        let progress_task = tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(2)).await;
                
                let elapsed = start.elapsed().as_secs() as i64;
                let progress = if let Some(eta) = eta_sec {
                    if eta > 0 {
                        let pct = ((elapsed as f64 / eta as f64) * 100.0).min(99.0) as i32;
                        Some(pct)
                    } else {
                        None
                    }
                } else {
                    None
                };
                
                let remaining = eta_sec.map(|e| (e - elapsed).max(0));
                
                let _ = client_clone.update_race(
                    &race_id_clone,
                    &RaceUpdate {
                        state: None,
                        progress,
                        eta_sec: remaining,
                        deeplink: None,
                        metadata: None,
                    }
                ).await;
            }
        });

        // Wait for command to complete
        let status = child.wait().await.context("Failed to wait for command")?;
        
        // Cancel progress task
        progress_task.abort();
        
        // Wait for output tasks to complete
        let _ = stdout_task.await;
        let _ = stderr_task.await;

        let duration = start.elapsed();
        let exit_code = status.code().unwrap_or(-1);
        
        // Final update
        let final_state = if status.success() {
            RaceState::Passed
        } else {
            RaceState::Failed
        };

        let mut final_metadata = HashMap::new();
        final_metadata.insert("exit_code".to_string(), exit_code.to_string());
        final_metadata.insert("duration_sec".to_string(), duration.as_secs().to_string());
        
        let _ = self.update_race(RaceUpdate {
            state: Some(final_state.clone()),
            progress: Some(100),
            eta_sec: Some(0),
            deeplink: None,
            metadata: Some(final_metadata),
        }).await;

        // Add completion event
        let _ = self.add_event(
            "completed",
            json!({
                "exit_code": exit_code,
                "duration_sec": duration.as_secs(),
                "success": status.success()
            })
        ).await;

        match final_state {
            RaceState::Passed => println!("✅ Command completed successfully"),
            RaceState::Failed => println!("❌ Command failed with exit code {}", exit_code),
            _ => {}
        }

        Ok(exit_code)
    }

    async fn run(mut self) -> Result<i32> {
        self.create_race().await?;
        
        match self.run_command().await {
            Ok(code) => Ok(code),
            Err(e) => {
                eprintln!("❌ Error running command: {}", e);
                
                // Try to update race to failed
                let _ = self.update_race(RaceUpdate {
                    state: Some(RaceState::Failed),
                    progress: Some(100),
                    eta_sec: Some(0),
                    deeplink: None,
                    metadata: Some({
                        let mut m = HashMap::new();
                        m.insert("error".to_string(), e.to_string());
                        m
                    }),
                }).await;
                
                Ok(1)
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let runner = CommandRunner::new(args)?;
    let exit_code = runner.run().await?;
    std::process::exit(exit_code);
}