//! IME ON/OFF 適用の「計画」と「結果」を型化する data-model 層。
//!
//! 実行は既存の [`crate::ime_controller`] Strategy 群が担うが、このモジュールは
//! 「どの機構で適用するか（[`ImeApplyPlan`]）」を副作用なしで決定し、
//! 実行後の [`awase::platform::ImeOpenOutcome`] を意味のある [`ImeApplyResult`] に
//! 正規化する。plan と execute を分離することでテスト可能性を高める。
//!
//! TODO(H-4/H-5): 現状 planner の決定は ime_controller の Strategy 選択と並走する
//! 純粋関数。将来的に `ImeController::apply` を `plan()` → `execute(plan)` の
//! 2 段構成へ移行し、この planner を SSOT にする。

use awase::platform::ImeOpenOutcome;
use crate::tsf::observer::ActiveImeKind;

/// IME を目標状態へ移すために選択された適用機構。
///
/// [`crate::ime_controller`] の戦略優先順（ImmCross → GjiDirect → MsImeDirect →
/// KanjiToggle）に対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImeApplyPlan {
    /// 既に目標状態のため何も送らない（`AlreadyMatched` 相当）。
    Noop,
    /// MS-IME 環境で冪等な VK_DBE_HIRAGANA / ALPHANUMERIC を送る。
    SendVkDbeHiragana,
    /// IME 種別不明環境のフォールバック。非冪等な VK_KANJI トグルを送る。
    SendKanjiToggle,
    /// IMM クロスプロセス / GJI direct で TSF compartment を開閉する。
    UseTsfCompartment,
    /// GJI プローブ実行中のため適用を保留する。
    DeferUntilProbe,
}

/// IME 適用の正規化された結果。
///
/// [`ImeOpenOutcome`] を「確定 / 未確定送信済み / 失敗 / 保留 / 陳腐化」に写像する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImeApplyResult {
    /// IMM 経由等で目標状態が確定した（`Applied` / `AlreadyMatched`）。
    Confirmed,
    /// フォールバックを送信したが OS 処理完了まで不確定（`FallbackSent`）。
    SentUnverified,
    /// 適用に失敗した（`Failed`、非日本語環境等）。
    Failed,
    /// プローブ保留等で適用しなかった。
    Deferred,
    /// shadow が stale でトグルが unsafe のため送信を見送った（`UnsafeToToggle`）。
    Stale,
}

impl ImeApplyResult {
    /// 実行後の [`ImeOpenOutcome`] を正規化する。
    #[must_use]
    pub(crate) fn from_outcome(outcome: ImeOpenOutcome) -> Self {
        match outcome {
            ImeOpenOutcome::Applied | ImeOpenOutcome::AlreadyMatched => Self::Confirmed,
            ImeOpenOutcome::FallbackSent => Self::SentUnverified,
            ImeOpenOutcome::Failed => Self::Failed,
            ImeOpenOutcome::UnsafeToToggle => Self::Stale,
        }
    }

    /// shadow model / applied_snapshot を更新してよい結果か。
    ///
    /// `Deferred` / `Stale` は適用が実行されていないため状態を進めてはいけない。
    #[must_use]
    pub(crate) fn should_commit_state(self) -> bool {
        matches!(self, Self::Confirmed | Self::SentUnverified)
    }
}

/// [`ImeApplyPlanner::plan`] の入力。観測値はここに集約して渡す（planner は自ら読まない）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImeApplyContext {
    /// shadow が既に目標 open 状態か。
    pub already_matched: bool,
    /// GJI プローブ実行中か。
    pub probe_in_flight: bool,
    /// IMM クロスプロセス bridge が生きているか（Imm32Unavailable では false）。
    pub imm_cross_available: bool,
    /// 検出済み IME 種別。
    pub active_ime_kind: ActiveImeKind,
    /// アプリが VK_KANJI トグル制御を要するか（Chrome/Edge 等）。
    pub uses_kanji_toggle: bool,
}

impl ImeApplyContext {
    /// [`crate::state::ImeControlView`] からコンテキストを構築する。
    ///
    /// - `open`: 目標 IME 開閉状態（`already_matched` の計算に必要）。
    /// - `probe_in_flight`: GJI TSF probe が実行中かどうか。
    ///   IME apply 経路では `false` を渡す（開閉適用は probe 中でも即時実行する）。
    ///
    /// ## `already_matched` の計算方針（保守的）
    ///
    /// - GJI: `open=true && shadow_on=true` のみ `AlreadyMatched` とする。
    ///   `GjiDirectStrategy` は `open=false` では shadow に関わらず常に `VK_IME_OFF` を送る。
    /// - その他（MS-IME / KanjiToggle）: `effective_shadow == open` で `AlreadyMatched`。
    ///   `effective_shadow` は `shadow_on | candidate_visible | candidate_was_seen` で、
    ///   `KanjiToggleStrategy` の既存ロジックと一致する。
    pub(crate) fn from_view(
        view: &crate::state::ImeControlView<'_>,
        open: bool,
        probe_in_flight: bool,
    ) -> Self {
        let already_matched = match view.observed.active_ime_kind {
            ActiveImeKind::GoogleJapaneseInput => {
                // GJI は IME OFF（open=false）のとき shadow に関わらず VK_IME_OFF を送るため、
                // already_matched は open=true かつ shadow ON のときのみ true にする。
                open && view.control.shadow_on
            }
            _ => {
                // KanjiToggle / MS-IME: effective_shadow が目標と一致すればスキップ。
                let effective_shadow = view.control.shadow_on
                    || view.observed.candidate_visible
                    || view.observed.candidate_was_seen;
                effective_shadow == open
            }
        };
        Self {
            already_matched,
            probe_in_flight,
            imm_cross_available: view.focus.profile.can_use_imm32_cross_process(),
            active_ime_kind: view.observed.active_ime_kind,
            uses_kanji_toggle: view.focus.profile.uses_kanji_toggle(),
        }
    }
}

