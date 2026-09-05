#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! IME ON/OFF 制御の Strategy パターン実装。
//!
//! `WindowsPlatform::apply_ime_open` の内部メカニズム選択ロジックを
//! `ImeController` + `ImeOpenStrategy` に分離する。
//!
//! # 戦略リスト（優先順）
//! 1. `ImmCrossProcessStrategy` — IMM-bridge が生きているウィンドウ向け（Imm32Unavailable は skip）
//! 2. `GjiDirectStrategy`       — GJI 検出済み時の一方向制御（VK_IME_ON/OFF）。全プロファイルで適用
//! 3. `MsImeDirectStrategy`     — MS-IME 環境の TSF アプリ向け（VK_IME_ON/OFF 冪等制御。
//!    2026-08-06 まで ON は `VK_DBE_HIRAGANA` だった、BUG-50 参照）
//! 4. `KanjiToggleStrategy`     — 最終フォールバック。実到達は「Standard プロファイル ×
//!    MS-IME × ImmCross 非同期失敗後（apply_skipping_imm）」の 1 組み合わせのみ
//!
//! `ImmCrossProcessStrategy` が `Failed` を返した場合（例: `SendMessageTimeout` タイムアウト）、
//! `ImeController` は次の適用可能な戦略へフォールスルーする。
//! GJI が検出されている場合は `GjiDirectStrategy` が後続戦略より優先される。
//!
//! ## GJI 前提の設計方針
//! VK_IME_ON (0x16) / VK_IME_OFF (0x1A) は Windows 標準の冪等キーで GJI がネイティブに処理する。
//! IME 層で処理されるためフォアグラウンドアプリのプロファイルに依存しない。
//! Chrome / WezTerm / Windows Terminal すべてで動作確認済み（2026-06-28）。
//! GJI が起動していない環境（MS-IME 等）では `MsImeDirectStrategy`（冪等 VK_DBE_*）が先行する。
//! 注: `ActiveImeKind` は GJI / MS-IME の 2 値で「IME 種別不明」という状態は存在しない
//! （未検出時は MicrosoftIme を安全デフォルトとして返す）。`KanjiToggleStrategy` に
//! 到達するのは Standard プロファイル × MS-IME で ImmCross 非同期適用が Failed した
//! 後（`apply_skipping_imm`）だけである（golden の戦略選択テーブルと一致、2026-07-06 監査）。
//!
//! ## アーキテクチャ制約
//! このモジュールは観測値を自ら読んではいけない。
//! すべての観測値は `ImeControlView` 経由で受け取ること。
//! `crate::tsf::observer::tsf_obs()` の直接呼び出し禁止（スナップショット経由で受け取ること）。

use awase::platform::ImeOpenOutcome;

use crate::state::actuation_chain::{
    needs_romaji_pre_write, ActuationOrder, MechanismWriter, VerifiedTarget, WriteMechanism,
};
use crate::state::app_ime_policy::caps;
use crate::state::ime_decision_view::ImeControlView;
use crate::state::key_sequence_policy::{self, ime_key_for, ImeOperation, KeyMechanism};
use crate::tsf::observer::ActiveImeKind;

/// IME ON/OFF を実行する戦略インターフェース。
///
/// **このモジュールの外へは出さない**（ADR-089 §2.3、Phase B 追随）。
/// `pub(crate)` のままだと `GjiDirectStrategy.apply(open, &view)` と書くだけで
/// `Actuation` 型状態チェーンを一切構築せずに 1 機構分の実 write
/// （`SendInput` / `post_kanji_toggle_to_focused`）を起こせてしまう。
/// crate 内の唯一の write 入口は `apply_mechanism`（呼び出し元 2 箇所を
/// `tests/architecture_guard.rs` の
/// `raw_mechanism_write_sites_are_confined_to_chain_writers` が固定）である。
trait ImeOpenStrategy: Sync {
    /// このコンテキストで戦略が有効かどうか。
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool;
    /// IME を指定状態に設定しその結果を返す。
    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome;
}

// ── ImmCrossProcessStrategy ──────────────────────────────────────

/// `ImmSetOpenStatus`（cross-process）を使う標準戦略。
///
/// IMM-bridge が機能しているウィンドウにのみ適用可能。
struct ImmCrossProcessStrategy;

impl ImeOpenStrategy for ImmCrossProcessStrategy {
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool {
        key_sequence_policy::imm_cross_applicable(view.focus.profile)
    }

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        // ROMAN ビットの事前補完（MS-IME + ImmCross でかなモードのまま IME ON すると
        // JIS かな入力になる問題への対処）は、ADR-089 §6 Phase C item 12 で
        // `apply_mechanism` の ROMAN 補完ステップへ移した（ADR-086 INV-14 の是正）。
        // 発火条件は `needs_romaji_pre_write` が SSOT。
        //
        // ADR-117（issue #138 切り分け）: この経路は Standard プロファイル×MS-IME の
        // 完全同期呼び出し（`try_force_on_bootstrap` 等）のみ到達し、報告環境の
        // 主経路（非同期 ImmCross）は `runtime/open_chain.rs::imm_cross_write` が
        // 別途担う。`composition_active`/`ime_show_seq`/`ime_change_seq` の解釈上の
        // 注意は `TsfObservations::ime_composition_active` の doc コメント参照。
        log::info!(
            "[apply-ime] ImmCross sync: open={open} composition_active={} show_seq={} \
             change_seq={} (issue #138診断)",
            view.observed.composition_active,
            view.observed.ime_show_seq,
            view.observed.ime_change_seq,
        );
        if unsafe { crate::ime::set_ime_open_cross_process(open) } {
            ImeOpenOutcome::Applied
        } else {
            log::info!("[apply-ime] ImmCross sync: set_ime_open_cross_process failed → Failed");
            ImeOpenOutcome::Failed
        }
    }
}

