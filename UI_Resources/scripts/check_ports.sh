#!/bin/bash
# Port availability checker for UI

check_port() {
    local port=$1
    if lsof -Pi :$port -sTCP:LISTEN -t >/dev/null 2>&1; then
        echo "busy"
    else
        echo "free"
    fi
}

find_next_free_port() {
    local start_port=$1
    local port=$start_port
    
    while [ $port -lt 65535 ]; do
        if [ "$(check_port $port)" = "free" ]; then
            echo $port
            return 0
        fi
        port=$((port + 1))
    done
    
    echo "0"  # No free port found
    return 1
}

# Check default ports
HTTP_PORT=7777
GRPC_PORT=50051

if [ "$(check_port $HTTP_PORT)" = "busy" ]; then
    HTTP_PORT=$(find_next_free_port $HTTP_PORT)
fi

if [ "$(check_port $GRPC_PORT)" = "busy" ]; then
    GRPC_PORT=$(find_next_free_port $GRPC_PORT)
fi

# Output JSON for UI to parse
echo "{\"http_port\": $HTTP_PORT, \"grpc_port\": $GRPC_PORT}"