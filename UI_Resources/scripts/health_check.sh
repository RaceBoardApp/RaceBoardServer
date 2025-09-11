#!/bin/bash
# Health check script for UI to monitor server and adapters

SERVER_URL="${1:-http://127.0.0.1:7777}"

# Check server health
check_server() {
    if curl -s -f "${SERVER_URL}/health" >/dev/null 2>&1; then
        echo "healthy"
    else
        echo "unhealthy"
    fi
}

# Check if process is running
check_process() {
    local process_name=$1
    if pgrep -f "$process_name" >/dev/null 2>&1; then
        echo "running"
    else
        echo "stopped"
    fi
}

# Output JSON status
cat << EOF
{
    "server": {
        "health": "$(check_server)",
        "process": "$(check_process raceboard-server)"
    },
    "adapters": {
        "gitlab": "$(check_process raceboard-gitlab)",
        "calendar": "$(check_process raceboard-calendar)",
        "codex": "$(check_process raceboard-codex-watch)",
        "claude": "$(check_process raceboard-claude)",
        "gemini": "$(check_process raceboard-gemini-watch)"
    }
}
EOF