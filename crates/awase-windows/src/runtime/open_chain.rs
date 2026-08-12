#![allow(unsafe_code)] // `read_ime_state_fast` は Win32 IMM API(lib.rs のクレート全体 allow から個別移管)
//! ImmCross を**チェーンの要素として**含む非同期 actuation（ADR-089 §2.3、Phase B）。
//!
//! # なぜこのモジュールが要るのか — 二重経路の解消
//!
//! Phase B 以前、ImmCross の書き込みは `dispatch_ime_set_open`
//! （`runtime/executor.rs`）と `kp_stage_shadow_ime_toggle`
//! （`runtime/key_pipeline.rs`）の `spawn_local` ブロックに**直接** inline されて
//! おり、`ImeController` の戦略チェーンの外側にあった。そのため
//! 「ImmCross が失敗したら残りの戦略で再走査する」ためだけに
//! `ImeController::apply_skipping_imm`（chain の 2 番目以降を走る 2 本目の入口）が
//! 必要だった。
//!
//! ImmCross を [`WriteMechanism::ImmCross`] として chain に入れると、
//! `Failed` のフォールスルーは
//! `state/actuation_chain.rs::Actuation::<Verified>::run_chain_async` が
//! 自動的に行う。**`apply_skipping_imm` は撤去した**（ADR-089 §6 Phase B item 6）。
//!
//! # 挙動を変えていないことの確認（実装時、2026-08-12）
//!
//! - ImmCross の帰結写像（`Written` → `Applied` / `Aborted` → `UnsafeToToggle` /
//!   `Failed` → 実状態を読んで `AlreadyMatched` か `Failed`）は、移設前の
//!   `executor.rs` / `key_pipeline.rs` の分岐をそのまま持ってきたもの。
//! - `Failed` を返した後に走る機構は、旧 `apply_skipping_imm`（`strategies[1..]` を
//!   `is_applicable` で絞って `Failed` のときだけ次へ）と同じ集合・同じ順序。
//! - 旧実装は `with_app` の中で **1 つの view** を作って残り戦略を全部評価して
//!   いたのに対し、本実装は**機構ごとに** view を作る。実害が無いのは
//!   「`Failed` を返す戦略が `ImmCrossProcessStrategy` だけ」（ADR-089 §2.3）で
//!   あり、ImmCross 以降で 2 回以上 write が走ることが構造的に無いため
//!   （`GjiDirectStrategy` / `MsImeDirectStrategy` は `Failed` を返さない）。
//! - 旧実装は `is_applicable` が偽の戦略を飛ばしていた。本実装は
//!   「適用不能なら `Failed` を返す」形にしているが、`Failed` は必ず
//!   フォールスルーするため走査結果は同一である。

use awase::platform::ImeOpenOutcome;

use crate::ime::{ActuationOutcome, ActuationTarget, ConvAfterOpen};
use crate::state::actuation_chain::{
    Actuation, AsyncMechanismWriter, VerifiedTarget, WriteMechanism,
};

/// ImmCross 機構の書き込み方法。呼び出し元が起案時に決める。
pub(crate) enum ImmCrossOp {
    /// ADR-086 INV-14 準拠: 起案時に捕獲した `ActuationTarget` へ
    /// open（+ ROMAN 補完 conv）を**同一の検証済み hwnd**で書く。
    Targeted {
        target: ActuationTarget,
        conv_after_open: ConvAfterOpen,
        /// 起案時点の focus 世代。`verify_still_current` の比較基準。
        focus_gen: u32,
    },
    /// 宛先を捕獲しないクロスプロセス書き込み（shadow-toggle の OFF 経路）。
    ///
    /// **ADR-086 INV-14 の未移行分**（ADR-089 §6 Phase C item 12）。Phase B では
    /// 挙動を変えないため、旧 `set_ime_open_cross_process_async` のままにする。
    Untargeted,
}

impl ImmCrossOp {
    const fn verified_target(&self) -> VerifiedTarget {
        match self {
            Self::Targeted { .. } => VerifiedTarget::Captured,
            Self::Untargeted => VerifiedTarget::FocusImplicit,
        }
    }
}

/// 非同期 writer。ImmCross だけが await し、残りは同期戦略へ委譲する。
struct AsyncChainWriter {
    /// 1 回だけ使える（`Actuation` 値のアフィン性と同じ理由で `Option`）。
    imm: Option<ImmCrossOp>,
}

impl AsyncMechanismWriter for AsyncChainWriter {
    fn is_applicable(&self, mechanism: WriteMechanism) -> bool {
        match mechanism {
            // 呼び出し元が ImmCross 経路だと判断した場合にのみ chain に入る。
            WriteMechanism::ImmCross => self.imm.is_some(),
            // 残りは実行時の `ImeControlView` を見ないと判断できない（view の
            // 構築に `with_app` が要る）。適用可否は `write` の中で判定し、
            // 適用不能なら `Failed` を返してフォールスルーさせる（モジュール
            // doc「挙動を変えていないことの確認」参照）。
            _ => true,
        }
    }