// ── BUG-113 診断専用スパイク・第2弾（一時的、恒久機能ではない） ─────────
//
// docs/known-bugs.md BUG-113「未解決・次にやること」・docs/experiments.md
// エントリ21参照。バッチ形状（候補V/A/B3/B4）・VK値（`VK_IME_OFF` vs
// `VK_KANJI`）の両仮説は実機A/Bで反証済み。ユーザー実機観測から PowerShell
// PSReadLine との相互作用が引き金であることは確認したが、awase 側の
// どの送信方式・呼び出し経路が「@」の必要十分条件かはまだ未特定——
// 「相手（GJI/PSReadLine）の実装が脆弱」であることは awase 側の送信動作の
// 責任を免除しない。
//
// 2026-09-05 の呼び出し連鎖調査で、`GjiDirectStrategy::apply(open=false)`
// は同一の物理キー押下に対し `shadow_toggle_off_sync`/`engine_decision_sync`
// の2経路から連続で2回呼ばれ、実際に `send_ime_mode_key` の `SendInput` を
// 2回発行していることを再確認した。また、これとは別に
// `runtime/key_pipeline.rs::kp_stage_idle_conv_check`（アイドル500ms超・
// 直近1500ms以内に明示的IME操作なしの条件で発火、本 BUG の再現条件と一致）
// が非同期ワーカースレッド上で `get_ime_conversion_mode_raw_timeout_async`
// （GJI の IME ウィンドウへの `SendMessageTimeoutW(WM_IME_CONTROL/
// IMC_GETCONVERSIONMODE)`、cross-process の**読み取り専用**クエリ）を
// spawn しており、これは `GjiDirectStrategy::apply` の同期 `SendInput`
// 発行と時間的に競合しうる——バッチ形状・VK値ラウンドでは一度も検証
// されていない新しい候補である。
//
// なお `set_ime_open_cross_process`（`ImmSetOpenStatus`）ベースの制御は
// 候補から除外した: `AppImeProfile::TsfNative`（Windows Terminal 含む）は
// `can_use_imm32_cross_process() == false`（`focus/class_names.rs`）であり、
// この API はそもそも TSF アプリに対して効果を持たないため、「@」が
// 出なくても「回避できた」のか「単に何も起きなかった」のか区別できず
// 意味のある比較にならない。
//
// 本スパイクは2つの独立した hidden config トグルを用意する（実機での
// 組み合わせテストは config.toml を編集して awase を再起動して行う。
// 統計的に意味のある判定には1トグル・1状態あたり最低20 episode程度の
// 試行が必要——round1 敵対的レビュー Major 7 の「少数試行では偽陰性で
// 候補が誤って『効いた』と判定されうる」という指摘が過去に一度的中して
// いるため、docs/experiments.md エントリ21参照）:
//
//   `diag_bug113_dedup_gji_off_actuation`（本ブロック）: 有効なら
//   `GjiDirectStrategy::apply(open=false)` の実アクチュエーション
//   （`send_ime_mode_key` の `SendInput`）を1クローズ episode につき
//   最初の呼び出し（`shadow_toggle_off_sync`）のみに限定し、2回目
//   （`engine_decision_sync`）は no-op で吸収する。
//
//   `diag_bug113_skip_idle_conv_probe`（`runtime/key_pipeline.rs` 側で
//   実装、本ブロックとは別ファイル）: 有効なら非resyncの
//   `kp_stage_idle_conv_check` が spawn する上記 cross-process 読み取りを
//   スキップする。
//
// 調査終了後は `crate::ime_controller::set_diag_bug113_dedup_off_actuation_enabled`
// の呼び出し元・本ブロック一式、および `key_pipeline.rs` 側の対応する
// トグルを削除すること。

static DIAG_BUG113_DEDUP_OFF_ACTUATION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// `open=false` の「クローズ episode」を跨いだかどうかの追跡。`true` なら
/// 直前に `open=true` を処理済み＝次に来る `open=false` は新しい episode
/// の最初の呼び出し。`GjiDirectStrategy::apply(open=false)` は
/// `shadow_toggle_off_sync` / `engine_decision_sync` の2経路から同一の
/// 物理押下に対して連続で2回呼ばれる（2026-09-05 実機ログで確認済み）。
static DIAG_BUG113_LAST_OPEN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// BUG-113 診断専用（一時的、第2弾）: `true` の間、
/// `GjiDirectStrategy::apply(open=false)` の実アクチュエーションを1クローズ
/// episode につき最初の呼び出しだけに限定する（dedup）。`bootstrap.rs`
/// （起動時）と `runtime/mod.rs::apply_config_update`（reload 時）から
/// `config.toml` の `diag_bug113_dedup_gji_off_actuation` に応じて直接呼ぶ。
pub(crate) fn set_diag_bug113_dedup_off_actuation_enabled(enabled: bool) {
    use std::sync::atomic::Ordering;
    if enabled != DIAG_BUG113_DEDUP_OFF_ACTUATION.swap(enabled, Ordering::Relaxed) {
        log::info!(
            "[bug113-diag2] dedup off-actuation: {}",
            if enabled { "有効化" } else { "無効化" }
        );
    }
    if enabled {
        DIAG_BUG113_LAST_OPEN.store(true, Ordering::Relaxed);
    }
}

/// `GjiDirectStrategy::apply(open=true)` を処理した際に呼ぶ。次に来る
/// `open=false` を新しいクローズ episode として扱えるようにする。
fn diag_bug113_mark_open() {
    use std::sync::atomic::Ordering;
    DIAG_BUG113_LAST_OPEN.store(true, Ordering::Relaxed);
}

/// dedup が無効なら常に `true`（現行どおり毎回送信）。有効なら、現在の
/// クローズ episode 内で最初の呼び出しのときだけ `true` を返し、2回目
/// 以降は `false`（呼び出し元は no-op で吸収する）。
fn diag_bug113_should_actuate_off() -> bool {
    use std::sync::atomic::Ordering;
    // combo cycle（第3弾）が有効ならそちらを優先し、無効なら単体トグル
    // （第2弾、`diag_bug113_dedup_gji_off_actuation`）にフォールバックする。
    let dedup_enabled = if crate::diag_bug113_combo::combo_cycle_enabled() {
        crate::diag_bug113_combo::current_combo_flags().0
    } else {
        DIAG_BUG113_DEDUP_OFF_ACTUATION.load(Ordering::Relaxed)
    };
    if !dedup_enabled {
        return true;
    }
    DIAG_BUG113_LAST_OPEN.swap(false, Ordering::Relaxed)
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
struct GjiDirectStrategy;

impl ImeOpenStrategy for GjiDirectStrategy {
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool {
        key_sequence_policy::gji_direct_applicable(view.observed.active_ime_kind)
    }

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        if open {
            // BUG-113 診断専用（一時的、第2弾）: open=true を処理したら episode
            // 境界をリセットする（AlreadyMatched で早期 return する場合も含む）。
            diag_bug113_mark_open();
        }
        if open && view.control.shadow_on {
            // shadow が ON を示しており VK_IME_ON は no-op と見込まれるためスキップ
            log::debug!("[apply-ime] GJI direct: shadow ON, skip VK_IME_ON");
            return ImeOpenOutcome::AlreadyMatched;
        }
        // 送信キーは KeySequencePolicy が SSOT（VK_IME_ON / VK_IME_OFF、GJI 冪等キー）。
        let vk = ime_key_for(KeyMechanism::GjiDirect, ImeOperation::from_open(open));
        if !open && !diag_bug113_should_actuate_off() {
            // BUG-113 診断専用（一時的、第2弾、dedup有効時のみ）: 同一クローズ
            // episode内の2回目以降の呼び出しを吸収する。
            log::debug!(
                "[bug113-diag2] dedup: episode内2回目以降のsend_ime_mode_key({vk:#06X})を吸収（no-op）"
            );
            return ImeOpenOutcome::FallbackSent;
        }
        log::debug!("[apply-ime] GJI direct: send {vk:#06X} (open={open})");
        // SAFETY: send_ime_mode_key は Win32 API を呼び出す unsafe fn。メインスレッドから呼ぶこと。
        if unsafe { crate::ime::send_ime_mode_key(vk) } {
            ImeOpenOutcome::Applied
        } else {
            // Win キー押下中で未送信。Applied 扱いにすると applied_snapshot がラッチされ
            // 以降の再試行が全て no-op になる（BUG-16 追補）。未適用として返す。
            ImeOpenOutcome::UnsafeToToggle
        }
    }
}

