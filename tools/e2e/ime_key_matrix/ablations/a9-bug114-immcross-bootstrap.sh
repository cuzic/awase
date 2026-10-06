#!/usr/bin/env bash
# A9: BUG-114 根本原因1(起動時の app_policy が ImmCross=FeedbackPolicy::Read に焼き付く)を再現する。
# 起動時スコープ(ADR-232 D1 の InitialFocusScopeEstablished)へ渡す profile を live の分類でなく ImmCross に固定し、
# 修正前(ADR-134 D1c 以前、Standard へのフォールバック)と同じ app_policy にする。wt-probe の相 D/E(tools/e2e/wt、BUG-114)の判定が
# 退行を検出できる(FAIL する)ことの負の対照。
python3 - <<'PY'
p = 'crates/awase-windows/src/runtime/focus_tracking.rs'
s = open(p, encoding='utf8').read()
old = """        let profile: crate::state::ime_event::ImePolicyProfile =
            self.platform.current_app_profile().into();
        let focus_epoch = self.platform_state.focus.focus_epoch;"""
new = """        let profile = crate::state::ime_event::ImePolicyProfile::ImmCross;
        let focus_epoch = self.platform_state.focus.focus_epoch;"""
assert s.count(old) == 1, 'sync_initial_focus_scope の profile 取得が見つからない'
s = s.replace(old, new, 1)
open(p, 'w', encoding='utf8').write(s)
PY
