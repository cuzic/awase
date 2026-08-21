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
//!
//! # なぜ Phase C でも chain が `WriteMechanism::ALL` のままなのか
//!
//! Phase C（ADR-089 §2.8、INV-44）は `caps(p, k).chain` を機構チェーンの
//! SSOT にしたが、**この非同期経路だけは `WriteMechanism::ALL` を渡し続ける**。
//! 理由は「起案時点の `(p, k)` を chain として固定すると挙動が変わる」ため:
//!
//! - `run_open_chain_async` は `spawn_local` の中から呼ばれ、ImmCross の
//!   書き込み（`SendMessageTimeout` を含む）を **await した後**に
//!   フォールバックへ進む。その間にフォーカスが動きうる。
//! - [`fallback_write`] は機構ごとに `shadow_ime_control_view()` を作り直して
//!   `is_applicable` を**その時点の観測で**評価する。つまり旧実装
//!   （`apply_skipping_imm`）と同じく「完了時点の状態で残り機構を選ぶ」。
//! - ここに起案時点の `caps(p, k).chain` を渡すと、await 中に profile が
//!   変わった場合に「完了時点では適用可能な機構が chain に載っていない」
//!   という新しい取りこぼしが生まれる（例: 起案時 Standard × MS-IME の
//!   chain は `[ImmCross, KanjiToggle]`。await 中に TsfNative へ移ると
//!   旧実装は `MsImeDirect` を選ぶが、固定 chain では `KanjiToggle` を送る）。
//! - `ImeKindId` は推測値である（INV-45 / P20）。await をまたいで K を
//!   固定するのは「推測値に安全側でないゲートを掛ける」に当たる。
//!
//! `WriteMechanism::ALL` は全 `caps` チェーンの**和集合**であり、
//! `is_applicable` による絞り込みと `falls_through`（`Failed` のときだけ次へ）
//! が同じである以上、`(p, k)` が変わらない限り `caps(p, k).chain` と同じ
//! 結果になる（同値性は `ime_controller.rs::caps_chain_matches_legacy_all_scan`
//! が全数で固定）。**同期経路（`ImeController::apply`）は view が 1 つに
//! 固定されているため caps chain を使う**——差が出るのは await をまたぐ
//! この経路だけである。ADR-089 §9-20 に残余論点として記録した。

use awase::platform::ImeOpenOutcome;

use crate::ime::{ActuationOutcome, ActuationTarget, ConvAfterOpen};
use crate::state::actuation_chain::{
    ActuationOrder, AsyncMechanismWriter, VerifiedTarget, WriteMechanism,
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

    // `AsyncChainWriter` は `ImmCrossOp`（`ActuationTarget` の HWND =
    // `*mut c_void`）を保持するため `Send` ではなく、その `&mut self` を await
    // をまたいで持つこの future も `Send` にならない。**シングルスレッド実行が
    // 前提の設計**であり、Send にする必要が本質的に無い:
    // この future を駆動するのは `win32_async::spawn_local`（= winmsg-executor、
    // 「現在のスレッドのメッセージループ」で回す）だけで、他スレッドへ送られる
    // 経路が無い。そもそも HWND を別スレッドへ送って IMM32 を叩くのは Win32 側の
    // スレッドアフィニティに反する（`ActuationTarget::verify_still_current` が
    // 同じ制約を持つ）。`run_open_chain_async` の同名 allow と対。
    #[allow(clippy::future_not_send)]
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
// `ImmCrossOp::Targeted` が保持する `ActuationTarget` は HWND(`*mut c_void`) を
// 含むため future は `Send` にならない。`AsyncChainWriter::write` と同じ理由
// （`spawn_local` のシングルスレッド実行が前提、HWND はスレッドアフィニティを
// 持つ）で Send は要求しない。
#[allow(clippy::future_not_send)]
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
///
/// # BUG-34 横展開 E-prep: 残存する同期ブロッキング（意図的に未解消）
///
/// この関数は `with_app`（`RUNTIME` の排他 borrow）を握ったまま
/// `apply_mechanism` → `romaji_pre_write` を呼ぶ。`romaji_pre_write` が
/// `SendMessageTimeoutW` ベースの `set_ime_romaji_mode_for_target_blocking` を
/// 呼ぶ経路では、`run_open_chain_async`（async chain の中、`spawn_local` 経由）に
/// いてもエンジンスレッドを同期的に、かつ `RUNTIME` borrow を握ったままブロック
/// しうる（round-2 premortem で発見: D を非同期化しても ImmCross が `Failed` を
/// 返しここへフォールスルーする限りこの露出は残る）。
///
/// **完全な修正（この呼び出しを `offload` で本当にワーカースレッドへ追い出す）は
/// 意図的にここでは行っていない。** `apply_mechanism` が受け取る
/// `&ImeControlView<'_>` は `Runtime` から借用したライフタイム付きの値であり、
/// これを `with_app` の外へ持ち出すには「捕獲した hwnd/focus_gen だけを渡して
/// ブロッキング write だけを外に出し、その後に別の view で strategy apply を
/// 再実行する」という 2 段階へ分割する必要がある。これは view1（捕獲時点）と
/// view2（write 完了後の strategy apply 時点）の間に新しい spawn-to-apply の
/// 窓を作る変更であり、E（`romaji_pre_write` の hwnd 解決統一、タスク #108）が
/// 実機ソークを前提に慎重に対処しようとしている問題そのもの。ソーク無しに
/// ここだけ先走って分割すると、BUG-34 追補と同型の「fence 無しの新しい race」を
/// 作り込む恐れがあるため、#108 の前提条件が揃うまで見送る。
///
/// **代わりに今回入れた緩和策**: `romaji_pre_write` 自体
/// （`ime_controller.rs`）が Step0-c の `SendHealth::blocking_allowed` を見て
/// おり、直近で slow 判定が出た後の cooldown 期間中はこの書き込みを発行しない
/// （degrade）。加えて `with_app_or_repost_with`（Step0-b）により、この関数が
/// `RUNTIME` borrow を握ってブロックしている間に他の完了メッセージ
/// （`WM_ASYNC_IME_APPLY_COMPLETE` 等）が届いても、以前のように黙って
/// 捨てられることはなく、次のメッセージループ周回で確実に再処理される。
/// 「初回のブロックそのもの」は残るが、「ブロック中に他の完了が永久に
/// 失われる」という round-2 の指摘した最悪の帰結は防げている。
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
pub(crate) async fn run_open_chain_async(order: ActuationOrder, imm: ImmCrossOp) -> ImeOpenOutcome {
    // ADR-090 §2.A A-1: 授権は起案側（`ImeStateHub::issue_actuation_order`）で
    // 発行済み。**shadow モード**なので授権が下りていなくても書き込みは
    // 止めない（止めるのは A-2）。
    //
    // なお `order` は起案時点の状態に基づくのに write は完了時点で起きる。
    // await をまたいだ失効の扱いは warrant ではなく**チェーンの再抽選**
    // （ADR-090 項 D、実機ソーク必須のため未実装）で行う。
    crate::ime_controller::log_shadow_warrant("async", &order);
    let actuation = order.into_actuation_shadow().verify(imm.verified_target());
    let mut writer = AsyncChainWriter { imm: Some(imm) };
    actuation
        .run_chain_async(&WriteMechanism::ALL, &mut writer)
        .await
}