// ── MsImeDirectStrategy ──────────────────────────────────────────

/// MS-IME 向けの冪等 IME 制御戦略（TsfNative アプリ用）。
///
/// CLSID ベースで MS-IME（または互換 IME）がアクティブと判定された場合に、
/// IMM32 クロスプロセス制御が使えない TSF アプリ（Windows Terminal 等）への制御を担う。
///
/// - ON  → `VK_IME_ON` (0x16) — DirectInput → IME ON（conv-mode には一切触れない）。
/// - OFF → `VK_IME_OFF` (0x1A) — DirectInput（直接入力）へ移行。MS-IME がネイティブに処理する冪等キー。
///   `VK_DBE_ALPHANUMERIC` は半角英数（IME-ON）に留まるため使用しない。
///   `VK_KANJI` はトグルのため使用しない（shadow desync で逆転する）。
///
/// 2026-08-06 まで ON は `VK_DBE_HIRAGANA`（モード選択キー）を使っていた。OFF は
/// `48a667a`（2026-06-27頃）で同種のモードキー `VK_DBE_ALPHANUMERIC` から `VK_IME_OFF`
/// （真の開閉キー）へ既に移行済みだったが、ON 側だけがモードキーのまま取り残されて
/// いた。`VK_DBE_HIRAGANA` は「開く」と「ひらがなに強制する」を1つの副作用に束ねて
/// おり、これが BUG-50（一度カタカナに入ると復旧不能になるデッドロック）の直接の
/// 前提だった: 現在の conv がカタカナのときこのキーを送るとカタカナが壊れるため、
/// 送信をスキップするガード（旧 `AlreadyMatched` 分岐）が必要になり、そのガードが
/// 「ユーザーの意図的なカタカナ」と「内部の誤ったカタカナ」を区別できずデッドロックを
/// 生んでいた。`VK_IME_ON` は GJI で既に実績のある「開くだけ」の冪等キーであり
/// conv-mode を一切変更しないため、このガード自体が不要になる（下記 `apply` 参照）。
///
/// 適用条件:
/// - `active_ime_kind == MicrosoftIme` (CLSID ベース判定)
/// - `can_use_imm32_cross_process() == false`（IMM32 が使えない TSF アプリ）
struct MsImeDirectStrategy;

impl ImeOpenStrategy for MsImeDirectStrategy {
    fn is_applicable(&self, view: &ImeControlView<'_>) -> bool {
        key_sequence_policy::ms_ime_direct_applicable(
            view.observed.active_ime_kind,
            view.focus.profile,
        )
    }

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        // ADR-117（issue #138 切り分け）: `composition_active`/`ime_show_seq`/
        // `ime_change_seq` の解釈上の注意は `TsfObservations::ime_composition_active`
        // の doc コメント参照（MS-IME での信頼性は未検証）。
        if open {
            // VK_IME_ON は conv-mode（ひらがな/カタカナ、全角/半角）に一切触れない
            // 真の開閉キーのため、旧 VK_DBE_HIRAGANA 版にあった「現在カタカナなら
            // 送信をスキップする」ガード（BUG-50 デッドロックの直接の前提）は不要。
            //
            // VK_IME_ON は ROMAN ビット (IME_CMODE_ROMAN=0x10) を変更しない。
            // かな入力の conv=0x09 のまま IME ON すると JIS かな入力になる（例: LINE, Edge）。
            // 先に ROMAN ビットを立てる補完は ADR-089 §6 Phase C item 12 で
            // `apply_mechanism` の ROMAN 補完ステップへ移した（ADR-086 INV-14 の是正）。
            // `ObservedKana`（ユーザーが意図的にかな入力に設定した状態）を上書きしない
            // 保護もそちらへ移動している（`needs_romaji_pre_write` が SSOT）。
            //
            // 送信キーは KeySequencePolicy が SSOT（VK_IME_ON、MS-IME 冪等 ON キー）。
            let vk = ime_key_for(KeyMechanism::MsImeDirect, ImeOperation::Open);
            log::info!(
                "[apply-ime] MS-IME direct: send {vk:#06X} (IME ON) composition_active={} \
                 show_seq={} change_seq={} (issue #138診断)",
                view.observed.composition_active,
                view.observed.ime_show_seq,
                view.observed.ime_change_seq,
            );
            // SAFETY: send_ime_mode_key は Win32 API を呼び出す unsafe fn。メインスレッドから呼ぶこと。
            if !unsafe { crate::ime::send_ime_mode_key(vk) } {
                // Win キー押下中（デスクトップ切替等）で未送信。Applied 扱いにすると
                // applied_snapshot がラッチされ、settle 明けの force-ON 再試行まで全て
                // 「適用済み」no-op になり belief ON × 実 IME OFF が固定される
                // （2026-07-07 実機: ロック解除 → Win+Ctrl+→ 直後の「korede」化。
                // BUG-16 追補）。未適用として返し、次の refresh/force-ON に再送させる。
                log::info!(
                    "[apply-ime] MS-IME direct: send_ime_mode_key failed (Winキー押下中等) \
                     → UnsafeToToggle"
                );
                return ImeOpenOutcome::UnsafeToToggle;
            }
        } else {
            // DirectInput（直接入力）へ移行する。
            // VK_IME_OFF は MS-IME がネイティブに処理する冪等キー。
            // 既に DirectInput の場合は no-op のため conv チェック不要。
            let vk = ime_key_for(KeyMechanism::MsImeDirect, ImeOperation::Close);
            log::info!(
                "[apply-ime] MS-IME direct: send {vk:#06X} (DirectInput, 冪等) \
                 composition_active={} show_seq={} change_seq={} (issue #138診断)",
                view.observed.composition_active,
                view.observed.ime_show_seq,
                view.observed.ime_change_seq,
            );
            // SAFETY: send_ime_mode_key は Win32 API を呼び出す unsafe fn。メインスレッドから呼ぶこと。
            if !unsafe { crate::ime::send_ime_mode_key(vk) } {
                log::info!(
                    "[apply-ime] MS-IME direct: send_ime_mode_key failed (Winキー押下中等) \
                     → UnsafeToToggle"
                );
                return ImeOpenOutcome::UnsafeToToggle;
            }
        }
        ImeOpenOutcome::Applied
    }
}

