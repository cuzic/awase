//! X11 XRecord キーボード入力バックエンド
//!
//! X11 環境専用。XRecordCreateContext + XRecordEnableContextAsync でキーイベントを監視。
//! XTest でキー出力。Wayland では動作しない。
//!
//! config.toml で `linux_input_backend = "x11"` を指定すると使用される。

use awase::scanmap::PhysicalPos;
use awase::types::KeyClassification;

/// X11 XRecord ベースのキーボード入力（スタブ実装）
///
/// 将来的に x11rb クレートで XRecord 拡張を使用する。
/// X11 keycode は evdev keycode + 8 のオフセットがある。
#[derive(Debug)]
pub struct X11Input {
    // future: x11rb::protocol::record::Context
}

impl X11Input {
    /// X11 ディスプレイに接続してキーボード監視を開始する
    pub fn new() -> anyhow::Result<Self> {
        anyhow::bail!("X11 backend not yet implemented. Use evdev backend instead (linux_input_backend = \"evdev\")")
    }

    /// X11 keycode を evdev keycode に変換する
    /// X11 keycode = evdev keycode + 8
    pub const fn x11_to_evdev(x11_keycode: u32) -> u32 {
        x11_keycode.saturating_sub(8)
    }

    /// X11 keycode からキー分類と物理位置を生成する
    pub fn classify_key(x11_keycode: u32) -> (KeyClassification, Option<PhysicalPos>) {
        let evdev_code = Self::x11_to_evdev(x11_keycode);
        // Reuse the evdev classification after offset conversion
        crate::hook::classify_key(evdev_code)
    }
}

/// X11 XTest ベースのキー出力（スタブ実装）
///
/// 将来的に XTestFakeKeyEvent でキーイベントを送信する。
#[derive(Debug)]
pub struct X11Output;

impl X11Output {
    pub fn new() -> anyhow::Result<Self> {
        anyhow::bail!("X11 output not yet implemented. Use uinput output instead.")
    }
}
