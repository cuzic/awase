//! IME ON/OFF 制御の Strategy パターン実装。
//!
//! `WindowsPlatform::apply_ime_open` の内部メカニズム選択ロジックを
//! `ImeController` + `ImeOpenStrategy` に分離する。
//!
//! # 戦略リスト（優先順）
//! 1. `ImmCrossProcessStrategy` — IMM-bridge が生きているウィンドウ向け
//! 2. `KanjiToggleStrategy`     — IMM-broken クラス（Chrome 等）向け

use awase::platform::ImeOpenOutcome;

/// 戦略が使用するフォーカス情報と現在の IME 状態。
pub struct ImeApplyContext<'a> {
    /// フォーカスウィンドウのクラス名
    pub class_name: &'a str,
    /// 現在の shadow IME ON 状態（`ImeBeliefStore::assume_or_false()`）
    pub shadow_on: bool,
}

/// IME ON/OFF を実行する戦略インターフェース。
pub trait ImeOpenStrategy: Sync {
    /// このコンテキストで戦略が有効かどうか。
    fn is_applicable(&self, ctx: &ImeApplyContext<'_>) -> bool;
    /// IME を指定状態に設定しその結果を返す。
    fn apply(&self, open: bool, ctx: &ImeApplyContext<'_>) -> ImeOpenOutcome;
}

// ── ImmCrossProcessStrategy ──────────────────────────────────────

/// `ImmSetOpenStatus`（cross-process）を使う標準戦略。
///
/// IMM-bridge が機能しているウィンドウにのみ適用可能。
pub struct ImmCrossProcessStrategy;

impl ImeOpenStrategy for ImmCrossProcessStrategy {
    fn is_applicable(&self, ctx: &ImeApplyContext<'_>) -> bool {
        !crate::focus::classify::is_imm_bridge_broken(ctx.class_name)
    }

    fn apply(&self, open: bool, _ctx: &ImeApplyContext<'_>) -> ImeOpenOutcome {
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
/// Chrome 等 IMM-broken クラス向け。VK_KANJI はトグルキーのため
/// shadow と目標が一致している場合は送信をスキップする。
pub struct KanjiToggleStrategy;

impl ImeOpenStrategy for KanjiToggleStrategy {
    fn is_applicable(&self, ctx: &ImeApplyContext<'_>) -> bool {
        crate::focus::classify::is_imm_bridge_broken(ctx.class_name)
    }

    fn apply(&self, open: bool, ctx: &ImeApplyContext<'_>) -> ImeOpenOutcome {
        if ctx.shadow_on != open {
            log::debug!("[apply-ime] shadow={} → desired={open}: SendInput VK_KANJI", ctx.shadow_on);
            unsafe { crate::ime::post_kanji_toggle_to_focused() };
            ImeOpenOutcome::FallbackSent
        } else {
            log::debug!("[apply-ime] shadow={} already matches desired={open}, skip VK_KANJI", ctx.shadow_on);
            ImeOpenOutcome::AlreadyMatched
        }
    }
}

// ── ImeController ────────────────────────────────────────────────

/// 戦略リストを走査して最初の有効な戦略を選択・実行するコントローラ。
pub struct ImeController {
    strategies: [&'static dyn ImeOpenStrategy; 2],
}

static IMM_STRATEGY: ImmCrossProcessStrategy = ImmCrossProcessStrategy;
static KANJI_STRATEGY: KanjiToggleStrategy = KanjiToggleStrategy;

impl ImeController {
    pub const fn new() -> Self {
        Self {
            strategies: [&IMM_STRATEGY, &KANJI_STRATEGY],
        }
    }

    /// コンテキストに応じた戦略を選択して IME を設定する。
    pub fn apply(&self, open: bool, ctx: &ImeApplyContext<'_>) -> ImeOpenOutcome {
        for strategy in self.strategies {
            if strategy.is_applicable(ctx) {
                return strategy.apply(open, ctx);
            }
        }
        log::warn!("[apply-ime] no applicable strategy for class={}", ctx.class_name);
        ImeOpenOutcome::Failed
    }
}
