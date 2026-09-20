#!/usr/bin/env bash
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
export REAL_ONLY=1
bash "$HERE/ablate.sh" a7-deploy "$HERE/ablations/a7-reinject-keep-scan.sh" e2e-args-default e2e-config-toggle-true
bash "$HERE/stat_runs.sh" bug147-a7 12
echo done > "$HERE/results/bug147-a7.done"
