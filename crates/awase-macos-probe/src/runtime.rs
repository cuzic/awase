//! macOS 実行基盤のプリミティブ群。
//!
//! メインスレッドの `NSApplication`、専用 `CFRunLoop` ワーカスレッド、autorelease
//! pool、shutdown signal、observer 生存期間管理を提供する。設計は
//! project_macos_port_strategy.md の「UIループ（AppKit main thread中心）」と
//! project_macos_probe_interfaces.md を参照。
//!
//! このモジュールは後続タスクが土台とする再利用可能なプリミティブ群を提供する。
//! それ自体はイベントの監視・送出を行わず、以下を提供するだけに徹する:
//!
//! - [`ShutdownSignal`]: 各モジュール（tap.rs / focus.rs / sleep-wake 等）が poll して
//!   終了要求を検知するための共有フラグ。SIGINT/Ctrl-C の配線はここでは行わない
//!   （後続の統合パスで main.rs が [`ShutdownSignal::trigger`] を呼ぶ形にする）。
//! - [`run_on_main_thread`] / [`MainAppHandle`]: メインスレッド上の `NSApplication`。
//! - [`spawn_run_loop_thread`] / [`RunLoopThreadHandle`]: 専用 `CFRunLoop` を持つワーカ
//!   スレッド。tap.rs は将来 CGEventTap をこの run loop に載せて回す。
//! - [`with_autorelease_pool`]: autorelease されたオブジェクトを扱うコールバック本体を
//!   包む scoped autorelease pool。
//! - [`workspace_notification_center`]: 共有 `NSWorkspace` の notification center。
//!   focus.rs や sleep/wake が observer を登録する先。別の notification center に
//!   登録するとアプリ切替・wake 通知を取りこぼすため、必ずこの accessor を使う。
//! - [`run_until_shutdown`]: [`ShutdownSignal`] とワーカスレッドの寿命を結び付け、
//!   フラグが立ったらワーカを停止させる driver ヘルパ。

// CFRunLoop のデフォルトモード定数（`kCFRunLoopDefaultMode`）読み取りに unsafe が必須。
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// ワーカスレッドが shutdown を検知するまでの最大待ち時間。CFRunLoop の 1 スライスの
/// 長さでもある。短くすると shutdown 応答が速くなるが wake が増える。50ms は診断
/// バイナリの用途では十分速く、かつ空 run loop 時のスピンも抑えられる妥協点。
const POLL_SLICE: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// ShutdownSignal（プラットフォーム非依存・Linux で単体テスト可能）
// ---------------------------------------------------------------------------

/// プロセス全体の協調的終了フラグ。`Arc<AtomicBool>` を包むだけの薄い型。
///
/// clone は同一フラグを共有する（`Arc` の clone）ため、あるスレッドで
/// [`trigger`](Self::trigger) すると全 clone の [`is_shutdown`](Self::is_shutdown) が
/// `true` を返す。tap.rs / focus.rs 等のワーカはこれを poll して自発的に停止する。
///
/// 注意: ここでは実際の OS シグナル（SIGINT/Ctrl-C）ハンドリングは配線しない。
/// 手動の [`trigger`](Self::trigger) のみを公開し、どう呼ぶかは後続の統合パスで
/// main.rs 側が決める（新規クレート依存 `ctrlc`/`signal-hook` を避け、main.rs を
/// 触らずに済ませるため）。
// tap.rs / sleep_wake.rs は Phase M0 では各自の内部 Arc<AtomicBool> で停止管理を
// 完結させており（session/health ハンドルと一体化しているため）、この汎用プリミティブ
// 経由の統合はまだしていない。main.rs の SIGINT 配線時に再検討する。
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct ShutdownSignal {
    flag: Arc<AtomicBool>,
}

