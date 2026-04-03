//! 新 Engine: NicolaFsm + InputTracker + IME/特殊キー処理を統合するラッパー。
//!
//! `on_input` / `on_timeout` / `on_command` が唯一のエントリポイント。
//! OS API を一切呼ばず、副作用は `Decision` として返す。
//!
//! # IME 状態の同期ルール
//!
//! - `ImeCoordinator::shadow_on`: 入力イベントから推定した IME 状態（Engine 内部）
//! - `InputContext::ime_cache`: メッセージループで観測した外界の IME 状態
//! - 判定: `ime_cache.resolve_with_shadow(shadow_on)` — キャッシュ優先、Unknown 時は shadow にフォールバック
//! - `Effect::Ime(ImeEffect::RequestCacheRefresh)` は非同期要求。次回の on_input で反映される保証はない
//! - Engine は常に現在の InputContext のスナップショットだけで判断する（先読みしない）

use smallvec::smallvec;

use crate::config::ParsedKeyCombo;
use crate::types::{ContextChange, KeyEventType, RawKeyEvent};

use super::decision::{
    Decision, Effect, EffectVec, EngineCommand, ImeEffect, ImeSyncKeys, InputContext, InputEffect,
    SpecialKeyCombos, TimerEffect, UiEffect,
};
use super::fsm_adapter::FsmAdapter;
use super::fsm_types::ModifierState;
use super::ime_coordinator::ImeCoordinator;
use super::input_tracker::InputTracker;
use super::key_lifecycle::KeyLifecycle;
use super::nicola_fsm::NicolaFsm;

