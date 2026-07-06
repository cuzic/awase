//! 新 Engine: NicolaFsm + 特殊キー処理を統合するラッパー。
//!
//! `on_input` / `on_timeout` / `on_command` が唯一のエントリポイント。
//! OS API を一切呼ばず、副作用は `Decision` として返す。
//!
//! # 設計方針
//!
//! Engine は near-pure function として設計されている。
//! - 物理キー状態（修飾キー、親指キー）は Platform 層が InputTracker で追跡し、
//!   InputContext 経由で毎回渡す
//! - IME ガード（遷移中のキーバッファリング）は Platform 層が担当する
//! - Engine は InputContext のスナップショットだけで判断する（先読みしない）

use crate::config::ParsedKeyCombo;
use crate::types::{ContextChange, KeyEventType, RawKeyEvent, VkCode};

use super::decision::{
    ActivationState, Decision, Effect, EffectVec, EngineCommand, ImeEffect,
    InactiveReason, InputContext, InputEffect, SpecialKeyCombos, UiEffect,
};
use super::fsm_adapter::FsmAdapter;
use super::fsm_types::ModifierState;
use super::input_tracker::PhysicalKeyState;
use super::key_lifecycle::KeyLifecycle;
use super::nicola_fsm::NicolaFsm;

/// 特殊キーコンボのマッチ結果
enum SpecialKeyMatch {
    EngineOn,
    EngineOff,
    ImeOn,
    ImeOff,
}

/// 統合エンジン: NicolaFsm + 特殊キー処理
///
/// Engine の有効状態は2軸で決まる:
/// - `user_enabled`: ユーザーの意図（ホットキー/トレイで操作）= FSM の `enabled` フラグ
/// - 環境前提条件: `InputContext { ime_on, is_romaji, is_japanese_ime, ... }` — Platform 層が毎回渡す
/// - 実効状態: `compute_state(ctx)` が `ActivationState::Active` を返すとき
///
/// Engine は前提条件を内部にキャッシュしない。毎回の呼び出しで Platform 層から受け取る。
///
/// `on_input` が唯一のキーイベントエントリポイント。
/// OS API を一切呼ばず、副作用は `Decision` として返す。
#[allow(missing_debug_implementations)]
pub struct Engine {
    adapter: FsmAdapter,
    special_keys: SpecialKeyCombos,
    /// キーの Down/Up ペア追跡
    lifecycle: KeyLifecycle,
    /// 直前の実効状態（遷移検知用）
    prev_activation: ActivationState,
    /// NICOLA 親指シフトキーの VK コード（左・右）。
    /// IME ON/OFF コンボ判定時に除外するために使用。
    /// VkCode(0) = 未設定。
    thumb_vks: (VkCode, VkCode),
}

impl Engine {
    #[must_use]
    pub const fn new(fsm: NicolaFsm, special_keys: SpecialKeyCombos) -> Self {
        Self {
            adapter: FsmAdapter::new(fsm),
            special_keys,
            lifecycle: KeyLifecycle::new(),
            prev_activation: ActivationState::Inactive(InactiveReason::UserDisabled),
            thumb_vks: (VkCode(0), VkCode(0)),
        }
    }

    /// NICOLA 親指シフトキーの VK コードを設定する。
    /// IME ON/OFF コンボが親指キーと衝突しないように除外するために使用。
    pub const fn set_thumb_vks(&mut self, left: VkCode, right: VkCode) {
        self.thumb_vks = (left, right);
    }

    /// ソロ N 連打でエンジン OFF を発動するキーを設定する。
    /// `VkCode(0)` を渡すと機能を無効にする。
    pub const fn set_engine_off_triple_vk(&mut self, vk: VkCode) {
        self.adapter.set_engine_off_triple_vk(vk);
    }

