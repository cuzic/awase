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

// ── Observation → Belief reduction ───────────────────────────────

/// [`reduce_apply_belief`] へ渡す観測値の集約。
///
/// 呼び出し元が収集できる全観測値をここにまとめる。
/// planner / strategy は自らこれらの値を読まない（テスト可能性のため）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ApplyBeliefInputs {
    // ── 指令状態 ──
    pub shadow_on: bool,
    pub applied: crate::state::AppliedImeState,
    // ── OS イベント観測 ──
    pub candidate_visible: bool,
    pub candidate_was_seen: bool,
    pub gji_monitor_ok: bool,
    // ── OS 直接読み取り（任意） ──
    pub conv_mode: Option<u32>,
    // ── コンテキスト ──
    pub can_imm32_cross_process: bool,
    pub is_engine_intent: bool,
    pub now_ms: u64,
}

/// 複数の観測値を 1 つの「適用時 IME 状態ビリーフ」に純粋関数で還元する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ApplyBelief {
    /// 現在の IME open 状態の推定値。
    pub effective_open: bool,
    /// 推定に十分な確信があるか。`false` の場合は already_matched を強制 false にする。
    pub confident: bool,
}

impl ApplyBelief {
    /// shadow のみから自明なビリーフを作る（後方互換ラッパー用）。
    pub(crate) fn from_shadow(shadow_on: bool) -> Self {
        Self { effective_open: shadow_on, confident: true }
    }
}

/// 観測値を純粋に還元して `ApplyBelief` を返す。
///
/// # effective_open の計算
/// `conv_mode` が取得できた場合はそれを ground-truth として使用する（conv=0 → DirectInput=false）。
/// 取得できない場合は shadow_on + candidate 観測で推定する。
///
/// # confident の計算
/// EngineIntent かつ ImmCross/GJI で確認できない環境（KanjiToggle 系）でのみ
/// `safely_confirmed` を検査する。それ以外は常に `true`。
/// `confident=false` は「already_matched を強制 false」つまり「必ず apply する」を意味する。
pub(crate) fn reduce_apply_belief(inputs: &ApplyBeliefInputs, desired_open: bool) -> ApplyBelief {
    let effective_open = if let Some(conv) = inputs.conv_mode {
        conv != 0
    } else {
        inputs.shadow_on
            || inputs.candidate_visible
            || (!desired_open && inputs.candidate_was_seen)
    };

    let confident = if inputs.is_engine_intent
        && !inputs.can_imm32_cross_process
        && !inputs.gji_monitor_ok
        && inputs.conv_mode.is_none()
    {
        // KanjiToggle 系（Chrome/TsfNative 等）: Confirmed かつ shadow 一致 かつ 300ms 以内のみ確信あり
        inputs.shadow_on == desired_open
            && inputs.applied.is_confirmed()
            && inputs.now_ms.saturating_sub(inputs.applied.confirmed_at_ms()) < 300
    } else {
        true
    };

    ApplyBelief { effective_open, confident }
}

// ─────────────────────────────────────────────────────────────────

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
    /// ここが `already_matched` の **唯一の計算場所**。Strategy は再チェックしない。
    ///
    /// - `belief.confident=false`: 必ず `already_matched=false`（強制 apply）。
    ///   KanjiToggle 系で desync の疑いがある場合（[`reduce_apply_belief`] で判定）に設定される。
    /// - GJI: `open=true && belief.effective_open=true` のみ `AlreadyMatched`。
    ///   `GjiDirectStrategy` は `open=false` では shadow に関わらず VK_IME_OFF（冪等）を送る。
    /// - KanjiToggle / MS-IME: `belief.effective_open == open` で `AlreadyMatched`。
    ///   `effective_open` は `reduce_apply_belief` が candidate_visible / candidate_was_seen を
    ///   加味して計算済み。
    pub(crate) fn from_view(
        view: &crate::state::ImeControlView<'_>,
        open: bool,
        probe_in_flight: bool,
        belief: ApplyBelief,
    ) -> Self {
        let already_matched = if !belief.confident {
            false
        } else {
            match view.observed.active_ime_kind {
                ActiveImeKind::GoogleJapaneseInput => {
                    // GJI は open=false では AlreadyMatched にしない（VK_IME_OFF は冪等なので常に送る）。
                    open && belief.effective_open
                }
                _ => belief.effective_open == open,
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
