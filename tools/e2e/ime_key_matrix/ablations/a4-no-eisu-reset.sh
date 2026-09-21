#!/usr/bin/env bash
# A4: eisu reset(PostSetOpenEisuReset/UserImeOnEisuReset/UserTurnOnEisuReset)の判定関数を、常にNoneを返すようにして全経路を無効化する。
python3 - <<'PY'
import re
p='crates/awase-windows/src/state/eisu_recovery.rs'
s=open(p,encoding='utf8').read()
for name in ['eisu_reset_on_ime_on','eisu_reset_on_turn_on_while_open']:
    m=re.search(r'pub fn '+name+r'\([^)]*\)\s*->\s*Option<InputModeState>\s*\{',s)
    assert m, name
    s=s[:m.end()]+"\n    if true {\n        return None;\n    }"+s[m.end():]
open(p,'w',encoding='utf8').write(s)
PY
