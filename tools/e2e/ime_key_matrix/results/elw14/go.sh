#!/bin/bash
export CLIPD_HOST=dragonflyg4
CW=~/powershell-clipd/target/release/clipwire
cd /home/cuzic/rust-nicola-worktrees/spike-ime-effect-learning/tools/e2e/ime_key_matrix
R=results/elw14
$CW exec elw-deploy191 2>&1 | head -1
for i in $(seq 1 60); do sleep 15; o=$($CW exec elw-deploy191-check 2>&1); echo "$o" | grep -q "build: \(BUILD_\|OK\|FAIL\)" && break; done
echo "$o" > $R/deploy-check.txt; cat $R/deploy-check.txt
echo "$o" | grep -q "BUILD_OK" || { echo DEPLOY_NOT_OK; exit 1; }
$CW exec elw-sync-build 2>&1 | head -1
for i in $(seq 1 40); do sleep 10; o=$($CW exec elw-status 2>&1 | grep 'spike build'); echo "$o" | grep -qE 'BUILD_(OK|FAILED)' && { echo "$o"; break; }; done
echo "$o" | grep -q BUILD_OK || { echo SPIKE_NOT_OK; exit 1; }
for pair in "Bh 1" "Bh 2" "Ah 1"; do
  set -- $pair
  $CW exec elw-cache-clear-spike >/dev/null 2>&1
  echo "== $1 $2 $(date +%T)"
  ./run_walk.sh $1 $2 $R/walk-$1$2 || echo FAIL
done
echo ALL_DONE
