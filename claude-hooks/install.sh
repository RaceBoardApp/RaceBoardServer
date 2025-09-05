#!/bin/bash

# Claude Code Raceboard Hooks Installer

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "🚀 Claude Code Raceboard Hooks Installer"
echo "========================================="
echo

# Check if raceboard-claude binary exists
if ! command -v raceboard-claude &> /dev/null && [ ! -f ./target/debug/raceboard-claude ]; then
    echo -e "${YELLOW}Building raceboard-claude binary...${NC}"
    cargo build --bin raceboard-claude
fi

# Determine Claude Code config directory
CLAUDE_CONFIG_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.config/claude}"
CLAUDE_SETTINGS_DIR="${CLAUDE_SETTINGS_DIR:-$HOME/.claude}"
HOOKS_DIR="$CLAUDE_CONFIG_DIR/hooks"

echo "📁 Installing hooks to: $HOOKS_DIR"

# Create hooks directory
mkdir -p "$HOOKS_DIR"

# Get the directory of this script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Copy hook scripts
echo "📝 Installing hook scripts..."

# Install prompt-submit hook
cp "$SCRIPT_DIR/prompt-submit.sh" "$HOOKS_DIR/prompt-submit"
chmod +x "$HOOKS_DIR/prompt-submit"
echo "   ✓ prompt-submit hook installed"

# Install response-received hook
cp "$SCRIPT_DIR/response-received.sh" "$HOOKS_DIR/response-received"
chmod +x "$HOOKS_DIR/response-received"
echo "   ✓ response-received hook installed"

# Create/update settings file
SETTINGS_FILE="$CLAUDE_SETTINGS_DIR/settings.json"
mkdir -p "$CLAUDE_SETTINGS_DIR"

if [ ! -f "$SETTINGS_FILE" ]; then
    echo "📄 Creating default settings file..."
    cat > "$SETTINGS_FILE" << 'EOF'
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "~/.config/claude/hooks/prompt-submit"
          }
        ]
      }
    ],
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "~/.config/claude/hooks/response-received"
          }
        ]
      }
    ]
  }
}
EOF
    echo "   ✓ Settings file created at $SETTINGS_FILE"
else
    echo -e "${YELLOW}⚠️  Settings file already exists at $SETTINGS_FILE${NC}"
    echo "   Please manually add or update the hooks configuration:"
    echo '   {
     "hooks": {
       "UserPromptSubmit": [
         {
           "matcher": "",
           "hooks": [{
             "type": "command",
             "command": "~/.config/claude/hooks/prompt-submit"
           }]
         }
       ],
       "Stop": [
         {
           "matcher": "",
           "hooks": [{
             "type": "command",
             "command": "~/.config/claude/hooks/response-received"
           }]
         }
       ]
     }
   }'
fi

echo
echo -e "${GREEN}✅ Installation complete!${NC}"
echo
echo "📋 Next steps:"
echo "1. Start the Raceboard server:"
echo "   cargo run --bin raceboard-server"
echo
echo "2. If using the raceboard-claude binary, add it to your PATH:"
echo "   export PATH=\"$PWD/target/debug:\$PATH\""
echo
echo "3. Restart Claude Code or reload configuration"
echo
echo "4. Submit a prompt to Claude and watch races appear in Raceboard!"
echo
echo "📊 View races at: http://localhost:7777/races"
echo

# Test server connectivity
if curl -s http://localhost:7777/health > /dev/null 2>&1; then
    echo -e "${GREEN}✓ Raceboard server is running${NC}"
else
    echo -e "${YELLOW}⚠️  Raceboard server is not running. Start it with: cargo run --bin raceboard-server${NC}"
fi