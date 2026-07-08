//! IME ON/OFF 制御の「戦略選択」と「送信キー」を宣言的に集約する state 層の policy。
//!
//! # 背景（防ぐバグクラス）
//! `ime_controller.rs` の各 Strategy が、適用条件（`is_applicable`）と送る VK を手続き的に
//! ハードコードしてきた結果、「IME 種別 × プロファイル × 操作 → キー列」の変更のたびに
//! 実機トライ&リバートが発生した（P2-1 ゴールデン `tests/golden/ime_key_sequences.txt`、
//! revert 24 件）。判断を1モジュールへ集約し、変更を「1行の diff」としてレビュー可能にする。
//!
//! # このモジュールが担う判断 / 担わない判断
//! - **担う**:
//!   - 戦略選択の適用条件（`*_applicable` 述語）。`ime_controller.rs` の各 `is_applicable` が
//!     これを引くだけになる。P2-1 ゴールデンが固定する戦略選択を挙動不変で集約する。
//!   - 冪等モードキーを送る2機構（GjiDirect / MsImeDirect）の `operation → 送信 VK`
//!     （[`ime_key_for`]）。キーは必ず [`crate::vk`] の名前付き定数（VK hex 直書き禁止, D-1）。
//! - **担わない（呼び出し側 = `ime_controller.rs` に現行ロジックを残す動的判断）**:
//!   - `ImmCrossProcessStrategy` の `ImmSetOpenStatus` クロスプロセス API（VK を送らない）。
//!   - `KanjiToggleStrategy` の `post_kanji_toggle_to_focused`（VK_KANJI をフォーカス窓へ送る
//!     専用経路。`send_ime_mode_key` とは送信機構が異なるためこの表には載せない）。
//!   - `shadow_on` スキップ（GjiDirect ON）・KATAKANA conv スキップ（MsImeDirect ON）・
//!     ROMAN pre-mode（`set_ime_romaji_mode`）・フォールバック前の実状態確認（`3510a08`,
//!     [[feedback_immcross_fallback_state_check]]）。いずれも observation / conv 依存の動的判断。
//!
//! # アプリ分岐を持ち込まない（C-4）
//! 述語は `AppImeProfile` / `ActiveImeKind` までの抽象で判断する。アプリ名文字列や class_name
//! マッチはここに新設しない（それらは focus 層の classifier が所有する）。

use crate::focus::class_names::AppImeProfile;
use crate::tsf::observer::ActiveImeKind;
use crate::tsf::warmup::probe_fsm::TransmitTarget;
use crate::vk::{VK_DBE_HIRAGANA, VK_IME_OFF, VK_IME_ON};
use awase::types::VkCode;

// ── 戦略選択の適用条件（ime_controller の is_applicable が引く述語）─────────────────

/// `ImmCrossProcessStrategy` の適用条件: IMM32 クロスプロセス制御が使えるプロファイルか。
#[must_use]
pub(crate) const fn imm_cross_applicable(profile: AppImeProfile) -> bool {
    profile.can_use_imm32_cross_process()
}

/// `GjiDirectStrategy` の適用条件: GJI が検出済みか（全プロファイルで適用）。
#[must_use]
pub(crate) const fn gji_direct_applicable(kind: ActiveImeKind) -> bool {
    matches!(kind, ActiveImeKind::GoogleJapaneseInput)
}

/// `MsImeDirectStrategy` の適用条件: MS-IME 検出済み かつ IMM32 クロスプロセス不可。
#[must_use]
pub(crate) const fn ms_ime_direct_applicable(kind: ActiveImeKind, profile: AppImeProfile) -> bool {
    matches!(kind, ActiveImeKind::MicrosoftIme) && !profile.can_use_imm32_cross_process()
}

// KanjiToggleStrategy は最終フォールバックで常に true。自明なため述語関数は設けない。

// ── 送信キー表（冪等モードキー機構）──────────────────────────────────────────────

/// IME 制御の操作。controller レベルでは開閉の2値。
///
/// カタカナ / 英数の細分は conv ビット依存の動的判断として `ime_controller.rs` 側に残す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImeOperation {
    /// IME ON（ひらがな入力へ）。
    Open,
    /// IME OFF（DirectInput / 半角英数へ）。
    Close,
}

