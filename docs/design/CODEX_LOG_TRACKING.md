# Codex Log-Based Tracking Solution

Related docs:
- Server Guide: `../guides/SERVER_GUIDE.md`
- Configuration & endpoints: `../guides/CONFIGURATION.md`, `api/openapi.yaml`
- Adapter page: `../adapters/CODEX_WATCH_ADAPTER.md`

## Discovery

Codex writes structured logs to `~/.codex/log/codex-tui.log` containing all function calls with JSON payloads. This gives us direct visibility into what Codex is actually doing!

## Log Format

```
[2m2025-08-28T16:54:32.010093Z[0m [32m INFO[0m FunctionCall: shell({"command":["bash","-lc","sed -n '1,200p' 'Raceboard UI/RaceRowView.swift'"]})
[2m2025-08-28T16:56:20.757461Z[0m [32m INFO[0m FunctionCall: update_plan({"plan":[{"status":"completed","step":"Survey repo structure"},{"status":"in_progress","step":"Review Swift sources"}]})
```

Key patterns:
- `FunctionCall: shell(...)` - Command execution
- `FunctionCall: update_plan(...)` - Task planning updates
- `FunctionCall: apply_patch(...)` - File modifications
- Timestamps in ISO format
- JSON payloads with structured data

## Design Summary (trimmed)

This page focuses on the signals and heuristics used for tracking, not full source listings.

Key signals parsed from the Codex log:
- Plan updates: start/complete steps → create/finish races.
- Function calls: activity bursts indicate progress; long idle implies wrapping up.
- Apply-patch events: increase progress and attach events.

Heuristics (high level):
- Prefer explicit plan progress when available; otherwise activity/time-based estimates.
- Cap optimistic progress below 100% (e.g., ≤95%) until a completion signal arrives.
- Detect inactivity >10s as “nearing completion” to avoid stalls.

Operational notes:
- Tails `~/.codex/log/codex-tui.log` using FS events with polling fallback.
- Starts from EOF by default; can process existing content on demand.
- Registers adapter health and reports periodically.

Implementation and usage details moved to: `../adapters/CODEX_WATCH_ADAPTER.md`.
                // Create a short-lived race for command execution
                let race = Race {
                    id: Uuid::new_v4().to_string(),
                    source: "codex-cmd".to_string(),
                    title,
                    state: RaceState::Running,
                    started_at: Utc::now().to_rfc3339(),
                    eta_sec: Some(5), // Most commands are quick
                    progress: Some(0),
                    deeplink: None,
                    metadata: Some({
                        let mut m = HashMap::new();
                        m.insert("command".to_string(), cmd_str);
                        m
                    }),
                };
                
                let created = create_race(race).await;
                
                // Auto-complete after 5 seconds
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    complete_race(&created.id).await;
                });
            }
        }
    }
    
    async fn handle_patch(&mut self, json_str: &str) {
        if let Ok(data) = serde_json::from_str::<Value>(json_str) {
            if let Some(patch_content) = data.as_str() {
                // Extract file being patched
                let file_re = Regex::new(r"\*\*\* (Add|Update|Delete) File: (.+)").unwrap();
                if let Some(caps) = file_re.captures(patch_content) {
                    let action = &caps[1];
                    let file_path = &caps[2];
                    
                    let race = Race {
                        id: Uuid::new_v4().to_string(),
                        source: "codex-patch".to_string(),
                        title: format!("{} {}", action, file_path),
                        state: RaceState::Running,
                        started_at: Utc::now().to_rfc3339(),
                        eta_sec: Some(3),
                        progress: Some(0),
                        deeplink: Some(format!("file://{}", file_path)),
                        metadata: None,
                    };
                    
                    let created = create_race(race).await;
                    println!("📝 Patching: {}", file_path);
                    
                    // Auto-complete quickly
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        complete_race(&created.id).await;
                    });
                }
            }
        }
    }
    
    fn extract_command_title(&self, cmd: &str) -> String {
        // Smart command title extraction
        if cmd.contains("xcodebuild") {
            "Building Xcode project".to_string()
        } else if cmd.contains("sed -n") && cmd.contains("p'") {
            if let Some(file) = cmd.split("'").nth(1) {
                format!("Reading {}", Path::new(file).file_name().unwrap_or_default().to_string_lossy())
            } else {
                "Reading file".to_string()
            }
        } else if cmd.contains("rg ") {
            "Searching codebase".to_string()
        } else if cmd.contains("ls ") {
            "Listing files".to_string()
        } else if cmd.contains("cat >") {
            "Creating file".to_string()
        } else if cmd.contains("apply_patch") {
            "Applying patch".to_string()
        } else {
            // Fallback: first meaningful word
            cmd.split_whitespace()
                .find(|w| !w.starts_with('-'))
                .unwrap_or("Command")
                .to_string()
        }
    }
}
```

## Binary: raceboard-codex-watch

```rust
// src/bin/raceboard_codex_watch.rs
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about = "Watch Codex logs for automatic race tracking")]
struct Args {
    /// Raceboard server URL
    #[arg(short, long, default_value = "http://localhost:7777")]
    server: String,
    
