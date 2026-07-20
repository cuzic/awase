# awase — Thumb-Shift (NICOLA) Keyboard Remapper

*[日本語](README.md)*

**awase** (合わせ) is a keyboard remapper that brings thumb-shift input to Windows.

---

## What is Thumb Shift?

Thumb shift (the NICOLA layout) is an input method that uses the 変換 (Henkan) and 無変換 (Muhenkan) keys on either side of the space bar as thumb-shift keys: pressing one of them simultaneously with a character key inputs a kana character directly. It lets you type Japanese with fewer keystrokes than romaji input, and once mastered it enables fast, highly efficient typing.

awase intercepts physical key input with a low-level keyboard hook, detects simultaneous keystrokes (chords), and sends them to the IME as romaji. The IME then handles kanji conversion as usual.

---

## Features

- **NICOLA-compliant chord detection** — 3-key arbitration based on d1/d2 comparison
- **Five confirm modes** — wait / speculative / two\_phase / adaptive\_timing / ngram\_predictive
- **n-gram adaptive thresholds** — dynamically tunes the detection window using 2/3-grams derived from a Wikipedia corpus, improving accuracy
- **Yamabuki-compatible `.yab` layout files** — use your existing layout data as-is
- **Broad application support** — automatically identifies Win32 / UWP / TSF-native apps (Chrome, VS Code, WezTerm, etc.)
- **Multi-layered fault tolerance** — hook liveness monitoring, sleep/resume recovery, IME-detection-failure fallback, and automatic TSF cold-start recovery, layered in stages
- **Asynchronous architecture** — an async executor built on the Windows message loop; blocking APIs are isolated on separate threads with timeout protection
- **Automatic focus detection** — automatically stops conversion when focus is not on a text input field
- **System tray resident** — switch layouts, open the settings screen, and toggle enable/disable
- **US layout support** — switch to a US physical layout with `keyboard_model = "us"`. Because a US keyboard has no Muhenkan/Henkan keys, awase can also impersonate thumb keys onto the left/right Alt keys, or turn the Space key into a thumb key

For details on the technical design, see [ARCHITECTURE.md](ARCHITECTURE.md).

---

## Requirements

- Windows 10 / 11 (64-bit)
- Google Japanese Input (recommended) or MS-IME
- Rust 1.85 or later (build time only)

---

## Quick Start

### 1. Build

```sh
cargo build --release --target x86_64-pc-windows-msvc
```

Output: `target/x86_64-pc-windows-msvc/release/awase.exe`

### 2. File Layout

Arrange the files as follows.

```
awase.exe
config.toml          ← configuration file
layout/
  nicola.yab         ← NICOLA layout (bundled)
data/
  ngram_hiragana.csv.gz  ← n-gram corpus (optional)
```

### 3. Launch

Double-click `awase.exe` and it becomes resident in the system tray.

### 4. Turn the Engine ON

Default key bindings:

| Action | Key |
|------|------|
| Engine ON | **Ctrl+Shift+変換** |
| Engine OFF | **Ctrl+Shift+無変換** |
| IME ON | **Ctrl+変換** (if the IME is already ON, resets it to hiragana / romaji / CapsLock OFF) |
| IME OFF | **Ctrl+無変換** |
| Toggle IME-ON halfwidth alphanumeric (MS-IME only) | **Left Shift single tap** (press and release without any other key; tap again to cancel) |
| Manually switch per-app behavior | **Ctrl+Shift+F11** |

> You can change these through the GUI by right-clicking the tray icon → "Settings".

### 5. Check the Thumb Keys

By default, 無変換 (Muhenkan) is the left thumb key and 変換 (Henkan) is the right thumb key.  
You can change this with `left_thumb_key` / `right_thumb_key` in `config.toml`.

---

## Configuration File (config.toml)

Minimal setup:

```toml
[general]
simultaneous_threshold_ms = 100   # 同時打鍵判定の閾値（ms）。NICOLA 規格は 100ms
left_thumb_key  = "無変換"
right_thumb_key = "変換"
layouts_dir     = "layout"
default_layout  = "nicola.yab"
```

For a full sample, see the bundled `config.toml`.

Note: `left_thumb_key` / `right_thumb_key` must be set to the literal Japanese key names (`"無変換"` / `"変換"`) — these are the exact values awase's config parser expects.

### Main Options

