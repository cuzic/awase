#!/usr/bin/env bash
# A9(ADR-247): F13〜F24 を「通したモードキー」から外す(直接観測の窓・通過マークを開かない)。予測(custom_f_key_prediction)だけが残る。
python3 - <<'PY'
p='crates/awase-windows/src/vk.rs'
s=open(p,encoding='utf8').read()
old=" || is_role_fkey(vk_code)\n}"
assert old in s
s=s.replace(old,"\n}",1)
open(p,'w',encoding='utf8').write(s)
PY
