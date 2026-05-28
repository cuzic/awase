//! Pending IME transition (Step 7)
//!
//! 旧 `ImeEffect::SetOpen` (Layer 3) + `last_applied_ime_on` (Layer 4) を
//! 単一の `pending` + `applied_open` に統合する。
//!
//! ## 必須: generation 照合
//!
//! async apply 完了時は **必ず** generation を照合する。これを忘れると
//! 「古い async apply の完了が新しい状態を壊す」事故が起きる。
//!
//! ```text
//! T1: apply true requested generation=10
//! T2: user intent false generation=11
//! T3: apply true succeeded generation=10 ← stale
//! → desired_open は false のまま (T2 が勝つ)
//! → applied_open は None (T3 は無視)
//! ```

use std::time::Instant;

use super::app_ime_policy::ImeActuatorKind;

/// OS への apply transaction。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeTransition {
    /// 適用したい IME 開閉状態
    pub target: bool,
    /// 世代 ID (apply 要求ごとに increment、stale 照合に使う)
    pub generation: u64,
    /// 要求時刻 (タイムアウト判定用)
    pub requested_at: Instant,
    /// 使用 actuator
    pub actuator: ImeActuatorKind,
    /// 楽観的に latch 済みか (ImmCross async でよく使う)
    pub optimistic_applied: bool,
    /// この transition のタイムアウト時刻
    pub timeout_at: Instant,
}

impl ImeTransition {
    /// タイムアウト済みか
    #[must_use]
    pub fn is_timed_out(&self, now: Instant) -> bool {
        now >= self.timeout_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timeout_check() {
        let t0 = Instant::now();
        let trans = ImeTransition {
            target: true,
            generation: 10,
            requested_at: t0,
            actuator: ImeActuatorKind::ImmCross,
            optimistic_applied: true,
            timeout_at: t0 + Duration::from_millis(100),
        };
        assert!(!trans.is_timed_out(t0));
        assert!(trans.is_timed_out(t0 + Duration::from_millis(200)));
    }
}
