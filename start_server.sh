#!/bin/bash

# Kill any existing server instances
echo "Stopping any existing server instances..."
pkill -f raceboard-server 2>/dev/null
sleep 1

# Build the server
echo "Building server..."
cargo build --bin raceboard-server

# Start the server
echo "Starting Raceboard server..."
cargo run --bin raceboard-server