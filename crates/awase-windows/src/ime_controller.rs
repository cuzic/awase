//! IME ON/OFF 制御の Strategy パターン実装。
//!
//! `WindowsPlatform::apply_ime_open` の内部メカニズム選択ロジックを
//! `ImeController` + `ImeOpenStrategy` に分離する。
//!
//! # 戦略リスト（優先順）
//! 1. `ImmCrossProcessStrategy` — IMM-bridge が生きているウィンドウ向け（Imm32Unavailable は skip）
//! 2. `GjiDirectStrategy`       — GJI 検出済み時の一方向制御（F13/Ctrl+Shift+Delete、shadow desync 耐性あり）
//! 3. `KanjiToggleStrategy`     — 汎用フォールバック（非 GJI の Imm32Unavailable + ImmCross 失敗時）
//!
//! `ImmCrossProcessStrategy` が `Failed` を返した場合（例: `SendMessageTimeout` タイムアウト）、
//! `ImeController` は次の適用可能な戦略へフォールスルーする。
//! GJI が検出されている場合は `GjiDirectStrategy` が `KanjiToggleStrategy` より先に選択される。
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

// ── GjiDirectStrategy ────────────────────────────────────────────

/// GJI 専用の一方向 IME 制御戦略。
///
/// VK_KANJI（トグル）の代わりに GJI 固有のキーを使うことで shadow desync の影響を排除する:
/// - ON  → F13（DirectInput 時にひらがなへ切り替え、既に ON なら no-op）
/// - OFF → F14（Precomposition/Composition/Conversion 時に IME OFF）
///
/// F13/F14 は実キーボードに存在しないためブラウザショートカットと衝突しない。
/// GJI の config1.db に以下を登録することで有効になる:
///   `DirectInput\tF13\tIMEOn`（デフォルト登録済み）
///   `Precomposition\tF14\tIMEOff`
///   `Composition\tF14\tIMEOff`
///   `Conversion\tF14\tIMEOff`
///
/// `gji_monitor_ok=true`（GJI プロセス検出済み）の場合のみ適用可能。
pub(crate) struct GjiDirectStrategy;

impl ImeOpenStrategy for GjiDirectStrategy {
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool {
        view.focus.profile.uses_kanji_toggle() && view.observed.gji_monitor_ok
    }

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        if open {
            if view.control.shadow_on {
                // shadow が ON を示しており F13 は no-op と見込まれるためスキップ
                log::debug!("[apply-ime] GJI direct: shadow ON, skip F13");
                return ImeOpenOutcome::AlreadyMatched;
            }
            log::debug!("[apply-ime] GJI direct: F13 (IME ON)");
            unsafe { crate::ime::post_gji_ime_on() };
        } else {
            // shadow が既に OFF なら F14 を送信しない（F14 は DirectInput でパススルーされるが
            // 無害なキーのため必須ではない。送信省略で効率化）。
            let effective_shadow = view.control.shadow_on
                || view.observed.candidate_visible
                || view.observed.candidate_was_seen;
            if !effective_shadow {
                log::debug!("[apply-ime] GJI direct: shadow OFF, skip F14");
                return ImeOpenOutcome::AlreadyMatched;
            }
            log::debug!(
                "[apply-ime] GJI direct: F14 (IME OFF, candidate={})",
                view.observed.candidate_visible
            );
            unsafe { crate::ime::post_gji_ime_off(view.observed.candidate_visible) };
        }
        ImeOpenOutcome::Applied
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
        //
        // candidate_was_seen: VK_KANJI が誤って OFF→ON トグルした場合の desync 検出ラッチ。
        // 例: 新タブ(IME実態=OFF)でshadow=true(ステール) → VK_KANJI → 実態ON, shadow=false
        //     → GJI candidate SHOW → candidate_was_seen=true
        //     → 次の apply_ime_open(false) で shadow=false でも VK_KANJI を送れるようにする。
        let effective_shadow = view.control.shadow_on
            || view.observed.candidate_visible
            || view.observed.candidate_was_seen;

        if effective_shadow == open {
            log::debug!(
                "[apply-ime] shadow={} candidate={} was_seen={} already matches desired={open}, skip VK_KANJI",
                view.control.shadow_on, view.observed.candidate_visible, view.observed.candidate_was_seen
            );
            ImeOpenOutcome::AlreadyMatched
        } else {
            log::debug!(
                "[apply-ime] shadow={} candidate={} was_seen={} → desired={open}: SendInput VK_KANJI",
                view.control.shadow_on, view.observed.candidate_visible, view.observed.candidate_was_seen
            );
            unsafe { crate::ime::post_kanji_toggle_to_focused(view.observed.candidate_visible) };
            ImeOpenOutcome::FallbackSent
        }
    }
}

// ── ImeController ────────────────────────────────────────────────

/// 戦略リストを走査して最初の有効な戦略を選択・実行するコントローラ。
pub(crate) struct ImeController {
    strategies: [&'static dyn ImeOpenStrategy; 3],
}

static IMM_STRATEGY: ImmCrossProcessStrategy = ImmCrossProcessStrategy;
static GJI_STRATEGY: GjiDirectStrategy = GjiDirectStrategy;
static KANJI_STRATEGY: KanjiToggleStrategy = KanjiToggleStrategy;

impl ImeController {
    pub(crate) const fn new() -> Self {
        Self {
            strategies: [&IMM_STRATEGY, &GJI_STRATEGY, &KANJI_STRATEGY],
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
