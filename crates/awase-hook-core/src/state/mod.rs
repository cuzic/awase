// 新しい crate でも、core の `state/mod.rs` と同じ `#![allow(unreachable_pub)]`（再エクスポートの連鎖を支える `pub mod`）。
#![allow(unreachable_pub)]

//! フック・修飾キー・半角英数トグルの状態（ADR-274 段階 1）。
//!
//! `awase-ime-core` の `state/` から、分割前と同じ `src/state/` 内の相対パスのまま移した（`git mv`、類似度 100%）。
//! 依存は `awase`（ルート crate）と `awase-ime-core` の `vk`・`state::{TickMs, foreground_scope, ime_kind}` だけ。
//!
//! 下位の crate（`awase-ime-core`）の `state` を glob で再公開するので、移したファイルの中の
//! `crate::state::TickMs` などは書き換えなくてよい。殻の `awase-windows` は `crate::state::...` の経路を変えずに使える。

pub use awase_ime_core::state::*;

pub mod hook_state;
// issue #165 (hook_starved) 自己修復の純粋判定。focus_resync_policy と同じ
// ungated パターン（Win32 API を呼ばず Linux でテストできる）。
pub mod hook_watchdog;
// 純粋関数モジュール（conv_classify と同じ ungated パターン）。唯一の呼び出し元
// hook.rs は #[cfg(windows)] のため非 Windows では未使用になる。BUG-41
// （decide_alt_impersonation の KeyUp 状態クリア漏れ）が Windows 実機で初めて
// テストが実行されるまで発見されなかったことの再発防止として、hook.rs から移設。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod alt_impersonation;
// ADR-249(BUG-197): 他アプリが注入した Ctrl の期限付き記録(hook.rs の唯一の呼び出し元。純粋なので Linux でテストできる)。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod foreign_modifier;
// hook.rs (#[cfg(windows)]) の唯一の呼び出し元。alt_impersonation と同じ
// 「純粋判定を Linux でテストできるようにする」移設パターン。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod half_width_alnum;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod win_key_guard;
// ADR-245 追補 2 R2: 戻った窓の最初の打鍵の保持(純粋な状態機械。殻は runtime/resume_hold.rs)。
pub mod half_width_resume_hold;
pub use hook_state::*;
