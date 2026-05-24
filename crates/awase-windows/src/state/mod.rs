// pub mod が必要: lib.rs の pub use crate::state::{...} 再エクスポートチェーンを支える。
// unreachable_pub lint はこの再エクスポートパターンを認識できないため抑制する。
#![allow(unreachable_pub)]

pub mod preconditions;
pub use preconditions::*;

pub mod hook_state;
pub use hook_state::*;

pub mod platform_state;
pub use platform_state::PlatformState;