impl ImeOperation {
    /// `apply(open, ..)` の `open: bool` を操作に変換する。
    #[must_use]
    pub(crate) const fn from_open(open: bool) -> Self {
        if open {
            Self::Open
        } else {
            Self::Close
        }
    }
}

/// `send_ime_mode_key` で冪等モードキーを送る適用機構。
///
/// `ImmCrossProcessStrategy`（API 呼び出し）と `KanjiToggleStrategy`（専用フォーカス窓経路）は
/// 送信機構が異なるためこの enum に含めない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyMechanism {
    /// `GjiDirectStrategy`: VK_IME_ON / VK_IME_OFF（GJI が TSF 層で処理する冪等キー）。
    GjiDirect,
    /// `MsImeDirectStrategy`: VK_DBE_HIRAGANA / VK_IME_OFF（MS-IME 冪等キー）。
    MsImeDirect,
}

/// `(機構, 操作) → 送信 VK` の宣言的テーブル。
///
/// 呼び出し側は `crate::ime::send_ime_mode_key(ime_key_for(..))` で送る。各行の挙動根拠
/// （コミットハッシュ）は P2-1 ゴールデンに集約済み。キー変更はこの match 1行の diff になる。
#[must_use]
// GjiDirect/Close と MsImeDirect/Close は現在同じ VK_IME_OFF を送るが、この表は
// 「1行 = 1 (機構, 操作) の送信キー根拠（コミットハッシュ付き）」という宣言的テーブル
// 設計を意図している。IME OFF キー選択は過去に複数回反転しており
// (.claude/rules/experiment-logging.md 参照)、行を統合すると片方だけキーを変える将来の
// 変更が 1 行 diff で済まなくなるため、意図的に統合しない。
#[allow(clippy::match_same_arms)]
pub(crate) const fn ime_key_for(mechanism: KeyMechanism, op: ImeOperation) -> VkCode {
    use ImeOperation::{Close, Open};
    use KeyMechanism::{GjiDirect, MsImeDirect};
    match (mechanism, op) {
        // GjiDirect: post_gji_ime_on/off 相当（GJI+TsfNative の OFF も VK_IME_OFF, 489cdf1）。
        (GjiDirect, Open) => VK_IME_ON,
        (GjiDirect, Close) => VK_IME_OFF,
        // MsImeDirect: ON=post_ms_ime_on(VK_DBE_HIRAGANA), OFF=post_ime_off_direct(VK_IME_OFF, 48a667a)。
        (MsImeDirect, Open) => VK_DBE_HIRAGANA,
        (MsImeDirect, Close) => VK_IME_OFF,
    }
}

// ── warmup 犠牲キー・gate ポリシー（output/probe_io.rs の dispatch_probe_actions が引く）──
//
// cold-start warmup で「Chrome か TSF/WezTerm か」で分岐していたキー選択・gate 判定を集約する。
// 実行（SendInput・FSM 構築）は probe_io 側に残し、ここは「どのキー種別か / gate を尊重するか」
// の判断だけを担う。各行の実機確定根拠は 22c3905（Chrome=VK_A+BS / TSF=VK_IME_OFF→ON 分岐確定）。

/// StartSacrificialWarmup が送る犠牲キーの種別。target ごとに実機で確定している（22c3905）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SacrificialWarmupKey {
    /// Chrome: VK_A + BS を同一 SendInput バッチ（文字フラッシュ防止）。
    /// VK_IME_OFF は Chrome TSF context を壊すため使用不可。
    VkAThenBackspace,
    /// TSF/WezTerm: VK_IME_OFF→VK_IME_ON（vim 安全プローブ、Off→On が GJI write を増やす）。
    ImeOffThenOn,
}

/// warmup 犠牲キーの種別を target から決める。
#[must_use]
pub(crate) const fn sacrificial_warmup_key(target: TransmitTarget) -> SacrificialWarmupKey {
    match target {
        TransmitTarget::Chrome => SacrificialWarmupKey::VkAThenBackspace,
        TransmitTarget::Tsf => SacrificialWarmupKey::ImeOffThenOn,
    }
}

