//! macOS メニューバーアイコン (NSStatusBar)
//!
//! NSStatusItem をメニューバーに常駐させ、エンジン ON/OFF トグルと終了を
//! メニューから操作できるようにする。メニュー action は ObjC ターゲット
//! クラス経由で `event_loop::dispatch_menu_action` に配送される。

#[cfg(target_os = "macos")]
mod imp {
    // define_class! が生成する ObjC メソッドと sel! に必要
    #![allow(unsafe_code)]

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{define_class, sel, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSMenu, NSMenuItem,
        NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    };
    use objc2_foundation::{ns_string, NSObject, NSString};

    use crate::event_loop::{dispatch_menu_action, MenuAction};

    /// メニューバーに表示するタイトル（エンジン ON）
    const TITLE_ENABLED: &str = "あ";
    /// メニューバーに表示するタイトル（エンジン OFF）
    const TITLE_DISABLED: &str = "A";

    define_class!(
        /// メニュー action の受け口。
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "AwaseTrayTarget"]
        struct TrayTarget;

        impl TrayTarget {
            #[unsafe(method(toggleEngine:))]
            fn toggle_engine(&self, _sender: Option<&AnyObject>) {
                dispatch_menu_action(MenuAction::ToggleEngine);
            }
        }
    );

    impl TrayTarget {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            // ivar なしの NSObject サブクラスなので既定の init で十分
            unsafe { objc2::msg_send![Self::alloc(mtm), init] }
        }
    }

    /// macOS メニューバー常駐アイコン。
    ///
    /// メイン thread（CFRunLoop/NSApplication と同じ）でのみ生成・操作すること。
    pub struct SystemTray {
        status_item: Retained<NSStatusItem>,
        toggle_item: Retained<NSMenuItem>,
        layout_item: Retained<NSMenuItem>,
        /// NSMenuItem.target は弱参照のため、所有権を保持して生存させる
        _target: Retained<TrayTarget>,
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
        /// メニューバーアイコンを作成する。
        ///
        /// # Panics
        ///
        /// メイン thread 以外から呼ばれた場合。
        #[must_use]
        pub fn new() -> Self {
            let mtm = MainThreadMarker::new()
                .expect("SystemTray must be created on the main thread");

            let status_bar = NSStatusBar::systemStatusBar();
            let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);

            let menu = NSMenu::new(mtm);
            let target = TrayTarget::new(mtm);

            // エンジン ON/OFF トグル（チェックマークで状態表示）
            let toggle_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    ns_string!("NICOLA 入力"),
                    Some(sel!(toggleEngine:)),
                    ns_string!(""),
                )
            };
            unsafe { toggle_item.setTarget(Some(&target)) };
            menu.addItem(&toggle_item);

            // 使用中レイアウト名（表示のみ）
            let layout_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    ns_string!("配列: -"),
                    None,
                    ns_string!(""),
                )
            };
            layout_item.setEnabled(false);
            menu.addItem(&layout_item);

            menu.addItem(&NSMenuItem::separatorItem(mtm));

            // 終了（NSApplication terminate:）
            let quit_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    ns_string!("awase を終了"),
                    Some(sel!(terminate:)),
                    ns_string!("q"),
                )
            };
            let app = NSApplication::sharedApplication(mtm);
            unsafe { quit_item.setTarget(Some(&app)) };
            menu.addItem(&quit_item);

            status_item.setMenu(Some(&menu));

            let tray = Self {
                status_item,
                toggle_item,
                layout_item,
                _target: target,
                enabled: true,
            };
            tray.sync_ui();
            tray
        }

        /// メニューバーのタイトルとトグルのチェック状態を現在の状態に合わせる。
        fn sync_ui(&self) {
            let title = if self.enabled {
                TITLE_ENABLED
            } else {
                TITLE_DISABLED
            };
            // 生成時に MainThreadOnly を確認済み（new() 参照）
            let mtm = MainThreadMarker::new()
                .expect("SystemTray must be used on the main thread");
            if let Some(button) = self.status_item.button(mtm) {
                button.setTitle(&NSString::from_str(title));
            }
            self.toggle_item.setState(if self.enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
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
            self.layout_item
                .setTitle(&NSString::from_str(&format!("配列: {name}")));
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
