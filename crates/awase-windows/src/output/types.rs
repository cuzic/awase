use crate::focus::classifier::InjectionHint;
use awase::types::AppKind;

/// 出力注入モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InjectionMode {
    /// Unicode 直接注入（Win32/UWP デフォルト）
    Unicode,
    /// VK Batched 注入（Chrome/Edge/Electron — IME composition 経由）
    Vk,
    /// VK Sequential 注入（WezTerm — TSF 直結アプリ向け）
    Tsf,
}

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
