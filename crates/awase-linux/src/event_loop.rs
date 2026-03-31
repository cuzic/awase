//! epoll ベースのイベントループ。
//!
//! evdev キーイベント、timerfd タイマー、D-Bus fd を単一の epoll で多重化し、
//! コールバックにイベント種別を通知する。

use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::time::Duration;

use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};
use nix::sys::time::TimeSpec;

/// イベントの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// evdev デバイス fd が読み取り可能になった。
    KeyEvent,
    /// タイマーが発火した（タイマー ID 付き）。
    Timer(usize),
    /// D-Bus fd が読み取り可能になった。
    Dbus,
}

/// コールバックの戻り値でループの継続・終了を制御する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopAction {
    /// イベントループを続行する。
    Continue,
    /// イベントループを終了する。
    Exit,
}

/// epoll トークン用の定数。
/// タイマー ID はそのまま `TIMER_BASE + id` をトークンとして使う。
const TOKEN_KEY_EVENT: u64 = 0;
const TOKEN_DBUS: u64 = 1;
const TIMER_BASE: u64 = 1000;

/// epoll ベースのイベントループ。
#[derive(Debug)]
pub struct EventLoop {
    epoll: Epoll,
    timers: HashMap<usize, TimerFd>,
}

impl EventLoop {
    /// 新しいイベントループを生成する。
    ///
    /// # Errors
    ///
    /// `epoll_create1` が失敗した場合。
    pub fn new() -> anyhow::Result<Self> {
        let epoll = Epoll::new(EpollCreateFlags::empty())?;
        Ok(Self {
            epoll,
            timers: HashMap::new(),
        })
    }

    /// evdev デバイスの fd を epoll に登録する。
    ///
    /// # Errors
    ///
    /// `epoll_ctl` が失敗した場合。
    pub fn register_evdev<Fd: AsFd>(&self, fd: &Fd) -> anyhow::Result<()> {
        self.epoll
            .add(fd, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_KEY_EVENT))?;
        Ok(())
    }

    /// D-Bus 接続の fd を epoll に登録する。
    ///
    /// # Errors
    ///
    /// `epoll_ctl` が失敗した場合。
    pub fn register_dbus<Fd: AsFd>(&self, fd: &Fd) -> anyhow::Result<()> {
        self.epoll
            .add(fd, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_DBUS))?;
        Ok(())
    }

    /// 任意の fd をトークン付きで epoll に登録する。
    ///
    /// # Errors
    ///
    /// `epoll_ctl` が失敗した場合。
    pub fn register_fd(&self, fd: BorrowedFd<'_>, token: u64) -> anyhow::Result<()> {
        self.epoll
            .add(fd, EpollEvent::new(EpollFlags::EPOLLIN, token))?;
        Ok(())
    }

    /// ワンショットタイマーを設定する。
    ///
    /// 同じ `id` のタイマーが既に存在する場合は上書きする。
    ///
    /// # Errors
    ///
    /// `timerfd_create` または `epoll_ctl` が失敗した場合。
    pub fn set_timer(&mut self, id: usize, duration: Duration) -> anyhow::Result<()> {
        // 既存のタイマーがあれば先に削除
        if self.timers.contains_key(&id) {
            self.kill_timer(id)?;
        }

        let timer = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::empty())?;
        let timespec = TimeSpec::from(duration);
        timer.set(
            Expiration::OneShot(timespec),
            TimerSetTimeFlags::empty(),
        )?;

        let token = TIMER_BASE + id as u64;
        self.epoll
            .add(&timer, EpollEvent::new(EpollFlags::EPOLLIN, token))?;
        self.timers.insert(id, timer);
        Ok(())
    }

    /// タイマーを削除する。
    ///
    /// 指定した `id` のタイマーが存在しない場合は何もしない。
    ///
    /// # Errors
    ///
    /// `epoll_ctl` が失敗した場合。
    pub fn kill_timer(&mut self, id: usize) -> anyhow::Result<()> {
        if let Some(timer) = self.timers.remove(&id) {
            self.epoll.delete(&timer)?;
            // timer は drop されて fd が close される
        }
        Ok(())
    }

    /// イベントループを開始する。
    ///
    /// `callback` にイベント種別を通知し、`LoopAction::Exit` が返されたら終了する。
    ///
    /// # Errors
    ///
    /// `epoll_wait` やタイマーの読み取りが失敗した場合。
    pub fn run<F>(&mut self, mut callback: F) -> anyhow::Result<()>
    where
        F: FnMut(EventKind) -> LoopAction,
    {
        let mut events = [EpollEvent::empty(); 32];

        loop {
            let n = match self.epoll.wait(&mut events, EpollTimeout::NONE) {
                Ok(n) => n,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(e.into()),
            };

            for event in &events[..n] {
                let token = event.data();
                let kind = if token == TOKEN_KEY_EVENT {
                    EventKind::KeyEvent
                } else if token == TOKEN_DBUS {
                    EventKind::Dbus
                } else if token >= TIMER_BASE {
                    let id = (token - TIMER_BASE) as usize;
                    // timerfd を読み取って発火を確認（読まないと再通知される）
                    if let Some(timer) = self.timers.get(&id) {
                        let _ = timer.wait();
                    }
                    EventKind::Timer(id)
                } else {
                    // 未知のトークンは無視
                    continue;
                };

                if callback(kind) == LoopAction::Exit {
                    return Ok(());
                }
            }
        }
    }
}
