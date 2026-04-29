//! NicolaFsm: 同時打鍵判定 FSM（timed-fsm ベース）

use std::time::Duration;

use timed_fsm::Response;

use crate::config::ConfirmMode;
use crate::engine::input_tracker::PhysicalKeyState;
use crate::engine::output_history::{OutputEntry, OutputHistory};
use crate::ngram::NgramModel;
use crate::scanmap::PhysicalPos;
use crate::types::{
    ContextChange, KeyAction, KeyEventType, RawKeyEvent, ScanCode, SpecialKey, Timestamp, VkCode,
};
use crate::yab::{YabFace, YabLayout, YabValue};

use super::fsm_types::{
    BypassReason, ClassifiedEvent, EngineState, Face, IdleIntent, KeyClass, OutputRecord,
    OutputUpdate, ParseAction, PendingKey, PendingThumbData, ResolvedAction, TimerIntent,
};
use super::timing;

/// 同時打鍵判定用タイマー ID
pub const TIMER_PENDING: usize = 1;

/// TwoPhase モード: Phase 1（短い待機）→ Phase 2（投機出力）遷移用タイマー ID
pub const TIMER_SPECULATIVE: usize = 2;

/// AdaptiveTiming モードで連続打鍵と判定する閾値（マイクロ秒）
pub const CONTINUOUS_KEYSTROKE_THRESHOLD_US: u64 = 80_000;

/// `Response` の型エイリアス
type Resp = Response<KeyAction, usize>;

impl From<&YabValue> for KeyAction {
    fn from(value: &YabValue) -> Self {
        match value {
            // kana が解決済みの場合は Char で直接出力。
            // Unicode モードでは IME を経由せず直接送信、VK モードでは
            // send_char_as_vk が kana_to_romaji で逆引きして batched 送信する。
            YabValue::Romaji { kana: Some(ch), .. } => Self::Char(*ch),
            // kana 未解決（拗音など単一 char に収まらないケース）は VK 経由でフォールバック
            YabValue::Romaji { romaji, kana: None } => Self::Romaji(romaji.clone()),
            YabValue::Literal(s) => s.chars().next().map_or(Self::Suppress, Self::Char),
            YabValue::KeySequence(s) => Self::KeySequence(s.clone()),
            YabValue::Special(sk) => Self::SpecialKey(*sk),
            YabValue::None => Self::Suppress,
        }
    }
}

#[cfg(test)]
/// `YabValue` をエンジン出力用の `KeyAction` に変換する（`From` トレイトへの委譲）。
pub(crate) fn yab_value_to_action(value: &YabValue) -> KeyAction {
    KeyAction::from(value)
}

/// `KeyAction` からローマ字文字列を抽出するヘルパー
fn romaji_of(action: &KeyAction) -> String {
    match action {
        KeyAction::Romaji(s) => s.clone(),
        _ => String::new(),
    }
}

/// `OutputUpdate::Record` を生成するヘルパー
pub fn record_output(scan_code: ScanCode, action: &KeyAction, kana: Option<char>) -> OutputUpdate {
    OutputUpdate::Record(OutputRecord {
        scan_code,
        romaji: romaji_of(action),
        kana,
        action: action.clone(),
    })
}

/// 配列変換エンジン（状態機械 + 同時打鍵判定）
#[allow(missing_debug_implementations)]
pub struct NicolaFsm {
    /// 配列定義（.yab ベース）
    pub(crate) layout: YabLayout,

    /// エンジンの状態（データ付き enum）
    pub(crate) state: EngineState,

    /// 同時打鍵の判定閾値（マイクロ秒）
    pub(crate) threshold_us: u64,

    /// エンジンの有効/無効
    pub(crate) enabled: bool,

    /// n-gram モデル（None なら固定閾値にフォールバック）
    pub(crate) ngram_model: Option<NgramModel>,

    /// 確定モード
    pub(crate) confirm_mode: ConfirmMode,

    /// 投機出力までの待機時間（マイクロ秒）
    pub(crate) speculative_delay_us: u64,

    /// 直前のキー押下時刻（AdaptiveTiming 用）
    pub(crate) last_key_timestamp: Option<Timestamp>,

    /// 直前のキーとの間隔（マイクロ秒）。on_key_down の冒頭で算出。
    pub(crate) last_key_gap_us: Option<u64>,

    /// 出力履歴（押下中キーの追跡と直近出力の記録を統合管理）
    pub(crate) output_history: OutputHistory,

    /// 最新の物理キー状態スナップショット（`on_event` の冒頭で更新される）
    pub(crate) phys: PhysicalKeyState,

    /// 消費済み左親指キーの押下タイムスタンプ。
    /// `phys.left_thumb_down` と一致すれば消費済み、不一致なら未消費。
    /// 新しい押下や KeyUp で物理状態が変われば自動的に不一致になるため、
    /// 明示的なリセットが不要。
    pub(crate) left_thumb_consumed: Option<Timestamp>,

    /// 消費済み右親指キーの押下タイムスタンプ（左と同様）。
    pub(crate) right_thumb_consumed: Option<Timestamp>,
}

// ── 公開 API ──
impl NicolaFsm {
    #[must_use]
    pub fn new(
        layout: YabLayout,
        _left_thumb_vk: VkCode,
        _right_thumb_vk: VkCode,
        threshold_ms: u32,
        confirm_mode: ConfirmMode,
        speculative_delay_ms: u32,
    ) -> Self {
        Self {
            layout,
            state: EngineState::Idle,
            threshold_us: u64::from(threshold_ms) * 1000,
            enabled: true,
            ngram_model: None,
            confirm_mode,
            speculative_delay_us: u64::from(speculative_delay_ms) * 1000,
            last_key_timestamp: None,
            last_key_gap_us: None,
            output_history: OutputHistory::new(),
            phys: PhysicalKeyState::empty(),
            left_thumb_consumed: None,
            right_thumb_consumed: None,
        }
    }

    /// Idle 状態に遷移するヘルパー
    const fn go_idle(&mut self) {
        self.state = EngineState::Idle;
    }

    /// `TimerIntent` を `Vec<TimerCommand<usize>>` に変換するヘルパー
    pub(crate) fn timer_cmds(&self, intent: TimerIntent) -> Vec<timed_fsm::TimerCommand<usize>> {
        intent.to_commands(self.threshold_us, self.speculative_delay_us)
    }

