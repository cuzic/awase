//! NicolaFsm と Decision/Effect 層の橋渡し。
//! timed-fsm の Response を Effect/Decision に変換する。

use timed_fsm::{Response, TimerCommand};

use crate::config::ConfirmMode;
use crate::ngram::NgramModel;
use crate::types::{ContextChange, KeyAction, RawKeyEvent};
use crate::yab::YabLayout;

use super::decision::{Decision, Effect, EffectVec, InputEffect, TimerEffect};
use super::input_tracker::PhysicalKeyState;
use super::nicola_fsm::NicolaFsm;

/// NicolaFsm と Decision/Effect 層の橋渡し。
/// timed-fsm の Response を Effect/Decision に変換する。
#[allow(missing_debug_implementations)]
pub struct FsmAdapter {
    fsm: NicolaFsm,
}

impl FsmAdapter {
    /// 新しい `FsmAdapter` を作成する。
    #[must_use]
    pub const fn new(fsm: NicolaFsm) -> Self {
        Self { fsm }
    }

    /// キーイベントを処理し、Decision を返す。
    pub fn on_event(&mut self, event: RawKeyEvent, phys: &PhysicalKeyState) -> Decision {
        let resp = self.fsm.on_event(event, phys);
        Self::response_to_decision(resp)
    }

    /// タイマー満了時の処理。
    pub fn on_timeout(&mut self, timer_id: usize, phys: &PhysicalKeyState) -> Decision {
        let resp = self.fsm.on_timeout(timer_id, phys);
        Self::response_to_decision(resp)
    }

    /// 保留中のキーをフラッシュし、Decision を返す。
    pub fn flush(&mut self, reason: ContextChange) -> Decision {
        let resp = self.fsm.flush_pending(reason);
        Self::response_to_decision(resp)
    }

    /// フラッシュして Effect リストのみを返す（他の Effect と結合する用途）。
    pub fn flush_to_effects(&mut self, reason: ContextChange) -> EffectVec {
        let resp = self.fsm.flush_pending(reason);
        Self::response_to_effects(resp)
    }

    /// エンジンの有効/無効をトグルする。
    pub fn toggle_enabled(&mut self) -> (bool, Decision) {
        let (enabled, resp) = self.fsm.toggle_enabled();
        (enabled, Self::response_to_decision(resp))
    }

    /// エンジンの有効/無効を明示的に設定する。
    pub fn set_enabled(&mut self, enabled: bool) -> (bool, Decision) {
        let (actual, resp) = self.fsm.set_enabled(enabled);
        (actual, Self::response_to_decision(resp))
    }

    /// 配列を動的に差し替える。
    pub fn swap_layout(&mut self, layout: YabLayout) -> Decision {
        let resp = self.fsm.swap_layout(layout);
        Self::response_to_decision(resp)
    }

    /// 同時打鍵判定の閾値を更新する（ミリ秒指定）。
    pub fn set_threshold_ms(&mut self, ms: u32) {
        self.fsm.set_threshold_ms(ms);
    }

    /// 確定モードと投機出力の待機時間を更新する。
    pub fn set_confirm_mode(&mut self, mode: ConfirmMode, delay_ms: u32) {
        self.fsm.set_confirm_mode(mode, delay_ms);
    }

    /// n-gram モデルを設定する。
    pub fn set_ngram_model(&mut self, model: NgramModel) {
        self.fsm.set_ngram_model(model);
    }

