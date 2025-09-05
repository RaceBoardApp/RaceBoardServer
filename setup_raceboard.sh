#!/bin/bash

# Raceboard Server - ZSH Setup Script
# This script adds Raceboard binaries to your PATH and creates convenient aliases

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
RELEASE_DIR="$SCRIPT_DIR/target/release"
DEBUG_DIR="$SCRIPT_DIR/target/debug"

# Color codes for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}Raceboard Server - ZSH Setup${NC}"
echo "================================"

# Check if .zshrc exists
if [ ! -f "$HOME/.zshrc" ]; then
    echo -e "${YELLOW}Creating .zshrc file...${NC}"
    touch "$HOME/.zshrc"
fi

# Backup .zshrc
cp "$HOME/.zshrc" "$HOME/.zshrc.backup.$(date +%Y%m%d_%H%M%S)"
echo -e "${GREEN}✓ Backed up .zshrc${NC}"

# Remove old Raceboard configuration if it exists
sed -i.tmp '/# ===== Raceboard Server Configuration =====/,/# ===== End Raceboard Configuration =====/d' "$HOME/.zshrc"

# Now add the complete configuration
cat >> "$HOME/.zshrc" << 'EOF'

# ===== Raceboard Server Configuration =====

# Path to Raceboard binaries
export PATH="RELEASE_DIR_PLACEHOLDER:$PATH"

# Raceboard aliases
alias raceboard-server='RELEASE_DIR_PLACEHOLDER/raceboard-server'
alias raceboard-server-dev='DEBUG_DIR_PLACEHOLDER/raceboard-server'
alias raceboard-gitlab='RELEASE_DIR_PLACEHOLDER/raceboard-gitlab'
alias raceboard-cmd='RELEASE_DIR_PLACEHOLDER/raceboard-cmd'
alias raceboard-claude='RELEASE_DIR_PLACEHOLDER/raceboard-claude'
alias raceboard-codex='RELEASE_DIR_PLACEHOLDER/raceboard-codex'
alias raceboard-gemini-watch='RELEASE_DIR_PLACEHOLDER/raceboard-gemini-watch'
alias raceboard-track='RELEASE_DIR_PLACEHOLDER/raceboard-track'

# Raceboard helper functions
raceboard-start() {
    echo 'Starting Raceboard Server...'
    RELEASE_DIR_PLACEHOLDER/raceboard-server &
    echo "Server started with PID: $!"
}

raceboard-stop() {
    echo 'Stopping Raceboard Server...'
    pkill -f raceboard-server
    echo 'Server stopped'
}

raceboard-restart() {
    raceboard-stop
    sleep 1
    raceboard-start
}

raceboard-status() {
    if pgrep -f raceboard-server > /dev/null; then
        echo '✓ Raceboard Server is running'
        echo "PID: $(pgrep -f raceboard-server)"
    else
        echo '✗ Raceboard Server is not running'
    fi
}

raceboard-logs() {
    if [ -f $HOME/.raceboard/server.log ]; then
        tail -f $HOME/.raceboard/server.log
    else
        echo 'No log file found'
    fi
}

raceboard-build() {
    echo 'Building Raceboard Server in release mode...'
    cd SCRIPT_DIR_PLACEHOLDER && cargo build --release
}

raceboard-build-all() {
    echo 'Building all Raceboard binaries in release mode...'
    cd SCRIPT_DIR_PLACEHOLDER && cargo build --release --all-targets
}

raceboard-test() {
    echo 'Running Raceboard tests...'
    cd SCRIPT_DIR_PLACEHOLDER && cargo test
}

raceboard-health() {
    curl -s http://localhost:7777/health | jq '.' 2>/dev/null || echo 'Server not responding'
}

raceboard-gitlab-start() {
    local config_file="${1:-SCRIPT_DIR_PLACEHOLDER/gitlab_config.toml}"
    if [ ! -f "$config_file" ]; then
        echo "Error: Config file not found: $config_file"
        echo "Usage: raceboard-gitlab-start [config_file]"
        return 1
    fi
    echo "Starting GitLab adapter with config: $config_file"
    RELEASE_DIR_PLACEHOLDER/raceboard-gitlab --config "$config_file" &
    echo "GitLab adapter started with PID: $!"
}