/// 統合エンジン: NicolaFsm + InputTracker + ImeCoordinator + 特殊キー処理
///
/// Engine の有効状態は2軸で決まる:
/// - `user_enabled`: ユーザーの意図（ホットキー/トレイで操作）— FSM の `enabled` フラグ
/// - 環境前提条件: `InputContext { ime_on, is_romaji, is_japanese_ime }` — Platform 層が毎回渡す
/// - 実効状態: `user_enabled && ctx.ime_on && ctx.is_romaji && ctx.is_japanese_ime`
///
/// Engine は前提条件を内部にキャッシュしない。毎回の呼び出しで Platform 層から受け取る。
///
/// `on_input` が唯一のキーイベントエントリポイント。
/// OS API を一切呼ばず、副作用は `Decision` として返す。
#[allow(missing_debug_implementations)]
pub struct Engine {
    adapter: FsmAdapter,
    tracker: InputTracker,
    ime: ImeCoordinator,
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
        tracker: InputTracker,
        ime_sync_keys: ImeSyncKeys,
        special_keys: SpecialKeyCombos,
    ) -> Self {
        Self {
            adapter: FsmAdapter::new(fsm),
            tracker,
            ime: ImeCoordinator::new(ime_sync_keys),
            special_keys,
            lifecycle: KeyLifecycle::new(),
            last_focus_info: None,
            prev_active: false,
        }
    }

    /// InputContext から実効状態を計算する
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
    /// 1. 物理キー状態追跡
    /// 2. IME 変更キー検出 → 保留フラッシュ
    /// 3. IME トグルガード（バッファリング）
    /// 4. エンジン ON/OFF トグルキー + IME 制御キー
    /// 5. 実効状態チェック + 遷移検知
    /// 6. NicolaFsm 処理
    pub fn on_input(&mut self, event: RawKeyEvent, ctx: &InputContext) -> Decision {
        // Phase 0: KeyUp 自動追跡
        let is_key_down = matches!(event.event_type, KeyEventType::KeyDown);
        if !is_key_down && self.lifecycle.on_key_up(event.vk_code) {
            return Decision::consumed();
        }

        // Phase 1: Physical key tracking
        let phys = self.tracker.process(&event);

        // Phase 2: IME 変更キー検出 → 保留キーをフラッシュ
        // IME トグルキーはフックで捕捉された時点で OS にまだ届いていないため、
        // Platform 層が shadow（アトミック変数）を即座反転し、InputContext に反映済み。
        let mut effects = EffectVec::new();
        let is_ime_change = is_key_down && event.ime_relevance.may_change_ime;
        if is_ime_change {
            let flush_effects = self.adapter.flush_to_effects(ContextChange::ImeOff);
            effects.extend(flush_effects);
        }

        // Phase 3: IME toggle guard
        if let Some(decision) = self.ime.check_guard(&event, &phys, &mut effects) {
            if is_key_down && decision.is_consumed() {
                self.lifecycle.on_key_down_consumed(&event);
            }
            return decision;
        }

        // Phase 4: Special keys (engine toggle + IME control)
        if is_key_down {
            if let Some(mut decision) = self.check_special_keys(ctx, &event) {
                decision.push_effect(Effect::Ime(ImeEffect::RequestCacheRefresh));
                if decision.is_consumed() {
                    self.lifecycle.on_key_down_consumed(&event);
                }
                return decision;
            }
        }

        // Phase 5: Active state check + transition detection
        let transition_effects = self.check_active_transition(ctx);
        effects.extend(transition_effects);

        if !self.compute_active(ctx) {
            if effects.is_empty() {
                return Decision::pass_through();
            }
            return Decision::pass_through_with(effects);
        }

        // Phase 6: NicolaFsm
        let decision = self.adapter.on_event(event, &phys);
        if is_key_down && decision.is_consumed() {
            self.lifecycle.on_key_down_consumed(&event);
        }
        decision
    }

    /// タイマー満了時のエントリポイント。
    pub fn on_timeout(&mut self, timer_id: usize, ctx: &InputContext) -> Decision {
        let phys = self.tracker.snapshot();

        // Engine が非活性なら on_timeout せず flush（コンテキスト喪失）
        if !self.compute_active(ctx) {
            return self.adapter.flush(ContextChange::ImeOff);
        }

        self.adapter.on_timeout(timer_id, &phys)
    }

    /// 遅延キーを再処理し、Decision のリストを返す。
    ///
    /// メッセージループから呼ばれる。IME 状態更新後に呼ぶこと。
    pub fn process_deferred_keys(&mut self, ctx: &InputContext) -> Vec<Decision> {
        let keys = self.ime.drain_deferred();

        if keys.is_empty() {
            return vec![];
        }

        log::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

        let active = self.compute_active(ctx);

        keys.into_iter()
            .map(|(event, phys)| {
                if active {
                    self.adapter.on_event(event, &phys)
                } else {
                    Decision::consumed_with(smallvec![Effect::Input(InputEffect::ReinjectKey(
                        event
                    ))])
                }
            })
            .collect()
    }

    /// 外部コマンドの統合エントリポイント。
    ///
    /// `toggle_engine`, `invalidate_engine_context`, `swap_layout` 等の個別メソッドを
    /// 単一のディスパッチに集約する。
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
            EngineCommand::SyncImeState { .. } => {
                // Platform 層がアトミック変数を更新済み。ctx に反映されている。
                let effects = self.check_active_transition(ctx);
                if effects.is_empty() {
                    Decision::pass_through()
                } else {
                    Decision::pass_through_with(effects)
                }
            }
            EngineCommand::SetGuard(on) => {
                self.ime.set_guard(on);
                Decision::pass_through()
            }
            EngineCommand::ClearDeferredKeys => {
                self.ime.clear_deferred();
                Decision::pass_through()
            }
            EngineCommand::ReloadKeys { special, sync } => {
                self.special_keys = special;
                self.ime.reload_sync_keys(sync);
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
            EngineCommand::ImeObserved(_obs) => {
                // Platform 層がアトミック変数を更新済み。ctx に反映されている。
                let effects = self.check_active_transition(ctx);
                if effects.is_empty() {
                    Decision::pass_through()
                } else {
                    Decision::pass_through_with(effects)
                }
            }
            EngineCommand::FocusChanged(obs) => self.handle_focus_changed(ctx, obs),
            EngineCommand::SyncModifiers(os_mods) => self.handle_sync_modifiers(os_mods),
        }
    }

    /// フォーカス変更の観測結果を処理し、コンテキスト無効化等の Decision を返す。
    fn handle_focus_changed(
        &mut self,
        ctx: &InputContext,
        obs: super::observation::FocusObservation,
    ) -> Decision {
        use super::decision::FocusEffect;

        if obs.skip {
            return Decision::pass_through();
        }

        let kind = obs.kind;
        let process_id = obs.process_id;
        let needs_uia = obs.needs_uia;
        let overridden = obs.overridden;
        let debounce_timer_id = obs.debounce_timer_id;
        let debounce_ms = obs.debounce_ms;
        let class_name = obs.class_name; // move ownership

        let mut effects = EffectVec::new();

        // last_focus_info を Engine 内部でも更新
        self.last_focus_info = Some((process_id, class_name.clone()));

        // last_focus_info を更新（Executor 側）
        effects.push(Effect::Focus(FocusEffect::UpdateLastFocusInfo {
            process_id,
            class_name: class_name.clone(),
        }));

        // IME 信頼度をリセット
        effects.push(Effect::Focus(FocusEffect::ResetImeReliability));

        // FOCUS_KIND を更新
        effects.push(Effect::Focus(FocusEffect::UpdateFocusKind(kind)));

        // キャッシュ格納（オーバーライドでない場合のみ）
        if !overridden {
            effects.push(Effect::Focus(FocusEffect::InsertFocusCache {
                process_id,
                class_name,
                kind,
            }));
        }

        // OS から取得した修飾キー状態で InputTracker を同期する。
        // フォーカス変更中にフックが取りこぼした修飾キーの押下/解放を補正する。
        if let Some(mods) = obs.os_modifiers {
            self.tracker.set_modifiers(mods);
        }

        // ウィンドウ切替時は常に内部状態をフラッシュする。
        // 前のウィンドウで入力途中だったキーを別のウィンドウに持ち越さない。
        let flush_effects = self.adapter.flush_to_effects(ContextChange::FocusChanged);
        effects.extend(flush_effects);

        // IME トグルガードをクリア（deferred keys は破棄）
        self.ime.clear_deferred();

        // Consume 済みで KeyUp が来ていないキーの KeyUp を再注入して
        // OS 側のキーボード状態と整合させる。
        let pending_key_ups = self.lifecycle.flush_pending_key_ups();
        for evt in pending_key_ups {
            effects.push(Effect::Input(InputEffect::ReinjectKey(evt)));
        }

        // UIA 非同期判定が必要なら要求
        if needs_uia {
            effects.push(Effect::Focus(FocusEffect::RequestUiaClassification));
        }

        // フォーカス変更デバウンスタイマー
        effects.push(Effect::Timer(TimerEffect::Set {
            id: debounce_timer_id,
            duration: std::time::Duration::from_millis(debounce_ms),
        }));

        // 実効状態の遷移を検知（Platform 層が ctx を新ウィンドウの状態で構築済み）
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

    /// OS の修飾キー状態と Engine 内部状態を同期し、不整合があれば KeyUp を再注入する。
    ///
    /// IME トグル直後など、フックが修飾キーの KeyUp を取りこぼす可能性がある
    /// タイミングで呼ぶ。
    fn handle_sync_modifiers(&mut self, os_mods: ModifierState) -> Decision {
        let engine_mods = self.tracker.modifiers();
        let mut effects = EffectVec::new();

        // Engine が「押下中」と思っているが OS では離されているキー
        // → lifecycle から KeyUp を再注入
        // Engine 側も内部状態を OS に合わせる
        let pending_ups = self.lifecycle.flush_pending_key_ups();
        for evt in pending_ups {
            // OS で既に離されている修飾キーの KeyUp のみ再注入
            let should_reinject = match evt.modifier_key {
                Some(crate::types::ModifierKey::Ctrl) => engine_mods.ctrl && !os_mods.ctrl,
                Some(crate::types::ModifierKey::Alt) => engine_mods.alt && !os_mods.alt,
                Some(crate::types::ModifierKey::Shift) => engine_mods.shift && !os_mods.shift,
                Some(crate::types::ModifierKey::Meta) => engine_mods.win && !os_mods.win,
                None => false, // 修飾キー以外は不整合チェック不要
            };
            if should_reinject {
                log::info!(
                    "Modifier sync: reinjecting KeyUp for vk=0x{:02X}",
                    evt.vk_code.0
                );
                effects.push(Effect::Input(InputEffect::ReinjectKey(evt)));
            }
        }

        // InputTracker の修飾キー状態を OS に合わせる
        self.tracker.set_modifiers(os_mods);

        if effects.is_empty() {
            Decision::pass_through()
        } else {
            Decision::pass_through_with(effects)
        }
    }

    // ── 内部メソッド ──

    /// 変換/無変換系の特殊キーを一括チェックし、一致した場合は状態変更して結果を返す。
    fn check_special_keys(
        &mut self,
        ctx: &InputContext,
        event: &RawKeyEvent,
    ) -> Option<Decision> {
        let modifiers = self.tracker.modifiers();

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
    ///
    /// InputTracker の修飾キー状態を使用する（OS API 不要）。
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
