//! NicolaFsm: 同時打鍵判定 FSM（timed-fsm ベース）

use smallvec::{smallvec, SmallVec};
use timed_fsm::{Response, ShiftReduceParser};

use crate::config::ConfirmMode;
use crate::engine::input_tracker::PhysicalKeyState;
use crate::engine::output_history::{OutputEntry, OutputHistory};
use crate::ngram::NgramModel;
use crate::scanmap::PhysicalPos;
use crate::types::{
    ContextChange, KeyAction, KeyEventType, RawKeyEvent, ScanCode, SpecialKey, Timestamp, VkCode,
};
use crate::yab::{YabFace, YabLayout, YabValue};

use super::consecutive_counter::ConsecutiveSoloCounter;
use super::fsm_types::{
    BypassReason, ClassifiedEvent, ComposingHint, EngineState, Face, IdleIntent, KeyClass,
    OutputUpdate, ParseAction, PendingKey, PendingThumbData, ResolvedAction, TimerIntent,
    TIMER_PENDING, TIMER_SPECULATIVE,
};
use super::timing;

/// AdaptiveTiming モードで連続打鍵と判定する閾値（マイクロ秒）
pub(super) const CONTINUOUS_KEYSTROKE_THRESHOLD_US: u64 = 80_000;

/// ソロ連打トリガーの連打間隔上限（マイクロ秒）
const SOLO_OFF_TIMEOUT_US: u64 = 400_000;

/// ソロ連打でエンジン OFF を発動する必要連打回数
///
/// 3 回だと、スリープ復帰直後など IME が混乱した状態で焦って無変換キーを
/// 連打しただけで誤発火し、「緊急脱出」のつもりが逆に engine を止めてしまう
/// 事例が実機で発生した（2026-07-08、conv がカタカナへ固定＋shadow OFF から
/// の復帰を試みて無変換を連打した結果、user_enabled が意図せず false に）。
/// 5 回に引き上げて誤発火しにくくする。
const SOLO_OFF_TRIGGER_COUNT: u32 = 5;

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

