#!/bin/bash
export CLIPD_HOST=dragonflyg4
CW=~/powershell-clipd/target/release/clipwire
cd /home/cuzic/rust-nicola-worktrees/spike-ime-effect-learning/tools/e2e/ime_key_matrix
R=results/elw15
for s in 1 2; do
  $CW exec elw-cache-blind-spike >/dev/null 2>&1
  echo "== blind Bh $s $(date +%T)"
  ./run_walk.sh Bh $s $R/walk-Bh$s || echo FAIL
done
$CW exec elw-cache-clear-spike >/dev/null 2>&1
echo ALL_DONE
