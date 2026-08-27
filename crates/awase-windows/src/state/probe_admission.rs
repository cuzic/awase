//! プローブ受理ポリシー（Observation Admission Layer）
//!
//! 各 probe が spawn 時にキャプチャしたコンテキストを保持し、
//! 完了時に「この観測を受理すべきか」を判定する。
//!
//! ## 設計思想
//!
//! ### フォーカスエポック vs 時間ベースのシャドウグレース
//!
//! 以前は `shadow_on && probe_age_ms < SHADOW_GRACE_MS` という時間ベースの
//! 抑制ロジックが複数箇所にコピーされていた。
//!
//! エポック方式に切り替えることで：
//!
//! - **正確**: ms 精度の競合なしに「フォーカスが変わったか」を判定できる
//! - **一元化**: 判定ロジックがこのモジュールに集約される
//! - **自己文書化**: チケットが spawn 時の意図を型で表す
//!
//! ### エポックだけでは足りない理由（ADR-106 決定3）
//!
//! `focus_epoch` は `on_focus_process_changed`（プロセス変更）でしか進まないため、
//! **同一プロセス内でウィンドウだけが変わるケース**（例: Chrome のタブ/ウィンドウ
//! 切替）を検知できない。`hwnd` を併せて照合することで、epoch が同じでも
//! ウィンドウが変わっていれば棄却できるようにする。
//!
//! ### 適用対象
//!
//! `ImmCrossProbe`（ImmLikeTicket）は非同期完了時に epoch/hwnd を照合し、
//! spawn 後にフォーカスが変わっていれば棄却する。
//! これにより仮想デスクトップ切替アニメーション中の経由ウィンドウ
//! （ForegroundStaging 等）が返す false 観測が High confidence で
//! 書き込まれ Engine OFF カスケードが起きる問題を構造的に排除する。
//!
//! ## 棄却カウンタ（Step 8）
//!
//! 棄却された probe はアトミックカウンタに記録される。
//! 診断ダンプ時に [`drain_stats`] で取り出し、ログ出力に使う。

use std::sync::atomic::{AtomicU64, Ordering};

use super::ime_event::HwndId;

/// 棄却統計（グローバルアトミック）。
static REJECTED_EPOCH_MISMATCH: AtomicU64 = AtomicU64::new(0);
/// hwnd 不一致による棄却統計のうち、spawn 時と現在で top-level 祖先ウィンドウ
/// （`root_hwnd`、`GetAncestor(hwnd, GA_ROOT)`）が同じだったケース（PR 109
/// コードレビュー指摘1 Step1: ネイティブ Win32 マルチフィールドダイアログでの
/// フィールド間 Tab 移動等、同一 top-level ウィンドウ内でのコントロール間
/// フォーカス移動が疑われる。BUG-91 参照）。
static REJECTED_HWND_MISMATCH_SAME_ROOT: AtomicU64 = AtomicU64::new(0);
/// hwnd 不一致による棄却統計のうち、spawn 時と現在で `root_hwnd` が異なった
/// ケース（真に別の top-level ウィンドウへの切替）。
static REJECTED_HWND_MISMATCH_CROSS_ROOT: AtomicU64 = AtomicU64::new(0);

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
        epoch_mismatch: REJECTED_EPOCH_MISMATCH.swap(0, Ordering::Relaxed),
        hwnd_mismatch_same_root: REJECTED_HWND_MISMATCH_SAME_ROOT.swap(0, Ordering::Relaxed),
        hwnd_mismatch_cross_root: REJECTED_HWND_MISMATCH_CROSS_ROOT.swap(0, Ordering::Relaxed),
    }
}

/// hwnd 不一致棄却を `root_hwnd` の一致/不一致で分類してカウンタへ積む。
///
/// `ImmLikeTicket::admit()` 自身は `root_hwnd` を持たない（判定ロジックには
/// 使わない設計、`FocusFence` は epoch/hwnd のみ）ため、`root_hwnd` に
/// アクセスできる呼び出し元（`admit_epoch_in_app`、Windows 専用）がここを呼ぶ
/// （PR 109 コードレビュー指摘1 Step1、計測のみで判定ロジックは変えない）。
#[cfg_attr(not(windows), allow(dead_code))]
fn record_hwnd_mismatch(same_root: bool) {
    if same_root {
        REJECTED_HWND_MISMATCH_SAME_ROOT.fetch_add(1, Ordering::Relaxed);
    } else {
        REJECTED_HWND_MISMATCH_CROSS_ROOT.fetch_add(1, Ordering::Relaxed);
    }
}

