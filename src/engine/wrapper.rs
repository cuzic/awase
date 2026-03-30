//! 新 Engine: NicolaFsm + InputTracker + IME/特殊キー処理を統合するラッパー。
//!
//! `on_input` / `on_timeout` が唯一のエントリポイント。
//! Win32 API を一切呼ばず、副作用は `Decision` として返す。

use timed_fsm::{Response, TimerCommand};

use crate::config::ParsedKeyCombo;
use crate::types::{ContextChange, KeyAction, KeyEventType, RawKeyEvent, VkCode};

use super::input_tracker::{InputTracker, PhysicalKeyState};
use super::types::{Decision, Effect, InputContext};
use super::NicolaFsm;

/// IME 同期キー（トグル・ON・OFF）を集約する構造体
#[derive(Debug)]
pub struct ImeSyncKeys {
    pub toggle: Vec<VkCode>,
    pub on: Vec<VkCode>,
    pub off: Vec<VkCode>,
}

/// エンジン切替・IME 制御の特殊キーコンボを集約する構造体。
#[derive(Debug)]
pub struct SpecialKeyCombos {
    pub engine_on: Vec<ParsedKeyCombo>,
    pub engine_off: Vec<ParsedKeyCombo>,
    pub ime_on: Vec<ParsedKeyCombo>,
    pub ime_off: Vec<ParsedKeyCombo>,
}

/// キーイベントバッファ管理
///
/// フック → メッセージループ間のキーイベント遅延・バッファリングを管理する。
/// OS 副作用は持たず、Engine メソッドがオーケストレーションを行う。
#[derive(Debug)]
pub struct KeyBuffer {
    /// IME 制御キー直後のガードフラグ（true: 後続キーを遅延処理する）
    pub ime_transition_guard: bool,
    /// ガード中に遅延されたキーイベント + 物理キー状態のバッファ
    pub deferred_keys: Vec<(RawKeyEvent, PhysicalKeyState)>,
}

impl Default for KeyBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyBuffer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ime_transition_guard: false,
            deferred_keys: Vec::new(),
        }
    }

    #[must_use]
    pub const fn is_guarded(&self) -> bool {
        self.ime_transition_guard
    }

    pub const fn set_guard(&mut self, on: bool) {
        self.ime_transition_guard = on;
    }

    pub fn push_deferred(&mut self, event: RawKeyEvent, phys: PhysicalKeyState) {
        self.deferred_keys.push((event, phys));
    }

    pub fn drain_deferred(&mut self) -> Vec<(RawKeyEvent, PhysicalKeyState)> {
        std::mem::take(&mut self.deferred_keys)
    }
}

/// 統合エンジン: NicolaFsm + InputTracker + IME/特殊キー処理
///
/// `on_input` が唯一のキーイベントエントリポイント。
/// Win32 API を一切呼ばず、副作用は `Decision` として返す。
#[allow(missing_debug_implementations)]
pub struct Engine {
    fsm: NicolaFsm,
    tracker: InputTracker,
    shadow_ime_on: bool,
    ime_sync_keys: ImeSyncKeys,
    special_keys: SpecialKeyCombos,
    key_buffer: KeyBuffer,
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
            fsm,
            tracker,
            shadow_ime_on: true, // safe default: engine ON
            ime_sync_keys,
            special_keys,
            key_buffer: KeyBuffer::new(),
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
        self.update_shadow_ime(&event);

        // Phase 3: IME key detection → request cache refresh
        let mut effects = Vec::new();
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if is_key_down && crate::vk::may_change_ime(event.vk_code) {
            effects.push(Effect::RequestImeCacheRefresh);
        }

        // Phase 4: IME toggle guard
        if let Some(decision) = self.check_ime_guard(&event, &phys, &mut effects) {
            return decision;
        }

        // Phase 5: Special keys (engine toggle + IME control)
        if is_key_down {
            if let Some(mut decision) = self.check_special_keys(&event) {
                // IME 制御キーの場合もキャッシュ更新を要求
                decision.effects.push(Effect::RequestImeCacheRefresh);
                return decision;
            }
        }

