//! libinput キーボード入力バックエンド
//!
//! libinput は evdev の上位レイヤーで、デバイスの自動検出とホットプラグをサポートする。
//! Wayland 環境でのキーボード入力に適している。
//!
//! config.toml で `linux_input_backend = "libinput"` を指定すると使用される。
//! input グループへの所属が必要。

use awase::scanmap::PhysicalPos;
use awase::types::KeyClassification;

/// libinput ベースのキーボード入力（スタブ実装）
///
/// 将来的に `input` クレート (libinput bindings) を使用する。
/// libinput は evdev keycode を使うため、キーマッピングは evdev バックエンドと共通。
#[derive(Debug)]
pub struct LibinputInput {
    // future: input::Libinput
}

impl LibinputInput {
    /// libinput コンテキストを作成し、seat0 に接続する
    pub fn new() -> anyhow::Result<Self> {
        anyhow::bail!(
            "libinput backend not yet implemented. Use evdev backend instead \
             (linux_input_backend = \"evdev\")"
        )
    }

    /// libinput は evdev keycode を使うため、evdev の分類関数をそのまま使用
    pub fn classify_key(evdev_keycode: u32) -> (KeyClassification, Option<PhysicalPos>) {
        crate::hook::classify_key(evdev_keycode)
    }
}

// NOTE: libinput + uinput ベースのキー出力
// libinput 自体は出力機能を持たないため、uinput を使用する。
// `crate::output::UinputOutput` をそのまま使用すればよい。
