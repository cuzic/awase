//! IME ウォームアップ戦略の抽象レイヤー。
//!
//! GJI（`GjiFsm`）と MS IME（`MsImeStrategy`）のプローブ動作の差異を隠蔽する。
//! GJI は cold-start probe 機構を持ち、MS IME は常に warm を返す。
//!
//! ## 設計
//!
//! [`ImeWarmupStrategy`] はプローブのライフサイクルと GJI イベント処理を抽象化する。
//! GJI 固有のイベント（`GjiEvent::ImeOn`、`FocusChange` 等）は依然 [`crate::tsf::gji_fsm::GjiFsm`]
//! が直接処理する。このトレイトは `Output` が MS IME 対応を追加する際の足場として用意する。

use timed_fsm::Response;
use crate::tsf::gji_fsm::{FocusEpoch, GjiAction, GjiEvent, GjiTimer, ProbeParams};

// ── トレイト定義 ──────────────────────────────────────────────────────────────

/// IME ウォームアップ戦略の共通インターフェース。
///
/// - [`crate::tsf::gji_fsm::GjiFsm`] が実装する（cold-start probe 機構付き）。
/// - [`MsImeStrategy`] が実装する（常に warm、probe なし）。
pub(crate) trait ImeWarmupStrategy {
    /// IME/TSF が現在 warm（ローマ字を即送信できる）かどうか。
    fn is_warm(&self) -> bool;

    /// `Authorized` 状態の場合に probe パラメータを返す。
    ///
    /// `None` は「probe 不要」を意味する（`NotStarted` 状態、または MS IME）。
    fn current_probe_params(&self) -> Option<ProbeParams>;

    /// GJI イベントを処理し `Response<GjiAction, GjiTimer>` を返す。
    ///
    /// MS IME では GJI が存在しないため `Response::consume()` を返す（デフォルト）。
    fn on_gji_event(
        &mut self,
        event: GjiEvent,
    ) -> Response<GjiAction, GjiTimer> {
        let _ = event;
        Response::consume()
    }

    /// GJI LongIdle タイムアウトを処理する。
    ///
    /// MS IME では no-op（デフォルト）。
    fn on_gji_long_idle(&mut self) -> Response<GjiAction, GjiTimer> {
        Response::consume()
    }

    /// OnComposing 状態の epoch を返す（EndComposition に使用）。
    ///
    /// MS IME / GJI が OnComposing でない場合は `None`（デフォルト）。
    fn gji_current_composition_epoch(&self) -> Option<FocusEpoch> {
        None
    }

    /// 次の `KeyInput` が long-cold（≥10s idle）の最初のキーか（Unicode cold defer 判定用）。
    ///
    /// MS IME は常に warm なので `false`（デフォルト）。
    fn is_next_key_long_cold(&self) -> bool {
        false
    }

}

// ── GjiFsm 実装 ───────────────────────────────────────────────────────────────

impl ImeWarmupStrategy for crate::tsf::gji_fsm::GjiFsm {
    fn is_warm(&self) -> bool {
        use crate::tsf::gji_fsm::GjiState;
        matches!(self.state(), GjiState::OnWarm { .. } | GjiState::OnComposing { .. })
    }

    fn current_probe_params(&self) -> Option<ProbeParams> {
        crate::tsf::gji_fsm::GjiFsm::current_probe_params(self)
    }

    fn on_gji_event(
        &mut self,
        event: GjiEvent,
    ) -> Response<GjiAction, GjiTimer> {
        use timed_fsm::TimedStateMachine as _;
        crate::tsf::gji_fsm::GjiFsm::on_event(self, event)
    }

    fn on_gji_long_idle(&mut self) -> Response<GjiAction, GjiTimer> {
        use timed_fsm::TimedStateMachine as _;
        crate::tsf::gji_fsm::GjiFsm::on_timeout(self, GjiTimer::LongIdle)
    }

    fn gji_current_composition_epoch(&self) -> Option<FocusEpoch> {
        use crate::tsf::gji_fsm::GjiState;
        match crate::tsf::gji_fsm::GjiFsm::state(self) {
            GjiState::OnComposing { epoch, .. } => Some(*epoch),
            _ => None,
        }
    }

    fn is_next_key_long_cold(&self) -> bool {
        crate::tsf::gji_fsm::GjiFsm::is_next_key_long_cold(self)
    }

}

// ── MsImeStrategy ─────────────────────────────────────────────────────────────

/// MS IME 向けウォームアップ戦略。
///
/// MS IME は TSF context が常にウォームで、GJI のような外部プローブが不要。
/// `is_warm()` は常に `true`、probe 操作は全て no-op となる。
/// `Output::set_active_ime_kind` が MS-IME 検出時に `GjiFsm` と差し替える。
pub(crate) struct MsImeStrategy;

impl ImeWarmupStrategy for MsImeStrategy {
    fn is_warm(&self) -> bool {
        true
    }

    fn current_probe_params(&self) -> Option<ProbeParams> {
        None
    }
    // on_gji_event, on_gji_long_idle, gji_current_composition_epoch はすべてデフォルト実装を使用する。
}