        // Phase 6: IME state check
        let ime_on = ctx.ime_cache.resolve_with_shadow(self.shadow_ime_on);
        if !ime_on {
            return Decision::pass_through();
        }

        // Phase 7: NicolaFsm
        let resp = self.fsm.on_event(event, &phys);
        self.response_to_decision(&resp)
    }

    /// タイマー満了時のエントリポイント。
    pub fn on_timeout(&mut self, timer_id: usize, ctx: &InputContext) -> Decision {
        let phys = self.tracker.snapshot();

        // IME が非活性なら on_timeout せず flush（コンテキスト喪失）
        let ime_on = ctx.ime_cache.resolve_with_shadow(self.shadow_ime_on);
        if !ime_on {
            let response = self.fsm.flush_pending(ContextChange::ImeOff);
            return self.response_to_decision(&response);
        }

        let response = self.fsm.on_timeout(timer_id, &phys);
        self.response_to_decision(&response)
    }

    /// 遅延キーを再処理し、Decision のリストを返す。
    ///
    /// メッセージループから呼ばれる。IME 状態キャッシュ更新後に呼ぶこと。
    pub fn process_deferred_keys(&mut self, ctx: &InputContext) -> Vec<Decision> {
        self.key_buffer.set_guard(false);
        let keys = self.key_buffer.drain_deferred();

        if keys.is_empty() {
            return vec![];
        }

        log::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

        let ime_on = ctx.ime_cache.resolve_with_shadow(self.shadow_ime_on);

        keys.into_iter()
            .map(|(event, phys)| {
                if ime_on {
                    let response = self.fsm.on_event(event, &phys);
                    self.response_to_decision(&response)
                } else {
                    Decision::with_effects(true, vec![Effect::ReinjectKey(event)])
                }
            })
            .collect()
    }

    /// エンジンの有効/無効を切り替え、トレイ更新の Effect を含む Decision を返す。
    pub fn toggle_engine(&mut self) -> Decision {
        let (enabled, flush_resp) = self.fsm.toggle_enabled();
        log::info!("Engine toggled: {}", if enabled { "ON" } else { "OFF" });
        let mut decision = self.response_to_decision(&flush_resp);
        decision.effects.push(Effect::UpdateTray { enabled });
        decision
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    pub fn invalidate_engine_context(&mut self, reason: ContextChange) -> Decision {
        let response = self.fsm.flush_pending(reason);
        self.response_to_decision(&response)
    }

    /// 配列を動的に切り替える。
    pub fn swap_layout(&mut self, layout: crate::yab::YabLayout) -> Decision {
        let response = self.fsm.swap_layout(layout);
        self.response_to_decision(&response)
    }

    /// NicolaFsm の設定変更への委譲メソッド群
    pub fn set_threshold_ms(&mut self, ms: u32) {
        self.fsm.set_threshold_ms(ms);
    }

    pub fn set_confirm_mode(
        &mut self,
        mode: crate::config::ConfirmMode,
        speculative_delay_ms: u32,
    ) {
        self.fsm.set_confirm_mode(mode, speculative_delay_ms);
    }

    pub fn set_ngram_model(&mut self, model: crate::ngram::NgramModel) {
        self.fsm.set_ngram_model(model);
    }

    #[must_use]
    pub const fn is_fsm_enabled(&self) -> bool {
        self.fsm.is_enabled()
    }

    #[must_use]
    pub const fn shadow_ime_on(&self) -> bool {
        self.shadow_ime_on
    }

    pub const fn set_shadow_ime_on(&mut self, on: bool) {
        self.shadow_ime_on = on;
    }

    /// 特殊キーコンボと IME 同期キーを一括再読み込みする
    pub fn reload_keys(&mut self, special: SpecialKeyCombos, sync: ImeSyncKeys) {
        self.special_keys = special;
        self.ime_sync_keys = sync;
    }

    /// ガード ON: 後続キーをバッファリングする
    pub const fn set_guard(&mut self, on: bool) {
        self.key_buffer.set_guard(on);
    }

    /// 遅延キーバッファをクリアする（フォーカスオーバーライド時等）
    pub fn clear_deferred_keys(&mut self) {
        self.key_buffer.deferred_keys.clear();
    }

    /// IME 状態変化に追随してエンジンの有効/無効を切り替える
    pub fn sync_with_ime_state(&mut self, ime_on: bool) -> Decision {
        if ime_on && !self.fsm.is_enabled() {
            let _ = self.fsm.set_enabled(true);
            Decision::with_effects(false, vec![Effect::UpdateTray { enabled: true }])
        } else if !ime_on && self.fsm.is_enabled() {
            let mut decision = self.invalidate_engine_context(ContextChange::ImeOff);
            let _ = self.fsm.set_enabled(false);
            decision.effects.push(Effect::UpdateTray { enabled: false });
            decision
        } else {
            Decision::pass_through()
        }
    }

    // ── 内部メソッド ──

    /// timed-fsm Response → Decision に変換
    #[allow(clippy::unused_self)] // メソッドチェーン可読性のために &self を保持
    fn response_to_decision(&self, resp: &Response<KeyAction, usize>) -> Decision {
        let mut effects = Vec::new();
        // Timer commands → Effects
        for cmd in &resp.timers {
            match cmd {
                TimerCommand::Set { id, duration } => {
                    effects.push(Effect::SetTimer {
                        id: *id,
                        duration: *duration,
                    });
                }
                TimerCommand::Kill { id } => {
                    effects.push(Effect::KillTimer(*id));
                }
            }
        }
        // Actions → SendKeys effect
        if !resp.actions.is_empty() {
            effects.push(Effect::SendKeys(resp.actions.clone()));
        }
        Decision::with_effects(resp.consumed, effects)
    }

    /// Shadow IME 状態を更新する（ime_sync キー + IME 制御キー）
    fn update_shadow_ime(&mut self, event: &RawKeyEvent) {
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if !is_key_down {
            return;
        }

        // ── ime_sync 設定キー ──
        let vk = event.vk_code;
        if self.ime_sync_keys.on.contains(&vk) {
            self.shadow_ime_on = true;
            log::debug!("Shadow IME ON (key 0x{:02X})", vk.0);
        }
        if self.ime_sync_keys.off.contains(&vk) {
            self.shadow_ime_on = false;
            log::debug!("Shadow IME OFF (key 0x{:02X})", vk.0);
        }
        if self.ime_sync_keys.toggle.contains(&vk) {
            self.shadow_ime_on = !self.shadow_ime_on;
            log::debug!(
                "Shadow IME toggle → {} (key 0x{:02X})",
                self.shadow_ime_on,
                vk.0
            );
        }

        // ── 日本語キーボード固有の IME ON/OFF キー ──
        if let Some(ime_key) = crate::vk::ImeKeyKind::from_vk(event.vk_code) {
            match ime_key.shadow_effect() {
                crate::vk::ShadowImeEffect::TurnOn => {
                    self.shadow_ime_on = true;
                    log::trace!("Shadow IME ON ({ime_key:?})");
                }
                crate::vk::ShadowImeEffect::TurnOff => {
                    self.shadow_ime_on = false;
                    log::trace!("Shadow IME OFF ({ime_key:?})");
                }
                crate::vk::ShadowImeEffect::Toggle => {
                    self.shadow_ime_on = !self.shadow_ime_on;
                    log::trace!("Shadow IME toggle → {} ({ime_key:?})", self.shadow_ime_on);
                }
            }
        }
    }

    /// IME トグルガードを処理し、キーをバッファリングすべきか判定する。
    ///
    /// 戻り値:
    /// - `Some(Decision)` — 呼び出し側はこれを即座に返すべき
    /// - `None` — ガード処理なし、続行
    fn check_ime_guard(
        &mut self,
        event: &RawKeyEvent,
        phys: &PhysicalKeyState,
        effects: &mut Vec<Effect>,
    ) -> Option<Decision> {
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );

        if is_key_down {
            // Check if current key IS a toggle/on/off key
            let is_toggle_key = self.ime_sync_keys.toggle.contains(&event.vk_code);
            let is_on_key = self.ime_sync_keys.on.contains(&event.vk_code);
            let is_off_key = self.ime_sync_keys.off.contains(&event.vk_code);

            if is_toggle_key || is_on_key || is_off_key {
                // Set guard — next keys will be buffered
                self.key_buffer.set_guard(true);
                log::debug!("IME toggle guard ON (vk=0x{:02X})", event.vk_code.0);
                // Prepend any accumulated effects, then pass through
                let all_effects = std::mem::take(effects);
                // pass through: let IME process the toggle
                return Some(Decision::with_effects(false, all_effects));
            }

            // While IME guard active, buffer keys
            if self.key_buffer.is_guarded() {
                self.key_buffer.push_deferred(*event, *phys);
                // Return consumed + RequestImeCacheRefresh (via effects already accumulated)
                // plus a "process deferred" signal
                let mut all_effects = std::mem::take(effects);
                all_effects.push(Effect::RequestImeCacheRefresh);
                return Some(Decision::consumed_with(all_effects));
            }
        }

        // Guard clear on KeyUp of toggle key
        if !is_key_down && self.key_buffer.is_guarded() {
            let is_toggle_key = self.ime_sync_keys.toggle.contains(&event.vk_code);
            let is_on_key = self.ime_sync_keys.on.contains(&event.vk_code);
            let is_off_key = self.ime_sync_keys.off.contains(&event.vk_code);
            if is_toggle_key || is_on_key || is_off_key {
                self.key_buffer.set_guard(false);
                effects.push(Effect::RequestImeCacheRefresh);
            }
        }

        None
    }

    /// 変換/無変換系の特殊キーを一括チェックし、一致した場合は状態変更して結果を返す。
    fn check_special_keys(&mut self, event: &RawKeyEvent) -> Option<Decision> {
        let modifiers = self.tracker.modifiers();

        // エンジントグルを先にチェック（より限定的な修飾キー）
        if !self.fsm.is_enabled()
            && self
                .special_keys
                .engine_on
                .iter()
                .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            let (enabled, flush_resp) = self.fsm.set_enabled(true);
            log::info!("Engine ON (key combo)");
            let mut decision = self.response_to_decision(&flush_resp);
            decision.consumed = true;
            decision.effects.push(Effect::UpdateTray { enabled });
            return Some(decision);
        }
        if self.fsm.is_enabled()
            && self
                .special_keys
                .engine_off
                .iter()
                .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            let (enabled, flush_resp) = self.fsm.set_enabled(false);
            log::info!("Engine OFF (key combo)");
            let mut decision = self.response_to_decision(&flush_resp);
            decision.consumed = true;
            decision.effects.push(Effect::UpdateTray { enabled });
            return Some(decision);
        }

        // IME 制御キー（エンジン状態に関わらずチェック）
        if self
            .special_keys
            .ime_on
            .iter()
            .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            self.shadow_ime_on = true;
            log::info!("IME ON (ImmSetOpenStatus, key combo)");
            return Some(Decision::consumed_with(vec![Effect::SetImeOpen(true)]));
        }
        if self
            .special_keys
            .ime_off
            .iter()
            .any(|k| Self::matches_key_combo(*k, event, modifiers))
        {
            self.shadow_ime_on = false;
            log::info!("IME OFF (ImmSetOpenStatus, key combo)");
            return Some(Decision::consumed_with(vec![Effect::SetImeOpen(false)]));
        }

        None
    }

    /// キーコンボが修飾キー条件を含めてイベントに一致するか判定する。
    ///
    /// InputTracker の修飾キー状態を使用する（Win32 API 不要）。
    fn matches_key_combo(
        combo: ParsedKeyCombo,
        event: &RawKeyEvent,
        modifiers: super::types::ModifierState,
    ) -> bool {
        if event.vk_code != combo.vk {
            return false;
        }
        combo.ctrl == modifiers.ctrl && combo.shift == modifiers.shift && combo.alt == modifiers.alt
    }
}