// ── KanjiToggleStrategy ──────────────────────────────────────────

/// `SendInput(VK_KANJI)` トグルを使う最終フォールバック戦略。
///
/// 実際に到達する組み合わせは 1 つだけ: **Standard プロファイル × MS-IME ×
/// ImmCross 非同期適用の失敗後（`apply_skipping_imm`）**。
/// `ActiveImeKind` は GJI / MS-IME の 2 値のため「IME 種別不明」は存在せず、
/// 通常の `apply` では ImmCross（Standard）か GJI/MsImeDirect（非 Standard）が
/// 必ず先に捕捉する（golden の戦略選択テーブル参照、2026-07-06 監査で確認）。
///
/// VK_KANJI はトグルキーのため冪等ではなく、`already_matched` の判定は行わず送信する。
/// GJI / MS-IME 環境では前段の戦略が処理するため、このフォールバックは稀にしか使われない。
struct KanjiToggleStrategy;

impl ImeOpenStrategy for KanjiToggleStrategy {
    fn is_applicable(&self, _view: &ImeControlView<'_>) -> bool {
        true // 汎用フォールバック: IME 種別不明環境 + ImmCross 失敗時の代替
    }

    fn apply(&self, open: bool, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        // ADR-117（issue #138 切り分け）: Standard×MS-IME で ImmCross が Failed を
        // 返した場合に `runtime/open_chain.rs::fallback_write` が実際に到達させる
        // 唯一の機構。composition_active/ime_show_seq/ime_change_seq の解釈上の
        // 注意（MS-IME での信頼性未検証）に加え、`fallback_write` が view を
        // 作り直すため、この値は直前の ImmCross 試行にとっては送信「後」の値
        // でもある点に注意（`fallback_write` の doc コメント参照）。
        log::info!(
            "[apply-ime] shadow={} candidate={} was_seen={} profile={:?} \
             composition_active={} show_seq={} change_seq={} → desired={open}: \
             SendInput VK_KANJI (issue #138診断)",
            view.control.shadow_on,
            view.observed.candidate_visible,
            view.observed.candidate_was_seen,
            view.focus.profile,
            view.observed.composition_active,
            view.observed.ime_show_seq,
            view.observed.ime_change_seq,
        );
        unsafe { crate::ime::post_kanji_toggle_to_focused() };
        ImeOpenOutcome::FallbackSent
    }
}

// ── 機構 → 戦略の写像 / ImeController（ADR-089 §2.3）─────────────

static IMM_STRATEGY: ImmCrossProcessStrategy = ImmCrossProcessStrategy;
static GJI_STRATEGY: GjiDirectStrategy = GjiDirectStrategy;
static MS_IME_STRATEGY: MsImeDirectStrategy = MsImeDirectStrategy;
static KANJI_STRATEGY: KanjiToggleStrategy = KanjiToggleStrategy;

/// `WriteMechanism` から実装戦略を引く唯一の写像。
///
/// `WriteMechanism`（`state/actuation_chain.rs`、ungated）が chain の語彙で、
/// ここが Windows 側の実装への橋である。`caps(p, k).chain` を導入する
/// Phase C（ADR-089 §2.8）でもこの写像はそのまま使える。
const fn strategy_for(mechanism: WriteMechanism) -> &'static dyn ImeOpenStrategy {
    match mechanism {
        WriteMechanism::ImmCross => &IMM_STRATEGY,
        WriteMechanism::GjiDirect => &GJI_STRATEGY,
        WriteMechanism::MsImeDirect => &MS_IME_STRATEGY,
        WriteMechanism::KanjiToggle => &KANJI_STRATEGY,
    }
}

/// 機構が現在のコンテキストで適用可能か（`runtime` 層の async writer からも使う）。
pub(crate) fn mechanism_is_applicable(
    mechanism: WriteMechanism,
    view: &ImeControlView<'_>,
) -> bool {
    strategy_for(mechanism).is_applicable(view)
}

/// 機構 1 つ分の同期 write（`runtime` 層の async writer のフォールバック側から使う）。
///
/// # 呼び出してよい場所（ADR-089 §2.3、Phase B 追随）
///
/// **この関数は `Actuation` 型状態チェーンを構築せずに実 write
/// （`SendInput` / `post_kanji_toggle_to_focused` / `ImmSetOpenStatus`）を起こせる
/// 唯一の口である。** 呼んでよいのは
/// `MechanismWriter` / `AsyncMechanismWriter` の `write` 実装
/// （= `run_chain` / `run_chain_async` が駆動する write ステップそのもの）だけ:
///
/// 1. `SyncChainWriter::write`（本ファイル、同期チェーン）
/// 2. `runtime::open_chain::fallback_write`（非同期チェーンの ImmCross 以降）
///
/// writer 実装は「チェーンの write ステップ」なので、定義上これ以上チェーンを
/// 経由させることができない（`impl` の中でチェーンを再度張ると再帰する）。
/// そのため型では閉じられず、**呼び出し元の件数を
/// `tests/architecture_guard.rs::raw_mechanism_write_sites_are_confined_to_chain_writers`
/// が固定している**（`ActuationOrder` / `run_open_chain_async` の件数ガードと
/// 同じパターン）。ここを増やすと、`falls_through` 規則も
/// `Actuation` のアフィン性（1 値 = 高々 1 回の成功 write、INV-41）も通らない
/// 3 本目の write 経路になる。
pub(crate) fn apply_mechanism(
    mechanism: WriteMechanism,
    open: bool,
    view: &ImeControlView<'_>,
) -> ImeOpenOutcome {
    romaji_pre_write(mechanism, open, view);
    strategy_for(mechanism).apply(open, view)
}