    /// InputContext から実効状態を `ActivationState` で返す。
    ///
    /// 判定順: user_enabled → is_japanese_ime → ime_on → is_romaji
    /// 各条件が false のとき対応する `InactiveReason` を返す。
    #[must_use]
    pub const fn compute_state(&self, ctx: &InputContext) -> ActivationState {
        if !self.adapter.is_enabled() {
            return ActivationState::Inactive(InactiveReason::UserDisabled);
        }
        if !ctx.is_japanese_ime {
            return ActivationState::Inactive(InactiveReason::NotJapaneseIme);
        }
        if !ctx.ime_on {
            return ActivationState::Inactive(InactiveReason::ImeOff);
        }
        if !ctx.input_mode.is_romaji_capable() {
            return ActivationState::Inactive(InactiveReason::NotRomajiInput);
        }
        ActivationState::Active
    }

    /// InputContext から実効状態を bool で返す（後方互換 API）。
    #[must_use]
    pub const fn compute_active(&self, ctx: &InputContext) -> bool {
        self.compute_state(ctx).is_active()
    }

    /// 実効状態の遷移を検知し、必要な Effect（flush, UI 通知）を返す。
    fn check_active_transition(&mut self, ctx: &InputContext) -> EffectVec {
        let new_state = self.compute_state(ctx);
        let was_active = self.prev_activation.is_active();
        let now_active = new_state.is_active();
        let mut effects = EffectVec::new();

        if was_active != now_active {
            if !now_active {
                // active → inactive: 保留キーをフラッシュ
                let reason = new_state.to_context_change();
                let flush = self.adapter.flush_to_effects(reason);
                effects.extend(flush);
                // lifecycle をクリア: Engine が consumed した KeyDown の対応 KeyUp が
                // Engine inactive 時に到着しても consumed されないようにする。
                let _ = self.lifecycle.flush_pending_key_ups();
            }
            log::info!(
                "Engine {} (ime={}, romaji={}, japanese={}, user={}, reason={:?})",
                if now_active {
                    "activated"
                } else {
                    "deactivated"
                },
                ctx.ime_on,
                ctx.input_mode.is_romaji_capable(),
                ctx.is_japanese_ime,
                self.adapter.is_enabled(),
                new_state,
            );
        }

        let transition_effects = self.transition_activation(new_state);
        effects.extend(transition_effects);
        effects
    }

    /// 実効状態を新しい状態に遷移させ、変化があった場合に SetOpen + UiEffect を返す。
    ///
    /// inactive → active: OS IME を強制的に開く（"nonaiyo" 問題対策）
    /// active → inactive: OS IME を強制的に閉じる（対称性のため）
    ///   ただし NotRomajiInput（tray での英数モード選択等）の場合は SetOpen(false) を出さない。
    ///   ユーザーが既に望むモード（全角英数等）を選択済みなので、VK_DBE_ALPHANUMERIC を
    ///   追加送信すると全角英数→半角英数のような意図しない conv 変化が起きる。
    /// 同じ状態: 空の EffectVec
    fn transition_activation(&mut self, new_state: ActivationState) -> EffectVec {
        let was_active = self.prev_activation.is_active();
        let now_active = new_state.is_active();
        let mut effects = EffectVec::new();

        if was_active != now_active {
            let suppress_set_open = matches!(
                new_state,
                ActivationState::Inactive(InactiveReason::NotRomajiInput)
            );
            if !suppress_set_open {
                effects.push(Effect::Ime(ImeEffect::SetOpen {
                    open: now_active,
                }));
            }
            // NotRomajiInput の場合は SetOpen も engine-state キーも不要。
            // ユーザーが選択した kana/katakana モードをそのまま維持する。
            let suppress_ime_key = suppress_set_open; // 同じ条件
            effects.push(Effect::Ui(UiEffect::EngineStateChanged {
                enabled: now_active,
                send_ime_key: !suppress_ime_key,
            }));
            self.prev_activation = new_state;
        }
        effects
    }

