#![cfg(windows)]

pub mod atomic_watcher;
pub mod single_thread_cell;
pub mod sleep;
pub mod thread_timeout;
pub mod win_event;

pub use atomic_watcher::{notify_all, AtomicWatcher};
pub use single_thread_cell::SingleThreadCell;
pub use sleep::sleep_ms;
pub use thread_timeout::run_with_timeout;
pub use win_event::WinEventStream;

/// winmsg-executor の `block_on` を再エクスポート。
/// メッセージループを内部で動かしながら Future を完了まで実行する。
pub use winmsg_executor::block_on;

/// winmsg-executor の `spawn_local` を再エクスポート。
/// 現在のスレッドのメッセージループで並行して実行する Future をスポーンする。
pub use winmsg_executor::spawn_local;
