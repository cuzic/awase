//! IME ON/OFF 制御の Strategy パターン実装。
//!
//! `WindowsPlatform::apply_ime_open` の内部メカニズム選択ロジックを
//! `ImeController` + `ImeOpenStrategy` に分離する。
//!
//! # 戦略リスト（優先順）
//! 1. `ImmCrossProcessStrategy` — IMM-bridge が生きているウィンドウ向け（Imm32Unavailable は skip）
//! 2. `GjiDirectStrategy`       — GJI 検出済み時の一方向制御（F21/F22）。TsfNative を**除く**全プロファイルで適用
//! 3. `MsImeDirectStrategy`     — MS-IME 環境の TSF アプリ向け（VK_DBE_HIRAGANA/ALPHANUMERIC 冪等制御）
//! 4. `KanjiToggleStrategy`     — 最終フォールバック。IME 種別不明環境および GJI + TsfNative 向け
//!
//! `ImmCrossProcessStrategy` が `Failed` を返した場合（例: `SendMessageTimeout` タイムアウト）、
//! `ImeController` は次の適用可能な戦略へフォールスルーする。
//! GJI が検出されている場合は `GjiDirectStrategy` が TsfNative を除く後続戦略より優先される。
//!
//! ## GJI 前提の設計方針
//! F21/F22 は IME 層で処理されフォアグラウンドアプリのプロファイルに依存しないが、
//! TsfNative（Windows Terminal 等）では F22 が GJI の TSF compartment を閉じず「半角英数」に
//! なるため除外し `KanjiToggleStrategy`（VK_KANJI）にフォールバックする。
//! VK_KANJI は GJI の TSF compartment を正しく閉じるため TsfNative で「直接入力」を達成できる。
//! GJI が起動していない環境（MS-IME 等）では `MsImeDirectStrategy`（冪等 VK_DBE_*）が先行し、
//! IME 種別不明時に限り `KanjiToggleStrategy`（トグル）がフォールバックする。
//!
//! ## アーキテクチャ制約
//! このモジュールは観測値を自ら読んではいけない。
//! すべての観測値は `ImeControlView` 経由で受け取ること。
//! `crate::tsf::observer::tsf_obs()` の直接呼び出し禁止（スナップショット経由で受け取ること）。

use awase::platform::ImeOpenOutcome;

use crate::state::ime_decision_view::ImeControlView;
use crate::tsf::observer::ActiveImeKind;

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

/// GJI を使った一方向 IME 制御戦略。
///
/// VK_KANJI（トグル）の代わりに冪等キーを使うことで shadow desync の影響を排除する:
/// - ON  → VK_IME_ON（IME ON、既に ON なら no-op）
/// - OFF → VK_IME_OFF（IME OFF、既に OFF なら no-op）
///
/// VK_IME_ON/OFF は Windows 標準 IME 制御キーで config1.db バインド不要。
///
/// TsfNative（Windows Terminal 等）では VK_IME_OFF が GJI の TSF compartment を正しく閉じるか
/// 未確認のため TsfNative を除外し KanjiToggleStrategy（VK_KANJI）にフォールバックする。
///
/// 適用条件:
/// - `active_ime_kind == GoogleJapaneseInput` (CLSID ベース判定)
/// - `profile != TsfNative`（TsfNative は VK_KANJI でフォールバック）
pub(crate) struct GjiDirectStrategy;

impl ImeOpenStrategy for GjiDirectStrategy {
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool {
        use crate::focus::class_names::AppImeProfile;
        view.observed.active_ime_kind == ActiveImeKind::GoogleJapaneseInput
            && !matches!(view.focus.profile, AppImeProfile::TsfNative)
    }

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        if open {
            if view.control.shadow_on {
                // shadow が ON を示しており VK_IME_ON は no-op と見込まれるためスキップ
                log::debug!("[apply-ime] GJI direct: shadow ON, skip VK_IME_ON");
                return ImeOpenOutcome::AlreadyMatched;
            }
            log::debug!("[apply-ime] GJI direct: VK_IME_ON");
            unsafe { crate::ime::post_gji_ime_on() };
        } else {
            log::debug!("[apply-ime] GJI direct: VK_IME_OFF");
            unsafe { crate::ime::post_gji_ime_off() };
        }
        ImeOpenOutcome::Applied
    }
}

// ── MsImeDirectStrategy ──────────────────────────────────────────

/// MS-IME 向けの冪等 IME 制御戦略（VK_DBE_HIRAGANA / VK_DBE_ALPHANUMERIC）。
///
/// CLSID ベースで MS-IME（または互換 IME）がアクティブと判定された場合に、
/// IMM32 クロスプロセス制御が使えない TSF アプリ（Chrome / Edge 等）へ冪等な VK_DBE_* を送信する。
///
/// - ON  → `VK_DBE_HIRAGANA` (0xF2) — ひらがなモードに設定（既に ON なら no-op）
/// - OFF → `VK_DBE_ALPHANUMERIC` (0xF0) — 半角英数モードに設定（既に OFF なら no-op）
///
/// 適用条件:
/// - `active_ime_kind == MicrosoftIme` (CLSID ベース判定)
/// - `can_use_imm32_cross_process() == false`（IMM32 が使えない TSF アプリ）
pub(crate) struct MsImeDirectStrategy;