    /// キーイベントの統合エントリポイント。
    ///
    /// 処理フロー:
    /// 1. KeyUp 自動追跡
    /// 2. 特殊キー（エンジン ON/OFF + IME 制御）
    /// 3. 実効状態チェック + 遷移検知
    /// 4. NicolaFsm 処理
    pub fn on_input(&mut self, event: RawKeyEvent, ctx: &InputContext) -> Decision {
        // Phase 0: KeyUp 自動追跡
        let is_key_down = matches!(event.event_type, KeyEventType::KeyDown);
        if !is_key_down && self.lifecycle.on_key_up(event.vk_code) {
            return Decision::consumed();
        }

        // Phase 1: Special keys (engine toggle + IME control)
        if is_key_down {
            if let Some(decision) = self.check_special_keys(ctx, &event) {
                if decision.is_consumed() {
                    self.lifecycle.on_key_down_consumed(&event);
                }
                return decision;
            }
        }

        // Phase 2: Active state check + transition detection
        let transition_effects = self.check_active_transition(ctx);
        if !self.compute_active(ctx) {
            if transition_effects.is_empty() {
                return Decision::pass_through();
            }
            return Decision::pass_through_with(transition_effects);
        }

        // Phase 3: NicolaFsm
        let phys = PhysicalKeyState::from_ctx(ctx, &event);
        let mut decision = self.adapter.on_event(event, &phys);
        if is_key_down && decision.is_consumed() {
            self.lifecycle.on_key_down_consumed(&event);
        }
        decision.prepend_effects(transition_effects);
        decision
    }

    /// タイマー満了時のエントリポイント。
    pub fn on_timeout(&mut self, timer_id: usize, ctx: &InputContext) -> Decision {
        let phys = PhysicalKeyState::from_ctx_snapshot(ctx);

        // Engine が非活性なら on_timeout せず flush（コンテキスト喪失）
        if !self.compute_active(ctx) {
            return self.adapter.flush(ContextChange::ImeOff);
        }

        let decision = self.adapter.on_timeout(timer_id, &phys, ctx.composing);

        // ソロ連打によるエンジン OFF トリガー
        if self.adapter.take_engine_off_requested() {
            log::info!("Engine OFF triggered by consecutive solo key presses");
            return self.apply_special_key_match(&SpecialKeyMatch::EngineOff, ctx);
        }

        decision
    }

    /// 外部コマンドの統合エントリポイント。
    ///
    /// `toggle_engine`, `invalidate_engine_context`, `swap_layout` 等の個別メソッドを
    /// 単一のディスパッチに集約する。
    pub fn on_command(&mut self, cmd: EngineCommand, ctx: &InputContext) -> Decision {
        match cmd {
            EngineCommand::ToggleEngine => {
                let old_active = self.compute_active(ctx);
                let (user_enabled, mut decision) = self.adapter.toggle_enabled();
                let new_active = self.compute_active(ctx);
                log::info!(
                    "Engine user_enabled toggled: {} (active: {})",
                    if user_enabled { "ON" } else { "OFF" },
                    if new_active { "ON" } else { "OFF" },
                );
                if user_enabled && !new_active {
                    // ユーザーが明示的に有効化したが ime_on=false 等で active になれない。
                    // pseudo_ctx で IME 強制 ON + tray 更新を行う。
                    self.apply_engine_on_with_ime_recovery(ctx, &mut decision);
                } else {
                    self.apply_active_transition(old_active, new_active, &mut decision);
                }
                decision
            }
            EngineCommand::InvalidateContext(reason) => self.adapter.flush(reason),
            EngineCommand::SwapLayout(layout) => self.adapter.swap_layout(layout),
            EngineCommand::ReloadKeys { special } => {
                self.special_keys = special;
                Decision::pass_through()
            }
            EngineCommand::UpdateFsmParams {
                threshold_ms,
                confirm_mode,
                speculative_delay_ms,
            } => {
                self.adapter.set_threshold_ms(threshold_ms);
                self.adapter
                    .set_confirm_mode(confirm_mode, speculative_delay_ms);
                Decision::pass_through()
            }
            EngineCommand::SetNgramModel(model) => {
                self.adapter.set_ngram_model(model);
                Decision::pass_through()
            }
            EngineCommand::RefreshState => {
                // Platform 層がアトミック変数を更新済み。ctx に反映されている。
                let effects = self.check_active_transition(ctx);
                if effects.is_empty() {
                    Decision::pass_through()
                } else {
                    Decision::pass_through_with(effects)
                }
            }
            EngineCommand::FocusChanged => self.handle_focus_changed(ctx),
        }
    }

