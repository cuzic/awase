pub mod output_history;
mod types;

use std::time::Duration;

use timed_fsm::{Response, TimedStateMachine};

use crate::config::ConfirmMode;
use crate::engine::output_history::{OutputEntry, OutputHistory};
use crate::ngram::NgramModel;
use crate::scanmap::scan_to_pos;
use crate::types::{
    ContextChange, KeyAction, KeyEventType, RawKeyEvent, ScanCode, Timestamp, VkCode,
};
use crate::yab::{YabFace, YabLayout, YabValue};

use types::{
    BypassReason, EnginePhase, Face, ModifierState, PendingKey, PendingThumbData, ResolvedAction,
};
pub use types::{ClassifiedEvent, FinalizePlan, KeyClass, OutputRecord, OutputUpdate, TimerIntent};

/// 同時打鍵判定用タイマー ID
pub const TIMER_PENDING: usize = 1;

/// TwoPhase モード: Phase 1（短い待機）→ Phase 2（投機出力）遷移用タイマー ID
pub const TIMER_SPECULATIVE: usize = 2;

/// AdaptiveTiming モードで連続打鍵と判定する閾値（マイクロ秒）
const CONTINUOUS_KEYSTROKE_THRESHOLD_US: u64 = 80_000;

/// VK_BACK (Backspace) の仮想キーコード
const VK_BACK: VkCode = VkCode(0x08);

/// `Response` の型エイリアス
type Resp = Response<KeyAction, usize>;

impl From<&YabValue> for KeyAction {
    fn from(value: &YabValue) -> Self {
        match value {
            YabValue::Romaji { romaji, .. } => Self::Romaji(romaji.clone()),
            YabValue::Literal(s) => s.chars().next().map_or(Self::Suppress, Self::Char),
            YabValue::Special(sk) => Self::Key(sk.to_vk()),
            YabValue::None => Self::Suppress,
        }
    }
}

#[cfg(test)]
/// `YabValue` をエンジン出力用の `KeyAction` に変換する（`From` トレイトへの委譲）。
fn yab_value_to_action(value: &YabValue) -> KeyAction {
    KeyAction::from(value)
}

/// 配列変換エンジン（状態機械 + 同時打鍵判定）
#[allow(missing_debug_implementations)]
pub struct Engine {
    /// 配列定義（.yab ベース）
    layout: YabLayout,

    /// 左親指キーの仮想キーコード
    left_thumb_vk: VkCode,

    /// 右親指キーの仮想キーコード
    right_thumb_vk: VkCode,

    /// エンジンのフェーズ（状態タグ）
    phase: EnginePhase,

    /// 保留中の文字キーデータ
    pending_char: Option<PendingKey>,

    /// 保留中の親指キーデータ
    pending_thumb: Option<PendingThumbData>,

    /// 修飾キーの押下状態
    modifiers: ModifierState,

    /// 左親指キーが押下中か（押下時刻を保持）
    left_thumb_down: Option<Timestamp>,

    /// 右親指キーが押下中か（押下時刻を保持）
    right_thumb_down: Option<Timestamp>,

    /// 同時打鍵の判定閾値（マイクロ秒）
    threshold_us: u64,

    /// エンジンの有効/無効
    enabled: bool,

    /// n-gram モデル（None なら固定閾値にフォールバック）
    ngram_model: Option<NgramModel>,

    /// 確定モード
    confirm_mode: ConfirmMode,

    /// 投機出力までの待機時間（マイクロ秒）
    speculative_delay_us: u64,

    /// 直前のキー押下時刻（AdaptiveTiming 用）
    last_key_timestamp: Option<Timestamp>,

    /// 直前のキーとの間隔（マイクロ秒）。on_key_down の冒頭で算出。
    last_key_gap_us: Option<u64>,

    /// 出力履歴（押下中キーの追跡と直近出力の記録を統合管理）
    output_history: OutputHistory,
}

/// `KeyAction` からローマ字文字列を抽出するヘルパー
fn romaji_of(action: &KeyAction) -> String {
    match action {
        KeyAction::Romaji(s) => s.clone(),
        _ => String::new(),
    }
}

/// `OutputUpdate::Record` を生成するヘルパー
fn record_output(scan_code: ScanCode, action: &KeyAction, kana: Option<char>) -> OutputUpdate {
    OutputUpdate::Record(OutputRecord {
        scan_code,
        romaji: romaji_of(action),
        kana,
        action: action.clone(),
    })
}

// ── 公開 API ──
impl Engine {
    #[must_use]
    pub fn new(
        layout: YabLayout,
        left_thumb_vk: VkCode,
        right_thumb_vk: VkCode,
        threshold_ms: u32,
        confirm_mode: ConfirmMode,
        speculative_delay_ms: u32,
    ) -> Self {
        Self {
            layout,
            left_thumb_vk,
            right_thumb_vk,
            phase: EnginePhase::Idle,
            pending_char: None,
            pending_thumb: None,
            modifiers: ModifierState::default(),
            left_thumb_down: None,
            right_thumb_down: None,
            threshold_us: u64::from(threshold_ms) * 1000,
            enabled: true,
            ngram_model: None,
            confirm_mode,
            speculative_delay_us: u64::from(speculative_delay_ms) * 1000,
            last_key_timestamp: None,
            last_key_gap_us: None,
            output_history: OutputHistory::new(),
        }
    }

    /// Idle 状態に遷移するヘルパー
    const fn go_idle(&mut self) {
        self.phase = EnginePhase::Idle;
        self.pending_char = None;
        self.pending_thumb = None;
    }

