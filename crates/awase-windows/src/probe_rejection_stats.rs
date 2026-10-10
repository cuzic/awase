//! プローブ棄却の診断カウンタ（殻。`probe_admission` の純粋な判定から切り出した、ADR-229 段階 A-3）。
//!
//! 棄却の判定は `probe_admission::ImmLikeTicket::admit`（純粋）が行い、ここは棄却の件数を
//! グローバルなアトミックカウンタに積むだけ。判定の結果は変えない。

use crate::lifetime_counter::LifetimeCounter;

/// 棄却統計（グローバルアトミック、ADR-164 フェーズ8）。
///
/// `record_epoch_mismatch()` と `record_hwnd_mismatch()` という2つの独立した呼び出し元から
/// 書かれる診断カウンタで、単一の呼び出し木を持たないため引数引き回しは
/// できない（ADR-164 分類C）。3フィールドを1つの singleton にまとめる。
struct RejectionCounters {
    /// FocusEpoch 不一致による棄却統計。
    epoch_mismatch: LifetimeCounter,
    /// hwnd 不一致による棄却統計のうち、spawn 時と現在で top-level 祖先ウィンドウ
    /// （`root_hwnd`、`GetAncestor(hwnd, GA_ROOT)`）が同じだったケース（PR 109
    /// コードレビュー指摘1 Step1: ネイティブ Win32 マルチフィールドダイアログでの
    /// フィールド間 Tab 移動等、同一 top-level ウィンドウ内でのコントロール間
    /// フォーカス移動が疑われる。BUG-91 参照）。
    hwnd_mismatch_same_root: LifetimeCounter,
    /// hwnd 不一致による棄却統計のうち、spawn 時と現在で `root_hwnd` が異なった
    /// ケース（真に別の top-level ウィンドウへの切替）。
    hwnd_mismatch_cross_root: LifetimeCounter,
}

impl RejectionCounters {
    const fn new() -> Self {
        Self {
            epoch_mismatch: LifetimeCounter::new(),
            hwnd_mismatch_same_root: LifetimeCounter::new(),
            hwnd_mismatch_cross_root: LifetimeCounter::new(),
        }
    }
}

static REJECTION_COUNTERS: RejectionCounters = RejectionCounters::new();

/// 棄却統計のスナップショット。
#[derive(Debug, Default, Clone, Copy)]
pub struct RejectionStats {
    /// FocusEpoch 不一致による棄却数（累積）
    pub epoch_mismatch: u64,
    /// hwnd 不一致による棄却数のうち `root_hwnd` が同じだったケース（累積、
    /// epoch は一致していたケースのみ。BUG-91 参照）。
    pub hwnd_mismatch_same_root: u64,
    /// hwnd 不一致による棄却数のうち `root_hwnd` も異なったケース（累積、
    /// epoch は一致していたケースのみ）。
    pub hwnd_mismatch_cross_root: u64,
}

/// 棄却カウンタを読み取り、ゼロにリセットする（診断ダンプ用）。
#[must_use]
pub fn drain_stats() -> RejectionStats {
    RejectionStats {
        epoch_mismatch: REJECTION_COUNTERS.epoch_mismatch.drain(),
        hwnd_mismatch_same_root: REJECTION_COUNTERS.hwnd_mismatch_same_root.drain(),
        hwnd_mismatch_cross_root: REJECTION_COUNTERS.hwnd_mismatch_cross_root.drain(),
    }
}

/// hwnd 不一致棄却を `root_hwnd` の一致/不一致で分類してカウンタへ積む。
///
/// `ImmLikeTicket::admit()` 自身は `root_hwnd` を持たない（判定ロジックには
/// 使わない設計、`FocusFence` は epoch/hwnd のみ）ため、`root_hwnd` に
/// アクセスできる呼び出し元（`admit_epoch_in_app`、Windows 専用）がここを呼ぶ
/// （PR 109 コードレビュー指摘1 Step1、計測のみで判定ロジックは変えない）。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn record_hwnd_mismatch(same_root: bool) {
    if same_root {
        REJECTION_COUNTERS.hwnd_mismatch_same_root.increment();
    } else {
        REJECTION_COUNTERS.hwnd_mismatch_cross_root.increment();
    }
}

/// FocusEpoch 不一致による棄却を数える。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn record_epoch_mismatch() {
    REJECTION_COUNTERS.epoch_mismatch.increment();
}

/// `with_app` クロージャの中で呼ぶための、`admit()` → 早期 return + ログの定型処理を一元化する。
///
/// 以前は「spawn 時にチケットをキャプチャ → await → `with_app` → `ticket.admit(current_epoch,
/// current_hwnd)` で再照合 → 不一致ならログを出して早期 return」という形が
/// `ImmCrossProbe` / `FocusProbe` 系の複数の非同期完了ハンドラにほぼ同じ形で複製
/// されていた（この struct 冒頭 doc の使用例が、まさにその複製されていたグルー
/// コード）。受理されれば `f(app, accepted)` を呼び、棄却時は `reject_log` を
/// そのまま `tracing::debug!` に渡して `None` を返す。
///
/// `reject_log` は呼び出し元ごとに異なる（タグ名・文言）ログ本文をそのまま渡す
/// （ログ文言自体は既存の観測結果であり、このリファクタで変更しない）。hwnd 不一致
/// 棄却の場合のみ、`same_root`（PR 109 コードレビュー指摘1 Step1、計測専用、
/// BUG-91）を追記する。
///
/// `crate::runtime::Runtime` は `#[cfg(windows)]`（`state/` は全プラットフォーム共通）
/// のため、この関数自体も Windows 専用にする（`conv_classify`/`eisu_recovery` と同じ
/// 「呼び出し元が `#[cfg(windows)]` の runtime/ のみ」パターン、`state/mod.rs` 参照）。
#[cfg(windows)]
pub(crate) fn admit_epoch_in_app<R>(
    app: &mut crate::runtime::Runtime,
    ticket: crate::state::probe_admission::ImmLikeTicket,
    reject_log: &str,
    f: impl FnOnce(
        &mut crate::runtime::Runtime,
        crate::state::probe_admission::AcceptedObservation,
    ) -> R,
) -> Option<R> {
    let current = app.focus_fence();
    match ticket.admit(current) {
        crate::state::probe_admission::Admission::Accept(accepted) => Some(f(app, accepted)),
        crate::state::probe_admission::Admission::Reject(
            crate::state::probe_admission::RejectReason::FocusEpochChanged { .. },
        ) => {
            record_epoch_mismatch();
            tracing::debug!("{reject_log}");
            None
        }
        crate::state::probe_admission::Admission::Reject(
            crate::state::probe_admission::RejectReason::FocusHwndChanged { at_spawn, .. },
        ) => {
            // root_hwnd は計測専用（BUG-91）。判定ロジック（上の admit()）は
            // 一切変更しておらず、ここは棄却が確定した後の分類のみ。
            let spawn_root = crate::focus::classify::root_hwnd_of(at_spawn.0);
            let current_root = app.platform.focus.current.root_hwnd;
            let same_root = spawn_root == current_root;
            record_hwnd_mismatch(same_root);
            tracing::debug!("{reject_log} (same_root={same_root})");
            None
        }
    }
}
