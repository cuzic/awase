#!/usr/bin/env bash
# Install dist/Awase.app to /Applications with a clean TCC slate.
#
# Why not plain `cp -R`: copying over an existing bundle MERGES old and new
# contents, which breaks the code signature — and the TCC (Accessibility)
# entry keeps pointing at the previous signature, so toggling the checkbox
# in System Settings has no effect on the new binary. The reliable sequence
# is: quit, remove, copy whole, reset the stale TCC entry, re-grant.
set -euo pipefail

cd "$(dirname "$0")/../.."

APP=dist/Awase.app
DEST=/Applications/Awase.app

if [[ ! -d "$APP" ]]; then
  echo "error: $APP not found — run ./packaging/macos/make-app.sh first" >&2
  exit 1
fi

# Quit a running instance so we don't replace a busy binary
osascript -e 'quit app "awase"' 2>/dev/null || true

rm -rf "$DEST"
ditto "$APP" "$DEST"

# Drop the Accessibility entry tied to the old signature. Without this the
# System Settings toggle operates on a dead entry. Ignore failure (first
# install has no entry yet).
tccutil reset Accessibility com.github.cuzic.awase || true

echo "Installed $DEST"
echo "Launch it (open $DEST) and grant Accessibility when prompted."
echo "Tip: build with CODESIGN_IDENTITY=<self-signed cert> to keep the"
echo "grant across rebuilds and skip the re-grant entirely."
