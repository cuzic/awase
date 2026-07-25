//! IME 制御のアプリ別ポリシー (Step 1.5)
//!
//! reducer / actuator にアプリ固有分岐がベタ書きされる前に、policy オブジェクトへ
//! 隔離する。Step 2B 以降の reducer 本格化のときに polymorphic な参照点として使う。
//!
//! ## 設計原則
//!
//! - **アプリ差分は AppImePolicy に閉じ込める** — reducer 本体に if-else を増やさない
//! - reducer は policy の "what to do" を参照するだけ、policy 自体に分岐ロジックを持たない

use super::ime_actuation::FeedbackPolicy;
use super::ime_event::{ImePolicyProfile, ObservationSource};
use std::time::Duration;

/// Blind feedback（読み戻し不能プロファイル）で actuation を打ち切るまでの試行回数。
///
/// **未検証の初期値**: 実 Windows 実機での soak テストによる裏付けはまだ無い
/// （このサンドボックスでは実測できない）。`5 × backoff`（`backoff` は現状
/// `DRIFT_CORRECTION_THRESHOLD_MS` を再利用、2026-07-25 時点で 400ms なので
/// 最悪 ~2s）を「遅いが最終的に成功する訂正を早すぎる段階で諦めない程度に長く、
/// かつ本当に stuck な状態を延々叩き続けない程度に短い」妥当な出発点として
/// 置いているだけ。この値を変更する場合は `.claude/rules/tuning-constants.md`
/// に従い実機実測の根拠を本文に添えること。
const IME_ACTUATION_BLIND_MAX_ATTEMPTS: u32 = 5;

/// アプリ別の IME 制御ポリシー。
///
/// `AppImeProfile` (クラス名から決定) を基に派生する。
/// reducer / actuator はこのポリシーを参照して挙動を変える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppImePolicy {
    /// 物理 KANJI / VK_F3 / VK_F4 を awase が完全所有するか。
    ///
    /// `true` のとき、物理 KANJI イベントはアプリに渡さない (Step 1/1b 実装済の概念)。
    /// LINE/Qt / Chrome/Edge ともに `true`。WezTerm は `false`。
    pub owns_physical_kanji: bool,

    /// IME 制御の actuator 種別 (ImmCross / VK_KANJI / TSF / Standard)。
    pub actuator_kind: ImeActuatorKind,

    /// フォーカス変更後、observer を信頼できるようになるまでの待ち時間 (ms)。
    pub focus_settle_ms: u64,

    /// このプロファイルの actuation デフォルト feedback（収束確認）方針。
    ///
    /// 読み戻し可能なプロファイル（ImmCross 系）は `Read`、読み戻し手段が
    /// 構造的に無いプロファイル（Imm32Unavailable / TsfNative）は `Blind`。
    pub default_feedback: FeedbackPolicy,
}

/// IME 制御 actuator の種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeActuatorKind {
    /// `set_ime_open_cross_process` (LINE/Qt 等)
    ImmCross,
    /// VK_KANJI SendInput (Chrome/Edge/UWP 等)
    Imm32Unavailable,
    /// TSF SetIMEStatus (WezTerm 等)
    TsfNative,
    /// 標準 IMM32 (Win32 アプリ)
    Standard,
}