    /// Follow mode - keep watching for new entries
    #[arg(short = 'f', long)]
    follow: bool,
    
    /// Start from end of file (skip existing logs)
    #[arg(short = 'n', long)]
    new_only: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    let mut watcher = CodexLogWatcher::new(args.server);
    
    if args.new_only {
        watcher.skip_to_end();
    }
    
    if args.follow {
        println!("👀 Watching Codex logs for activity...");
        watcher.watch().await?;
    } else {
        println!("📖 Processing existing Codex logs...");
        watcher.process_existing().await?;
    }
    
    Ok(())
}
```

## Usage

### Start watching Codex logs
```bash
# Watch for new activity only
raceboard-codex-watch --follow --new-only

# Process existing logs and watch for new
raceboard-codex-watch --follow

# One-time processing of existing logs
raceboard-codex-watch
```

### Run Codex with Debug Logging
```bash
# Enable debug logs for session tracking
RUST_LOG=codex_core=debug,codex_tui=debug codex

# Or create an alias
alias codex-tracked='RUST_LOG=codex_core=debug,codex_tui=debug codex'
```

### Run alongside Codex
```bash
# Terminal 1: Start the watcher
raceboard-codex-watch -f -n

# Terminal 2: Use Codex with debug logging
RUST_LOG=codex_core=debug,codex_tui=debug codex "Help me implement authentication"
```

### Automated Setup Script
```bash
#!/bin/bash
# codex-with-tracking.sh

# Start the log watcher in background
raceboard-codex-watch -f -n &
WATCHER_PID=$!

# Run Codex with debug logging
RUST_LOG=codex_core=debug,codex_tui=debug codex "$@"

# Clean up watcher on exit
kill $WATCHER_PID 2>/dev/null
```

## Benefits

1. **Zero Configuration** - Works with any Codex session automatically
2. **Rich Context** - Extracts actual tasks from plan updates
3. **Command Tracking** - See what commands Codex runs
4. **File Tracking** - Track which files are being modified
5. **No Agent Cooperation Required** - Works with existing Codex versions
6. **Real-time** - Uses file watching for instant updates

## Progress Tracking

Progress estimation uses multiple signals:

1. **Plan-based** (most accurate): When `update_plan` shows `N completed of M total steps`
2. **Activity-based**: Count function calls as proxy (0-5 calls = 10%, 20+ = 75%, etc.)
3. **Idle detection**: No activity for 10+ seconds suggests nearing completion (90%)
4. **Prompt complexity**: Simple prompts get faster progress increments

Example implementation:
```rust
fn calculate_progress(&self) -> i32 {
    // If we have plan steps, use them
    if let Some(plan) = &self.plan_data {
        let completed = plan.steps.iter().filter(|s| s.status == "completed").count();
        return ((completed as f64 / plan.steps.len() as f64) * 100.0) as i32;
    }
    
    // Otherwise use function call heuristics
    match self.function_call_count {
        0..=5 => 10,
        6..=10 => 25,
        11..=20 => 50,
        21..=30 => 75,
        _ => 90,
    }
}
```

See `CODEX_PROGRESS_TRACKING.md` for detailed implementation.

## Implementation Considerations & Risks

While this log-based approach is robust, it's important to consider the following points during implementation:

1.  **Log Format Stability:** The primary risk is that the format of the `codex-tui.log` file could change in a future version of Codex. The parsing logic should be implemented with this in mind, including robust error handling for lines that don't match the expected patterns.

2.  **Shell Command Duration:** The proposal suggests auto-completing shell command races after a fixed duration (e.g., 5 seconds). This is a great heuristic for a first version, but it may be inaccurate for long-running commands. A future enhancement could be to have the log watcher invoke the dedicated `raceboard-cmd-runner` adapter for commands that are expected to take a long time, allowing for precise tracking of their completion.

3.  **Dynamic Plan Updates:** The logic for `handle_plan_update` should be designed to be resilient. Live plans can change (steps reordered, added, or removed). Using the `step_name` as a unique key in the `races` HashMap might be fragile if names are not unique or can change. A more robust identifier from the plan, if available, would be preferable.

## Example Output

When Codex is working:
```
👀 Watching Codex logs for activity...
🏁 Started: Survey repo structure [a3f4d5e6]
📖 Reading: RaceRowView.swift [b7c8d9e0]
📖 Reading: Readme.md [c1d2e3f4]
🔍 Searching codebase [d5e6f7a8]
✅ Completed: Survey repo structure
🏁 Started: Review Swift sources [e9f0a1b2]
📝 Patching: UI_PROPOSAL.md [f3g4h5i6]
✅ Completed: Review Swift sources
```

## Advanced Features

### Session Detection
Detect when a new Codex session starts by watching for patterns:
- Gap in timestamps > 5 minutes
- ERROR messages indicating restart
- First function call after long pause

### Intelligent Grouping
Group related operations:
- Multiple `sed` commands on same file = single "Analyzing file" race
- Sequential `rg` searches = single "Searching for X" race
- Plan steps as parent races with command sub-races

### Progress Estimation
Use historical data to estimate completion:
- Track average duration for command types
- Learn from plan step patterns
- Adjust ETA based on command complexity

## Installation

1. Add to Cargo.toml:
```toml
[[bin]]
name = "raceboard-codex-watch"
path = "src/bin/raceboard_codex_watch.rs"

