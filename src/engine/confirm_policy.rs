//! 確定モード（ConfirmPolicy）の実装。
//! NicolaFsm の idle_* メソッド群。
//!
//! Idle 状態で文字キーまたは親指キーが到着したとき、
//! `ConfirmMode` に応じて保留・投機出力・即時確定を選択する。

use crate::config::ConfirmMode;

use super::fsm_types::{
    ClassifiedEvent, Face, ParseAction, PendingKey, PendingThumbData, TimerIntent,
};
use super::nicola_fsm::{record_output, NicolaFsm, CONTINUOUS_KEYSTROKE_THRESHOLD_US};

impl NicolaFsm {
    /// 確定モードに応じた保留処理へディスパッチ
    pub(crate) fn dispatch_confirm_mode(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match self.confirm_mode {
            ConfirmMode::Wait => self.idle_wait(ev),
            ConfirmMode::Speculative => self.idle_speculative(ev),
            ConfirmMode::TwoPhase => self.idle_two_phase(ev),
            ConfirmMode::AdaptiveTiming => {
                let is_continuous = self
                    .last_key_gap_us
                    .is_some_and(|gap| gap < CONTINUOUS_KEYSTROKE_THRESHOLD_US);
                if is_continuous {
                    self.idle_wait(ev)
                } else {
                    self.idle_two_phase(ev)
                }
            }
            ConfirmMode::NgramPredictive => self.idle_ngram(ev),
        }
    }

    /// Idle + Wait モード: 新規キーを保留状態に遷移させタイマーを起動する
    pub(crate) const fn idle_wait(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if ev.key_class.is_thumb() {
            self.enter_pending_thumb(PendingThumbData {
                scan_code: ev.scan_code,
                vk_code: ev.vk_code,
                is_left: ev.key_class.is_left_thumb(),
                timestamp: ev.timestamp,
            });
        } else {
            self.enter_pending_char(PendingKey {
                scan_code: ev.scan_code,
                vk_code: ev.vk_code,
                pos: ev.pos,
                timestamp: ev.timestamp,
            });
        }
        ParseAction::Shift {
            timer: TimerIntent::Pending,
        }
    }

    /// Idle + Speculative モード: 文字キーは即時出力して SpeculativeChar へ遷移
    pub(crate) fn idle_speculative(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if ev.key_class.is_thumb() {
            // Thumb key → same as Wait mode (pending thumb)
            return self.idle_wait(ev);
        }

        // Character key → immediately output normal face, enter SpeculativeChar
        let face = Face::Normal;
        if let Some((action, kana)) = self.lookup_face(ev.pos, self.get_face(face)) {
            self.enter_speculative_char(PendingKey {
                scan_code: ev.scan_code,
                vk_code: ev.vk_code,
                pos: ev.pos,
                timestamp: ev.timestamp,
            });
            // Output immediately + set timer for the threshold window
            ParseAction::Reduce {
                actions: vec![action.clone()],
                record: record_output(ev.scan_code, &action, kana),
                timer: TimerIntent::Pending,
            }
        } else {
            ParseAction::PassThrough {
                timer: TimerIntent::Keep,
            }
        }
    }

    /// Idle + TwoPhase モード: Phase 1 は短い待機、Phase 2 は投機出力
    ///
    /// 親指キーは Wait モードと同じ扱い。
    /// 文字キーは短い待機（speculative_delay_us）の後、投機出力に遷移する。
    pub(crate) const fn idle_two_phase(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if ev.key_class.is_thumb() {
            // Thumb keys use Wait mode (same as Speculative)
            return self.idle_wait(ev);
        }

        // Phase 1: Short wait (speculative_delay_us)
        // Same as Wait mode but with shorter timer
        self.enter_pending_char(PendingKey {
            scan_code: ev.scan_code,
            vk_code: ev.vk_code,
            pos: ev.pos,
            timestamp: ev.timestamp,
        });

        // Use TIMER_SPECULATIVE with the short delay
        ParseAction::Shift {
            timer: TimerIntent::SpeculativeWait,
        }
    }

