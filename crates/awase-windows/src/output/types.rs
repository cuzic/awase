use crate::focus::classifier::InjectionHint;

/// 出力注入モードの型定義は `state::injection_mode`（ungated）へ移設した。
/// `InjectionHint` が windows-gated のため、この `From` 実装だけはここに残す
/// （SSOT 二重化を避けるため、`InjectionMode` の定義自体はミラーしない）。
pub(crate) use crate::state::injection_mode::InjectionMode;

/// `InjectionHint` から `InjectionMode` を決定する。
///
/// 優先順位:
///   1. `InjectionHint::ForceTsf` → Tsf
///   2. `InjectionHint::Default`  → Vk（ADR-276 で Unicode 注入を撤去。旧 `ForceVk` は 2026-10-22 に退役）
///
/// 核の型 `InjectionMode` への `From<InjectionHint>` は書けない（`InjectionHint` が殻の型、
/// `InjectionMode` が別 crate の型なので孤児規則に反する。ADR-229 D4）ため関数にした。
pub(crate) const fn injection_mode_for(hint: InjectionHint) -> InjectionMode {
    match hint {
        InjectionHint::ForceTsf => InjectionMode::Tsf,
        InjectionHint::Default => InjectionMode::Vk,
    }
}