/// 配列変換エンジン（状態機械 + 同時打鍵判定）
#[allow(missing_debug_implementations)]
#[allow(clippy::struct_excessive_bools)]
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

    /// ソロ確定の連続回数を追跡する汎用カウンター。
    solo_counter: ConsecutiveSoloCounter,

    /// ソロ N 連打でエンジン OFF を発動するキー（VkCode(0) = 機能無効）。
    engine_off_triple_vk: VkCode,

    /// ソロ連打でのエンジン OFF 要求フラグ（1ショット）。
    engine_off_requested: bool,

    /// `left_thumb_key`/`right_thumb_key` のいずれかが Space (`VK_SPACE`) に
    /// 割り当てられている場合、その VK コード。どちらも Space でなければ `None`。
    ///
    /// 実際の VK 番号（Windows の magic hex）は Platform 層（各 `vk.rs`）の
    /// 責務であり、core はここで渡された値と等値比較するだけで「Space かどうか」
    /// を判定する（`GeneralConfig::space_thumb_ignore_composing_guard` 等参照）。
    space_thumb_vk: Option<VkCode>,

    /// Space 親指キー単独タップ確定時、IME 変換候補ウィンドウ表示中
    /// （`composing`）でも生 VK を送出するか。`space_thumb_vk` が `None` なら無効。
    space_thumb_ignore_composing_guard: bool,

    /// Shift を押しながら Space 親指キーを押した場合、同時打鍵判定を試みず
    /// 即座にリテラルなスペースとして送出するか。`space_thumb_vk` が `None` なら無効。
    space_thumb_shift_literal: bool,

    /// `left_thumb_key`/`right_thumb_key` のいずれかが無変換 (`VK_NONCONVERT`) に
    /// 割り当てられている場合、その VK コード。`space_thumb_vk` と同様、実際の VK
    /// 番号は Platform 層の責務で、core は等値比較のみ行う。
    muhenkan_vk: Option<VkCode>,

    /// 無変換キー単独タップ確定時、IME 変換候補ウィンドウ表示中（`composing`）でも
    /// 生 VK を送出するか。`muhenkan_vk` が `None` なら無効。
    muhenkan_solo_tap_ignore_composing_guard: bool,

    /// `left_thumb_key`/`right_thumb_key` のいずれかが変換 (`VK_CONVERT`) に
    /// 割り当てられている場合、その VK コード。`muhenkan_vk` と同様の扱い。
    henkan_vk: Option<VkCode>,

    /// 変換キー単独タップ確定時、IME 変換候補ウィンドウ表示中（`composing`）でも
    /// 生 VK を送出するか。`henkan_vk` が `None` なら無効。
    henkan_solo_tap_ignore_composing_guard: bool,
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
            solo_counter: ConsecutiveSoloCounter::new(SOLO_OFF_TIMEOUT_US),
            engine_off_triple_vk: VkCode(0),
            engine_off_requested: false,
            // 既定値は GeneralConfig::default() と揃える（Space 未割当 / ガード類は
            // 有効）。実際の Space VK は Platform 層が set_space_thumb_config() で
            // 明示的に配線する（config.rs の doc 参照）。
            space_thumb_vk: None,
            space_thumb_ignore_composing_guard: true,
            space_thumb_shift_literal: true,
            // 無変換/変換の VK は Platform 層が set_thumb_key_solo_tap_config() で
            // 明示的に配線するまで None。ガード既定値は GeneralConfig::default() と
            // 揃えて false（従来通り composing 中は抑制）。
            muhenkan_vk: None,
            muhenkan_solo_tap_ignore_composing_guard: false,
            henkan_vk: None,
            henkan_solo_tap_ignore_composing_guard: false,
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
    /// `composing` は `PendingThumb` の Space フォールバック判定（composing 中でも
    /// 生 VK_SPACE を送出する例外）に使う。**`Trusted(bool)` を渡してよいのは、
    /// 呼び出し元がこの `composing` 値を「保留キーが入力された時点と同一のウィンドウ/
    /// コンテキスト」のものだと保証できる場合のみ**（同一イベント処理内の割り込み、
    /// 直近の `self.phys.composing` 等）。
    ///
    /// `FocusChanged`（フォーカス変更）や `InvalidateContext`（IME OFF・言語切替等の
    /// 外部コンテキスト喪失）経由のフラッシュは、`InputContext::composing` を
    /// 呼び出し時点で読み直す設計上、**既に切り替わった後の新しいウィンドウ**の状態を
    /// 指している（`Runtime::ir_notify_focus_changed` は `detect_and_update_focus()` で
    /// フォーカスを切り替えた後に `build_ctx()` を呼ぶ）。この場合は `Unknown` を渡す。
    /// `Unknown` では Space 例外も含め無条件 suppress する（フォーカス切替後に別ウィンドウへ
    /// 生 VK_SPACE 等が誤注入されるのを防ぐ安全側の選択。過去に類似の focus 遷移バグを
    /// 繰り返してきたため — `docs/known-bugs.md` 参照）。
    ///
    /// # Panics
    ///
    /// Panics if internal state is inconsistent (e.g. `PendingChar` phase
    /// without a stored `pending_char`). This indicates a logic error.
    pub fn flush_pending(&mut self, reason: ContextChange, composing: ComposingHint) -> Resp {
        let old_state = std::mem::replace(&mut self.state, EngineState::Idle);
        let was_idle = matches!(old_state, EngineState::Idle);

        let response = match old_state {
            EngineState::Idle => {
                // Already idle — no-op
                Response::consume()
            }
            EngineState::PendingChar(pending) => {
                // 保留中の文字キーを通常面で単独確定
                let resolved = self.resolve_pending_char_as_single(&pending);
                self.update_history(resolved.output);
                Response::emit(resolved.actions.into_vec())
            }
            EngineState::PendingThumb(thumb) => {
                // 保留中の親指キーを単独確定。composing を信頼できない場合は
                // Space 例外も含め無条件 suppress する（上記 doc 参照）。
                let resolved = match composing {
                    ComposingHint::Trusted(c) => self.resolve_pending_thumb_as_single(
                        thumb.scan_code,
                        thumb.vk_code,
                        thumb.modifier_key,
                        c,
                    ),
                    ComposingHint::Unknown => ResolvedAction {
                        actions: SmallVec::new(),
                        output: OutputUpdate::None,
                    },
                };
                self.update_history(resolved.output);
                Response::emit(resolved.actions.into_vec())
            }
            EngineState::PendingCharThumb {
                char_key,
                thumb,
                char1_released,
            } => {
                // 文字+親指を同時打鍵として確定
                let resolved = self.resolve_char_thumb_as_simultaneous(&char_key, thumb.face());
                self.update_history(resolved.output);
                let mut actions = resolved.actions;
                if char1_released {
                    // char1 は既に物理的に離されている → Key 出力があれば KeyUp も追加
                    self.append_key_up_for(&mut actions, char_key.scan_code);
                }
                Response::emit(actions.into_vec())
            }
            EngineState::SpeculativeChar(_) => {
                // 既に投機出力済み → 出力は正しかったとみなす。何も追加しない。
                Response::consume()
            }
        };

        // タイミング状態・ソロ連打カウンターもリセット
        self.last_key_timestamp = None;
        self.last_key_gap_us = None;
        self.solo_counter.reset();

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
        let flush_resp = self.flush_pending(
            ContextChange::EngineDisabled,
            ComposingHint::Trusted(self.phys.composing),
        );
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

    /// 診断用: 現在の FSM 状態を短い文字列で返す。
    #[must_use]
    pub fn debug_state_label(&self) -> String {
        self.state.debug_label()
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
    /// ソロ N 連打でエンジン OFF を発動するキーを設定する。
    /// `VkCode(0)` を渡すと機能を無効にする。
    pub const fn set_engine_off_triple_vk(&mut self, vk: VkCode) {
        self.engine_off_triple_vk = vk;
    }

    /// Space 親指キーのフォールバック挙動を設定する。
    ///
    /// `space_thumb_vk` は `left_thumb_key`/`right_thumb_key` のいずれかが
    /// Space (`VK_SPACE`) に解決された場合の VK コード（Platform 層が
    /// `crate::vk::VK_SPACE` との等値比較で判定し渡す）。どちらも Space でなければ
    /// `None` を渡すこと。`ignore_composing_guard`/`shift_literal` は
    /// `GeneralConfig::space_thumb_ignore_composing_guard`/
    /// `space_thumb_shift_literal` にそのまま対応する。
    pub const fn set_space_thumb_config(
        &mut self,
        space_thumb_vk: Option<VkCode>,
        ignore_composing_guard: bool,
        shift_literal: bool,
    ) {
        self.space_thumb_vk = space_thumb_vk;
        self.space_thumb_ignore_composing_guard = ignore_composing_guard;
        self.space_thumb_shift_literal = shift_literal;
    }

    /// 無変換/変換キー単独タップの composing 中ガードの扱いを設定する。
    ///
    /// `muhenkan_vk`/`henkan_vk` は `left_thumb_key`/`right_thumb_key` がそれぞれ
    /// 無変換/変換に解決された場合の VK コード（Platform 層が判定して渡す）。
    /// 割り当てられていなければ `None` を渡すこと。各 `ignore_composing_guard` は
    /// `GeneralConfig::muhenkan_solo_tap_ignore_composing_guard`/
    /// `henkan_solo_tap_ignore_composing_guard` にそのまま対応する。
    pub const fn set_thumb_key_solo_tap_config(
        &mut self,
        muhenkan_vk: Option<VkCode>,
        muhenkan_ignore_composing_guard: bool,
        henkan_vk: Option<VkCode>,
        henkan_ignore_composing_guard: bool,
    ) {
        self.muhenkan_vk = muhenkan_vk;
        self.muhenkan_solo_tap_ignore_composing_guard = muhenkan_ignore_composing_guard;
        self.henkan_vk = henkan_vk;
        self.henkan_solo_tap_ignore_composing_guard = henkan_ignore_composing_guard;
    }

    /// triple 連打によるエンジン OFF 要求を取り出す（1ショット）。
    pub(super) fn take_engine_off_requested(&mut self) -> bool {
        std::mem::take(&mut self.engine_off_requested)
    }

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
        let flush_resp = self.flush_pending(
            ContextChange::LayoutSwapped,
            ComposingHint::Trusted(self.phys.composing),
        );
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

    /// `Face` に対応する面でキー位置を引き、仮名文字のみを返す。
    fn lookup_kana_at(&self, pos: Option<PhysicalPos>, face: Face) -> Option<char> {
        self.lookup_face(pos, self.get_face(face))
            .and_then(|(_, k)| k)
    }

    /// `PendingCharThumb` 状態で char1+thumb を同時打鍵として解決し、アクション列と OutputUpdate を返す。
    ///
    /// 親指キーの物理押下状態を「消費」する。消費後は `active_thumb_face()` が `None` を
    /// 返すようになり、後続のキーが同じ親指押下で二重にシフトされるのを防ぐ。
    fn resolve_char_thumb_as_simultaneous(
        &mut self,
        char_key: &PendingKey,
        thumb_face: Face,
    ) -> ResolvedAction {
        if let Some((action, kana)) = self.lookup_face(char_key.pos, self.get_face(thumb_face)) {
            // 親指キーを「消費」: 同じ物理押下で後続キーがシフトされないようにする
            self.consume_thumb(thumb_face);
            let output = OutputUpdate::record(char_key.scan_code, &action, kana);
            ResolvedAction {
                actions: smallvec![action],
                output,
            }
        } else {
            // 親指面に定義がない場合は文字キーを単独確定
            self.resolve_pending_char_as_single(char_key)
        }
    }

    /// 親指キーを同時打鍵に「消費済み」とマークし、同じ押下の再利用を防ぐ。
    ///
    /// 現在の物理押下タイムスタンプを記録する。物理状態が変われば（新しい KeyDown
    /// や KeyUp）タイムスタンプが不一致になり、自動的に「未消費」に戻る。
    /// 同時打鍵として消費されたことはソロ連打ではないため、ソロ連打カウンターをリセットする。
    const fn consume_thumb(&mut self, face: Face) {
        match face {
            Face::LeftThumb => self.left_thumb_consumed = self.phys.left_thumb_down,
            Face::RightThumb => self.right_thumb_consumed = self.phys.right_thumb_down,
            Face::Normal | Face::Shift => {} // 親指面以外は消費対象なし
        }
        self.solo_counter.reset();
    }

    pub(crate) const fn enter_pending_char(&mut self, key: PendingKey) {
        self.state = EngineState::PendingChar(key);
    }

    pub(crate) const fn enter_pending_thumb(&mut self, thumb: PendingThumbData) {
        self.state = EngineState::PendingThumb(thumb);
    }

    const fn enter_pending_char_thumb(&mut self, char_key: PendingKey, thumb: PendingThumbData) {
        self.state = EngineState::PendingCharThumb {
            char_key,
            thumb,
            char1_released: false,
        };
    }

    pub(crate) const fn enter_speculative_char(&mut self, key: PendingKey) {
        self.state = EngineState::SpeculativeChar(key);
    }

    /// output_history から `scan_code` のエントリを取り出し、Key(vk) なら KeyUp(vk) を `actions` に追記する。
    ///
    /// Char/Romaji は Down+Up 一括送信済みのため、Key(vk) のみが追記対象。
    fn append_key_up_for(&mut self, actions: &mut SmallVec<[KeyAction; 2]>, scan_code: ScanCode) {
        if let Some(entry) = self.output_history.remove_by_scan(scan_code) {
            if let KeyAction::Key(vk) = entry.action {
                actions.push(KeyAction::KeyUp(vk));
            }
        }
    }

    /// アクション列・consumed フラグ・タイマー指示から `Response` を組み立てる
    pub(crate) fn build_response(
        &self,
        actions: SmallVec<[KeyAction; 2]>,
        consumed: bool,
        timer: TimerIntent,
    ) -> Resp {
        let mut response = if actions.is_empty() && consumed {
            Response::consume()
        } else if actions.is_empty() {
            Response::pass_through()
        } else {
            Response::emit(actions.into_vec())
        };
        response.timers = self.timer_cmds(timer);
        response
    }
}

/// `timed_fsm::ParseAction` の具象型エイリアス（ShiftReduceParser 実装用）。
type TieredParseAction = timed_fsm::ParseAction<KeyAction, ClassifiedEvent, usize, OutputUpdate>;

// ── ShiftReduceParser 実装 ──
impl ShiftReduceParser for NicolaFsm {
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
                actions: actions.into_vec(),
                record,
                timers: self.timer_cmds(timer),
            },
            ParseAction::ReduceAndContinue {
                actions,
                record,
                remaining,
            } => TieredParseAction::ReduceAndContinue {
                actions: actions.into_vec(),
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
        if let Some(reason) = self.bypass_reason(&ev) {
            return self.handle_bypass(&ev, reason);
        }

        self.parse(ev)
    }

    /// 状態とイベントに基づいてアクションを決定し、状態遷移を行う
    fn decide_and_transition(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        // State-based dispatch (bypass is handled in on_key_down before entering the loop)
        match self.state {
            EngineState::Idle => self.decide_idle(ev),
            EngineState::PendingChar(_) => self.decide_pending_char(ev),
            EngineState::PendingThumb(_) => self.decide_pending_thumb(ev),
            EngineState::PendingCharThumb { .. } => self.step_pending_char_thumb_3key(ev),
            EngineState::SpeculativeChar(_) => self.decide_speculative(ev),
        }
    }

    /// Shift 面で Reduce する共通ヘルパー
    ///
    /// `.yab` の Shift 面に定義された値をそのまま IME 経由で確定出力する
    /// （通常の Reduce 経路と同じ、`lookup_face` が返す `KeyAction`/kana をそのまま使う）。
    /// 定義が無いキーは OS に素通しする。
    ///
    /// 2026年3月導入（`72bd118`）の Shift 面ルーティング機構そのもの。
    /// 「Shift 押しっぱなしで IME-ON 半角英数 hold」（BUG-15、`shift_plane_halfwidth`）の
    /// PassThrough/EmitText 分岐はここに実装されていたが、左Shift単独タップによる
    /// 持続トグル方式へ置き換えたため撤去した（2026-07-11、`docs/known-bugs.md`
    /// BUG-15 参照）。撤去後は本関数がこの本来の姿（.yab の値をそのまま Reduce）に戻る。
    fn shift_face_reduce(&self, ev: &ClassifiedEvent) -> ParseAction {
        let face = self.get_face(Face::Shift);
        if let Some((action, kana)) = self.lookup_face(ev.pos, face) {
            ParseAction::Reduce {
                actions: smallvec![action.clone()],
                record: OutputUpdate::record(ev.scan_code, &action, kana),
                timer: TimerIntent::CancelAll,
            }
        } else {
            ParseAction::PassThrough {
                timer: TimerIntent::Keep,
            }
        }
    }

    /// Space 親指キーを Shift と同時に押した場合、同時打鍵判定を一切試みず
    /// 即座にリテラルなスペースとして送出すべきかを判定する。
    ///
    /// NICOLA の小指シフト面（Shift 単独系）と親指シフト（同時打鍵系）はそもそも
    /// 組み合わせない設計のため、Shift 押下中の Space 親指キーを `PendingThumb` に
    /// 入れず即座に素通しにしても、通常の同時打鍵判定と衝突しない。
    const fn is_space_thumb_shift_literal(&self, ev: &ClassifiedEvent) -> bool {
        self.space_thumb_shift_literal
            && self.phys.modifiers.shift
            && ev.key_class.is_thumb()
            && matches!(self.space_thumb_vk, Some(vk) if vk.0 == ev.vk_code.0)
    }

    /// Idle 状態でのキー到着時の意図を分類する（純粋関数）。
    fn classify_idle_intent(&self, ev: &ClassifiedEvent) -> IdleIntent {
        // Shift+Space literal: 明示的なスペース入力のエスケープハッチ（最優先）。
        if self.is_space_thumb_shift_literal(ev) {
            return IdleIntent::PassThrough;
        }
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
            self.consume_thumb(face);
            ParseAction::Reduce {
                actions: smallvec![action.clone()],
                record: OutputUpdate::record(ev.scan_code, &action, kana),
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
                let pending = self.state.expect_pending_char();
                log::debug!(
                    "[passthrough-flush] pending=PendingChar(vk={:#04x}) → flush, then reprocess passthrough_vk={:#04x} ts={}us",
                    pending.vk_code.0,
                    ev.vk_code.0,
                    ev.timestamp,
                );
                self.go_idle();
                let resolved = self.resolve_pending_char_as_single(&pending);
                resolved.into_reduce_and_continue(*ev)
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
                let thumb = self.state.expect_pending_thumb();
                log::debug!(
                    "[passthrough-flush] pending=PendingThumb(vk={:#04x}) → flush, then reprocess passthrough_vk={:#04x} ts={}us",
                    thumb.vk_code.0,
                    ev.vk_code.0,
                    ev.timestamp,
                );
                self.go_idle();
                let resolved = self.resolve_pending_thumb_as_single(
                    thumb.scan_code,
                    thumb.vk_code,
                    thumb.modifier_key,
                    self.phys.composing,
                );
                resolved.into_reduce_and_continue(*ev)
            }
        }
    }

    /// SpeculativeChar 状態でのキー押下処理
    fn decide_speculative(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match ev.key_class {
            KeyClass::LeftThumb | KeyClass::RightThumb => self.step_speculative_thumb(ev),
            KeyClass::Char | KeyClass::Passthrough => {
                // 投機出力は正しかった → Idle に戻って再処理
                self.go_idle();
                ParseAction::ReduceAndContinue {
                    actions: SmallVec::new(),
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
    ///
    /// `RetractAndRecord` を使うことで、retract と record を `update_history()` で
    /// アトミックに処理し、この関数を副作用のない純粋な構築関数にする。
    fn retract_and_replace(
        pending: PendingKey,
        new_action: &KeyAction,
        kana: Option<char>,
    ) -> ParseAction {
        let actions = smallvec![
            KeyAction::SpecialKey(SpecialKey::Backspace),
            new_action.clone(),
        ];
        ParseAction::Reduce {
            actions,
            record: OutputUpdate::RetractAndRecord(OutputEntry {
                scan_code: pending.scan_code,
                romaji: new_action.romaji().to_owned(),
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
        let pending = self.state.expect_speculative_char();
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
                self.consume_thumb(face);
                self.go_idle();
                return Self::retract_and_replace(pending, &thumb_action, thumb_kana);
            }
            // Outside threshold → speculative was correct, process thumb as new key
        } else {
            // No thumb face entry → speculative was correct
        }
        // Go idle and re-process the thumb key
        self.go_idle();
        ParseAction::ReduceAndContinue {
            actions: SmallVec::new(),
            record: OutputUpdate::None,
            remaining: *ev,
        }
    }

    /// PendingChar + 親指キー → 同時打鍵候補（閾値内なら PendingCharThumb、超過なら flush+新規）
    fn step_pending_char_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let pending = self.state.expect_pending_char();
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
                    modifier_key: ev.modifier_key,
                },
            );
            return ParseAction::Shift {
                timer: TimerIntent::Pending,
            };
        }

        // 時間超過 → 前の保留を単独確定し、今回のキーを再処理
        self.go_idle();
        let resolved = self.resolve_pending_char_as_single(&pending);
        resolved.into_reduce_and_continue(*ev)
    }

    /// PendingChar + 文字キー → 前の保留を単独確定し、今回のキーを再処理
    fn step_pending_char_char(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let pending = self.state.expect_pending_char();
        self.go_idle();
        let resolved = self.resolve_pending_char_as_single(&pending);
        resolved.into_reduce_and_continue(*ev)
    }

    /// PendingThumb + 文字キー → 同時打鍵候補（閾値内なら即時確定、超過なら flush+新規）
    fn step_pending_thumb_char(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let thumb = self.state.expect_pending_thumb();
        // 親指面で到着文字キーの候補を取得し閾値を調整
        let pending_face = thumb.face();
        let candidate = self.lookup_face(ev.pos, self.get_face(pending_face));
        let candidate_kana = candidate.as_ref().and_then(|(_, kana)| *kana);

        if self
            .timing_judge()
            .is_simultaneous(thumb.timestamp, ev.timestamp, candidate_kana)
        {
            if let Some((action, kana)) = candidate {
                // 保留=親指, 到着=文字 → 同時打鍵
                // 親指を消費: 同じ押下で後続キーが二重シフトされるのを防ぐ
                self.consume_thumb(thumb.face());
                self.go_idle();
                return ParseAction::Reduce {
                    actions: smallvec![action.clone()],
                    record: OutputUpdate::record(ev.scan_code, &action, kana),
                    timer: TimerIntent::CancelAll,
                };
            }
        }

        // 時間超過 or 候補なし → 前の保留を単独確定し、今回のキーを再処理
        self.go_idle();
        let resolved = self.resolve_pending_thumb_as_single(
            thumb.scan_code,
            thumb.vk_code,
            thumb.modifier_key,
            self.phys.composing,
        );
        resolved.into_reduce_and_continue(*ev)
    }

    /// PendingThumb + 親指キー → 前の保留を単独確定し、今回のキーを再処理
    fn step_pending_thumb_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let thumb = self.state.expect_pending_thumb();
        self.go_idle();
        let resolved = self.resolve_pending_thumb_as_single(
            thumb.scan_code,
            thumb.vk_code,
            thumb.modifier_key,
            self.phys.composing,
        );
        resolved.into_reduce_and_continue(*ev)
    }

    /// OutputUpdate に基づいて出力履歴を更新する共通ヘルパー
    pub(crate) fn update_history(&mut self, output: OutputUpdate) {
        match output {
            OutputUpdate::Record(entry) => {
                self.output_history.push(entry);
            }
            OutputUpdate::RetractAndRecord(entry) => {
                self.output_history.retract_last();
                self.output_history.push(entry);
            }
            OutputUpdate::None => {}
        }
    }

    /// 保留中の文字キーを単独打鍵として解決し、アクション列と OutputUpdate を返す
    fn resolve_pending_char_as_single(&self, pending: &PendingKey) -> ResolvedAction {
        if let Some((action, kana)) = self.lookup_face(pending.pos, self.get_face(Face::Normal)) {
            let output = OutputUpdate::record(pending.scan_code, &action, kana);
            ResolvedAction {
                actions: smallvec![action],
                output,
            }
        } else {
            // Normal face に定義なし（yab に明示的に '無' がある場合は lookup_face が
            // Some(Suppress) を返すため、ここには来ない）。
            // 配列定義外のキー → Key(vk_code) でそのまま通す
            let action = KeyAction::Key(pending.vk_code);
            let output = OutputUpdate::record(pending.scan_code, &action, None);
            ResolvedAction {
                actions: smallvec![action],
                output,
            }
        }
    }

    /// 保留中の親指キーを単独打鍵として解決し、アクション列と `OutputUpdate` を返す。
    ///
    /// NICOLA では親指キー (無変換 / 変換) は文字キーとの同時打鍵専用であり、
    /// 単独打鍵は本質的に誤打鍵 / 親指の離しの遅れ / 文字キーが間に合わなかった等の
    /// 偶発的なケース。元の VK_NONCONVERT / VK_CONVERT を OS に送ってしまうと
    /// IME 側で カタカナトグル等の副作用（Microsoft IME のデフォルト挙動）が起こり、
    /// 入力モードが意図せず切り替わる。
    ///
    /// したがって composing 中は何も送出しない（suppress）。これにより親指の単独打鍵は
    /// composing 中は完全に無視され、IME に対して透明になる。Engine が無効な場合は
    /// hook 層で bypass されてここには来ないので、Windows 全般での 無変換 / 変換 キー
    /// 機能は composing していない場面では引き続き使える。
    ///
    /// **Space の例外**: `space_thumb_vk` に一致し `space_thumb_ignore_composing_guard`
    /// が true の場合、composing 中でも常に生 VK_SPACE を送出する。MS-IME/Google 日本語
    /// 入力とも Space による「変換候補送り」は正規機能であり、無変換/変換と同じ理由
    /// （かな/カタカナ切替・再変換の誤発火防止）で composing 中に抑制すると、通常の
    /// 変換操作そのものが壊れるため。
    ///
    /// **無変換/変換の明示的なオプトアウト**: `muhenkan_vk`/`henkan_vk` に一致し、
    /// それぞれ対応する `*_solo_tap_ignore_composing_guard` が true の場合も同様に
    /// composing 中でも生 VK を送出する。既定値は `false`（従来通り抑制）で、
    /// ユーザーが明示的に有効化した場合のみ、上記のかな/カタカナ切替等の副作用
    /// リスクを引き受けて composing 中の単独タップを素通しさせる。
    ///
    /// タイムアウト経路（`timeout_pending_thumb`）とフラッシュ経路（`flush_pending`、
    /// `decide_pending_thumb` の Passthrough 割り込み、`step_pending_thumb_char`/
    /// `step_pending_thumb_thumb`）の双方から共通で呼ぶ。以前はフラッシュ経路が
    /// `composing`/VK 種別を一切見ずに常時 suppress していたため、フォーカス変更や
    /// 別キー割り込みで Space が消えることがあった（この不整合を解消するために
    /// `composing`/`modifier_key` を明示的に受け取る形にした）。
    fn resolve_pending_thumb_as_single(
        &self,
        scan_code: ScanCode,
        vk_code: VkCode,
        modifier_key: Option<crate::types::ModifierKey>,
        composing: bool,
    ) -> ResolvedAction {
        // 親指キーが OS 修飾キー（Ctrl/Shift/Alt/Meta）に割り当てられている場合は
        // composing に関わらず常に suppress する（Alt 単独送出の副作用回避）。
        if modifier_key.is_some() {
            return ResolvedAction {
                actions: SmallVec::new(),
                output: OutputUpdate::None,
            };
        }

        let is_space_with_fallback =
            self.space_thumb_vk == Some(vk_code) && self.space_thumb_ignore_composing_guard;
        let is_muhenkan_with_fallback =
            self.muhenkan_vk == Some(vk_code) && self.muhenkan_solo_tap_ignore_composing_guard;
        let is_henkan_with_fallback =
            self.henkan_vk == Some(vk_code) && self.henkan_solo_tap_ignore_composing_guard;
        let ignore_composing_guard =
            is_space_with_fallback || is_muhenkan_with_fallback || is_henkan_with_fallback;
        if composing && !ignore_composing_guard {
            return ResolvedAction {
                actions: SmallVec::new(),
                output: OutputUpdate::None,
            };
        }

        let action = KeyAction::Key(vk_code);
        let output = OutputUpdate::record(scan_code, &action, None);
        ResolvedAction {
            actions: smallvec![action],
            output,
        }
    }

    /// 3 鍵仲裁で char1+thumb を優先するかを判定する（純粋関数）。
    ///
    /// char1 が既に離されていれば無条件で false（タイミング比較不要）。
    /// それ以外は `TimingJudge::three_key_pairing` でタイミング + n-gram を総合判定。
    fn compute_prefer_char1(
        &self,
        pending: &PendingKey,
        thumb: &PendingThumbData,
        ev: &ClassifiedEvent,
        char1_released: bool,
    ) -> bool {
        if char1_released {
            return false;
        }
        let thumb_face = thumb.face();
        let judge = self.timing_judge();
        let char1_thumb_kana = self.lookup_kana_at(pending.pos, thumb_face);
        let char1_single_kana = self.lookup_kana_at(pending.pos, Face::Normal);
        let char2_thumb_kana = self.lookup_kana_at(ev.pos, thumb_face);
        judge.three_key_pairing(
            pending.timestamp,
            thumb.timestamp,
            ev.timestamp,
            char1_thumb_kana,
            char1_single_kana,
            char2_thumb_kana,
        ) == timing::ThreeKeyResult::PairWithChar1
    }

    /// char1+thumb を同時打鍵として確定し、`remaining` を再処理する `ReduceAndContinue` を返す。
    fn reduce_char_thumb_and_continue(
        &mut self,
        pending: PendingKey,
        thumb_face: Face,
        remaining: ClassifiedEvent,
    ) -> ParseAction {
        let resolved = self.resolve_char_thumb_as_simultaneous(&pending, thumb_face);
        resolved.into_reduce_and_continue(remaining)
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
        let (pending, thumb, char1_released) = self.state.expect_pending_char_thumb();
        let thumb_face = thumb.face();
        self.go_idle();

        // 新しい親指キーが来た → char1+thumb を同時打鍵として確定し、新しい親指を再処理
        if ev.key_class.is_thumb() {
            return self.reduce_char_thumb_and_continue(pending, thumb_face, *ev);
        }

        // char2 が来た → 3 鍵仲裁
        if self.compute_prefer_char1(&pending, &thumb, ev, char1_released) {
            // char1+thumb = 同時打鍵、char2 は再処理
            return self.reduce_char_thumb_and_continue(pending, thumb_face, *ev);
        }

        // char1 = 単独、char2+thumb = 同時打鍵（または char2 単独）
        let char1_resolved = self.resolve_pending_char_as_single(&pending);
        if let Some((action, kana)) = self.lookup_face(ev.pos, self.get_face(thumb_face)) {
            // char1 の履歴を先に更新してから char2+thumb を確定
            self.update_history(char1_resolved.output);
            self.consume_thumb(thumb_face);
            let mut all_actions = char1_resolved.actions;
            all_actions.push(action.clone());
            return ParseAction::Reduce {
                actions: all_actions,
                record: OutputUpdate::record(ev.scan_code, &action, kana),
                timer: TimerIntent::CancelAll,
            };
        }
        // 親指面に char2 の定義がない → char1 単独確定、char2 を再処理
        char1_resolved.into_reduce_and_continue(*ev)
    }
}