#[allow(dead_code)]
impl ShutdownSignal {
    /// 未 trigger 状態の新規シグナルを作る。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 終了が要求されているか。
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// 終了を要求する。全 clone から観測できる。冪等。
    pub fn trigger(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// autorelease pool
// ---------------------------------------------------------------------------

/// scoped autorelease pool の中で `f` を実行する。
///
/// objc2 0.6 は RAII ガード型ではなく scoped なクロージャ API
/// (`objc2::rc::autoreleasepool`) を採用している（pool の drain 順序を型で保証して
/// 健全性を担保するため、スタック上に自由に置ける RAII 型は提供されない）。ここでは
/// その事実を隠蔽し、autorelease されたオブジェクトを生成するコールバック本体
/// （tap callback / 通知ハンドラ等）を包む用途に絞った薄いラッパを提供する。
///
/// 非 macOS では pool は存在しないので単に `f` を実行する。
#[cfg(target_os = "macos")]
pub fn with_autorelease_pool<T>(f: impl FnOnce() -> T) -> T {
    objc2::rc::autoreleasepool(|_pool| f())
}

/// [`with_autorelease_pool`] の非 macOS スタブ。pool は無いので `f` をそのまま実行。
#[cfg(not(target_os = "macos"))]
pub fn with_autorelease_pool<T>(f: impl FnOnce() -> T) -> T {
    f()
}

// ---------------------------------------------------------------------------
// メインスレッドの NSApplication
// ---------------------------------------------------------------------------

/// メインスレッドに束縛された `NSApplication` ハンドル。
///
/// `Retained<NSApplication>` と `MainThreadMarker` はいずれも `!Send`/`!Sync` なので、
/// このハンドルはメインスレッド外へ移動できない（コンパイル時に強制される）。
#[cfg(target_os = "macos")]
pub struct MainAppHandle {
    app: objc2::rc::Retained<objc2_app_kit::NSApplication>,
    mtm: objc2::MainThreadMarker,
}

#[cfg(target_os = "macos")]
impl std::fmt::Debug for MainAppHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MainAppHandle")
            .field("mtm", &self.mtm)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "macos")]
impl MainAppHandle {
    /// 基盤となる `NSApplication`。delegate 設定や activation policy 変更に使う。
    /// 現状どのサブコマンドも delegate を設定しないため未使用。
    #[allow(dead_code)]
    #[must_use]
    pub fn app(&self) -> &objc2_app_kit::NSApplication {
        &self.app
    }

    /// メインスレッドであることの証明トークン。他の main-thread-only API に渡す。
    /// 現状どのサブコマンドも追加の main-thread-only API を呼ばないため未使用。
    #[allow(dead_code)]
    #[must_use]
    pub const fn main_thread_marker(&self) -> objc2::MainThreadMarker {
        self.mtm
    }

    /// AppKit のメインイベントループに入る。`NSApplication` が terminate されるまで
    /// ブロックする。呼び出し前に delegate や activation policy を設定しておくこと。
    pub fn run(&self) {
        self.app.run();
    }
}

/// メインスレッド上で共有 `NSApplication` を用意し、ハンドルを返す再利用プリミティブ。
///
/// 実際のイベントループ突入（[`MainAppHandle::run`]）は呼び出し側に委ねる。この
/// Probe バイナリの `main()` 配線はこのタスクの範囲外なので、どこから呼ぶかを固定
/// しない。
///
/// # Errors
///
/// メインスレッド以外から呼ばれた場合に `Err` を返す。
#[cfg(target_os = "macos")]
pub fn setup_main_application() -> anyhow::Result<MainAppHandle> {
    let mtm = objc2::MainThreadMarker::new().ok_or_else(|| {
        anyhow::anyhow!("setup_main_application must be called on the main thread")
    })?;
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    Ok(MainAppHandle { app, mtm })
}

/// メインスレッドで `NSApplication` を用意し、そのイベントループに入る簡易プリミティブ。
///
/// [`MainAppHandle::run`] がブロックするため、`NSApplication` が terminate されるまで
/// 戻らない。delegate 等の事前設定が必要なら [`setup_main_application`] を使うこと。
///
/// # Errors
///
/// メインスレッド以外から呼ばれた場合に `Err` を返す。
///
/// 現状どのサブコマンドも setup と run の間に処理を挟む必要があり
/// [`setup_main_application`] を直接使っているため未使用（挟む処理が無い将来の
/// サブコマンド向けの簡易版）。
#[allow(dead_code)]
#[cfg(target_os = "macos")]
pub fn run_on_main_thread() -> anyhow::Result<()> {
    let handle = setup_main_application()?;
    handle.run();
    Ok(())
}