    /// フォーカス変更の観測結果を処理し、コンテキスト無効化等の Decision を返す。
    /// フォーカス変更（前面プロセス変更）の処理。
    ///
    /// デバウンス後に Platform 層が前面プロセスの変化を検出した場合のみ呼ばれる（ADR 028）。
    /// focus_kind / app_kind / last_focus_info / キャッシュの更新は Platform 層で完了済み。
    /// Engine は pending flush と lifecycle 整合のみ担当する。
    fn handle_focus_changed(&mut self, ctx: &InputContext) -> Decision {
        let mut effects = EffectVec::new();

        // アプリ切替: 前のウィンドウで入力途中だったキーを別のウィンドウに持ち越さない。
        let flush_effects = self.adapter.flush_to_effects(ContextChange::FocusChanged);
        effects.extend(flush_effects);

        // Consume 済みで KeyUp が来ていないキーの KeyUp を再注入して
        // OS 側のキーボード状態と整合させる。
        let pending_key_ups = self.lifecycle.flush_pending_key_ups();
        for evt in pending_key_ups {
            effects.push(Effect::Input(InputEffect::ReinjectKey(evt)));
        }

        // 実効状態の遷移を検知
        let transition_effects = self.check_active_transition(ctx);
        effects.extend(transition_effects);

        Decision::pass_through_with(effects)
    }

    /// user_enabled のみ
    #[must_use]
    pub const fn is_user_enabled(&self) -> bool {
        self.adapter.is_enabled()
    }

    /// 診断用: 現在の FSM 状態を短い文字列で返す。
    /// `[engine-input]` ログで `on_input` 呼び出し前の状態を可視化するために使用。
    #[must_use]
    pub fn debug_state_label(&self) -> String {
        self.adapter.debug_state_label()
    }

    /// user_enabled を直接設定する（テスト・初期化用）
    pub fn set_user_enabled(&mut self, enabled: bool) {
        let _ = self.adapter.set_enabled(enabled);
    }

    /// 前回の実効状態を直接設定する（テスト・初期化用）。
    pub const fn set_prev_active(&mut self, active: bool) {
        self.prev_activation = if active {
            ActivationState::Active
        } else {
            ActivationState::Inactive(InactiveReason::UserDisabled)
        };
    }

    // ── 内部メソッド ──

    /// user_enabled 変更後の active 遷移を Decision に反映する。
    fn apply_active_transition(
        &mut self,
        old_active: bool,
        new_active: bool,
        decision: &mut Decision,
    ) {
        if old_active != new_active {
            // prev_activation を呼び出し時点の実際の状態に同期してから遷移させる
            self.prev_activation = if old_active {
                ActivationState::Active
            } else {
                ActivationState::Inactive(InactiveReason::UserDisabled)
            };

            let new_state = if new_active {
                ActivationState::Active
            } else {
                ActivationState::Inactive(InactiveReason::UserDisabled)
            };
            let effects = self.transition_activation(new_state);
            for e in effects {
                decision.push_effect(e);
            }
        }
    }