/// IME ON の直前に ROMAN ビットを補完する同期 IMC write
/// （ADR-089 §6 Phase C item 12 = ADR-086 INV-14 の未移行分の是正）。
///
/// # Phase C 以前との差分
///
/// | | Phase C 以前 | Phase C |
/// |---|---|---|
/// | 書き込み口 | `ImmCrossProcessStrategy::apply` と `MsImeDirectStrategy::apply` の**2 箇所** | 本関数の**1 箇所** |
/// | 宛先 | `set_ime_romaji_mode()` が write 時点にライブクエリで**自己決定** | 起案時に捕獲した [`crate::ime::ActuationTarget`] |
/// | 世代照合 | 無し | 型としては `view.focus.focus_gen` と照合し不一致なら `Aborted`。**ただし現在の呼び出し方では常に一致する**（下記） |
/// | 結果 | `let _ =` で握り潰し | `Written` 以外は必ずログに残す（INV-14） |
/// | 発火条件 | 2 戦略に別々に書かれた（Linux から検査不能） | `needs_romaji_pre_write`（ungated、全数テスト済み） |
///
/// # 世代照合は現状では恒真である（ADR-089 §9-22）
///
/// 本関数は `let focus_gen = view.focus.focus_gen;` で読んだ**同一の値**を
/// `capture_blocking(focus_gen)` と
/// `set_ime_romaji_mode_for_target_blocking(target, focus_gen)` の両方へ渡す。
/// 前者は受け取った値をそのまま `ActuationTarget::focus_gen` に格納し、後者は
/// それを引数と比較する。したがって **`GenStale` は原理的に返らない**——
/// 間に `focus_gen` を読み直す点も await 点も無いため、比較は
/// `focus_gen == focus_gen` に退化している。
///
/// これは欠陥ではなく意図した形である（同期経路には捕獲と write の間に
/// フォーカスが動く余地が構造的に無い）が、**「世代照合があるから
/// stale target への write は防げている」とは読まないこと**。実効的な
/// 検出力を持つのは、将来この経路に await 点が挟まるか、
/// `Output::ime_mode_focus_gen` を write 直前に読み直す形へ変えたときである。
/// その時までは「構造だけがある」状態として扱う。
///
/// **hwnd 解決の関数・タイムアウト・フォールバックは変えていない**——
/// `ActuationTarget::capture_blocking` は旧 `set_ime_romaji_mode()` と同じ
/// `get_focused_hwnd()`（`GetGUIThreadInfo` 30ms → `GetForegroundWindow`
/// フォールバック）を使う。Win32 往復の回数も 1 回のままである。
///
/// # 残る INV-14 の穴（ADR-089 §9-18）
///
/// `ImmCrossProcessStrategy` の open write（`set_ime_open_cross_process`）は
/// **依然として自分でライブクエリする**（`get_gui_thread_info_with_timeout(150ms)`、
/// フォールバック無し）。したがって ROMAN 補完と open 書き込みが別ウィンドウへ
/// 着弾する可能性は同期 ImmCross 経路に残っている。捕獲を共有させるには
/// hwnd 解決のタイムアウトとフォールバックの意味論を変える必要があり
/// （30ms+fallback ↔ 150ms+no-fallback）、実機ソーク無しでは動かせない。
///
/// **この穴は到達しうる**——初出時は「同期呼び出し元はすべて
/// `imm_cross_is_first_applicable` で async 分岐するか
/// `!can_use_imm32_cross_process()` に限定されているので到達しない」と
/// 書いていたが、`runtime/mod.rs::try_force_on_bootstrap`（`:892`）が
/// プロファイルガードを持たないため Standard でも同期で到達する
/// （ADR-089 §9-21 の訂正、実機確認は §9-17 の 17-h）。
/// ただし **Phase C 以前から同じ挙動**であり、Phase C が作り込んだ
/// 回帰ではない。
fn romaji_pre_write(mechanism: WriteMechanism, open: bool, view: &ImeControlView<'_>) {
    if !needs_romaji_pre_write(
        mechanism,
        open,
        view.observed.active_ime_kind.into(),
        view.belief_input_mode,
    ) {
        return;
    }
    // BUG-34 横展開 Step0-c/レビュー: set_ime_romaji_mode_for_target_blocking は
    // SendMessageTimeoutW ベースで同期ブロックしうる（フルな offload+hwnd 統一化は
    // E として保留中）。当初ここに SendHealth::blocking_allowed の gate を入れて
    // いたが、この関数はフリー関数で Runtime にアクセスできず、スキップ時に
    // 再試行をスケジュールする手段が無かった。ブレーカのcooldown中にopen遷移が
    // 重なると、ROMAN ビットが**次にユーザーが明示的に開閉トグルするまで**
    // 補完されないまま放置される——「ブロックする」を「静かに間違った状態のまま
    // 固着する」に置き換えるだけで、後者は診断ログも残らず前者より発見しにくい。
    // 再試行の仕組み（E の一部として、hwnd 解決統一と合わせて実機ソーク前提で
    // 設計する必要がある）が無いまま gate だけ入れるのは見送り、元の常時試行に
    // 戻す。
    let focus_gen = view.focus.focus_gen;
    // SAFETY: capture_blocking / set_ime_romaji_mode_for_target_blocking はいずれも
    //         Win32 API を呼ぶ unsafe fn。`ImeOpenStrategy::apply` の呼び出しチェーンは
    //         すべてメインスレッド（フックまたはメッセージループ）である。
    let Some(target) = (unsafe { crate::ime::ActuationTarget::capture_blocking(focus_gen) }) else {
        log::debug!("[imm-romaji] capture 失敗（フォーカス無し）→ ROMAN 補完スキップ");
        return;
    };
    // SAFETY: 同上。
    let outcome = unsafe { crate::ime::set_ime_romaji_mode_for_target_blocking(target, focus_gen) };
    if outcome != crate::ime::ActuationOutcome::Written {
        // INV-14: `Aborted` を成功として扱わない。実機での競合頻度を事後に
        // 測れるよう必ずログに残す（ADR-086 §6 段4）。
        log::debug!("[imm-romaji] ROMAN 補完 {outcome:?} (mechanism={mechanism:?})");
    }
}

/// 同期 writer。`view` は呼び出し元が一度だけ構築したものを使い回す
/// （`tsf_obs()` の二重呼び出しを避ける既存方針をそのまま維持）。
struct SyncChainWriter<'v, 'a> {
    view: &'v ImeControlView<'a>,
}