/// [`run_on_main_thread`] の非 macOS スタブ。
///
/// # Errors
///
/// 非 macOS では常に `Err`（unsupported platform）を返す。
#[allow(dead_code)]
#[cfg(not(target_os = "macos"))]
pub fn run_on_main_thread() -> anyhow::Result<()> {
    anyhow::bail!("run_on_main_thread is only supported on macOS")
}

// ---------------------------------------------------------------------------
// 共有 NSWorkspace の notification center
// ---------------------------------------------------------------------------

/// 共有 `NSWorkspace` の notification center を返す。
///
/// アプリ切替（`NSWorkspaceDidActivateApplicationNotification` 等）や wake
/// （`NSWorkspaceDidWakeNotification`）は **この** center にしか流れない。既定の
/// `NSNotificationCenter::defaultCenter` に observer を登録すると通知を取りこぼすため、
/// focus.rs / sleep-wake は必ずこの accessor 経由で登録する。
///
/// 返り値の `Retained<NSNotificationCenter>` は `!Send` なので、observer 登録と通知
/// 処理を行うスレッド（通常はメインスレッド）で取得・使用すること。
#[cfg(target_os = "macos")]
#[must_use]
pub fn workspace_notification_center() -> objc2::rc::Retained<objc2_foundation::NSNotificationCenter>
{
    objc2_app_kit::NSWorkspace::sharedWorkspace().notificationCenter()
}

// ---------------------------------------------------------------------------
// CFRunLoop ワーカスレッド
// ---------------------------------------------------------------------------

/// 専用 `CFRunLoop` を回すワーカスレッドのハンドル。
///
/// tap.rs は [`spawn_run_loop_thread`] でこのスレッドを起こし、`setup` クロージャの中で
/// CGEventTap の run loop source を **そのスレッドの** run loop に載せる（tap source は
/// スレッドローカルな run loop に紐付くため）。停止は [`shutdown`](Self::shutdown) か
/// drop で行う。
///
/// 停止機構: `CFRetained<CFRunLoop>` は `!Send` でスレッドを跨いで `CFRunLoopStop` を
/// 呼べないため、ワーカ自身が停止フラグをスライスごとに poll して自発停止する方式に
/// した。停止レイテンシは [`POLL_SLICE`] で bound される。
#[derive(Debug)]
pub struct RunLoopThreadHandle {
    #[allow(dead_code)] // Debug 表示用に保持。name() accessor 経由の読み出しはまだ無い
    name: String,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl RunLoopThreadHandle {
    /// スレッド名。現状呼び出す診断コードが無い（Debug 表示のみ）。
    #[allow(dead_code)]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// ワーカがまだ動いているか（join 済み・停止完了なら `false`）。
    /// 現状呼び出す診断コードが無い。
    #[allow(dead_code)]
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.join.as_ref().is_some_and(|j| !j.is_finished())
    }

    /// 停止を要求する（join はしない）。次のスライス境界でワーカが抜ける。
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// 停止を要求し、ワーカスレッドの終了まで join する。冪等。
    pub fn shutdown(&mut self) {
        self.stop();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for RunLoopThreadHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 専用 `CFRunLoop` を持つワーカスレッドを起こす。
///
/// `setup` はワーカスレッド上で（autorelease pool の中で）run loop 開始前に一度だけ
/// 実行される。CGEventTap 等の run loop source を **このスレッドの** run loop に載せる
/// のはここ（`CFRunLoop::current()` がこのスレッドの run loop を指す）。source を run
/// loop に登録すると run loop 側が保持するので、`setup` を抜けても生き続ける。
///
/// 非 macOS では実 run loop は無く、停止フラグを poll するだけの無害なスレッドを起こす
/// （Linux 開発機でクレートを回すためのスタブ）。
///
/// # Panics
///
/// OS がワーカスレッドの生成に失敗した場合に panic する（起動時に一度だけ起こす基盤
/// スレッドであり、ここでの失敗は続行不能なため）。
pub fn spawn_run_loop_thread<F>(name: &str, setup: F) -> RunLoopThreadHandle
where
    F: FnOnce() + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let thread_name = name.to_owned();
    let join = std::thread::Builder::new()
        .name(thread_name.clone())
        .spawn(move || run_loop_worker(&worker_stop, setup))
        .expect("failed to spawn run loop thread");
    RunLoopThreadHandle {
        name: thread_name,
        stop,
        join: Some(join),
    }
}

#[cfg(target_os = "macos")]
fn run_loop_worker(stop: &Arc<AtomicBool>, setup: impl FnOnce()) {
    use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopRunResult};

