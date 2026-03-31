//! Linux 固有のプラットフォーム実装クレート。
//!
//! evdev キーボード入力、uinput 出力、IBus/Fcitx D-Bus IME 制御など
//! すべての Linux 固有コードを集約する。

pub mod hook; // evdev/libinput input
pub mod ime; // IBus/Fcitx D-Bus
pub mod libinput; // libinput input backend
pub mod scanmap;
pub mod vk;
pub mod x11; // X11 XRecord input
             // Future modules:
pub mod output; // uinput output
pub mod tray; // StatusNotifierItem
              // pub mod event_loop; // epoll
