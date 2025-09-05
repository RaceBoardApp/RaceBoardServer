#!/bin/bash

# Codex CLI Raceboard Aliases
# Add to your ~/.bashrc or ~/.zshrc:
# source /path/to/codex-aliases.sh

# Path to raceboard-cmd
RACEBOARD_CMD="${RACEBOARD_CMD:-/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-cmd}"

# Alias for codex ai commands
alias cdx-ai='function _cdx_ai() { 
    "$RACEBOARD_CMD" -t "Codex AI: ${1:0:30}" -o -- codex ai "$@"
}; _cdx_ai'

# Alias for codex run commands  
alias cdx-run='function _cdx_run() {
    "$RACEBOARD_CMD" -t "Codex Run: ${1:0:30}" -o -- codex run "$@"
}; _cdx_run'

# Alias for codex chat
alias cdx-chat='function _cdx_chat() {
    "$RACEBOARD_CMD" -t "Codex Chat Session" -o -- codex chat "$@"
}; _cdx_chat'

# General codex tracking
alias cdx='function _cdx() {
    "$RACEBOARD_CMD" -t "Codex: $1" -o -- codex "$@"
}; _cdx'