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
- **Two confirm modes** — wait / ngram\_predictive
- **n-gram adaptive thresholds** — dynamically tunes the detection window using 2/3-grams derived from a Wikipedia corpus, improving accuracy
- **Yamabuki-compatible `.yab` layout files** — use your existing layout data as-is
- **Broad application support** — automatically identifies Win32 / UWP / TSF-native apps (Chrome, VS Code, WezTerm, etc.)
- **Multi-layered fault tolerance** — hook liveness monitoring, sleep/resume recovery, IME-detection-failure fallback, and automatic TSF cold-start recovery, layered in stages
- **Asynchronous architecture** — an async executor built on the Windows message loop; blocking APIs are isolated on separate threads with timeout protection
- **Automatic focus detection** — automatically stops conversion when focus is not on a text input field
- **System tray resident** — switch layouts, open the settings screen, and toggle thumb-shift/romaji input
- **US layout support** — switch to a US physical layout with `keyboard_model = "us"`. Because a US keyboard has no Muhenkan/Henkan keys, awase can also impersonate thumb keys onto the left/right Alt keys, or turn the Space key into a thumb key

For details on the technical design, see [ARCHITECTURE.md](ARCHITECTURE.md).

---

## End of maintenance for v1, and moving to v2

With the release of awase v2.0.0 (2026-10-03), **the v1 line (1.x) is no longer maintained** (end-of-maintenance date: the v2.0.0 release date, 2026-10-03).
v1 will receive no further bug fixes or features. The last v1 release is 1.21.1.

