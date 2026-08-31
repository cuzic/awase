//! キーの Down/Up ペア追跡
//!
//! Engine が KeyDown を Consume した場合、対応する KeyUp も必ず Consume すべき。
//! KeyDown を PassThrough した場合、対応する KeyUp も PassThrough すべき。
//! この不変条件を保証する。
//!
//! コンテキスト変更（フォーカス移動、IME OFF 等）時は、Consume 済みだが
//! KeyUp が来ていないキーの KeyUp を OS に再注入して状態を整合させる。

use crate::types::{RawKeyEvent, VkCode};

/// KeyUp が到着したときに Engine が負う義務（ADR-112 決定2）。
///
/// 旧実装（`on_key_up` が bool を返し、Consume 済みなら `Engine::on_input` が
/// FSM を呼ばずに即 `Decision::consumed()` を返していた）は、「OS へ漏らさない」
/// という義務の判定と、「FSM にイベントを渡すかどうか」を同じ早期 return に
/// 混同していたため、`NicolaFsm::on_key_up` 配下の KeyUp 処理が実運用で
/// 一切呼ばれないという構造的リグレッション（BUG-101）を生んでいた。
///
/// `UpDuty` は前者（義務の有無）だけを表す。イベントは義務の有無によらず
/// 常に FSM まで届き、`Engine::on_input` の唯一の出口で `Consume` 義務が
/// あれば `Decision::force_consume()` により最終的に必ず Consume へ格上げする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpDuty {
    /// 対応する KeyDown が無い、または PassThrough だった。義務なし。
    None,
    /// 対応する KeyDown が Consume 済み。KeyUp も最終的に必ず Consume へ格上げする。
    Consume,
}

/// Consume 済みで KeyUp 待ちのキー
#[derive(Debug, Clone, Copy)]
struct ActiveKey {
    vk_code: VkCode,
    /// 再注入用の元イベントデータ
    event: RawKeyEvent,
}

/// キーの Down/Up ペア追跡
#[derive(Debug)]
pub struct KeyLifecycle {
    /// Consume 済みで KeyUp 待ちのキー一覧
    active_keys: Vec<ActiveKey>,
}

