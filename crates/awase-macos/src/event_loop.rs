//! macOS イベントループ (CGEventTap + CFRunLoop)
//!
//! CGEventTap でキーイベントを捕捉し、CFRunLoopTimer で Engine の
//! `TimerEffect`（Wait/TwoPhase 確定モードのタイムアウト等）を駆動する。
//! すべて単一スレッド（main thread の CFRunLoop）で動くため、ハンドラは
//! `Rc<RefCell<_>>` で共有する。

#[cfg(target_os = "macos")]
mod imp {
    // CoreFoundation/CoreGraphics の C API 呼び出しに必要
    #![allow(unsafe_code)]

    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::rc::Rc;
    use std::time::Duration;

    use anyhow::{anyhow, Result};
    use core_foundation::date::CFDate;
    use core_foundation::mach_port::CFMachPortRef;
    use core_foundation::runloop::{
        kCFRunLoopCommonModes, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext,
        CFRunLoopTimerRef,
    };
    use core_graphics::event::{
        CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventType,
    };

    /// tap コールバックの結果: イベントを通すか握りつぶすか。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TapAction {
        /// イベントをそのまま下流（アプリケーション）へ通す
        Pass,
        /// イベントを握りつぶす（下流へ届けない）
        Consume,
    }

    /// メニューバーから届く操作。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MenuAction {
        /// NICOLA エンジンの有効/無効を切り替える
        ToggleEngine,
    }

    /// イベントループから駆動されるハンドラ。
    ///
    /// tap コールバックとタイマーコールバックの両方から `RefCell` 経由で
    /// 呼ばれる。CFRunLoop は単一スレッドでイベントを直列に配送するため
    /// 再入は起きない。
    pub trait LoopHandler {
        /// KeyDown / KeyUp / FlagsChanged イベント。
        fn on_cg_event(&mut self, etype: CGEventType, event: &CGEvent) -> TapAction;
        /// `Timers::set` で設定したワンショットタイマーの発火。
        fn on_timer_fired(&mut self, id: usize);
        /// メニューバー操作（`dispatch_menu_action` 経由）。
        fn on_menu_action(&mut self, action: MenuAction);
        /// 定期ポーリング（`EventLoop::run` の `poll_interval`）。
        ///
        /// activation 遷移はキーイベント処理の中でしか検知されないため、
        /// IME 切替後に打鍵が無いとトレイ表示が古いまま残る。イベント駆動を
        /// 補完する安全ネット（Windows 版の ime_poll_interval_ms と同じ役割）。
        fn on_poll(&mut self);
    }

    /// メニュー action（ObjC コールバック）を現在のハンドラへ配送する。
    ///
    /// tray のターゲットクラスから呼ばれる。ハンドラ未登録なら無視する。
    pub fn dispatch_menu_action(action: MenuAction) {
        let handler = HANDLER.with(|h| h.borrow().clone());
        if let Some(handler) = handler {
            handler.borrow_mut().on_menu_action(action);
        }
    }

    /// NSApplication を Accessory（Dock 非表示・メニューバー操作可）で初期化する。
    ///
    /// メニューバーアイコン（`SystemTray`）を作る前に、メイン thread で呼ぶこと。
    ///
    /// # Panics
    ///
    /// メイン thread 以外から呼ばれた場合。
    pub fn init_nsapp() {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

        let mtm = MainThreadMarker::new().expect("init_nsapp must run on the main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }

    thread_local! {
        /// タイマー・tap コールバックからハンドラへ届けるための共有参照。
        /// C コールバックには生ポインタしか渡せないため thread_local で持つ。
        static HANDLER: RefCell<Option<Rc<RefCell<dyn LoopHandler>>>> =
            const { RefCell::new(None) };
        /// TapDisabledByTimeout からの自動復帰用に tap の mach port を保持する。
        static TAP_PORT: RefCell<Option<CFMachPortRef>> = const { RefCell::new(None) };
    }

    // core-graphics クレートは CGEventTapEnable を re-export していないため
    // 自前で宣言する（CoreGraphics framework は依存クレート経由でリンク済み）。
    extern "C" {
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    }

    /// ワンショットタイマーの集合。Engine の `TimerEffect::Set`/`Kill` に対応する。
    ///
    /// タイマー ID はポインタ幅に収まるため、ヒープ確保を避けて
    /// `info` ポインタへ直接エンコードする。
    #[derive(Default)]
    pub struct Timers {
        active: HashMap<usize, CFRunLoopTimer>,
    }

    impl std::fmt::Debug for Timers {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Timers")
                .field("active_ids", &self.active.keys().collect::<Vec<_>>())
                .finish()
        }
    }

    extern "C" fn timer_fired_trampoline(_timer: CFRunLoopTimerRef, info: *mut c_void) {
        let id = info as usize;
        let handler = HANDLER.with(|h| h.borrow().clone());
        if let Some(handler) = handler {
            handler.borrow_mut().on_timer_fired(id);
        }
    }

    extern "C" fn poll_timer_trampoline(_timer: CFRunLoopTimerRef, _info: *mut c_void) {
        let handler = HANDLER.with(|h| h.borrow().clone());
        if let Some(handler) = handler {
            handler.borrow_mut().on_poll();
        }
    }

    impl Timers {
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }

        /// ワンショットタイマーを設定する。同じ ID があれば置き換える。
        pub fn set(&mut self, id: usize, duration: Duration) {
            self.kill(id);
            let fire_date = CFDate::now().abs_time() + duration.as_secs_f64();
            let mut context = CFRunLoopTimerContext {
                version: 0,
                info: id as *mut c_void,
                retain: None,
                release: None,
                copyDescription: None,
            };
            let timer = CFRunLoopTimer::new(
                fire_date,
                0.0,
                0,
                0,
                timer_fired_trampoline,
                &raw mut context,
            );
            unsafe {
                CFRunLoop::get_current().add_timer(&timer, kCFRunLoopCommonModes);
            }
            self.active.insert(id, timer);
        }

        /// タイマーをキャンセルする。存在しない ID は無視する。
        pub fn kill(&mut self, id: usize) {
            if let Some(timer) = self.active.remove(&id) {
                unsafe {
                    CFRunLoop::get_current().remove_timer(&timer, kCFRunLoopCommonModes);
                }
            }
        }

        /// 発火済みタイマーの後始末（非リピートタイマーは自動失効するため
        /// マップから外すだけでよい）。
        pub fn fired(&mut self, id: usize) {
            self.active.remove(&id);
        }
    }

    /// macOS イベントループ。
    #[derive(Debug, Default)]
    pub struct EventLoop;

    impl EventLoop {
        #[must_use]
        pub const fn new() -> Self {
            Self
        }

        /// CGEventTap を作成して CFRunLoop を開始する。
        ///
        /// この呼び出しはブロックする。tap の作成にはアクセシビリティ
        /// （入力監視）権限が必要で、権限がない場合はエラーを返す。
        /// `poll_interval` ごとに `LoopHandler::on_poll` を呼ぶ。
        ///
        /// # Errors
        ///
        /// CGEventTap の作成または run loop source の作成に失敗した場合。
        pub fn run(
            &mut self,
            handler: Rc<RefCell<dyn LoopHandler>>,
            poll_interval: Duration,
        ) -> Result<()> {
            HANDLER.with(|h| *h.borrow_mut() = Some(handler));

            let tap = CGEventTap::new(
                CGEventTapLocation::HID,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                vec![
                    CGEventType::KeyDown,
                    CGEventType::KeyUp,
                    CGEventType::FlagsChanged,
                    // クリックは IME の未確定文字列を確定させるため、
                    // composing ヒントのクリア判定に使う（ダウンのみ・素通し）
                    CGEventType::LeftMouseDown,
                    CGEventType::RightMouseDown,
                    CGEventType::OtherMouseDown,
                ],
                |_proxy, etype, event| {
                    Self::tap_callback(etype, event);
                    // None = （必要なら Null 化済みの）元イベントをそのまま返す
                    None
                },
            )
            .map_err(|()| {
                anyhow!(
                    "Failed to create CGEventTap. \
                     Grant Input Monitoring / Accessibility permission in \
                     System Settings > Privacy & Security."
                )
            })?;

            TAP_PORT.with(|p| {
                use core_foundation::base::TCFType;
                *p.borrow_mut() = Some(tap.mach_port.as_concrete_TypeRef());
            });

            let source = tap
                .mach_port
                .create_runloop_source(0)
                .map_err(|()| anyhow!("Failed to create run loop source for event tap"))?;
            let run_loop = CFRunLoop::get_current();
            unsafe {
                run_loop.add_source(&source, kCFRunLoopCommonModes);
            }
            tap.enable();

            // 定期ポーリングタイマー（リピート）。runloop が retain するが、
            // 生存を明示するためローカルにも保持する。
            let interval = poll_interval.as_secs_f64();
            let poll_timer = CFRunLoopTimer::new(
                CFDate::now().abs_time() + interval,
                interval,
                0,
                0,
                poll_timer_trampoline,
                std::ptr::null_mut(),
            );
            unsafe {
                run_loop.add_timer(&poll_timer, kCFRunLoopCommonModes);
            }

            // [NSApp run] はメイン CFRunLoop を回すため、上で登録した tap source と
            // CFRunLoopTimer はそのまま発火する。メニューバーのメニュー追跡には
            // NSApplication のイベントループが必要なので CFRunLoop::run_current では
            // なくこちらを使う。
            log::info!("Event tap installed, entering NSApplication run loop");
            {
                use objc2::MainThreadMarker;
                use objc2_app_kit::NSApplication;
                let mtm = MainThreadMarker::new()
                    .ok_or_else(|| anyhow!("EventLoop::run must run on the main thread"))?;
                NSApplication::sharedApplication(mtm).run();
            }
            Ok(())
        }

        fn tap_callback(etype: CGEventType, event: &CGEvent) {
            // OS はタイムアウトや大量入力で tap を自動無効化することがある。
            // 検知したら再有効化する（イベント自体は触らない）。
            if matches!(
                etype,
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
            ) {
                log::warn!("Event tap disabled by OS ({etype:?}), re-enabling");
                TAP_PORT.with(|p| {
                    if let Some(port) = *p.borrow() {
                        unsafe { CGEventTapEnable(port, true) };
                    }
                });
                return;
            }

            let handler = HANDLER.with(|h| h.borrow().clone());
            let Some(handler) = handler else { return };
            let action = handler.borrow_mut().on_cg_event(etype, event);
            if action == TapAction::Consume {
                // core-graphics 0.24 の tap ラッパーは NULL 返却（=イベント破棄）を
                // 表現できないため、型を Null に書き換えて無効化する。
                event.set_type(CGEventType::Null);
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{
    dispatch_menu_action, init_nsapp, EventLoop, LoopHandler, MenuAction, TapAction, Timers,
};

/// 非 macOS ビルド用スタブ（ワークスペース全体のクロスチェック用）。
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default)]
pub struct EventLoop;

#[cfg(not(target_os = "macos"))]
impl EventLoop {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// イベントループを開始する（スタブ: 即座にリターン）
    ///
    /// # Errors
    ///
    /// スタブのため常に成功する。
    pub fn run(&mut self) -> anyhow::Result<()> {
        log::warn!("macOS event loop is only available on macOS");
        Ok(())
    }
}
