//! 他アプリが注入した Ctrl を、注入キー自身の修飾として期限付きで数える判断の核
//! ([ADR-249](../../../../docs/adr/249-foreign-injected-modifier-ttl.md)、BUG-197)。
//!
//! Spokenly 等の音声入力ソフトは `Ctrl↓ → V↓↑ → (約100ms 後) Ctrl↑` を `SendInput` で注入して貼り付ける。
//! awase は注入 Ctrl を物理押下状態に数えない(ADR-054、X サーバーの Ctrl KeyUp 欠落による stuck 対策)ため、
//! V が `ctrl=false` の Char として NICOLA 変換されて「ふ」になっていた。
//!
//! ここは **注入された打鍵の `modifier_snapshot` にだけ** `ctrl` を足すための記録(`ForeignCtrlLatch`)と判定。
//! 物理打鍵・`read_os_modifiers`・`HeldModifiers`・`ctrl_consumed_since_down` は読まない(ADR-249 決定5)。
//! `std` の atomic だけを使い Win32 を呼ばないので、Linux の単体テストで境界を固定できる。

use std::sync::atomic::{AtomicU64, Ordering};

use awase::types::VkCode;

/// Ctrl のスロット(左 = 0、右 = 1)。generic `VK_CONTROL`(0x11)は左スロットに数える
/// (OS は VK ごと1ビットで、0x11 は左右どちらの Up でも落ちうる)。Ctrl 以外は `None`。
#[must_use]
pub const fn ctrl_slot(vk: VkCode) -> Option<usize> {
    match vk.0 {
        0x11 | 0xA2 => Some(0),
        0xA3 => Some(1),
        _ => None,
    }
}

/// 時刻 `ts_us` に、`down_at_us` に記録した注入 Ctrl が期限(`ttl_us`)内か。
/// `down_at_us == 0` は記録なし。`down_at_us > ts_us` でもアンダーフローしない。
#[must_use]
pub const fn foreign_ctrl_active(ts_us: u64, down_at_us: u64, ttl_us: u64) -> bool {
    down_at_us != 0 && ts_us.saturating_sub(down_at_us) < ttl_us
}

/// 注入 Ctrl↓ の時刻(µs、0 = 記録なし)を左右別に持つ。書くのはフックスレッド(記録・注入 Up・物理 Up)、
/// メインスレッド(reset・Leave・watchdog の解除)、ゾンビの旧フックの3か所なので atomic。
/// CAS・Mutex は不要(解除と競合しても記録が消える安全側に倒れるだけ)。読むのはフックスレッドだけ。
#[derive(Debug)]
pub struct ForeignCtrlLatch {
    down_at_us: [AtomicU64; 2],
}

impl ForeignCtrlLatch {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            down_at_us: [AtomicU64::new(0), AtomicU64::new(0)],
        }
    }

    /// 他アプリの注入 Ctrl KeyDown。**まだ記録がなければ** `ts_us` を書く
    /// (オートリピート・押し直しで期限を延ばさない)。`ts_us == 0` は「記録なし」と区別できないので書かない。
    pub fn on_injected_down(&self, vk: VkCode, ts_us: u64) {
        if let Some(slot) = ctrl_slot(vk).and_then(|i| self.down_at_us.get(i)) {
            if ts_us != 0 && slot.load(Ordering::Relaxed) == 0 {
                slot.store(ts_us, Ordering::Relaxed);
            }
        }
    }

    /// 注入 Ctrl KeyUp、または同じスロットの物理 Ctrl KeyUp(OS の VK ごと1ビットと一致させる)。
    pub fn on_up(&self, vk: VkCode) {
        if let Some(slot) = ctrl_slot(vk).and_then(|i| self.down_at_us.get(i)) {
            slot.store(0, Ordering::Relaxed);
        }
    }

    /// 全解除(`reset_physical_key_state`・app-disable の Leave・watchdog reinstall)。
    pub fn clear(&self) {
        for slot in &self.down_at_us {
            slot.store(0, Ordering::Relaxed);
        }
    }

    /// 注入された打鍵の `modifier_snapshot.ctrl` に足すか(左右の OR)。
    #[must_use]
    pub fn ctrl_for_injected_key(&self, ts_us: u64, ttl_us: u64) -> bool {
        self.down_at_us
            .iter()
            .any(|s| foreign_ctrl_active(ts_us, s.load(Ordering::Relaxed), ttl_us))
    }
}

