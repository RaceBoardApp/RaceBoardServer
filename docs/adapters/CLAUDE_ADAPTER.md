# Claude Code Adapter for Raceboard

The Claude Code Adapter integrates Claude Code with Raceboard, creating races for each Claude interaction to track response times and success rates.

## 🆕 Simplified Architecture

The adapter has been redesigned to move most functionality into the Rust binary:
- **Single binary** handles all operations (start, progress, complete)
- **Built-in progress tracking** with ETA estimation
- **Minimal shell scripts** - just thin wrappers
- **Automatic session detection** from Claude's JSON input
- **Background progress updates** managed by the binary

## 🎯 Features

- **Automatic Race Creation**: Creates a race when you submit a prompt to Claude
- **Response Tracking**: Marks the race as complete when Claude responds
- **Real-time Progress Updates**: Shows incremental progress during Claude's processing
- **ETA Estimation**: Estimates completion time based on prompt complexity (advanced version)
- **Metadata Collection**: Tracks prompt length, response time, workspace, and more
- **Hook Integration**: Seamless integration with Claude Code's hook system

## 📦 Installation

### Quick Install (Simplified)

The new simplified approach uses a single binary with built-in progress tracking:

```bash
# Build the binary
cargo build --bin raceboard-claude

# Copy simplified hooks
cp claude-hooks/unified-hook.sh ~/.config/claude/hooks/prompt-submit
cp claude-hooks/unified-hook.sh ~/.config/claude/hooks/response-received
chmod +x ~/.config/claude/hooks/*

# Set hook type in response hook
sed -i '' 's/HOOK_TYPE:-prompt-submit/HOOK_TYPE:-response-received/' ~/.config/claude/hooks/response-received
```

### Legacy Install

Run the installation script:
```bash
./claude-hooks/install.sh
```

### Manual Installation

1. **Build the adapter binary**:
```bash
cargo build --bin raceboard-claude
```

2. **Copy hook scripts**:
```bash
mkdir -p ~/.config/claude/hooks
cp claude-hooks/prompt-submit.sh ~/.config/claude/hooks/prompt-submit
cp claude-hooks/response-received.sh ~/.config/claude/hooks/response-received
chmod +x ~/.config/claude/hooks/*
```

3. **Configure Claude Code**:
Add to `~/.claude/settings.json` (note: NOT in `.config/claude`):
```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "~/.config/claude/hooks/prompt-submit"
          }
        ]
      }
    ],
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "~/.config/claude/hooks/response-received"
          }
        ]
      }
    ]
  }
}
```

## 🚀 Usage

### Starting the System

1. **Start Raceboard server**:
```bash
cargo run --bin raceboard-server
```

2. **Use Claude Code normally**:
- Submit prompts as usual
- Each interaction creates a race with progress tracking
- Progress updates every 1-5 seconds during processing
- View races at http://localhost:7777/races

### Progress Tracking

The simplified adapter includes built-in progress tracking:
- **Automatic ETA estimation** based on prompt complexity
- **Smooth progress updates** every 2 seconds
- **All logic in the binary** - no complex shell scripts needed

The binary estimates ETA based on:
- Prompt length (100 chars ≈ 2 seconds)
- Keywords like "implement", "create" (+10 seconds)
- Analysis tasks like "debug", "review" (+8 seconds)
- Image processing indicators (+5 seconds)
- Maximum ETA capped at 60 seconds

Progress smoothly increases from 0% to 95% based on elapsed time vs ETA, then stays at 95% until completion.

### Using the CLI Adapter

The `raceboard-claude` binary can also be used standalone:

#### Start a race manually:
```bash
raceboard-claude start --title "My Claude Task" --prompt "Write a function"
```

#### Complete a race:
```bash
raceboard-claude complete <race-id> --response "Function completed"
```

#### Update race progress:
```bash
raceboard-claude update <race-id> --progress 50
```

## 🪝 Hook System

### prompt-submit Hook
- **Triggered**: When you submit a prompt to Claude (UserPromptSubmit event)
- **Actions**:
  - Uses `raceboard-claude` binary to create a new race
  - Sets race state to "running"
  - Extracts and saves race ID (UUID) for tracking
  - Shows "🏁 Race started: [race-id]" in stderr

