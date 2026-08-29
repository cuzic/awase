#!/usr/bin/env bash
# Build dist/Awase.app from the release binary.
#
# A proper .app bundle gives awase a stable TCC identity: the Accessibility /
# Input Monitoring grant sticks to the bundle instead of whichever terminal
# launched the raw binary.
set -euo pipefail

cd "$(dirname "$0")/../.."

cargo build --release -p awase-macos

APP=dist/Awase.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
cp target/release/awase "$APP/Contents/MacOS/awase"

# Prefer a user-edited config.toml; fall back to the sample.
if [[ -f config.toml ]]; then
  cp config.toml "$APP/Contents/Resources/config.toml"
else
  cp config.sample.toml "$APP/Contents/Resources/config.toml"
fi
cp -R layout "$APP/Contents/Resources/layout"

# Sign the bundle. Ad-hoc ("-") works but its identity changes on every
# rebuild, so macOS drops the Accessibility grant each time. For a stable
# grant, create a self-signed code-signing certificate in Keychain Access
# and pass it via CODESIGN_IDENTITY.
codesign --force --sign "${CODESIGN_IDENTITY:--}" "$APP"

echo "Built $APP"
echo "Install: ./packaging/macos/install-app.sh"
