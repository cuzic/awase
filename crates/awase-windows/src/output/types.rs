use awase::types::AppKind;
use crate::focus::classifier::InjectionHint;

/// 出力注入モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionMode {
    /// Unicode 直接注入（Win32/UWP デフォルト）
    Unicode,
    /// VK Batched 注入（Chrome/Edge/Electron — IME composition 経由）
    Vk,
    /// VK Sequential 注入（WezTerm — TSF 直結アプリ向け）
    Tsf,
}

/// `InjectionHint` と `AppKind` から `InjectionMode` を決定する純粋関数。
///
/// 優先順位:
///   1. `InjectionHint::ForceTsf` → Tsf
///   2. `InjectionHint::ForceVk`  → Vk
///   3. `AppKind::TsfNative`      → Vk
///   4. それ以外 (Win32 / Uwp)   → Unicode
pub fn resolve_injection_mode_from(hint: InjectionHint, app_kind: AppKind) -> InjectionMode {
    match hint {
        InjectionHint::ForceTsf => InjectionMode::Tsf,
        InjectionHint::ForceVk  => InjectionMode::Vk,
        InjectionHint::Default  => {
            if app_kind == AppKind::TsfNative {
                InjectionMode::Vk
            } else {
                InjectionMode::Unicode
            }
        }
    }
}
