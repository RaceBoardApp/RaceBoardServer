#!/bin/bash

# Codex CLI Raceboard Adapter Installer

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "🚀 Codex CLI Raceboard Adapter Installer"
echo "========================================="
echo

# Detect shell
if [ -n "$ZSH_VERSION" ]; then
    SHELL_TYPE="zsh"
    SHELL_RC="$HOME/.zshrc"
elif [ -n "$BASH_VERSION" ]; then
    SHELL_TYPE="bash"
    SHELL_RC="$HOME/.bashrc"
else
    echo -e "${RED}Unsupported shell. Please install manually.${NC}"
    exit 1
fi

echo "📁 Detected shell: $SHELL_TYPE"
echo "📄 Shell config: $SHELL_RC"
echo

# Check if codex is installed
if ! command -v codex &> /dev/null; then
    echo -e "${RED}❌ Codex CLI not found. Please install Codex first.${NC}"
    echo "   Visit: https://github.com/khulnasoft/codex"
    exit 1
fi

# Check if raceboard-cmd exists
if [ ! -f ./target/debug/raceboard-cmd ]; then
    echo -e "${YELLOW}Building raceboard-cmd binary...${NC}"
    cargo build --bin raceboard-cmd
fi

# Get the directory of this script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

echo "🔧 Choose installation method:"
echo "1) Shell wrapper (intercepts all codex commands)"
echo "2) Aliases (use cdx, cdx-ai, cdx-run, etc.)"
echo "3) Both"
read -p "Enter choice [1-3]: " choice

case $choice in
    1)
        # Install wrapper
        echo "📝 Installing shell wrapper..."
        echo "" >> "$SHELL_RC"
        echo "# Codex Raceboard Integration" >> "$SHELL_RC"
        echo "source $SCRIPT_DIR/codex-wrapper.sh" >> "$SHELL_RC"
        echo "   ✓ Wrapper installed"
        ;;
    2)
        # Install aliases
        echo "📝 Installing aliases..."
        echo "" >> "$SHELL_RC"
        echo "# Codex Raceboard Aliases" >> "$SHELL_RC"
        echo "source $SCRIPT_DIR/codex-aliases.sh" >> "$SHELL_RC"
        echo "   ✓ Aliases installed"
        echo
        echo "Available aliases:"
        echo "  cdx       - Track any codex command"
        echo "  cdx-ai    - Track codex ai commands"
        echo "  cdx-run   - Track codex run commands"
        echo "  cdx-chat  - Track codex chat sessions"
        ;;
    3)
        # Install both
        echo "📝 Installing wrapper and aliases..."
        echo "" >> "$SHELL_RC"
        echo "# Codex Raceboard Integration" >> "$SHELL_RC"
        echo "source $SCRIPT_DIR/codex-wrapper.sh" >> "$SHELL_RC"
        echo "source $SCRIPT_DIR/codex-aliases.sh" >> "$SHELL_RC"
        echo "   ✓ Wrapper and aliases installed"
        ;;
    *)
        echo -e "${RED}Invalid choice${NC}"
        exit 1
        ;;
esac

echo
echo -e "${GREEN}✅ Installation complete!${NC}"
echo
echo "📋 Next steps:"
echo "1. Reload your shell configuration:"
echo "   source $SHELL_RC"
echo
echo "2. Start the Raceboard server:"
echo "   cargo run --bin raceboard-server"
echo
echo "3. Use Codex normally (wrapper) or with aliases:"
if [ "$choice" = "1" ] || [ "$choice" = "3" ]; then
    echo "   codex ai 'write hello world'"
fi
if [ "$choice" = "2" ] || [ "$choice" = "3" ]; then
    echo "   cdx-ai 'write hello world'"
fi
echo
echo "4. View races at: http://localhost:7777/races"
echo

# Test server connectivity
if curl -s http://localhost:7777/health > /dev/null 2>&1; then
    echo -e "${GREEN}✓ Raceboard server is running${NC}"
else
    echo -e "${YELLOW}⚠️  Raceboard server is not running. Start it with: cargo run --bin raceboard-server${NC}"
fi