impl ImeOpenStrategy for MsImeDirectStrategy {
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool {
        view.observed.active_ime_kind == ActiveImeKind::MicrosoftIme
            && !view.focus.profile.can_use_imm32_cross_process()
    }

    fn apply(&self, open: bool, _view: &ImeControlView<'_>) -> ImeOpenOutcome {
        if open {
            log::debug!("[apply-ime] MS-IME direct: VK_DBE_HIRAGANA (IME ON)");
            // SAFETY: post_ms_ime_on は Win32 API を呼び出す unsafe fn。メインスレッドから呼ぶこと。
            unsafe { crate::ime::post_ms_ime_on() };
        } else {
            log::debug!("[apply-ime] MS-IME direct: VK_DBE_ALPHANUMERIC (IME OFF)");
            // SAFETY: post_ms_ime_off は Win32 API を呼び出す unsafe fn。メインスレッドから呼ぶこと。
            unsafe { crate::ime::post_ms_ime_off() };
        }
        ImeOpenOutcome::Applied
    }
}

// ── KanjiToggleStrategy ──────────────────────────────────────────

/// `SendInput(VK_KANJI)` トグルを使う最終フォールバック戦略。
///
/// IME 種別が不明な環境での全プロファイル共通フォールバック。
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
            // 候補表示中は VK_KANJI が候補窓に吸われて IME OFF に失敗する可能性があるが、
            // Ctrl+Enter で事前に候補確定する方式は Chrome フォームを送信してしまうため廃止。
            // GJI 環境では GjiDirectStrategy が先行して処理するため、ここには GJI 以外か
            // GJI 失敗時のみ到達する。VK_KANJI をそのまま送り、稀に候補窓に吸われる場合は許容する。
            log::debug!(
                "[apply-ime] shadow={} candidate={} was_seen={} profile={:?} → desired={open}: SendInput VK_KANJI",
                view.control.shadow_on, view.observed.candidate_visible, view.observed.candidate_was_seen,
                view.focus.profile,
            );
            unsafe { crate::ime::post_kanji_toggle_to_focused() };
            ImeOpenOutcome::FallbackSent
        }
    }
}

// ── ImeController ────────────────────────────────────────────────

/// 戦略リストを走査して最初の有効な戦略を選択・実行するコントローラ。
pub(crate) struct ImeController {
    strategies: [&'static dyn ImeOpenStrategy; 4],
}

static IMM_STRATEGY: ImmCrossProcessStrategy = ImmCrossProcessStrategy;
static GJI_STRATEGY: GjiDirectStrategy = GjiDirectStrategy;
static MS_IME_STRATEGY: MsImeDirectStrategy = MsImeDirectStrategy;
static KANJI_STRATEGY: KanjiToggleStrategy = KanjiToggleStrategy;

impl ImeController {
    pub(crate) const fn new() -> Self {
        Self {
            strategies: [&IMM_STRATEGY, &GJI_STRATEGY, &MS_IME_STRATEGY, &KANJI_STRATEGY],
        }
    }

    /// コンテキストに応じた戦略を選択して IME を設定する。
    ///
    /// 戦略が `Failed` を返した場合（例: `ImmCrossProcessStrategy` の `SendMessageTimeout` タイムアウト）、
    /// 次の適用可能な戦略にフォールスルーする。
    pub(crate) fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        Self::apply_iter(&self.strategies, open, view)
    }

    /// `ImmCrossProcessStrategy` を除いた戦略リストで IME を設定する。
    ///
    /// async 化された IMM クロスプロセス経路が `Failed` を返した後のフォールバック用。
    /// `strategies[0]` (IMM) をスキップして GJI / KanjiToggle のみで再試行する。
    pub(crate) fn apply_skipping_imm(
        &self,
        open: bool,
        view: &ImeControlView<'_>,
    ) -> ImeOpenOutcome {
        Self::apply_iter(&self.strategies[1..], open, view)
    }

    /// `ImmCrossProcessStrategy` が現在のコンテキストで最初に適用可能か。
    ///
    /// dispatch 側で「async 経路 (IMM)」と「sync 経路 (GJI/Kanji)」を branch するための判定。
    /// `strategies` の構築順 (`new` で IMM が index 0) に依存する。
    pub(crate) fn imm_cross_is_first_applicable(&self, view: &ImeControlView<'_>) -> bool {
        self.strategies
            .iter()
            .position(|s| s.is_applicable(view))
            .is_some_and(|idx| idx == 0)
    }

    fn apply_iter(
        strategies: &[&'static dyn ImeOpenStrategy],
        open: bool,
        view: &ImeControlView<'_>,
    ) -> ImeOpenOutcome {
        for strategy in strategies {
            if strategy.is_applicable(view) {
                let outcome = strategy.apply(open, view);
                if outcome != ImeOpenOutcome::Failed {
                    return outcome;
                }
                log::debug!("[apply-ime] strategy failed, trying next fallback");
            }
        }
        log::warn!(
            "[apply-ime] all strategies failed for class={}",
            view.focus.class_name
        );
        ImeOpenOutcome::Failed
    }
}

/// モジュール公開のコントローラインスタンス。
///
/// `WindowsPlatform::apply_ime_open` と `DecisionExecutor::dispatch_effect` の
/// async branch 経路の双方から参照される（ImmCross が first applicable かどうかで
/// async / sync 経路を切り替えるため、両所で同じインスタンスを共有する必要がある）。
pub(crate) static CONTROLLER: ImeController = ImeController::new();
