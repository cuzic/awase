//! evdev キーボード入力バックエンド
//!
//! `/dev/input/event*` からキーボードデバイスを自動検出し、
//! `EV_KEY` イベントを読み取って `RawKeyEvent` に変換する。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Instant;

use anyhow::{bail, Context as _};
use evdev::{Device, InputEventKind, Key};

use awase::scanmap::PhysicalPos;
use awase::types::{
    ImeRelevance, KeyClassification, KeyEventType, ModifierKey, RawKeyEvent, ScanCode, Timestamp,
    VkCode,
};

use crate::scanmap::evdev_to_pos;

// ── 親指キー設定 ──

/// 左親指キーの evdev キーコード（デフォルト: KEY_MUHENKAN = 94）
static LEFT_THUMB_KEYCODE: AtomicU16 = AtomicU16::new(94);

/// 右親指キーの evdev キーコード（デフォルト: KEY_HENKAN = 92）
static RIGHT_THUMB_KEYCODE: AtomicU16 = AtomicU16::new(92);

/// 親指キーの evdev キーコードを設定する（config 読み込み後に呼ぶ）
pub fn set_thumb_keycodes(left: VkCode, right: VkCode) {
    LEFT_THUMB_KEYCODE.store(left.0, Ordering::Relaxed);
    RIGHT_THUMB_KEYCODE.store(right.0, Ordering::Relaxed);
}

// ── キー分類 ──

/// evdev キーコードからキー分類と物理位置を生成する
pub(crate) fn classify_key(keycode: u32) -> (KeyClassification, Option<PhysicalPos>) {
    let left_thumb = u32::from(LEFT_THUMB_KEYCODE.load(Ordering::Relaxed));
    let right_thumb = u32::from(RIGHT_THUMB_KEYCODE.load(Ordering::Relaxed));

    if keycode == left_thumb {
        (KeyClassification::LeftThumb, None)
    } else if keycode == right_thumb {
        (KeyClassification::RightThumb, None)
    } else if is_passthrough(keycode) {
        (KeyClassification::Passthrough, None)
    } else if let Some(pos) = evdev_to_pos(keycode) {
        (KeyClassification::Char, Some(pos))
    } else {
        (KeyClassification::Passthrough, None)
    }
}

/// パススルー対象のキーか判定する（修飾キー、Fキー、ナビゲーション等）
const fn is_passthrough(keycode: u32) -> bool {
    matches!(
        keycode,
        // 修飾キー
        42 | 54    // KEY_LEFTSHIFT, KEY_RIGHTSHIFT
        | 29 | 97  // KEY_LEFTCTRL, KEY_RIGHTCTRL
        | 56 | 100 // KEY_LEFTALT, KEY_RIGHTALT
        | 125 | 126 // KEY_LEFTMETA, KEY_RIGHTMETA
        // ファンクションキー
        | 59
            ..=68  // KEY_F1..KEY_F10
        | 87 | 88  // KEY_F11, KEY_F12
        // ナビゲーション
        | 1        // KEY_ESC
        | 14       // KEY_BACKSPACE
        | 15       // KEY_TAB
        | 28       // KEY_ENTER
        | 57       // KEY_SPACE
        | 111      // KEY_DELETE
        | 102      // KEY_HOME
        | 107      // KEY_END
        | 104      // KEY_PAGEUP
        | 109      // KEY_PAGEDOWN
        | 103      // KEY_UP
        | 108      // KEY_DOWN
        | 105      // KEY_LEFT
        | 106      // KEY_RIGHT
        | 110      // KEY_INSERT
        // Print Screen, Scroll Lock, Pause
        | 99 | 70 | 119
        // Caps Lock, Num Lock
        | 58 | 69
    )
}

/// evdev キーコードから修飾キー分類を生成する
const fn classify_modifier(keycode: u32) -> Option<ModifierKey> {
    match keycode {
        42 | 54 => Some(ModifierKey::Shift), // KEY_LEFTSHIFT, KEY_RIGHTSHIFT
        29 | 97 => Some(ModifierKey::Ctrl),  // KEY_LEFTCTRL, KEY_RIGHTCTRL
        56 | 100 => Some(ModifierKey::Alt),  // KEY_LEFTALT, KEY_RIGHTALT
        125 | 126 => Some(ModifierKey::Meta), // KEY_LEFTMETA, KEY_RIGHTMETA
        _ => None,
    }
}

/// IME 関連の事前分類情報を生成する
///
/// Linux での IME 検出は D-Bus 経由で別タスクとして実装するため、
/// 現時点ではデフォルト値を返す。
fn classify_ime_relevance(_keycode: u32) -> ImeRelevance {
    ImeRelevance::default()
}

// ── タイムスタンプ ──

/// 起動時点からの経過マイクロ秒を返す
fn now_timestamp() -> Timestamp {
    use std::sync::OnceLock;
    static BASELINE: OnceLock<Instant> = OnceLock::new();
    let baseline = BASELINE.get_or_init(Instant::now);
    baseline.elapsed().as_micros() as u64
}