impl MechanismWriter for SyncChainWriter<'_, '_> {
    fn is_applicable(&self, mechanism: WriteMechanism) -> bool {
        mechanism_is_applicable(mechanism, self.view)
    }

    fn write(&mut self, mechanism: WriteMechanism, open: bool) -> ImeOpenOutcome {
        apply_mechanism(mechanism, open, self.view)
    }
}

/// 機構チェーンを走査して IME を設定するコントローラ。
///
/// **走査規則（`is_applicable` で絞り、`Failed` のときだけ次へ）は
/// `state/actuation_chain.rs` の `Actuation::<Verified>::run_chain` が SSOT で
/// ある**（ADR-089 §2.3）。旧 `apply_iter` はここに inline されていたが、
/// Phase B で ungated 側へ移し、Linux で全数テストできるようにした。
///
/// 旧 `apply_skipping_imm`（async IMM が `Failed` を返した後の再走査）は
/// **撤去した**。ImmCross が chain の要素になったことでフォールスルーは
/// `run_chain_async` が自動的に行う（`runtime/open_chain.rs`）。
///
/// # なぜインスタンスを持たない（すべて関連関数なのか）
///
/// Phase B で `strategies` フィールドが消え（chain は `caps(p, k).chain` /
/// `WriteMechanism::ALL` が SSOT、ADR-089 §2.3・§2.8）、この型は状態を 1 つも
/// 持たない ZST になった。したがって `&self` を取る意味が無く、
/// **`ImeController::apply(..)` のような関連関数として呼ぶ**。
/// 状態を持たせる方向（`ImeStateHub` を直接読む等）は ADR-090 §4.2 で
/// `with_app` 再入を理由に却下済みなので、将来 `self` が要る見込みも無い。
pub(crate) struct ImeController;

/// A-1 shadow の測定点（ADR-090 §2.A 設計案 2、§6 ステップ 5 item 21）。
///
/// 「実機で実際にどの入口が何回 warrant を取れないか」を測るための唯一のログ点。
/// **差分オラクル（`open_warrant.rs`）は 240 通りの組合せを測っているが、
/// 実機でどの組合せが実際に起きるかは測っていない。** A-2（強制）の対象入口は
/// このログがゼロだった入口から順に決める。
///
/// # 「ゼロだったから安全」と「そもそも発火していない」を混同しないこと
///
/// ADR-090 §7-1 が指摘するとおり、`try_force_on_bootstrap` の発火条件
/// （`IME_DETECT_MISS_THRESHOLD` 回連続の検出失敗）は稀であり、1 日の通常利用
/// では一度も踏まない可能性が高い。そのため**授権が下りた場合も 1 行出す**
/// ——入口が発火したこと自体をログに残さないと、`would_have_blocked` の
/// ゼロが「安全」なのか「未測定」なのか区別できない。
pub(crate) fn log_shadow_warrant(chain: &str, order: &ActuationOrder) {
    if order.would_have_blocked() {
        log::info!(
            "[warrant-shadow] chain={chain} open={} origin={:?} would_have_blocked=true \
             (A-1 shadow: 書き込みは止めない。A-2 で強制する際の判断材料)",
            order.open(),
            order.origin(),
        );
    } else {
        log::debug!(
            "[warrant-shadow] chain={chain} open={} origin={:?} warranted",
            order.open(),
            order.origin(),
        );
    }
}

impl ImeController {
    /// コンテキストに応じた機構チェーンを走査して IME を設定する（同期経路）。
    ///
    /// 機構が `Failed` を返した場合（例: `ImmCrossProcessStrategy` の
    /// `SendMessageTimeout` タイムアウト）、次の適用可能な機構へフォールスルーする。
    pub(crate) fn apply(order: ActuationOrder, view: &ImeControlView<'_>) -> ImeOpenOutcome {
        // issue #136 / BUG-90 決定4: この窓は awase が IME actuation を所有しない
        // （InputRelay）。ここが同期経路（`runtime/key_pipeline.rs`/`runtime/mod.rs`
        // の apply_ime_open_with_view 経由も含む）の唯一の合流点であり、
        // `runtime/executor.rs::dispatch_ime_set_open` の早期 gate をバイパスする
        // 経路（`key_pipeline.rs:1065`/`mod.rs:897` 等）を含めてここで確実に止める。
        // ADR-119 参照（gate をここ1点に集約できなかった経緯）。
        if view.focus.profile == crate::focus::class_names::AppImeProfile::InputRelay {
            return ImeOpenOutcome::NotOwned;
        }
        // ADR-090 §2.A A-1: 授権は入口側（`ImeStateHub::issue_actuation_order`）で
        // 発行済み。ここは **shadow モード**なので、授権が下りていなくても
        // 書き込みは止めず `Authorization::LegacyUnwarranted { would_have_blocked }`
        // として記録するだけである（止めるのは A-2、入口ごと・実機ソーク必須）。
        //
        // 宛先: VK 送信機構（GjiDirect / MsImeDirect / KanjiToggle）は SendInput が
        // フォアグラウンドのフォーカスへ配送するため、hwnd を捕獲する余地が
        // 構造的に無い（`SendInput` は宛先引数を取らない）。したがって
        // `FocusImplicit` はこの経路では「未移行」ではなく**機構固有の性質**で
        // ある（ADR-089 §9-19 の訂正）。同期経路で hwnd を持つ唯一の write は
        // ROMAN 補完であり、そちらは `apply_mechanism` が
        // `ActuationTarget::capture_blocking` で捕獲する（Phase C item 12）。
        log_shadow_warrant("sync", &order);
        let actuation = order
            .into_actuation_shadow()
            .verify(VerifiedTarget::FocusImplicit);
        let mut writer = SyncChainWriter { view };
        let outcome = actuation.run_chain(caps_chain_for(view), &mut writer);
        if outcome == ImeOpenOutcome::Failed {
            log::warn!(
                "[apply-ime] all strategies failed for class={}",
                view.focus.class_name
            );
        }
        outcome
    }