/// warmup 送信で TSF gate=Bypass を尊重すべき target か。
///
/// Chrome は常に gate=Bypass 運用（IME を直接制御せずバッチ送信）のため、gate による送信スキップを
/// してはならない（スキップすると Chrome では一切送信されない）。TSF/WezTerm は Bypass 時に見送る。
/// probe_io の複数箇所で重複していた `target != Chrome` 判定をここへ集約する。
#[must_use]
pub(crate) const fn warmup_respects_bypass_gate(target: TransmitTarget) -> bool {
    !matches!(target, TransmitTarget::Chrome)
}

/// SacrificialResend で犠牲 VK_A を消す cleanup BS を要する target か。
///
/// Chrome は VK_A+BS を atomic batch で送信済みのため cleanup BS 不要。
/// 呼び出し側で `skip_cleanup_bs`（VK_A を送らない ImeOffOnWarmupFsm）と AND すること。
#[must_use]
pub(crate) const fn target_needs_sacrificial_cleanup_bs(target: TransmitTarget) -> bool {
    !matches!(target, TransmitTarget::Chrome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_open_maps_bool() {
        assert_eq!(ImeOperation::from_open(true), ImeOperation::Open);
        assert_eq!(ImeOperation::from_open(false), ImeOperation::Close);
    }

    #[test]
    fn gji_direct_keys() {
        assert_eq!(
            ime_key_for(KeyMechanism::GjiDirect, ImeOperation::Open),
            VK_IME_ON
        );
        assert_eq!(
            ime_key_for(KeyMechanism::GjiDirect, ImeOperation::Close),
            VK_IME_OFF
        );
    }

    #[test]
    fn ms_ime_direct_keys() {
        assert_eq!(
            ime_key_for(KeyMechanism::MsImeDirect, ImeOperation::Open),
            VK_DBE_HIRAGANA
        );
        assert_eq!(
            ime_key_for(KeyMechanism::MsImeDirect, ImeOperation::Close),
            VK_IME_OFF
        );
    }

    // ── 戦略選択述語が現行 is_applicable と一致することを固定（ゴールデンの一次診断補助）──

    #[test]
    fn imm_cross_only_standard() {
        assert!(imm_cross_applicable(AppImeProfile::Standard));
        assert!(!imm_cross_applicable(AppImeProfile::Imm32Unavailable));
        assert!(!imm_cross_applicable(AppImeProfile::TsfNative));
    }

    #[test]
    fn gji_direct_any_profile_when_gji() {
        assert!(gji_direct_applicable(ActiveImeKind::GoogleJapaneseInput));
        assert!(!gji_direct_applicable(ActiveImeKind::MicrosoftIme));
    }

    #[test]
    fn ms_ime_direct_requires_non_imm_cross() {
        // MS-IME × 非 Standard のみ true。
        assert!(ms_ime_direct_applicable(
            ActiveImeKind::MicrosoftIme,
            AppImeProfile::Imm32Unavailable
        ));
        assert!(ms_ime_direct_applicable(
            ActiveImeKind::MicrosoftIme,
            AppImeProfile::TsfNative
        ));
        assert!(!ms_ime_direct_applicable(
            ActiveImeKind::MicrosoftIme,
            AppImeProfile::Standard
        ));
        assert!(!ms_ime_direct_applicable(
            ActiveImeKind::GoogleJapaneseInput,
            AppImeProfile::TsfNative
        ));
    }

    // ── warmup ポリシー ──

    #[test]
    fn sacrificial_key_per_target() {
        assert_eq!(
            sacrificial_warmup_key(TransmitTarget::Chrome),
            SacrificialWarmupKey::VkAThenBackspace
        );
        assert_eq!(
            sacrificial_warmup_key(TransmitTarget::Tsf),
            SacrificialWarmupKey::ImeOffThenOn
        );
    }

    #[test]
    fn only_non_chrome_respects_bypass_gate() {
        assert!(!warmup_respects_bypass_gate(TransmitTarget::Chrome));
        assert!(warmup_respects_bypass_gate(TransmitTarget::Tsf));
    }

    #[test]
    fn only_non_chrome_needs_cleanup_bs() {
        assert!(!target_needs_sacrificial_cleanup_bs(TransmitTarget::Chrome));
        assert!(target_needs_sacrificial_cleanup_bs(TransmitTarget::Tsf));
    }
}
