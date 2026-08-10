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
    guards: Vec<ForceGuard>,
}

impl ForceGuardSet {
    /// 期限切れの guard を除去する。
    pub fn purge_expired(&mut self, now: Instant) {
        self.guards.retain(|g| !g.is_expired(now));
    }

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
    /// 新しいアプリには引き継ぐべきでない。ProfilePolicy 由来のものも
    /// FocusChanged で app_policy が更新されるため再評価が必要。
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
    /// `has_explicit_intent=true`（ユーザーが `UserImeSetIntent`/`UserImeToggleIntent`
    /// で明示的に意図を示している）場合、`ForceOnReason::overrides_explicit_intent()`
    /// が `false` の guard（`BrokenAppBootstrap` 等のヒューリスティック由来）は無視する。
    /// 観測できないことの推測が、ユーザーの本物の意図を上書きしてはならないため。
    /// `PanicReset` 等の安全弁は明示的意図があっても引き続き override する。
    #[must_use]
    pub fn effective_open(&self, desired_open: bool, has_explicit_intent: bool) -> bool {
        self.resolve(desired_open, has_explicit_intent).0
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
    pub fn resolve(
        &self,
        desired_open: bool,
        has_explicit_intent: bool,
    ) -> (bool, Option<ForceOnReason>) {
        // override 権限を持つ reason を優先して報告する（Opus round4 最終確認の
        // 補足指摘: `has_explicit_intent==false` のとき素の `.find()` は挿入順で
        // 最初の guard を返すため、`PanicReset` と `BrokenAppBootstrap` が同時に
        // 立っていると弱い方を報告しうる。`.0` の値は変わらないが、診断としては
        // 権限の強い方を報告する方が自然）。
        let override_reason = self
            .guards
            .iter()
            .map(|g| g.reason)
            .find(|r| r.overrides_explicit_intent());
        let forcing = override_reason.or_else(|| {
            if has_explicit_intent {
                None
            } else {
                self.guards.iter().map(|g| g.reason).next()
            }
        });
        match forcing {
            Some(reason) if !desired_open => (true, Some(reason)),
            Some(_) => (true, None), // 既に true だったので override は無かった
            None => (desired_open, None),
        }
    }

    /// 明示意図があっても override してよい（`overrides_explicit_intent()==true`）
    /// guard が active なら、その reason を返す。ADR-087 §2.3 P15 Step 0
    /// （真の安全弁）の判定に使う。
    ///
    /// `guards` フィールドは private のため（過去の直接フィールド操作の迂回を
    /// 塞ぐための設計、本ファイル冒頭コメント参照）、`iter()` を公開する代わりに
    /// 目的別のアクセサとして追加する。
    ///
    /// **`expires_at`（`ForceGuard::is_expired`）を見ない**——`effective_open()`
    /// と同じ意味論（`purge_expired()` は production から一度も呼ばれておらず、
    /// `expires_at` は事実上機能していない）。期限を見るように変える場合は
    /// `effective_open()`/`resolve()` も同時に変えること（ADR-087 §7 round4 S-D）。
    #[must_use]
    pub fn active_override_reason(&self) -> Option<ForceOnReason> {
        self.guards
            .iter()
            .map(|g| g.reason)
            .find(|r| r.overrides_explicit_intent())
    }

    /// override 権限を持たない（`overrides_explicit_intent()==false`）
    /// ヒューリスティック guard（`BrokenAppBootstrap` 等）が active なら、
    /// その reason を返す。ADR-087 §2.3 P15 Step 4b の判定に使う。
    ///
    /// `active_override_reason()` と同じ理由で `expires_at` を見ない
    /// （§7 round4 S-D）。
    #[must_use]
    pub fn active_heuristic_reason(&self) -> Option<ForceOnReason> {
        self.guards
            .iter()
            .map(|g| g.reason)
            .find(|r| !r.overrides_explicit_intent())
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

    /// `expires_at: None`（無期限ガード）は期限切れ扱いにしてはならない。
    /// `is_expired -> true` に壊れると PanicReset/BrokenAppBootstrap 等の
    /// 無期限ガードが即座に無効化され、安全弁として機能しなくなる。
    #[test]
    fn is_expired_false_when_no_expiry_set() {
        let guard = ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        };
        assert!(!guard.is_expired(Instant::now()));
    }

    #[test]
    fn is_expired_true_after_expiry_time() {
        let now = Instant::now();
        let guard = ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: Some(now),
            generation: 1,
        };
        assert!(guard.is_expired(now + Duration::from_millis(1)));
        assert!(!guard.is_expired(now - Duration::from_millis(1)));
    }

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

