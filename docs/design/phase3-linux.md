# Phase 3 設計書: Linux 対応

## 概要

Linux 向けの `PlatformRuntime` 実装。3つの入力バックエンド（evdev, X11 XRecord, libinput）をすべて実装し、config.toml で切り替え可能にする。

## 入力バックエンド比較

| | evdev | X11 XRecord | libinput |
|---|-------|------------|----------|
| **Wayland 対応** | ○ | ✕ | ○ |
| **X11 対応** | ○ | ○ | ○ |
| **権限** | input グループ | ユーザー空間 | input グループ |
| **root 必要** | 不要（input グループ） | 不要 | 不要（input グループ） |
| **デバイス指定** | 必要（/dev/input/eventX） | 不要 | 不要（自動検出） |
| **キーコード** | Linux evdev keycode | X11 keycode | Linux evdev keycode |
| **推奨用途** | Wayland + 一般 | X11 レガシー | Wayland + モダン |

## config.toml

```toml
[general]
# Linux 入力バックエンド
#   evdev    - /dev/input/eventX 直接読み取り（Wayland 対応、input グループ必要）
#   x11      - X11 XRecord 拡張（X11 専用、権限不要）
#   libinput - libinput イベント監視（Wayland 対応、input グループ必要）
linux_input_backend = "evdev"

# evdev バックエンド: キーボードデバイスパス（自動検出または手動指定）
# linux_evdev_device = "/dev/input/event0"
```

## プラットフォーム API マッピング

| 機能 | Windows | Linux (evdev) | Linux (X11) | Linux (libinput) |
|------|---------|--------------|-------------|-----------------|
| キーボードフック | WH_KEYBOARD_LL | evdev read | XRecordCreateContext | libinput_get_event |
| キー出力 | SendInput | uinput write | XTest XTestFakeKeyEvent | uinput write |
| IME 検出 | TSF + IMM32 | IBus/Fcitx D-Bus | IBus/Fcitx D-Bus | IBus/Fcitx D-Bus |
| トレイ | Shell_NotifyIconW | StatusNotifierItem (D-Bus) | 同左 | 同左 |
| タイマー | SetTimer | timerfd + epoll | 同左 | 同左 |
| フォーカス | WinEventHook | X11 XGetInputFocus / D-Bus | X11 XGetInputFocus | Wayland compositor API |
| イベントループ | GetMessageW | epoll_wait | XNextEvent + epoll | libinput_dispatch + epoll |

## バックエンド 1: evdev

### キーボード読み取り

```rust
// /dev/input/eventX を開いて排他取得
let fd = open("/dev/input/event3", O_RDONLY);
ioctl(fd, EVIOCGRAB, 1);  // 排他取得（他アプリにキーが届かなくなる）

// イベント読み取り
loop {
    let ev: input_event = read(fd);
    if ev.type_ == EV_KEY {
        // ev.code = KEY_A (30), ev.value = 0(up)/1(down)/2(repeat)
    }
}
```

### キー出力: uinput

```rust
// 仮想入力デバイスを作成
let uinput_fd = open("/dev/uinput", O_WRONLY);
ioctl(uinput_fd, UI_SET_EVBIT, EV_KEY);
// 全キーコードを登録
for key in 0..KEY_MAX {
    ioctl(uinput_fd, UI_SET_KEYBIT, key);
}
// デバイス作成
write(uinput_fd, &uinput_setup { name: "awase-virtual-kbd", ... });
ioctl(uinput_fd, UI_DEV_CREATE);

// キーイベント送信
write(uinput_fd, &input_event { type_: EV_KEY, code: KEY_A, value: 1 });
write(uinput_fd, &input_event { type_: EV_SYN, code: SYN_REPORT, value: 0 });
```

### キーコードマッピング

evdev keycode は Windows scan code とほぼ一致（Set 1 スキャンコード）:

| キー | evdev | Windows scan |
|------|-------|-------------|
| A | KEY_A (30) | 0x1E |
| S | KEY_S (31) | 0x1F |
| 変換 | KEY_HENKAN (92) | 0x79 |
| 無変換 | KEY_MUHENKAN (94) | 0x7B |

`scan_to_pos` がそのまま使える可能性が高い（evdev keycode = scan code）。

### デバイス自動検出

```rust
// /dev/input/event* を列挙し、キーボードデバイスを検出
for entry in fs::read_dir("/dev/input/")? {
    let path = entry?.path();
    if let Ok(fd) = open(&path, O_RDONLY) {
        let mut ev_bits = [0u8; EV_MAX / 8 + 1];
        ioctl(fd, EVIOCGBIT(0, ev_bits.len()), &mut ev_bits);
        if ev_bits[EV_KEY / 8] & (1 << (EV_KEY % 8)) != 0 {
            // キーボードデバイス
        }
    }
}
```

## バックエンド 2: X11 XRecord

### キーボード読み取り

```rust
let display = XOpenDisplay(null());
let ctx = XRecordCreateContext(display, ...);
XRecordEnableContextAsync(display, ctx, callback, null());

extern "C" fn callback(closure: *mut c_char, data: *mut XRecordInterceptData) {
    // data.data: XEvent (KeyPress / KeyRelease)
    let keycode = ...;
    let keysym = XKeycodeToKeysym(display, keycode, 0);
}
```

### キー出力: XTest

```rust
XTestFakeKeyEvent(display, keycode, is_press, CurrentTime);
XFlush(display);
```

### Unicode 出力

```rust
// XDoTool 方式: XSendEvent + XKeyEvent
// または: xdotool type "か"
// または: xclip + Ctrl+V
```

