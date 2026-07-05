//! Force guard と drift monitor (Step 6)
//!
//! 旧 `ImeRecoveryState` を 2 つの責務に分解する：
//!
//! - `ForceGuardSet`: 発火後の guard 集合 (`effective_open()` を override する)
//! - `ObserveMissMonitor`: 発火前の観測失敗カウンタ（Observer が `None` を返した連続回数）
//!
//! ## 関係性
//!
//! ```text
//! ObserveMissMonitor → 閾値到達 → ForceGuardSet に ForceGuard を追加
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
    /// AppImePolicy が常時 force-on を要求
    ProfilePolicy,
}

impl ForceOnReason {
    /// この guard がユーザーの明示的な意図（`UserImeSetIntent`/`UserImeToggleIntent`
    /// 由来、SyncKey/PhysicalImeKey/Command）よりも優先されるべきか。
    ///
    /// `true`: 明示的意図があっても force-on する（安全弁として意図的にユーザー操作を
    /// 一時的に上書きする）。`PanicReset`（クラッシュ直後の安全弁）・`ProfilePolicy`
    /// （アプリ側の制約による恒久的な要求）が該当する。
    ///
    /// `false`: 「観測できない/信頼できない」ことのヒューリスティックな推測にすぎず、
    /// ユーザーの本物の意図を上書きしてはならない。`BrokenAppBootstrap` は
    /// observation-miss カウンタというヒューリスティックで立つため、ユーザーが
    /// 明示的に IME を OFF にした場合はそちらを優先する（`ObservationConfidence` の
    /// Low を `desired_open`/明示意図より優先させない、という belief 全体のルールと同じ）。
    #[must_use]
    pub const fn overrides_explicit_intent(self) -> bool {
        matches!(self, Self::PanicReset | Self::ProfilePolicy)
    }
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

    /// フォーカス変更時にすべての guard を解除する。
    ///
    /// force_guard は旧フォーカスアプリの文脈で発火したものであり、
    /// 新しいアプリには引き継ぐべきでない。ProfilePolicy 由来のものも
    /// FocusChanged で app_policy が更新されるため再評価が必要。
    pub fn clear_for_focus_change(&mut self) {
        self.guards.clear();
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
    /// `has_explicit_intent=true`（ユーザーが `UserImeSetIntent`/`UserImeToggleIntent`
    /// で明示的に意図を示している）場合、`ForceOnReason::overrides_explicit_intent()`
    /// が `false` の guard（`BrokenAppBootstrap` 等のヒューリスティック由来）は無視する。
    /// 観測できないことの推測が、ユーザーの本物の意図を上書きしてはならないため。
    /// `PanicReset` 等の安全弁は明示的意図があっても引き続き override する。
    #[must_use]
    pub fn effective_open(&self, desired_open: bool, has_explicit_intent: bool) -> bool {
        let forces_on = self
            .guards
            .iter()
            .any(|g| !has_explicit_intent || g.reason.overrides_explicit_intent());
        if forces_on {
            true
        } else {
            desired_open
        }
    }
}

/// Drift detection 用の連続観測失敗カウンタ。
///
/// 旧 `ImeRecoveryState::ime_detect_miss_count` の責務分離版。
/// 閾値到達で `Runtime::try_force_on_bootstrap()` が `BrokenAppBootstrap` guard を追加する。
#[derive(Debug, Default, Clone)]
pub struct ObserveMissMonitor {
    pub consecutive_miss_count: u32,
    pub first_miss_at: Option<Instant>,
    pub last_miss_at: Option<Instant>,
}

impl ObserveMissMonitor {
    /// 観測失敗を 1 件計上する。
    pub const fn record_miss(&mut self, now: Instant) {
        if self.consecutive_miss_count == 0 {
            self.first_miss_at = Some(now);
        }
        self.last_miss_at = Some(now);
        self.consecutive_miss_count = self.consecutive_miss_count.saturating_add(1);
    }

    /// 観測成功で counter を reset する。
    pub const fn record_success(&mut self) {
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
        assert!(
            !set.effective_open(false, false),
            "guard なし → desired そのまま"
        );
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        assert!(
            set.effective_open(false, false),
            "guard で true に override (明示的意図なし)"
        );
    }

    #[test]
    fn panic_reset_guard_overrides_even_explicit_intent() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        assert!(
            set.effective_open(false, true),
            "PanicReset は安全弁のため明示的意図があっても override する"
        );
    }

    #[test]
    fn broken_app_bootstrap_guard_does_not_override_explicit_intent() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        assert!(
            set.effective_open(false, false),
            "明示的意図が無ければ BrokenAppBootstrap も override する"
        );
        assert!(
            !set.effective_open(false, true),
            "BrokenAppBootstrap はヒューリスティックにすぎないため、ユーザーの明示的な \
             OFF 意図を上書きしてはならない"
        );
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
    fn observe_miss_monitor_counts_misses() {
        let mut d = ObserveMissMonitor::default();
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