impl AppImePolicy {
    /// `ImePolicyProfile` から派生する。
    ///
    /// 各 profile に対応するポリシーを固定する。
    /// Step 1/1b で「ImmCross と Imm32Unavailable は KANJI を awase が所有」と決定済み。
    #[must_use]
    pub const fn from_profile(profile: ImePolicyProfile) -> Self {
        match profile {
            // ImmCross: 通常 Win32、IMM32 クロスプロセスが使えるため awase が所有。
            // Plain / Unknown は安全デフォルト (ImmCross 同等) を使う。
            ImePolicyProfile::ImmCross | ImePolicyProfile::Plain | ImePolicyProfile::Unknown => {
                Self {
                    owns_physical_kanji: true,
                    actuator_kind: ImeActuatorKind::ImmCross,
                    focus_settle_ms: 100,
                    default_feedback: FeedbackPolicy::Read {
                        source: ObservationSource::ImmGetOpenStatus,
                        deadline: Duration::from_millis(
                            crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS,
                        ),
                    },
                }
            }
            ImePolicyProfile::Imm32Unavailable => Self {
                owns_physical_kanji: true,
                actuator_kind: ImeActuatorKind::Imm32Unavailable,
                // Chrome/Edge は GJI/IMM が信頼できないので settle 長め
                focus_settle_ms: 500,
                default_feedback: FeedbackPolicy::Blind {
                    max_attempts: IME_ACTUATION_BLIND_MAX_ATTEMPTS,
                    backoff: Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS),
                },
            },
            ImePolicyProfile::TsfNative => Self {
                // WezTerm 等は TSF が KANJI を正しく処理するため通す
                owns_physical_kanji: false,
                actuator_kind: ImeActuatorKind::TsfNative,
                focus_settle_ms: 200,
                default_feedback: FeedbackPolicy::Blind {
                    max_attempts: IME_ACTUATION_BLIND_MAX_ATTEMPTS,
                    backoff: Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS),
                },
            },
        }
    }

    /// `ImmCross` プロファイルのデフォルト値。初期化時 / 不明 profile 時に使う。
    #[must_use]
    pub const fn standard() -> Self {
        Self::from_profile(ImePolicyProfile::ImmCross)
    }
}

impl Default for AppImePolicy {
    fn default() -> Self {
        Self::standard()
    }
}

impl From<ImePolicyProfile> for AppImePolicy {
    fn from(profile: ImePolicyProfile) -> Self {
        Self::from_profile(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imm_cross_owns_physical_kanji() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        assert!(p.owns_physical_kanji);
        assert_eq!(p.actuator_kind, ImeActuatorKind::ImmCross);
    }

    #[test]
    fn imm32_unavailable_owns_physical_kanji() {
        // Step 1/1b の決定: Chrome/Edge も awase が KANJI を所有
        let p = AppImePolicy::from_profile(ImePolicyProfile::Imm32Unavailable);
        assert!(p.owns_physical_kanji);
        assert_eq!(p.actuator_kind, ImeActuatorKind::Imm32Unavailable);
    }

    #[test]
    fn tsf_native_does_not_own_physical_kanji() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        assert!(!p.owns_physical_kanji);
        assert_eq!(p.actuator_kind, ImeActuatorKind::TsfNative);
    }

    #[test]
    fn unknown_falls_back_to_imm_cross() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::Unknown);
        assert!(p.owns_physical_kanji);
        assert_eq!(p.actuator_kind, ImeActuatorKind::ImmCross);
    }

    #[test]
    fn default_is_standard() {
        assert_eq!(AppImePolicy::default(), AppImePolicy::standard());
    }

    #[test]
    fn imm_cross_default_feedback_is_read_via_imm_get_open_status() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        assert!(matches!(
            p.default_feedback,
            FeedbackPolicy::Read {
                source: ObservationSource::ImmGetOpenStatus,
                deadline,
            } if deadline
                == Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
        ));
    }

    #[test]
    fn plain_and_unknown_share_imm_cross_read_feedback() {
        for profile in [ImePolicyProfile::Plain, ImePolicyProfile::Unknown] {
            let p = AppImePolicy::from_profile(profile);
            assert!(matches!(
                p.default_feedback,
                FeedbackPolicy::Read {
                    source: ObservationSource::ImmGetOpenStatus,
                    ..
                }
            ));
        }
    }

    #[test]
    fn imm32_unavailable_default_feedback_is_blind() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::Imm32Unavailable);
        assert!(matches!(
            p.default_feedback,
            FeedbackPolicy::Blind {
                max_attempts: IME_ACTUATION_BLIND_MAX_ATTEMPTS,
                backoff,
            } if backoff == Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
        ));
    }

    #[test]
    fn tsf_native_default_feedback_is_blind() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        assert!(matches!(
            p.default_feedback,
            FeedbackPolicy::Blind {
                max_attempts: IME_ACTUATION_BLIND_MAX_ATTEMPTS,
                backoff,
            } if backoff == Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
        ));
    }

    #[test]
    fn from_trait_impl() {
        let p: AppImePolicy = ImePolicyProfile::Imm32Unavailable.into();
        assert_eq!(p.actuator_kind, ImeActuatorKind::Imm32Unavailable);
    }
}