impl Default for ForeignCtrlLatch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: u64 = 1_000_000;
    const L: VkCode = VkCode(0xA2);
    const R: VkCode = VkCode(0xA3);
    const GENERIC: VkCode = VkCode(0x11);

    #[test]
    fn slot_mapping() {
        assert_eq!(ctrl_slot(GENERIC), Some(0));
        assert_eq!(ctrl_slot(L), Some(0));
        assert_eq!(ctrl_slot(R), Some(1));
        assert_eq!(ctrl_slot(VkCode(0x56)), None);
        assert_eq!(ctrl_slot(VkCode(0xA0)), None, "Shift は対象外");
        assert_eq!(ctrl_slot(VkCode(0xA4)), None, "Alt は対象外");
    }

    #[test]
    fn active_boundaries() {
        assert!(!foreign_ctrl_active(500, 0, TTL), "記録なし");
        assert!(foreign_ctrl_active(1_000, 1_000, TTL), "同時刻");
        assert!(
            foreign_ctrl_active(1_000 + TTL - 1, 1_000, TTL),
            "期限の1µs手前"
        );
        assert!(
            !foreign_ctrl_active(1_000 + TTL, 1_000, TTL),
            "期限ちょうどは失効"
        );
        assert!(
            foreign_ctrl_active(500, 1_000, TTL),
            "down_at > ts でもアンダーフローしない"
        );
    }

    #[test]
    fn injected_key_gets_ctrl_within_ttl_only() {
        let l = ForeignCtrlLatch::new();
        assert!(!l.ctrl_for_injected_key(100, TTL));
        l.on_injected_down(L, 100);
        assert!(l.ctrl_for_injected_key(101, TTL));
        assert!(!l.ctrl_for_injected_key(100 + TTL, TTL));
    }

    #[test]
    fn first_down_is_kept_so_autorepeat_does_not_extend() {
        let l = ForeignCtrlLatch::new();
        l.on_injected_down(L, 100);
        l.on_injected_down(L, 900_000);
        assert!(!l.ctrl_for_injected_key(100 + TTL, TTL));
    }

    #[test]
    fn zero_timestamp_is_never_recorded() {
        let l = ForeignCtrlLatch::new();
        l.on_injected_down(L, 0);
        assert!(!l.ctrl_for_injected_key(1, TTL));
    }

    #[test]
    fn up_releases_only_the_same_slot() {
        let l = ForeignCtrlLatch::new();
        l.on_injected_down(L, 100);
        l.on_injected_down(R, 100);
        l.on_up(R);
        assert!(l.ctrl_for_injected_key(200, TTL), "左はまだ有効");
        l.on_up(L);
        assert!(!l.ctrl_for_injected_key(200, TTL));
    }

    #[test]
    fn generic_ctrl_shares_left_slot() {
        let l = ForeignCtrlLatch::new();
        l.on_injected_down(GENERIC, 100);
        assert!(l.ctrl_for_injected_key(200, TTL));
        l.on_up(L);
        assert!(!l.ctrl_for_injected_key(200, TTL));
    }

    #[test]
    fn non_ctrl_keys_do_not_touch_latch() {
        let l = ForeignCtrlLatch::new();
        l.on_injected_down(VkCode(0x56), 100);
        l.on_injected_down(L, 100);
        l.on_up(VkCode(0x56));
        assert!(l.ctrl_for_injected_key(200, TTL));
    }

    #[test]
    fn clear_drops_both_slots() {
        let l = ForeignCtrlLatch::new();
        l.on_injected_down(L, 100);
        l.on_injected_down(R, 100);
        l.clear();
        assert!(!l.ctrl_for_injected_key(200, TTL));
    }

    #[test]
    fn either_slot_suffices() {
        let l = ForeignCtrlLatch::new();
        l.on_injected_down(R, 100);
        assert!(l.ctrl_for_injected_key(200, TTL));
    }
}