// ── KeyUp 処理 ──
impl NicolaFsm {
    fn on_key_up(&mut self, event: &RawKeyEvent) -> Resp {
        // phys.classified は on_key_down 側で使用済み

        // PendingCharThumb 状態での KeyUp 処理
        if let EngineState::PendingCharThumb {
            char_key, thumb, ..
        } = self.state
        {
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

        // OS modifier (Ctrl/Alt/Win) 保持中: on_key_down と対称にバイパス。
        // output_history に古いエントリが残っていても誤 Suppress しない。
        if self.phys.modifiers.is_os_modifier_held() {
            return Response::pass_through();
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
        let (pending, thumb, char1_released) = self.state.expect_pending_char_thumb();

        // char1 の最初の KeyUp → フラグを立てて待機継続。
        // 後から char2 が来れば「char1 単独 + char2+thumb 同時」と確実に判定できる。
        if event.vk_code == pending.vk_code && !char1_released {
            self.state = EngineState::PendingCharThumb {
                char_key: pending,
                thumb,
                char1_released: true,
            };
            return self.build_response(SmallVec::new(), true, TimerIntent::Keep);
        }

        // char1+thumb を同時打鍵として確定する
        self.go_idle();
        let resolved = self.resolve_char_thumb_as_simultaneous(&pending, thumb.face());
        self.update_history(resolved.output);
        let mut actions = resolved.actions;

        // どの物理キーが離されたかに応じて char1 の KeyUp 追記を判定
        let key_up_scan = if event.vk_code == pending.vk_code {
            // char1 が再度離された (char1_released=true 済み)
            Some(event.scan_code)
        } else if char1_released {
            // thumb が離された + char1 は既に物理的に離されている
            Some(pending.scan_code)
        } else {
            // thumb が離された + char1 はまだ押下中 → KeyUp 不要
            None
        };
        if let Some(scan) = key_up_scan {
            self.append_key_up_for(&mut actions, scan);
        }
        self.build_response(actions, true, TimerIntent::CancelAll)
    }

    /// 保留中のキーが離された場合、保留を単独確定して KeyUp を処理する
    fn handle_key_up_pending(&mut self, event: &RawKeyEvent) -> Resp {
        let old_state = std::mem::replace(&mut self.state, EngineState::Idle);

        let resolved = match old_state {
            EngineState::PendingChar(pending) => self.resolve_pending_char_as_single(&pending),
            EngineState::PendingThumb(thumb) => self.resolve_pending_thumb_as_single(
                thumb.scan_code,
                thumb.vk_code,
                thumb.modifier_key,
                self.phys.composing,
            ),
            EngineState::Idle
            | EngineState::PendingCharThumb { .. }
            | EngineState::SpeculativeChar(_) => {
                log::error!(
                    "unexpected state in handle_key_up_pending: {:?}",
                    self.state
                );
                ResolvedAction {
                    actions: SmallVec::new(),
                    output: OutputUpdate::None,
                }
            }
        };
        self.update_history(resolved.output);
        let mut result = resolved.actions;
        self.append_key_up_for(&mut result, event.scan_code);
        // Unicode 文字 (Char) は Down+Up 一括送信済みなので KeyUp 追加不要
        self.build_response(result, true, TimerIntent::CancelAll)
    }

    /// output_history から対応する注入済みキーを探してリリースする
    fn handle_key_up_active(&mut self, event: &RawKeyEvent) -> Resp {
        if let Some(entry) = self.output_history.remove_by_scan(event.scan_code) {
            return match entry.action {
                // Unicode 文字やローマ字列の場合、KeyUp は不要（押下時に入力完了）
                KeyAction::Char(_) | KeyAction::Romaji(_) => self.build_response(
                    smallvec![KeyAction::Suppress],
                    true,
                    TimerIntent::CancelAll,
                ),
                KeyAction::Key(vk) => self.build_response(
                    smallvec![KeyAction::KeyUp(vk)],
                    true,
                    TimerIntent::CancelAll,
                ),
                _ => Response::pass_through(),
            };
        }
        Response::pass_through()
    }
}

// ── タイムアウト処理 ──
impl NicolaFsm {
    /// PendingChar タイムアウト：文字キーを単独打鍵として確定する
    fn timeout_pending_char(&mut self, pending: &PendingKey) -> Resp {
        let resolved = self.resolve_pending_char_as_single(pending);
        self.update_history(resolved.output);
        self.build_response(resolved.actions, true, TimerIntent::CancelAll)
    }

    /// PendingThumb タイムアウト：親指キーを単独打鍵として確定する
    fn timeout_pending_thumb(
        &mut self,
        scan_code: ScanCode,
        vk_code: VkCode,
        timestamp: Timestamp,
        composing: bool,
        modifier_key: Option<crate::types::ModifierKey>,
    ) -> Resp {
        // ソロ連打によるエンジン OFF トリガーチェック
        if self.engine_off_triple_vk.0 != 0 && vk_code == self.engine_off_triple_vk {
            let count = self.solo_counter.record(vk_code, timestamp);
            if count >= SOLO_OFF_TRIGGER_COUNT {
                self.solo_counter.reset();
                self.engine_off_requested = true;
                // N 回目は suppress（OS への VK 送出を防ぐ）
                return self.build_response(SmallVec::new(), true, TimerIntent::CancelAll);
            }
        } else {
            self.solo_counter.reset();
        }

        // scan_code には物理キーの実スキャンコードを使う。
        // 以前は ScanCode(u32::from(vk_code.0)) という合成値を使っていたが、
        // VK_CONVERT (VK=0x1C) の合成スキャンコードが Enter の物理スキャンコード (0x1C) と
        // 衝突し、後から Enter KeyUp が来たときに誤って KeyUp(VK_CONVERT) が送出されていた。
        //
        // suppress/送出の判定（composing ガード・Space 例外・OS 修飾キーガード）は
        // resolve_pending_thumb_as_single に委譲し、flush 経路と挙動を統一する。
        let resolved =
            self.resolve_pending_thumb_as_single(scan_code, vk_code, modifier_key, composing);
        self.update_history(resolved.output);
        self.build_response(resolved.actions, true, TimerIntent::CancelAll)
    }

    /// PendingCharThumb タイムアウト：char1+thumb を同時打鍵として確定する
    fn timeout_pending_char_thumb(
        &mut self,
        char_key: &PendingKey,
        thumb_face: Face,
        char1_released: bool,
    ) -> Resp {
        let resolved = self.resolve_char_thumb_as_simultaneous(char_key, thumb_face);
        self.update_history(resolved.output);
        let mut actions = resolved.actions;
        if char1_released {
            // char1 は既に物理的に離されている → Key 出力があれば KeyUp も追加
            if let Some(entry) = self.output_history.remove_by_scan(char_key.scan_code) {
                if let KeyAction::Key(vk) = entry.action {
                    actions.push(KeyAction::KeyUp(vk));
                }
            }
        }
        self.build_response(actions, true, TimerIntent::CancelAll)
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
                    self.update_history(OutputUpdate::record(pending.scan_code, &action, kana));
                    self.build_response(
                        smallvec![action],
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
                log::warn!("TIMER_SPECULATIVE fired in unexpected state: {other:?}");
                Response::pass_through().with_kill_timer(TIMER_SPECULATIVE)
            }
        }
    }
}

// ── バイパス ──
impl NicolaFsm {
    /// 親指キーが消費済み（同時打鍵に使用済み）かどうかを返す。
    ///
    /// 消費タイムスタンプが現在の物理押下と一致すれば消費済み。
    /// 物理状態が変わると自動的に不一致になるため、明示的なリセットは不要。
    fn is_thumb_consumed(&self, face: Face) -> bool {
        let (phys_down, consumed) = match face {
            Face::LeftThumb => (self.phys.left_thumb_down, self.left_thumb_consumed),
            Face::RightThumb => (self.phys.right_thumb_down, self.right_thumb_consumed),
            Face::Normal | Face::Shift => return false,
        };
        phys_down.is_some() && consumed == phys_down
    }