    with_autorelease_pool(setup);

    while !stop.load(Ordering::SeqCst) {
        let result = with_autorelease_pool(|| {
            // SAFETY: kCFRunLoopDefaultMode は CoreFoundation が提供する有効なグローバル
            //         定数。参照のコピー（Option<&'static>）を取り出すだけで解放不要。
            let mode = unsafe { kCFRunLoopDefaultMode };
            CFRunLoop::run_in_mode(mode, POLL_SLICE.as_secs_f64(), false)
        });
        // source/timer が未登録の run loop は即座に Finished を返す。ここで sleep して
        // ビジーループを避ける（実際に tap source が載れば run_in_mode がスライス分
        // ブロックするのでこの分岐は通らない）。
        if result == CFRunLoopRunResult::Finished {
            std::thread::sleep(POLL_SLICE);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn run_loop_worker(stop: &Arc<AtomicBool>, setup: impl FnOnce()) {
    with_autorelease_pool(setup);
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(POLL_SLICE);
    }
}

/// [`ShutdownSignal`] をワーカスレッドの寿命に結び付ける driver ヘルパ。
///
/// 呼び出しスレッドで `signal` が trigger されるまで poll し、trigger されたら
/// `handle` を [`shutdown`](RunLoopThreadHandle::shutdown) してワーカの終了まで join
/// してから戻る。後続タスク（tap.rs 等）はワーカを起こしたあと、driver スレッドで
/// これを呼んで「shutdown が来るまで動かし続け、来たら畳む」を表現する。
///
/// 実際の shutdown トリガ（SIGINT/Ctrl-C→`signal.trigger()`）の配線は後続の統合パスに
/// 委ねる。ここは仕組みだけを提供する。tap.rs/sleep_wake.rs はまだこれを使わず自前の
/// 停止管理で完結しているため、現状呼び出し元が無い。
#[allow(dead_code)]
pub fn run_until_shutdown(signal: &ShutdownSignal, mut handle: RunLoopThreadHandle) {
    while !signal.is_shutdown() {
        std::thread::sleep(POLL_SLICE);
    }
    handle.shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_signal_starts_untriggered() {
        let signal = ShutdownSignal::new();
        assert!(!signal.is_shutdown());
        assert!(!ShutdownSignal::default().is_shutdown());
    }

    #[test]
    fn shutdown_signal_trigger_is_observable() {
        let signal = ShutdownSignal::new();
        signal.trigger();
        assert!(signal.is_shutdown());
    }

    #[test]
    fn shutdown_signal_trigger_is_idempotent() {
        let signal = ShutdownSignal::new();
        signal.trigger();
        signal.trigger();
        assert!(signal.is_shutdown());
    }

    #[test]
    fn shutdown_signal_clone_shares_flag() {
        let signal = ShutdownSignal::new();
        let clone = signal.clone();
        clone.trigger();
        assert!(
            signal.is_shutdown(),
            "trigger on a clone must be visible on the original"
        );
    }

    #[test]
    fn spawn_run_loop_thread_runs_and_shuts_down() {
        let mut handle = spawn_run_loop_thread("test-worker", || {});
        assert_eq!(handle.name(), "test-worker");
        handle.shutdown();
        assert!(!handle.is_running());
        // 二重 shutdown は冪等で panic しない。
        handle.shutdown();
    }

    #[test]
    fn run_until_shutdown_returns_after_trigger() {
        let signal = ShutdownSignal::new();
        let ran = Arc::new(AtomicBool::new(false));
        let ran_worker = Arc::clone(&ran);
        let handle = spawn_run_loop_thread("test-driver", move || {
            ran_worker.store(true, Ordering::SeqCst);
        });
        // 別スレッドから終了を要求。
        let signal_for_trigger = signal.clone();
        std::thread::spawn(move || {
            std::thread::sleep(POLL_SLICE);
            signal_for_trigger.trigger();
        });
        run_until_shutdown(&signal, handle);
        assert!(
            ran.load(Ordering::SeqCst),
            "setup closure must have run on the worker thread"
        );
    }
}