    /// エンジン有効化時に IME が OFF で active になれない場合の回復処理。
    ///
    /// `user_enabled=true` だが `ime_on=false` 等で `compute_active` が false のとき、
    /// pseudo_ctx (ime_on=true) で目標状態を計算し `transition_activation` を実行する。
    /// これにより `ImeEffect::SetOpen{true}` と `UiEffect::EngineStateChanged{true}` が
    /// 発行され、IME 強制 ON と tray 更新が行われる。
    ///
    /// `is_japanese_ime=false` 等で IME を ON にしても active になれない場合は
    /// `SetOpen{true}` のみ追加する（意図を Platform 層に伝えるため）。
    fn apply_engine_on_with_ime_recovery(&mut self, ctx: &InputContext, decision: &mut Decision) {
        let pseudo_ctx = InputContext {
            ime_on: true,
            ..*ctx
        };
        let target_state = self.compute_state(&pseudo_ctx);
        let effects = self.transition_activation(target_state);
        if effects.is_empty() {
            decision.push_effect(Effect::Ime(ImeEffect::SetOpen {
                open: true,
            }));
        } else {
            for e in effects {
                decision.push_effect(e);
            }
        }
    }

    /// IME ON/OFF コンボキーに対する Decision を構築する。
    ///
    /// `open` を反映した擬似 `InputContext` で新 `ActivationState` を求め、
    /// `transition_activation` で `SetOpen + EngineStateChanged` を発行する。
    /// 状態が遷移しない場合（例: `user_enabled=false` で既に Inactive）は
    /// `SetOpen` のみを明示的に追加する（IME 制御の意図を Platform 層に伝えるため）。
    ///
    /// # 二重 enqueue 防止
    ///
    /// `transition_activation` で `prev_activation` を新状態に推進するため、
    /// 次回の `check_active_transition` は no-op となり、構造的に重複を排除する。
    fn build_ime_set_open_decision(&mut self, ctx: &InputContext, open: bool) -> Decision {
        let pseudo_ctx = InputContext {
            ime_on: open,
            ..*ctx
        };
        let new_state = self.compute_state(&pseudo_ctx);
        let was_active = self.prev_activation.is_active();
        let now_active = new_state.is_active();

        let mut effects = self.transition_activation(new_state);
        if was_active == now_active {
            // 状態遷移なし → transition_activation は空 effects を返す。
            // IME 制御の意図 (SetOpen) は明示的に追加する。
            effects.push(Effect::Ime(ImeEffect::SetOpen {
                open,
            }));
        }
        Decision::consumed_with(effects)
    }

    /// 与えられたイベントが IME OFF コンボキーにマッチするかを副作用なしで返す。
    ///
    /// Platform 層が「即時 IME OFF か 50ms 救済窓 か」を判断するための先読み用 API。
    /// 状態は何も変更しないので `&self`。
    #[must_use]
    pub fn matches_ime_off(&self, ctx: &InputContext, event: &RawKeyEvent) -> bool {
        matches!(
            self.match_special_keys(ctx, event),
            Some(SpecialKeyMatch::ImeOff)
        )
    }

    /// 変換/無変換系の特殊キーのコンボマッチのみを行う純粋判定メソッド（副作用なし）。
    fn match_special_keys(
        &self,
        ctx: &InputContext,
        event: &RawKeyEvent,
    ) -> Option<SpecialKeyMatch> {
        self.special_keys
            .match_event(event, ctx.modifiers, self.adapter.is_enabled())
    }

