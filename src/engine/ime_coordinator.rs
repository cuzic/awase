//! IME 状態管理: shadow 追跡、ガード、同期キー判定
//!
//! Engine から IME 関連の状態管理ロジックを分離し、凝集度を高める。

use crate::types::{KeyEventType, RawKeyEvent};

use super::decision::{Decision, Effect, EffectVec, ImeEffect, ImeSyncKeys, KeyBuffer};
use super::input_tracker::PhysicalKeyState;

/// IME トグルガード管理
///
/// Shadow IME 状態の追跡は Platform 層に移動済み。
/// Engine 側はガード（IME 遷移中のキーバッファリング）のみ担当する。
#[derive(Debug)]
pub struct ImeCoordinator {
    sync_keys: ImeSyncKeys,
    guard: KeyBuffer,
}

impl ImeCoordinator {
    #[must_use]
    pub const fn new(sync_keys: ImeSyncKeys) -> Self {
        Self {
            sync_keys,
            guard: KeyBuffer::new(),
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
        effects: &mut EffectVec,
    ) -> Option<Decision> {
        let is_key_down = matches!(event.event_type, KeyEventType::KeyDown);

        if is_key_down {
            // ime_sync_keys（設定ベース）のみ guard のトリガーにする。
            // ハードウェア IME キー（ロック型/トグル型で KeyUp が来ない）は guard に使わない。
            // それらは shadow 更新 + cache refresh で十分。
            let is_sync_key = event.ime_relevance.is_sync_key;

            if is_sync_key {
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
        }

        // ガード中は KeyDown/KeyUp 両方をバッファする。
        // KeyDown だけバッファして KeyUp を素通しすると、KeyDown が Consume されているのに
        // KeyUp だけ OS に渡り、修飾キーの状態が不整合になる。
        if self.guard.is_guarded() {
            // Guard clear on KeyUp of sync key.
            if !is_key_down && event.ime_relevance.is_sync_key {
                self.guard.set_guard(false);
                log::debug!("IME toggle guard OFF (vk=0x{:02X})", event.vk_code.0);
                effects.push(Effect::Ime(ImeEffect::RequestCacheRefresh));
                // sync key の KeyUp は素通し（OS に IME トグルを処理させる）
                let all_effects = std::mem::take(effects);
                if all_effects.is_empty() {
                    return Some(Decision::pass_through());
                }
                return Some(Decision::pass_through_with(all_effects));
            }

            // 安全策: バッファが 10 キーを超えたらガードを強制解除（スタック防止）
            if self.guard.deferred_keys.len() >= 10 {
                log::warn!("IME guard forced clear: deferred buffer overflow");
                self.guard.set_guard(false);
                effects.push(Effect::Ime(ImeEffect::RequestCacheRefresh));
                return None; // ガード解除、通常処理に戻る
            }

            // KeyDown も KeyUp もバッファする
            self.guard.push_deferred(*event, *phys);
            let mut all_effects = std::mem::take(effects);
            all_effects.push(Effect::Ime(ImeEffect::RequestCacheRefresh));
            return Some(Decision::consumed_with(all_effects));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ImeRelevance, KeyClassification, KeyEventType, ScanCode, VkCode,
    };

    fn make_event(event_type: KeyEventType) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: VkCode(0),
            scan_code: ScanCode(0),
            event_type,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
        }
    }

    fn empty_sync_keys() -> ImeSyncKeys {
        ImeSyncKeys {
            toggle: vec![],
            on: vec![],
            off: vec![],
        }
    }

    #[test]
    fn check_guard_sync_key_down_sets_guard_and_returns_pass_through() {
        let mut coord = ImeCoordinator::new(empty_sync_keys());
        let mut effects = EffectVec::new();

        let mut event = make_event(KeyEventType::KeyDown);
        event.ime_relevance.is_sync_key = true;
        let phys = PhysicalKeyState::empty();

        let decision = coord.check_guard(&event, &phys, &mut effects);
        assert!(decision.is_some());
        assert!(coord.is_guarded());

        // Decision should be pass-through
        let d = decision.unwrap();
        assert!(!d.is_consumed());
    }

    #[test]
    fn check_guard_while_guarded_buffers_keys_and_returns_consumed() {
        let mut coord = ImeCoordinator::new(empty_sync_keys());
        let mut effects = EffectVec::new();

        // First: set guard via sync key
        let mut sync_event = make_event(KeyEventType::KeyDown);
        sync_event.ime_relevance.is_sync_key = true;
        let phys = PhysicalKeyState::empty();
        coord.check_guard(&sync_event, &phys, &mut effects);
        assert!(coord.is_guarded());

        // Now send a regular key while guarded
        let regular_event = make_event(KeyEventType::KeyDown);
        let mut effects2 = EffectVec::new();
        let decision = coord.check_guard(&regular_event, &phys, &mut effects2);
        assert!(decision.is_some());
        assert!(decision.unwrap().is_consumed());
    }

    #[test]
    fn is_guarded_set_guard_round_trip() {
        let mut coord = ImeCoordinator::new(empty_sync_keys());
        assert!(!coord.is_guarded());
        coord.set_guard(true);
        assert!(coord.is_guarded());
        coord.set_guard(false);
        assert!(!coord.is_guarded());
    }

    #[test]
    fn drain_deferred_clears_guard_and_returns_buffered_keys() {
        let mut coord = ImeCoordinator::new(empty_sync_keys());
        coord.set_guard(true);

        // Push a deferred key manually via check_guard
        let event = make_event(KeyEventType::KeyDown);
        let phys = PhysicalKeyState::empty();
        let mut effects = EffectVec::new();
        coord.check_guard(&event, &phys, &mut effects);

        // Drain
        let deferred = coord.drain_deferred();
        assert!(!coord.is_guarded());
        assert_eq!(deferred.len(), 1);
    }

    #[test]
    fn reload_sync_keys_replaces_sync_keys() {
        let mut coord = ImeCoordinator::new(empty_sync_keys());

        let new_keys = ImeSyncKeys {
            toggle: vec![VkCode(0x19)],
            on: vec![VkCode(0xF2)],
            off: vec![VkCode(0xF3)],
        };
        coord.reload_sync_keys(new_keys);

        // Verify by confirming no panic and the coordinator still works
        assert!(!coord.is_guarded());
    }
}