    /// 保留中のキーを安全に解消し、Idle 状態に戻す。
    ///
    /// 外部コンテキスト変更（IMEオフ、エンジン無効化、言語切替、レイアウト差替え等）
    /// 時に呼ぶ。現在の外部コンテキストではもう待てないので、保留を解消して安全側に倒す。
    ///
    /// # 事後条件
    /// - `state` は `Idle`
    /// - `TIMER_PENDING` / `TIMER_SPECULATIVE` は停止済み（Response に含まれる）
    /// - 再入しても no-op（Idle → 空の Response）
    /// - 出力は二重送信されない（SpeculativeChar は既に出力済みなので何もしない）
    ///
    /// 呼び出し側は戻り値の `Response` を `dispatch()` で処理すること。
    ///
    /// # Panics
    ///
    /// Panics if internal state is inconsistent (e.g. `PendingChar` phase
    /// without a stored `pending_char`). This indicates a logic error.
    pub fn flush_pending(&mut self, reason: ContextChange) -> Resp {
        let old_state = std::mem::replace(&mut self.state, EngineState::Idle);
        let was_idle = matches!(old_state, EngineState::Idle);

        let response = match old_state {
            EngineState::Idle => {
                // Already idle — no-op
                Response::consume()
            }
            EngineState::PendingChar(pending) => {
                // 保留中の文字キーを通常面で単独確定
                let resolved = self.resolve_pending_char_as_single(
                    pending.scan_code,
                    pending.vk_code,
                    pending.pos,
                );
                self.update_history(resolved.output);
                Response::emit(resolved.actions)
            }
            EngineState::PendingThumb(thumb) => {
                // 保留中の親指キーを単独確定
                let resolved = self.resolve_pending_thumb_as_single(thumb.vk_code);
                self.update_history(resolved.output);
                Response::emit(resolved.actions)
            }
            EngineState::PendingCharThumb { char_key, thumb } => {
                // 文字+親指を同時打鍵として確定
                let resolved = self.resolve_char_thumb_as_simultaneous(
                    char_key.scan_code,
                    char_key.vk_code,
                    char_key.pos,
                    thumb.is_left,
                );
                self.update_history(resolved.output);
                Response::emit(resolved.actions)
            }
            EngineState::SpeculativeChar(_) => {
                // 既に投機出力済み → 出力は正しかったとみなす。何も追加しない。
                Response::consume()
            }
        };

        // タイミング状態もリセット
        self.last_key_timestamp = None;
        self.last_key_gap_us = None;

        if !was_idle {
            log::info!(
                "flush_pending({:?}): flushed {} action(s)",
                reason,
                response.actions.len()
            );
        }

        // 全タイマー停止を付与
        response
            .with_kill_timer(TIMER_PENDING)
            .with_kill_timer(TIMER_SPECULATIVE)
    }

    /// エンジンの有効/無効を切り替える。
    ///
    /// 無効化時は保留キーをフラッシュする。
    /// 戻り値の `Resp` を `dispatch()` で処理すること（タイマー停止 + 保留キー確定）。
    pub fn toggle_enabled(&mut self) -> (bool, Resp) {
        let flush_resp = self.flush_pending(ContextChange::EngineDisabled);
        self.enabled = !self.enabled;
        self.output_history.clear();
        // 物理キー状態（modifiers, thumb_down）は InputTracker が常に追跡しているため、
        // ここでのリセットは不要。
        log::info!(
            "Engine {}",
            if self.enabled { "enabled" } else { "disabled" }
        );
        (self.enabled, flush_resp)
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// エンジンの有効/無効を明示的に設定する。
    ///
    /// 現在の状態と同じ場合は何もしない。
    /// 無効化時は保留キーをフラッシュする。
    /// 戻り値の `Resp` を `dispatch()` で処理すること。
    pub fn set_enabled(&mut self, enable: bool) -> (bool, Resp) {
        if self.enabled == enable {
            return (self.enabled, Response::pass_through());
        }
        self.toggle_enabled()
    }

    /// 同時打鍵判定の閾値を更新する（ミリ秒指定）。
    pub fn set_threshold_ms(&mut self, ms: u32) {
        self.threshold_us = u64::from(ms) * 1000;
    }

    /// 確定モードと投機出力の待機時間を更新する。
    pub fn set_confirm_mode(&mut self, mode: ConfirmMode, speculative_delay_ms: u32) {
        self.confirm_mode = mode;
        self.speculative_delay_us = u64::from(speculative_delay_ms) * 1000;
    }

    /// n-gram モデルを設定する。
    ///
    /// 設定すると、同時打鍵判定の閾値が候補文字の出現頻度に応じて動的に調整される。
    pub fn set_ngram_model(&mut self, model: NgramModel) {
        self.ngram_model = Some(model);
    }

    /// タイミング判定器を構築するヘルパー
    pub(crate) fn timing_judge(&self) -> timing::TimingJudge<'_> {
        timing::TimingJudge::new(
            self.threshold_us,
            self.ngram_model.as_ref(),
            self.output_history.recent_kana(timing::NGRAM_CONTEXT_SIZE),
        )
    }

    /// 配列を動的に差し替える。保留中のキーがあれば安全にフラッシュする。
    pub fn swap_layout(&mut self, layout: YabLayout) -> Resp {
        let flush_resp = self.flush_pending(ContextChange::LayoutSwapped);
        self.layout = layout;
        self.output_history.clear();
        flush_resp
    }
}

// ── 内部ユーティリティ ──
impl NicolaFsm {
    /// Face 列挙値に対応する YabFace への参照を返す
    pub(crate) const fn get_face(&self, face: Face) -> &YabFace {
        match face {
            Face::Normal => &self.layout.normal,
            Face::LeftThumb => &self.layout.left_thumb,
            Face::RightThumb => &self.layout.right_thumb,
            Face::Shift => &self.layout.shift,
        }
    }

    /// scan_code から PhysicalPos を経由して YabFace を引き、`KeyAction` と
    /// 事前解決済みの仮名文字を返す。
    #[allow(clippy::unused_self)]
    pub(crate) fn lookup_face(
        &self,
        pos: Option<PhysicalPos>,
        face: &YabFace,
    ) -> Option<(KeyAction, Option<char>)> {
        let value = face.get(&pos?)?;
        let kana = match value {
            YabValue::Romaji { kana, .. } => *kana,
            YabValue::Literal(s) => s.chars().next(),
            _ => None,
        };
        Some((KeyAction::from(value), kana))
    }