impl Default for KeyLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyLifecycle {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active_keys: Vec::new(),
        }
    }

    /// KeyDown が Consume された場合に呼ぶ。対応する KeyUp も Consume すべきことを記録。
    pub fn on_key_down_consumed(&mut self, event: &RawKeyEvent) {
        let vk = event.vk_code;
        // 同じキーの重複登録を防ぐ（キーリピートの場合）
        if !self.active_keys.iter().any(|k| k.vk_code == vk) {
            self.active_keys.push(ActiveKey {
                vk_code: vk,
                event: *event,
            });
        }
    }

    /// KeyUp が到着した場合に呼ぶ。対応する KeyDown が Consume 済みなら
    /// `active_keys` から除去して `UpDuty::Consume` を返す（KeyUp は最終的に
    /// 必ず Consume すべき）。対応が無ければ `UpDuty::None`（PassThrough すべき）。
    ///
    /// 除去はするが、イベント自体を FSM に渡すかどうかはこのメソッドの
    /// 呼び出し元（`Engine::on_input`）が決める（ADR-112 決定2、`UpDuty` のdoc参照）。
    pub(super) fn take_key_up_duty(&mut self, vk_code: VkCode) -> UpDuty {
        if let Some(pos) = self.active_keys.iter().position(|k| k.vk_code == vk_code) {
            self.active_keys.remove(pos);
            UpDuty::Consume
        } else {
            UpDuty::None
        }
    }

    /// コンテキスト変更時: Consume 済みだが KeyUp が来ていないキーの KeyUp を
    /// 再注入用イベントとして返す。OS 側のキーボード状態と整合させる。
    ///
    /// 返されたイベントは `event_type` が `KeyUp` に書き換えられている。
    pub fn flush_pending_key_ups(&mut self) -> Vec<RawKeyEvent> {
        let keys = std::mem::take(&mut self.active_keys);
        keys.into_iter()
            .map(|k| {
                let mut evt = k.event;
                evt.event_type = crate::types::KeyEventType::KeyUp;
                evt
            })
            .collect()
    }

    /// アクティブキーの数
    #[must_use]
    pub const fn active_count(&self) -> usize {
        self.active_keys.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn make_event(vk: VkCode, event_type: KeyEventType) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: vk,
            scan_code: ScanCode(0),
            event_type,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
            modifier_snapshot: Default::default(),
            injected: false,
        }
    }

    #[test]
    fn consumed_key_down_makes_key_up_consumed() {
        let mut lc = KeyLifecycle::new();
        let event = make_event(VkCode(0x41), KeyEventType::KeyDown);

        lc.on_key_down_consumed(&event);
        assert_eq!(lc.active_count(), 1);

        assert_eq!(lc.take_key_up_duty(VkCode(0x41)), UpDuty::Consume);
        assert_eq!(lc.active_count(), 0);
    }

    #[test]
    fn non_consumed_key_up_passes_through() {
        let mut lc = KeyLifecycle::new();
        assert_eq!(lc.take_key_up_duty(VkCode(0x41)), UpDuty::None);
    }

    #[test]
    fn flush_returns_pending_key_ups() {
        let mut lc = KeyLifecycle::new();
        lc.on_key_down_consumed(&make_event(VkCode(0x10), KeyEventType::KeyDown));
        lc.on_key_down_consumed(&make_event(VkCode(0x41), KeyEventType::KeyDown));

        let flushed = lc.flush_pending_key_ups();
        assert_eq!(flushed.len(), 2);
        assert!(flushed.iter().all(|e| e.event_type == KeyEventType::KeyUp));
        assert_eq!(lc.active_count(), 0);
    }

    #[test]
    fn duplicate_key_down_not_doubled() {
        let mut lc = KeyLifecycle::new();
        let event = make_event(VkCode(0x41), KeyEventType::KeyDown);
        lc.on_key_down_consumed(&event);
        lc.on_key_down_consumed(&event); // repeat
        assert_eq!(lc.active_count(), 1);
    }

    #[test]
    fn flush_pending_key_ups_sets_event_type_to_keyup() {
        let mut lc = KeyLifecycle::new();
        lc.on_key_down_consumed(&make_event(VkCode(0x41), KeyEventType::KeyDown));
        lc.on_key_down_consumed(&make_event(VkCode(0x42), KeyEventType::KeyDown));

        let flushed = lc.flush_pending_key_ups();
        for evt in &flushed {
            assert_eq!(evt.event_type, KeyEventType::KeyUp);
        }
        assert_eq!(flushed[0].vk_code, VkCode(0x41));
        assert_eq!(flushed[1].vk_code, VkCode(0x42));
    }

    #[test]
    fn on_key_up_for_never_consumed_returns_none() {
        let mut lc = KeyLifecycle::new();
        // Consume key 0x41 but ask about 0x42
        lc.on_key_down_consumed(&make_event(VkCode(0x41), KeyEventType::KeyDown));
        assert_eq!(lc.take_key_up_duty(VkCode(0x42)), UpDuty::None);
        // 0x41 still active
        assert_eq!(lc.active_count(), 1);
    }

    #[test]
    fn multiple_keys_consumed_then_flushed() {
        let mut lc = KeyLifecycle::new();
        lc.on_key_down_consumed(&make_event(VkCode(0x10), KeyEventType::KeyDown)); // Shift
        lc.on_key_down_consumed(&make_event(VkCode(0x41), KeyEventType::KeyDown)); // A
        lc.on_key_down_consumed(&make_event(VkCode(0x42), KeyEventType::KeyDown)); // B
        assert_eq!(lc.active_count(), 3);

        let flushed = lc.flush_pending_key_ups();
        assert_eq!(flushed.len(), 3);
        assert_eq!(lc.active_count(), 0);
        // All flushed events should be KeyUp
        assert!(flushed.iter().all(|e| e.event_type == KeyEventType::KeyUp));
    }

    #[test]
    fn consume_keyup_consume_same_key_again() {
        let mut lc = KeyLifecycle::new();
        let event = make_event(VkCode(0x41), KeyEventType::KeyDown);

        // First cycle: consume then key_up
        lc.on_key_down_consumed(&event);
        assert_eq!(lc.active_count(), 1);
        assert_eq!(lc.take_key_up_duty(VkCode(0x41)), UpDuty::Consume);
        assert_eq!(lc.active_count(), 0);

        // Second cycle: consume same key again
        lc.on_key_down_consumed(&event);
        assert_eq!(lc.active_count(), 1);
        assert_eq!(lc.take_key_up_duty(VkCode(0x41)), UpDuty::Consume);
        assert_eq!(lc.active_count(), 0);
    }
}
