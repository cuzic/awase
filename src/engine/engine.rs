//! 新 Engine: NicolaFsm + 特殊キー処理を統合するラッパー。
//!
//! `on_input` / `on_timeout` / `on_command` が唯一のエントリポイント。
//! OS API を一切呼ばず、副作用は `Decision` として返す。
//!
//! # 設計方針
//!
//! Engine は near-pure function として設計され��。
//! - 物理キー状態（修飾キー、親指キー）は Platform 層が InputTracker で追跡し、
//!   InputContext 経由で毎回渡す
//! - IME ガード（遷移中のキーバッファリング）は Platform 層が担当する
//! - Engine は InputContext のスナップショットだけで判断する（先読みしない）

use smallvec::smallvec;

use crate::config::ParsedKeyCombo;
use crate::types::{ContextChange, KeyEventType, RawKeyEvent};

use super::decision::{
    Decision, Effect, EffectVec, EngineCommand, ImeEffect, InputContext, InputEffect,
    SpecialKeyCombos, TimerEffect, UiEffect,
};
use super::fsm_adapter::FsmAdapter;
use super::fsm_types::ModifierState;
use super::input_tracker::PhysicalKeyState;
use super::key_lifecycle::KeyLifecycle;
use super::nicola_fsm::NicolaFsm;

/// 統合エンジン: NicolaFsm + 特殊キー処理
///
/// Engine の有効状態は2軸で決まる:
/// - `user_enabled`: ユーザーの意図（ホットキー/トレイで操作）�� FSM の `enabled` フラグ
/// - 環境前提条件: `InputContext { ime_on, is_romaji, is_japanese_ime, ... }` — Platform 層が毎回渡す
/// - 実効状態: `user_enabled && ctx.ime_on && ctx.is_romaji && ctx.is_japanese_ime`
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
    /// 最後のフォーカス情報
    last_focus_info: Option<(u32, String)>,
    /// 前回の呼び出し時の実効状態（遷移検知用）
    prev_active: bool,
}

impl Engine {
    #[must_use]
    pub const fn new(
        fsm: NicolaFsm,
        special_keys: SpecialKeyCombos,
    ) -> Self {
        Self {
            adapter: FsmAdapter::new(fsm),
            special_keys,
            lifecycle: KeyLifecycle::new(),
            last_focus_info: None,
            prev_active: false,
        }
    }

    /// InputContext から実効状態を計算する
    #[must_use]
    pub fn compute_active(&self, ctx: &InputContext) -> bool {
        self.adapter.is_enabled() && ctx.ime_on && ctx.is_romaji && ctx.is_japanese_ime
    }

    /// 実効状態の遷移を検知し、必要な Effect（flush, UI 通知）を返す。
    /// `prev_active` を更新する。
    fn check_active_transition(&mut self, ctx: &InputContext) -> EffectVec {
        let new_active = self.compute_active(ctx);
        let mut effects = EffectVec::new();

        if self.prev_active != new_active {
            if !new_active {
                // active → inactive: 保留キーをフラッシュ
                let flush = self.adapter.flush_to_effects(ContextChange::ImeOff);
                effects.extend(flush);
            }
            effects.push(Effect::Ui(UiEffect::EngineStateChanged {
                enabled: new_active,
            }));
            log::info!(
                "Engine {} (ime={}, romaji={}, japanese={}, user={})",
                if new_active { "activated" } else { "deactivated" },
                ctx.ime_on,
                ctx.is_romaji,
                ctx.is_japanese_ime,
                self.adapter.is_enabled(),
            );
            self.prev_active = new_active;
        }
        effects
    }

