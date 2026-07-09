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
//! ### 適用対象
//!
//! `ImmCrossProbe`（ImmLikeTicket）は非同期完了時に epoch を照合し、
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

/// 棄却統計（グローバルアトミック）。
static REJECTED_EPOCH_MISMATCH: AtomicU64 = AtomicU64::new(0);

/// 棄却統計のスナップショット。
#[derive(Debug, Default, Clone, Copy)]
pub struct RejectionStats {
    /// FocusEpoch 不一致による棄却数（累積）
    pub epoch_mismatch: u64,
}

/// 棄却カウンタを読み取り、ゼロにリセットする（診断ダンプ用）。
#[must_use]
pub fn drain_stats() -> RejectionStats {
    RejectionStats {
        epoch_mismatch: REJECTED_EPOCH_MISMATCH.swap(0, Ordering::Relaxed),
    }
}

/// フォーカス変更のエポック番号。
///
/// `FocusStore::focus_epoch` に格納され、`on_focus_process_changed` ごとに
/// `wrapping_add(1)` でインクリメントされる。
pub type FocusEpoch = u64;

/// ImmLike プローブ（`ImmCrossProbe` / `FocusProbe`）が spawn 時にキャプチャするチケット。
///
/// 非同期完了後に [`ImmLikeTicket::admit`] を呼び、epoch が変わっていれば棄却する。
///
/// # 使用例
///
/// ```ignore
/// // spawn 直前にチケットを作成
/// let ticket = ImmLikeTicket { focus_epoch: self.platform_state.focus.focus_epoch };
/// win32_async::spawn_local(async move {
///     let snap = read_ime_state_full_async().await;
///     if let Some(open) = snap.ime_on {
///         let _ = with_app(|app| {
///             let current = app.platform_state.focus.focus_epoch;
///             if let Admission::Reject(r) = ticket.admit(current) {
///                 log::debug!("[ImmCrossProbe] epoch rejected: {r}");
///                 return;
///             }
///             app.platform_state.ime.write_imm_cross_probe(open, tick_ms);
///         });
///     }
/// });
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ImmLikeTicket {
    /// spawn 時のフォーカスエポック
    pub focus_epoch: FocusEpoch,
}

/// 受理済み観測のトークン。プライベートコンストラクタにより admission を通過した証明になる。
///
/// `write_*` 関数はこの型を受け取ることで、コンパイラレベルで
/// "admission を通らない write" を防止する。
///
/// - 非同期 probe: `ImmLikeTicket::admit()` → `Admission::Accept(AcceptedObservation)`
/// - 同期 probe: `AcceptedObservation::for_sync(epoch)` で直接構築（シングルスレッドのため常に有効）
// `#[non_exhaustive]` は他クレートからの構造体リテラル構築のみを禁止するが、この
// `_private` フィールドは同一クレート内の他モジュールからの構築も禁止する意図的な
// カプセル化（admission を通らない write をコンパイラで防ぐ）。#[non_exhaustive] に
// 置き換えると同クレート内の抜け道を許してしまうため見送る。
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, Copy)]
pub struct AcceptedObservation {
    /// 受理時のフォーカスエポック（診断・derive_open フィルタ用）
    pub focus_epoch: FocusEpoch,
    /// プライベートフィールドにより外部から直接構築不可。
    _private: (),
}

impl AcceptedObservation {
    /// 同期プローブ専用コンストラクタ。
    ///
    /// シングルスレッド実行のため、spawn 〜 complete 間にフォーカスが変わることは
    /// ない（epoch mismatch 不可）。epoch は観測の来歴記録・epoch フィルタ用。
    #[must_use]
    pub fn for_sync(focus_epoch: FocusEpoch) -> Self {
        Self {
            focus_epoch,
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
    /// フォーカスエポックが変わった（probe spawn 後にフォーカス変更があった）
    FocusEpochChanged {
        at_spawn: FocusEpoch,
        current: FocusEpoch,
    },
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FocusEpochChanged { at_spawn, current } => {
                write!(f, "focus epoch changed ({at_spawn} → {current})")
            }
        }
    }
}

impl ImmLikeTicket {
    /// 完了時の受理判定。
    ///
    /// `current_epoch` は `with_app` 内で `app.platform_state.focus.focus_epoch` を渡す。
    /// 棄却時は [`drain_stats`] で集計できるアトミックカウンタをインクリメントする。
    #[must_use]
    pub fn admit(self, current_epoch: FocusEpoch) -> Admission {
        if current_epoch != self.focus_epoch {
            REJECTED_EPOCH_MISMATCH.fetch_add(1, Ordering::Relaxed);
            return Admission::Reject(RejectReason::FocusEpochChanged {
                at_spawn: self.focus_epoch,
                current: current_epoch,
            });
        }
        Admission::Accept(AcceptedObservation {
            focus_epoch: current_epoch,
            _private: (),
        })
    }
}