    /// Idle + NgramPredictive モード: n-gram スコアで投機/待機を動的切替
    ///
    /// 親指キーは Wait モードと同じ扱い。
    /// 文字キーは通常面と親指面の n-gram スコアを比較し、
    /// 通常面が明らかに有利なら Speculative、そうでなければ Wait。
    pub(crate) fn idle_ngram(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if ev.key_class.is_thumb() {
            return self.idle_wait(ev);
        }

        // If no n-gram model, fall back to TwoPhase
        if self.ngram_model.is_none() {
            return self.idle_two_phase(ev);
        }

        // Get candidate kana for each face
        let normal_kana = self
            .lookup_face(ev.pos, self.get_face(Face::Normal))
            .and_then(|(_, kana)| kana);
        let left_kana = self
            .lookup_face(ev.pos, self.get_face(Face::LeftThumb))
            .and_then(|(_, kana)| kana);
        let right_kana = self
            .lookup_face(ev.pos, self.get_face(Face::RightThumb))
            .and_then(|(_, kana)| kana);

        // Decision: if normal is clearly more likely, output speculatively
        let judge = self.timing_judge();
        if judge.should_speculate(normal_kana, left_kana, right_kana) {
            self.idle_speculative(ev)
        } else {
            self.idle_wait(ev)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fsm_types::{EngineState, KeyClass, ParseAction, TimerIntent};
    use super::super::nicola_fsm::NicolaFsm;
    use crate::config::ConfirmMode;
    use crate::scanmap::PhysicalPos;
    use crate::types::{ScanCode, VkCode};
    use crate::yab::{YabFace, YabLayout, YabValue};

    // ── Test fixtures ────────────────────────────────────────────────

    const VK_A: VkCode = VkCode(0x41);
    const VK_S: VkCode = VkCode(0x53);
    const VK_NONCONVERT: VkCode = VkCode(0x1D);
    const VK_CONVERT: VkCode = VkCode(0x1C);

    const SCAN_A: ScanCode = ScanCode(0x1E);
    const SCAN_S: ScanCode = ScanCode(0x1F);
    const SCAN_NONCONVERT: ScanCode = ScanCode(0x7B);
    const SCAN_CONVERT: ScanCode = ScanCode(0x79);

    const POS_A: PhysicalPos = PhysicalPos::new(2, 0);
    const POS_S: PhysicalPos = PhysicalPos::new(2, 1);
    /// A position that is NOT present in the layout faces.
    const POS_UNKNOWN: PhysicalPos = PhysicalPos::new(9, 9);

    fn lit(ch: char) -> YabValue {
        YabValue::Literal(ch.to_string())
    }

    fn make_layout() -> YabLayout {
        let mut normal = YabFace::new();
        normal.insert(POS_A, lit('う'));
        normal.insert(POS_S, lit('し'));

        let mut left_thumb = YabFace::new();
        left_thumb.insert(POS_A, lit('を'));
        left_thumb.insert(POS_S, lit('あ'));

        let mut right_thumb = YabFace::new();
        right_thumb.insert(POS_A, lit('ゔ'));
        right_thumb.insert(POS_S, lit('じ'));

        YabLayout {
            name: String::from("test"),
            normal,
            left_thumb,
            right_thumb,
            shift: YabFace::new(),
        }
    }

    fn make_fsm(mode: ConfirmMode) -> NicolaFsm {
        NicolaFsm::new(make_layout(), VK_NONCONVERT, VK_CONVERT, 100, mode, 30)
    }

    /// Build a `ClassifiedEvent` for a regular character key.
    fn char_ev(vk: VkCode, scan: ScanCode, pos: Option<PhysicalPos>) -> super::super::fsm_types::ClassifiedEvent {
        super::super::fsm_types::ClassifiedEvent {
            key_class: KeyClass::Char,
            pos,
            scan_code: scan,
            vk_code: vk,
            timestamp: 0,
            is_ime_control: false,
        }
    }

    /// Build a `ClassifiedEvent` for a left-thumb key.
    fn left_thumb_ev() -> super::super::fsm_types::ClassifiedEvent {
        super::super::fsm_types::ClassifiedEvent {
            key_class: KeyClass::LeftThumb,
            pos: None,
            scan_code: SCAN_NONCONVERT,
            vk_code: VK_NONCONVERT,
            timestamp: 0,
            is_ime_control: false,
        }
    }

    /// Build a `ClassifiedEvent` for a right-thumb key.
    fn right_thumb_ev() -> super::super::fsm_types::ClassifiedEvent {
        super::super::fsm_types::ClassifiedEvent {
            key_class: KeyClass::RightThumb,
            pos: None,
            scan_code: SCAN_CONVERT,
            vk_code: VK_CONVERT,
            timestamp: 0,
            is_ime_control: false,
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────

    fn timer_is_pending(intent: &TimerIntent) -> bool {
        matches!(intent, TimerIntent::Pending)
    }

    fn timer_is_speculative_wait(intent: &TimerIntent) -> bool {
        matches!(intent, TimerIntent::SpeculativeWait)
    }

    // ── idle_wait ────────────────────────────────────────────────────

    #[test]
    fn wait_char_key_shifts_with_pending_timer() {
        let mut fsm = make_fsm(ConfirmMode::Wait);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.idle_wait(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "Wait + char key should Shift with Pending timer, got {action:?}"
        );
    }

    #[test]
    fn wait_char_key_enters_pending_char_state() {
        let mut fsm = make_fsm(ConfirmMode::Wait);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        fsm.idle_wait(&ev);
        assert!(
            matches!(fsm.state, EngineState::PendingChar(_)),
            "state should be PendingChar after idle_wait with char key"
        );
    }

    #[test]
    fn wait_left_thumb_key_shifts_with_pending_timer() {
        let mut fsm = make_fsm(ConfirmMode::Wait);
        let ev = left_thumb_ev();
        let action = fsm.idle_wait(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "Wait + left-thumb key should Shift with Pending timer"
        );
    }

    #[test]
    fn wait_left_thumb_key_enters_pending_thumb_state() {
        let mut fsm = make_fsm(ConfirmMode::Wait);
        let ev = left_thumb_ev();
        fsm.idle_wait(&ev);
        assert!(
            matches!(fsm.state, EngineState::PendingThumb(_)),
            "state should be PendingThumb after idle_wait with left-thumb key"
        );
    }

    #[test]
    fn wait_right_thumb_key_enters_pending_thumb_state() {
        let mut fsm = make_fsm(ConfirmMode::Wait);
        let ev = right_thumb_ev();
        fsm.idle_wait(&ev);
        assert!(
            matches!(fsm.state, EngineState::PendingThumb(_)),
            "state should be PendingThumb after idle_wait with right-thumb key"
        );
    }

    // ── idle_speculative ─────────────────────────────────────────────

    #[test]
    fn speculative_char_key_in_layout_reduces_immediately() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.idle_speculative(&ev);
        assert!(
            matches!(action, ParseAction::Reduce { .. }),
            "Speculative + layout char key should Reduce immediately, got {action:?}"
        );
    }

    #[test]
    fn speculative_char_key_reduce_timer_is_pending() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.idle_speculative(&ev);
        if let ParseAction::Reduce { timer, .. } = action {
            assert!(
                timer_is_pending(&timer),
                "Speculative Reduce should carry Pending timer"
            );
        } else {
            panic!("expected Reduce, got {action:?}");
        }
    }

    #[test]
    fn speculative_char_key_enters_speculative_char_state() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        fsm.idle_speculative(&ev);
        assert!(
            matches!(fsm.state, EngineState::SpeculativeChar(_)),
            "state should be SpeculativeChar after speculative output"
        );
    }

    #[test]
    fn speculative_char_key_not_in_layout_passes_through() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        // pos not present in any face → lookup_face returns None
        let ev = char_ev(VK_A, SCAN_A, Some(POS_UNKNOWN));
        let action = fsm.idle_speculative(&ev);
        assert!(
            matches!(action, ParseAction::PassThrough { timer: TimerIntent::Keep }),
            "Speculative + unknown pos should PassThrough(Keep), got {action:?}"
        );
    }

