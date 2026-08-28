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

use super::ApplyGeneration;
use crate::state::probe_admission::FocusEpoch;
use std::time::Instant;

/// OS への apply transaction。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeTransition {
    /// 適用したい IME 開閉状態
    pub target: bool,
    /// 世代 ID (apply 要求ごとに increment、stale 照合に使う)
    pub generation: ApplyGeneration,
    /// この apply が送られたフォーカスプロセス epoch。
    ///
    /// `ObservationStore::current_fence().epoch` と同じ値を使う。`FocusStore` 側の
    /// epoch とは bootstrap 直後にずれることがあるため混ぜないこと。
    ///
    /// actuation の `applied` 汚染防止では epoch 単独で十分。`applied = Unknown`
    /// へのリセットは `ImeEvent::FocusChanged` アームだけで起き、同じアームが
    /// `current_fence().epoch` を張り替えるため、完了側はその epoch と一致するかだけを
    /// 見ればフォーカスプロセス跨ぎを弾ける。`FocusHwndUpdated` のような hwnd だけの
    /// 変更は `applied` に触れないので、ここで守るべき不変条件を新しく作らない。
    pub focus_epoch: FocusEpoch,
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
            generation: ApplyGeneration::new(10).unwrap(),
            focus_epoch: 0,
            timeout_at: t0 + Duration::from_millis(100),
        };
        assert!(!trans.is_timed_out(t0));
        assert!(trans.is_timed_out(t0 + Duration::from_millis(200)));
    }
}
