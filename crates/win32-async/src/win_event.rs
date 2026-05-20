use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// SetWinEventHook で受け取った WinEvent を Stream として公開する。
/// TODO: Task 5 完了後に本実装する（現在はスタブ）
#[derive(Debug)]
#[must_use]
pub struct WinEventStream {
    _event_min: u32,
    _event_max: u32,
}

impl WinEventStream {
    pub const fn new(event_min: u32, event_max: u32) -> Self {
        Self { _event_min: event_min, _event_max: event_max }
    }

    /// 次のイベントを待つ Future を返す。
    pub const fn recv(&mut self) -> WinEventFuture<'_> {
        WinEventFuture { _stream: self }
    }
}

#[derive(Debug)]
pub struct WinEventFuture<'a> {
    _stream: &'a mut WinEventStream,
}

impl Future for WinEventFuture<'_> {
    type Output = Option<WinEventData>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<WinEventData>> {
        // TODO: 本実装
        Poll::Pending
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WinEventData {
    pub event: u32,
    pub hwnd: isize,
    pub object_id: i32,
    pub child_id: i32,
}
