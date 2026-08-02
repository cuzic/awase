use crate::focus::classifier::InjectionHint;
use crate::focus::AppKind;

/// 出力注入モードの型定義は `state::injection_mode`（ungated）へ移設した。
/// `InjectionHint` が windows-gated のため、この `From` 実装だけはここに残す
/// （SSOT 二重化を避けるため、`InjectionMode` の定義自体はミラーしない）。
pub(crate) use crate::state::injection_mode::InjectionMode;

/// `InjectionHint` と `AppKind` から `InjectionMode` を決定する。
///
/// 優先順位:
///   1. `InjectionHint::ForceTsf` → Tsf
///   2. `InjectionHint::ForceVk`  → Vk
///   3. `AppKind::TsfNative`      → Vk
///   4. それ以外 (Win32 / Uwp)   → Unicode
impl From<(InjectionHint, AppKind)> for InjectionMode {
    fn from((hint, app_kind): (InjectionHint, AppKind)) -> Self {
        match hint {
            InjectionHint::ForceTsf => Self::Tsf,
            InjectionHint::ForceVk => Self::Vk,
            InjectionHint::Default => {
                if app_kind == AppKind::TsfNative {
                    Self::Vk
                } else {
                    Self::Unicode
                }
            }
        }
    }
}
