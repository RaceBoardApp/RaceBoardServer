#!/bin/bash

# Codex Session Wrapper - Tracks individual prompts/responses like Claude
# This wrapper uses the Python session monitor for interactive tracking

# Path to raceboard-codex binary
RACEBOARD_CODEX="${RACEBOARD_CODEX:-/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-codex}"

# Function that replaces the codex command
codex() {
    # Check if this looks like an interactive session
    # (no arguments, or just flags, no prompt provided)
    local is_interactive=false
    
    if [ $# -eq 0 ]; then
        is_interactive=true
    elif [[ "$*" =~ ^-[^\ ]*$ ]]; then
        is_interactive=true
    fi
    
    # Check if raceboard server is running
    if curl -s http://localhost:7777/health >/dev/null 2>&1; then
        if [ "$is_interactive" = true ] && [ -x "$RACEBOARD_CODEX" ]; then
            # Use Rust session monitor for interactive mode
            # This tracks each prompt/response as a separate race
            exec "$RACEBOARD_CODEX" session -- codex "$@"
        elif [ -x "$RACEBOARD_CODEX" ]; then
            # Use command wrapper for non-interactive mode
            # This tracks the entire command as one race
            "$RACEBOARD_CODEX" wrap -- codex "$@"
        else
            # Fallback to regular codex
            command codex "$@"
        fi
    else
        # Server not running, use regular codex
        command codex "$@"
    fi
}

# Export the function
export -f codex

# Aliases for convenience
alias cdx='codex'

# For explicit session tracking
codex-session() {
    if [ -x "$RACEBOARD_CODEX" ]; then
        "$RACEBOARD_CODEX" session -- codex "$@"
    else
        echo "raceboard-codex not found at: $RACEBOARD_CODEX" >&2
        command codex "$@"
    fi
}

export -f codex-session