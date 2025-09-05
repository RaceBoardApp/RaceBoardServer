#!/bin/bash

# Claude Progress Updater
# Runs in background to update race progress while Claude is processing

RACE_ID="$1"
RACEBOARD_CMD="${2:-/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-claude}"

# Exit if no race ID provided
if [ -z "$RACE_ID" ]; then
    exit 1
fi

# Progress stages for Claude interactions
# Typical Claude response takes 5-30 seconds
progress_sequence=(20 35 50 65 75 85 90 95)
sleep_intervals=(1 2 2 3 3 4 5 10)

# Update progress periodically
for i in "${!progress_sequence[@]}"; do
    # Check if race is still active (file exists)
    if [ ! -f /tmp/claude_race_current ]; then
        exit 0
    fi
    
    # Check if current race matches
    current_race=$(cat /tmp/claude_race_current 2>/dev/null)
    if [ "$current_race" != "$RACE_ID" ]; then
        exit 0
    fi
    
    # Sleep before update
    sleep "${sleep_intervals[$i]}"
    
    # Update progress
    "$RACEBOARD_CMD" update "$RACE_ID" --progress "${progress_sequence[$i]}" 2>/dev/null
done