    /// `PendingCharThumb` 状態で char1+thumb を同時打鍵として解決し、アクション列と OutputUpdate を返す。
    ///
    /// 親指キーの物理押下状態を「消費」する。消費後は `active_thumb_face()` が `None` を
    /// 返すようになり、後続のキーが同じ親指押下で二重にシフトされるのを防ぐ。
    fn resolve_char_thumb_as_simultaneous(
        &mut self,
        char_scan: ScanCode,
        char_vk: VkCode,
        char_pos: Option<PhysicalPos>,
        thumb_is_left: bool,
    ) -> ResolvedAction {
        let thumb_face = Face::from_thumb_bool(thumb_is_left);
        if let Some((action, kana)) = self.lookup_face(char_pos, self.get_face(thumb_face)) {
            // 親指キーを「消費」: 同じ物理押下で後続キーがシフトされないようにする
            self.consume_thumb(thumb_is_left);
            let output = record_output(char_scan, &action, kana);
            ResolvedAction {
                actions: vec![action],
                output,
            }
        } else {
            // 親指面に定義がない場合は文字キーを単独確定
            self.resolve_pending_char_as_single(char_scan, char_vk, char_pos)
        }
    }

    /// 親指キーを同時打鍵に「消費済み」とマークし、同じ押下の再利用を防ぐ。
    ///
    /// 現在の物理押下タイムスタンプを記録する。物理状態が変われば（新しい KeyDown
    /// や KeyUp）タイムスタンプが不一致になり、自動的に「未消費」に戻る。
    const fn consume_thumb(&mut self, is_left: bool) {
        if is_left {
            self.left_thumb_consumed = self.phys.left_thumb_down;
        } else {
            self.right_thumb_consumed = self.phys.right_thumb_down;
        }
    }

    pub(crate) const fn enter_pending_char(&mut self, key: PendingKey) {
        self.state = EngineState::PendingChar(key);
    }

    pub(crate) const fn enter_pending_thumb(&mut self, thumb: PendingThumbData) {
        self.state = EngineState::PendingThumb(thumb);
    }

    const fn enter_pending_char_thumb(&mut self, char_key: PendingKey, thumb: PendingThumbData) {
        self.state = EngineState::PendingCharThumb { char_key, thumb };
    }

    pub(crate) const fn enter_speculative_char(&mut self, key: PendingKey) {
        self.state = EngineState::SpeculativeChar(key);
    }

    /// アクション列・consumed フラグ・タイマー指示から `Response` を組み立てる
    pub(crate) fn build_response(
        &self,
        actions: Vec<KeyAction>,
        consumed: bool,
        timer: TimerIntent,
    ) -> Resp {
        let response = if actions.is_empty() && consumed {
            Response::consume()
        } else if actions.is_empty() {
            Response::pass_through()
        } else {
            Response::emit(actions)
        };

        match timer {
            TimerIntent::CancelAll => response
                .with_kill_timer(TIMER_PENDING)
                .with_kill_timer(TIMER_SPECULATIVE),
            TimerIntent::Pending => response
                .with_kill_timer(TIMER_PENDING)
                .with_kill_timer(TIMER_SPECULATIVE)
                .with_timer(TIMER_PENDING, Duration::from_micros(self.threshold_us)),
            TimerIntent::SpeculativeWait => response
                .with_kill_timer(TIMER_PENDING)
                .with_kill_timer(TIMER_SPECULATIVE)
                .with_timer(
                    TIMER_SPECULATIVE,
                    Duration::from_micros(self.speculative_delay_us),
                ),
            TimerIntent::Phase2Transition { remaining_us } => response
                .with_kill_timer(TIMER_SPECULATIVE)
                .with_timer(TIMER_PENDING, Duration::from_micros(remaining_us)),
            TimerIntent::Keep => response,
        }
    }
}

/// `timed_fsm::ParseAction` の具象型エイリアス（ShiftReduceParser 実装用）。
type TieredParseAction = timed_fsm::ParseAction<KeyAction, ClassifiedEvent, usize, OutputUpdate>;

// ── ShiftReduceParser 実装 ──
impl timed_fsm::ShiftReduceParser for NicolaFsm {
    type Action = KeyAction;
    type Token = ClassifiedEvent;
    type TimerId = usize;
    type ReduceRecord = OutputUpdate;

    fn decide(&mut self, token: &ClassifiedEvent) -> TieredParseAction {
        let local = self.decide_and_transition(token);
        match local {
            ParseAction::Shift { timer } => TieredParseAction::Shift {
                timers: self.timer_cmds(timer),
            },
            ParseAction::Reduce {
                actions,
                record,
                timer,
            } => TieredParseAction::Reduce {
                actions,
                record,
                timers: self.timer_cmds(timer),
            },
            ParseAction::ReduceAndContinue {
                actions,
                record,
                remaining,
            } => TieredParseAction::ReduceAndContinue {
                actions,
                record,
                remaining,
            },
            ParseAction::PassThrough { timer } => TieredParseAction::PassThrough {
                timers: self.timer_cmds(timer),
            },
        }
    }

    fn on_reduce(&mut self, record: OutputUpdate) {
        self.update_history(record);
    }
}

// ── KeyDown ディスパッチ ──
impl NicolaFsm {
    /// AdaptiveTiming 用: 直前キーとの間隔を算出してタイムスタンプを更新する
    fn update_timing(&mut self, event: &RawKeyEvent) {
        self.last_key_gap_us = self
            .last_key_timestamp
            .map(|prev| event.timestamp.saturating_sub(prev));
        self.last_key_timestamp = Some(event.timestamp);
    }

    /// Shift 面を使うべきかどうかを判定する
    const fn should_use_shift_plane(&self, ev: &ClassifiedEvent) -> bool {
        self.phys.modifiers.shift && !ev.key_class.is_thumb()
    }

    fn on_key_down(&mut self, event: &RawKeyEvent) -> Resp {
        self.update_timing(event);

        // Bypass check: modifiers, IME control, OS shortcuts.
        // Handled before the parser loop because bypass needs consumed=false
        // even when flush actions are emitted.
        let ev = self.phys.classified;
        if self.bypass_reason(&ev).is_some() {
            return self.handle_bypass(&ev);
        }

        timed_fsm::parse(self, ev)
    }

