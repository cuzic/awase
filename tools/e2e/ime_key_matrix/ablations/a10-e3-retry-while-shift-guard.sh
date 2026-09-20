#!/usr/bin/env bash
# E3(BUG-149): E2(モードキー後の遅延conv読み取り)に、「Shift関連のガード中なら100msごとに再試行(最大30回)」を足す。
HERE="$(cd "$(dirname "$0")" && pwd)"
bash "$HERE/a9-e2-delayed-check.sh" || exit 1
python3 - <<'PY'
def edit(p,f):
    s=open(p,encoding='utf8').read(); s=f(s); open(p,'w',encoding='utf8').write(s)
def rep1(s,a,b):
    assert s.count(a)==1,(a,s.count(a)); return s.replace(a,b)
edit('crates/awase-windows/src/state/platform_state.rs', lambda s: rep1(rep1(s,
  '    pub force_conv_check: bool,','    pub force_conv_check: bool,\n    pub modekey_retries: u8,'),
  '            force_conv_check: false,','            force_conv_check: false,\n            modekey_retries: 0,'))
edit('crates/awase-windows/src/runtime/key_pipeline.rs', lambda s: rep1(s,'''    pub(crate) fn run_modekey_conv_check(&mut self) {
        let Some(mut ev) = self.platform_state.gate.pending_modekey_event.take() else {
            return;
        };''','''    pub(crate) fn run_modekey_conv_check(&mut self) {
        if (self.platform_state.gate.half_width_alnum.is_guard_pending()
            || self.platform_state.gate.half_width_alnum.is_toggle_active())
            && self.platform_state.gate.modekey_retries < 30
        {
            self.platform_state.gate.modekey_retries += 1;
            tracing::debug!(
                "[E3] shift関連ガード中 → 100ms後に再試行 ({}回目)",
                self.platform_state.gate.modekey_retries
            );
            self.platform
                .timer
                .set(crate::TIMER_MODEKEY_CONV, std::time::Duration::from_millis(100));
            return;
        }
        self.platform_state.gate.modekey_retries = 0;
        let Some(mut ev) = self.platform_state.gate.pending_modekey_event.take() else {
            return;
        };
        tracing::debug!("[E3] 強制conv読み取りを実行");'''))
PY
