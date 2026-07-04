//! IME ON/OFF 制御の Strategy パターン実装。
//!
//! `WindowsPlatform::apply_ime_open` の内部メカニズム選択ロジックを
//! `ImeController` + `ImeOpenStrategy` に分離する。
//!
//! # 戦略リスト（優先順）
//! 1. `ImmCrossProcessStrategy` — IMM-bridge が生きているウィンドウ向け（Imm32Unavailable は skip）
//! 2. `GjiDirectStrategy`       — GJI 検出済み時の一方向制御（VK_IME_ON/OFF）。全プロファイルで適用
//! 3. `MsImeDirectStrategy`     — MS-IME 環境の TSF アプリ向け（VK_DBE_HIRAGANA/ALPHANUMERIC 冪等制御）
//! 4. `KanjiToggleStrategy`     — 最終フォールバック。IME 種別不明環境向け
//!
//! `ImmCrossProcessStrategy` が `Failed` を返した場合（例: `SendMessageTimeout` タイムアウト）、
//! `ImeController` は次の適用可能な戦略へフォールスルーする。
//! GJI が検出されている場合は `GjiDirectStrategy` が後続戦略より優先される。
//!
//! ## GJI 前提の設計方針
//! VK_IME_ON (0x16) / VK_IME_OFF (0x1A) は Windows 標準の冪等キーで GJI がネイティブに処理する。
//! IME 層で処理されるためフォアグラウンドアプリのプロファイルに依存しない。
//! Chrome / WezTerm / Windows Terminal すべてで動作確認済み（2026-06-28）。
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

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        if open
            && view.observed.active_ime_kind == ActiveImeKind::MicrosoftIme
            && !matches!(view.belief_input_mode, awase::engine::InputModeState::ObservedKana)
        {
            // MS-IME + ImmCross (LINE 等): かなモードのまま IME ON すると JIS かな入力になる。
            // 先に ROMAN ビットを追加してローマ字モードに戻す。
            // SAFETY: set_ime_romaji_mode は Win32 API。メインスレッドから呼ぶこと。
            let _ = unsafe { crate::ime::set_ime_romaji_mode() };
        }
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
/// VK_IME_ON/OFF は Windows 標準 IME 制御キーで GJI が TSF 層でネイティブに処理する。
/// Chrome・WezTerm・Windows Terminal（TsfNative）すべてで動作確認済み（2026-06-28）。
/// TsfNative では旧 F22 キーバインド時代に「半角英数止まり」の問題があったが、
/// VK_IME_OFF (0x1A) 移行後は TSF compartment が正しく閉じることを確認済み。
///
/// 適用条件:
/// - `active_ime_kind == GoogleJapaneseInput` (CLSID ベース判定)
pub(crate) struct GjiDirectStrategy;

impl ImeOpenStrategy for GjiDirectStrategy {
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool {
        view.observed.active_ime_kind == ActiveImeKind::GoogleJapaneseInput
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

/// MS-IME 向けの冪等 IME 制御戦略（TsfNative アプリ用）。
///
/// CLSID ベースで MS-IME（または互換 IME）がアクティブと判定された場合に、
/// IMM32 クロスプロセス制御が使えない TSF アプリ（Windows Terminal 等）への制御を担う。
///
/// - ON  → `VK_DBE_HIRAGANA` (0xF2) — ひらがなモードに設定（カタカナ時はスキップ）
/// - OFF → `VK_IME_OFF` (0x1A) — DirectInput（直接入力）へ移行。MS-IME がネイティブに処理する冪等キー。
///         `VK_DBE_ALPHANUMERIC` は半角英数（IME-ON）に留まるため使用しない。
///         `VK_KANJI` はトグルのため使用しない（shadow desync で逆転する）。
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

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        if open {
            // カタカナモード（KATAKANA bit 立ち）のとき VK_DBE_HIRAGANA を送ると
            // ひらがなに切り替わる（IME 的には「ON→ON」だが conv mode が破壊される）。
            // 現在の conv を読んで KATAKANA bit が立っている場合は送信をスキップする。
            // Safety: get_ime_conversion_mode_raw_timeout は Win32 API。メインスレッドから呼ぶこと。
            if let Some(conv) =
                unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(5) }
            {
                if crate::imm::cmode_has(conv, crate::imm::IME_CMODE_KATAKANA) {
                    log::debug!(
                        "[apply-ime] MS-IME direct: conv=0x{conv:08X} カタカナモード \
                         → VK_DBE_HIRAGANA スキップ (AlreadyMatched)"
                    );
                    return ImeOpenOutcome::AlreadyMatched;
                }
            }
            // VK_DBE_HIRAGANA は ROMAN ビット (IME_CMODE_ROMAN=0x10) を変更しない。
            // かな入力の conv=0x09 のまま IME ON すると JIS かな入力になる（例: LINE, Edge）。
            // 先に ROMAN ビットを立てておくことでフォーカス直後のかな入力化けを防ぐ。
            // ただし ObservedKana はユーザーが意図的にかな入力に設定した状態なので上書きしない。
            // SAFETY: set_ime_romaji_mode は Win32 API。メインスレッドから呼ぶこと。
            if !matches!(view.belief_input_mode, awase::engine::InputModeState::ObservedKana) {
                let _ = unsafe { crate::ime::set_ime_romaji_mode() };
            }
            log::debug!("[apply-ime] MS-IME direct: VK_DBE_HIRAGANA (IME ON)");
            // SAFETY: post_ms_ime_on は Win32 API を呼び出す unsafe fn。メインスレッドから呼ぶこと。
            unsafe { crate::ime::post_ms_ime_on() };
        } else {
            // DirectInput（直接入力）へ移行する。
            // VK_IME_OFF は MS-IME がネイティブに処理する冪等キー。
            // 既に DirectInput の場合は no-op のため conv チェック不要。
            log::debug!("[apply-ime] MS-IME direct: VK_IME_OFF (DirectInput, 冪等)");
            // SAFETY: post_ime_off_direct は Win32 API を呼び出す unsafe fn。メインスレッドから呼ぶこと。
            unsafe { crate::ime::post_ime_off_direct() };
        }
        ImeOpenOutcome::Applied
    }
}

// ── KanjiToggleStrategy ──────────────────────────────────────────

/// `SendInput(VK_KANJI)` トグルを使う最終フォールバック戦略。
///
/// IME 種別が不明な環境向けの汎用フォールバック。
/// GJI 環境では `GjiDirectStrategy` が全プロファイルで先行するため、
/// ここには GJI 以外（IME 種別不明 / ImmCross 失敗後）のみ到達する。
///
/// VK_KANJI はトグルキーのため冪等ではなく、`already_matched` の判定は行わず送信する。
/// GJI / MS-IME 環境では前段の戦略が処理するため、このフォールバックは稀にしか使われない。
pub(crate) struct KanjiToggleStrategy;

impl ImeOpenStrategy for KanjiToggleStrategy {
    fn is_applicable(&self, _view: &ImeControlView<'_>) -> bool {
        true // 汎用フォールバック: IME 種別不明環境 + ImmCross 失敗時の代替
    }

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        log::debug!(
            "[apply-ime] shadow={} candidate={} was_seen={} profile={:?} → desired={open}: SendInput VK_KANJI",
            view.control.shadow_on, view.observed.candidate_visible, view.observed.candidate_was_seen,
            view.focus.profile,
        );
        unsafe { crate::ime::post_kanji_toggle_to_focused() };
        ImeOpenOutcome::FallbackSent
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
