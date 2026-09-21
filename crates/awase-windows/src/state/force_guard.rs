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

/// 今回のOS読み取りが**新しい観測失敗を数えなかった**か（`consecutive_miss_count`が増えていない）。
///
/// 通過マークの追随（`ir_stage_observe`の`OsPoll`後、意図の破棄と60ms間隔の読み直し）を続けてよい条件。
/// 以前は`miss_after == miss_before`だったが、直前の読み取りが失敗（`ime_on=None`、カウント1）していて
/// 今回**成功**（カウントが0へリセット）すると等しくなくなり、追随が黙って止まっていた。すると
/// 最初の読み取りがfence（`KEY_EFFECT_SETTLE_MS`）内で無視された予測は、その後の打鍵中
/// （typing-idleガード）に訂正の機会を失い、約12秒Engineが固まる（実機cold、`removal-cold-2`）。
/// カウントが**減った**（成功で復帰した）ときも追随を続けるので`<=`とする。
pub(crate) const fn poll_counted_no_new_miss(miss_before: u32, miss_after: u32) -> bool {
    miss_after <= miss_before
}

/// 通過マーク（ADR-187）に対して、古い明示意図を捨ててよいか（`age_ms` = 通過からの経過）。
///
/// - `on_expiry == false`（観測が成功したとき）: 窓の間（`age_ms < window_ms`）だけ。
/// - `on_expiry == true`（窓の終了時、BUG-158）: 窓が切れて（`age_ms >= window_ms`）、まだ一度も観測で
///   捨てていない（`!invalidated`）ときだけ。窓の間・捨て済みは何もしない。
///
/// 判定を純関数にして、`#[cfg(windows)]`配下の`platform_state`のテストに頼らずLinuxで固定する。
#[must_use]
pub(crate) const fn should_drop_intents_for_mode_key_pass(
    age_ms: u64,
    invalidated: bool,
    on_expiry: bool,
    window_ms: u64,
) -> bool {
    let expired = age_ms >= window_ms;
    if on_expiry {
        expired && !invalidated
    } else {
        !expired
    }
}

/// 通過マークの窓が切れるまでの残り時間(ms)。窓が切れていれば`None`。
#[must_use]
pub(crate) const fn mode_key_pass_window_remaining_ms(age_ms: u64, window_ms: u64) -> Option<u64> {
    if age_ms >= window_ms {
        None
    } else {
        Some(window_ms - age_ms)
    }
}

/// 通過マークの窓の間、次のIME読み取りを何ms後に予約するか（BUG-158）。
///
/// - 直前の読み取りが**成功**した（連続失敗カウント0）: ADR-187どおり`reread_ms`ごとに読み直す
///   （最初の観測はGJI/IMEがキーを処理する前の古い状態のことがある）。
/// - 直前の読み取りが**失敗**した（`ime_on=None`等）: 読み直しを窓の終了時の1回に絞る（`remaining_ms + 1`）。
///   失敗する環境（MS-IME本体のIMMクロスプロセスprobeが50〜100ms）で60msごとに読み直すと、probeが重なって
///   連続失敗を積み上げ、`IME_DETECT_MISS_THRESHOLD`(3)で`imm-learning`が窓を`Imm32Unavailable`へ誤って降格する
///   （CI `ci/e2e-msime-native-e`）。窓の終了時の読み取りの後、`ir_stage_notify`が古い意図を捨てて通常の
///   ポーリングへ戻る。
#[must_use]
pub(crate) const fn mode_key_pass_next_read_ms(
    last_read_succeeded: bool,
    remaining_ms: u64,
    reread_ms: u64,
) -> u64 {
    if last_read_succeeded {
        reread_ms
    } else {
        remaining_ms.saturating_add(1)
    }
}

/// `SendMessageTimeoutW`が失敗（戻り値0）したとき、それが**時間切れ**か**即時の拒否**かを分類する純関数。
///
/// - `ERROR_TIMEOUT`(1460)、または宣言したタイムアウト以上かかっている: 時間切れ（遅い応答。負荷・忙しいIME）。
/// - それ以外（`ERROR_ACCESS_DENIED`=昇格プロセスへのUIPI拒否、即時の失敗）: 拒否（IMMが使えない証拠になりうる）。
///
/// 実測（CI、MS-IME本体、awase.logの`[ime-io]`）: 成功は 0〜20ms、時間切れは 50〜100ms（宣言50ms+スケジューリング）の
/// 二峰性で、`elapsed_us >= timeout_ms*1000`で時間切れと判別できる。
#[must_use]
pub(crate) const fn send_failure_is_timeout(
    last_error: u32,
    elapsed_us: u64,
    timeout_ms: u32,
) -> bool {
    const ERROR_TIMEOUT: u32 = 1460;
    last_error == ERROR_TIMEOUT || elapsed_us >= (timeout_ms as u64) * 1000
}