    /// 現在押下中かつ未消費の親指キーに対応するシフト面を返す。
    fn active_thumb_face(&self) -> Option<Face> {
        if self.phys.left_thumb_down.is_some() && !self.is_thumb_consumed(Face::LeftThumb) {
            Some(Face::LeftThumb)
        } else if self.phys.right_thumb_down.is_some() && !self.is_thumb_consumed(Face::RightThumb)
        {
            Some(Face::RightThumb)
        } else {
            None
        }
    }

    /// いずれかの配列面に非 None の出力定義があるキーかどうか。
    ///
    /// YabValue::None（'無'）は「その面では出力なし」を明示するが配列キーではないため除外する。
    /// これにより、全面が '無' のキーはパススルー扱いとなり、
    /// Shift面など一部に定義がある場合のみ NICOLA 処理対象となる。
    pub(crate) fn is_layout_key(&self, pos: Option<PhysicalPos>) -> bool {
        let Some(pos) = pos else {
            return false;
        };
        let has_output =
            |face: &YabFace| face.get(&pos).is_some_and(|v| !matches!(v, YabValue::None));
        has_output(self.get_face(Face::Normal))
            || has_output(self.get_face(Face::LeftThumb))
            || has_output(self.get_face(Face::RightThumb))
            || has_output(self.get_face(Face::Shift))
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
    fn handle_bypass(&mut self, ev: &ClassifiedEvent, reason: BypassReason) -> Resp {
        // バイパスされたキーの output_history エントリを削除する。
        // OsModifierHeld で J↓ がバイパスされた後、modifier が J↑ より先にリリースされると
        // on_key_up の is_os_modifier_held() チェックが通らず、output_history に前回の
        // NICOLA 組み合わせのエントリが残っていると J↑ が誤って Suppress される。
        self.output_history.remove_by_scan(ev.scan_code);
        if self.state.is_idle() {
            return Response::pass_through();
        }
        log::debug!(
            "handle_bypass: vk=0x{:02X} reason={:?} state={}",
            ev.vk_code.0,
            reason,
            self.state.debug_label(),
        );
        let flush = self.flush_pending(
            ContextChange::BypassKey,
            ComposingHint::Trusted(self.phys.composing),
        );
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
    /// `composing` は IME composition が現在進行中か（`InputContext::composing` 由来）。
    pub fn on_timeout(
        &mut self,
        timer_id: usize,
        phys: &PhysicalKeyState,
        composing: bool,
    ) -> Resp {
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
            EngineState::PendingChar(pending) => self.timeout_pending_char(&pending),
            EngineState::PendingThumb(thumb) => self.timeout_pending_thumb(
                thumb.scan_code,
                thumb.vk_code,
                thumb.timestamp,
                composing,
                thumb.modifier_key,
            ),
            EngineState::PendingCharThumb {
                char_key,
                thumb,
                char1_released,
            } => self.timeout_pending_char_thumb(&char_key, thumb.face(), char1_released),
            // 投機出力済み → タイムアウト = 親指キー未到着 → 投機出力は正しかった → Idle へ
            EngineState::SpeculativeChar(_) => Response::consume().with_kill_timer(TIMER_PENDING),
        }
    }
}
