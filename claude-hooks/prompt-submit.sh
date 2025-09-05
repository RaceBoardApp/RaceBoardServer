#!/bin/bash

# Claude Code Raceboard Hook - Prompt Submit
# This hook is called when a prompt is submitted to Claude Code

# Get the prompt from stdin or first argument
if [ -t 0 ]; then
    PROMPT="$*"
else
    PROMPT=$(cat)
fi

# Path to raceboard-claude binary
RACEBOARD_CMD="/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-claude"

# Check if raceboard-claude exists
if [ ! -x "$RACEBOARD_CMD" ]; then
    echo "⚠️  raceboard-claude not found. Build with: cargo build --bin raceboard-claude" >&2
    # Pass through the prompt
    echo "$PROMPT"
    exit 0
fi

# Start Raceboard server if not running
if ! curl -s http://localhost:7777/health > /dev/null 2>&1; then
    echo "⚠️  Raceboard server not running. Start with: cargo run --bin raceboard-server" >&2
    # Pass through the prompt
    echo "$PROMPT"
    exit 0
fi

# Extract session_id from JSON if present, otherwise use first 50 chars
if echo "$PROMPT" | grep -q '"session_id"'; then
    SESSION_ID=$(echo "$PROMPT" | grep -oE '"session_id":"[^"]*' | cut -d'"' -f4)
    if [ -n "$SESSION_ID" ]; then
        # Use shortened session ID (first 8 chars for readability)
        TITLE="Claude: ${SESSION_ID:0:8}"
    else
        TITLE="Claude: $(echo "$PROMPT" | head -c 50 | tr '\n' ' ')"
    fi
else
    TITLE="Claude: $(echo "$PROMPT" | head -c 50 | tr '\n' ' ')"
fi
RACE_OUTPUT=$("$RACEBOARD_CMD" start --title "$TITLE" --prompt "$PROMPT" 2>&1)

# Extract just the race ID (UUID format)
RACE_ID=$(echo "$RACE_OUTPUT" | grep -oE '[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}' | head -1)

if [ -n "$RACE_ID" ]; then
    echo "$RACE_ID" > /tmp/claude_race_current
    echo "🏁 Race started: $RACE_ID" >&2
    
    # Start background progress updater
    PROGRESS_SCRIPT="/Users/user/RustroverProjects/RaceboardServer/claude-hooks/progress-updater.sh"
    if [ -x "$PROGRESS_SCRIPT" ]; then
        nohup "$PROGRESS_SCRIPT" "$RACE_ID" "$RACEBOARD_CMD" >/dev/null 2>&1 &
        echo $! > /tmp/claude_race_progress_pid
    fi
fi

# Pass through the prompt
echo "$PROMPT"