    // ── active_override_reason / active_heuristic_reason（ADR-087 §2.3 P15） ──

    #[test]
    fn active_override_reason_finds_panic_reset() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        assert_eq!(
            set.active_override_reason(),
            Some(ForceOnReason::PanicReset),
            "override 権限を持つ PanicReset が見つかるべき"
        );
    }

    #[test]
    fn active_override_reason_none_when_only_heuristic_guards() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        assert_eq!(
            set.active_override_reason(),
            None,
            "BrokenAppBootstrap は override 権限を持たないため None"
        );
        assert_eq!(
            set.active_heuristic_reason(),
            Some(ForceOnReason::BrokenAppBootstrap)
        );
    }

    #[test]
    fn active_heuristic_reason_none_when_only_override_guards() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::ProfilePolicy,
            expires_at: None,
            generation: 1,
        });
        assert_eq!(set.active_heuristic_reason(), None);
        assert_eq!(
            set.active_override_reason(),
            Some(ForceOnReason::ProfilePolicy)
        );
    }

    #[test]
    fn active_reasons_both_none_when_empty() {
        let set = ForceGuardSet::default();
        assert_eq!(set.active_override_reason(), None);
        assert_eq!(set.active_heuristic_reason(), None);
    }

    // ── resolve()（ADR-087 §7 round4 M-C） ──

    #[test]
    fn resolve_reports_no_override_when_base_already_true() {
        // guard は active だが base が既に true なので、override は起きていない。
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        assert_eq!(
            set.resolve(true, false),
            (true, None),
            "base が既に true なら guard は何も変えていないので reason は None"
        );
    }

    #[test]
    fn resolve_reports_override_reason_when_it_actually_flips_the_value() {
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        assert_eq!(
            set.resolve(false, true),
            (true, Some(ForceOnReason::PanicReset)),
            "base=false を override して true にしたので reason が返る"
        );
    }

    #[test]
    fn resolve_no_guard_no_override() {
        let set = ForceGuardSet::default();
        assert_eq!(set.resolve(false, false), (false, None));
        assert_eq!(set.resolve(true, false), (true, None));
    }

    #[test]
    fn resolve_matches_effective_open_value() {
        // resolve().0 == effective_open() が常に成り立つことの pinned test。
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        for desired in [true, false] {
            for has_intent in [true, false] {
                assert_eq!(
                    set.resolve(desired, has_intent).0,
                    set.effective_open(desired, has_intent)
                );
            }
        }
    }

    #[test]
    fn resolve_prefers_override_reason_over_heuristic_when_both_active() {
        // 両方の guard が同時に active なとき、resolve() は override 権限を
        // 持つ reason を優先して報告する（Opus round4 最終確認の補足指摘）。
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        set.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 2,
        });
        assert_eq!(
            set.resolve(false, false),
            (true, Some(ForceOnReason::PanicReset)),
            "挿入順は BrokenAppBootstrap が先だが、override 権限を持つ \
             PanicReset を優先して報告する"
        );
    }

    #[test]
    fn resolve_profile_policy_also_overrides_explicit_intent() {
        // overrides_explicit_intent() のもう一方の true variant（ProfilePolicy）も
        // PanicReset と同じ経路を通ることを確認する（enum を閉じる）。
        let mut set = ForceGuardSet::default();
        set.add(ForceGuard {
            reason: ForceOnReason::ProfilePolicy,
            expires_at: None,
            generation: 1,
        });
        assert_eq!(
            set.resolve(false, true),
            (true, Some(ForceOnReason::ProfilePolicy))
        );
    }
}
