#!/usr/bin/env bash
# E4(BUG-149、Opusレビュー案A): 新タイマーを足さず、既存の「パススルーしたmay_change_imeキーの20ms後refresh」を一般化する。
#  - arm: パススルーした(=FSMがConsumeしていない)モードキーで、awaseが自分でactuateしないもの(shadow_action/sync_directionなし)
#  - refreshの中(ir_stage_observe)で、conv読み取り(idle-conv-check)を強制1回。Shift関連ガード中は100ms後のrefreshで再試行。
DELAY="${E4_DELAY:-150}"
python3 - "$DELAY" <<'PY'
import sys
delay=sys.argv[1]
def edit(p,f):
    s=open(p,encoding='utf8').read(); s=f(s); open(p,'w',encoding='utf8').write(s)
def rep1(s,a,b):
    assert s.count(a)==1,(a,s.count(a)); return s.replace(a,b)
# 1. gate
def gate(s):
    s=rep1(s,'    pub idle_conv_check_in_flight_since_ms: Option<u64>,','    pub idle_conv_check_in_flight_since_ms: Option<u64>,\n    pub force_conv_check: bool,\n    pub pending_modekey_event: Option<awase::types::RawKeyEvent>,')
    return rep1(s,'            idle_conv_check_in_flight_since_ms: None,','            idle_conv_check_in_flight_since_ms: None,\n            force_conv_check: false,\n            pending_modekey_event: None,')
edit('crates/awase-windows/src/state/platform_state.rs', gate)
# 2. key_pipeline: arm と inner の force
def kp(s):
    s=rep1(s,'''        if !decision.is_consumed()
            && event.ime_relevance.may_change_ime
            && matches!(event.event_type, KeyEventType::KeyDown)
        {
            self.schedule_ime_refresh(20);''','''        let follow_key = !decision.is_consumed()
            && matches!(event.event_type, KeyEventType::KeyDown)
            && event.ime_relevance.is_ime_mode_key
            && event.ime_relevance.shadow_action.is_none()
            && event.ime_relevance.sync_direction.is_none();
        if follow_key {
            self.platform_state.gate.pending_modekey_event = Some(*event);
            self.schedule_ime_refresh(%s);
            tracing::debug!("[E4] mode key passed through → follow refresh scheduled");
        } else if !decision.is_consumed()
            && event.ime_relevance.may_change_ime
            && matches!(event.event_type, KeyEventType::KeyDown)
        {
            self.schedule_ime_refresh(20);''' % delay)
    s=rep1(s,'    fn kp_stage_idle_conv_check_inner(','    pub(crate) fn kp_stage_idle_conv_check_inner(')
    i=s.index('pub(crate) fn kp_stage_idle_conv_check_inner(')
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
# 3. refresh の中で強制conv読み取り
edit('crates/awase-windows/src/runtime/ime_refresh.rs', lambda s: rep1(s,'''        match strategy {
            ImeReadStrategy::SkipTyping => {}''','''        if self.platform_state.gate.pending_modekey_event.is_some() {
            if self.platform_state.gate.half_width_alnum.is_guard_pending()
                || self.platform_state.gate.half_width_alnum.is_toggle_active()
            {
                tracing::debug!("[E4] shift関連ガード中 → 100ms後のrefreshで再試行");
                self.schedule_ime_refresh(100);
            } else if let Some(mut ev) = self.platform_state.gate.pending_modekey_event.take() {
                ev.ime_relevance.is_ime_mode_key = false;
                self.platform_state.gate.force_conv_check = true;
                tracing::debug!("[E4] refresh内で強制conv読み取りを実行");
                let _ = self.kp_stage_idle_conv_check_inner(&ev, false, None);
            }
        }
        match strategy {
            ImeReadStrategy::SkipTyping => {}'''))
PY
