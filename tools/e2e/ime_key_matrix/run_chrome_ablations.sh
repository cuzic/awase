#!/usr/bin/env bash
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"; A="$HERE/ablations"
export CLIPD_HOST=dragonflyg4; CW="$HOME/powershell-clipd/target/release/clipwire"
timeout 250 "$CW" exec chrome-probe-build >/dev/null 2>&1
echo "| 構成 | 集計 | ケース3 | ケース7 |" > "$HERE/results/CHROME_ABL.md"; echo "|---|---|---|---|" >> "$HERE/results/CHROME_ABL.md"
rm -f "$HERE/results/chrome-abl.progress"
bash "$HERE/ablate_chrome.sh" base none
bash "$HERE/ablate_chrome.sh" E1-widen-guard2 "$A/a8-e1-widen-guard2.sh"
bash "$HERE/ablate_chrome.sh" E2-150ms "$A/a9-e2-delayed-check.sh" 150
bash "$HERE/ablate_chrome.sh" E2-300ms "$A/a9-e2-delayed-check.sh" 300
echo done > "$HERE/results/chrome-abl.done"
