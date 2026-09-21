#!/bin/bash
export CLIPD_HOST=dragonflyg4
CW=~/powershell-clipd/target/release/clipwire
cd /home/cuzic/rust-nicola-worktrees/spike-ime-effect-learning/tools/e2e/ime_key_matrix
R=results/elw13
for pair in "Ah 1" "Bh 2"; do
  set -- $pair
  $CW exec elw-cache-clear-spike >/dev/null 2>&1
  echo "== $1 $2 $(date +%T)"
  ./run_walk.sh $1 $2 $R/walk-$1$2 || echo FAIL
done
echo ALL_DONE
