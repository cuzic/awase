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
    /// panic_reset 直後の stale poll 上書き防止
    PanicReset,
}

/// 単一の force-on ガード。
///
/// 期限（旧 `expires_at`・`purge_expired`）は本番で一度も設定・参照されなかったため撤去した（MC-E2）。
/// ガードは `remove`・`clear`・`clear_for_focus_change` でだけ外れる。
#[derive(Debug, Clone, Copy)]
pub struct ForceGuard {
    pub reason: ForceOnReason,
    pub generation: u64,
}

/// 発火中の force-on ガード集合。
///
/// 同時に複数立つ可能性を考えて `Vec<ForceGuard>` で保持する
/// (旧モデルは 2 つの bool フィールドで OR 評価していた)。
#[derive(Debug, Default, Clone)]
pub struct ForceGuardSet {
    guards: Vec<ForceGuard>,
}

impl ForceGuardSet {
    /// すべての guard を解除する。
    ///
    /// `guards` フィールドは非 `pub`（過去に `platform_state.rs` から
    /// `.guards.clear()` で直接フィールドを触る迂回が実在した。フィールドを
    /// private 化し、この唯一の公開クリア口を経由させる）。
    pub fn clear(&mut self) {
        self.guards.clear();
    }

    /// フォーカス変更時にすべての guard を解除する。
    ///
    /// force_guard は旧フォーカスアプリの文脈で発火したものであり、
    /// 新しいアプリには引き継ぐべきでない。
    pub fn clear_for_focus_change(&mut self) {
        self.clear();
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
    /// guard は全て安全弁（`PanicReset`。旧 `ProfilePolicy` は構築元が無く MC-E2 で撤去）で、ユーザーの明示意図があっても
    /// 常に override する。かつては「観測できない」ことの推測で立つヒューリスティック guard
    /// （`BrokenAppBootstrap`）だけ明示意図に譲る区別があったが、追加元が `621bf93c` で撤去され、
    /// 区別ごと削除した。
    #[must_use]
    pub fn effective_open(&self, desired_open: bool) -> bool {
        self.resolve(desired_open).0
    }

    /// `effective_open()` と同じ判定を行い、**実際に override が起きた場合のみ**
    /// その reason も返す（`(value, Some(reason))`）。override が起きなければ
    /// `(desired_open, None)`。
    ///
    /// `effective_open()` はこの `.0` を返す薄いラッパー。診断 API
    /// （`ImeModel::resolve_open_at`、ADR-087 §5 Phase 0a）が「guard が存在した」
    /// ことと「guard が実際に値を変えた」ことを混同しないよう、判定ロジックを
    /// ここに一本化する（ADR-087 §7 round4 M-C: 手書きの複製が
    /// `platform_state.rs:1300-1304` と同型の乖離バグを生む前例があるため）。
    #[must_use]
    pub fn resolve(&self, desired_open: bool) -> (bool, Option<ForceOnReason>) {
        match self.guards.first().map(|g| g.reason) {
            Some(reason) if !desired_open => (true, Some(reason)),
            Some(_) => (true, None), // 既に true だったので override は無かった
            None => (desired_open, None),
        }
    }

    /// active な guard があれば、その reason を返す（最初に追加されたもの）。
    /// ADR-087 §2.3 P15 Step 0（真の安全弁）の判定に使う。
    ///
    /// `guards` フィールドは private のため（過去の直接フィールド操作の迂回を
    /// 塞ぐための設計、本ファイル冒頭コメント参照）、`iter()` を公開する代わりに
    /// 目的別のアクセサとして追加する。
    #[must_use]
    pub fn active_reason(&self) -> Option<ForceOnReason> {
        self.guards.first().map(|g| g.reason)
    }
}

/// Drift detection 用の連続観測失敗カウンタ。
///
/// 旧 `ImeRecoveryState::ime_detect_miss_count` の責務分離版。
/// 閾値到達で `Runtime::try_force_on_bootstrap()` が guard（旧 `BrokenAppBootstrap`）を追加していたが、
/// `621bf93c` で撤去済み（現在は連続失敗の記録のみ）。
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

    /// `record_miss` の `consecutive_miss_count == 0` ガードが反転すると、
    /// 初回 miss で `first_miss_at` が記録されず、drift 窓の起点がずれる。
    #[test]
    fn record_miss_sets_first_miss_at_only_on_first_call() {
        let mut m = ObserveMissMonitor::default();
        let t0 = Instant::now();
        m.record_miss(t0);
        assert_eq!(m.first_miss_at, Some(t0));

        let t1 = t0 + Duration::from_millis(100);
        m.record_miss(t1);
        assert_eq!(
            m.first_miss_at,
            Some(t0),
            "2回目以降は first_miss_at を更新しない"
        );
        assert_eq!(m.last_miss_at, Some(t1));
        assert_eq!(m.consecutive_miss_count, 2);
    }

    #[test]
    fn guard_set_add_and_remove() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
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
            reason: ForceOnReason::PanicReset,
            generation: 1,
        });
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
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
            generation: 1,
        });
        assert!(
            set.effective_open(false),
            "guard で true に override (明示的意図なし)"
        );
    }

    #[test]
    fn panic_reset_guard_overrides_even_explicit_intent() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            generation: 1,
        });
        assert!(
            set.effective_open(false),
            "PanicReset は安全弁のため明示的意図があっても override する"
        );
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

    // ── active_reason（ADR-087 §2.3 P15） ──

    #[test]
    fn active_reason_finds_panic_reset() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            generation: 1,
        });
        assert_eq!(
            set.active_reason(),
            Some(ForceOnReason::PanicReset),
            "override 権限を持つ PanicReset が見つかるべき"
        );
    }

    #[test]
    fn active_reason_none_when_empty() {
        let set = ForceGuardSet::default();
        assert_eq!(set.active_reason(), None);
    }

    // ── resolve()（ADR-087 §7 round4 M-C） ──

    #[test]
    fn resolve_reports_no_override_when_base_already_true() {
        // guard は active だが base が既に true なので、override は起きていない。
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            generation: 1,
        });
        assert_eq!(
            set.resolve(true),
            (true, None),
            "base が既に true なら guard は何も変えていないので reason は None"
        );
    }

    #[test]
    fn resolve_reports_override_reason_when_it_actually_flips_the_value() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            generation: 1,
        });
        assert_eq!(
            set.resolve(false),
            (true, Some(ForceOnReason::PanicReset)),
            "base=false を override して true にしたので reason が返る"
        );
    }

    #[test]
    fn resolve_no_guard_no_override() {
        let set = ForceGuardSet::default();
        assert_eq!(set.resolve(false), (false, None));
        assert_eq!(set.resolve(true), (true, None));
    }

    #[test]
    fn resolve_matches_effective_open_value() {
        // resolve().0 == effective_open() が常に成り立つことの pinned test。
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            generation: 1,
        });
        for desired in [true, false] {
            assert_eq!(set.resolve(desired).0, set.effective_open(desired));
        }
    }
}
