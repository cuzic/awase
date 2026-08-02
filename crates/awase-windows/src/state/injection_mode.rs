//! 出力注入モードの型定義（`output/types.rs` から移設、ADR-082 決定1実施記録の
//! 次の一歩・BUG-33）。
//!
//! `gji_fsm`（`tsf/gji_fsm.rs`、windows crate 依存ゼロ）が `InjectionMode` を
//! フィールドとして持つため、`gji_fsm` を Linux で実行可能にするには
//! `InjectionMode` 自体も ungated である必要がある。`InjectionHint` から
//! `InjectionMode` を導出する `From` 実装は `InjectionHint`
//! （`focus::classifier`、windows-gated）に依存するため `output/types.rs` に残す
//! （SSOT を二重化しないため、`InjectionMode` の定義はここ1箇所のみ）。

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
