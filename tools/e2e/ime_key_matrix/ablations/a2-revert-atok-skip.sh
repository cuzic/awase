#!/usr/bin/env bash
# A2: ATOKプリセットでcustom_keymap_tableを読まない修正を元に戻す(ADR-174のフォールスルーを全プリセットで有効に戻す)。
python3 - <<'PY'
p='crates/awase-windows/src/gji_charset_autodetect.rs'
s=open(p,encoding='utf8').read()
a="if raw.session_keymap != Some(awase_gji_config::SESSION_KEYMAP_ATOK) {"
assert a in s
open(p,'w',encoding='utf8').write(s.replace(a,"if true {",1))
PY
