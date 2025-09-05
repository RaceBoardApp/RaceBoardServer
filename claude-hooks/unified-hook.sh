#!/bin/bash

# Unified Claude Code Raceboard Hook
# Handles both prompt submission and response completion

# Binary path
RACEBOARD_CMD="${RACEBOARD_CMD:-/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-claude}"

# Check what type of hook this is
HOOK_TYPE="${HOOK_TYPE:-prompt-submit}"

if [ "$HOOK_TYPE" = "prompt-submit" ] || [ "$1" = "prompt" ]; then
    # Handle prompt submission with progress tracking
    exec "$RACEBOARD_CMD" hook --progress true --interval 2
    
elif [ "$HOOK_TYPE" = "response-received" ] || [ "$1" = "response" ]; then
    # Handle response completion
    RACE_ID=$(cat /tmp/claude_race_current 2>/dev/null)
    if [ -n "$RACE_ID" ]; then
        # Read response from stdin
        RESPONSE=$(cat)
        RESPONSE_LENGTH=${#RESPONSE}
        
        # Complete the race
        "$RACEBOARD_CMD" complete "$RACE_ID" \
            --response "Response completed (${RESPONSE_LENGTH} chars)" 2>/dev/null
        
        echo "✅ Race completed: $RACE_ID" >&2
        rm -f /tmp/claude_race_current
        
        # Pass through the response
        echo "$RESPONSE"
    else
        # No race to complete, just pass through
        cat
    fi
else
    # Unknown hook type, pass through
    cat
fi