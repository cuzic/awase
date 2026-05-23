//! Windows ネイティブのワーカースレッド管理クレート。
//!
//! # 概要
//! `WorkerThread` は Windows の手動リセットイベントオブジェクトを使って
//! ワーカースレッドに停止を通知する RAII ラッパー。
//! Drop 時に自動でシャットダウンシグナルを送り、join する。
//!
//! # 使い方
//! ```ignore
//! let worker = WorkerThread::spawn("my-worker", |token| {
//!     loop {
//!         do_work();
//!         // thread::sleep の代わり: シャットダウン通知で早期に抜ける
//!         if token.sleep_ms(10).is_break() { break; }
//!     }
//! });
//! // worker が drop されると自動的に停止・join される
//! ```

#![cfg(windows)]
#![allow(unsafe_code)]

use std::ops::ControlFlow;
use std::sync::Arc;
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

// ── 内部: Windows Event オブジェクトの Arc ラッパー ──

struct EventHandle(HANDLE);

// HANDLE はプロセス内グローバルリソースなのでスレッド間送信可能。
unsafe impl Send for EventHandle {}
unsafe impl Sync for EventHandle {}

impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

// ── 公開 API ──

/// ワーカースレッドに渡すシャットダウントークン。
///
/// `sleep_ms` でスリープしながらシャットダウン通知を待てる。
#[derive(Clone)]
pub struct ShutdownToken(Arc<EventHandle>);

impl ShutdownToken {
    /// `ms` ミリ秒スリープする。
    ///
    /// シャットダウンが通知された場合は早期に `ControlFlow::Break(())` を返す。
    /// タイムアウトした場合は `ControlFlow::Continue(())` を返す。
    pub fn sleep_ms(&self, ms: u32) -> ControlFlow<()> {
        let result = unsafe { WaitForSingleObject((self.0).0, ms) };
        if result == WAIT_OBJECT_0 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    /// シャットダウンが既に通知されているか確認する（ノンブロッキング）。
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        unsafe { WaitForSingleObject((self.0).0, 0) == WAIT_OBJECT_0 }
    }
}

impl std::fmt::Debug for ShutdownToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownToken").finish_non_exhaustive()
    }
}

/// ワーカースレッドの RAII ハンドル。
///
/// Drop 時にシャットダウンイベントを発火し、スレッドの終了を join して待つ。
pub struct WorkerThread {
    event: Arc<EventHandle>,
    handle: Option<JoinHandle<()>>,
}

impl WorkerThread {
    /// 名前付きワーカースレッドを起動する。
    ///
    /// `f` は [`ShutdownToken`] を受け取り、`token.sleep_ms()` で停止を検知できる。
    ///
    /// # Panics
    /// `CreateEventW` または `thread::Builder::spawn` が失敗した場合。
    pub fn spawn(name: &str, f: impl FnOnce(ShutdownToken) + Send + 'static) -> Self {
        // 手動リセット、初期状態は非シグナル
        let raw = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        assert!(!raw.is_null(), "CreateEventW failed");

        let event = Arc::new(EventHandle(raw));
        let token = ShutdownToken(Arc::clone(&event));

        let handle = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || f(token))
            .unwrap_or_else(|e| panic!("failed to spawn worker thread '{name}': {e}"));

        Self { event, handle: Some(handle) }
    }

    /// シャットダウンを通知してスレッド終了を待つ。
    ///
    /// Drop でも同じ処理が行われるが、明示的に呼ぶ場合に使う。
    pub fn shutdown(mut self) {
        self.do_shutdown();
    }

    fn do_shutdown(&mut self) {
        unsafe { SetEvent((self.event).0) };
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for WorkerThread {
    fn drop(&mut self) {
        self.do_shutdown();
    }
}

impl std::fmt::Debug for WorkerThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerThread")
            .field("running", &self.handle.as_ref().is_some_and(|h| !h.is_finished()))
            .finish_non_exhaustive()
    }
}
