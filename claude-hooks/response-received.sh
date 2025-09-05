#!/bin/bash

# Claude Code Raceboard Hook - Response Received
# This hook is called when Claude Code completes a response

# Get the response from stdin or first argument
if [ -t 0 ]; then
    RESPONSE="$*"
else
    RESPONSE=$(cat)
fi

# Path to raceboard-claude binary
RACEBOARD_CMD="/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-claude"

# Check if we have a current race
if [ -f /tmp/claude_race_current ]; then
    RACE_ID=$(cat /tmp/claude_race_current)
    
    # Stop progress updater if running
    if [ -f /tmp/claude_race_progress_pid ]; then
        kill $(cat /tmp/claude_race_progress_pid) 2>/dev/null
        rm -f /tmp/claude_race_progress_pid
    fi
    
    # Check if raceboard-claude exists and server is running
    if [ -x "$RACEBOARD_CMD" ] && curl -s http://localhost:7777/health > /dev/null 2>&1; then
        # Complete the race using raceboard-claude
        RESPONSE_LENGTH=$(echo -n "$RESPONSE" | wc -c | tr -d ' ')
        "$RACEBOARD_CMD" complete "$RACE_ID" --response "Response received (${RESPONSE_LENGTH} chars)" 2>/dev/null
        
        echo "✅ Race completed: $RACE_ID" >&2
        rm -f /tmp/claude_race_current
    fi
fi

# Pass through the response
echo "$RESPONSE"