/// IME 適用機構を副作用なしで決定する planner。
pub(crate) struct ImeApplyPlanner;

impl ImeApplyPlanner {
    /// 観測コンテキストから適用計画を導出する。
    ///
    /// 戦略優先順（ime_controller 準拠）:
    /// 1. 既に一致 → `Noop`
    /// 2. プローブ実行中 → `DeferUntilProbe`
    /// 3. IMM クロス可 or GJI 検出 → `UseTsfCompartment`
    /// 4. MS-IME 検出 → `SendVkDbeHiragana`
    /// 5. それ以外（VK_KANJI トグルアプリ / 種別不明）→ `SendKanjiToggle`
    #[must_use]
    pub(crate) fn plan(ctx: &ImeApplyContext) -> ImeApplyPlan {
        if ctx.already_matched {
            return ImeApplyPlan::Noop;
        }
        if ctx.probe_in_flight {
            return ImeApplyPlan::DeferUntilProbe;
        }
        match ctx.active_ime_kind {
            ActiveImeKind::GoogleJapaneseInput => ImeApplyPlan::UseTsfCompartment,
            ActiveImeKind::MicrosoftIme if !ctx.uses_kanji_toggle => {
                ImeApplyPlan::SendVkDbeHiragana
            }
            _ if ctx.imm_cross_available && !ctx.uses_kanji_toggle => {
                ImeApplyPlan::UseTsfCompartment
            }
            _ => ImeApplyPlan::SendKanjiToggle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ImeApplyContext {
        ImeApplyContext {
            already_matched: false,
            probe_in_flight: false,
            imm_cross_available: false,
            active_ime_kind: ActiveImeKind::GoogleJapaneseInput,
            uses_kanji_toggle: false,
        }
    }

    #[test]
    fn already_matched_is_noop() {
        let c = ImeApplyContext { already_matched: true, ..ctx() };
        assert_eq!(ImeApplyPlanner::plan(&c), ImeApplyPlan::Noop);
    }

    #[test]
    fn probe_in_flight_defers_before_strategy() {
        let c = ImeApplyContext { probe_in_flight: true, ..ctx() };
        assert_eq!(ImeApplyPlanner::plan(&c), ImeApplyPlan::DeferUntilProbe);
    }

    #[test]
    fn gji_uses_compartment() {
        assert_eq!(ImeApplyPlanner::plan(&ctx()), ImeApplyPlan::UseTsfCompartment);
    }

    #[test]
    fn msime_without_kanji_toggle_sends_dbe() {
        let c = ImeApplyContext {
            active_ime_kind: ActiveImeKind::MicrosoftIme,
            ..ctx()
        };
        assert_eq!(ImeApplyPlanner::plan(&c), ImeApplyPlan::SendVkDbeHiragana);
    }

    #[test]
    fn kanji_toggle_app_falls_back() {
        let c = ImeApplyContext {
            active_ime_kind: ActiveImeKind::MicrosoftIme,
            uses_kanji_toggle: true,
            ..ctx()
        };
        assert_eq!(ImeApplyPlanner::plan(&c), ImeApplyPlan::SendKanjiToggle);
    }

    #[test]
    fn outcome_normalization() {
        assert_eq!(ImeApplyResult::from_outcome(ImeOpenOutcome::Applied), ImeApplyResult::Confirmed);
        assert_eq!(
            ImeApplyResult::from_outcome(ImeOpenOutcome::AlreadyMatched),
            ImeApplyResult::Confirmed
        );
        assert_eq!(
            ImeApplyResult::from_outcome(ImeOpenOutcome::FallbackSent),
            ImeApplyResult::SentUnverified
        );
        assert_eq!(ImeApplyResult::from_outcome(ImeOpenOutcome::Failed), ImeApplyResult::Failed);
        assert_eq!(
            ImeApplyResult::from_outcome(ImeOpenOutcome::UnsafeToToggle),
            ImeApplyResult::Stale
        );
        assert!(ImeApplyResult::Confirmed.should_commit_state());
        assert!(!ImeApplyResult::Deferred.should_commit_state());
        assert!(!ImeApplyResult::Stale.should_commit_state());
    }
}