    /// `ImmCrossProcessStrategy` が現在のコンテキストで最初に適用可能か。
    ///
    /// dispatch 側で「async 経路 (IMM)」と「sync 経路 (GJI/Kanji)」を branch する
    /// ための判定。`caps(p, k).chain` の先頭が `ImmCross` で、かつそれが
    /// 適用可能であることを要求する。
    ///
    /// **`chain[0]` の同一性チェックが要る**——`Imm32Unavailable` /
    /// `TsfNative` の chain は `[GjiDirect]` / `[MsImeDirect]` なので、
    /// 「chain 中で最初に適用可能な要素の index が 0 か」だけを見ると真になって
    /// しまい、GJI 経路が誤って async 分岐へ流れる。
    pub(crate) fn imm_cross_is_first_applicable(view: &ImeControlView<'_>) -> bool {
        let chain = caps_chain_for(view);
        chain.first() == Some(&WriteMechanism::ImmCross)
            && chain.iter().position(|m| mechanism_is_applicable(*m, view)) == Some(0)
    }
}

/// この view が指す `(profile, IME 種別)` の機構チェーン（ADR-089 §2.8、INV-44）。
///
/// windows-gated な観測型（`AppImeProfile` / `ActiveImeKind`）から ungated な
/// 表の引数（`ImePolicyProfile` / `ImeKindId`）への変換は、それぞれ
/// `focus/class_names.rs` と `tsf/observer.rs` の `From` impl 1 箇所ずつが担う。
fn caps_chain_for(view: &ImeControlView<'_>) -> &'static [WriteMechanism] {
    caps(
        view.focus.profile.into(),
        view.observed.active_ime_kind.into(),
    )
    .chain
}

// 旧 `pub(crate) static CONTROLLER: ImeController` は撤去した。Phase B で
// `strategies` フィールドが消えて `ImeController` が ZST になり、
// 「両所で同じインスタンスを共有する」という当時の理由（状態の共有）が
// 実体を失ったため。呼び出しは `ImeController::apply(..)` 等の関連関数で行う。

// ── キャラクタライゼーションテスト用シーム ──────────────────────────
//
// P2-1 ゴールデンテスト（`tests/ime_key_sequence_golden.rs`）が、リファクタ前の
// 現状の戦略選択を副作用なしで観測するために提供する読み取り専用 API。
// `apply()` は Win32 SendInput 副作用を持つため呼ばない。ここで評価するのは
// 純粋な `is_applicable` のみ（戦略の「選択」だけを固定し、送信キー自体は
// ゴールデンファイル側にソース由来のドキュメントとして注記する）。
// 本番経路（`ImeController::apply` / `runtime/open_chain.rs`）からは参照されない。

impl ImeController {
    /// 与えた view で最初に `is_applicable` を返す機構の名前（`apply` は実行しない）。
    ///
    /// **走査対象は `caps(p, k).chain` ではなく `WriteMechanism::ALL` のままに
    /// してある**（ADR-089 §6 Phase C 実施記録 C-4）。ここは
    /// 「4 機構のうちどれが最初に名乗り出るか」というキャラクタライゼーションの
    /// 観測点であり、golden（`tests/golden/ime_key_sequences.txt`）はその
    /// 出力を固定している。caps chain へ切り替えると `skip_imm=true` の行
    /// （`chain[1..]` が空になる組み合わせがある）が変わって golden が壊れる。
    /// **caps chain が ALL 走査と同じ結論になること**は
    /// `caps_chain_matches_legacy_all_scan` が別途固定する。
    fn first_applicable_name(view: &ImeControlView<'_>) -> &'static str {
        WriteMechanism::ALL
            .iter()
            .find(|m| mechanism_is_applicable(**m, view))
            .map_or("None", |m| m.name())
    }

    /// `ImmCross` を除いた（async IMM が `Failed` を返した後の）フォールバック
    /// 選択の名前。`run_chain_async` が ImmCross の `Failed` 後に辿る範囲と同じ。
    fn first_applicable_name_skipping_imm(view: &ImeControlView<'_>) -> &'static str {
        WriteMechanism::ALL[1..]
            .iter()
            .find(|m| mechanism_is_applicable(**m, view))
            .map_or("None", |m| m.name())
    }
}

/// キャラクタライゼーションテスト用: プリミティブから最小の `ImeControlView` を構築し、
/// 現状のコードが選択する戦略名を返す（`apply` は実行せず `is_applicable` のみ評価）。
///
/// - `active_gji`: `active_ime_kind == GoogleJapaneseInput` かどうか。
/// - `profile`: `"Standard"` / `"Imm32Unavailable"` / `"TsfNative"` のいずれか。
/// - `skip_imm`: `true` なら ImmCross を除いた（IMM 失敗後の）フォールバック選択を返す。
///
/// 戦略選択は `active_ime_kind` と `profile.can_use_imm32_cross_process()` のみに
/// 依存するため、`shadow_on` / `belief_input_mode` はここでは選択に影響しない既定値を渡す。
///
/// # Panics
/// `profile` が `"Standard"` / `"Imm32Unavailable"` / `"TsfNative"` のいずれでもない場合。
#[must_use]
pub fn characterize_strategy(active_gji: bool, profile: &str, skip_imm: bool) -> &'static str {
    use crate::focus::class_names::AppImeProfile;
    use crate::state::ime_decision_view::{ControlLog, FocusFacts, ObservedState};

    let profile = match profile {
        "Standard" => AppImeProfile::Standard,
        "Imm32Unavailable" => AppImeProfile::Imm32Unavailable,
        "TsfNative" => AppImeProfile::TsfNative,
        other => panic!("unknown profile: {other}"),
    };
    let active_ime_kind = if active_gji {
        ActiveImeKind::GoogleJapaneseInput
    } else {
        ActiveImeKind::MicrosoftIme
    };
    let view = ImeControlView {
        focus: FocusFacts {
            class_name: "",
            profile,
            // `is_applicable` の評価だけを行うシームであり、ROMAN 補完
            // （`focus_gen` を使う唯一の経路）は走らない。
            focus_gen: 0,
        },
        observed: ObservedState {
            active_ime_kind,
            ..ObservedState::default()
        },
        control: ControlLog { shadow_on: false },
        belief_input_mode: awase::engine::InputModeState::Unknown,
    };
    if skip_imm {
        ImeController::first_applicable_name_skipping_imm(&view)
    } else {
        ImeController::first_applicable_name(&view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::class_names::AppImeProfile;
    use crate::state::ime_decision_view::{ControlLog, FocusFacts, ObservedState};

    /// 実行時に到達する `(profile, IME 種別)` の全組み合わせ
    /// （golden の `COMBOS` と同じ 6 行。`Plain`/`Unknown` は `AppImeProfile` に
    /// 対応物が無いため対象外——ADR-089 §1.3(e)）。
    const REACHABLE_COMBOS: [(AppImeProfile, ActiveImeKind); 6] = [
        (AppImeProfile::Standard, ActiveImeKind::GoogleJapaneseInput),
        (AppImeProfile::Standard, ActiveImeKind::MicrosoftIme),
        (
            AppImeProfile::Imm32Unavailable,
            ActiveImeKind::GoogleJapaneseInput,
        ),
        (AppImeProfile::Imm32Unavailable, ActiveImeKind::MicrosoftIme),
        (AppImeProfile::TsfNative, ActiveImeKind::GoogleJapaneseInput),
        (AppImeProfile::TsfNative, ActiveImeKind::MicrosoftIme),
    ];

    fn view_for(profile: AppImeProfile, kind: ActiveImeKind) -> ImeControlView<'static> {
        ImeControlView {
            focus: FocusFacts {
                class_name: "",
                profile,
                focus_gen: 0,
            },
            observed: ObservedState {
                active_ime_kind: kind,
                ..ObservedState::default()
            },
            control: ControlLog { shadow_on: false },
            belief_input_mode: awase::engine::InputModeState::Unknown,
        }
    }

    /// issue #136 / BUG-90 決定4: `ImeController::apply`（同期経路の唯一の
    /// 合流点）が `InputRelay` を確実に `NotOwned` で止めることを固定する。
    ///
    /// この経路は `runtime/key_pipeline.rs`（物理 IME キー押下 → shadow toggle
    /// → IME OFF/ON、issue #136 で報告された操作そのもの）と `runtime/mod.rs`
    /// の force-ON 経路がどちらも通る唯一の合流点であり、ここが InputRelay を
    /// 見落とすと、物理キーが `transport.rs::plan` で `Allow`（素通し）される
    /// のと同時に awase 自身も actuate してしまう二重 actuation（BUG-46型）が
    /// 発生する（コードレビューで発見、修正済み）。
    #[test]
    fn apply_returns_not_owned_for_input_relay_without_attempting_any_mechanism() {
        use crate::state::actuation_chain::ActuationOrder;
        use crate::state::app_ime_policy::AppImePolicy;
        use crate::state::event_origin::{EventOrigin, EventSource, Generation};
        use crate::state::force_guard::ForceGuardSet;
        use crate::state::ime_event::{HwndId, ImePolicyProfile};
        use crate::state::intent_store::IntentStore;
        use crate::state::observation_store::ObservationStore;
        use crate::state::open_warrant::WarrantContext;
        use std::time::Instant;

        let intent_store = IntentStore::default();
        let obs = ObservationStore::default();
        let guards = ForceGuardSet::default();
        // ImmCross を要求しても実際には使われない（gate が先に効くため）。
        let policy = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        let ctx = WarrantContext {
            intent_store: &intent_store,
            obs: &obs,
            guards: &guards,
            policy: &policy,
            desired_open: false,
            is_japanese_ime: true,
            now: Instant::now(),
            now_ms: crate::state::TickMs(0),
        };
        let origin = EventOrigin::new(EventSource::Physical, Generation::new(1));
        let order = ActuationOrder::issue(false, HwndId(0x1234), &ctx, origin);

        let view = view_for(
            AppImeProfile::InputRelay,
            ActiveImeKind::GoogleJapaneseInput,
        );
        let outcome = ImeController::apply(order, &view);

        assert_eq!(
            outcome,
            ImeOpenOutcome::NotOwned,
            "InputRelay では ImeController::apply がどの機構も試行せず \
             NotOwned を即返さなければならない（issue #136 / BUG-90 決定4）"
        );
    }

    /// **Phase C の挙動不変性の核**（ADR-089 §7「新設するもの — 全数テスト」）。
    ///
    /// `caps(p, k).chain` を `is_applicable` で絞ったものが、Phase B までの
    /// `WriteMechanism::ALL` 走査（`is_applicable` で絞り、`Failed` のときだけ
    /// 次へ進む）が**実際に到達しうる機構列**と一致することを全数で確認する。
    ///
    /// 「実際に到達しうる列」= `ALL` を `is_applicable` で絞ったうえで、
    /// 最初に `may_return_failed() == false` の機構が現れたところで打ち切った列。
    /// その機構は `Failed` を返さない以上、`falls_through` が偽になり
    /// `run_chain` はそこで必ず終わるためである。
    ///
    /// これが通る限り、`ALL` から `caps` へ切り替えても**送るキーも順序も
    /// 変わらない**。落ちた場合は caps 表か戦略の outcome 集合のどちらかが
    /// 変わったということであり、golden 更新ではなく設計の見直しが要る。
    #[test]
    fn caps_chain_matches_legacy_all_scan() {
        for (profile, kind) in REACHABLE_COMBOS {
            let view = view_for(profile, kind);
            let mut legacy: Vec<WriteMechanism> = Vec::new();
            for mechanism in WriteMechanism::ALL {
                if !mechanism_is_applicable(mechanism, &view) {
                    continue;
                }
                legacy.push(mechanism);
                if !mechanism.may_return_failed() {
                    break;
                }
            }
            let from_caps: Vec<WriteMechanism> = caps_chain_for(&view)
                .iter()
                .copied()
                .filter(|m| mechanism_is_applicable(*m, &view))
                .collect();
            assert_eq!(
                from_caps, legacy,
                "caps chain が Phase B までの ALL 走査と乖離: {profile:?} × {kind:?}"
            );
        }
    }

    /// `caps` チェーンの全要素が、その `(p, k)` で実際に `is_applicable` である
    /// こと（表に「適用され得ない行」を書いていないことの確認、INV-44）。
    #[test]
    fn every_caps_chain_element_is_applicable_in_its_row() {
        for (profile, kind) in REACHABLE_COMBOS {
            let view = view_for(profile, kind);
            for mechanism in caps_chain_for(&view) {
                assert!(
                    mechanism_is_applicable(*mechanism, &view),
                    "{profile:?} × {kind:?}: {mechanism:?} は is_applicable=false"
                );
            }
        }
    }

    /// `imm_cross_is_first_applicable` が Phase B までの `ALL` ベース判定と
    /// 同値であること（async/sync 分岐が変わっていないことの確認）。
    #[test]
    fn imm_cross_first_applicable_is_unchanged_by_caps() {
        for (profile, kind) in REACHABLE_COMBOS {
            let view = view_for(profile, kind);
            let legacy = WriteMechanism::ALL
                .iter()
                .position(|m| mechanism_is_applicable(*m, &view))
                == Some(0);
            assert_eq!(
                ImeController::imm_cross_is_first_applicable(&view),
                legacy,
                "{profile:?} × {kind:?}"
            );
        }
    }
}