[dependencies]
notify = "6"
```

2. Build and install:
```bash
cargo build --release --bin raceboard-codex-watch
cp target/release/raceboard-codex-watch /usr/local/bin/
```

3. Run automatically on login (optional):
```bash
# Add to ~/.zshrc or ~/.bashrc
if command -v raceboard-codex-watch >/dev/null 2>&1; then
    raceboard-codex-watch -f -n &>/dev/null &
fi
```

## Response Completion Detection

The key to knowing when Codex finishes responding is the **"Turn completed"** marker in DEBUG logs:

```
2025-08-28T18:28:57.272601Z DEBUG Turn completed
```

### Complete Session Lifecycle

1. **User Input**: `DEBUG Submission sub=Submission { id: "0", op: UserInput { items: [Text { text: "..." }] } }`
2. **Processing**: Various `FunctionCall` entries (shell, update_plan, etc.)
3. **Response Ready**: `DEBUG Output item item=Message { ... role: "assistant" ...}`
4. **Turn Complete**: `DEBUG Turn completed` ← This marks the end!

### Implementation for Completion Detection

```rust
async fn parse_log_line(&mut self, line: &str) {
    // Detect turn completion
    if line.contains("DEBUG Turn completed") {
        if let Some(active_race) = &self.current_session_race {
            self.complete_race(&active_race).await;
            println!("✅ Response completed");
        }
        self.current_session_race = None;
        return;
    }
    
    // Detect user input (start of new turn)
    if line.contains("DEBUG Submission") && line.contains("UserInput") {
        // Extract prompt from the submission
        let prompt = self.extract_user_input(line);
        
        let race = Race {
            id: Uuid::new_v4().to_string(),
            source: "codex-session".to_string(),
            title: format!("Codex: {}", truncate(&prompt, 50)),
            state: RaceState::Running,
            started_at: Utc::now().to_rfc3339(),
            eta_sec: Some(30),
            progress: Some(0),
            deeplink: None,
            metadata: Some({
                let mut m = HashMap::new();
                m.insert("prompt".to_string(), prompt);
                m
            }),
        };
        
        self.current_session_race = Some(self.create_race(race).await);
        println!("🏁 Started: {}", prompt);
    }
    
    // Track function calls during the turn
    if let Some(race) = &self.current_session_race {
        if line.contains("FunctionCall") {
            // Update progress based on activity
            self.update_race_progress(race, line).await;
        }
    }
}
```

## Conclusion

Log parsing with DEBUG level provides complete visibility:
- **User inputs** via `Submission` entries
- **Function calls** showing what Codex does
- **Turn completion** via `DEBUG Turn completed` marker
- Works with existing Codex installations
- Requires no modification to Codex or user workflow

Enable debug logs: `RUST_LOG=codex_core=debug,codex_tui=debug codex`

This is the definitive solution for Codex tracking!
