#!/usr/bin/env bash
# A10: BUG-114 の 2 つ目の修正(ADR-134 Finding 5、`ReadBackQuery::AnyFreshEvidence` から ObserverPoll・ConvOpenInference を外す)を
# 無効化する。修正前は GJI I/O 監視(ObserverPoll)が「外界が動いた証拠」として扱われ、give-up の 3 秒クールダウン明けに
# 再武装してしまった。wt-probe の相 E(tools/e2e/wt、BUG-114)の基準 5(再武装)が退行を検出できる(FAIL する)ことの負の対照。
python3 - <<'PY'
p = 'crates/awase-windows/src/state/observation_store.rs'
s = open(p, encoding='utf8').read()
old = """                let latest = self.most_recent_trusted_after_excluding(
                    now,
                    since,
                    &EXCLUDED_FROM_ANY_FRESH_EVIDENCE,
                );"""
new = """                let _ = EXCLUDED_FROM_ANY_FRESH_EVIDENCE;
                let latest = self.most_recent_trusted_after_excluding(now, since, &[]);"""
assert s.count(old) == 1, 'AnyFreshEvidence の除外リストが見つからない'
s = s.replace(old, new, 1)
open(p, 'w', encoding='utf8').write(s)
PY