/// IME状態の読み取りの空振り（`ime_on`が`None`）を、`imm-learning`の「IMMが使えない」証拠（miss）に数えるか。
///
/// **時間切れ**（遅い応答。負荷・忙しいIME・CIの遅いランナー）は証拠ではない（判定保留=数えない）。
/// **即時の拒否**（`ERROR_ACCESS_DENIED`、IME窓なし=`ImmGetDefaultIMEWnd`=NULL、即時の失敗）は従来どおり数える。
/// `IME_DETECT_MISS_THRESHOLD`（連続3回で`Unavailable`を学習）の値は変えない（tuning-constants: 盲目的な引き上げをしない）。
#[must_use]
pub(crate) const fn read_miss_is_imm_evidence(probe_timed_out: bool) -> bool {
    !probe_timed_out
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
        assert!(!guard.is_expired(
            now.checked_sub(Duration::from_millis(1))
                .expect("test instant can be backdated")
        ));
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

    /// 実機cold（`removal-cold-2`）の再発防止: 直前の読み取りが失敗（カウント1）して今回成功（0へリセット）した
    /// とき、通過マークの追随（意図の破棄と読み直し）を止めない。止めると、最初の読み取りがfence内で無視された
    /// 予測が、以後の打鍵中（typing-idleガード）に訂正されず約12秒Engineが固まる。
    #[test]
    fn poll_counted_no_new_miss_continues_follow_after_recovery() {
        assert!(poll_counted_no_new_miss(0, 0), "失敗なし");
        assert!(
            poll_counted_no_new_miss(1, 0),
            "直前の失敗から成功で復帰(リセット)しても追随は続ける"
        );
        assert!(poll_counted_no_new_miss(2, 1));
        assert!(
            !poll_counted_no_new_miss(0, 1),
            "今回新しく失敗したら追随しない"
        );
        assert!(!poll_counted_no_new_miss(1, 2));
    }

    /// BUG-158: 意図の破棄の判定。観測成功時は窓の間だけ、窓の終了時は窓が切れて未破棄のときだけ。
    #[test]
    fn should_drop_intents_for_mode_key_pass_distinguishes_observation_and_expiry() {
        let w = 300;
        // 観測が成功したとき: 窓の間だけ捨てる。
        assert!(should_drop_intents_for_mode_key_pass(0, false, false, w));
        assert!(
            should_drop_intents_for_mode_key_pass(299, true, false, w),
            "2回目以降の観測でも(desired揃え)"
        );
        assert!(!should_drop_intents_for_mode_key_pass(300, false, false, w));
        // 窓の終了時: 窓の間は何もしない(最初のtickで早すぎる破棄をしない。CIで実際に起きたバグ)。
        assert!(
            !should_drop_intents_for_mode_key_pass(142, false, true, w),
            "窓の間は捨てない"
        );
        assert!(!should_drop_intents_for_mode_key_pass(299, false, true, w));
        // 窓が切れて未破棄なら捨てる。
        assert!(should_drop_intents_for_mode_key_pass(300, false, true, w));
        assert!(should_drop_intents_for_mode_key_pass(5000, false, true, w));
        // 観測の成功で既に捨てたなら、窓が切れても捨てない(通過より後の明示意図を守る)。
        assert!(!should_drop_intents_for_mode_key_pass(300, true, true, w));
    }

    /// BUG-158: 通過マークの窓の間の読み直し間隔。成功なら再読み取り間隔、失敗なら窓の終了時の1回だけ。
    #[test]
    fn mode_key_pass_next_read_ms_backs_off_after_failed_read() {
        assert_eq!(mode_key_pass_next_read_ms(true, 280, 60), 60);
        assert_eq!(mode_key_pass_next_read_ms(false, 280, 60), 281);
        assert_eq!(mode_key_pass_window_remaining_ms(20, 300), Some(280));
        assert_eq!(mode_key_pass_window_remaining_ms(300, 300), None);
        assert_eq!(mode_key_pass_window_remaining_ms(301, 300), None);
    }

    /// `imm-learning`の入口の意味: 時間切れは「IMMが使えない」証拠に数えない。本当に使えないパターン
    /// （即時の拒否・IME窓なしが連続）は従来どおり閾値で降格する。
    #[test]
    fn timeouts_are_not_imm_evidence_but_immediate_refusals_are() {
        // 分類: 実測(CI、MS-IME本体)の二峰性。成功は0〜20ms、時間切れは50〜100ms。
        assert!(send_failure_is_timeout(1460, 51_000, 50));
        assert!(
            send_failure_is_timeout(0, 50_953, 50),
            "GetLastErrorが0でも50ms以上なら時間切れ"
        );
        assert!(
            send_failure_is_timeout(1460, 300, 50),
            "ERROR_TIMEOUTなら短くても時間切れ"
        );
        assert!(
            !send_failure_is_timeout(5, 120, 50),
            "ERROR_ACCESS_DENIED(昇格プロセスのUIPI拒否)は時間切れではない"
        );
        assert!(
            !send_failure_is_timeout(0, 800, 50),
            "即時の失敗は時間切れではない"
        );
        // 数え方: 3連続のシミュレーション。時間切れ3連続は数えず(降格しない)、即時の拒否3連続は数える(降格する)。
        let count = |seq: &[bool]| -> u32 {
            // seq の各要素 = そのreadが時間切れか。時間切れは数えない(カウントも変えない)。
            seq.iter()
                .filter(|&&timed_out| read_miss_is_imm_evidence(timed_out))
                .count() as u32
        };
        assert_eq!(
            count(&[true, true, true]),
            0,
            "時間切れの連続は降格の材料にならない(CI MS-IME本体)"
        );
        assert_eq!(
            count(&[false, false, false]),
            3,
            "即時の拒否が3連続なら閾値(3)に届く(本当にIMM不可のアプリ)"
        );
        assert_eq!(
            count(&[true, false, true, false]),
            2,
            "混在は即時の拒否だけを数える"
        );
    }
}
