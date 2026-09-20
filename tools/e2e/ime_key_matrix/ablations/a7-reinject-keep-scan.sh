#!/usr/bin/env bash
# A7(BUG-147の仮説検証): hook で吸収して再注入するキーに、元の物理キーのスキャンコードを引き継ぐ(現状は wScan=0)。
python3 - <<'PY'
p='crates/awase-windows/src/lib.rs'
s=open(p,encoding='utf8').read()
a="                    wScan: 0,\n                    dwFlags: if is_keyup {"
b="                    wScan: self.scan_code.0 as u16,\n                    dwFlags: if is_keyup {"
assert s.count(a)==1
open(p,'w',encoding='utf8').write(s.replace(a,b,1))
PY
