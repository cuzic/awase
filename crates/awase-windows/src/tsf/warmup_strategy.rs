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
use crate::tsf::probe_fsm::{ProbeAction, TsfEnvSnapshot, TsfProbeMachine};

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
    /// `None` は「probe 不要」を意味する（`Executing`・`NotStarted` 状態、または MS IME）。
    fn current_probe_params(&self) -> Option<ProbeParams>;

    /// `Authorized → Executing`: probe machine をインストールする。
    fn install_probe_machine(&mut self, machine: Box<TsfProbeMachine>);

    /// probe machine を 1 ステップ進め、アクションと machine を返す。
    ///
    /// `None` は「probe 未実行」（`Executing` 状態でない、または MS IME）。
    fn tick_probe_machine(
        &mut self,
        tick_t: u64,
        env: &TsfEnvSnapshot,
    ) -> Option<(Vec<ProbeAction>, Box<TsfProbeMachine>)>;

    /// tick 後に probe が継続する場合 machine を戻す。
    fn restore_probe_machine(&mut self, machine: Box<TsfProbeMachine>);

    /// 実行中の probe をキャンセルし machine を drop する。
    fn cancel_probe(&mut self);

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

    /// GJI probe が Executing 状態かどうかを返す（タイマー継続判定に使用）。
    ///
    /// MS IME は常に warm でプローブを持たないため `false`（デフォルト）。
    fn is_gji_probe_executing(&self) -> bool {
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

    fn install_probe_machine(&mut self, machine: Box<TsfProbeMachine>) {
        crate::tsf::gji_fsm::GjiFsm::install_probe_machine(self, machine);
    }

    fn tick_probe_machine(
        &mut self,
        tick_t: u64,
        env: &TsfEnvSnapshot,
    ) -> Option<(Vec<ProbeAction>, Box<TsfProbeMachine>)> {
        crate::tsf::gji_fsm::GjiFsm::tick_probe_machine(self, tick_t, env)
    }

    fn restore_probe_machine(&mut self, machine: Box<TsfProbeMachine>) {
        crate::tsf::gji_fsm::GjiFsm::restore_probe_machine(self, machine);
    }

    fn cancel_probe(&mut self) {
        let _ = crate::tsf::gji_fsm::GjiFsm::take_probe_machine(self);
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
            GjiState::OnComposing { epoch } => Some(*epoch),
            _ => None,
        }
    }

    fn is_gji_probe_executing(&self) -> bool {
        use crate::tsf::gji_fsm::{GjiState, ProbeStatus};
        matches!(
            crate::tsf::gji_fsm::GjiFsm::state(self),
            GjiState::OnCold { probe: ProbeStatus::Executing { .. }, .. }
        )
    }
}

// ── MsImeStrategy ─────────────────────────────────────────────────────────────

/// MS IME 向けウォームアップ戦略。
///
/// MS IME は TSF context が常にウォームで、GJI のような外部プローブが不要。
/// `is_warm()` は常に `true`、probe 操作は全て no-op となる。
#[allow(dead_code)]
pub(crate) struct MsImeStrategy;

impl ImeWarmupStrategy for MsImeStrategy {
    fn is_warm(&self) -> bool {
        true
    }

    fn current_probe_params(&self) -> Option<ProbeParams> {
        None
    }

    fn install_probe_machine(&mut self, machine: Box<TsfProbeMachine>) {
        drop(machine);
        log::warn!("[ms-ime-strategy] install_probe_machine: unexpected call, machine dropped");
    }

    fn tick_probe_machine(
        &mut self,
        _tick_t: u64,
        _env: &TsfEnvSnapshot,
    ) -> Option<(Vec<ProbeAction>, Box<TsfProbeMachine>)> {
        None
    }

    fn restore_probe_machine(&mut self, machine: Box<TsfProbeMachine>) {
        drop(machine);
        log::warn!("[ms-ime-strategy] restore_probe_machine: unexpected call, machine dropped");
    }

    fn cancel_probe(&mut self) {
        // MS IME はプローブを持たない。
    }
    // on_gji_event, on_gji_long_idle, gji_current_composition_epoch, is_gji_probe_executing
    // はすべてデフォルト実装を使用する。
}