### response-received Hook  
- **Triggered**: When Claude completes a response (Stop event)
- **Actions**:
  - Uses `raceboard-claude` binary to complete the race
  - Marks race as "passed" with 100% progress
  - Shows "✅ Race completed: [race-id]" in stderr
  - Cleans up temporary race ID file

## 📊 Race Data Structure

Each Claude interaction creates a race with:

```json
{
  "id": "claude-<timestamp>-<pid>",
  "source": "claude-code",
  "title": "Claude: <first 50 chars of prompt>",
  "state": "running|passed|failed",
  "started_at": "2025-08-27T21:00:00Z",
  "progress": 0-100,
  "metadata": {
    "prompt_length": "123",
    "workspace": "/path/to/workspace",
    "user": "username",
    "response_length": "456"
  }
}
```

## 🔧 Configuration

### Environment Variables

- `CLAUDE_CONFIG_DIR`: Claude Code config directory (default: `~/.config/claude`)
- `RACEBOARD_SERVER`: Raceboard server URL (default: `http://localhost:7777`)

### Hook Customization

Edit the hook scripts in `~/.config/claude/hooks/` to customize:
- Race titles
- Metadata fields
- Progress tracking
- Error handling

## 📈 Monitoring

### View All Claude Races
```bash
curl http://localhost:7777/races | jq '.[] | select(.source == "claude-code")'
```

### Get Race Details
```bash
curl http://localhost:7777/race/<race-id> | jq .
```

### Stream Updates (via gRPC)
```bash
grpcurl -plaintext -import-path ./grpc -proto race.proto \
  localhost:50051 raceboard.RaceService/StreamRaces
```

## 🐛 Troubleshooting

### Hooks Not Triggering
1. Check Claude Code settings: `cat ~/.claude/settings.json` (NOT `~/.config/claude/settings.json`)
2. Verify hook scripts are executable: `ls -la ~/.config/claude/hooks/`
3. Ensure `raceboard-claude` binary exists: `ls -la target/debug/raceboard-claude`
4. Test hooks manually: `echo "test" | ~/.config/claude/hooks/prompt-submit`

### Server Connection Issues
1. Ensure server is running: `curl http://localhost:7777/health`
2. Check server logs for errors
3. Verify firewall settings

### Race Not Completing
1. Check `/tmp/claude_race_current` exists
2. Verify response-received hook is called
3. Check server for race status

## 🎨 Advanced Usage

### Custom Metadata
Modify hook scripts to add custom metadata:
```bash
\"custom_field\": \"value\",
\"model\": \"claude-3-opus\",
\"temperature\": \"0.7\"
```

### Integration with CI/CD
Track Claude Code usage in CI pipelines:
```yaml
- name: Run Claude Task
  run: |
    raceboard-claude start --title "CI: Generate Docs"
    # Run Claude task
    raceboard-claude complete $RACE_ID
```

### Analytics
Query races for analytics:
```sql
-- Example: Average response time
SELECT AVG(EXTRACT(EPOCH FROM (completed_at - started_at)))
FROM races
WHERE source = 'claude-code';
```

## 📝 Examples

### Example Race Lifecycle

1. **User submits prompt**: "Write a Python function to sort a list"
2. **prompt-submit hook**:
   - Creates race "Claude: Write a Python function to sort a list"
   - Sets progress to 10%
3. **Claude processes** (race shows as "running")
4. **Claude responds** with the function
5. **response-received hook**:
   - Adds response event
   - Sets progress to 100%
   - Marks as "passed"

### Example Integration Script
```bash
#!/bin/bash
# Track all Claude interactions for a session

# Start session race
SESSION_ID=$(raceboard-claude start --title "Dev Session" --eta 3600)

# Your Claude Code interactions happen here
# Each creates a child race automatically

# End session
raceboard-claude complete $SESSION_ID --metadata "total_prompts=5"
```

## 🔗 Related Documentation

- [Raceboard README](../Readme.md)
- [Command Runner Adapter](./CMD_RUNNER.md)
- [Implementation Plan](./IMPLEMENTATION_PLAN.md)