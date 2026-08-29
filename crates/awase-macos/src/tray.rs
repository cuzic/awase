//! macOS メニューバーアイコン (NSStatusBar)
//!
//! NSStatusItem をメニューバーに常駐させ、エンジン ON/OFF トグルと終了を
//! メニューから操作できるようにする。メニュー action は ObjC ターゲット
//! クラス経由で `event_loop::dispatch_menu_action` に配送される。

#[cfg(target_os = "macos")]
mod imp {
    // AppKit (NSStatusBar/NSMenu) の ObjC メッセージ送信に必要
    #![allow(unsafe_code)]
    // objc 0.2 のマクロが cfg(feature = "cargo-clippy") を展開するための抑制
    #![allow(unexpected_cfgs)]
    // cocoa クレートは全体が deprecated（objc2-app-kit への移行推奨）。
    // upstream 選定の依存のため v1 はこのまま使い、objc2 移行は別途行う。
    #![allow(deprecated)]

    use cocoa::appkit::{
        NSButton, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    };
    use cocoa::base::{id, nil, NO};
    use cocoa::foundation::NSString;
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    use std::sync::Once;

    use crate::event_loop::{dispatch_menu_action, MenuAction};

    /// メニューバーに表示するタイトル（エンジン ON）
    const TITLE_ENABLED: &str = "あ";
    /// メニューバーに表示するタイトル（エンジン OFF）
    const TITLE_DISABLED: &str = "A";

    extern "C" fn on_toggle_engine(_this: &Object, _cmd: Sel, _sender: id) {
        dispatch_menu_action(MenuAction::ToggleEngine);
    }

    /// メニュー action の受け口となる ObjC クラスを一度だけ登録する。
    fn target_class() -> &'static Class {
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            let mut decl = ClassDecl::new("AwaseTrayTarget", class!(NSObject))
                .expect("AwaseTrayTarget class already registered");
            unsafe {
                decl.add_method(
                    sel!(toggleEngine:),
                    on_toggle_engine as extern "C" fn(&Object, Sel, id),
                );
            }
            decl.register();
        });
        Class::get("AwaseTrayTarget").expect("AwaseTrayTarget must be registered")
    }

    /// retain 済みの NSString を作り、クロージャ適用後に release する。
    unsafe fn with_ns_string<R>(s: &str, f: impl FnOnce(id) -> R) -> R {
        let ns: id = NSString::alloc(nil).init_str(s);
        let result = f(ns);
        let _: () = msg_send![ns, release];
        result
    }

    /// macOS メニューバー常駐アイコン。
    ///
    /// メイン thread（CFRunLoop/NSApplication と同じ）でのみ生成・操作すること。
    pub struct SystemTray {
        status_item: id,
        toggle_item: id,
        layout_item: id,
        enabled: bool,
    }

    impl std::fmt::Debug for SystemTray {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SystemTray")
                .field("enabled", &self.enabled)
                .finish_non_exhaustive()
        }
    }

    impl SystemTray {
        #[must_use]
        pub fn new() -> Self {
            unsafe {
                let status_bar = NSStatusBar::systemStatusBar(nil);
                let status_item: id = status_bar.statusItemWithLength_(NSVariableStatusItemLength);
                let _: () = msg_send![status_item, retain];

                let menu: id = NSMenu::new(nil);
                let target: id = msg_send![target_class(), new];

                // エンジン ON/OFF トグル（チェックマークで状態表示）
                let toggle_item: id = with_ns_string("NICOLA 入力", |title| {
                    with_ns_string("", |key| {
                        NSMenuItem::alloc(nil).initWithTitle_action_keyEquivalent_(
                            title,
                            sel!(toggleEngine:),
                            key,
                        )
                    })
                });
                let _: () = msg_send![toggle_item, setTarget: target];
                menu.addItem_(toggle_item);

                // 使用中レイアウト名（表示のみ）
                let layout_item: id = with_ns_string("配列: -", |title| {
                    with_ns_string("", |key| {
                        NSMenuItem::alloc(nil).initWithTitle_action_keyEquivalent_(
                            title,
                            sel!(toggleEngine:),
                            key,
                        )
                    })
                });
                let _: () = msg_send![layout_item, setEnabled: NO];
                menu.addItem_(layout_item);

                menu.addItem_(NSMenuItem::separatorItem(nil));

                // 終了（NSApplication terminate:）
                let quit_item: id = with_ns_string("awase を終了", |title| {
                    with_ns_string("q", |key| {
                        NSMenuItem::alloc(nil).initWithTitle_action_keyEquivalent_(
                            title,
                            sel!(terminate:),
                            key,
                        )
                    })
                });
                let nsapp = cocoa::appkit::NSApp();
                let _: () = msg_send![quit_item, setTarget: nsapp];
                menu.addItem_(quit_item);

                status_item.setMenu_(menu);

                let mut tray = Self {
                    status_item,
                    toggle_item,
                    layout_item,
                    enabled: true,
                };
                tray.sync_ui();
                tray
            }
        }

        /// メニューバーのタイトルとトグルのチェック状態を現在の状態に合わせる。
        fn sync_ui(&mut self) {
            let title = if self.enabled {
                TITLE_ENABLED
            } else {
                TITLE_DISABLED
            };
            unsafe {
                let button: id = self.status_item.button();
                if button != nil {
                    with_ns_string(title, |t| NSButton::setTitle_(button, t));
                }
                // NSControlStateValueOn = 1 / Off = 0
                let _: () = msg_send![self.toggle_item, setState: i64::from(self.enabled)];
            }
        }

        pub fn set_enabled(&mut self, enabled: bool) {
            self.enabled = enabled;
            self.sync_ui();
            log::info!("Tray: engine {}", if enabled { "ON" } else { "OFF" });
        }

        /// 通知表示（未実装: ログのみ。NSUserNotification は deprecated のため
        /// UserNotifications framework 対応まで保留）。
        pub fn show_balloon(&self, title: &str, message: &str) {
            log::info!("Notification: {title}: {message}");
        }

        pub fn set_layout_name(&self, name: &str) {
            unsafe {
                with_ns_string(&format!("配列: {name}"), |t| {
                    let _: () = msg_send![self.layout_item, setTitle: t];
                });
            }
        }
    }

    impl Default for SystemTray {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::SystemTray;

/// 非 macOS ビルド用スタブ（ワークスペース全体のクロスチェック用）。
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub struct SystemTray {
    enabled: bool,
}

#[cfg(not(target_os = "macos"))]
impl SystemTray {
    #[must_use]
    pub fn new() -> Self {
        log::info!("Menu bar icon is only available on macOS");
        Self { enabled: true }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        log::info!("Tray: engine {}", if enabled { "ON" } else { "OFF" });
    }

    pub fn show_balloon(&self, title: &str, message: &str) {
        log::info!("Notification: {title}: {message}");
    }

    pub fn set_layout_name(&self, name: &str) {
        log::info!("Tray: layout = {name}");
    }
}

#[cfg(not(target_os = "macos"))]
impl Default for SystemTray {
    fn default() -> Self {
        Self::new()
    }
}
