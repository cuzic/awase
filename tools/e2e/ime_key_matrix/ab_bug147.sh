#!/usr/bin/env bash
# BUG-147 切り分け: GJI単体(awase停止)とawase起動で、実IMEだけの判定(REAL_ONLY)の失敗率を比べる。
set -u
export CLIPD_HOST=${CLIPD_HOST:-dragonflyg4}
CW="${CLIPWIRE:-$HOME/powershell-clipd/target/release/clipwire}"
HERE="$(cd "$(dirname "$0")" && pwd)"
N=${1:-12}; TAG=${2:-bug147}
export REAL_ONLY=1
"$CW" exec e2e-awase-stop >/dev/null 2>&1
bash "$HERE/stat_runs.sh" "$TAG-noawase" "$N"
"$CW" exec e2e-awase-start >/dev/null 2>&1
bash "$HERE/stat_runs.sh" "$TAG-awase" "$N"
echo "done" > "$HERE/results/$TAG.done"
