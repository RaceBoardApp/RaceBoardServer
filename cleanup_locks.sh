#!/bin/bash
# Script to clean up stale sled database locks

RACEBOARD_DB_DIR="$HOME/.raceboard"

if [ -d "$RACEBOARD_DB_DIR" ]; then
    echo "Cleaning up database locks in $RACEBOARD_DB_DIR..."
    
    # Remove lock files
    find "$RACEBOARD_DB_DIR" -name "*.lock" -type f -delete 2>/dev/null
    
    # Remove temporary databases from crashed processes
    find "$RACEBOARD_DB_DIR" -name "eta_history.db.*" -type d -mtime +1 -exec rm -rf {} \; 2>/dev/null
    
    echo "Cleanup complete."
else
    echo "Raceboard database directory not found at $RACEBOARD_DB_DIR"
fi