#!/usr/bin/env bash
# Build the Gmail channel WASM component and install the gmail-summarize skill
#
# Prerequisites:
#   - Rust with wasm32-wasip2 target: rustup target add wasm32-wasip2
#   - wasm-tools for component creation: cargo install wasm-tools
#
# Output:
#   - gmail.wasm - WASM component ready for deployment
#   - gmail.capabilities.json - Capabilities file (copy alongside .wasm)
#   - Installs gmail-summarize skill to ~/.ironclaw/skills/

set -euo pipefail

cd "$(dirname "$0")"

echo "Building Gmail channel WASM component..."

# Build the WASM module
cargo build --release --target wasm32-wasip2

# Convert to component model (if not already a component)
# wasm-tools component new is idempotent on components
WASM_PATH="target/wasm32-wasip2/release/gmail_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    # Create component if needed
    wasm-tools component new "$WASM_PATH" -o gmail.wasm 2>/dev/null || cp "$WASM_PATH" gmail.wasm

    # Optimize the component
    wasm-tools strip gmail.wasm -o gmail.wasm

    echo "Built: gmail.wasm ($(du -h gmail.wasm | cut -f1))"
    echo ""
    
    echo "To complete the installation:"
    echo "  1. Install the WASM channel:"
    echo "     mkdir -p ~/.ironclaw/channels"
    echo "     cp gmail.wasm gmail.capabilities.json ~/.ironclaw/channels/"
    echo ""
    echo "  2. Install the gmail-summarize skill:"
    echo "     mkdir -p ~/.ironclaw/skills"
    echo "     cp -r skills/gmail-summarize ~/.ironclaw/skills/"
    echo ""
    echo "  3. Verify the skill is installed:"
    echo "     ironclaw skills list"
    echo "     ironclaw skills info gmail-summarize"
    echo ""
    echo "  4. Restart IronClaw to load the new channel and skill"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