/// フォーカス変更のエポック番号。
///
/// `FocusStore::focus_epoch` に格納され、`on_focus_process_changed` ごとに
/// `wrapping_add(1)` でインクリメントされる。
pub type FocusEpoch = u64;

/// フォーカスの同一性を表す「epoch + hwnd」のペア（ADR-106 決定3）。
///
/// `epoch` はプロセス変更でのみ進むため、同一プロセス内でウィンドウだけが
/// 変わるケースを検知できない。`hwnd` を併せて持つことで、`ImmLikeTicket::admit`
/// や `ObservationStore::is_identity_ok` が両軸を1つの値として原子的に照合・
/// 更新できるようにする（従来は epoch と hwnd を別々のフィールド/引数として
/// 持ち回っており、更新口が2つに分かれて片方だけ古くなる退行が起きた）。
///
/// **命名注意**: `focus/current.rs` の `FocusIdentity`（journal 用スナップショット:
/// hwnd/pid/class_name/process_name/app_profile/app_kind/focus_kind）とは別物。
/// ADR-106 本文が提案する `FocusIdentity` という名前は既にこの別の型が使っているため
/// 採用せず、`FocusFence` とした。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusFence {
    pub epoch: FocusEpoch,
    pub hwnd: HwndId,
}

/// ImmLike プローブ（`ImmCrossProbe` / `FocusProbe`）が spawn 時にキャプチャするチケット。
///
/// 非同期完了後に [`ImmLikeTicket::admit`] を呼び、epoch/hwnd のいずれかが変わって
/// いれば棄却する（ADR-106 決定3、ADR-086 の `ActuationTarget{hwnd, focus_gen}` と
/// 構造的に同型: 時間軸(epoch)と空間軸(hwnd)の両方をフェンスする）。
///
/// # 使用例
///
/// ```ignore
/// // spawn 直前にチケットを作成
/// let ticket = ImmLikeTicket {
///     fence: self.focus_fence(),
/// };
/// win32_async::spawn_local(async move {
///     let snap = read_ime_state_full_async().await;
///     if let Some(open) = snap.ime_on {
///         let _ = with_app(|app| {
///             let current = app.focus_fence();
///             if let Admission::Reject(r) = ticket.admit(current) {
///                 log::debug!("[ImmCrossProbe] rejected: {r}");
///                 return;
///             }
///             app.platform_state.ime.write_imm_cross_probe(open, tick_ms);
///         });
///     }
/// });
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ImmLikeTicket {
    /// spawn 時のフォーカス同一性（epoch + hwnd）
    pub fence: FocusFence,
}

/// 受理済み観測のトークン。プライベートコンストラクタにより admission を通過した証明になる。
///
/// `write_*` 関数はこの型を受け取ることで、コンパイラレベルで
/// "admission を通らない write" を防止する。
///
/// - 非同期 probe: `ImmLikeTicket::admit()` → `Admission::Accept(AcceptedObservation)`
/// - 同期 probe: `AcceptedObservation::for_sync(fence)` で直接構築
///   （シングルスレッドのため常に有効）
// `#[non_exhaustive]` は他クレートからの構造体リテラル構築のみを禁止するが、この
// `_private` フィールドは同一クレート内の他モジュールからの構築も禁止する意図的な
// カプセル化（admission を通らない write をコンパイラで防ぐ）。#[non_exhaustive] に
// 置き換えると同クレート内の抜け道を許してしまうため見送る。
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, Copy)]
pub struct AcceptedObservation {
    /// 受理時のフォーカス同一性（診断・derive_any フィルタ用）
    pub fence: FocusFence,
    /// プライベートフィールドにより外部から直接構築不可。
    _private: (),
}

impl AcceptedObservation {
    /// 受理時のフォーカスエポック（診断・derive_any フィルタ用）。
    #[must_use]
    pub const fn epoch(&self) -> FocusEpoch {
        self.fence.epoch
    }

    /// 受理時のフォーカス hwnd（診断・derive_any フィルタ用、ADR-106 決定3）。
    #[must_use]
    pub const fn hwnd(&self) -> HwndId {
        self.fence.hwnd
    }