# Raceboard-cmd specific functions
race() {
    # Wrapper for raceboard-cmd that starts a race
    RELEASE_DIR_PLACEHOLDER/raceboard-cmd start --title "$@"
}

race-with-prompt() {
    # Start a race with both title and prompt
    local title="$1"
    shift
    local prompt="$@"
    RELEASE_DIR_PLACEHOLDER/raceboard-cmd start --title "$title" --prompt "$prompt"
}

race-pass() {
    # Mark the last race as passed
    local race_id=$(RELEASE_DIR_PLACEHOLDER/raceboard-cmd list --format json 2>/dev/null | jq -r '.[0].id' 2>/dev/null)
    if [ -n "$race_id" ]; then
        RELEASE_DIR_PLACEHOLDER/raceboard-cmd update "$race_id" --state passed
        echo "✓ Race $race_id marked as passed"
    else
        echo "No active race found"
    fi
}

race-fail() {
    # Mark the last race as failed
    local race_id=$(RELEASE_DIR_PLACEHOLDER/raceboard-cmd list --format json 2>/dev/null | jq -r '.[0].id' 2>/dev/null)
    if [ -n "$race_id" ]; then
        RELEASE_DIR_PLACEHOLDER/raceboard-cmd update "$race_id" --state failed
        echo "✗ Race $race_id marked as failed"
    else
        echo "No active race found"
    fi
}

race-cancel() {
    # Cancel the last race
    local race_id=$(RELEASE_DIR_PLACEHOLDER/raceboard-cmd list --format json 2>/dev/null | jq -r '.[0].id' 2>/dev/null)
    if [ -n "$race_id" ]; then
        RELEASE_DIR_PLACEHOLDER/raceboard-cmd update "$race_id" --state canceled
        echo "⚠ Race $race_id canceled"
    else
        echo "No active race found"
    fi
}

race-list() {
    # List recent races
    RELEASE_DIR_PLACEHOLDER/raceboard-cmd list "$@"
}

race-progress() {
    # Update progress of the current race
    local progress="$1"
    local race_id=$(RELEASE_DIR_PLACEHOLDER/raceboard-cmd list --format json 2>/dev/null | jq -r '.[0].id' 2>/dev/null)
    if [ -n "$race_id" ]; then
        RELEASE_DIR_PLACEHOLDER/raceboard-cmd update "$race_id" --progress "$progress"
        echo "Race $race_id progress: $progress%"
    else
        echo "No active race found"
    fi
}

race-eta() {
    # Update ETA of the current race
    local eta="$1"
    local race_id=$(RELEASE_DIR_PLACEHOLDER/raceboard-cmd list --format json 2>/dev/null | jq -r '.[0].id' 2>/dev/null)
    if [ -n "$race_id" ]; then
        RELEASE_DIR_PLACEHOLDER/raceboard-cmd update "$race_id" --eta "$eta"
        echo "Race $race_id ETA: $eta seconds"
    else
        echo "No active race found"
    fi
}

race-event() {
    # Add an event to the current race
    local event="$@"
    local race_id=$(RELEASE_DIR_PLACEHOLDER/raceboard-cmd list --format json 2>/dev/null | jq -r '.[0].id' 2>/dev/null)
    if [ -n "$race_id" ]; then
        RELEASE_DIR_PLACEHOLDER/raceboard-cmd event "$race_id" --message "$event"
        echo "Event added to race $race_id"
    else
        echo "No active race found"
    fi
}

# Command wrapper that tracks execution
track() {
    # Run command with raceboard-cmd and show output
    RELEASE_DIR_PLACEHOLDER/raceboard-cmd --output --title "Command: $*" -- "$@"
}

# Track command without output (silent tracking)
track-silent() {
    # Run command with raceboard-cmd without showing output
    RELEASE_DIR_PLACEHOLDER/raceboard-cmd --title "Command: $*" -- "$@"
}

