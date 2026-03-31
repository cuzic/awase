//! 新 Engine: NicolaFsm + InputTracker + IME/特殊キー処理を統合するラッパー。
//!
//! `on_input` / `on_timeout` / `on_command` が唯一のエントリポイント。
//! Win32 API を一切呼ばず、副作用は `Decision` として返す。
//!
//! # IME 状態の同期ルール
//!
//! - `ImeCoordinator::shadow_on`: 入力イベントから推定した IME 状態（Engine 内部）
//! - `InputContext::ime_cache`: メッセージループで観測した外界の IME 状態
//! - 判定: `ime_cache.resolve_with_shadow(shadow_on)` — キャッシュ優先、Unknown 時は shadow にフォールバック
//! - `Effect::Ime(ImeEffect::RequestCacheRefresh)` は非同期要求。次回の on_input で反映される保証はない
//! - Engine は常に現在の InputContext のスナップショットだけで判断する（先読みしない）

use crate::config::ParsedKeyCombo;
use crate::types::{ContextChange, KeyEventType, RawKeyEvent};

use super::decision::{
    Decision, Effect, EngineCommand, ImeEffect, ImeSyncKeys, InputContext, InputEffect,
    SpecialKeyCombos, TimerEffect, UiEffect,
};
use super::fsm_adapter::FsmAdapter;
use super::fsm_types::ModifierState;
use super::ime_coordinator::ImeCoordinator;
use super::input_tracker::InputTracker;
use super::nicola_fsm::NicolaFsm;