    /// 同期プローブ専用コンストラクタ。
    ///
    /// シングルスレッド実行のため、spawn 〜 complete 間にフォーカスが変わることは
    /// ない（epoch/hwnd mismatch 不可）。fence は観測の来歴記録・derive_any
    /// フィルタ用。
    ///
    /// 呼び出し元は `runtime` 層の 3 箇所のみ（ADR-089 §1.3(b)・§6 Phase A item 4
    /// に従い `pub` から `pub(crate)` へ縮小した）。この関数は
    /// `Observed<FocusProbe>` / `Observed<ImmCrossProbe>` / `Observed<ObserverPoll>`
    /// の witness を作れる唯一の同期経路であり、crate 外へ出す理由が無い。
    /// 唯一の呼び出し元 `runtime/` は `#[cfg(windows)]` のため、非 Windows では
    /// 未使用になる（`state/mod.rs` の ungated モジュール群と同じ局所抑制）。
    #[cfg_attr(not(windows), allow(dead_code))]
    #[must_use]
    pub(crate) fn for_sync(fence: FocusFence) -> Self {
        Self {
            fence,
            _private: (),
        }
    }
}

/// プローブ受理/棄却の判定結果
#[derive(Debug)]
pub enum Admission {
    /// 受理。`AcceptedObservation` トークンを持つ。
    Accept(AcceptedObservation),
    Reject(RejectReason),
}

/// 棄却理由
#[derive(Debug)]
pub enum RejectReason {
    /// フォーカスエポックが変わった（probe spawn 後にプロセスが変わった）
    FocusEpochChanged {
        at_spawn: FocusEpoch,
        current: FocusEpoch,
    },
    /// エポックは同じだが hwnd が変わった（同一プロセス内でウィンドウだけが
    /// 変わった。ADR-106 決定3: `focus_epoch` はプロセス変更でのみ進むため、
    /// この種の変化はエポック単独では検知できない）
    FocusHwndChanged {
        epoch: FocusEpoch,
        at_spawn: HwndId,
        current: HwndId,
    },
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FocusEpochChanged { at_spawn, current } => {
                write!(f, "focus epoch changed ({at_spawn} → {current})")
            }
            Self::FocusHwndChanged {
                epoch,
                at_spawn,
                current,
            } => {
                write!(
                    f,
                    "focus hwnd changed within epoch {epoch} ({at_spawn:?} → {current:?})"
                )
            }
        }
    }
}

impl ImmLikeTicket {
    /// 完了時の受理判定。
    ///
    /// `current` は `with_app` 内で実際のフォーカス文脈から取得した「現在」の
    /// フェンスを渡す。epoch を先に照合し（プロセス変更を検知）、一致していれば
    /// 続けて hwnd を照合する（同一プロセス内のウィンドウ変更を検知、ADR-106
    /// 決定3）。epoch 不一致は [`drain_stats`] で集計できるアトミックカウンタを
    /// ここで直接インクリメントする。hwnd 不一致は `root_hwnd`（`admit()` 自身は
    /// 持たない）による same_root/cross_root 分類が必要なため、呼び出し元
    /// （`admit_epoch_in_app`、PR 109 コードレビュー指摘1 Step1）で計測する
    /// ——この関数自体の Accept/Reject 判定ロジックは変更していない。
    #[must_use]
    pub fn admit(self, current: FocusFence) -> Admission {
        if current.epoch != self.fence.epoch {
            REJECTED_EPOCH_MISMATCH.fetch_add(1, Ordering::Relaxed);
            return Admission::Reject(RejectReason::FocusEpochChanged {
                at_spawn: self.fence.epoch,
                current: current.epoch,
            });
        }
        if current.hwnd != self.fence.hwnd {
            return Admission::Reject(RejectReason::FocusHwndChanged {
                epoch: current.epoch,
                at_spawn: self.fence.hwnd,
                current: current.hwnd,
            });
        }
        Admission::Accept(AcceptedObservation {
            fence: current,
            _private: (),
        })
    }
}