    /// 状態とイベントに基づいてアクションを決定し、状態遷移を行う
    fn decide_and_transition(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        // State-based dispatch (bypass is handled in on_key_down before entering the loop)
        match self.state {
            EngineState::Idle => self.decide_idle(ev),
            EngineState::PendingChar(_) => self.decide_pending_char(ev),
            EngineState::PendingThumb(_) => self.decide_pending_thumb(ev),
            EngineState::PendingCharThumb { .. } => self.decide_pending_char_thumb(ev),
            EngineState::SpeculativeChar(_) => self.decide_speculative(ev),
        }
    }

    /// Shift 面で Reduce する共通ヘルパー
    fn shift_face_reduce(&self, ev: &ClassifiedEvent) -> ParseAction {
        if let Some((action, kana)) = self.lookup_face(ev.pos, self.get_face(Face::Shift)) {
            ParseAction::Reduce {
                actions: vec![action.clone()],
                record: record_output(ev.scan_code, &action, kana),
                timer: TimerIntent::CancelAll,
            }
        } else {
            // Shift 面に定義がないキーは OS に任せる
            ParseAction::PassThrough {
                timer: TimerIntent::Keep,
            }
        }
    }

    /// Idle 状態でのキー到着時の意図を分類する（純粋関数）。
    fn classify_idle_intent(&self, ev: &ClassifiedEvent) -> IdleIntent {
        // Shift plane
        if self.should_use_shift_plane(ev) {
            return IdleIntent::ShiftPlane;
        }
        // Active thumb combo
        if !ev.key_class.is_thumb() {
            if let Some(face) = self.active_thumb_face() {
                if self.lookup_face(ev.pos, self.get_face(face)).is_some() {
                    return IdleIntent::ActiveThumb(face);
                }
                // 親指面に定義がない → 確定モードに委譲（fall through）
            }
        }
        // Non-layout key
        if !ev.key_class.is_thumb() && !self.is_layout_key(ev.pos) {
            return IdleIntent::PassThrough;
        }
        // Confirm mode dispatch
        IdleIntent::ConfirmMode
    }

    /// Idle 状態でのキー押下処理
    fn decide_idle(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match self.classify_idle_intent(ev) {
            IdleIntent::ShiftPlane => self.shift_face_reduce(ev),
            IdleIntent::ActiveThumb(face) => self.reduce_active_thumb(ev, face),
            IdleIntent::PassThrough => ParseAction::PassThrough {
                timer: TimerIntent::Keep,
            },
            IdleIntent::ConfirmMode => self.dispatch_confirm_mode(ev),
        }
    }

    /// 未消費の親指キーが押下中の場合に親指面で即時確定する。
    fn reduce_active_thumb(&mut self, ev: &ClassifiedEvent, face: Face) -> ParseAction {
        if let Some((action, kana)) = self.lookup_face(ev.pos, self.get_face(face)) {
            // 親指を消費: 同じ押下で後続キーが二重シフトされるのを防ぐ
            let is_left = matches!(face, Face::LeftThumb);
            self.consume_thumb(is_left);
            ParseAction::Reduce {
                actions: vec![action.clone()],
                record: record_output(ev.scan_code, &action, kana),
                timer: TimerIntent::CancelAll,
            }
        } else {
            // classify_idle_intent が lookup 成功を確認済みなのでここには来ないが、
            // 安全側に倒して確定モードに委譲する。
            self.dispatch_confirm_mode(ev)
        }
    }

    /// PendingChar 状態でのキー押下処理
    fn decide_pending_char(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match ev.key_class {
            KeyClass::LeftThumb | KeyClass::RightThumb => self.step_pending_char_thumb(ev),
            KeyClass::Char => self.step_pending_char_char(ev),
            KeyClass::Passthrough => {
                // Passthrough key (e.g. ENTER) arrived while a char is pending.
                // Flush the pending char first so it reaches IME before the passthrough key.
                let EngineState::PendingChar(pending) = self.state else {
                    unreachable!("unexpected state: {:?}", self.state)
                };
                self.go_idle();
                let resolved = self.resolve_pending_char_as_single(
                    pending.scan_code,
                    pending.vk_code,
                    pending.pos,
                );
                ParseAction::ReduceAndContinue {
                    actions: resolved.actions,
                    record: resolved.output,
                    remaining: *ev,
                }
            }
        }
    }

    /// PendingThumb 状態でのキー押下処理
    fn decide_pending_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match ev.key_class {
            KeyClass::Char => self.step_pending_thumb_char(ev),
            KeyClass::LeftThumb | KeyClass::RightThumb => self.step_pending_thumb_thumb(ev),
            KeyClass::Passthrough => {
                // Passthrough key arrived while a thumb is pending.
                // Flush the pending thumb first so it reaches IME before the passthrough key.
                let EngineState::PendingThumb(thumb) = self.state else {
                    unreachable!("unexpected state: {:?}", self.state)
                };
                self.go_idle();
                let resolved = self.resolve_pending_thumb_as_single(thumb.vk_code);
                ParseAction::ReduceAndContinue {
                    actions: resolved.actions,
                    record: resolved.output,
                    remaining: *ev,
                }
            }
        }
    }

    /// PendingCharThumb 状態でのキー押下処理
    fn decide_pending_char_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        self.step_pending_char_thumb_3key(ev)
    }

    /// SpeculativeChar 状態でのキー押下処理
    fn decide_speculative(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match ev.key_class {
            KeyClass::LeftThumb | KeyClass::RightThumb => self.step_speculative_thumb(ev),
            KeyClass::Char => {
                // SpeculativeChar + Char: speculative was correct, go Idle and re-loop
                self.go_idle();
                ParseAction::ReduceAndContinue {
                    actions: vec![],
                    record: OutputUpdate::None,
                    remaining: *ev,
                }
            }
            KeyClass::Passthrough => {
                // Speculative char was already sent to IME; just go idle and pass through.
                self.go_idle();
                ParseAction::ReduceAndContinue {
                    actions: vec![],
                    record: OutputUpdate::None,
                    remaining: *ev,
                }
            }
        }
    }
}