| Key | Default | Description |
|------|-----------|------|
| `simultaneous_threshold_ms` | 100 | Time window (ms) within which two presses count as a simultaneous keystroke |
| `left_thumb_key` | `無変換` | Left thumb-shift key |
| `right_thumb_key` | `変換` | Right thumb-shift key |
| `confirm_mode` | `wait` | Confirm mode (see below) |
| `output_mode` | `unicode` | Output method (normally no need to change) |
| `engine_toggle_hotkey` | none | Hotkey to toggle the engine ON/OFF |
| `keyboard_model` | `jis` | Physical keyboard layout. For a US layout use `"us"` (also change `default_layout` to `nicola_us.yab`) |

### Confirm Modes

| Mode | Characteristics |
|--------|------|
| `wait` | Waits until the timeout. Most accurate, with slight latency |
| `speculative` | Outputs immediately and cancels/resends if wrong. Fast, but with flicker |
| `two_phase` | Speculative output after a brief wait. A middle ground between wait and speculative |
| `adaptive_timing` | Auto-adjusts based on typing speed |
| `ngram_predictive` | Dynamically tunes the threshold using Wikipedia-derived n-gram statistics (n-gram file recommended) |

If unsure, start with `wait`, and if latency bothers you, try `adaptive_timing`.

For details on how the n-gram mechanism works, see [ARCHITECTURE.md](ARCHITECTURE.md#n-gram-による同時打鍵判定の精度向上).

### Per-App Settings ([app_overrides])

Force specific behavior when an app doesn't work correctly.

```toml
[app_overrides]
# 常にテキスト入力として扱う
force_text = [
    { process = "myapp.exe", class = "Edit" },
]
# エンジンを常に無効にする
force_bypass = [
    { process = "launcher.exe", class = "LauncherClass" },
]
# TSF ネイティブモード（WezTerm 等）
force_tsf = [
    { process = "wezterm-gui.exe", class = "org.wezfurlong.wezterm" },
]
```

You can find the process name and class name in the log output of `RUST_LOG=debug awase.exe`.

---

## Layout Files (.yab)

Layouts are defined in the Yamabuki-compatible CSV format. Place `.yab` files in `layout/` to switch between them from the tray menu.

From the "Layout Editor" tab of the settings screen (`awase-settings.exe`), instead of editing the CSV directly in a text editor, you can also edit and save visually by clicking on a keyboard-style grid.

```
; コメント行はセミコロンで始める
[ローマ字シフト無し]
'。',ka,ta,ko,sa, ra,ti,ku,tu,'，','、',無
u, si,te,ke,se, ha,to,ki, i, nn, 後, 逃
...

[ローマ字左親指シフト]
...

[ローマ字右親指シフト]
...
```

The standard NICOLA layouts are bundled as `layout/nicola.yab` (JIS layout) and `layout/nicola_us.yab` (US layout). Because a US keyboard physically lacks the Muhenkan/Henkan keys, the settings screen lets you impersonate thumb keys onto the left/right Alt keys, or assign the Space key as a thumb key.

---

## Application Support

awase automatically identifies the focused application and switches its output method. No manual configuration is required.

| App type | Examples | Output method |
|-----------|-----|---------|
| Win32 / WinForms | Notepad, Word, Excel | Direct Unicode injection |
| TSF native | Chrome, Edge, VS Code, WezTerm, Electron-based apps | VK keystrokes |
| UWP / XAML | Windows Store apps | Direct Unicode injection |

Identification results are learned and cached per app class name (`cache.toml`) and persist across restarts. If automatic identification is wrong, you can specify it manually with `[app_overrides]`.

---

## Troubleshooting

**No characters are typed / strange characters appear**  
→ The engine may be OFF. Turn it ON with Ctrl+Shift+変換.

**Doesn't work in a specific app**  
→ Launch with `RUST_LOG=debug awase.exe`, check the log, and add the app to `[app_overrides]`.

**The IME turns ON/OFF on its own**  
→ Check the shadow-tracking keys under `[keys.ime_detect]` in `config.toml`.

**Too many misfired chord detections**  
→ Adjust `simultaneous_threshold_ms` within the 80–120ms range.

**The IME or FSM got into a broken state**  
→ Right-click the tray icon → "Reset internal state" to reinitialize all internal state.

---

## License

You may choose either the [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT License](LICENSE-MIT).