/// `with_app` クロージャの中で呼ぶための、`admit()` → 早期 return + ログの定型処理を一元化する。
///
/// 以前は「spawn 時にチケットをキャプチャ → await → `with_app` → `ticket.admit(current_epoch,
/// current_hwnd)` で再照合 → 不一致ならログを出して早期 return」という形が
/// `ImmCrossProbe` / `FocusProbe` 系の複数の非同期完了ハンドラにほぼ同じ形で複製
/// されていた（この struct 冒頭 doc の使用例が、まさにその複製されていたグルー
/// コード）。受理されれば `f(app, accepted)` を呼び、棄却時は `reject_log` を
/// そのまま `log::debug!` に渡して `None` を返す。
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
    ticket: ImmLikeTicket,
    reject_log: &str,
    f: impl FnOnce(&mut crate::runtime::Runtime, AcceptedObservation) -> R,
) -> Option<R> {
    let current = app.focus_fence();
    match ticket.admit(current) {
        Admission::Accept(accepted) => Some(f(app, accepted)),
        Admission::Reject(RejectReason::FocusHwndChanged { at_spawn, .. }) => {
            // root_hwnd は計測専用（BUG-91）。判定ロジック（上の admit()）は
            // 一切変更しておらず、ここは棄却が確定した後の分類のみ。
            let spawn_root = crate::focus::classify::root_hwnd_of(at_spawn.0);
            let current_root = app.platform.focus.current.root_hwnd;
            let same_root = spawn_root == current_root;
            record_hwnd_mismatch(same_root);
            log::debug!("{reject_log} (same_root={same_root})");
            None
        }
        Admission::Reject(_) => {
            log::debug!("{reject_log}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HWND: HwndId = HwndId(0x1234);
    const OTHER_HWND: HwndId = HwndId(0x5678);

    /// `admit()` は `current.epoch != self.fence.epoch` で棄却する。この比較演算子
    /// (`!=`→`==`) が反転すると、epoch が一致するときに棄却・不一致のときに受理という
    /// 完全に逆の判定になり、`current.epoch != self.fence.epoch` の目的（フォーカス変更後
    /// の stale な probe を弾いて Engine OFF カスケードを防ぐ）が壊れる。この関数には
    /// これまでテストが1件も無かった。
    #[test]
    fn admit_accepts_when_epoch_and_hwnd_match() {
        let ticket = ImmLikeTicket {
            fence: FocusFence {
                epoch: 5,
                hwnd: HWND,
            },
        };
        assert!(matches!(
            ticket.admit(FocusFence {
                epoch: 5,
                hwnd: HWND
            }),
            Admission::Accept(_)
        ));
    }

    #[test]
    fn admit_rejects_when_epoch_differs() {
        let ticket = ImmLikeTicket {
            fence: FocusFence {
                epoch: 5,
                hwnd: HWND,
            },
        };
        match ticket.admit(FocusFence {
            epoch: 6,
            hwnd: HWND,
        }) {
            Admission::Reject(RejectReason::FocusEpochChanged { at_spawn, current }) => {
                assert_eq!(at_spawn, 5);
                assert_eq!(current, 6);
            }
            other => panic!("expected Reject(FocusEpochChanged), got {other:?}"),
        }
    }

    /// ADR-106 決定3: epoch は一致するが hwnd だけが変わった場合も棄却する
    /// （同一プロセス内でウィンドウだけが切り替わるケースは `focus_epoch` 単独
    /// では検知できない）。
    #[test]
    fn admit_rejects_when_hwnd_differs_even_with_matching_epoch() {
        let ticket = ImmLikeTicket {
            fence: FocusFence {
                epoch: 5,
                hwnd: HWND,
            },
        };
        match ticket.admit(FocusFence {
            epoch: 5,
            hwnd: OTHER_HWND,
        }) {
            Admission::Reject(RejectReason::FocusHwndChanged {
                epoch,
                at_spawn,
                current,
            }) => {
                assert_eq!(epoch, 5);
                assert_eq!(at_spawn, HWND);
                assert_eq!(current, OTHER_HWND);
            }
            other => panic!("expected Reject(FocusHwndChanged), got {other:?}"),
        }
    }

    /// epoch と hwnd の両方が異なる場合は epoch 不一致が優先して報告される
    /// （epoch はプロセス変更の検知であり、より上位の不一致のため）。
    #[test]
    fn admit_reports_epoch_mismatch_when_both_epoch_and_hwnd_differ() {
        let ticket = ImmLikeTicket {
            fence: FocusFence {
                epoch: 5,
                hwnd: HWND,
            },
        };
        assert!(matches!(
            ticket.admit(FocusFence {
                epoch: 6,
                hwnd: OTHER_HWND,
            }),
            Admission::Reject(RejectReason::FocusEpochChanged { .. })
        ));
    }
}
