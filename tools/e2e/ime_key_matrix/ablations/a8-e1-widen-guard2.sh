#!/usr/bin/env bash
# E1(BUG-149): idle-conv-check の入口(ガード2、TsfNativeのみ)を Imm32Unavailable(Chrome等)にも広げる。
python3 - <<'PY'
p='crates/awase-windows/src/runtime/key_pipeline.rs'
s=open(p,encoding='utf8').read()
i=s.index('fn kp_stage_idle_conv_check_inner(')
a='.is_effectively_tsf_native(self.platform.focus.class_name());'
j=s.index(a,i)
s=s[:j]+'.cannot_verify_real_ime_state(self.platform.focus.class_name());'+s[j+len(a):]
open(p,'w',encoding='utf8').write(s)
PY
