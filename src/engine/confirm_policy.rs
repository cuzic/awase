//! 確定モード（ConfirmPolicy）の実装。
//! NicolaFsm の idle_* メソッド群。
//!
//! Idle 状態で文字キーまたは親指キーが到着したとき、
//! `ConfirmMode` に応じて保留・投機出力・即時確定を選択する。

use smallvec::smallvec;

use crate::config::ConfirmMode;

use super::fsm_types::{
    ClassifiedEvent, Face, OutputUpdate, ParseAction, PendingKey, PendingThumbData, TimerIntent,
};
use super::nicola_fsm::{NicolaFsm, CONTINUOUS_KEYSTROKE_THRESHOLD_US};

impl NicolaFsm {
    /// 確定モードに応じた保留処理へディスパッチ
    pub(crate) fn dispatch_confirm_mode(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match self.confirm_mode {
            ConfirmMode::Wait => self.idle_wait(ev),
            // `AppConfig::validate_thresholds`（config.rs）がconfirm_mode=
            // "speculative"を必ずTwoPhase+delay=0へ正規化するため、この分岐は
            // 本番経路（bootstrap等、validate()を通ったconfigからNicolaFsmを
            // 構築する全経路）には到達しない。直接NicolaFsm::newを呼ぶ
            // テスト・後方互換の型としてのみ生存している（/code-review指摘、
            // PR #127、6回目）。
            ConfirmMode::Speculative => self.idle_speculative(ev),
            ConfirmMode::TwoPhase => self.idle_two_phase_or_speculative(ev),
            ConfirmMode::AdaptiveTiming => {
                let is_continuous = self
                    .last_key_gap_us
                    .is_some_and(|gap| gap < CONTINUOUS_KEYSTROKE_THRESHOLD_US);
                if is_continuous {
                    self.idle_wait(ev)
                } else {
                    self.idle_two_phase_or_speculative(ev)
                }
            }
            ConfirmMode::NgramPredictive => self.idle_ngram(ev),
        }
    }

    /// `idle_two_phase` へディスパッチする全箇所（`dispatch_confirm_mode` の
    /// `TwoPhase`/`AdaptiveTiming` 分岐、`idle_ngram` の n-gram モデル未読込
    /// フォールバック）が共通で使うヘルパー。
    ///
    /// `speculative_delay_us == 0` のときは `idle_two_phase` を経由せず
    /// `idle_speculative` へ直接ディスパッチする（/code-review指摘、
    /// PR #127）。`idle_two_phase` は `SpeculativeWait` タイマー
    /// （delay_us分）を張ってから投機出力するのに対し、`idle_speculative`
    /// は同一の呼び出し内で即座に出力して `SpeculativeChar` へ遷移する。
    /// 「delay=0なら待たずに直接遷移する」というこの判定自体はプラット
    /// フォーム非依存（ADR-019）で、どのOSでも意味のある最適化——
    /// タイマーの往復1回分、後続キーが `PendingChar` のまま処理される窓を
    /// 単純に無くすだけ。ただしこれを**必須**にした実測上の動機はWindows
    /// 固有: WindowsのSetTimerはUSER_TIMER_MINIMUM（10ms）未満に短縮
    /// されないため、delay_us=0を指定してもその間に届いた後続キーが
    /// `SpeculativeChar` ではなく `PendingChar` 状態で処理されてしまい、
    /// `confirm_mode="speculative"` 廃止時のTwoPhase(delay=0)正規化
    /// （`config.rs::validate_thresholds`）が主張する「完全に等価」が崩れて
    /// いた（他OSのタイマー実装がdelay=0を真に即時扱いするなら、この分岐は
    /// そちらでは理論上の最適化に留まり必須ではないが、無害かつ一貫した
    /// 挙動になる）。この判定を複数箇所に個別実装すると、将来どれか一箇所
    /// だけ更新し忘れるリスクがあるため一本化した。
    fn idle_two_phase_or_speculative(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if self.speculative_delay_us == 0 {
            self.idle_speculative(ev)
        } else {
            self.idle_two_phase(ev)
        }
    }

    /// Idle + Wait モード: 新規キーを保留状態に遷移させタイマーを起動する
    pub(crate) const fn idle_wait(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if ev.key_class.is_thumb() {
            self.enter_pending_thumb(PendingThumbData::from_event(ev));
        } else {
            self.enter_pending_char(PendingKey::from_event(ev));
        }
        ParseAction::Shift {
            timer: TimerIntent::Pending,
        }
    }

