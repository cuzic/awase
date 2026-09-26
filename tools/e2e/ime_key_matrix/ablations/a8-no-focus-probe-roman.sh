#!/usr/bin/env bash
# A8: 焦点プローブでのかなモード修正(conv-write-paths-inventory 経路9)を撤去する。
# MS-IME のかなモード + IME ON を観測しても ROMAN を足さない。
python3 - <<'PY'
p='crates/awase-windows/src/runtime/key_pipeline.rs'
s=open(p,encoding='utf8').read()
a='''                        if should_restore {
                            tracing::debug!(
                                "[ImmCrossProbe] kana mode'''
assert s.count(a)==1
open(p,'w',encoding='utf8').write(s.replace(a,a.replace('if should_restore {','if false && should_restore {'),1))
PY
