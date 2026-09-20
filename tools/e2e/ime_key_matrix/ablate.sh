#!/usr/bin/env bash
# 撤去・統合の実験(ablation): awase の実装ブランチにミューテーション(コード撤去)を当て、Windows へデプロイし、
# E2E(run.sh)を実行して結果を results/<label>.txt に残す。
#
# 使い方: ablate.sh <label> <mutator.sh|none> <argsターゲット> <configターゲット>
#   mutator.sh : ablation用worktree(adr186-ablation)の中で実行するコード変更スクリプト(none=変更なし)
#   argsターゲット: e2e-args-default | e2e-args-hold180 | e2e-args-henkan | e2e-args-henkan-hold180
#   configターゲット: e2e-config-toggle-true | e2e-config-toggle-false
set -u
LABEL=$1; MUT=$2; ARGS=$3; CFG=$4
HERE="$(cd "$(dirname "$0")" && pwd)"
WT="${ABL_WT:-$HOME/rust-nicola-worktrees/adr186-ablation}"
BASE="${ABL_BASE:-feat/adr186-nonconvert-toggle-belief-follow}"
: "${CLIPD_HOST:=dragonflyg4}"; export CLIPD_HOST
CW="${CLIPWIRE:-$HOME/powershell-clipd/target/release/clipwire}"
RES="$HERE/results"; mkdir -p "$RES"

git -C "$WT" checkout -q -f -B e2e/ablation "$BASE" || exit 2
if [ "$MUT" != none ]; then
  ( cd "$WT" && bash "$MUT" ) || { echo "mutator失敗"; exit 2; }
  git -C "$WT" commit -q -am "ablation: $LABEL" || { echo "mutatorが差分を作らなかった"; exit 2; }
fi
# コンパイル確認(Windows向け)
( cd "$WT" && cargo check -q --target x86_64-pc-windows-msvc -p awase-windows 2>&1 | grep -E "^error" -A5 | head -20 )
git -C "$WT" push -q -f origin e2e/ablation 2>&1 | tail -1

"$CW" exec "$CFG" >/dev/null 2>&1
"$CW" exec "$ARGS" >/dev/null 2>&1
"$CW" exec e2e-deploy >/dev/null 2>&1
for _ in $(seq 1 30); do
  out=$("$CW" exec adr186-deploy-check 2>&1)
  echo "$out" | grep -q "BUILD_OK_STARTED_TESTINJ" && break
  echo "$out" | grep -q "BUILD_FAILED" && { echo "awaseのビルド失敗($LABEL)"; echo "$out" | tail -5; exit 2; }
  sleep 15
done
echo "$out" | grep -q "BUILD_OK_STARTED_TESTINJ" || { echo "ビルド待ちtimeout($LABEL)"; exit 2; }
sleep 4
{
  echo "### $LABEL  (mutator=$MUT args=$ARGS config=$CFG base=$(git -C "$WT" rev-parse --short "$BASE"))"
  bash "$HERE/run.sh" "$RES/$LABEL"
  echo "exit=$?"
} 2>&1 | tee "$RES/$LABEL.txt"