    /// Idle + Speculative モード: 文字キーは即時出力して SpeculativeChar へ遷移
    pub(crate) fn idle_speculative(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if self.phys.modifiers.shift {
            return self.idle_wait(ev);
        }
        if ev.key_class.is_thumb() {
            // Thumb key → same as Wait mode (pending thumb)
            return self.idle_wait(ev);
        }

        // Character key → immediately output normal face, enter SpeculativeChar
        let face = Face::Normal;
        if let Some((action, kana)) = self.lookup_face(ev.pos, self.get_face(face)) {
            self.enter_speculative_char(PendingKey::from_event(ev));
            // Output immediately + set timer for the threshold window
            ParseAction::Reduce {
                actions: smallvec![action.clone()],
                record: OutputUpdate::record(ev.scan_code, &action, kana),
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
        if self.phys.modifiers.shift {
            return self.idle_wait(ev);
        }
        if ev.key_class.is_thumb() {
            // Thumb keys use Wait mode (same as Speculative)
            return self.idle_wait(ev);
        }

        // Phase 1: Short wait (speculative_delay_us)
        // Same as Wait mode but with shorter timer
        self.enter_pending_char(PendingKey::from_event(ev));

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
        if self.phys.modifiers.shift {
            return self.idle_wait(ev);
        }
        if ev.key_class.is_thumb() {
            return self.idle_wait(ev);
        }

        // If no n-gram model, fall back to TwoPhase
        // （/code-review指摘、PR #127: delay=0のときはidle_speculativeと
        // 等価にするidle_two_phase_or_speculativeを使う。dispatch_confirm_mode
        // 参照）
        if self.ngram_model.is_none() {
            return self.idle_two_phase_or_speculative(ev);
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
    use super::super::fsm_types::{
        ClassifiedEvent, EngineState, KeyClass, ParseAction, TimerIntent,
    };
    use super::super::nicola_fsm::NicolaFsm;
    use super::super::test_support::*;
    use crate::config::ConfirmMode;
    use crate::scanmap::PhysicalPos;
    use crate::types::{ScanCode, VkCode};

    // ── Test fixtures specific to this module ────────────────────────

    /// A position that is NOT present in the layout faces.
    const POS_UNKNOWN: PhysicalPos = PhysicalPos::new(9, 9);

    fn make_fsm(mode: ConfirmMode) -> NicolaFsm {
        NicolaFsm::new(make_layout(), VK_NONCONVERT, VK_CONVERT, 100, mode, 30)
    }

    fn char_ev(vk: VkCode, scan: ScanCode, pos: Option<PhysicalPos>) -> ClassifiedEvent {
        ClassifiedEvent {
            key_class: KeyClass::Char,
            pos,
            scan_code: scan,
            vk_code: vk,
            timestamp: 0,
            is_ime_control: false,
            modifier_key: None,
        }
    }

    fn left_thumb_ev() -> ClassifiedEvent {
        ClassifiedEvent {
            key_class: KeyClass::LeftThumb,
            pos: None,
            scan_code: SCAN_NONCONVERT,
            vk_code: VK_NONCONVERT,
            timestamp: 0,
            is_ime_control: false,
            modifier_key: None,
        }
    }

    fn right_thumb_ev() -> ClassifiedEvent {
        ClassifiedEvent {
            key_class: KeyClass::RightThumb,
            pos: None,
            scan_code: SCAN_CONVERT,
            vk_code: VK_CONVERT,
            timestamp: 0,
            is_ime_control: false,
            modifier_key: None,
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
            matches!(
                action,
                ParseAction::PassThrough {
                    timer: TimerIntent::Keep
                }
            ),
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

    // ── dispatch_confirm_mode: TwoPhase(delay=0) ≡ Speculative ────────
    // /code-review指摘（PR #127）: confirm_mode="speculative"の廃止時、
    // config.rs::validate_thresholds は TwoPhase + speculative_delay_ms=0
    // へ正規化するが、その正規化後の値をNicolaFsmが実際にidle_speculativeと
    // 同じ振る舞いで処理することは(このディスパッチ分岐を追加するまで)
    // 未検証だった。idle_two_phase経由だとSpeculativeWaitタイマー
    // (Windows実機ではUSER_TIMER_MINIMUM=10ms未満に短縮されない)を挟むため、
    // その間に届いた後続キーがPendingChar状態で処理されてしまい、
    // 完全な等価にならない。dispatch_confirm_mode がdelay=0のとき
    // idle_speculativeへ直接ディスパッチするようになったことを固定する。

    fn make_fsm_with_delay(mode: ConfirmMode, speculative_delay_ms: u32) -> NicolaFsm {
        NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            mode,
            speculative_delay_ms,
        )
    }

    #[test]
    fn two_phase_zero_delay_dispatches_to_speculative_not_pending() {
        let mut fsm = make_fsm_with_delay(ConfirmMode::TwoPhase, 0);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Reduce { .. }),
            "TwoPhase(delay=0) は idle_speculative と同じく即時Reduceになるべき、\
             got {action:?}"
        );
        assert!(
            matches!(fsm.state, EngineState::SpeculativeChar(_)),
            "TwoPhase(delay=0) は idle_two_phase 経由のPendingCharではなく\
             SpeculativeCharへ直接遷移するべき"
        );
    }

    #[test]
    fn two_phase_nonzero_delay_still_uses_pending_char_path() {
        // delay=0 専用の分岐が、通常のTwoPhase（delay>0）を巻き込んで
        // いないことを確認する。
        let mut fsm = make_fsm_with_delay(ConfirmMode::TwoPhase, 30);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Shift { timer } if timer_is_speculative_wait(&timer)),
            "TwoPhase(delay>0) は従来通りSpeculativeWaitタイマー付きShiftのはず、\
             got {action:?}"
        );
        assert!(
            matches!(fsm.state, EngineState::PendingChar(_)),
            "TwoPhase(delay>0) は従来通りPendingCharへ遷移するべき"
        );
    }

    #[test]
    fn two_phase_zero_delay_matches_speculative_reduce_action() {
        // 正規化の主張（TwoPhase(delay=0) と Speculative は等価）そのものを、
        // 同一入力に対する dispatch_confirm_mode の出力比較で直接固定する。
        let mut fsm_speculative = make_fsm_with_delay(ConfirmMode::Speculative, 0);
        let mut fsm_two_phase_zero = make_fsm_with_delay(ConfirmMode::TwoPhase, 0);
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));

