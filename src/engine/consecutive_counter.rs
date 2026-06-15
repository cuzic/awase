//! 汎用連続ソロ確定カウンター

use crate::types::{Timestamp, VkCode};

/// 同一 VK のソロ確定が連続した回数を追跡する汎用カウンター。
///
/// - 前回と異なる VK が記録されたとき → count を 1 にリセット
/// - 前回の記録から `timeout_us` を超過したとき → count を 1 にリセット
/// - 同一 VK かつタイムアウト以内 → count をインクリメント
///
/// 呼び出し側は `record()` の戻り値（現在の連続回数）を閾値と比較して
/// アクションを起こす。`VkCode(0)` は「未記録」の番兵値として予約されており、
/// キーとして渡してはならない。
#[derive(Debug, Clone)]
pub struct ConsecutiveSoloCounter {
    count: u32,
    last_vk: VkCode,
    last_us: Timestamp,
    timeout_us: u64,
}

impl ConsecutiveSoloCounter {
    #[must_use]
    pub const fn new(timeout_us: u64) -> Self {
        Self {
            count: 0,
            last_vk: VkCode(0),
            last_us: 0,
            timeout_us,
        }
    }

    /// ソロ確定を記録し、現在の連続回数を返す。
    ///
    /// `timestamp` は確定したキーの KeyDown 時刻（マイクロ秒）。
    pub fn record(&mut self, vk: VkCode, timestamp: Timestamp) -> u32 {
        let gap = timestamp.saturating_sub(self.last_us);
        if vk != self.last_vk || gap > self.timeout_us {
            self.count = 1;
        } else {
            self.count += 1;
        }
        self.last_vk = vk;
        self.last_us = timestamp;
        self.count
    }

    pub const fn reset(&mut self) {
        self.count = 0;
        self.last_vk = VkCode(0);
        self.last_us = 0;
    }

    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK_A: VkCode = VkCode(0x41);
    const VK_B: VkCode = VkCode(0x42);
    const TIMEOUT: u64 = 400_000; // 400ms

    #[test]
    fn first_record_returns_one() {
        let mut c = ConsecutiveSoloCounter::new(TIMEOUT);
        assert_eq!(c.record(VK_A, 1_000_000), 1);
    }

    #[test]
    fn same_vk_within_timeout_increments() {
        let mut c = ConsecutiveSoloCounter::new(TIMEOUT);
        c.record(VK_A, 0);
        c.record(VK_A, 200_000); // 200ms 後
        assert_eq!(c.record(VK_A, 400_000), 3); // 400ms 後（累計）
    }

    #[test]
    fn same_vk_at_exact_timeout_still_counts() {
        let mut c = ConsecutiveSoloCounter::new(TIMEOUT);
        c.record(VK_A, 0);
        assert_eq!(c.record(VK_A, TIMEOUT), 2); // ちょうど境界 → reset しない
    }

    #[test]
    fn gap_over_timeout_resets() {
        let mut c = ConsecutiveSoloCounter::new(TIMEOUT);
        c.record(VK_A, 0);
        c.record(VK_A, 200_000); // last_us = 200_000
                                 // 前回(200_000µs)から TIMEOUT 超過 → reset
        assert_eq!(c.record(VK_A, 200_000 + TIMEOUT + 1), 1);
    }

    #[test]
    fn different_vk_resets_to_one() {
        let mut c = ConsecutiveSoloCounter::new(TIMEOUT);
        c.record(VK_A, 0);
        c.record(VK_A, 100_000);
        assert_eq!(c.record(VK_B, 200_000), 1);
    }

    #[test]
    fn reset_clears_state() {
        let mut c = ConsecutiveSoloCounter::new(TIMEOUT);
        c.record(VK_A, 0);
        c.record(VK_A, 100_000);
        c.reset();
        assert_eq!(c.count(), 0);
        // reset 後の record は 1 から始まる
        assert_eq!(c.record(VK_A, 150_000), 1);
    }
}