    async fn write(&mut self, mechanism: WriteMechanism, open: bool) -> ImeOpenOutcome {
        match mechanism {
            WriteMechanism::ImmCross => {
                let Some(op) = self.imm.take() else {
                    return ImeOpenOutcome::Failed;
                };
                imm_cross_write(op, open).await
            }
            other => fallback_write(other, open),
        }
    }
}

/// ImmCross の実書き込み。旧 `executor.rs` / `key_pipeline.rs` の分岐をそのまま
/// 持ってきたもの。
async fn imm_cross_write(op: ImmCrossOp, open: bool) -> ImeOpenOutcome {
    let raw = match op {
        ImmCrossOp::Targeted {
            target,
            conv_after_open,
            focus_gen,
        } => {
            // ADR-086 §1.2 欠陥1 是正: 起案時点の focus_gen を捕獲し、
            // verify → open → conv をすべて 1 回の呼び出しに閉じ込めて
            // 同一 hwnd を使い回す。
            let result = crate::ime::set_ime_open_then_conv_for_target(
                target,
                open,
                conv_after_open,
                || {
                    crate::with_app(|runtime| runtime.platform.output.ime_mode_focus_gen.get())
                        .unwrap_or_else(|| focus_gen.wrapping_add(1))
                },
            )
            .await;
            if let Some(conv_outcome) = result.conv {
                log::debug!("[apply-ime] ROMAN 補完結果: {conv_outcome:?}");
            }
            result.open
        }
        ImmCrossOp::Untargeted => {
            if crate::ime::set_ime_open_cross_process_async(open).await {
                ActuationOutcome::Written
            } else {
                ActuationOutcome::Failed
            }
        }
    };

    match raw {
        ActuationOutcome::Written => ImeOpenOutcome::Applied,
        ActuationOutcome::Aborted(reason) => {
            // INV-14: Aborted は「一度も書いていない」ので Applied 扱いにしない。
            // UnsafeToToggle は `on_ime_apply_complete` の C/D（SSOT の
            // applied/belief 書き込み）を一切実行させない。**フォールバックも
            // 通さない**（検証済みでない hwnd への意図しない送信を避けるため）
            // ——`UnsafeToToggle` は `falls_through` が偽なので chain はここで
            // 止まる（ADR-089 §2.3）。E（post_ime_refresh）だけは
            // UnsafeToToggle でも走るため Aborted(GenStale) の取りこぼしは
            // 20ms 後の refresh で拾われる。
            log::debug!("[apply-ime] ImmCross open Aborted({reason:?}) → UnsafeToToggle");
            ImeOpenOutcome::UnsafeToToggle
        }
        ActuationOutcome::Failed => {
            // SAFETY: `read_ime_state_fast` は Win32 IMM API を呼ぶ。
            //         spawn_local はメインスレッドのメッセージループで実行される。
            let actual = unsafe { crate::ime::read_ime_state_fast() }.ime_on;
            if actual == Some(open) {
                log::debug!(
                    "[apply-ime] ImmCross failed but actual ime_on={actual:?} \
                     already matches desired={open}, skip fallback"
                );
                ImeOpenOutcome::AlreadyMatched
            } else {
                log::debug!(
                    "[apply-ime] ImmCross failed (async, actual ime_on={actual:?}), \
                     falling through to next mechanism"
                );
                // `Failed` は `falls_through` が真 → run_chain_async が次の機構へ
                // 進む（旧 `apply_skipping_imm` と同じ範囲）。
                ImeOpenOutcome::Failed
            }
        }
    }
}

/// ImmCross 以外の機構の同期 write。view は完了時点の状態から作り直す
/// （旧 `apply_skipping_imm` が `with_app` 内で `shadow_ime_control_view()` を
/// 作り直していたのと同じ）。
fn fallback_write(mechanism: WriteMechanism, open: bool) -> ImeOpenOutcome {
    crate::with_app(|app| {
        let view = app.shadow_ime_control_view();
        if crate::ime_controller::mechanism_is_applicable(mechanism, &view) {
            crate::ime_controller::apply_mechanism(mechanism, open, &view)
        } else {
            ImeOpenOutcome::Failed
        }
    })
    .unwrap_or(ImeOpenOutcome::Failed)
}

/// ImmCross を先頭に含む機構チェーンを非同期に走査する。
///
/// 走査規則（`Failed` のときだけ次へ）は `state/actuation_chain.rs` が SSOT。
// `ActuationTarget`（HWND を含む）を await をまたいで保持するため Future は
// `Send` にならない。`win32_async::spawn_local`（シングルスレッド実行）経由で
// のみ呼ばれるため実害はない（`ActuationTarget::verify_still_current` と同じ制約）。
#[allow(clippy::future_not_send)]
pub(crate) async fn run_open_chain_async(open: bool, imm: ImmCrossOp) -> ImeOpenOutcome {
    // ADR-087 の `issue_open_warrant()` は本番未配線のため暫定授権を使う
    // （`state/actuation_chain.rs` モジュール doc の「差分」3）。
    let actuation = Actuation::request(open)
        .warrant_pending_adr087()
        .verify(imm.verified_target());
    let mut writer = AsyncChainWriter { imm: Some(imm) };
    actuation
        .run_chain_async(&WriteMechanism::ALL, &mut writer)
        .await
}
