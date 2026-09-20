#!/usr/bin/env bash
# A1: KeyUp解決(delegate_to_open_axisを持つ親指の単独タップ)の変更を元に戻す(ADR-186の根本原因修正の撤去)。
python3 - <<'PY'
p='src/engine/nicola_fsm.rs'
s=open(p,encoding='utf8').read()
a="""            && (special.delegate_to_open_axis.is_some()
                || special.mode_key_config.is_some_and(|cfg| {
                    matches!(
                        SoloTapAction::from(cfg.for_composing(composing)),
                        SoloTapAction::Passthrough
                    )
                }))"""
b="""            && special.delegate_to_open_axis.is_none()
            && special.mode_key_config.is_some_and(|cfg| {
                matches!(
                    SoloTapAction::from(cfg.for_composing(composing)),
                    SoloTapAction::Passthrough
                )
            })"""
assert a in s
open(p,'w',encoding='utf8').write(s.replace(a,b,1))
PY
