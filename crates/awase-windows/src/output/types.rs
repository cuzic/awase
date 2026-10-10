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
///
/// 核の型 `InjectionMode` への `From<(InjectionHint, AppKind)>` は書けない（`InjectionHint` が殻の型、
/// タプルは外部の型なので孤児規則に反する。ADR-229 D4）ため関数にした。
pub(crate) fn injection_mode_for(hint: InjectionHint, app_kind: AppKind) -> InjectionMode {
    match hint {
        InjectionHint::ForceTsf => InjectionMode::Tsf,
        InjectionHint::ForceVk => InjectionMode::Vk,
        InjectionHint::Default => {
            if app_kind == AppKind::TsfNative {
                InjectionMode::Vk
            } else {
                InjectionMode::Unicode
            }
        }
    }
}
