#!/bin/bash

# Simple Codex wrapper using raceboard-codex binary
# Add to your shell RC file: source /path/to/codex-wrapper-simple.sh

# Path to the raceboard-codex binary
RACEBOARD_CODEX="${RACEBOARD_CODEX:-/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-codex}"

# Define the wrapper function
codex() {
    # Check if raceboard-codex exists and server is running
    if [ -x "$RACEBOARD_CODEX" ] && curl -s http://localhost:7777/health >/dev/null 2>&1; then
        # Use the binary to wrap and track the command
        "$RACEBOARD_CODEX" wrap -- "$@"
    else
        # Fallback to regular codex
        command codex "$@"
    fi
}

# Export the function for use in subshells
export -f codex

# Optional aliases for common commands
alias cdx='codex'

# Since codex doesn't have an 'ai' subcommand, these are helper aliases
alias cdx-ai='codex'  # Direct prompt to codex
alias cdx-run='codex run'
alias cdx-chat='codex chat'

# Helper function for common use case
codex-ai() {
    codex "$@"
}