    /// キーイベントの統合エントリポイント。
    ///
    /// 処理フロー:
    /// 1. KeyUp 自動追跡
    /// 2. 特殊キー（エンジン ON/OFF + IME 制御）
    /// 3. 実効状態チェッ��� + 遷移検知
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
        // Prepend transition effects if any
        if !transition_effects.is_empty() {
            let effects = decision.effects_mut();
            for e in transition_effects.into_iter().rev() {
                effects.insert(0, e);
            }
        }
        decision
    }

    /// タイマー満了時のエントリポイント。
    pub fn on_timeout(&mut self, timer_id: usize, ctx: &InputContext) -> Decision {
        let phys = PhysicalKeyState::from_ctx_snapshot(ctx);

        // Engine が非活性なら on_timeout せず flush（コンテキスト喪失）
        if !self.compute_active(ctx) {
            return self.adapter.flush(ContextChange::ImeOff);
        }

        self.adapter.on_timeout(timer_id, &phys)
    }

    /// 外部コマンドの統合エントリポイント。
    ///
    /// `toggle_engine`, `invalidate_engine_context`, `swap_layout` 等の個別メソッドを
    /// 単一のデ���スパッチに集約する。
    pub fn on_command(&mut self, cmd: EngineCommand, ctx: &InputContext) -> Decision {
        match cmd {
            EngineCommand::ToggleEngine => {
                let old_active = self.compute_active(ctx);
                let (user_enabled, decision) = self.adapter.toggle_enabled();
                let new_active = self.compute_active(ctx);
                log::info!(
                    "Engine user_enabled toggled: {} (active: {})",
                    if user_enabled { "ON" } else { "OFF" },
                    if new_active { "ON" } else { "OFF" },
                );
                let mut decision = decision;
                if old_active != new_active {
                    decision.push_effect(Effect::Ui(UiEffect::EngineStateChanged {
                        enabled: new_active,
                    }));
                    self.prev_active = new_active;
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
            EngineCommand::FocusChanged(obs) => self.handle_focus_changed(ctx, obs),
        }
    }

    /// フォーカス変更の観測結果を処理し、コン���キスト無効化等の Decision を返す。
    /// フォーカス変更（前面プロセス変更）の処理。
    ///
    /// デバウンス後に Platform 層が前面プロセスの変化を検出した場合のみ呼ばれる（ADR 028）。
    /// focus_kind / app_kind / last_focus_info / キャッシュの更新は Platform 層で完了済み。
    /// Engine は pending flush と lifecycle 整合のみ担当する。
    fn handle_focus_changed(
        &mut self,
        ctx: &InputContext,
        obs: super::observation::FocusObservation,
    ) -> Decision {
        let mut effects = EffectVec::new();

        // Engine 内部の last_focus_info を更新
        self.last_focus_info = Some((obs.process_id, obs.class_name));

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

    /// user_enabled を直接設定する（テスト・初期化用）
    pub fn set_user_enabled(&mut self, enabled: bool) {
        let _ = self.adapter.set_enabled(enabled);
    }

    /// prev_active を直接設定する（テスト・初期化用）
    pub fn set_prev_active(&mut self, active: bool) {
        self.prev_active = active;
    }

    // ── 内部メソッド ──

    /// 変換/無変換系の特殊キーを一括チェックし、一致した場合は状態変更して結果を返す。
    fn check_special_keys(
        &mut self,
        ctx: &InputContext,
        event: &RawKeyEvent,
    ) -> Option<Decision> {
        let modifiers = ctx.modifiers;

        // エンジン ON/OFF コンボキー — user_enabled のみ変更
        if !self.adapter.is_enabled()
            && self
                .special_keys
                .engine_on
                .iter()
                .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            let old_active = self.compute_active(ctx);
            let (_, mut decision) = self.adapter.set_enabled(true);
            let new_active = self.compute_active(ctx);
            log::info!("Engine user_enabled ON (key combo, active={})", new_active);
            if old_active != new_active {
                decision.push_effect(Effect::Ui(UiEffect::EngineStateChanged {
                    enabled: new_active,
                }));
                self.prev_active = new_active;
            }
            return Some(decision);
        }
        if self.adapter.is_enabled()
            && self
                .special_keys
                .engine_off
                .iter()
                .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            let old_active = self.compute_active(ctx);
            let (_, mut decision) = self.adapter.set_enabled(false);
            let new_active = self.compute_active(ctx);
            log::info!("Engine user_enabled OFF (key combo, active={})", new_active);
            if old_active != new_active {
                decision.push_effect(Effect::Ui(UiEffect::EngineStateChanged {
                    enabled: new_active,
                }));
                self.prev_active = new_active;
            }
            return Some(decision);
        }

        // IME 制御キー（エンジン状態に関わらずチェック）
        // shadow 更新は Platform 層の責務（SetOpen Effect 処理時にアトミック反転）
        if self
            .special_keys
            .ime_on
            .iter()
            .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            log::info!("IME ON (key combo)");
            return Some(Decision::consumed_with(smallvec![Effect::Ime(
                ImeEffect::SetOpen(true),
            )]));
        }
        if self
            .special_keys
            .ime_off
            .iter()
            .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            log::info!("IME OFF (key combo)");
            return Some(Decision::consumed_with(smallvec![Effect::Ime(
                ImeEffect::SetOpen(false),
            )]));
        }

        None
    }

    /// キーコンボが修飾キー条件を含めてイベントに一致するか判定する。
    fn matches_key_combo(
        combo: ParsedKeyCombo,
        event: &RawKeyEvent,
        modifiers: ModifierState,
    ) -> bool {
        if event.vk_code != combo.vk {
            return false;
        }
        combo.ctrl == modifiers.ctrl && combo.shift == modifiers.shift && combo.alt == modifiers.alt
    }
}