// ── デバイス自動検出 ──

/// `/dev/input/event*` をスキャンしてキーボードデバイスを自動検出する
///
/// `EV_KEY` 機能を持ち、実際の文字キー（`KEY_A` 等）をサポートするデバイスを
/// 最初に見つけた時点でそのパスを返す。
pub fn find_keyboard_device() -> anyhow::Result<PathBuf> {
    let input_dir = Path::new("/dev/input");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(input_dir)
        .context("Failed to read /dev/input")?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"))
        })
        .collect();

    // event0, event1, ... の順にソート
    entries.sort();

    for path in entries {
        match Device::open(&path) {
            Ok(device) => {
                if is_suitable_keyboard(&device) {
                    log::info!(
                        "Found keyboard device: {} ({})",
                        path.display(),
                        device.name().unwrap_or("unknown")
                    );
                    return Ok(path);
                }
            }
            Err(e) => {
                log::debug!("Cannot open {}: {}", path.display(), e);
            }
        }
    }

    bail!(
        "No suitable keyboard device found in /dev/input/. \
           Ensure the user has read access (input group) or run as root."
    )
}

/// デバイスが NICOLA 入力に適したキーボードかを判定する
///
/// EV_KEY 機能を持ち、実際の文字キー（KEY_A 等）をサポートしていることを確認する。
/// メディアボタンのみのデバイスは除外する。
fn is_suitable_keyboard(device: &Device) -> bool {
    let Some(keys) = device.supported_keys() else {
        return false;
    };

    // 文字キーが存在するか確認（KEY_A=30, KEY_Z=44 等）
    let has_letter_keys =
        keys.contains(Key::KEY_A) && keys.contains(Key::KEY_Z) && keys.contains(Key::KEY_SPACE);

    has_letter_keys
}

// ── EvdevInput ──

/// evdev デバイスからキーイベントを読み取る入力バックエンド
pub struct EvdevInput {
    device: Device,
}

impl std::fmt::Debug for EvdevInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvdevInput")
            .field("device_name", &self.device.name())
            .finish()
    }
}

impl EvdevInput {
    /// 指定パスのデバイスを開く
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let device =
            Device::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
        log::info!(
            "Opened evdev device: {} ({})",
            path.display(),
            device.name().unwrap_or("unknown")
        );
        Ok(Self { device })
    }

    /// 自動検出でキーボードデバイスを開く
    pub fn open_auto() -> anyhow::Result<Self> {
        let path = find_keyboard_device()?;
        Self::open(&path)
    }

    /// デバイスを排他的に取得する（EVIOCGRAB）
    ///
    /// 排他取得中は他のプロセス（X11/Wayland 等）がこのデバイスの
    /// イベントを受け取れなくなる。uinput で再注入する場合に使う。
    pub fn grab(&mut self) -> anyhow::Result<()> {
        self.device.grab().context("EVIOCGRAB failed")?;
        log::info!("Exclusive grab acquired on device");
        Ok(())
    }

    /// 排他取得を解除する
    pub fn ungrab(&mut self) -> anyhow::Result<()> {
        self.device.ungrab().context("EVIOCUNGRAB failed")?;
        log::info!("Exclusive grab released");
        Ok(())
    }

    /// ブロッキングループでキーイベントを読み取り、コールバックに渡す
    ///
    /// コールバックが `false` を返すとループを終了する。
    pub fn run_blocking<F>(&mut self, mut callback: F) -> anyhow::Result<()>
    where
        F: FnMut(RawKeyEvent) -> bool,
    {
        loop {
            let events = self
                .device
                .fetch_events()
                .context("Failed to fetch evdev events")?;

            for ev in events {
                // EV_KEY イベントのみ処理
                if let InputEventKind::Key(_) = ev.kind() {
                    let keycode = u32::from(ev.code());
                    let event_type = match ev.value() {
                        0 => KeyEventType::KeyUp,
                        1 | 2 => KeyEventType::KeyDown, // 2 = repeat → KeyDown として扱う
                        _ => continue,
                    };

                    let vk = VkCode(ev.code());
                    let scan = ScanCode(keycode);
                    let (key_classification, physical_pos) = classify_key(keycode);

                    let raw_event = RawKeyEvent {
                        vk_code: vk,
                        scan_code: scan,
                        event_type,
                        extra_info: 0,
                        timestamp: now_timestamp(),
                        key_classification,
                        physical_pos,
                        ime_relevance: classify_ime_relevance(keycode),
                        modifier_key: classify_modifier(keycode),
                    };

                    log::trace!(
                        "evdev: code={} type={:?} classification={:?}",
                        keycode,
                        event_type,
                        key_classification
                    );

                    if !callback(raw_event) {
                        return Ok(());
                    }
                }
            }
        }
    }
}
