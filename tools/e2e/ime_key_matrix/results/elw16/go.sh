#!/bin/bash
export CLIPD_HOST=dragonflyg4
CW=~/powershell-clipd/target/release/clipwire
cd /home/cuzic/rust-nicola-worktrees/spike-ime-effect-learning/tools/e2e/ime_key_matrix
R=results/elw16
$CW exec elw-bypass-on 2>&1 | tail -2
for s in 5 6 7 8 9 10 11 12; do
  echo "== Apg $s $(date +%T)"
  ./run_walk.sh Apg $s $R/walk-Apg$s || echo FAIL
done
$CW exec elw-bypass-off 2>&1 | tail -2
echo ALL_DONE