// ── 同時打鍵解決 ──
impl NicolaFsm {
    /// 投機出力を取り消して新しい出力に差し替える。
    ///
    /// 前提: IME は完結済みローマ字を1つの変換単位として扱うため、
    /// BACKSPACE 1発で投機出力全体を削除できる。
    fn retract_and_replace(
        &mut self,
        pending: PendingKey,
        new_action: &KeyAction,
        kana: Option<char>,
    ) -> ParseAction {
        self.output_history.retract_last();
        let actions = vec![
            KeyAction::SpecialKey(SpecialKey::Backspace),
            new_action.clone(),
        ];
        ParseAction::Reduce {
            actions,
            record: OutputUpdate::Record(OutputRecord {
                scan_code: pending.scan_code,
                romaji: romaji_of(new_action),
                kana,
                action: new_action.clone(),
            }),
            timer: TimerIntent::CancelAll,
        }
    }

    /// 投機出力済み状態で親指キーが到着した場合の処理。
    ///
    /// `SpeculativeChar` 状態では通常面の文字が既に IME に送信されている。
    /// 親指キーが閾値時間内に到着した場合、`retract_and_replace()` で出力を差し替える。
    ///
    /// 閾値超過時や親指面に定義がない場合は、投機出力は正しかったとみなし、
    /// Idle に戻って親指キーを新規イベントとして再処理する。
    fn step_speculative_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let EngineState::SpeculativeChar(pending) = self.state else {
            unreachable!("unexpected state: {:?}", self.state)
        };
        let face = Face::from_thumb(ev.key_class);

