#!/usr/bin/env bash
# A3: 無変換/変換の単独タップでのeisu reset抑止(ADR-186決定2で追加した条件)を削除する。
python3 - <<'PY'
p='crates/awase-windows/src/runtime/key_pipeline.rs'
s=open(p,encoding='utf8').read()
a="applied && new_ime_on && !keep_observed_eisu,"
assert a in s
open(p,'w',encoding='utf8').write(s.replace(a,"applied && new_ime_on,",1))
PY
