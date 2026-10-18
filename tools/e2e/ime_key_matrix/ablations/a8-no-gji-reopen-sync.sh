#!/usr/bin/env bash
# A8: BUG-170 の修正(ADR-203 (i) 送信時の belief 起点 ON 同期、(ii) 物理 ON キーでの GjiFsm Reopen)を無効化する。
# 修正前の挙動(物理 OFF→ON の後も GjiFsm が OnWarm/OffCold のまま)を再現し、sc-reopen-* の判定が退行を検出できる(FAIL する)ことの
# 負の対照にする(ADR-203 (d)「修正前 FAIL・修正後 PASS の両方を実測」)。
python3 - <<'PY'
p1 = 'crates/awase-windows-core/src/state/gji_direct_mechanism.rs'
s = open(p1, encoding='utf8').read()
old1 = "    send_has_romaji\n        && !injection_is_unicode"
new1 = "    false\n        && send_has_romaji\n        && !injection_is_unicode"
assert s.count(old1) == 1, 'needs_belief_sync_on の本体が見つからない'
s = s.replace(old1, new1, 1)
old2 = ") -> Option<GjiFsmSync> {\n    if candidate_visible {"
new2 = ") -> Option<GjiFsmSync> {\n    if candidate_visible || true {"
assert s.count(old2) == 1, 'reopen_obligation の本体が見つからない'
s = s.replace(old2, new2, 1)
open(p1, 'w', encoding='utf8').write(s)
PY
