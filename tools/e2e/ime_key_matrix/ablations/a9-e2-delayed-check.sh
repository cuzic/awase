#!/usr/bin/env bash
# E2(BUG-149): 物理のIMEモードキー(KeyDown)の E2_DELAY ms 後に、ガード(TsfNativeのみ/タイピング停止/明示操作の抑止窓/モードキー自身)を
# 無視して idle-conv-check を1回走らせる。E2_DELAY 既定250ms。
DELAY="${E2_DELAY:-250}"
python3 - "$DELAY" <<'PY'
import sys
delay=sys.argv[1]
def edit(p,f):
    s=open(p,encoding='utf8').read(); s=f(s); open(p,'w',encoding='utf8').write(s)
def rep1(s,a,b):
    assert s.count(a)==1,(a,s.count(a)); return s.replace(a,b)
# 1. タイマーID
edit('crates/awase-windows/src/lib.rs', lambda s: rep1(s,'pub const TIMER_FOCUS_RESYNC: usize = 109;','pub const TIMER_FOCUS_RESYNC: usize = 109;\npub const TIMER_MODEKEY_CONV: usize = 110;'))
# 2. gate のフィールド
def gate(s):
    s=rep1(s,'    pub idle_conv_check_in_flight_since_ms: Option<u64>,','    pub idle_conv_check_in_flight_since_ms: Option<u64>,\n    pub force_conv_check: bool,\n    pub pending_modekey_event: Option<awase::types::RawKeyEvent>,')
    s=rep1(s,'            idle_conv_check_in_flight_since_ms: None,','            idle_conv_check_in_flight_since_ms: None,\n            force_conv_check: false,\n            pending_modekey_event: None,')
    return s
edit('crates/awase-windows/src/state/platform_state.rs', gate)
# 3. key_pipeline: タイマー武装 + 強制実行 + 入口のバイパス
def kp(s):
    s=rep1(s,'''    fn kp_stage_idle_conv_check(&mut self, event: &RawKeyEvent) {
        let _ = self.kp_stage_idle_conv_check_inner(event, false, None);
    }''','''    fn kp_stage_idle_conv_check(&mut self, event: &RawKeyEvent) {
        if matches!(event.event_type, KeyEventType::KeyDown)
            && event.ime_relevance.is_ime_mode_key
            && !event.injected
        {
            self.platform_state.gate.pending_modekey_event = Some(*event);
            self.platform
                .timer
                .set(crate::TIMER_MODEKEY_CONV, std::time::Duration::from_millis(%s));
        }
        let _ = self.kp_stage_idle_conv_check_inner(event, false, None);
    }

    /// E2: モードキーの後の遅延conv読み取り(ガードを無視して1回)。
    pub(crate) fn run_modekey_conv_check(&mut self) {
        let Some(mut ev) = self.platform_state.gate.pending_modekey_event.take() else {
            return;
        };
        ev.ime_relevance.is_ime_mode_key = false;
        self.platform_state.gate.force_conv_check = true;
        let _ = self.kp_stage_idle_conv_check_inner(&ev, false, None);
    }''' % delay)
    i=s.index('fn kp_stage_idle_conv_check_inner(')
    a='''        let output_idle_ms_at_spawn = self.platform.output_in_flight_ms();'''
    j=s.index(a,i)
    s=s[:j]+'''        let force = std::mem::take(&mut self.platform_state.gate.force_conv_check);
        let output_idle_ms_at_spawn = if force {
            u64::MAX
        } else {
            self.platform.output_in_flight_ms()
        };'''+s[j+len(a):]
    a='''        let explicit_age = self
            .platform_state
            .ime
            .explicit_ime_action_age_ms(now_tick_at_spawn);
        let is_tsf_native = self
            .platform
            .current_app_profile()
            .is_effectively_tsf_native(self.platform.focus.class_name());'''
    j=s.index(a,i)
    s=s[:j]+'''        let explicit_age = if force {
            u64::MAX
        } else {
            self.platform_state
                .ime
                .explicit_ime_action_age_ms(now_tick_at_spawn)
        };
        let is_tsf_native = force
            || self
                .platform
                .current_app_profile()
                .is_effectively_tsf_native(self.platform.focus.class_name());'''+s[j+len(a):]
    return s
edit('crates/awase-windows/src/runtime/key_pipeline.rs', kp)
# 4. タイマーハンドラ
edit('crates/awase-windows/src/runtime/message_handlers.rs', lambda s: rep1(s,'''        Some(id) if id == TIMER_POWER_RESUME => {''','''        Some(id) if id == crate::TIMER_MODEKEY_CONV => {
            app.platform.timer.kill(crate::TIMER_MODEKEY_CONV);
            app.run_modekey_conv_check();
        }
        Some(id) if id == TIMER_POWER_RESUME => {'''))
PY
