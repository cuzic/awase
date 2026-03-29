# awase

A simultaneous-keystroke keyboard remapper for NICOLA thumb shift and beyond.

**awase** (合わせ) remaps physical keys based on simultaneous keystroke detection — pressing a character key and a thumb key at the same time produces a different character, like playing a chord on a piano.

## Features

- **NICOLA thumb shift** with accurate 3-key arbitration (d1/d2 comparison per NICOLA spec)
- **Yamabuki-compatible** `.yab` layout files
- **n-gram adaptive threshold** — adjusts simultaneous detection window based on Japanese character frequency
- **IME integration** — TSF + IMM32 hybrid detection, auto-bypass when IME is off
- **System tray** with layout switching and hotkey toggle
- **timed-fsm** — a reusable timed finite state machine framework (included as a workspace crate)

## Architecture

```
Physical key + timestamp
    → timed-fsm (simultaneous keystroke detection)
    → .yab layout lookup (physical position → romaji)
    → SendInput (romaji VK codes → IME → kana)
```

The engine is platform-independent. Windows-specific code (hooks, SendInput, IME) is isolated behind traits for future cross-platform support.

## Quick Start

```sh
# Build (requires Windows target for the binary)
cargo build --release --target x86_64-pc-windows-gnu

# Run
awase.exe
```

Place `config.toml` and `layout/nicola.yab` next to the executable.

## Configuration

**config.toml** — application settings:
```toml
[general]
simultaneous_threshold_ms = 100
toggle_hotkey = "Ctrl+Shift+F12"
layouts_dir = "layout"
default_layout = "nicola.yab"
```

**layout/nicola.yab** — yamabuki-compatible key layout:
```
[ローマ字シフト無し]
．,ｋａ,ｔａ,ｋｏ,ｓａ,ｒａ,ｔｉ,ｋｕ,ｔｕ,'，',，,無
ｕ,ｓｉ,ｔｅ,ｋｅ,ｓｅ,ｈａ,ｔｏ,ｋｉ,ｉ,ｎｎ,後,逃
...
```

## Testing

```sh
cargo test --lib -p awase          # 157 unit tests
cargo test --test scenarios        # 7 scenario tests
cargo test -p timed-fsm            # 24 framework tests
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
