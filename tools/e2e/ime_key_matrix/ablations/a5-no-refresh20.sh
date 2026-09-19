#!/usr/bin/env bash
# A5: 物理IMEキーが通過した後の20ms IME再読み取り(may_change_ime → schedule_ime_refresh(20))を削除する。
python3 - <<'PY'
p='crates/awase-windows/src/runtime/key_pipeline.rs'
s=open(p,encoding='utf8').read()
a="""            self.schedule_ime_refresh(20);
            tracing::debug!("may_change_ime key passed through → IME refresh scheduled (20ms)");"""
assert a in s
open(p,'w',encoding='utf8').write(s.replace(a,"            tracing::debug!(\"may_change_ime key passed through (refresh ablated)\");",1))
PY