    #[test]
    fn speculative_char_key_with_none_pos_passes_through() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        let ev = char_ev(VK_A, SCAN_A, None);
        let action = fsm.idle_speculative(&ev);
        assert!(
            matches!(action, ParseAction::PassThrough { .. }),
            "Speculative + None pos should PassThrough, got {action:?}"
        );
    }

    #[test]
    fn speculative_left_thumb_key_delegates_to_wait() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        let ev = left_thumb_ev();
        let action = fsm.idle_speculative(&ev);
        // Thumb keys in Speculative mode should behave exactly like Wait mode.
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "Speculative + left-thumb should fall back to Wait (Shift+Pending), got {action:?}"
        );
        assert!(matches!(fsm.state, EngineState::PendingThumb(_)));
    }

    #[test]
    fn speculative_right_thumb_key_delegates_to_wait() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        let ev = right_thumb_ev();
        let action = fsm.idle_speculative(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "Speculative + right-thumb should fall back to Wait, got {action:?}"
        );
    }

    // ── idle_two_phase ───────────────────────────────────────────────

    #[test]
    fn two_phase_char_key_shifts_with_speculative_wait_timer() {
        let mut fsm = make_fsm(ConfirmMode::TwoPhase);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.idle_two_phase(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_speculative_wait(&timer)),
            "TwoPhase + char key should Shift with SpeculativeWait timer, got {action:?}"
        );
    }

    #[test]
    fn two_phase_char_key_enters_pending_char_state() {
        let mut fsm = make_fsm(ConfirmMode::TwoPhase);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        fsm.idle_two_phase(&ev);
        assert!(
            matches!(fsm.state, EngineState::PendingChar(_)),
            "TwoPhase + char key should enter PendingChar"
        );
    }

    #[test]
    fn two_phase_left_thumb_key_delegates_to_wait() {
        let mut fsm = make_fsm(ConfirmMode::TwoPhase);
        let ev = left_thumb_ev();
        let action = fsm.idle_two_phase(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "TwoPhase + left-thumb should fall back to Wait (Shift+Pending), got {action:?}"
        );
        assert!(matches!(fsm.state, EngineState::PendingThumb(_)));
    }

    #[test]
    fn two_phase_right_thumb_key_delegates_to_wait() {
        let mut fsm = make_fsm(ConfirmMode::TwoPhase);
        let ev = right_thumb_ev();
        let action = fsm.idle_two_phase(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "TwoPhase + right-thumb should fall back to Wait, got {action:?}"
        );
    }

    // ── dispatch_confirm_mode: AdaptiveTiming ────────────────────────

    #[test]
    fn adaptive_timing_no_gap_is_wait() {
        // last_key_gap_us is None → not continuous → TwoPhase path
        let mut fsm = make_fsm(ConfirmMode::AdaptiveTiming);
        assert!(fsm.last_key_gap_us.is_none());
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        // TwoPhase path for char key: SpeculativeWait
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_speculative_wait(&timer)),
            "AdaptiveTiming with no gap should behave like TwoPhase, got {action:?}"
        );
    }

    #[test]
    fn adaptive_timing_continuous_gap_is_wait() {
        // gap < threshold → continuous → Wait path
        let mut fsm = make_fsm(ConfirmMode::AdaptiveTiming);
        fsm.last_key_gap_us = Some(50_000); // 50 ms < 80 ms threshold
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "AdaptiveTiming with continuous gap should behave like Wait, got {action:?}"
        );
    }

    #[test]
    fn adaptive_timing_slow_gap_is_two_phase() {
        // gap >= threshold → not continuous → TwoPhase path
        let mut fsm = make_fsm(ConfirmMode::AdaptiveTiming);
        fsm.last_key_gap_us = Some(200_000); // 200 ms > 80 ms threshold
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_speculative_wait(&timer)),
            "AdaptiveTiming with slow gap should behave like TwoPhase, got {action:?}"
        );
    }

    #[test]
    fn adaptive_timing_exactly_at_threshold_is_two_phase() {
        use super::super::nicola_fsm::CONTINUOUS_KEYSTROKE_THRESHOLD_US;
        // gap == threshold is NOT < threshold → not continuous → TwoPhase path
        let mut fsm = make_fsm(ConfirmMode::AdaptiveTiming);
        fsm.last_key_gap_us = Some(CONTINUOUS_KEYSTROKE_THRESHOLD_US);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_speculative_wait(&timer)),
            "AdaptiveTiming at exact threshold should use TwoPhase, got {action:?}"
        );
    }

    #[test]
    fn adaptive_timing_just_below_threshold_is_wait() {
        use super::super::nicola_fsm::CONTINUOUS_KEYSTROKE_THRESHOLD_US;
        let mut fsm = make_fsm(ConfirmMode::AdaptiveTiming);
        fsm.last_key_gap_us = Some(CONTINUOUS_KEYSTROKE_THRESHOLD_US - 1);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "AdaptiveTiming just below threshold should use Wait, got {action:?}"
        );
    }

    // ── dispatch_confirm_mode: NgramPredictive ────────────────────────

    #[test]
    fn ngram_predictive_no_model_falls_back_to_two_phase() {
        let mut fsm = make_fsm(ConfirmMode::NgramPredictive);
        assert!(fsm.ngram_model.is_none());
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        // Without a model → TwoPhase path for char key → SpeculativeWait
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_speculative_wait(&timer)),
            "NgramPredictive without model should fall back to TwoPhase, got {action:?}"
        );
    }

    #[test]
    fn ngram_predictive_thumb_key_delegates_to_wait() {
        let mut fsm = make_fsm(ConfirmMode::NgramPredictive);
        let ev = left_thumb_ev();
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "NgramPredictive + thumb key should always use Wait, got {action:?}"
        );
        assert!(matches!(fsm.state, EngineState::PendingThumb(_)));
    }

    // ── dispatch_confirm_mode: all modes dispatch to correct handler ──

    #[test]
    fn dispatch_wait_mode_char_key() {
        let mut fsm = make_fsm(ConfirmMode::Wait);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_pending(&timer)),
            "dispatch Wait mode char key should give Pending timer"
        );
    }

    #[test]
    fn dispatch_speculative_mode_char_key() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Reduce { .. }),
            "dispatch Speculative mode char key should Reduce immediately"
        );
    }

    #[test]
    fn dispatch_two_phase_mode_char_key() {
        let mut fsm = make_fsm(ConfirmMode::TwoPhase);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_speculative_wait(&timer)),
            "dispatch TwoPhase mode char key should give SpeculativeWait timer"
        );
    }

    // ── speculative output contains the correct kana action ──────────

    #[test]
    fn speculative_reduce_emits_correct_action_for_pos_a() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.idle_speculative(&ev);
        if let ParseAction::Reduce { actions, .. } = action {
            assert_eq!(actions.len(), 1);
            assert!(
                matches!(&actions[0], crate::types::KeyAction::Char('う')),
                "POS_A normal face should output 'う', got {:?}",
                actions[0]
            );
        } else {
            panic!("expected Reduce, got {action:?}");
        }
    }

    #[test]
    fn speculative_reduce_emits_correct_action_for_pos_s() {
        let mut fsm = make_fsm(ConfirmMode::Speculative);
        let ev = char_ev(VK_S, SCAN_S, Some(POS_S));
        let action = fsm.idle_speculative(&ev);
        if let ParseAction::Reduce { actions, .. } = action {
            assert_eq!(actions.len(), 1);
            assert!(
                matches!(&actions[0], crate::types::KeyAction::Char('し')),
                "POS_S normal face should output 'し', got {:?}",
                actions[0]
            );
        } else {
            panic!("expected Reduce, got {action:?}");
        }
    }

    // ── PendingKey / PendingThumbData fields are populated correctly ──

    #[test]
    fn wait_char_pending_key_fields_match_event() {
        let mut fsm = make_fsm(ConfirmMode::Wait);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        fsm.idle_wait(&ev);
        if let EngineState::PendingChar(pk) = fsm.state {
            assert_eq!(pk.scan_code, SCAN_A);
            assert_eq!(pk.vk_code, VK_A);
            assert_eq!(pk.pos, Some(POS_A));
        } else {
            panic!("expected PendingChar");
        }
    }

    #[test]
    fn wait_thumb_pending_thumb_data_fields_match_event() {
        let mut fsm = make_fsm(ConfirmMode::Wait);
        let ev = left_thumb_ev();
        fsm.idle_wait(&ev);
        if let EngineState::PendingThumb(td) = fsm.state {
            assert_eq!(td.vk_code, VK_NONCONVERT);
            assert!(td.is_left, "left thumb should set is_left = true");
        } else {
            panic!("expected PendingThumb");
        }
    }

    #[test]
    fn wait_right_thumb_pending_thumb_data_is_right() {
        let mut fsm = make_fsm(ConfirmMode::Wait);
        let ev = right_thumb_ev();
        fsm.idle_wait(&ev);
        if let EngineState::PendingThumb(td) = fsm.state {
            assert!(!td.is_left, "right thumb should set is_left = false");
        } else {
            panic!("expected PendingThumb");
        }
    }
}
