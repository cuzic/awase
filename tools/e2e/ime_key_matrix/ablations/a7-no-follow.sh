#!/usr/bin/env bash
# A7: follow(ADR-187)を無効化する(通過マークを立てない)。ATOKパススルーで実IMEとEngineのずれを意図的に起こし、
# リセット操作(Ctrl+無変換→Ctrl+変換)で直るかを見るための対照(`--resync`)。
python3 - <<'PY'
import re
for p,old in [('crates/awase-windows/src/runtime/executor.rs','ime.arm_mode_key_pass_mark(now);'),
              ('crates/awase-windows/src/runtime/key_pipeline.rs','self.platform_state.ime.arm_mode_key_pass_mark(now);')]:
    s=open(p,encoding='utf8').read()
    assert old in s, p
    open(p,'w',encoding='utf8').write(s.replace(old,'let _ = now;',1))
PY