        // Look up what the simultaneous keystroke would produce
        if let Some((thumb_action, thumb_kana)) = self.lookup_face(pending.pos, self.get_face(face))
        {
            if self
                .timing_judge()
                .is_simultaneous(pending.timestamp, ev.timestamp, thumb_kana)
            {
                // Within threshold → retract speculative output + emit thumb face

                // 親指を消費: 同じ押下で後続キーが二重シフトされるのを防ぐ
                let is_left = matches!(face, Face::LeftThumb);
                self.consume_thumb(is_left);

                self.go_idle();
                return self.retract_and_replace(pending, &thumb_action, thumb_kana);
            }
            // Outside threshold → speculative was correct, process thumb as new key
        } else {
            // No thumb face entry → speculative was correct
        }
        // Go idle and re-process the thumb key
        self.go_idle();
        ParseAction::ReduceAndContinue {
            actions: vec![],
            record: OutputUpdate::None,
            remaining: *ev,
        }
    }

    /// PendingChar + 親指キー → 同時打鍵候補（閾値内なら PendingCharThumb、超過なら flush+新規）
    fn step_pending_char_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let EngineState::PendingChar(pending) = self.state else {
            unreachable!("unexpected state: {:?}", self.state)
        };
        // 親指面で保留文字キーの候補を取得し閾値を調整
        let candidate_face = Face::from_thumb(ev.key_class);
        let candidate = self.lookup_face(pending.pos, self.get_face(candidate_face));
        let candidate_kana = candidate.as_ref().and_then(|(_, kana)| *kana);

        if self
            .timing_judge()
            .is_simultaneous(pending.timestamp, ev.timestamp, candidate_kana)
        {
            // 保留=文字, 到着=親指 → PendingCharThumb へ遷移（3 鍵目を待つ）
            self.enter_pending_char_thumb(
                pending,
                PendingThumbData {
                    scan_code: ev.scan_code,
                    vk_code: ev.vk_code,
                    is_left: ev.key_class.is_left_thumb(),
                    timestamp: ev.timestamp,
                },
            );
            return ParseAction::Shift {
                timer: TimerIntent::Pending,
            };
        }

        // 時間超過 → 前の保留を単独確定し、今回のキーを再処理
        self.go_idle();
        let resolved =
            self.resolve_pending_char_as_single(pending.scan_code, pending.vk_code, pending.pos);
        ParseAction::ReduceAndContinue {
            actions: resolved.actions,
            record: resolved.output,
            remaining: *ev,
        }
    }

    /// PendingChar + 文字キー → 前の保留を単独確定し、今回のキーを再処理
    fn step_pending_char_char(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let EngineState::PendingChar(pending) = self.state else {
            unreachable!("unexpected state: {:?}", self.state)
        };
        self.go_idle();
        let resolved =
            self.resolve_pending_char_as_single(pending.scan_code, pending.vk_code, pending.pos);
        ParseAction::ReduceAndContinue {
            actions: resolved.actions,
            record: resolved.output,
            remaining: *ev,
        }
    }

    /// PendingThumb + 文字キー → 同時打鍵候補（閾値内なら即時確定、超過なら flush+新規）
    fn step_pending_thumb_char(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let EngineState::PendingThumb(thumb) = self.state else {
            unreachable!("unexpected state: {:?}", self.state)
        };
        // 親指面で到着文字キーの候補を取得し閾値を調整
        let pending_face = Face::from_thumb_bool(thumb.is_left);
        let candidate = self.lookup_face(ev.pos, self.get_face(pending_face));
        let candidate_kana = candidate.as_ref().and_then(|(_, kana)| *kana);

        if self
            .timing_judge()
            .is_simultaneous(thumb.timestamp, ev.timestamp, candidate_kana)
        {
            if let Some((action, kana)) = candidate {
                // 保留=親指, 到着=文字 → 同時打鍵
                // 親指を消費: 同じ押下で後続キーが二重シフトされるのを防ぐ
                self.consume_thumb(thumb.is_left);
                self.go_idle();
                return ParseAction::Reduce {
                    actions: vec![action.clone()],
                    record: record_output(ev.scan_code, &action, kana),
                    timer: TimerIntent::CancelAll,
                };
            }
        }

        // 時間超過 or 候補なし → 前の保留を単独確定し、今回のキーを再処理
        self.go_idle();
        let resolved = self.resolve_pending_thumb_as_single(thumb.vk_code);
        ParseAction::ReduceAndContinue {
            actions: resolved.actions,
            record: resolved.output,
            remaining: *ev,
        }
    }

    /// PendingThumb + 親指キー → 前の保留を単独確定し、今回のキーを再処理
    fn step_pending_thumb_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let EngineState::PendingThumb(thumb) = self.state else {
            unreachable!("unexpected state: {:?}", self.state)
        };
        self.go_idle();
        let resolved = self.resolve_pending_thumb_as_single(thumb.vk_code);
        ParseAction::ReduceAndContinue {
            actions: resolved.actions,
            record: resolved.output,
            remaining: *ev,
        }
    }

    /// OutputUpdate に基づいて出力履歴を更新する共通ヘルパー
    pub(crate) fn update_history(&mut self, output: OutputUpdate) {
        match output {
            OutputUpdate::Record(rec) => {
                self.output_history.push(OutputEntry {
                    scan_code: rec.scan_code,
                    romaji: rec.romaji,
                    kana: rec.kana,
                    action: rec.action,
                });
            }
            OutputUpdate::RetractAndRecord(rec) => {
                self.output_history.retract_last();
                self.output_history.push(OutputEntry {
                    scan_code: rec.scan_code,
                    romaji: rec.romaji,
                    kana: rec.kana,
                    action: rec.action,
                });
            }
            OutputUpdate::None => {}
        }
    }

    /// 保留中の文字キーを単独打鍵として解決し、アクション列と OutputUpdate を返す
    fn resolve_pending_char_as_single(
        &self,
        scan_code: ScanCode,
        vk_code: VkCode,
        pos: Option<PhysicalPos>,
    ) -> ResolvedAction {
        if let Some((action, kana)) = self.lookup_face(pos, self.get_face(Face::Normal)) {
            let output = record_output(scan_code, &action, kana);
            ResolvedAction {
                actions: vec![action],
                output,
            }
        } else {
            let action = KeyAction::Key(vk_code);
            let output = record_output(scan_code, &action, None);
            ResolvedAction {
                actions: vec![action],
                output,
            }
        }
    }

    /// 保留中の親指キーを単独打鍵として解決し、アクション列と OutputUpdate を返す
    #[allow(clippy::unused_self)]
    fn resolve_pending_thumb_as_single(&self, vk_code: VkCode) -> ResolvedAction {
        // Thumb keys use vk_code as their scan_code key since they don't have
        // standard scan codes in our map; use u32 from vk_code for consistency.
        let action = KeyAction::Key(vk_code);
        let output = record_output(ScanCode(u32::from(vk_code.0)), &action, None);
        ResolvedAction {
            actions: vec![action],
            output,
        }
    }

    /// `PendingCharThumb` 状態で新しいキーが到着した場合の 3 鍵仲裁処理
    ///
    /// char1 → thumb → char2 の並びで、親指キーを char1 と char2 のどちらに
    /// ペアリングするかを決定する。判定基準:
    ///
    /// 1. タイミング: d1 (char1→thumb) vs d2 (thumb→char2)
    /// 2. n-gram スコア: char1+thumb の出力候補 vs char2+thumb の出力候補
    ///
    /// タイミング差が小さいとき（どちらとも取れる場合）は n-gram スコアで
    /// より自然な日本語になるほうを選ぶ。
    fn step_pending_char_thumb_3key(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let EngineState::PendingCharThumb {
            char_key: pending,
            thumb,
        } = self.state
        else {
            unreachable!("unexpected state: {:?}", self.state)
        };
        self.go_idle();

        if !ev.key_class.is_thumb() {
            // char2 到着 → 3 鍵仲裁
            let judge = self.timing_judge();
            let thumb_face = Face::from_thumb_bool(thumb.is_left);
            let char1_thumb_kana = self
                .lookup_face(pending.pos, self.get_face(thumb_face))
                .and_then(|(_, k)| k);
            let char1_single_kana = self
                .lookup_face(pending.pos, self.get_face(Face::Normal))
                .and_then(|(_, k)| k);
            let char2_thumb_kana = self
                .lookup_face(ev.pos, self.get_face(thumb_face))
                .and_then(|(_, k)| k);
            let result = judge.three_key_pairing(
                pending.timestamp,
                thumb.timestamp,
                ev.timestamp,
                char1_thumb_kana,
                char1_single_kana,
                char2_thumb_kana,
            );
            let prefer_char1 = result == timing::ThreeKeyResult::PairWithChar1;

            if prefer_char1 {
                // char1+thumb = 同時打鍵、char2 は再処理
                let resolved = self.resolve_char_thumb_as_simultaneous(
                    pending.scan_code,
                    pending.vk_code,
                    pending.pos,
                    thumb.is_left,
                );
                return ParseAction::ReduceAndContinue {
                    actions: resolved.actions,
                    record: resolved.output,
                    remaining: *ev,
                };
            }
            // char1 = 単独、char2+thumb = 同時打鍵
            let char1_resolved = self.resolve_pending_char_as_single(
                pending.scan_code,
                pending.vk_code,
                pending.pos,
            );
            let thumb_face = Face::from_thumb_bool(thumb.is_left);
            if let Some((action, kana)) = self.lookup_face(ev.pos, self.get_face(thumb_face)) {
                // char1 の履歴を先に更新してから char2+thumb を確定
                self.update_history(char1_resolved.output);
                self.consume_thumb(thumb.is_left);
                let mut all_actions = char1_resolved.actions;
                all_actions.push(action.clone());
                return ParseAction::Reduce {
                    actions: all_actions,
                    record: record_output(ev.scan_code, &action, kana),
                    timer: TimerIntent::CancelAll,
                };
            }
            // 親指面に char2 の定義がない場合は char1 を単独確定し、char2 を再処理
            return ParseAction::ReduceAndContinue {
                actions: char1_resolved.actions,
                record: char1_resolved.output,
                remaining: *ev,
            };
        }
        // 親指キーが来た場合は char1+thumb を同時打鍵として確定し、
        // 新しい親指キーを再処理
        let resolved = self.resolve_char_thumb_as_simultaneous(
            pending.scan_code,
            pending.vk_code,
            pending.pos,
            thumb.is_left,
        );
        ParseAction::ReduceAndContinue {
            actions: resolved.actions,
            record: resolved.output,
            remaining: *ev,
        }
    }
}

// ── KeyUp 処理 ──
impl NicolaFsm {
    fn on_key_up(&mut self, event: &RawKeyEvent) -> Resp {
        // phys.classified は on_key_down 側で使用済み

        // PendingCharThumb 状態での KeyUp 処理
        if let EngineState::PendingCharThumb { char_key, thumb } = self.state {
            if event.vk_code == char_key.vk_code || event.vk_code == thumb.vk_code {
                return self.handle_key_up_pending_char_thumb(event);
            }
        }

        // SpeculativeChar 状態で投機出力キーが離された場合 → 出力確定（Idle へ遷移）
        if let EngineState::SpeculativeChar(pending) = self.state {
            if event.vk_code == pending.vk_code {
                self.go_idle();
                // output_history から対応するキーの KeyUp を処理
                return self.handle_key_up_active(event);
            }
        }

        // 保留中のキーが離された場合、保留を単独確定
        if self.is_pending_key(event.vk_code) {
            return self.handle_key_up_pending(event);
        }

        // output_history から対応する注入済みキーを探してリリース
        self.handle_key_up_active(event)
    }

