//! IME 制御のアプリ別ポリシー (Step 1.5)
//!
//! reducer / actuator にアプリ固有分岐がベタ書きされる前に、policy オブジェクトへ
//! 隔離する。Step 2B 以降の reducer 本格化のときに polymorphic な参照点として使う。
//!
//! ## 設計原則
//!
//! - **アプリ差分は AppImePolicy に閉じ込める** — reducer 本体に if-else を増やさない
//! - reducer は policy の "what to do" を参照するだけ、policy 自体に分岐ロジックを持たない

use super::ime_event::ImePolicyProfile;

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
            ImePolicyProfile::ImmCross => Self {
                // 通常 Win32: IMM32 クロスプロセスが使えるため awase が所有
                owns_physical_kanji: true,
                actuator_kind: ImeActuatorKind::ImmCross,
                focus_settle_ms: 100,
            },
            ImePolicyProfile::Imm32Unavailable => Self {
                owns_physical_kanji: true,
                actuator_kind: ImeActuatorKind::Imm32Unavailable,
                // Chrome/Edge は GJI/IMM が信頼できないので settle 長め
                focus_settle_ms: 500,
            },
            ImePolicyProfile::TsfNative => Self {
                // WezTerm 等は TSF が KANJI を正しく処理するため通す
                owns_physical_kanji: false,
                actuator_kind: ImeActuatorKind::TsfNative,
                focus_settle_ms: 200,
            },
            // Plain / Unknown は安全デフォルト (ImmCross 同等) を使う
            ImePolicyProfile::Plain | ImePolicyProfile::Unknown => Self {
                owns_physical_kanji: true,
                actuator_kind: ImeActuatorKind::ImmCross,
                focus_settle_ms: 100,
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
    fn from_trait_impl() {
        let p: AppImePolicy = ImePolicyProfile::Imm32Unavailable.into();
        assert_eq!(p.actuator_kind, ImeActuatorKind::Imm32Unavailable);
    }
}
