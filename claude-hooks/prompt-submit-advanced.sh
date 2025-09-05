#!/bin/bash

# Claude Code Raceboard Hook - Prompt Submit (Advanced with Progress)
# This version includes ETA estimation based on prompt complexity

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
    echo "$PROMPT"
    exit 0
fi

# Start Raceboard server if not running
if ! curl -s http://localhost:7777/health > /dev/null 2>&1; then
    echo "⚠️  Raceboard server not running. Start with: cargo run --bin raceboard-server" >&2
    echo "$PROMPT"
    exit 0
fi

# Extract session_id and actual prompt from JSON
if echo "$PROMPT" | grep -q '"session_id"'; then
    SESSION_ID=$(echo "$PROMPT" | grep -oE '"session_id":"[^"]*' | cut -d'"' -f4)
    ACTUAL_PROMPT=$(echo "$PROMPT" | grep -oE '"prompt":"[^"]*' | cut -d':' -f2- | sed 's/"$//')
    TITLE="Claude: ${SESSION_ID:0:8}"
else
    ACTUAL_PROMPT="$PROMPT"
    TITLE="Claude: $(echo "$PROMPT" | head -c 50 | tr '\n' ' ')"
fi

# Estimate ETA based on prompt complexity
estimate_eta() {
    local prompt="$1"
    local length=${#prompt}
    local base_time=5
    
    # Factors that increase time:
    # - Length of prompt (every 100 chars adds ~2 seconds)
    # - Code generation keywords
    # - Complex analysis keywords
    
    local time_estimate=$base_time
    time_estimate=$((time_estimate + (length / 100) * 2))
    
    # Check for complexity indicators
    if echo "$prompt" | grep -qiE 'implement|create|generate|write.*function|write.*class|build'; then
        time_estimate=$((time_estimate + 10))
    fi
    
    if echo "$prompt" | grep -qiE 'analyze|debug|fix|refactor|optimize|review'; then
        time_estimate=$((time_estimate + 8))
    fi
    
    if echo "$prompt" | grep -qiE '\[Image #[0-9]+\]|screenshot|image'; then
        time_estimate=$((time_estimate + 5))
    fi
    
    # Cap at reasonable maximum
    if [ $time_estimate -gt 60 ]; then
        time_estimate=60
    fi
    
    echo $time_estimate
}

ETA=$(estimate_eta "$ACTUAL_PROMPT")

# Create a race with ETA
RACE_OUTPUT=$("$RACEBOARD_CMD" start --title "$TITLE" --prompt "$PROMPT" --eta "$ETA" 2>&1)

# Extract race ID
RACE_ID=$(echo "$RACE_OUTPUT" | grep -oE '[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}' | head -1)

if [ -n "$RACE_ID" ]; then
    echo "$RACE_ID" > /tmp/claude_race_current
    echo "$ETA" > /tmp/claude_race_eta
    echo "$(date +%s)" > /tmp/claude_race_start_time
    echo "🏁 Race started: $RACE_ID (ETA: ${ETA}s)" >&2
    
    # Start smarter progress updater
    {
        start_time=$(date +%s)
        while [ -f /tmp/claude_race_current ]; do
            current_time=$(date +%s)
            elapsed=$((current_time - start_time))
            
            if [ $elapsed -lt $ETA ]; then
                # Calculate progress based on elapsed time
                progress=$((elapsed * 95 / ETA))
                if [ $progress -gt 95 ]; then
                    progress=95
                fi
                
                "$RACEBOARD_CMD" update "$RACE_ID" --progress "$progress" 2>/dev/null
                
                # Adaptive sleep - update more frequently as we approach completion
                if [ $progress -lt 30 ]; then
                    sleep 2
                elif [ $progress -lt 70 ]; then
                    sleep 1.5
                else
                    sleep 1
                fi
            else
                # Beyond ETA, stay at 95% until complete
                "$RACEBOARD_CMD" update "$RACE_ID" --progress 95 2>/dev/null
                sleep 2
            fi
            
            # Safety check - stop after 5 minutes
            if [ $elapsed -gt 300 ]; then
                break
            fi
        done
    } &
    
    echo $! > /tmp/claude_race_progress_pid
fi

# Pass through the prompt
echo "$PROMPT"