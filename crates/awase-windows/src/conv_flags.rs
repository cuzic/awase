//! IME 変換モード(conversion mode)のビットフラグ(ADR-271 段階 A)。
//!
//! `imm.rs` は Windows 専用のため、Linux でビルドする FSM(`tsf/ime_mode_fsm.rs` など)が
//! 使う値だけをここへ置き、`imm` から再エクスポートする。

/// ローマ字入力モードフラグ（0x0010）
pub(crate) const IME_CMODE_ROMAN: u32 = 0x0010;
/// 日本語ネイティブ入力モードフラグ（0x0001）
pub(crate) const IME_CMODE_NATIVE: u32 = 0x0001;
/// カタカナ入力モードフラグ（0x0002）
pub(crate) const IME_CMODE_KATAKANA: u32 = 0x0002;
/// 全角モードフラグ（0x0008）
pub(crate) const IME_CMODE_FULLSHAPE: u32 = 0x0008;
