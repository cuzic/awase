//! Force guard と drift monitor (Step 6)
//!
//! 旧 `ImeRecoveryState` を 2 つの責務に分解する：
//!
//! - `ForceGuardSet`: 発火後の guard 集合 (`effective_open()` を override する)
//! - `DriftMonitor`: 発火前の観測カウンタ
//!
//! ## 関係性
//!
//! ```text
//! DriftMonitor → 閾値到達 → ForceGuardSet に ForceGuard を追加
//! ```
//!
//! ## 重要な原則
//!
//! `ForceGuard` は `desired_open` を直接書き換えない。
//! `effective_open()` で一時的に override する形にする。

use std::time::Instant;

/// force-on ガードが立った理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceOnReason {
    /// Imm32Unavailable アプリへの初回フォーカス時の IME OFF 誤認防止
    BrokenAppBootstrap,
    /// panic_reset 直後の stale poll 上書き防止
    PanicReset,
    /// 連続検出失敗カウンタが閾値に到達
    DetectMissThreshold,
    /// AppImePolicy が常時 force-on を要求
    ProfilePolicy,
}

/// 単一の force-on ガード。
#[derive(Debug, Clone, Copy)]
pub struct ForceGuard {
    pub reason: ForceOnReason,
    pub expires_at: Option<Instant>,
    pub generation: u64,
}

impl ForceGuard {
    /// 期限切れか
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|exp| now >= exp)
    }
}

/// 発火中の force-on ガード集合。
///
/// 同時に複数立つ可能性を考えて `Vec<ForceGuard>` で保持する
/// (旧モデルは 2 つの bool フィールドで OR 評価していた)。
#[derive(Debug, Default, Clone)]
pub struct ForceGuardSet {
    pub guards: Vec<ForceGuard>,
}

impl ForceGuardSet {
    /// 期限切れの guard を除去する。
    pub fn purge_expired(&mut self, now: Instant) {
        self.guards.retain(|g| !g.is_expired(now));
    }

    /// 指定 reason の guard を追加する (既存があれば置換)。
    pub fn add(&mut self, guard: ForceGuard) {
        self.guards.retain(|g| g.reason != guard.reason);
        self.guards.push(guard);
    }

    /// 指定 reason の guard を削除する。
    pub fn remove(&mut self, reason: ForceOnReason) {
        self.guards.retain(|g| g.reason != reason);
    }

    /// いずれかの guard が active か (force-on を要求しているか)。
    #[must_use]
    pub const fn requires_on(&self) -> bool {
        !self.guards.is_empty()
    }

    /// `desired_open` を guard で override した最終値を返す。
    ///
    /// guard が active なら true (= force-on)、そうでなければ desired をそのまま。
    /// ChatGPT 推奨の `effective_open()` パターン。
    #[must_use]
    pub const fn effective_open(&self, desired_open: bool) -> bool {
        if self.requires_on() { true } else { desired_open }
    }
}

/// Drift detection 用の連続観測失敗カウンタ。
///
/// 旧 `ImeRecoveryState::ime_detect_miss_count` の責務分離版。
/// 閾値到達で `ForceGuardSet` に `ForceOnReason::DetectMissThreshold` を追加する流れ。
#[derive(Debug, Default, Clone)]
pub struct DriftMonitor {
    pub consecutive_miss_count: u32,
    pub first_miss_at: Option<Instant>,
    pub last_miss_at: Option<Instant>,
}

impl DriftMonitor {
    /// 観測失敗を 1 件計上する。
    pub fn record_miss(&mut self, now: Instant) {
        if self.consecutive_miss_count == 0 {
            self.first_miss_at = Some(now);
        }
        self.last_miss_at = Some(now);
        self.consecutive_miss_count = self.consecutive_miss_count.saturating_add(1);
    }

    /// 観測成功で counter を reset する。
    pub fn record_success(&mut self) {
        self.consecutive_miss_count = 0;
        self.first_miss_at = None;
        self.last_miss_at = None;
    }

    /// 閾値に達しているか
    #[must_use]
    pub const fn exceeds(&self, threshold: u32) -> bool {
        self.consecutive_miss_count >= threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn guard_set_add_and_remove() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        assert!(set.requires_on());
        set.remove(ForceOnReason::PanicReset);
        assert!(!set.requires_on());
    }

    #[test]
    fn guard_set_replaces_same_reason() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        set.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 2,
        });
        assert_eq!(set.guards.len(), 1);
        assert_eq!(set.guards[0].generation, 2);
    }

    #[test]
    fn effective_open_overrides_when_guard_active() {
        let mut set = ForceGuardSet::default();
        assert!(!set.effective_open(false), "guard なし → desired そのまま");
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        assert!(set.effective_open(false), "guard で true に override");
    }

    #[test]
    fn purge_expired_removes_old_guards() {
        let mut set = ForceGuardSet::default();
        let t0 = Instant::now();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: Some(t0),
            generation: 1,
        });
        set.purge_expired(t0 + Duration::from_millis(1));
        assert!(set.guards.is_empty());
    }

    #[test]
    fn drift_monitor_counts_misses() {
        let mut d = DriftMonitor::default();
        let t0 = Instant::now();
        d.record_miss(t0);
        d.record_miss(t0);
        d.record_miss(t0);
        assert_eq!(d.consecutive_miss_count, 3);
        assert!(d.exceeds(3));
        assert!(!d.exceeds(4));
        d.record_success();
        assert_eq!(d.consecutive_miss_count, 0);
    }
}