    /// `SpecialKeyMatch` に応じた状態変更と `Decision` 生成を行う副作用適用メソッド。
    fn apply_special_key_match(&mut self, m: &SpecialKeyMatch, ctx: &InputContext) -> Decision {
        match m {
            SpecialKeyMatch::EngineOn => {
                let old_active = self.compute_active(ctx);
                let (_, mut decision) = self.adapter.set_enabled(true);
                let new_active = self.compute_active(ctx);
                log::info!("Engine user_enabled ON (key combo, active={new_active})");
                if !new_active {
                    // ime_on=false 等で active になれない → pseudo_ctx で IME 強制 ON
                    self.apply_engine_on_with_ime_recovery(ctx, &mut decision);
                } else {
                    self.apply_active_transition(old_active, new_active, &mut decision);
                }
                decision
            }
            SpecialKeyMatch::EngineOff => {
                let old_active = self.compute_active(ctx);
                let (_, mut decision) = self.adapter.set_enabled(false);
                let new_active = self.compute_active(ctx);
                log::info!("Engine user_enabled OFF (key combo, active={new_active})");
                self.apply_active_transition(old_active, new_active, &mut decision);
                decision
            }
            SpecialKeyMatch::ImeOn => {
                log::info!("IME ON (key combo)");
                self.build_ime_set_open_decision(ctx, true)
            }
            SpecialKeyMatch::ImeOff => {
                log::info!("IME OFF (key combo)");
                self.build_ime_set_open_decision(ctx, false)
            }
        }
    }

    /// 変換/無変換系の特殊キーを一括チェックし、一致した場合は状態変更して結果を返す。
    fn check_special_keys(&mut self, ctx: &InputContext, event: &RawKeyEvent) -> Option<Decision> {
        let m = self.match_special_keys(ctx, event)?;
        Some(self.apply_special_key_match(&m, ctx))
    }
}

#[allow(clippy::suspicious_operation_groupings)]
fn matches_key_combo(combo: ParsedKeyCombo, event: &RawKeyEvent, modifiers: ModifierState) -> bool {
    event.vk_code == combo.vk
        && combo.ctrl == modifiers.ctrl
        && combo.shift == modifiers.shift
        && combo.alt == modifiers.alt
}

impl SpecialKeyCombos {
    /// エンジン有効状態を考慮したうえでコンボマッチを行い、最初に一致した種別を返す。
    ///
    /// 副作用なし。`engine_enabled` は `adapter.is_enabled()` の値を渡すこと。
    fn match_event(
        &self,
        event: &RawKeyEvent,
        modifiers: ModifierState,
        engine_enabled: bool,
    ) -> Option<SpecialKeyMatch> {
        // エンジン ON/OFF コンボキー — user_enabled のみ変更
        if !engine_enabled
            && self
                .engine_on
                .iter()
                .any(|k| matches_key_combo(*k, event, modifiers))
        {
            return Some(SpecialKeyMatch::EngineOn);
        }
        if engine_enabled
            && self
                .engine_off
                .iter()
                .any(|k| matches_key_combo(*k, event, modifiers))
        {
            return Some(SpecialKeyMatch::EngineOff);
        }

        // IME 制御キー（エンジン状態に関わらずチェック）
        //
        // 注: 以前は thumb_vks のキーを除外するガードがあったが、
        // ModifierTiming の grace 猶予廃止（OS 実状態のみ使用）により
        // 誤マッチリスクが解消されたため除去。
        if self
            .ime_on
            .iter()
            .any(|k| matches_key_combo(*k, event, modifiers))
        {
            log::debug!(
                "[special-key] IME ON match: vk={:#06X} ctrl={} shift={} alt={} extra_info={:#x}",
                event.vk_code,
                modifiers.ctrl,
                modifiers.shift,
                modifiers.alt,
                event.extra_info
            );
            return Some(SpecialKeyMatch::ImeOn);
        }
        if self
            .ime_off
            .iter()
            .any(|k| matches_key_combo(*k, event, modifiers))
        {
            log::debug!(
                "[special-key] IME OFF match: vk={:#06X} ctrl={} shift={} alt={} extra_info={:#x}",
                event.vk_code,
                modifiers.ctrl,
                modifiers.shift,
                modifiers.alt,
                event.extra_info
            );
            return Some(SpecialKeyMatch::ImeOff);
        }

        None
    }
}
