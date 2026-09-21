#!/usr/bin/env bash
# A6: idle-conv-check(次の打鍵後に変換モードを受動観測して英数を検出する仕組み)を無効化する。
python3 - <<'PY'
p='src/engine/idle_check.rs'
s=open(p,encoding='utf8').read()
a="    // ガード 1: KeyDown イベントのみ対象\n"
assert a in s
open(p,'w',encoding='utf8').write(s.replace(a,"    if true {\n        return false;\n    }\n"+a,1))
PY
