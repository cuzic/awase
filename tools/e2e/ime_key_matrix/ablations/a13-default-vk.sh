#!/usr/bin/env bash
# A13: Win32/UWP の既定の注入モードを Unicode から Vk に変える(`force_tsf` に無い窓は Vk 重畳送信になる)。
# 「Unicode 注入を撤去した後の姿」を作る実験(ECRS、docs/tasks/mech-ecrs-2026-10-21/unicode-injection-removal.md)。
# `injection_mode_for` の既定の腕だけを変える。Chrome 等の TsfNative(Vk)、force_tsf の窓(Tsf)は変わらない。
python3 - <<'PY'
p = 'crates/awase-windows/src/output/types.rs'
s = open(p, encoding='utf8').read()
old = """            if app_kind == AppKind::TsfNative {
                InjectionMode::Vk
            } else {
                InjectionMode::Unicode
            }"""
new = """            let _ = app_kind;
            InjectionMode::Vk"""
assert s.count(old) == 1, 'injection_mode_for の既定の腕が見つからない'
s = s.replace(old, new, 1)
open(p, 'w', encoding='utf8').write(s)
PY
