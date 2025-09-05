#!/bin/bash

# Codex Session Monitor - Tracks individual prompts/responses as races
# This script intercepts Codex I/O to track each interaction

RACEBOARD_CMD="${RACEBOARD_CMD:-/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-codex}"
CURRENT_RACE_FILE="/tmp/codex_race_current_$$"
SESSION_LOG="/tmp/codex_session_$$"

# Cleanup on exit
trap "rm -f $CURRENT_RACE_FILE $SESSION_LOG" EXIT

# Function to start a race when prompt detected
start_race() {
    local prompt="$1"
    
    # Create race for this prompt
    local race_id=$("$RACEBOARD_CMD" start "prompt" "$prompt" --title "Codex: ${prompt:0:50}" 2>/dev/null)
    
    if [ -n "$race_id" ]; then
        echo "$race_id" > "$CURRENT_RACE_FILE"
        echo "🏁 Race started for prompt: ${prompt:0:50}" >&2
        
        # Start background progress updater
        (
            start_time=$(date +%s)
            while [ -f "$CURRENT_RACE_FILE" ]; do
                current_time=$(date +%s)
                elapsed=$((current_time - start_time))
                
                # Update progress based on elapsed time (assume 10 sec average)
                progress=$((elapsed * 10))
                if [ $progress -gt 95 ]; then
                    progress=95
                fi
                
                "$RACEBOARD_CMD" update "$(cat $CURRENT_RACE_FILE)" --progress $progress 2>/dev/null
                sleep 2
            done
        ) &
    fi
}

# Function to complete race when response detected
complete_race() {
    if [ -f "$CURRENT_RACE_FILE" ]; then
        local race_id=$(cat "$CURRENT_RACE_FILE")
        "$RACEBOARD_CMD" complete "$race_id" --exit-code 0 2>/dev/null
        echo "✅ Race completed: $race_id" >&2
        rm -f "$CURRENT_RACE_FILE"
    fi
}

# Monitor pattern variables
PROMPT_DETECTED=false
PROMPT_BUFFER=""
IN_RESPONSE=false

# Use script to capture all I/O
script -q "$SESSION_LOG" codex "$@" &
SCRIPT_PID=$!

# Monitor the session log for patterns
tail -f "$SESSION_LOG" 2>/dev/null | while IFS= read -r line; do
    # Echo the line to stdout for normal display
    echo "$line"
    
    # Detect user prompt (customize these patterns for Codex)
    # Common patterns: ">>> ", "codex> ", "? ", or after "Enter prompt:"
    if echo "$line" | grep -qE '^>>>|^codex>|^\?|Enter.*prompt:' ; then
        PROMPT_DETECTED=true
        PROMPT_BUFFER=""
    elif [ "$PROMPT_DETECTED" = true ]; then
        # Capture the prompt text
        if [ -n "$line" ] && ! echo "$line" | grep -qE '^\[|^Thinking|^Processing'; then
            PROMPT_BUFFER="$line"
            start_race "$PROMPT_BUFFER"
            PROMPT_DETECTED=false
            IN_RESPONSE=true
        fi
    fi
    
    # Detect response completion
    # Look for patterns like "Done", command prompt return, or double newline
    if [ "$IN_RESPONSE" = true ]; then
        if echo "$line" | grep -qE '^>>>|^codex>|^Done\.|^Completed|^\?|^$' ; then
            complete_race
            IN_RESPONSE=false
        fi
    fi
done &
MONITOR_PID=$!

# Wait for script to finish
wait $SCRIPT_PID

# Cleanup
kill $MONITOR_PID 2>/dev/null
complete_race  # Complete any pending race