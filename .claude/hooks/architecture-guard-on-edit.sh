#!/usr/bin/env bash
# PostToolUse hook (Edit|Write): after Claude edits a *.rs file, run the
# awase-windows architecture_guard test so cross-crate architecture
# violations surface automatically.
#
# Runs async (see .claude/settings.json) with asyncRewake: exit 2 wakes
# Claude with this script's stdout as feedback; exit 0 is silent.
#
# This sandbox runs many concurrent Claude Code sessions across sibling
# repos, so this hook is run at low CPU/IO priority and skipped outright
# when the box is already oversubscribed.
set -u

input=$(cat)
fp=$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_response.filePath // empty' 2>/dev/null)

case "$fp" in
  *.rs) ;;
  *) exit 0 ;;
esac

# Skip entirely if the box is already oversubscribed (1-min load average
# above core count) — don't pile another cargo invocation onto a sandbox
# that other concurrent sessions are already saturating.
nproc_count=$(nproc 2>/dev/null || echo 8)
load1=$(awk '{print $1}' /proc/loadavg 2>/dev/null || echo 0)
if awk -v l="$load1" -v n="$nproc_count" 'BEGIN{exit !(l > n)}'; then
  exit 0
fi

cd /home/cuzic/rust-nicola || exit 0
out=$(ionice -c3 nice -n 19 cargo test -p awase-windows --test architecture_guard 2>&1)
fail=$(printf '%s' "$out" | grep -E 'FAILED|^error' | head -5)

if [ -n "$fail" ]; then
  printf '%s\n' "$fail"
  exit 2
fi
exit 0