    /// エンジンが有効かどうかを返す。
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.fsm.is_enabled()
    }

    // ── 内部メソッド ──

    /// timed-fsm Response → Effect リストに変換（consumed フラグは呼び出し側で判定）
    fn response_to_effects(resp: Response<KeyAction, usize>) -> EffectVec {
        let mut effects = EffectVec::new();
        for cmd in &resp.timers {
            match cmd {
                TimerCommand::Set { id, duration } => {
                    effects.push(Effect::Timer(TimerEffect::Set {
                        id: *id,
                        duration: *duration,
                    }));
                }
                TimerCommand::Kill { id } => {
                    effects.push(Effect::Timer(TimerEffect::Kill(*id)));
                }
            }
        }
        if !resp.actions.is_empty() {
            effects.push(Effect::Input(InputEffect::SendKeys(resp.actions)));
        }
        effects
    }

    /// timed-fsm Response → Decision に変換（Response を消費する）
    fn response_to_decision(resp: Response<KeyAction, usize>) -> Decision {
        let consumed = resp.consumed;
        let effects = Self::response_to_effects(resp);
        if consumed {
            Decision::consumed_with(effects)
        } else if effects.is_empty() {
            Decision::pass_through()
        } else {
            Decision::pass_through_with(effects)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use timed_fsm::Response;

    use crate::config::ConfirmMode;
    use crate::engine::decision::{Decision, Effect, InputEffect, TimerEffect};
    use crate::engine::input_tracker::{InputTracker, PhysicalKeyState};
    use crate::engine::nicola_fsm::NicolaFsm;
    use crate::ngram::NgramModel;
    use crate::scanmap::PhysicalPos;
    use crate::types::{
        ContextChange, ImeRelevance, KeyAction, KeyClassification, KeyEventType, RawKeyEvent,
        ScanCode, VkCode,
    };
    use crate::yab::{YabFace, YabLayout, YabValue};

    use super::FsmAdapter;

    // ── VK / Scan 定数 ──────────────────────────────────────────────────────

    const VK_A: VkCode = VkCode(0x41);
    const VK_NONCONVERT: VkCode = VkCode(0x1D); // 無変換（左親指）
    const VK_CONVERT: VkCode = VkCode(0x1C);   // 変換（右親指）
    const VK_RETURN: VkCode = VkCode(0x0D);    // パススルーキー

    const SCAN_A: ScanCode = ScanCode(0x1E);
    const SCAN_NONCONVERT: ScanCode = ScanCode(0x7B);
    const SCAN_RETURN: ScanCode = ScanCode(0x1C);

    const POS_A: PhysicalPos = PhysicalPos::new(2, 0);
    const POS_S: PhysicalPos = PhysicalPos::new(2, 1);

    // ── ヘルパー関数 ────────────────────────────────────────────────────────

    fn lit(ch: char) -> YabValue {
        YabValue::Literal(ch.to_string())
    }

    /// テスト用の最小限の YabLayout を構築する。
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

    /// テスト用の FsmAdapter (Wait モード) を生成する。
    fn make_adapter() -> FsmAdapter {
        let fsm = NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        );
        FsmAdapter::new(fsm)
    }

    /// VkCode / ScanCode / KeyClassification / PhysicalPos のマッピングからイベントを生成する。
    fn make_event(
        vk: VkCode,
        scan: ScanCode,
        event_type: KeyEventType,
        classification: KeyClassification,
        pos: Option<PhysicalPos>,
    ) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: vk,
            scan_code: scan,
            event_type,
            extra_info: 0,
            timestamp: 0,
            key_classification: classification,
            physical_pos: pos,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
        }
    }

    fn key_down_char(vk: VkCode, scan: ScanCode, pos: PhysicalPos) -> RawKeyEvent {
        make_event(vk, scan, KeyEventType::KeyDown, KeyClassification::Char, Some(pos))
    }

    fn key_down_passthrough(vk: VkCode, scan: ScanCode) -> RawKeyEvent {
        make_event(vk, scan, KeyEventType::KeyDown, KeyClassification::Passthrough, None)
    }

    fn key_down_left_thumb() -> RawKeyEvent {
        make_event(
            VK_NONCONVERT,
            SCAN_NONCONVERT,
            KeyEventType::KeyDown,
            KeyClassification::LeftThumb,
            None,
        )
    }

    fn key_up_left_thumb() -> RawKeyEvent {
        make_event(
            VK_NONCONVERT,
            SCAN_NONCONVERT,
            KeyEventType::KeyUp,
            KeyClassification::LeftThumb,
            None,
        )
    }

    /// テスト用の InputTracker スナップショットを生成する（空の物理キー状態）。
    fn empty_phys() -> PhysicalKeyState {
        PhysicalKeyState::empty()
    }

    // ── response_to_decision の各分岐テスト ─────────────────────────────────

    /// consumed=true, effects 無し → Consume { effects: [] }
    #[test]
    fn response_to_decision_consumed_no_effects() {
        let resp: Response<KeyAction, usize> = Response::consume();
        let decision = FsmAdapter::response_to_decision(resp);
        assert!(decision.is_consumed(), "consumed response should yield Consume decision");
        match decision {
            Decision::Consume { effects } => assert!(effects.is_empty()),
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    /// consumed=true, タイマー Set あり → Consume { effects に TimerEffect::Set }
    #[test]
    fn response_to_decision_consumed_with_timer_set() {
        let dur = Duration::from_millis(50);
        let resp: Response<KeyAction, usize> =
            Response::consume().with_timer(1usize, dur);
        let decision = FsmAdapter::response_to_decision(resp);
        assert!(decision.is_consumed());
        match decision {
            Decision::Consume { effects } => {
                assert_eq!(effects.len(), 1);
                match &effects[0] {
                    Effect::Timer(TimerEffect::Set { id, duration }) => {
                        assert_eq!(*id, 1);
                        assert_eq!(*duration, dur);
                    }
                    other => panic!("expected TimerEffect::Set, got {:?}", other),
                }
            }
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    /// consumed=true, タイマー Kill あり → Consume { effects に TimerEffect::Kill }
    #[test]
    fn response_to_decision_consumed_with_timer_kill() {
        let resp: Response<KeyAction, usize> = Response::consume().with_kill_timer(2usize);
        let decision = FsmAdapter::response_to_decision(resp);
        assert!(decision.is_consumed());
        match decision {
            Decision::Consume { effects } => {
                assert_eq!(effects.len(), 1);
                match &effects[0] {
                    Effect::Timer(TimerEffect::Kill(id)) => assert_eq!(*id, 2),
                    other => panic!("expected TimerEffect::Kill, got {:?}", other),
                }
            }
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    /// consumed=true, actions あり → Consume { effects に InputEffect::SendKeys }
    #[test]
    fn response_to_decision_consumed_with_actions() {
        let action = KeyAction::Char('う');
        let resp: Response<KeyAction, usize> = Response::emit(vec![action.clone()]);
        // Response::emit は consumed=true を設定する
        let decision = FsmAdapter::response_to_decision(resp);
        assert!(decision.is_consumed());
        match decision {
            Decision::Consume { effects } => {
                assert_eq!(effects.len(), 1);
                match &effects[0] {
                    Effect::Input(InputEffect::SendKeys(actions)) => {
                        assert_eq!(actions.len(), 1);
                        assert!(matches!(&actions[0], KeyAction::Char('う')));
                    }
                    other => panic!("expected InputEffect::SendKeys, got {:?}", other),
                }
            }
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    /// consumed=false, effects 無し → PassThrough
    #[test]
    fn response_to_decision_pass_through_no_effects() {
        let resp: Response<KeyAction, usize> = Response::pass_through();
        let decision = FsmAdapter::response_to_decision(resp);
        assert!(!decision.is_consumed());
        assert!(matches!(decision, Decision::PassThrough));
    }

    /// consumed=false, タイマー Kill あり → PassThroughWith
    #[test]
    fn response_to_decision_pass_through_with_timer() {
        let resp: Response<KeyAction, usize> =
            Response::pass_through().with_kill_timer(3usize);
        let decision = FsmAdapter::response_to_decision(resp);
        assert!(!decision.is_consumed());
        match decision {
            Decision::PassThroughWith { effects } => {
                assert_eq!(effects.len(), 1);
                match &effects[0] {
                    Effect::Timer(TimerEffect::Kill(id)) => assert_eq!(*id, 3),
                    other => panic!("expected TimerEffect::Kill, got {:?}", other),
                }
            }
            other => panic!("expected PassThroughWith, got {:?}", other),
        }
    }

    /// タイマー Set + Kill + actions が正しい順序で Effect に変換される
    #[test]
    fn response_to_effects_ordering() {
        let dur = Duration::from_millis(100);
        let action = KeyAction::Char('a');
        let resp: Response<KeyAction, usize> = Response::emit(vec![action])
            .with_timer(1usize, dur)
            .with_kill_timer(2usize);
        let effects = FsmAdapter::response_to_effects(resp);
        // timer commands come first, then actions
        assert!(effects.len() >= 3);
        let timer_effects: Vec<_> = effects
            .iter()
            .filter(|e| matches!(e, Effect::Timer(_)))
            .collect();
        assert_eq!(timer_effects.len(), 2);
        let input_effects: Vec<_> = effects
            .iter()
            .filter(|e| matches!(e, Effect::Input(_)))
            .collect();
        assert_eq!(input_effects.len(), 1);
    }

    // ── FsmAdapter::new / is_enabled ─────────────────────────────────────────

    #[test]
    fn new_adapter_is_enabled_by_default() {
        let adapter = make_adapter();
        assert!(adapter.is_enabled());
    }

    // ── toggle_enabled ────────────────────────────────────────────────────────

    #[test]
    fn toggle_enabled_disables_then_re_enables() {
        let mut adapter = make_adapter();
        assert!(adapter.is_enabled());

        let (enabled, decision) = adapter.toggle_enabled();
        assert!(!enabled, "first toggle should disable");
        assert!(!adapter.is_enabled());
        // flush_pending から来る Decision は consumed（タイマー Kill を含む）
        assert!(decision.is_consumed());

        let (enabled2, _) = adapter.toggle_enabled();
        assert!(enabled2, "second toggle should re-enable");
        assert!(adapter.is_enabled());
    }

    #[test]
    fn toggle_enabled_returns_correct_bool() {
        let mut adapter = make_adapter();
        let (v1, _) = adapter.toggle_enabled(); // false
        let (v2, _) = adapter.toggle_enabled(); // true
        let (v3, _) = adapter.toggle_enabled(); // false
        assert!(!v1);
        assert!(v2);
        assert!(!v3);
    }

    // ── set_enabled ───────────────────────────────────────────────────────────

    #[test]
    fn set_enabled_same_state_is_noop() {
        let mut adapter = make_adapter();
        // エンジンはデフォルトで有効
        let (actual, decision) = adapter.set_enabled(true);
        assert!(actual);
        // 状態変化なし → pass_through になる
        assert!(!decision.is_consumed());
    }

    #[test]
    fn set_enabled_false_disables_engine() {
        let mut adapter = make_adapter();
        let (actual, _) = adapter.set_enabled(false);
        assert!(!actual);
        assert!(!adapter.is_enabled());
    }

    #[test]
    fn set_enabled_false_then_true_re_enables() {
        let mut adapter = make_adapter();
        adapter.set_enabled(false);
        let (actual, _) = adapter.set_enabled(true);
        assert!(actual);
        assert!(adapter.is_enabled());
    }

    #[test]
    fn set_enabled_false_when_already_false_is_noop() {
        let mut adapter = make_adapter();
        adapter.set_enabled(false);
        // 既に false → no-op; pass_through を返す
        let (actual, decision) = adapter.set_enabled(false);
        assert!(!actual);
        assert!(!decision.is_consumed());
    }

    // ── on_event: パススルーキー ─────────────────────────────────────────────

    #[test]
    fn on_event_passthrough_key_returns_pass_through() {
        let mut adapter = make_adapter();
        let event = key_down_passthrough(VK_RETURN, SCAN_RETURN);
        let phys = empty_phys();
        let decision = adapter.on_event(event, &phys);
        assert!(!decision.is_consumed(), "passthrough key should not be consumed");
        assert!(matches!(decision, Decision::PassThrough));
    }

    // ── on_event: エンジン無効時 ─────────────────────────────────────────────

    #[test]
    fn on_event_when_disabled_passes_through_char_key() {
        let mut adapter = make_adapter();
        adapter.set_enabled(false);
        let event = key_down_char(VK_A, SCAN_A, POS_A);
        let phys = empty_phys();
        let decision = adapter.on_event(event, &phys);
        assert!(!decision.is_consumed(), "char key while disabled should pass through");
    }

    // ── on_event: 通常文字キー押下（Wait モード）→ 保留 → Consume ────────────

    #[test]
    fn on_event_char_key_down_is_consumed_as_pending() {
        let mut adapter = make_adapter();
        let event = key_down_char(VK_A, SCAN_A, POS_A);
        let mut tracker = InputTracker::new();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        // Wait モードでは文字キーが保留されるので Consumed を期待
        assert!(decision.is_consumed());
    }

    // ── on_timeout ────────────────────────────────────────────────────────────

    #[test]
    fn on_timeout_after_char_down_emits_action() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = key_down_char(VK_A, SCAN_A, POS_A);
        let phys = tracker.process(&event);
        adapter.on_event(event, &phys);

        // タイマー満了で文字が確定される
        let phys_snap = tracker.snapshot();
        let decision = adapter.on_timeout(1, &phys_snap);
        assert!(decision.is_consumed());
        match decision {
            Decision::Consume { effects } => {
                let has_send_keys = effects.iter().any(|e| matches!(e, Effect::Input(_)));
                assert!(has_send_keys, "timeout should produce SendKeys effect");
            }
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    // ── flush ─────────────────────────────────────────────────────────────────

    /// Idle 状態でのフラッシュは no-op (consumed + 空 effects + タイマー Kill)
    #[test]
    fn flush_when_idle_returns_consumed() {
        let mut adapter = make_adapter();
        let decision = adapter.flush(ContextChange::ImeOff);
        // Idle からのフラッシュは consume() が返る（タイマー Kill 2つ付き）
        assert!(decision.is_consumed());
    }

    /// 保留キーがある状態でフラッシュすると actions が含まれる
    #[test]
    fn flush_with_pending_key_emits_action() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = key_down_char(VK_A, SCAN_A, POS_A);
        let phys = tracker.process(&event);
        adapter.on_event(event, &phys);

        let decision = adapter.flush(ContextChange::FocusChanged);
        assert!(decision.is_consumed());
        match decision {
            Decision::Consume { effects } => {
                let has_send_keys = effects.iter().any(|e| matches!(e, Effect::Input(_)));
                assert!(has_send_keys, "flush with pending should produce SendKeys effect");
            }
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    /// 連続してフラッシュしても二重送信されない（再入 no-op）
    #[test]
    fn flush_twice_does_not_double_emit() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = key_down_char(VK_A, SCAN_A, POS_A);
        let phys = tracker.process(&event);
        adapter.on_event(event, &phys);

        let d1 = adapter.flush(ContextChange::ImeOff);
        let d2 = adapter.flush(ContextChange::ImeOff);

        // 1回目は actions を含む
        let has_keys_1 = match &d1 {
            Decision::Consume { effects } => {
                effects.iter().any(|e| matches!(e, Effect::Input(_)))
            }
            _ => false,
        };
        // 2回目は既に Idle なので actions は空
        let has_keys_2 = match &d2 {
            Decision::Consume { effects } => {
                effects.iter().any(|e| matches!(e, Effect::Input(_)))
            }
            _ => false,
        };

        assert!(has_keys_1, "first flush should emit actions");
        assert!(!has_keys_2, "second flush (idle) should not re-emit actions");
    }

    // ── flush_to_effects ──────────────────────────────────────────────────────

    /// flush_to_effects は EffectVec を直接返す（consumed フラグなし）
    #[test]
    fn flush_to_effects_when_idle_returns_timer_kills() {
        let mut adapter = make_adapter();
        let effects = adapter.flush_to_effects(ContextChange::ImeOff);
        // Idle → タイマー Kill x2 が必ず含まれる
        let kill_count = effects
            .iter()
            .filter(|e| matches!(e, Effect::Timer(TimerEffect::Kill(_))))
            .count();
        assert_eq!(kill_count, 2, "flush from idle should produce 2 timer kill effects");
    }

    #[test]
    fn flush_to_effects_with_pending_key_contains_send_keys() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = key_down_char(VK_A, SCAN_A, POS_A);
        let phys = tracker.process(&event);
        adapter.on_event(event, &phys);

        let effects = adapter.flush_to_effects(ContextChange::FocusChanged);
        let has_send_keys = effects.iter().any(|e| matches!(e, Effect::Input(_)));
        assert!(has_send_keys);
    }

    // ── swap_layout ────────────────────────────────────────────────────────────

    #[test]
    fn swap_layout_flushes_pending_and_returns_consumed() {
        let mut adapter = make_adapter();
        let new_layout = make_layout();
        let decision = adapter.swap_layout(new_layout);
        // swap_layout は flush_pending → consumed を返す
        assert!(decision.is_consumed());
    }

    #[test]
    fn swap_layout_with_pending_key_emits_action_before_swap() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = key_down_char(VK_A, SCAN_A, POS_A);
        let phys = tracker.process(&event);
        adapter.on_event(event, &phys);

        let new_layout = make_layout();
        let decision = adapter.swap_layout(new_layout);
        // 保留キーが確定されるはず
        assert!(decision.is_consumed());
        match decision {
            Decision::Consume { effects } => {
                let has_send_keys = effects.iter().any(|e| matches!(e, Effect::Input(_)));
                assert!(has_send_keys, "swap_layout should flush pending key as action");
            }
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    // ── set_threshold_ms ──────────────────────────────────────────────────────

    /// 設定変更は panic せず適用される（副作用は FSM 内部にあり直接観測不可）
    #[test]
    fn set_threshold_ms_does_not_panic() {
        let mut adapter = make_adapter();
        adapter.set_threshold_ms(50);
        adapter.set_threshold_ms(0);
        adapter.set_threshold_ms(u32::MAX);
    }

    // ── set_confirm_mode ──────────────────────────────────────────────────────

    #[test]
    fn set_confirm_mode_does_not_panic() {
        let mut adapter = make_adapter();
        adapter.set_confirm_mode(ConfirmMode::Speculative, 30);
        adapter.set_confirm_mode(ConfirmMode::TwoPhase, 50);
        adapter.set_confirm_mode(ConfirmMode::AdaptiveTiming, 0);
        adapter.set_confirm_mode(ConfirmMode::Wait, 100);
    }

    // ── set_ngram_model ───────────────────────────────────────────────────────

    #[test]
    fn set_ngram_model_does_not_panic() {
        let mut adapter = make_adapter();
        let model = NgramModel::new(100_000, 50_000, 30_000, 200_000);
        adapter.set_ngram_model(model);
    }

    // ── 統合: 親指シフト同時打鍵 (左親指 + 文字) ────────────────────────────

    /// 左親指キー KeyDown → Consumed になること（保留状態）
    #[test]
    fn on_event_left_thumb_down_is_consumed() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = key_down_left_thumb();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        assert!(decision.is_consumed(), "thumb key down should be consumed");
    }

    /// 親指キー KeyUp → パススルーまたは Consume（保留解消）
    #[test]
    fn on_event_left_thumb_up_after_down() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let down = key_down_left_thumb();
        let phys_down = tracker.process(&down);
        adapter.on_event(down, &phys_down);

        let up = key_up_left_thumb();
        let phys_up = tracker.process(&up);
        let decision = adapter.on_event(up, &phys_up);
        // 単打の親指キー → 保留解消で Consume
        assert!(decision.is_consumed());
    }

    /// 左親指 + 文字を同時押しした後にタイムアウトで解消しない（同時打鍵 → 即確定）
    #[test]
    fn on_event_simultaneous_thumb_and_char_resolves() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();

        let thumb_down = key_down_left_thumb();
        let phys1 = tracker.process(&thumb_down);
        adapter.on_event(thumb_down, &phys1);

        let char_down = key_down_char(VK_A, SCAN_A, POS_A);
        let phys2 = tracker.process(&char_down);
        let decision = adapter.on_event(char_down, &phys2);

        // 左親指 + A の同時打鍵で 'を' が確定
        assert!(decision.is_consumed());
        match decision {
            Decision::Consume { effects } => {
                let has_action = effects.iter().any(|e| matches!(e, Effect::Input(_)));
                assert!(has_action, "simultaneous thumb+char should emit action immediately");
            }
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    // ── 統合: ContextChange バリアント全種でフラッシュできること ─────────────

    #[test]
    fn flush_with_all_context_change_variants() {
        use ContextChange::*;
        let variants = [
            ImeOff,
            InputLanguageChanged,
            EngineDisabled,
            LayoutSwapped,
            FocusChanged,
        ];
        for variant in variants {
            let mut adapter = make_adapter();
            // panic しないこと
            let _decision = adapter.flush(variant);
        }
    }

    // ── FsmAdapter::response_to_effects の直接テスト ─────────────────────────

    /// actions が空のときに Effect::Input が追加されないこと
    #[test]
    fn response_to_effects_no_input_when_actions_empty() {
        let resp: Response<KeyAction, usize> = Response::consume();
        let effects = FsmAdapter::response_to_effects(resp);
        let has_input = effects.iter().any(|e| matches!(e, Effect::Input(_)));
        assert!(!has_input, "no actions → no Input effect");
    }

    /// actions が非空のとき Effect::Input が末尾に追加されること
    #[test]
    fn response_to_effects_input_added_at_end() {
        let action = KeyAction::Char('a');
        let resp: Response<KeyAction, usize> = Response::emit(vec![action]);
        let effects = FsmAdapter::response_to_effects(resp);
        assert!(!effects.is_empty());
        let last = effects.last().unwrap();
        assert!(
            matches!(last, Effect::Input(_)),
            "Input effect should be the last element"
        );
    }

    /// TimerCommand::Set が TimerEffect::Set に変換されること
    #[test]
    fn response_to_effects_set_timer_maps_correctly() {
        let dur = Duration::from_millis(75);
        let resp: Response<KeyAction, usize> = Response::consume().with_timer(42usize, dur);
        let effects = FsmAdapter::response_to_effects(resp);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::Timer(TimerEffect::Set { id, duration }) => {
                assert_eq!(*id, 42);
                assert_eq!(*duration, dur);
            }
            other => panic!("expected TimerEffect::Set, got {:?}", other),
        }
    }

    /// TimerCommand::Kill が TimerEffect::Kill に変換されること
    #[test]
    fn response_to_effects_kill_timer_maps_correctly() {
        let resp: Response<KeyAction, usize> = Response::consume().with_kill_timer(99usize);
        let effects = FsmAdapter::response_to_effects(resp);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::Timer(TimerEffect::Kill(id)) => assert_eq!(*id, 99),
            other => panic!("expected TimerEffect::Kill, got {:?}", other),
        }
    }
}