raceboard-help() {
    echo 'Raceboard Server Commands:'
    echo '  raceboard-start         - Start the Raceboard server'
    echo '  raceboard-stop          - Stop the Raceboard server'
    echo '  raceboard-restart       - Restart the Raceboard server'
    echo '  raceboard-status        - Check if server is running'
    echo '  raceboard-logs          - Tail server logs'
    echo '  raceboard-health        - Check server health'
    echo '  raceboard-build         - Build server in release mode'
    echo '  raceboard-build-all     - Build all binaries in release mode'
    echo '  raceboard-test          - Run tests'
    echo '  raceboard-gitlab-start  - Start GitLab adapter with config'
    echo ''
    echo 'Race Tracking Commands (via raceboard-cmd):'
    echo '  race "title"            - Start a new race with title'
    echo '  race-with-prompt        - Start race with title and prompt'
    echo '  race-pass               - Mark current race as passed'
    echo '  race-fail               - Mark current race as failed'
    echo '  race-cancel             - Cancel current race'
    echo '  race-list               - List recent races'
    echo '  race-progress <0-100>   - Update progress of current race'
    echo '  race-eta <seconds>      - Update ETA of current race'
    echo '  race-event "message"    - Add event to current race'
    echo '  track <command>         - Execute and track a command'
    echo ''
    echo 'Direct Adapter Commands:'
    echo '  raceboard-cmd           - Command line adapter (direct access)'
    echo '  raceboard-gitlab        - GitLab pipeline adapter'
    echo '  raceboard-claude        - Claude AI adapter'
    echo '  raceboard-codex         - Codex adapter'
    echo '  raceboard-gemini-watch  - Gemini watcher adapter'
    echo '  raceboard-track         - General tracking adapter'
    echo ''
    echo 'Examples:'
    echo '  race "Building project"'
    echo '  track cargo build --release'
    echo '  race-progress 50'
    echo '  race-pass'
    echo ''
    echo 'Debug Commands:'
    echo '  raceboard-server-dev    - Run debug build of server'
}

# ===== End Raceboard Configuration =====
EOF

# Replace placeholders with actual paths
sed -i.tmp2 "s|SCRIPT_DIR_PLACEHOLDER|$SCRIPT_DIR|g" "$HOME/.zshrc"
sed -i.tmp3 "s|RELEASE_DIR_PLACEHOLDER|$RELEASE_DIR|g" "$HOME/.zshrc"
sed -i.tmp4 "s|DEBUG_DIR_PLACEHOLDER|$DEBUG_DIR|g" "$HOME/.zshrc"

# Clean up temp files
rm -f "$HOME/.zshrc.tmp" "$HOME/.zshrc.tmp2" "$HOME/.zshrc.tmp3" "$HOME/.zshrc.tmp4"

echo -e "${GREEN}✓ Added Raceboard configuration to .zshrc${NC}"

# Create a config directory if it doesn't exist
mkdir -p "$HOME/.raceboard"
echo -e "${GREEN}✓ Created ~/.raceboard directory${NC}"

# Build the binaries if they don't exist
echo ""
if [ ! -f "$RELEASE_DIR/raceboard-server" ]; then
    echo -e "${YELLOW}Release binaries not found. Building...${NC}"
    cd "$SCRIPT_DIR" && cargo build --release --all-targets
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✓ Successfully built all binaries${NC}"
    else
        echo -e "${RED}✗ Build failed. Please run 'cargo build --release' manually${NC}"
    fi
else
    echo -e "${GREEN}✓ Release binaries found${NC}"
fi

echo ""
echo -e "${BLUE}Setup complete!${NC}"
echo ""
echo "To apply changes, run:"
echo -e "  ${GREEN}source ~/.zshrc${NC}"
echo ""
echo "Then you can use:"
echo -e "  ${GREEN}raceboard-help${NC}     - Show all available commands"
echo -e "  ${GREEN}raceboard-start${NC}    - Start the server"
echo -e "  ${GREEN}raceboard-status${NC}   - Check server status"
echo ""
echo "For GitLab adapter:"
echo -e "  1. Edit ${YELLOW}gitlab_config.toml${NC} with your GitLab credentials"
echo -e "  2. Run: ${GREEN}raceboard-gitlab-start${NC}"