    /// 保留中キーの vk_code と一致するか判定する
    fn is_pending_key(&self, vk_code: VkCode) -> bool {
        match self.state {
            EngineState::PendingChar(pending) => pending.vk_code == vk_code,
            EngineState::PendingThumb(thumb) => thumb.vk_code == vk_code,
            EngineState::Idle
            | EngineState::PendingCharThumb { .. }
            | EngineState::SpeculativeChar(_) => false,
        }
    }

    /// PendingCharThumb 状態で char1 または thumb が離された場合の処理
    fn handle_key_up_pending_char_thumb(&mut self, event: &RawKeyEvent) -> Resp {
        let EngineState::PendingCharThumb {
            char_key: pending,
            thumb,
        } = self.state
        else {
            unreachable!("unexpected state: {:?}", self.state)
        };
        self.go_idle();
        let resolved = self.resolve_char_thumb_as_simultaneous(
            pending.scan_code,
            pending.vk_code,
            pending.pos,
            thumb.is_left,
        );
        self.update_history(resolved.output);
        let mut actions = resolved.actions;
        if event.vk_code == pending.vk_code {
            // char1 が離された: Char は KeyUp 不要だが Key なら KeyUp 追加
            if let Some(entry) = self.output_history.remove_by_scan(event.scan_code) {
                if let KeyAction::Key(vk) = entry.action {
                    actions.push(KeyAction::KeyUp(vk));
                }
            }
        }
        // thumb 側が離された場合: thumb KeyDown は consume 済みなので KeyUp も consume する
        // （output_history には thumb のエントリはないため remove 不要）
        self.build_response(actions, true, TimerIntent::CancelAll)
    }

    /// 保留中のキーが離された場合、保留を単独確定して KeyUp を処理する
    fn handle_key_up_pending(&mut self, event: &RawKeyEvent) -> Resp {
        let old_state = std::mem::replace(&mut self.state, EngineState::Idle);

        let resolved = match old_state {
            EngineState::PendingChar(pending) => {
                self.resolve_pending_char_as_single(pending.scan_code, pending.vk_code, pending.pos)
            }
            EngineState::PendingThumb(thumb) => self.resolve_pending_thumb_as_single(thumb.vk_code),
            EngineState::Idle
            | EngineState::PendingCharThumb { .. }
            | EngineState::SpeculativeChar(_) => {
                log::error!(
                    "unexpected state in handle_key_up_pending: {:?}",
                    self.state
                );
                ResolvedAction {
                    actions: vec![],
                    output: OutputUpdate::None,
                }
            }
        };
        self.update_history(resolved.output);
        let mut result = resolved.actions;
        if let Some(entry) = self.output_history.remove_by_scan(event.scan_code) {
            if let KeyAction::Key(vk) = entry.action {
                result.push(KeyAction::KeyUp(vk));
            }
        }
        // Unicode 文字 (Char) は Down+Up 一括送信済みなので KeyUp 追加不要
        self.build_response(result, true, TimerIntent::CancelAll)
    }

    /// output_history から対応する注入済みキーを探してリリースする
    fn handle_key_up_active(&mut self, event: &RawKeyEvent) -> Resp {
        if let Some(entry) = self.output_history.remove_by_scan(event.scan_code) {
            return match entry.action {
                // Unicode 文字やローマ字列の場合、KeyUp は不要（押下時に入力完了）
                KeyAction::Char(_) | KeyAction::Romaji(_) => {
                    self.build_response(vec![KeyAction::Suppress], true, TimerIntent::CancelAll)
                }
                KeyAction::Key(vk) => {
                    self.build_response(vec![KeyAction::KeyUp(vk)], true, TimerIntent::CancelAll)
                }
                _ => Response::pass_through(),
            };
        }
        Response::pass_through()
    }
}

// ── タイムアウト処理 ──
impl NicolaFsm {
    /// PendingChar タイムアウト：文字キーを単独打鍵として確定する
    fn timeout_pending_char(
        &mut self,
        scan_code: ScanCode,
        vk_code: VkCode,
        pos: Option<PhysicalPos>,
    ) -> Resp {
        if let Some((action, kana)) = self.lookup_face(pos, self.get_face(Face::Normal)) {
            self.update_history(record_output(scan_code, &action, kana));
            self.build_response(vec![action], true, TimerIntent::CancelAll)
        } else {
            // 配列定義に含まれないキーはそのまま通す
            let action = KeyAction::Key(vk_code);
            self.update_history(record_output(scan_code, &action, None));
            self.build_response(vec![action], true, TimerIntent::CancelAll)
        }
    }

    /// PendingThumb タイムアウト：親指キーを単独打鍵として確定する
    fn timeout_pending_thumb(&mut self, vk_code: VkCode) -> Resp {
        let action = KeyAction::Key(vk_code);
        self.update_history(record_output(ScanCode(u32::from(vk_code.0)), &action, None));
        self.build_response(vec![action], true, TimerIntent::CancelAll)
    }

    /// PendingCharThumb タイムアウト：char1+thumb を同時打鍵として確定する
    fn timeout_pending_char_thumb(
        &mut self,
        char_scan: ScanCode,
        char_vk: VkCode,
        char_pos: Option<PhysicalPos>,
        thumb_is_left: bool,
    ) -> Resp {
        let resolved =
            self.resolve_char_thumb_as_simultaneous(char_scan, char_vk, char_pos, thumb_is_left);
        self.update_history(resolved.output);
        self.build_response(resolved.actions, true, TimerIntent::CancelAll)
    }