X11 での Unicode 出力は複雑。XTest は keycode ベースなので、Unicode 文字の直接送信には IM (Input Method) プロトコルが必要。実用的には `xdotool type` コマンドの内部ロジックを参考にする。

## バックエンド 3: libinput

### キーボード読み取り

```rust
let li = libinput_udev_create_context(&interface, null(), udev);
libinput_udev_assign_seat(li, "seat0");

loop {
    libinput_dispatch(li);
    while let Some(event) = libinput_get_event(li) {
        if libinput_event_get_type(event) == LIBINPUT_EVENT_KEYBOARD_KEY {
            let key_event = libinput_event_get_keyboard_event(event);
            let key = libinput_event_keyboard_get_key(key_event);  // evdev keycode
            let state = libinput_event_keyboard_get_key_state(key_event);
        }
    }
}
```

libinput は evdev keycode を使うので、キーマッピングは evdev と共通。
キー出力も uinput を使用（evdev と同じ）。

## IME 検出: IBus / Fcitx D-Bus

### IBus

```rust
// D-Bus: org.freedesktop.IBus
let bus = Connection::new_session()?;
let proxy = bus.with_proxy(
    "org.freedesktop.IBus",
    "/org/freedesktop/IBus",
    Duration::from_millis(100),
);
// 現在のエンジン取得
let engine: String = proxy.get("org.freedesktop.IBus", "GlobalEngine")?;
// "anthy" / "mozc" / "kkc" 等
```

### Fcitx5

```rust
// D-Bus: org.fcitx.Fcitx5
let proxy = bus.with_proxy(
    "org.fcitx.Fcitx5",
    "/controller",
    Duration::from_millis(100),
);
let state: i32 = proxy.method_call("org.fcitx.Fcitx.Controller1", "State", ())?;
// 0: inactive, 1: active
```

### ImeMode 判定

| IME エンジン | アクティブ | ImeMode |
|-------------|----------|---------|
| IBus なし | — | Off |
| anthy (hiragana) | ○ | Hiragana |
| mozc (direct) | ○ | Off（直接入力）|
| mozc (hiragana) | ○ | Hiragana |
| fcitx inactive | — | Off |

## トレイ: StatusNotifierItem (D-Bus)

```rust
// freedesktop.org StatusNotifierItem プロトコル
// D-Bus サービス: org.kde.StatusNotifierItem-{pid}-{id}
// インターフェース: org.kde.StatusNotifierItem
let item = StatusNotifierItem {
    id: "awase",
    title: "awase",
    icon_name: "input-keyboard",
    status: "Active",
    menu: ObjectPath("/MenuBar"),
};
```

代替: `libappindicator` / `ksni` クレートを使うと簡単。

## フォーカス検出

### X11

```rust
let mut focus_window: Window = 0;
let mut revert_to: i32 = 0;
XGetInputFocus(display, &mut focus_window, &mut revert_to);

// WM_CLASS プロパティで分類
let mut class_hint = XClassHint::default();
XGetClassHint(display, focus_window, &mut class_hint);
// class_hint.res_name = "wezterm"
// class_hint.res_class = "org.wezfurlong.wezterm"
```

### Wayland

Wayland にはグローバルなフォーカス検出 API がない。
代替: `wlr-foreign-toplevel-management` プロトコル（Sway, wlroots ベース）。
汎用的な方法がないため、Wayland では限定的な対応になる。

## イベントループ

```rust
// epoll ベースの統合イベントループ
let epoll = epoll_create1(0)?;

// 入力デバイス fd を登録
epoll_ctl(epoll, EPOLL_CTL_ADD, evdev_fd, &event)?;

// タイマー fd を登録
let timer_fd = timerfd_create(CLOCK_MONOTONIC, 0)?;
epoll_ctl(epoll, EPOLL_CTL_ADD, timer_fd, &event)?;

// X11 の場合は X11 接続 fd も登録
epoll_ctl(epoll, EPOLL_CTL_ADD, x11_fd, &event)?;

// D-Bus 接続 fd も登録（IME 監視用）
epoll_ctl(epoll, EPOLL_CTL_ADD, dbus_fd, &event)?;

loop {
    let n = epoll_wait(epoll, &mut events, -1)?;
    for event in &events[..n] {
        match event.data.fd {
            fd if fd == evdev_fd => handle_key_event(),
            fd if fd == timer_fd => handle_timer(),
            fd if fd == dbus_fd => handle_dbus(),
            _ => {}
        }
    }
}
```

## Rust クレート依存

| 機能 | クレート |
|------|---------|
| evdev | `evdev` |
| uinput | `uinput` or `evdev` (write mode) |
| libinput | `input` (libinput bindings) |
| X11 | `x11rb` or `xcb` |
| XTest | `x11rb` with xtest extension |
| D-Bus | `dbus` or `zbus` |
| epoll | `nix` or `mio` |
| トレイ | `ksni` (StatusNotifierItem) |

## Linux 固有の考慮事項

1. **権限**: evdev/libinput は `input` グループへの所属が必要。インストーラーでユーザーをグループに追加
2. **Wayland 制限**: グローバルキーボードフック、フォーカス検出、Unicode 出力が制限される
3. **IME 多様性**: IBus, Fcitx, SCIM 等。D-Bus 経由で統一的にアクセスするが、エンジンごとにモード判定が異なる
4. **日本語キーボード**: JIS 配列のキーコードは evdev でサポートされている（KEY_HENKAN, KEY_MUHENKAN, KEY_KATAKANAHIRAGANA）
5. **自動起動**: systemd user service (`~/.config/systemd/user/awase.service`) または XDG autostart (`~/.config/autostart/awase.desktop`)
