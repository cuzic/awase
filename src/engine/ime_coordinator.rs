//! IME 状態管理: shadow 追跡、ガード、同期キー判定
//!
//! Engine から IME 関連の状態管理ロジックを分離し、凝集度を高める。

use crate::types::{KeyEventType, RawKeyEvent};

use super::decision::{Decision, Effect, ImeEffect, ImeSyncKeys, KeyBuffer};
use super::input_tracker::PhysicalKeyState;

/// IME 状態管理: shadow 追跡、ガード、同期キー判定
#[derive(Debug)]
pub struct ImeCoordinator {
    shadow_on: bool,
    sync_keys: ImeSyncKeys,
    guard: KeyBuffer,
}

impl ImeCoordinator {
    #[must_use]
    pub const fn new(sync_keys: ImeSyncKeys) -> Self {
        Self {
            shadow_on: true, // safe default: engine ON
            sync_keys,
            guard: KeyBuffer::new(),
        }
    }

    #[must_use]
    pub const fn shadow_on(&self) -> bool {
        self.shadow_on
    }

    pub const fn set_shadow_on(&mut self, on: bool) {
        self.shadow_on = on;
    }

    /// Shadow IME 状態を更新する（ime_sync キー + IME 制御キー）
    pub fn update_shadow(&mut self, event: &RawKeyEvent) {
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if !is_key_down {
            return;
        }

        // ── ime_sync 設定キー ──
        let vk = event.vk_code;
        if self.sync_keys.on.contains(&vk) {
            self.shadow_on = true;
            log::debug!("Shadow IME ON (key 0x{:02X})", vk.0);
        }
        if self.sync_keys.off.contains(&vk) {
            self.shadow_on = false;
            log::debug!("Shadow IME OFF (key 0x{:02X})", vk.0);
        }
        if self.sync_keys.toggle.contains(&vk) {
            self.shadow_on = !self.shadow_on;
            log::debug!(
                "Shadow IME toggle → {} (key 0x{:02X})",
                self.shadow_on,
                vk.0
            );
        }

        // ── 日本語キーボード固有の IME ON/OFF キー ──
        if let Some(ime_key) = crate::vk::ImeKeyKind::from_vk(event.vk_code) {
            match ime_key.shadow_effect() {
                crate::vk::ShadowImeEffect::TurnOn => {
                    self.shadow_on = true;
                    log::trace!("Shadow IME ON ({ime_key:?})");
                }
                crate::vk::ShadowImeEffect::TurnOff => {
                    self.shadow_on = false;
                    log::trace!("Shadow IME OFF ({ime_key:?})");
                }
                crate::vk::ShadowImeEffect::Toggle => {
                    self.shadow_on = !self.shadow_on;
                    log::trace!("Shadow IME toggle → {} ({ime_key:?})", self.shadow_on);
                }
            }
        }
    }

    /// IME トグルガードを処理し、キーをバッファリングすべきか判定する。
    ///
    /// 戻り値:
    /// - `Some(Decision)` — 呼び出し側はこれを即座に返すべき
    /// - `None` — ガード処理なし、続行
    pub fn check_guard(
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
            // ime_sync_keys（設定ベース）と ImeKeyKind（ハードコード）の両方をチェック
            let is_sync_key = self.sync_keys.toggle.contains(&event.vk_code)
                || self.sync_keys.on.contains(&event.vk_code)
                || self.sync_keys.off.contains(&event.vk_code);
            let is_ime_key = crate::vk::ImeKeyKind::from_vk(event.vk_code).is_some();

            if is_sync_key || is_ime_key {
                // Set guard — next keys will be buffered
                self.guard.set_guard(true);
                log::debug!("IME toggle guard ON (vk=0x{:02X})", event.vk_code.0);
                // Prepend any accumulated effects, then pass through
                let all_effects = std::mem::take(effects);
                // pass through: let IME process the toggle
                if all_effects.is_empty() {
                    return Some(Decision::pass_through());
                }
                return Some(Decision::pass_through_with(all_effects));
            }

            // While IME guard active, buffer keys
            // 安全策: バッファが 10 キーを超えたらガードを強制解除（スタック防止）
            if self.guard.is_guarded() && self.guard.deferred_keys.len() >= 10 {
                log::warn!("IME guard forced clear: deferred buffer overflow");
                self.guard.set_guard(false);
                effects.push(Effect::Ime(ImeEffect::RequestCacheRefresh));
                return None; // ガード解除、通常処理に戻る
            }
            if self.guard.is_guarded() {
                self.guard.push_deferred(*event, *phys);
                // Return consumed + RequestImeCacheRefresh (via effects already accumulated)
                // plus a "process deferred" signal
                let mut all_effects = std::mem::take(effects);
                all_effects.push(Effect::Ime(ImeEffect::RequestCacheRefresh));
                return Some(Decision::consumed_with(all_effects));
            }
        }

        // Guard clear on KeyUp of any IME key.
        if !is_key_down && self.guard.is_guarded() {
            let is_sync_key = self.sync_keys.toggle.contains(&event.vk_code)
                || self.sync_keys.on.contains(&event.vk_code)
                || self.sync_keys.off.contains(&event.vk_code);
            let is_ime_key = crate::vk::ImeKeyKind::from_vk(event.vk_code).is_some();
            if is_sync_key || is_ime_key {
                self.guard.set_guard(false);
                effects.push(Effect::Ime(ImeEffect::RequestCacheRefresh));
            }
        }

        None
    }

    pub const fn set_guard(&mut self, on: bool) {
        self.guard.set_guard(on);
    }

    pub fn clear_deferred(&mut self) {
        self.guard.deferred_keys.clear();
    }

    pub fn drain_deferred(&mut self) -> Vec<(RawKeyEvent, PhysicalKeyState)> {
        self.guard.set_guard(false);
        self.guard.drain_deferred()
    }

    #[must_use]
    pub const fn is_guarded(&self) -> bool {
        self.guard.is_guarded()
    }

    pub fn reload_sync_keys(&mut self, keys: ImeSyncKeys) {
        self.sync_keys = keys;
    }
}