    /// TwoPhase モード: Phase 1 の短い待機がタイムアウトした場合の処理。
    ///
    /// 親指キーが Phase 1 内に到着しなかったので、投機出力（Phase 2）に遷移する。
    /// 通常面の文字を出力し、`SpeculativeChar` 状態に入る。
    /// 残りの閾値時間（`threshold_us - speculative_delay_us`）で `TIMER_PENDING` を設定する。
    ///
    /// Phase 2 に入った後、残り時間内に親指キーが到着すれば
    /// `step_speculative_thumb()` が BACKSPACE で投機出力を取り消す。
    /// `TIMER_PENDING` が満了すれば投機出力は正しかったとみなし、Idle に戻る。
    fn on_timeout_speculative(&mut self) -> Resp {
        match self.state {
            EngineState::PendingChar(pending) => {
                // Output normal face speculatively
                let face = Face::Normal;
                if let Some((action, kana)) = self.lookup_face(pending.pos, self.get_face(face)) {
                    self.enter_speculative_char(pending);
                    // Emit the speculative output + set TIMER_PENDING for remaining time
                    let remaining_us = self.threshold_us.saturating_sub(self.speculative_delay_us);
                    self.update_history(record_output(pending.scan_code, &action, kana));
                    self.build_response(
                        vec![action],
                        true,
                        TimerIntent::Phase2Transition { remaining_us },
                    )
                } else {
                    self.go_idle();
                    Response::pass_through().with_kill_timer(TIMER_SPECULATIVE)
                }
            }
            // Other states shouldn't have TIMER_SPECULATIVE active
            other => {
                log::warn!("TIMER_SPECULATIVE fired in unexpected state: {other:?}",);
                Response::pass_through().with_kill_timer(TIMER_SPECULATIVE)
            }
        }
    }
}

// ── バイパス ──
impl NicolaFsm {
    /// 現在押下中かつ未消費の親指キーに対応するシフト面を返す。
    ///
    /// 消費済みかどうかはタイムスタンプの一致で判定する。物理状態が変われば
    /// （新しい KeyDown や KeyUp）自動的に不一致になるため、明示的なリセット不要。
    fn active_thumb_face(&self) -> Option<Face> {
        let left_consumed = self.phys.left_thumb_down.is_some()
            && self.left_thumb_consumed == self.phys.left_thumb_down;
        let right_consumed = self.phys.right_thumb_down.is_some()
            && self.right_thumb_consumed == self.phys.right_thumb_down;

        if self.phys.left_thumb_down.is_some() && !left_consumed {
            Some(Face::LeftThumb)
        } else if self.phys.right_thumb_down.is_some() && !right_consumed {
            Some(Face::RightThumb)
        } else {
            None
        }
    }

    /// いずれかの配列面に定義されているキーかどうか
    pub(crate) fn is_layout_key(&self, pos: Option<PhysicalPos>) -> bool {
        let Some(pos) = pos else {
            return false;
        };
        self.get_face(Face::Normal).contains_key(&pos)
            || self.get_face(Face::LeftThumb).contains_key(&pos)
            || self.get_face(Face::RightThumb).contains_key(&pos)
            || self.get_face(Face::Shift).contains_key(&pos)
    }

    /// キーイベントがエンジン処理をバイパスすべきかを判定する
    fn bypass_reason(&self, ev: &ClassifiedEvent) -> Option<BypassReason> {
        if ev.key_class == KeyClass::Passthrough {
            return Some(BypassReason::Passthrough);
        }
        if ev.is_ime_control {
            return Some(BypassReason::ImeControl);
        }
        if self.phys.modifiers.is_os_modifier_held() {
            return Some(BypassReason::OsModifierHeld);
        }
        None
    }

    /// バイパス理由に基づいて保留キーをフラッシュしつつパススルーする
    ///
    /// 全てのバイパス理由で同一の処理: 保留があればフラッシュ、元のキーは OS にパススルー。
    /// consumed=false を維持するため ParseAction ループの外で直接 Resp を返す。
    fn handle_bypass(&mut self, ev: &ClassifiedEvent) -> Resp {
        if let Some(reason) = self.bypass_reason(ev) {
            log::trace!("bypass: {reason:?}");
        }
        if self.state.is_idle() {
            return Response::pass_through();
        }
        let flush = self.flush_pending(ContextChange::ImeOff);
        let mut resp = Response::pass_through();
        resp.actions = flush.actions;
        resp.timers = flush.timers;
        resp
    }
}

// ── イベント処理エントリポイント ──
impl NicolaFsm {
    /// キーイベントを処理する。
    ///
    /// `phys` は `InputTracker::process()` が返した物理キー状態スナップショット。
    /// 内部メソッドは `self.phys` フィールド経由でこの状態を参照する。
    pub fn on_event(&mut self, event: RawKeyEvent, phys: &PhysicalKeyState) -> Resp {
        self.phys = *phys;
        // 親指消費フラグのリセットは不要: タイムスタンプ比較で自動判定される

        if !self.enabled {
            return Response::pass_through();
        }

        match event.event_type {
            KeyEventType::KeyDown => self.on_key_down(&event),
            KeyEventType::KeyUp => self.on_key_up(&event),
        }
    }

    /// タイマー満了時の処理。
    ///
    /// `phys` は `InputTracker` の最新スナップショット。
    /// タイマー発火時点の正確な物理キー状態を反映する。
    pub fn on_timeout(&mut self, timer_id: usize, phys: &PhysicalKeyState) -> Resp {
        self.phys = *phys;
        match timer_id {
            TIMER_SPECULATIVE => return self.on_timeout_speculative(),
            TIMER_PENDING => {}
            _ => return Response::pass_through(),
        }

        let old_state = std::mem::replace(&mut self.state, EngineState::Idle);

        match old_state {
            EngineState::Idle => {
                // Spurious timeout — state already transitioned to Idle.
                // pass_through to avoid suppressing unrelated keys.
                Response::pass_through().with_kill_timer(TIMER_PENDING)
            }
            EngineState::PendingChar(pending) => {
                self.timeout_pending_char(pending.scan_code, pending.vk_code, pending.pos)
            }
            EngineState::PendingThumb(thumb) => self.timeout_pending_thumb(thumb.vk_code),
            EngineState::PendingCharThumb { char_key, thumb } => self.timeout_pending_char_thumb(
                char_key.scan_code,
                char_key.vk_code,
                char_key.pos,
                thumb.is_left,
            ),
            // 投機出力済み → タイムアウト = 親指キー未到着 → 投機出力は正しかった → Idle へ
            EngineState::SpeculativeChar(_) => Response::consume().with_kill_timer(TIMER_PENDING),
        }
    }
}