/// 統合エンジン: NicolaFsm + InputTracker + ImeCoordinator + 特殊キー処理
///
/// `on_input` が唯一のキーイベントエントリポイント。
/// Win32 API を一切呼ばず、副作用は `Decision` として返す。
#[allow(missing_debug_implementations)]
pub struct Engine {
    adapter: FsmAdapter,
    tracker: InputTracker,
    ime: ImeCoordinator,
    special_keys: SpecialKeyCombos,
    /// 最後のフォーカス情報（エンジン状態保存用）
    last_focus_info: Option<(u32, String)>,
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
            last_focus_info: None,
        }
    }

    /// キーイベントの統合エントリポイント。
    ///
    /// 処理フロー:
    /// 1. 物理キー状態追跡
    /// 2. Shadow IME 状態追跡
    /// 3. IME 制御キー検出 → キャッシュ更新要求
    /// 4. IME トグルガード（バッファリング）
    /// 5. エンジン ON/OFF トグルキー + IME 制御キー
    /// 6. IME 状態判定
    /// 7. NicolaFsm 処理
    pub fn on_input(&mut self, event: RawKeyEvent, ctx: &InputContext) -> Decision {
        // Phase 1: Physical key tracking
        let phys = self.tracker.process(&event);

        // Phase 2: Shadow IME update
        self.ime.update_shadow(&event);

        // Phase 3: IME key detection → request cache refresh
        let mut effects = Vec::new();
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if is_key_down && crate::vk::may_change_ime(event.vk_code) {
            effects.push(Effect::Ime(ImeEffect::RequestCacheRefresh));
        }

        // Phase 3.5: IME 変更キー検出時、保留キーを先にフラッシュする。
        // IME が切り替わる前に、現在の IME 状態で保留キーを確定する。
        let is_ime_change = is_key_down
            && (crate::vk::ImeKeyKind::from_vk(event.vk_code).is_some()
                || crate::vk::may_change_ime(event.vk_code));
        if is_ime_change {
            let flush_effects = self.adapter.flush_to_effects(ContextChange::ImeOff);
            effects.extend(flush_effects);
        }

        // Phase 4: IME toggle guard
        if let Some(decision) = self.ime.check_guard(&event, &phys, &mut effects) {
            return decision;
        }

        // Phase 5: Special keys (engine toggle + IME control)
        if is_key_down {
            if let Some(mut decision) = self.check_special_keys(&event) {
                decision.push_effect(Effect::Ime(ImeEffect::RequestCacheRefresh));
                return decision;
            }
        }

        // Phase 6: IME state check
        // 蓄積された effects（RequestImeCacheRefresh 等）を含めて返す。
        // effects を捨てると IME キャッシュ更新が行われず、
        // 半角/全角後にエンジンが有効にならない。
        let ime_on = ctx.ime_cache.resolve_with_shadow(self.ime.shadow_on());
        if !ime_on {
            if effects.is_empty() {
                return Decision::pass_through();
            }
            return Decision::pass_through_with(effects);
        }

        // Phase 7: NicolaFsm
        self.adapter.on_event(event, &phys)
    }

    /// タイマー満了時のエントリポイント。
    pub fn on_timeout(&mut self, timer_id: usize, ctx: &InputContext) -> Decision {
        let phys = self.tracker.snapshot();

        // IME が非活性なら on_timeout せず flush（コンテキスト喪失）
        let ime_on = ctx.ime_cache.resolve_with_shadow(self.ime.shadow_on());
        if !ime_on {
            return self.adapter.flush(ContextChange::ImeOff);
        }

        self.adapter.on_timeout(timer_id, &phys)
    }

    /// 遅延キーを再処理し、Decision のリストを返す。
    ///
    /// メッセージループから呼ばれる。IME 状態キャッシュ更新後に呼ぶこと。
    pub fn process_deferred_keys(&mut self, ctx: &InputContext) -> Vec<Decision> {
        let keys = self.ime.drain_deferred();

        if keys.is_empty() {
            return vec![];
        }

        log::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

        let ime_on = ctx.ime_cache.resolve_with_shadow(self.ime.shadow_on());

        keys.into_iter()
            .map(|(event, phys)| {
                if ime_on {
                    self.adapter.on_event(event, &phys)
                } else {
                    Decision::consumed_with(vec![Effect::Input(InputEffect::ReinjectKey(event))])
                }
            })
            .collect()
    }

    /// 外部コマンドの統合エントリポイント。
    ///
    /// `toggle_engine`, `invalidate_engine_context`, `swap_layout` 等の個別メソッドを
    /// 単一のディスパッチに集約する。
    pub fn on_command(&mut self, cmd: EngineCommand) -> Decision {
        match cmd {
            EngineCommand::ToggleEngine => {
                let (enabled, decision) = self.adapter.toggle_enabled();
                log::info!("Engine toggled: {}", if enabled { "ON" } else { "OFF" });
                let mut decision = decision;
                decision.push_effect(Effect::Ui(UiEffect::EngineStateChanged { enabled }));
                decision
            }
            EngineCommand::InvalidateContext(reason) => self.adapter.flush(reason),
            EngineCommand::SwapLayout(layout) => self.adapter.swap_layout(layout),
            EngineCommand::SyncImeState { ime_on } => {
                if ime_on && !self.adapter.is_enabled() {
                    let _ = self.adapter.set_enabled(true);
                    Decision::pass_through_with(vec![Effect::Ui(UiEffect::EngineStateChanged {
                        enabled: true,
                    })])
                } else if !ime_on && self.adapter.is_enabled() {
                    let mut decision = self.adapter.flush(ContextChange::ImeOff);
                    let _ = self.adapter.set_enabled(false);
                    decision
                        .push_effect(Effect::Ui(UiEffect::EngineStateChanged { enabled: false }));
                    decision
                } else {
                    Decision::pass_through()
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
            EngineCommand::ImeObserved(obs) => self.handle_ime_observed(obs),
            EngineCommand::FocusChanged(obs) => self.handle_focus_changed(obs),
        }
    }

    /// IME 観測結果を処理し、キャッシュ更新 + エンジン同期の Decision を返す。
    fn handle_ime_observed(&mut self, obs: super::observation::ImeObservation) -> Decision {
        use super::decision::ImeCacheEffect;

        let Some(ime_on) = obs.resolve(self.ime.shadow_on()) else {
            return Decision::pass_through();
        };

        let mut effects = Vec::new();

        // エンジンを IME 状態に追随させる（SyncImeState と同じロジック）
        // フラッシュをキャッシュ更新より先に実行する（保留キーが消失しないように）
        if ime_on && !self.adapter.is_enabled() {
            let _ = self.adapter.set_enabled(true);
            effects.push(Effect::Ui(UiEffect::EngineStateChanged { enabled: true }));
            log::info!("Engine auto-enabled (IME ON)");
        } else if !ime_on && self.adapter.is_enabled() {
            let flush_effects = self.adapter.flush_to_effects(ContextChange::ImeOff);
            effects.extend(flush_effects);
            let _ = self.adapter.set_enabled(false);
            effects.push(Effect::Ui(UiEffect::EngineStateChanged { enabled: false }));
            log::info!("Engine auto-disabled (IME OFF)");
        }

        // キャッシュ更新はフラッシュの後（保留キーの出力が先に実行される）
        effects.push(Effect::ImeCache(ImeCacheEffect::UpdateStateCache {
            ime_on,
        }));

        Decision::pass_through_with(effects)
    }

    /// フォーカス変更の観測結果を処理し、コンテキスト無効化等の Decision を返す。
    fn handle_focus_changed(&mut self, obs: super::observation::FocusObservation) -> Decision {
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
        let cached_engine_enabled = obs.cached_engine_enabled;
        let class_name = obs.class_name; // move ownership

        let mut effects: Vec<Effect> = Vec::new();

        // 旧ウィンドウのエンジン状態をキャッシュに保存
        if let Some((old_pid, ref old_class)) = self.last_focus_info {
            effects.push(Effect::Focus(FocusEffect::SaveEngineState {
                process_id: old_pid,
                class_name: old_class.clone(),
                enabled: self.adapter.is_enabled(),
            }));
        }

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

        // IME トグルガードもクリア（前ウィンドウのガードを引きずらない）
        self.ime.clear_deferred();
        self.ime.set_guard(false);

        // UIA 非同期判定が必要なら要求
        if needs_uia {
            effects.push(Effect::Focus(FocusEffect::RequestUiaClassification));
        }

        // フォーカス変更デバウンスタイマー
        effects.push(Effect::Timer(TimerEffect::Set {
            id: debounce_timer_id,
            duration: std::time::Duration::from_millis(debounce_ms),
        }));

        // 新ウィンドウのエンジン状態を復元
        if let Some(enabled) = cached_engine_enabled {
            if enabled != self.adapter.is_enabled() {
                let _ = self.adapter.set_enabled(enabled);
                effects.push(Effect::Ui(UiEffect::EngineStateChanged { enabled }));
                log::info!(
                    "Engine state restored for window: {}",
                    if enabled { "ON" } else { "OFF" }
                );
            }
        }

        Decision::pass_through_with(effects)
    }

    #[must_use]
    pub const fn is_fsm_enabled(&self) -> bool {
        self.adapter.is_enabled()
    }

    #[must_use]
    pub const fn shadow_ime_on(&self) -> bool {
        self.ime.shadow_on()
    }

    pub const fn set_shadow_ime_on(&mut self, on: bool) {
        self.ime.set_shadow_on(on);
    }

    // ── 内部メソッド ──

    /// 変換/無変換系の特殊キーを一括チェックし、一致した場合は状態変更して結果を返す。
    fn check_special_keys(&mut self, event: &RawKeyEvent) -> Option<Decision> {
        let modifiers = self.tracker.modifiers();

        // エンジントグルを先にチェック（より限定的な修飾キー）
        if !self.adapter.is_enabled()
            && self
                .special_keys
                .engine_on
                .iter()
                .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            let (enabled, mut decision) = self.adapter.set_enabled(true);
            log::info!("Engine ON (key combo)");
            decision.push_effect(Effect::Ui(UiEffect::EngineStateChanged { enabled }));
            // ウィンドウごとのエンジン状態をキャッシュに保存
            if let Some((pid, ref cls)) = self.last_focus_info {
                decision.push_effect(Effect::Focus(
                    super::decision::FocusEffect::SaveEngineState {
                        process_id: pid,
                        class_name: cls.clone(),
                        enabled,
                    },
                ));
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
            let (enabled, mut decision) = self.adapter.set_enabled(false);
            log::info!("Engine OFF (key combo)");
            decision.push_effect(Effect::Ui(UiEffect::EngineStateChanged { enabled }));
            // ウィンドウごとのエンジン状態をキャッシュに保存
            if let Some((pid, ref cls)) = self.last_focus_info {
                decision.push_effect(Effect::Focus(
                    super::decision::FocusEffect::SaveEngineState {
                        process_id: pid,
                        class_name: cls.clone(),
                        enabled,
                    },
                ));
            }
            return Some(decision);
        }

        // IME 制御キー（エンジン状態に関わらずチェック）
        if self
            .special_keys
            .ime_on
            .iter()
            .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            self.ime.set_shadow_on(true);
            log::info!("IME ON (ImmSetOpenStatus, key combo)");
            return Some(Decision::consumed_with(vec![Effect::Ime(
                ImeEffect::SetOpen(true),
            )]));
        }
        if self
            .special_keys
            .ime_off
            .iter()
            .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            self.ime.set_shadow_on(false);
            log::info!("IME OFF (ImmSetOpenStatus, key combo)");
            return Some(Decision::consumed_with(vec![Effect::Ime(
                ImeEffect::SetOpen(false),
            )]));
        }

        None
    }

    /// キーコンボが修飾キー条件を含めてイベントに一致するか判定する。
    ///
    /// InputTracker の修飾キー状態を使用する（Win32 API 不要）。
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
