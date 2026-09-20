#!/usr/bin/env bash
cd /home/cuzic/rust-nicola-worktrees/spike-ime-effect-learning/tools/e2e/ime_key_matrix
for m in "A 1" "B 1" "A 2" "B 2"; do set -- $m; echo "== start $1 $2 $(date +%T)"; ./run_walk.sh $1 $2 /home/cuzic/rust-nicola-worktrees/spike-ime-effect-learning/tools/e2e/ime_key_matrix/results/elw2/$1$2 || { echo "FAILED $1 $2"; break; }; done
echo ALL_DONE
