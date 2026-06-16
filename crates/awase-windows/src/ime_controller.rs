//! IME ON/OFF 制御の Strategy パターン実装。
//!
//! `WindowsPlatform::apply_ime_open` の内部メカニズム選択ロジックを
//! `ImeController` + `ImeOpenStrategy` に分離する。
//!
//! # 戦略リスト（優先順）
//! 1. `ImmCrossProcessStrategy` — IMM-bridge が生きているウィンドウ向け（Imm32Unavailable は skip）
//! 2. `GjiDirectStrategy`       — GJI 検出済み時の一方向制御（F13/F14）。**全プロファイル**で適用可能
//! 3. `KanjiToggleStrategy`     — 最終フォールバック。GJI 非検出時の MS-IME 環境向け
//!
//! `ImmCrossProcessStrategy` が `Failed` を返した場合（例: `SendMessageTimeout` タイムアウト）、
//! `ImeController` は次の適用可能な戦略へフォールスルーする。
//! GJI が検出されている場合は `GjiDirectStrategy` が全プロファイルで `KanjiToggleStrategy` より優先される。
//!
//! ## GJI 前提の設計方針
//! F13/F14 は IME 層で処理されフォアグラウンドアプリのプロファイルに依存しないため、
//! GJI 稼働中はアプリ種別に関わらず GJI を使うことで VK_KANJI トグルアーティファクトを回避できる。
//! GJI が起動していない環境（MS-IME 等）では `KanjiToggleStrategy` が引き続き機能する。
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

/// GJI を使った一方向 IME 制御戦略。
///
/// VK_KANJI（トグル）の代わりに GJI 固有のキーを使うことで shadow desync の影響を排除する:
/// - ON  → F13（DirectInput 時にひらがなへ切り替え、既に ON なら no-op）
/// - OFF → F14（Precomposition/Composition/Conversion 時に IME OFF）
///
/// F13/F14 は IME 層で処理されフォアグラウンドアプリのプロファイルに依存しないため、
/// Standard / Imm32Unavailable **全プロファイル**で利用できる。
/// F13/F14 は実キーボードに存在しないためブラウザショートカットと衝突しない。
///
/// **TsfNative プロファイル（WezTerm / Windows Terminal 等）について**
/// これらの VT ターミナルエミュレータは通常 F13 を ESC[25~、F14 を ESC[26~ という
/// VT エスケープシーケンスに変換するが、WezTerm 側で F13/F14 を Nop にバインドすれば
/// ターミナルへの漏れを防げる。その場合、GJI の TSF 層が F14 を消費すれば
/// VK_KANJI（トグル）を使わずに IME OFF を達成できる（desync フリー）。
/// GJI が TSF 層で消費しない場合は KanjiToggleStrategy にフォールスルーする。
///
/// GJI の config1.db に以下を登録することで有効になる:
///   `DirectInput\tF13\tIMEOn`（デフォルト登録済み）
///   `Precomposition\tF14\tIMEOff`
///   `Composition\tF14\tIMEOff`
///   `Conversion\tF14\tIMEOff`
///
/// `gji_monitor_ok=true`（GJI プロセス検出済み）かつ
/// `gji_keybinds_ok=true`（F13/F14 が config1.db に登録済み）の場合のみ適用可能。
/// どちらかが false の場合は `KanjiToggleStrategy`（MS-IME 向け）がフォールバックする。
pub(crate) struct GjiDirectStrategy;

impl ImeOpenStrategy for GjiDirectStrategy {
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool {
        view.observed.gji_monitor_ok && view.observed.gji_keybinds_ok
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
            // F14 は GJI config で Conversion\tF14\tIMEOff が登録されているため、
            // 候補ウィンドウ表示中でも直接 F14 を送れば IME OFF になる。
            // Ctrl+Enter で候補確定→F14 の2段構えは不要かつ Chrome フォーム送信を
            // 引き起こす副作用があるため送らない。
            log::debug!(
                "[apply-ime] GJI direct: F14 (IME OFF, candidate={})",
                view.observed.candidate_visible,
            );
            unsafe { crate::ime::post_gji_ime_off() };
        }
        ImeOpenOutcome::Applied
    }
}

// ── KanjiToggleStrategy ──────────────────────────────────────────

/// `SendInput(VK_KANJI)` トグルを使う最終フォールバック戦略（MS-IME 向け）。
///
/// GJI が起動していない環境（MS-IME 等）での全プロファイル共通フォールバック。
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