- **We recommend upgrading to v2.** Download it from [GitHub Releases](https://github.com/cuzic/awase/releases). The full list of differences is in [docs/migration-v1-to-v2.md](docs/migration-v1-to-v2.md) (Japanese).
- Known problems that will not be fixed in v1 are listed in [Known issues remaining in v1](#known-issues-remaining-in-v1).
- Your `config.toml` carries over, but a few settings change in v2. See
  [Settings that change when moving from v1 to v2](#settings-that-change-when-moving-from-v1-to-v2).
- The in-app update notification of v1.21.1 does not announce v2. Check this page or GitHub Releases.

### Main bugs fixed in v2

Items not yet checked on a real machine (physical keys, real apps) say so. Details are in the BUG records under `docs/known-bugs/` (Japanese).

- Fixed: with Google Japanese Input in TSF-native apps such as Windows Terminal, the input could get stuck in katakana with no way back (BUG-173). Not yet fully verified on a real machine.
- Fixed: with Google Japanese Input in Chrome, fast typing could erase the composition in progress (BUG-168). Awaiting CI and real-machine confirmation.
- Fixed: with Google Japanese Input in Chrome-like windows, after another program injected a key that closed the IME, awase kept NICOLA on and plain romaji was typed (BUG-172). Confirmed in CI (Windows runners on GitHub Actions), and the effect was also seen on a real machine in Edge; however, one case of "IME still open but NICOLA turned off" was seen on a real machine and its cause is not identified (BUG-176). MS-IME is not covered.
- Fixed: right after startup awase could try to open the IME even though you had closed it (BUG-163). Not verified on a real machine.
- Key names in `config.toml` are now read by one common rule for every setting, and mistakes produce a warning instead of being silently ignored.
- The single-tap behavior of Muhenkan/Henkan was reorganized (see the migration section; whether it fixes the "@" seen with Windows Terminal + Google Japanese Input is not verified on a real machine).

### Known issues remaining in v1

These remain in v1.21.1 and will not be fixed in v1.

| What happens | When | Workaround | Status in v2 |
|---|---|---|---|
| Cannot return to hiragana; katakana keeps being typed (BUG-173) | Google Japanese Input + Windows Terminal etc. (TSF-native), after switching to katakana | Switch the IME again with Hankaku/Zenkaku | Fixed (not verified on a real machine) |
| Composition in progress disappears during very fast typing (BUG-168) | Chrome/Edge + Google Japanese Input | Retype | Fixed (awaiting CI/real-machine confirmation) |
| Composition in progress disappears in the middle of a word (BUG-171) | Chrome/Edge + Google Japanese Input, when typing after being idle | Retype; confirm before continuing | Not fixed in v2 either |
| After another program closes the IME, NICOLA does not resume and plain romaji is typed (BUG-172) | Chrome-like windows + Google Japanese Input | Switch with a physical key (Hankaku/Zenkaku etc.) | Fixed (MS-IME not covered) |
| With MS-IME, the first hiragana key does not work and the IME may close (BUG-152) | Only the first time, when the IME switch request times out | Press the hiragana key again | Fixed |
| Right after startup the IME is switched back on several times even if you closed it (BUG-163) | Right after starting awase with the IME closed | Switch after a short while | Fixed (not verified on a real machine) |
| Pressing Hankaku/Zenkaku repeatedly leaves the IME stuck on (BUG-142) | Windows Terminal + Google Japanese Input | Set `keys.ime_detect.toggle` | Permanent fix not yet done |
| Clearing the n-gram file field in the settings app reverts to the default, so it cannot be disabled (BUG-169) | Settings app | Edit `config.toml` directly | Not fixed in v2 either |

### Settings that change when moving from v1 to v2

Your `config.toml` carries over. These settings behave differently in v2.

- **The default of `keys.ime_toggle` is now empty.** It used to be `VK_KANJI` (Hankaku/Zenkaku). If your `config.toml` explicitly contains `VK_KANJI`, it is respected and kept. The exact old default written by the v1 settings app (`["VK_KANJI"]`) is treated as empty and removed when you save.
- **The defaults of `keys.ime_detect.on` / `off` are now empty** (they used to be `IMEオン` / `IMEオフ`). Following the IME On/Off keys works automatically without them. Values you wrote are used as before.
- **`keys.engine_on_ime_key` / `engine_off_ime_key` were removed.** If you saved settings in the settings app before 2026-08-15, the old defaults may remain in your `config.toml` and the feature that sends a full-width/half-width mode key when the engine turns on/off may have been active. In v2 it stops (leftover values are ignored and a notice appears at startup; saving from the settings app removes the lines). There is no replacement. Use `keys.ime_on` / `ime_off` to open/close the IME.
- **`muhenkan_solo_tap_ime_action` / `henkan_solo_tap_ime_action` were removed.** They are migrated to the equivalent `keys.ime_*` entries when loaded, with a warning (not migrated if the same key already has an entry). A single tap of Muhenkan/Henkan now follows the Suppress/Passthrough setting.
- **`confirm_mode` now has two choices: `wait` and `ngram_predictive`.** Old values are treated as `wait` on load and rewritten to `wait` on save.
- **Key-name parsing is now uniform.** Spellings that were silently ignored before (no `VK_`, lowercase, Japanese names, ...) now take effect, so a remap you forgot about may start working after the update. The old spelling `[[keymap]]` (correct: `[[keymaps]]`) also starts working.

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
  nicola.yab         ← NICOLA layout (Backspace/Escape substitute variant)
  nicola_keytop.yab  ← NICOLA layout (keytop-symbol variant, default for new installs)
  nicola_kakutei.yab ← NICOLA layout (keytop-symbol variant + confirm-on-punctuation)
  nicola_f.yab       ← for genuine Fujitsu thumb-shift keyboards (e.g. FKB7628-801)
  nicola_kb232.yab   ← for genuine Fujitsu thumb-shift keyboards (FMV-KB232)
  nicola_us.yab      ← US layout
data/
  ngram_hiragana.csv.gz  ← n-gram corpus (optional)
```

### 3. Launch

Double-click `awase.exe` and it becomes resident in the system tray.

### 4. Switch to thumb-shift input

Default key bindings:

| Action | Key |
|------|------|
| Switch to thumb-shift input | **Ctrl+Shift+変換** |
| Switch to romaji input | **Ctrl+Shift+無変換** |
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
default_layout  = "nicola_keytop.yab"
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
| `engine_toggle_hotkey` | none | Hotkey to toggle thumb-shift/romaji input |
| `keyboard_model` | `jis` | Physical keyboard layout. For a US layout use `"us"` (also change `default_layout` to `nicola_us.yab`) |

### Confirm Modes

| Mode | Characteristics |
|--------|------|
| `wait` | Waits until the timeout. Most accurate, with slight latency |
| `ngram_predictive` | Dynamically tunes the threshold using Wikipedia-derived n-gram statistics (n-gram file recommended) |

If unsure, start with `wait`, and if latency bothers you, try `ngram_predictive`.

> The older `speculative` / `two_phase` / `adaptive_timing` values were removed. If they remain in `config.toml`, awase warns on load and treats them as `wait`.

For details on how the n-gram mechanism works, see [ARCHITECTURE.md](ARCHITECTURE.md#n-gram-による同時打鍵判定の精度向上).

### Per-App Settings ([app_overrides])

Force specific behavior when an app doesn't work correctly.

```toml
[app_overrides]
# 常にテキスト入力として扱う
force_text = [
    { process = "myapp.exe", class = "Edit" },
]
# 常にローマ字入力にする（awase を素通しする）
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

The standard NICOLA layouts are bundled in two JIS variants. Both share the identical 44-key kana layout from the official NICOLA spec; they differ only in what they do with the physical key positions the spec leaves undefined (digit row columns 12-13, QWERTY row columns 11-12, home row columns 11-12).

- `layout/nicola_keytop.yab` (**default for new installs**): outputs the symbols actually printed on a standard JIS keyboard's keytops (＠／［／］／：／￥／＾) at those positions. The ＠ is an exception: the physical @ key is assigned to "、" (Japanese comma) by the official NICOLA spec, so a bare tap still outputs "、" — ＠ only appears while holding a thumb-shift key at that position.
- `layout/nicola.yab` (the default through v1.16.1): software-assigns those same positions to Backspace/Escape instead. Upgrading an existing install does not change the contents of `layout/nicola.yab` — the installer never silently overwrites a file you may have edited by hand in the Layout Editor tab. To switch to the keytop-symbol behavior, manually change `default_layout` to `"nicola_keytop.yab"`.

### Confirm-on-punctuation layout

`layout/nicola_kakutei.yab` is identical to `nicola_keytop.yab` except for two
cells: "。" (kuten) and "、" (touten). Pressing either key now sends Ctrl+M (the
IME's "confirm all" shortcut) right after outputting the punctuation mark — the
same mechanism used by やまぶき／やまぶきR and DvorakJ's "confirm on
punctuation" feature.

Just pick `nicola_kakutei.yab` under "Layout" in the settings screen — the
keystroke-sequence feature is on by default.

You'll also need to confirm that your IME (Google Japanese Input / MS-IME) has
Ctrl+M bound to "confirm all", and that no other application has Ctrl+M bound
to something else (that would conflict).

For a US layout, use `layout/nicola_us.yab`. Because a US keyboard physically lacks the Muhenkan/Henkan keys, the settings screen lets you impersonate thumb keys onto the left/right Alt keys, or assign the Space key as a thumb key.

`layout/nicola_f.yab` is also bundled for genuine Fujitsu thumb-shift keyboards (e.g. FKB7628-801). Its physical key layout and scan codes are identical to a JIS keyboard, so `keyboard_model` should stay `"jis"` — just set `default_layout` to `"nicola_f.yab"`.

The genuine Fujitsu thumb-shift keyboard "FMV-KB232" has a different symbol layout than `nicola_f.yab`, so a dedicated `layout/nicola_kb232.yab` is bundled as well. Set `default_layout` to `"nicola_kb232.yab"` (`keyboard_model` stays `"jis"`). This layout comes from a single user's real hardware, so other units or model variants may not match exactly.

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
→ It may be in romaji input. Switch to thumb-shift input with Ctrl+Shift+変換.

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
