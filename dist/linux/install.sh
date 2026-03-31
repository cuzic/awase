#!/bin/bash
set -euo pipefail

echo "=== awase Linux installer ==="

# Check input group membership
if ! groups | grep -q input; then
    echo "WARNING: Current user is not in the 'input' group."
    echo "Run: sudo usermod -aG input $USER"
    echo "Then log out and back in."
    echo ""
fi

# Install binary
BINARY="target/release/awase"
if [ ! -f "$BINARY" ]; then
    echo "Building release binary..."
    cargo build --release -p awase-linux
    BINARY="target/release/awase"
fi

mkdir -p ~/.local/bin
cp "$BINARY" ~/.local/bin/awase
echo "Installed binary to ~/.local/bin/awase"

# Install config
mkdir -p ~/.config/awase
if [ ! -f ~/.config/awase/config.toml ]; then
    cp config.toml ~/.config/awase/config.toml 2>/dev/null || echo "# Default config" > ~/.config/awase/config.toml
    echo "Installed config to ~/.config/awase/config.toml"
fi

# Install systemd service
mkdir -p ~/.config/systemd/user
cp dist/linux/awase.service ~/.config/systemd/user/
systemctl --user daemon-reload
echo "Installed systemd service"

# Install XDG autostart
mkdir -p ~/.config/autostart
cp dist/linux/awase.desktop ~/.config/autostart/
echo "Installed XDG autostart entry"

echo ""
echo "=== Installation complete ==="
echo ""
echo "To start now:  systemctl --user start awase"
echo "To enable:     systemctl --user enable awase"
echo "To check:      systemctl --user status awase"
echo "To view logs:  journalctl --user -u awase -f"
