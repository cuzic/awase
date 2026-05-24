//! IME ON/OFF 制御の Strategy パターン実装。
//!
//! `WindowsPlatform::apply_ime_open` の内部メカニズム選択ロジックを
//! `ImeController` + `ImeOpenStrategy` に分離する。
//!
//! # 戦略リスト（優先順）
//! 1. `ImmCrossProcessStrategy` — IMM-bridge が生きているウィンドウ向け（Imm32Unavailable は skip）
//! 2. `KanjiToggleStrategy`     — 汎用フォールバック（Imm32Unavailable の主戦略 + ImmCross 失敗時の fallback）
//!
//! `ImmCrossProcessStrategy` が `Failed` を返した場合（例: `SendMessageTimeout` タイムアウト）、
//! `ImeController` は `KanjiToggleStrategy` へフォールスルーする。
//!
//! ## アーキテクチャ制約
//! このモジュールは観測値を自ら読んではいけない。
//! すべての観測値は `ImeControlView` 経由で受け取ること。
//! `crate::tsf::observer::tsf_obs()` の直接呼び出し禁止（スナップショット経由で受け取ること）。

use awase::platform::ImeOpenOutcome;

use crate::state::ime_decision_view::ImeControlView;

/// IME ON/OFF を実行する戦略インターフェース。
pub(crate) trait ImeOpenStrategy: Sync {
    /// このコンテキストで戦略が有効かどうか。
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool;
    /// IME を指定状態に設定しその結果を返す。
    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome;
}

// ── ImmCrossProcessStrategy ──────────────────────────────────────

/// `ImmSetOpenStatus`（cross-process）を使う標準戦略。
///
/// IMM-bridge が機能しているウィンドウにのみ適用可能。
pub(crate) struct ImmCrossProcessStrategy;

impl ImeOpenStrategy for ImmCrossProcessStrategy {
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool {
        view.focus.profile.can_use_imm32_cross_process()
    }

    fn apply(&self, open: bool, _view: &ImeControlView<'_>) -> ImeOpenOutcome {
        if unsafe { crate::ime::set_ime_open_cross_process(open) } {
            ImeOpenOutcome::Applied
        } else {
            ImeOpenOutcome::Failed
        }
    }
}

// ── KanjiToggleStrategy ──────────────────────────────────────────

/// `SendInput(VK_KANJI)` トグルを使うフォールバック戦略。
///
/// Chrome 等 Imm32Unavailable クラスでの主戦略、および `ImmCrossProcessStrategy` が
/// タイムアウト失敗した際の汎用フォールバックとして機能する。
/// VK_KANJI はトグルキーのため shadow と目標が一致している場合は送信をスキップする。
pub(crate) struct KanjiToggleStrategy;

impl ImeOpenStrategy for KanjiToggleStrategy {
    fn is_applicable(&self, _view: &ImeControlView<'_>) -> bool {
        true // 汎用フォールバック: Imm32Unavailable の主戦略 + ImmCross 失敗時の代替
    }

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        // 候補ウィンドウが表示中 → Chrome/Edge の IME は確実に ON。
        // shadow が desync で false になっていても強制送信して desync を修復する。
        let effective_shadow = view.control.shadow_on || view.observed.candidate_visible;

        if effective_shadow == open {
            log::debug!(
                "[apply-ime] shadow={} candidate={} already matches desired={open}, skip VK_KANJI",
                view.control.shadow_on, view.observed.candidate_visible
            );
            ImeOpenOutcome::AlreadyMatched
        } else {
            log::debug!(
                "[apply-ime] shadow={} candidate={} → desired={open}: SendInput VK_KANJI",
                view.control.shadow_on, view.observed.candidate_visible
            );
            unsafe { crate::ime::post_kanji_toggle_to_focused(view.observed.candidate_visible) };
            ImeOpenOutcome::FallbackSent
        }
    }
}

// ── ImeController ────────────────────────────────────────────────

/// 戦略リストを走査して最初の有効な戦略を選択・実行するコントローラ。
pub(crate) struct ImeController {
    strategies: [&'static dyn ImeOpenStrategy; 2],
}

static IMM_STRATEGY: ImmCrossProcessStrategy = ImmCrossProcessStrategy;
static KANJI_STRATEGY: KanjiToggleStrategy = KanjiToggleStrategy;

impl ImeController {
    pub(crate) const fn new() -> Self {
        Self {
            strategies: [&IMM_STRATEGY, &KANJI_STRATEGY],
        }
    }

    /// コンテキストに応じた戦略を選択して IME を設定する。
    ///
    /// 戦略が `Failed` を返した場合（例: `ImmCrossProcessStrategy` の `SendMessageTimeout` タイムアウト）、
    /// 次の適用可能な戦略にフォールスルーする。
    pub(crate) fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        for strategy in self.strategies {
            if strategy.is_applicable(view) {
                let outcome = strategy.apply(open, view);
                if outcome != ImeOpenOutcome::Failed {
                    return outcome;
                }
                log::debug!("[apply-ime] strategy failed, trying next fallback");
            }
        }
        log::warn!("[apply-ime] all strategies failed for class={}", view.focus.class_name);
        ImeOpenOutcome::Failed
    }
}
