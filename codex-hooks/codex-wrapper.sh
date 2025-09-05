#!/bin/bash

# Codex CLI Raceboard Wrapper
# Add this to your shell profile: source /path/to/codex-wrapper.sh

# Path to raceboard-claude binary
RACEBOARD_CMD="${RACEBOARD_CMD:-/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-claude}"

# Original codex path
CODEX_BIN="${CODEX_BIN:-$(which codex)}"

codex() {
    # Check if raceboard is available
    if [ -x "$RACEBOARD_CMD" ] && curl -s http://localhost:7777/health > /dev/null 2>&1; then
        # Extract command for title
        local cmd_summary="$1"
        if [ "$1" = "ai" ] && [ -n "$2" ]; then
            # For 'codex ai "prompt"' commands
            cmd_summary="AI: ${2:0:30}"
        elif [ "$1" = "run" ] && [ -n "$2" ]; then
            # For 'codex run <command>' 
            cmd_summary="Run: ${2:0:30}"
        fi
        
        # Start race
        local race_id=$("$RACEBOARD_CMD" start --title "Codex: $cmd_summary" --prompt "$*" 2>&1 | grep -oE '[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}' | head -1)
        
        if [ -n "$race_id" ]; then
            echo "🏁 Race started: $race_id" >&2
        fi
        
        # Run actual codex command
        "$CODEX_BIN" "$@"
        local exit_code=$?
        
        # Complete race
        if [ -n "$race_id" ]; then
            if [ $exit_code -eq 0 ]; then
                "$RACEBOARD_CMD" complete "$race_id" --response "Command completed successfully" 2>/dev/null
                echo "✅ Race completed: $race_id" >&2
            else
                "$RACEBOARD_CMD" fail "$race_id" --response "Command failed with exit code $exit_code" 2>/dev/null
                echo "❌ Race failed: $race_id" >&2
            fi
        fi
        
        return $exit_code
    else
        # Fallback to regular codex
        "$CODEX_BIN" "$@"
    fi
}

# Export the function
export -f codex