        let action_speculative = fsm_speculative.dispatch_confirm_mode(&ev);
        let action_two_phase_zero = fsm_two_phase_zero.dispatch_confirm_mode(&ev);

        assert_eq!(
            format!("{action_speculative:?}"),
            format!("{action_two_phase_zero:?}"),
            "Speculative と TwoPhase(delay=0) は同一入力に対して同一の\
             ParseAction を返すべき（正規化の等価性主張そのものの回帰テスト）"
        );
        assert_eq!(
            std::mem::discriminant(&fsm_speculative.state),
            std::mem::discriminant(&fsm_two_phase_zero.state),
            "Speculative と TwoPhase(delay=0) は同一の遷移先状態を持つべき"
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
    fn ngram_predictive_no_model_zero_delay_falls_back_to_speculative_not_two_phase() {
        // /code-review指摘（PR #127、2回目）: idle_ngramのno-modelフォール
        // バックはidle_two_phaseを直接呼んでおり、dispatch_confirm_modeの
        // TwoPhase/AdaptiveTiming分岐にだけ追加したdelay=0バイパスが
        // 効いていなかった。idle_two_phase_or_speculativeへの一本化で
        // このパスも救われることを固定する。
        let mut fsm = make_fsm_with_delay(ConfirmMode::NgramPredictive, 0);
        assert!(fsm.ngram_model.is_none());
        let ev = char_ev(VK_A, SCAN_A, Some(POS_A));
        let action = fsm.dispatch_confirm_mode(&ev);
        assert!(
            matches!(action, ParseAction::Reduce { .. }),
            "NgramPredictive without model, delay=0 は idle_speculative と\
             同じく即時Reduceになるべき、got {action:?}"
        );
        assert!(
            matches!(fsm.state, EngineState::SpeculativeChar(_)),
            "NgramPredictive without model, delay=0 は SpeculativeChar へ\
             直接遷移するべき"
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
