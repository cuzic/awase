#!/usr/bin/env bash
# A9: 開閉書き込みに付随する ROMAN 補完(conv-write-paths-inventory 経路1+2、sync/async の両窓口)を同時に撤去する。
python3 - <<'PY'
p='crates/awase-windows/src/state/ime_actuation_decision.rs'
s=open(p,encoding='utf8').read()
a1='''    belief_input_mode: InputModeState,
) -> bool {
    open && matches!('''
assert s.count(a1)==1
s=s.replace(a1,a1.replace('    open && matches!(','    if true {\n        return false;\n    }\n    open && matches!('),1)
a2='''    if open && !matches!(inputs.belief_input_mode, InputModeState::ObservedKana) {
        ConvAfterOpenId::Write(None)'''
assert s.count(a2)==1
s=s.replace(a2,a2.replace('if open &&','if false && open &&'),1)
open(p,'w',encoding='utf8').write(s)
PY