    /// 保留中のキーを安全に解消し、Idle 状態に戻す。
    ///
    /// 外部コンテキスト変更（IMEオフ、エンジン無効化、言語切替、レイアウト差替え等）
    /// 時に呼ぶ。現在の外部コンテキストではもう待てないので、保留を解消して安全側に倒す。
    ///
    /// # 事後条件
    /// - `state` は `Idle`
    /// - `TIMER_PENDING` / `TIMER_SPECULATIVE` は停止済み（Response に含��れる）
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
        let was_idle = self.phase == EnginePhase::Idle;
        let old_phase = self.phase;
        let old_char = self.pending_char.take();
        let old_thumb = self.pending_thumb.take();
        self.phase = EnginePhase::Idle;

        let response = match old_phase {
            EnginePhase::Idle => {
                // Already idle — no-op
                Response::consume()
            }
            EnginePhase::PendingChar => {
                // 保留中の文字キーを通常面で単独確定
                let pending = old_char.expect("PendingChar phase requires pending_char");
                let resolved =
                    self.resolve_pending_char_as_single(pending.scan_code, pending.vk_code);
                self.update_history(resolved.output);
                Response::emit(resolved.actions)
            }
            EnginePhase::PendingThumb => {
                // 保留中の親指キーを単独確定
                let thumb = old_thumb.expect("PendingThumb phase requires pending_thumb");
                let resolved = self.resolve_pending_thumb_as_single(thumb.vk_code);
                self.update_history(resolved.output);
                Response::emit(resolved.actions)
            }
            EnginePhase::PendingCharThumb => {
                // 文字+親指を同時打鍵として確定
                let pending = old_char.expect("PendingCharThumb phase requires pending_char");
                let thumb = old_thumb.expect("PendingCharThumb phase requires pending_thumb");
                let resolved = self.resolve_char_thumb_as_simultaneous(
                    pending.scan_code,
                    pending.vk_code,
                    thumb.is_left,
                    thumb.timestamp,
                );
                self.update_history(resolved.output);
                Response::emit(resolved.actions)
            }
            EnginePhase::SpeculativeChar => {
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
    /// 戻り値の `Resp` を `dispatch()` で処理すること（タイマー停��� + 保留キー確定）。
    pub fn toggle_enabled(&mut self) -> (bool, Resp) {
        let flush_resp = self.flush_pending(ContextChange::EngineDisabled);
        self.enabled = !self.enabled;
        self.output_history.clear();
        // エンジン OFF 中は on_event が早期 return するため modifiers.update() が
        // 呼ばれず、OFF 中に離された修飾キーの KeyUp を見逃す。
        // 再 ON 時に Ctrl/Alt が stuck して全キーが OsModifierHeld バイパスされるのを防止。
        self.modifiers = ModifierState::default();
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

    /// n-gram モデルに基づいて閾値を動的に調整する。
    ///
    /// モデルが未設定の場合は固定閾値 (`threshold_us`) を返す。
    fn adjusted_threshold_us(&self, candidate: char) -> u64 {
        self.ngram_model
            .as_ref()
            .map_or(self.threshold_us, |model| {
                let recent = self.output_history.recent_kana(3);
                model.adjusted_threshold(&recent, candidate)
            })
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
impl Engine {
    /// Face 列挙値に対応する YabFace への参照を返す
    const fn get_face(&self, face: Face) -> &YabFace {
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
    fn lookup_face(
        &self,
        scan_code: ScanCode,
        _vk_code: VkCode,
        face: &YabFace,
    ) -> Option<(KeyAction, Option<char>)> {
        let pos = scan_to_pos(scan_code.0)?;
        let value = face.get(&pos)?;
        let kana = match value {
            YabValue::Romaji { kana, .. } => *kana,
            YabValue::Literal(s) => s.chars().next(),
            _ => None,
        };
        Some((KeyAction::from(value), kana))
    }

    /// `PendingCharThumb` 状態で char1+thumb を同時打鍵として解決し、アクション列と OutputUpdate を返す
    fn resolve_char_thumb_as_simultaneous(
        &mut self,
        char_scan: ScanCode,
        char_vk: VkCode,
        thumb_is_left: bool,
        thumb_timestamp: Timestamp,
    ) -> ResolvedAction {
        let thumb_face = Face::from_thumb_bool(thumb_is_left);
        if let Some((action, kana)) =
            self.lookup_face(char_scan, char_vk, self.get_face(thumb_face))
        {
            if thumb_is_left {
                self.left_thumb_down = Some(thumb_timestamp);
            } else {
                self.right_thumb_down = Some(thumb_timestamp);
            }
            let output = record_output(char_scan, &action, kana);
            ResolvedAction {
                actions: vec![action],
                output,
            }
        } else {
            // 親指面に定義がない場合は文字キーを単独確定
            self.resolve_pending_char_as_single(char_scan, char_vk)
        }
    }

    const fn enter_pending_char(&mut self, key: PendingKey) {
        self.phase = EnginePhase::PendingChar;
        self.pending_char = Some(key);
        self.pending_thumb = None;
    }

    const fn enter_pending_thumb(&mut self, thumb: PendingThumbData) {
        self.phase = EnginePhase::PendingThumb;
        self.pending_char = None;
        self.pending_thumb = Some(thumb);
    }

    const fn enter_pending_char_thumb(&mut self, char_key: PendingKey, thumb: PendingThumbData) {
        self.phase = EnginePhase::PendingCharThumb;
        self.pending_char = Some(char_key);
        self.pending_thumb = Some(thumb);
    }

    const fn enter_speculative_char(&mut self, key: PendingKey) {
        self.phase = EnginePhase::SpeculativeChar;
        self.pending_char = Some(key);
        self.pending_thumb = None;
    }

    /// `FinalizePlan` を `Response` に変換する
    fn finalize_plan(&mut self, plan: FinalizePlan) -> Resp {
        // 1. 出力履歴の更新
        self.update_history(plan.output);

        // 2. Response 構築
        let response = if plan.actions.is_empty() {
            Response::consume()
        } else {
            Response::emit(plan.actions)
        };

        // 3. タイマー命令付与
        match plan.timer {
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

// ── KeyDown ディスパッチ ──
impl Engine {
    /// AdaptiveTiming 用: 直前キーとの間隔を算出してタイムスタンプを更新する
    fn update_timing(&mut self, event: &RawKeyEvent) {
        self.last_key_gap_us = self
            .last_key_timestamp
            .map(|prev| event.timestamp.saturating_sub(prev));
        self.last_key_timestamp = Some(event.timestamp);
    }

    /// Shift 面を使うべきかどうかを判定する
    const fn should_use_shift_plane(&self, ev: &ClassifiedEvent) -> bool {
        self.modifiers.shift && !ev.key_class.is_thumb()
    }

    fn on_key_down(&mut self, event: &RawKeyEvent) -> Resp {
        self.update_timing(event);
        let ev = self.classify(event);

        if let Some(reason) = self.bypass_reason(&ev) {
            return self.handle_bypass(reason);
        }

        // Shift 面：Shift が押下中かつ親指キーでない場合、配列の Shift 面を参照
        // （Idle 以外の状態からも Shift 面へ早期リターンする。handle_idle 内にも
        //   同じチェックがあるが、そちらは pending ハンドラ経由で呼ばれる場合用。）
        if self.should_use_shift_plane(&ev) {
            return self.handle_shift(&ev);
        }

        self.dispatch_key_down(&ev)
    }

    /// (phase, key_class) ディスパッチテーブル
    fn dispatch_key_down(&mut self, ev: &ClassifiedEvent) -> Resp {
        match self.phase {
            EnginePhase::Idle => self.handle_idle(ev),
            EnginePhase::PendingChar => match ev.key_class {
                KeyClass::LeftThumb | KeyClass::RightThumb => self.handle_pending_char_thumb(ev),
                KeyClass::Char => self.handle_pending_char_char(ev),
                KeyClass::Passthrough => {
                    log::error!("unexpected Passthrough in PendingChar");
                    Response::pass_through()
                }
            },
            EnginePhase::PendingThumb => match ev.key_class {
                KeyClass::Char => self.handle_pending_thumb_char(ev),
                KeyClass::LeftThumb | KeyClass::RightThumb => self.handle_pending_thumb_thumb(ev),
                KeyClass::Passthrough => {
                    log::error!("unexpected Passthrough in PendingThumb");
                    Response::pass_through()
                }
            },
            EnginePhase::PendingCharThumb => self.resolve_pending_char_thumb(ev),
            EnginePhase::SpeculativeChar => match ev.key_class {
                KeyClass::LeftThumb | KeyClass::RightThumb => self.handle_speculative_thumb(ev),
                KeyClass::Char => self.handle_idle(ev),
                KeyClass::Passthrough => {
                    log::error!("unexpected Passthrough in SpeculativeChar");
                    Response::pass_through()
                }
            },
        }
    }

    /// Shift 面の処理（状態非依存）
    fn handle_shift(&mut self, ev: &ClassifiedEvent) -> Resp {
        if let Some((action, kana)) =
            self.lookup_face(ev.scan_code, ev.vk_code, self.get_face(Face::Shift))
        {
            self.finalize_plan(FinalizePlan {
                actions: vec![action.clone()],
                timer: TimerIntent::CancelAll,
                output: record_output(ev.scan_code, &action, kana),
            })
        } else {
            // Shift 面に定義がないキーは OS に任せる
            Response::pass_through()
        }
    }
}

// ── ConfirmMode ハンドラ ──
impl Engine {
    /// Idle 状態での新規キー押下処理
    fn handle_idle(&mut self, ev: &ClassifiedEvent) -> Resp {
        // Shift 面チェック: on_key_down の Shift チェックは直接呼び出し時のみ有効。
        // handle_idle は pending ハンドラ（handle_pending_char_char 等）からも
        // 呼ばれるため、ここにも Shift チェックが必要。
        if self.should_use_shift_plane(ev) {
            return self.handle_shift(ev);
        }

        // 親指キーが既に押下中なら即時同時打鍵（モード非依存）
        if !ev.key_class.is_thumb() {
            if let Some(face) = self.active_thumb_face() {
                if let Some((action, kana)) =
                    self.lookup_face(ev.scan_code, ev.vk_code, self.get_face(face))
                {
                    return self.finalize_plan(FinalizePlan {
                        actions: vec![action.clone()],
                        timer: TimerIntent::CancelAll,
                        output: record_output(ev.scan_code, &action, kana),
                    });
                }
            }
        }

        // 配列定義にもなく親指キーでもないキーは即座に素通しする（モード非依存）
        // （Enter, Backspace, Tab 等に不要な 100ms 遅延を生じさせない）
        if !ev.key_class.is_thumb() && !self.is_layout_key(ev.scan_code) {
            return Response::pass_through();
        }

        // 確定モードに応じた保留処理へディスパッチ
        match self.confirm_mode {
            ConfirmMode::Wait => self.handle_idle_wait(ev),
            ConfirmMode::Speculative => self.handle_idle_speculative(ev),
            ConfirmMode::TwoPhase => self.handle_idle_two_phase(ev),
            ConfirmMode::AdaptiveTiming => {
                let is_continuous = self
                    .last_key_gap_us
                    .is_some_and(|gap| gap < CONTINUOUS_KEYSTROKE_THRESHOLD_US);
                if is_continuous {
                    self.handle_idle_wait(ev)
                } else {
                    self.handle_idle_two_phase(ev)
                }
            }
            ConfirmMode::NgramPredictive => self.handle_idle_ngram_predictive(ev),
        }
    }

    /// Idle + Wait モード: 新規キーを保留状態に遷移させタイマーを起動する
    fn handle_idle_wait(&mut self, ev: &ClassifiedEvent) -> Resp {
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
                timestamp: ev.timestamp,
            });
        }
        self.finalize_plan(FinalizePlan {
            actions: vec![],
            timer: TimerIntent::Pending,
            output: OutputUpdate::None,
        })
    }

    /// Idle + Speculative モード: 文字キーは即時出力して SpeculativeChar へ遷移
    fn handle_idle_speculative(&mut self, ev: &ClassifiedEvent) -> Resp {
        if ev.key_class.is_thumb() {
            // Thumb key → same as Wait mode (pending thumb)
            return self.handle_idle_wait(ev);
        }

        // Character key → immediately output normal face, enter SpeculativeChar
        let face = Face::Normal;
        if let Some((action, kana)) =
            self.lookup_face(ev.scan_code, ev.vk_code, self.get_face(face))
        {
            self.enter_speculative_char(PendingKey {
                scan_code: ev.scan_code,
                vk_code: ev.vk_code,
                timestamp: ev.timestamp,
            });
            // Output immediately + set timer for the threshold window
            self.finalize_plan(FinalizePlan {
                actions: vec![action.clone()],
                timer: TimerIntent::Pending,
                output: record_output(ev.scan_code, &action, kana),
            })
        } else {
            Response::pass_through()
        }
    }

    /// Idle + TwoPhase モード: Phase 1 は短い待機、Phase 2 は投機出力
    ///
    /// 親指キーは Wait モードと同じ扱い。
    /// 文字キーは短い待機（speculative_delay_us）の後、投機出力に遷移する。
    fn handle_idle_two_phase(&mut self, ev: &ClassifiedEvent) -> Resp {
        if ev.key_class.is_thumb() {
            // Thumb keys use Wait mode (same as Speculative)
            return self.handle_idle_wait(ev);
        }

        // Phase 1: Short wait (speculative_delay_us)
        // Same as Wait mode but with shorter timer
        self.enter_pending_char(PendingKey {
            scan_code: ev.scan_code,
            vk_code: ev.vk_code,
            timestamp: ev.timestamp,
        });

        // Use TIMER_SPECULATIVE with the short delay
        self.finalize_plan(FinalizePlan {
            actions: vec![],
            timer: TimerIntent::SpeculativeWait,
            output: OutputUpdate::None,
        })
    }

    /// Idle + NgramPredictive モード: n-gram スコアで投機/待機を動的切替
    ///
    /// 親指キーは Wait モードと同じ扱い。
    /// 文字キーは通常面と親指面の n-gram スコアを比較し、
    /// 通常面が明らかに有利なら Speculative、そうでなければ Wait。
    fn handle_idle_ngram_predictive(&mut self, ev: &ClassifiedEvent) -> Resp {
        if ev.key_class.is_thumb() {
            return self.handle_idle_wait(ev);
        }

        // If no n-gram model, fall back to TwoPhase
        let Some(ref model) = self.ngram_model else {
            return self.handle_idle_two_phase(ev);
        };

        // Get candidate kana for each face
        let normal_kana = self
            .lookup_face(ev.scan_code, ev.vk_code, self.get_face(Face::Normal))
            .and_then(|(_, kana)| kana);
        let left_kana = self
            .lookup_face(ev.scan_code, ev.vk_code, self.get_face(Face::LeftThumb))
            .and_then(|(_, kana)| kana);
        let right_kana = self
            .lookup_face(ev.scan_code, ev.vk_code, self.get_face(Face::RightThumb))
            .and_then(|(_, kana)| kana);

        // Compute scores
        let recent = self.output_history.recent_kana(3);
        let normal_score = normal_kana.map_or(0.0, |ch| model.frequency_score(&recent, ch));
        let thumb_score = [left_kana, right_kana]
            .iter()
            .filter_map(|k| k.map(|ch| model.frequency_score(&recent, ch)))
            .fold(f32::NEG_INFINITY, f32::max);
        let thumb_score = if thumb_score == f32::NEG_INFINITY {
            0.0
        } else {
            thumb_score
        };

        // Decision: if normal is clearly more likely, output speculatively
        let score_diff = normal_score - thumb_score;
        if score_diff > 0.5 {
            // Normal face is much more likely → Speculative
            self.handle_idle_speculative(ev)
        } else {
            // Unclear or thumb is likely → Wait (safe)
            self.handle_idle_wait(ev)
        }
    }
}

// ── 同時打鍵解決 ──
impl Engine {
    /// 投機出力済み状態で親指キーが到着した場合の処理
    fn handle_speculative_thumb(&mut self, ev: &ClassifiedEvent) -> Resp {
        let pending = self
            .pending_char
            .expect("SpeculativeChar phase requires pending_char");
        let elapsed = ev.timestamp.saturating_sub(pending.timestamp);
        let face = Face::from_thumb(ev.key_class);

        // Look up what the simultaneous keystroke would produce
        if let Some((thumb_action, thumb_kana)) =
            self.lookup_face(pending.scan_code, pending.vk_code, self.get_face(face))
        {
            let threshold =
                thumb_kana.map_or(self.threshold_us, |ch| self.adjusted_threshold_us(ch));

            if elapsed < threshold {
                // Within threshold → retract speculative output + emit thumb face

                // Record thumb as held (for continuous shift)
                if ev.key_class == KeyClass::LeftThumb {
                    self.left_thumb_down = Some(ev.timestamp);
                } else {
                    self.right_thumb_down = Some(ev.timestamp);
                }

                // Retract the speculative output: always 1 BS because IME treats
                // complete romaji as a single composition unit (Bug #3 fix)
                self.output_history.retract_last();

                let mut actions = vec![KeyAction::Key(VK_BACK.0)];
                actions.push(thumb_action.clone());

                self.go_idle();
                // Use Record (not RetractAndRecord) since we already retracted above
                return self.finalize_plan(FinalizePlan {
                    actions,
                    timer: TimerIntent::CancelAll,
                    output: OutputUpdate::Record(OutputRecord {
                        scan_code: pending.scan_code,
                        romaji: romaji_of(&thumb_action),
                        kana: thumb_kana,
                        action: thumb_action,
                    }),
                });
            }
            // Outside threshold → speculative was correct, process thumb as new key
        } else {
            // No thumb face entry → speculative was correct
        }
        self.go_idle();
        self.handle_idle(ev)
    }

    /// PendingChar + 親指キー → 同時打鍵候補（閾値内なら PendingCharThumb、超過なら flush+新規）
    fn handle_pending_char_thumb(&mut self, ev: &ClassifiedEvent) -> Resp {
        let pending = self
            .pending_char
            .expect("PendingChar phase requires pending_char");
        let elapsed_us = ev.timestamp.saturating_sub(pending.timestamp);

        // 親指面で保留文字キーの候補を取得し閾値を調整
        let candidate_face = Face::from_thumb(ev.key_class);
        let candidate = self.lookup_face(
            pending.scan_code,
            pending.vk_code,
            self.get_face(candidate_face),
        );
        let threshold = candidate
            .as_ref()
            .and_then(|(_, kana)| *kana)
            .map_or(self.threshold_us, |ch| self.adjusted_threshold_us(ch));

        if elapsed_us < threshold {
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
            return self.finalize_plan(FinalizePlan {
                actions: vec![],
                timer: TimerIntent::Pending,
                output: OutputUpdate::None,
            });
        }

        // 時間超過 → 前の保留を単独確定 + 今回のキーを新規処理
        self.go_idle();
        let prev = self.resolve_pending_char_as_single(pending.scan_code, pending.vk_code);
        self.update_history(prev.output);
        let new_result = self.handle_idle(ev);
        Self::combine_prev_and_new(prev.actions, new_result)
    }

    /// PendingChar + 文字キー → 前の保留を単独確定 + 今回のキーを新規処理
    fn handle_pending_char_char(&mut self, ev: &ClassifiedEvent) -> Resp {
        let pending = self
            .pending_char
            .expect("PendingChar phase requires pending_char");
        self.go_idle();
        let prev = self.resolve_pending_char_as_single(pending.scan_code, pending.vk_code);
        self.update_history(prev.output);
        let new_result = self.handle_idle(ev);
        Self::combine_prev_and_new(prev.actions, new_result)
    }

    /// PendingThumb + 文字キー → 同時打鍵候補（閾値内なら即時確定、超過なら flush+新規）
    fn handle_pending_thumb_char(&mut self, ev: &ClassifiedEvent) -> Resp {
        let thumb = self
            .pending_thumb
            .expect("PendingThumb phase requires pending_thumb");
        let elapsed_us = ev.timestamp.saturating_sub(thumb.timestamp);

        // 親指面で到着文字キーの候補を取得し閾値を調整
        let pending_face = Face::from_thumb_bool(thumb.is_left);
        let candidate = self.lookup_face(ev.scan_code, ev.vk_code, self.get_face(pending_face));
        let threshold = candidate
            .as_ref()
            .and_then(|(_, kana)| *kana)
            .map_or(self.threshold_us, |ch| self.adjusted_threshold_us(ch));

        if elapsed_us < threshold {
            if let Some((action, kana)) = candidate {
                // 保留=親指, 到着=文字 → 同時打鍵
                if thumb.is_left {
                    self.left_thumb_down = Some(thumb.timestamp);
                } else {
                    self.right_thumb_down = Some(thumb.timestamp);
                }
                self.go_idle();
                return self.finalize_plan(FinalizePlan {
                    actions: vec![action.clone()],
                    timer: TimerIntent::CancelAll,
                    output: record_output(ev.scan_code, &action, kana),
                });
            }
        }

        // 時間超過 or 候補なし → 前の保留を単独確定 + 今回のキーを新規処理
        self.go_idle();
        let prev = self.resolve_pending_thumb_as_single(thumb.vk_code);
        self.update_history(prev.output);
        let new_result = self.handle_idle(ev);
        Self::combine_prev_and_new(prev.actions, new_result)
    }

    /// PendingThumb + 親指キー → 前の保留を単独確定 + 今回のキーを新規処理
    fn handle_pending_thumb_thumb(&mut self, ev: &ClassifiedEvent) -> Resp {
        let thumb = self
            .pending_thumb
            .expect("PendingThumb phase requires pending_thumb");
        self.go_idle();
        let prev = self.resolve_pending_thumb_as_single(thumb.vk_code);
        self.update_history(prev.output);
        let new_result = self.handle_idle(ev);
        Self::combine_prev_and_new(prev.actions, new_result)
    }

    /// OutputUpdate に基づいて出力履歴を更新する共通ヘルパー
    fn update_history(&mut self, output: OutputUpdate) {
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

    /// 前の保留のアクションと今回の結果を結合する
    fn combine_prev_and_new(prev_actions: Vec<KeyAction>, new_result: Resp) -> Resp {
        if new_result.consumed {
            // Consumed (emit or pending): prepend prev_actions, keep new_result's timers
            if prev_actions.is_empty() && new_result.actions.is_empty() {
                // Both empty → pure consume (pending state with timer)
                new_result
            } else {
                let mut all_actions = prev_actions;
                all_actions.extend(new_result.actions);
                let mut r = Response::emit(all_actions);
                r.timers = new_result.timers;
                r
            }
        } else if prev_actions.is_empty() {
            // PassThrough with no prev → pass through as-is
            new_result
        } else {
            // PassThrough with prev_actions → emit prev + cancel all timers
            Response::emit(prev_actions)
                .with_kill_timer(TIMER_PENDING)
                .with_kill_timer(TIMER_SPECULATIVE)
        }
    }

    /// 保留中の文字キーを単独打鍵として解決し、アクション列と OutputUpdate を返す
    fn resolve_pending_char_as_single(
        &self,
        scan_code: ScanCode,
        vk_code: VkCode,
    ) -> ResolvedAction {
        if let Some((action, kana)) =
            self.lookup_face(scan_code, vk_code, self.get_face(Face::Normal))
        {
            let output = record_output(scan_code, &action, kana);
            ResolvedAction {
                actions: vec![action],
                output,
            }
        } else {
            let action = KeyAction::Key(vk_code.0);
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
        let action = KeyAction::Key(vk_code.0);
        let output = record_output(ScanCode(u32::from(vk_code.0)), &action, None);
        ResolvedAction {
            actions: vec![action],
            output,
        }
    }

    /// `PendingCharThumb` 状態で新しいキーが到着した場合の 3 鍵仲裁処理
    fn resolve_pending_char_thumb(&mut self, ev: &ClassifiedEvent) -> Resp {
        let pending = self
            .pending_char
            .expect("PendingCharThumb phase requires pending_char");
        let thumb = self
            .pending_thumb
            .expect("PendingCharThumb phase requires pending_thumb");
        self.go_idle();

        if !ev.key_class.is_thumb() {
            // char2 到着 → d1/d2 比較で 3 鍵仲裁
            let d1 = thumb.timestamp.saturating_sub(pending.timestamp);
            let d2 = ev.timestamp.saturating_sub(thumb.timestamp);

            if d1 < d2 {
                // char1+thumb = 同時打鍵、char2 は新規処理
                let prev = self.resolve_char_thumb_as_simultaneous(
                    pending.scan_code,
                    pending.vk_code,
                    thumb.is_left,
                    thumb.timestamp,
                );
                self.update_history(prev.output);
                let new_result = self.handle_idle(ev);
                return Self::combine_prev_and_new(prev.actions, new_result);
            }
            // d1 >= d2: char1 = 単独、char2+thumb = 同時打鍵
            let prev = self.resolve_pending_char_as_single(pending.scan_code, pending.vk_code);
            self.update_history(prev.output);
            let thumb_face = Face::from_thumb_bool(thumb.is_left);
            if let Some((action, kana)) =
                self.lookup_face(ev.scan_code, ev.vk_code, self.get_face(thumb_face))
            {
                if thumb.is_left {
                    self.left_thumb_down = Some(thumb.timestamp);
                } else {
                    self.right_thumb_down = Some(thumb.timestamp);
                }
                let mut all_actions = prev.actions;
                all_actions.push(action.clone());
                return self.finalize_plan(FinalizePlan {
                    actions: all_actions,
                    timer: TimerIntent::CancelAll,
                    output: record_output(ev.scan_code, &action, kana),
                });
            }
            // 親指面に char2 の定義がない場合はそれぞれ単独確定
            let new_result = self.handle_idle(ev);
            return Self::combine_prev_and_new(prev.actions, new_result);
        }
        // 親指キーが来た場合は char1+thumb を同時打鍵として確定し、
        // 新しい親指キーを保留にする
        let prev = self.resolve_char_thumb_as_simultaneous(
            pending.scan_code,
            pending.vk_code,
            thumb.is_left,
            thumb.timestamp,
        );
        self.update_history(prev.output);
        let new_result = self.handle_idle(ev);
        Self::combine_prev_and_new(prev.actions, new_result)
    }
}

// ── KeyUp 処理 ──
impl Engine {
    fn on_key_up(&mut self, event: &RawKeyEvent) -> Resp {
        // 親指キーのリリース追跡
        let ev = self.classify(event);
        if ev.key_class.is_left_thumb() {
            self.left_thumb_down = None;
        } else if ev.key_class == KeyClass::RightThumb {
            self.right_thumb_down = None;
        }

        // PendingCharThumb 状態での KeyUp 処理
        if self.phase == EnginePhase::PendingCharThumb {
            let char_vk = self
                .pending_char
                .expect("PendingCharThumb requires pending_char")
                .vk_code;
            let thumb_vk = self
                .pending_thumb
                .expect("PendingCharThumb requires pending_thumb")
                .vk_code;
            if event.vk_code == char_vk || event.vk_code == thumb_vk {
                return self.handle_key_up_pending_char_thumb(event);
            }
        }

        // SpeculativeChar 状態で投機出力キーが離された場合 → 出力確定（Idle へ遷移）
        if self.phase == EnginePhase::SpeculativeChar {
            let spec_vk = self
                .pending_char
                .expect("SpeculativeChar requires pending_char")
                .vk_code;
            if event.vk_code == spec_vk {
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
        match self.phase {
            EnginePhase::PendingChar => self.pending_char.is_some_and(|p| p.vk_code == vk_code),
            EnginePhase::PendingThumb => self.pending_thumb.is_some_and(|t| t.vk_code == vk_code),
            EnginePhase::Idle | EnginePhase::PendingCharThumb | EnginePhase::SpeculativeChar => {
                false
            }
        }
    }

    /// PendingCharThumb 状態で char1 または thumb が離された場合の処理
    fn handle_key_up_pending_char_thumb(&mut self, event: &RawKeyEvent) -> Resp {
        let pending = self
            .pending_char
            .expect("PendingCharThumb requires pending_char");
        let thumb = self
            .pending_thumb
            .expect("PendingCharThumb requires pending_thumb");
        self.go_idle();
        let resolved = self.resolve_char_thumb_as_simultaneous(
            pending.scan_code,
            pending.vk_code,
            thumb.is_left,
            thumb.timestamp,
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
        self.finalize_plan(FinalizePlan {
            actions,
            timer: TimerIntent::CancelAll,
            output: OutputUpdate::None,
        })
    }

    /// 保留中のキーが離された場合、保留を単独確定して KeyUp を処理する
    fn handle_key_up_pending(&mut self, event: &RawKeyEvent) -> Resp {
        let old_phase = self.phase;
        let old_char = self.pending_char.take();
        let old_thumb = self.pending_thumb.take();
        self.phase = EnginePhase::Idle;

        let resolved = match old_phase {
            EnginePhase::PendingChar => {
                let pending = old_char.expect("PendingChar phase requires pending_char");
                self.resolve_pending_char_as_single(pending.scan_code, pending.vk_code)
            }
            EnginePhase::PendingThumb => {
                let thumb = old_thumb.expect("PendingThumb phase requires pending_thumb");
                self.resolve_pending_thumb_as_single(thumb.vk_code)
            }
            EnginePhase::Idle | EnginePhase::PendingCharThumb | EnginePhase::SpeculativeChar => {
                log::error!(
                    "unexpected phase in handle_key_up_pending: {:?}",
                    self.phase
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
        self.finalize_plan(FinalizePlan {
            actions: result,
            timer: TimerIntent::CancelAll,
            output: OutputUpdate::None,
        })
    }

    /// output_history から対応する注入済みキーを探してリリースする
    fn handle_key_up_active(&mut self, event: &RawKeyEvent) -> Resp {
        if let Some(entry) = self.output_history.remove_by_scan(event.scan_code) {
            return match entry.action {
                // Unicode 文字やローマ字列の場合、KeyUp は不要（押下時に入力完了）
                KeyAction::Char(_) | KeyAction::Romaji(_) => self.finalize_plan(FinalizePlan {
                    actions: vec![KeyAction::Suppress],
                    timer: TimerIntent::CancelAll,
                    output: OutputUpdate::None,
                }),
                KeyAction::Key(vk) => self.finalize_plan(FinalizePlan {
                    actions: vec![KeyAction::KeyUp(vk)],
                    timer: TimerIntent::CancelAll,
                    output: OutputUpdate::None,
                }),
                _ => Response::pass_through(),
            };
        }
        Response::pass_through()
    }
}

// ── タイムアウト処理 ──
impl Engine {
    /// PendingChar タイムアウト：文字キーを単独打鍵として確定する
    fn timeout_pending_char(&mut self, scan_code: ScanCode, vk_code: VkCode) -> Resp {
        if let Some((action, kana)) =
            self.lookup_face(scan_code, vk_code, self.get_face(Face::Normal))
        {
            self.finalize_plan(FinalizePlan {
                actions: vec![action.clone()],
                timer: TimerIntent::CancelAll,
                output: record_output(scan_code, &action, kana),
            })
        } else {
            // 配列定義に含まれないキーはそのまま通す
            let action = KeyAction::Key(vk_code.0);
            self.finalize_plan(FinalizePlan {
                actions: vec![action.clone()],
                timer: TimerIntent::CancelAll,
                output: record_output(scan_code, &action, None),
            })
        }
    }

    /// PendingThumb タイムアウト：親指キーを単独打鍵として確定する
    fn timeout_pending_thumb(&mut self, vk_code: VkCode) -> Resp {
        let action = KeyAction::Key(vk_code.0);
        self.finalize_plan(FinalizePlan {
            actions: vec![action.clone()],
            timer: TimerIntent::CancelAll,
            output: record_output(ScanCode(u32::from(vk_code.0)), &action, None),
        })
    }

    /// PendingCharThumb タイムアウト：char1+thumb を同時打鍵として確定する
    fn timeout_pending_char_thumb(
        &mut self,
        char_scan: ScanCode,
        char_vk: VkCode,
        thumb_is_left: bool,
        thumb_timestamp: Timestamp,
    ) -> Resp {
        let resolved = self.resolve_char_thumb_as_simultaneous(
            char_scan,
            char_vk,
            thumb_is_left,
            thumb_timestamp,
        );
        self.finalize_plan(FinalizePlan {
            actions: resolved.actions,
            timer: TimerIntent::CancelAll,
            output: resolved.output,
        })
    }

    /// TwoPhase モード: Phase 1 の短い待機がタイムアウトした場合の処理
    ///
    /// 親指キーが Phase 1 内に到着しなかったので、投機出力（Phase 2）に遷移する。
    /// 通常面の文字を出力し、SpeculativeChar 状態に入る。
    /// 残りの閾値時間（threshold_us - speculative_delay_us）で TIMER_PENDING を設定する。
    fn on_timeout_speculative(&mut self) -> Resp {
        match self.phase {
            EnginePhase::PendingChar => {
                let pending = self
                    .pending_char
                    .expect("PendingChar phase requires pending_char");
                // Output normal face speculatively
                let face = Face::Normal;
                if let Some((action, kana)) =
                    self.lookup_face(pending.scan_code, pending.vk_code, self.get_face(face))
                {
                    self.enter_speculative_char(pending);
                    // Emit the speculative output + set TIMER_PENDING for remaining time
                    let remaining_us = self.threshold_us.saturating_sub(self.speculative_delay_us);
                    self.finalize_plan(FinalizePlan {
                        actions: vec![action.clone()],
                        timer: TimerIntent::Phase2Transition { remaining_us },
                        output: record_output(pending.scan_code, &action, kana),
                    })
                } else {
                    self.go_idle();
                    Response::pass_through().with_kill_timer(TIMER_SPECULATIVE)
                }
            }
            // Other phases shouldn't have TIMER_SPECULATIVE active
            other => {
                log::warn!("TIMER_SPECULATIVE fired in unexpected phase: {other:?}",);
                Response::pass_through().with_kill_timer(TIMER_SPECULATIVE)
            }
        }
    }
}

// ── 分類・バイパス ──
impl Engine {
    /// キーイベントを分類する
    fn classify(&self, event: &RawKeyEvent) -> ClassifiedEvent {
        let key_class = if event.vk_code == self.left_thumb_vk {
            KeyClass::LeftThumb
        } else if event.vk_code == self.right_thumb_vk {
            KeyClass::RightThumb
        } else if crate::vk::is_passthrough(event.vk_code) {
            KeyClass::Passthrough
        } else {
            KeyClass::Char
        };

        let pos = if key_class == KeyClass::Char {
            scan_to_pos(event.scan_code.0)
        } else {
            None
        };

        ClassifiedEvent {
            key_class,
            pos,
            scan_code: event.scan_code,
            vk_code: event.vk_code,
            timestamp: event.timestamp,
        }
    }

    /// 現在押下中の親指キーに対応するシフト面を返す
    const fn active_thumb_face(&self) -> Option<Face> {
        if self.left_thumb_down.is_some() {
            Some(Face::LeftThumb)
        } else if self.right_thumb_down.is_some() {
            Some(Face::RightThumb)
        } else {
            None
        }
    }

    /// いずれかの配列面に定義されているキーかどうか
    fn is_layout_key(&self, scan_code: ScanCode) -> bool {
        let Some(pos) = scan_to_pos(scan_code.0) else {
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
        if crate::vk::is_ime_control(ev.vk_code) {
            return Some(BypassReason::ImeControl);
        }
        if self.modifiers.is_os_modifier_held() {
            return Some(BypassReason::OsModifierHeld);
        }
        None
    }

    /// バイパス理由に基づいて保留キーをフラッシュしつつパススルーする
    ///
    /// 全てのバイパス理由で同一の処理: 保留があればフラッシュ、元のキーは OS にパススルー。
    fn handle_bypass(&mut self, reason: BypassReason) -> Resp {
        log::trace!("bypass: {reason:?}");
        if self.phase == EnginePhase::Idle {
            return Response::pass_through();
        }
        let flush = self.flush_pending(ContextChange::ImeOff);
        let mut resp = Response::pass_through();
        resp.actions = flush.actions;
        resp.timers = flush.timers;
        resp
    }
}

// ── TimedStateMachine 実装 ──
impl TimedStateMachine for Engine {
    type Event = RawKeyEvent;
    type Action = KeyAction;
    type TimerId = usize;

    fn on_event(&mut self, event: RawKeyEvent) -> Resp {
        // 修飾キー（Ctrl / Alt）の押下状態を追跡
        // エンジン無効中も継続して追跡する。早期 return の後に置くと
        // OFF 中の KeyUp を見逃して modifier が stuck する。
        self.modifiers.update(&event);

        if !self.enabled {
            return Response::pass_through();
        }

        match event.event_type {
            KeyEventType::KeyDown | KeyEventType::SysKeyDown => self.on_key_down(&event),
            KeyEventType::KeyUp | KeyEventType::SysKeyUp => self.on_key_up(&event),
        }
    }

    fn on_timeout(&mut self, timer_id: usize) -> Resp {
        match timer_id {
            TIMER_SPECULATIVE => return self.on_timeout_speculative(),
            TIMER_PENDING => {}
            _ => return Response::pass_through(),
        }

        let old_phase = self.phase;
        let old_char = self.pending_char.take();
        let old_thumb = self.pending_thumb.take();
        self.phase = EnginePhase::Idle;

        match old_phase {
            EnginePhase::Idle => {
                // Spurious timeout — state already transitioned to Idle.
                // pass_through to avoid suppressing unrelated keys.
                Response::pass_through().with_kill_timer(TIMER_PENDING)
            }
            EnginePhase::PendingChar => {
                let pending = old_char.expect("PendingChar phase requires pending_char");
                self.timeout_pending_char(pending.scan_code, pending.vk_code)
            }
            EnginePhase::PendingThumb => {
                let thumb = old_thumb.expect("PendingThumb phase requires pending_thumb");
                self.timeout_pending_thumb(thumb.vk_code)
            }
            EnginePhase::PendingCharThumb => {
                let pending = old_char.expect("PendingCharThumb phase requires pending_char");
                let thumb = old_thumb.expect("PendingCharThumb phase requires pending_thumb");
                self.timeout_pending_char_thumb(
                    pending.scan_code,
                    pending.vk_code,
                    thumb.is_left,
                    thumb.timestamp,
                )
            }
            // 投機出力済み → タイムアウト = 親指キー未到着 → 投機出力は正しかった → Idle へ
            EnginePhase::SpeculativeChar => Response::consume().with_kill_timer(TIMER_PENDING),
        }
    }
}

#[cfg(test)]
mod tests;
