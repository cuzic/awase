#!/usr/bin/env bash
# A10(ADR-247): CUSTOM 表の F13〜F24 の行からの打鍵時予測(custom_f_key_prediction)を常に None にする。直接観測だけが残る。
python3 - <<'PY'
import re
p='crates/awase-windows/src/state/key_effect_predictor.rs'
s=open(p,encoding='utf8').read()
m=re.search(r'fn custom_f_key_prediction\([^)]*\)\s*->\s*Option<Prediction>\s*\{',s)
assert m
s=s[:m.end()]+"\n        if true {\n            return None;\n        }"+s[m.end():]
open(p,'w',encoding='utf8').write(s)
PY
