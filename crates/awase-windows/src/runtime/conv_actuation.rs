//! `Runtime::actuate_conv_mode` — `Output::actuate_conv_mode` への 1 行 delegate。
//!
//! 実体（ADR-084 P1/INV-1 の単一窓口、ADR-086 INV-14 のターゲット同一性）は
//! [`crate::output::conv_actuation`] へ移設済み（2026-08-08、ADR-086 Phase 2 設計調査 —
//! `actuate_conv_mode` の同期部が `Runtime` の状態を一切読んでいなかったため）。
//! この delegate を残す理由は2つ: (1) ADR-084 INV-1 が「`Runtime::actuate_conv_mode`」
//! という関数名を指しているため、(2) 既存呼び出し元
//! `runtime/key_pipeline.rs::kp_shift_conv_guard_key_down` を触らずに済むため。
//!
//! module doc の詳細（移行済み経路一覧・INV-1 未達箇所・未移行経路）は
//! [`crate::output::conv_actuation`] を SSOT とする（本ファイルは参照しない）。

use super::Runtime;
use crate::state::{ConvActuationOutcome, ConvModeTarget, ConvMutationReason, TickMs};

impl Runtime {
    /// conv-mode を変更する唯一の窓口。詳細は [`crate::output::conv_actuation`] を参照。
    #[tracing::instrument(level = "debug", skip_all, fields(?target, ?reason))]
    pub(crate) fn actuate_conv_mode(
        &self,
        target: ConvModeTarget,
        reason: ConvMutationReason,
        tick_ms: TickMs,
    ) -> ConvActuationOutcome {
        self.platform
            .output
            .actuate_conv_mode(target, reason, tick_ms)
    }
}
