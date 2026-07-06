#![cfg(windows)]

// 旧 atomic_watcher（AtomicU32 監視 Future + notify_all）は 2026-07-06 の
// 到達不能パス監査で撤去 — AtomicWatcher::new の呼び出しがワークスペースに
// 存在せず、notify_all は毎回空の waker リストを drain するだけだった。
// event-driven 待機はポーリング方式（ChangeCounter::baseline）に置換済み。
pub mod offload;
pub mod race_timeout;
pub mod sleep;
pub mod thread_timeout;

pub use offload::{offload, offload_timeout};
pub use race_timeout::race_with_timeout;
pub use sleep::sleep_ms;
pub use thread_timeout::run_with_timeout;

/// winmsg-executor の `block_on` を再エクスポート。
/// メッセージループを内部で動かしながら Future を完了まで実行する。
pub use winmsg_executor::block_on;

/// winmsg-executor の `spawn_local` を再エクスポート。
/// 現在のスレッドのメッセージループで並行して実行する Future をスポーンする。
pub use winmsg_executor::spawn_local;
