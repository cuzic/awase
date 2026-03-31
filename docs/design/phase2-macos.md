# Phase 2 設計書: macOS 対応

## 概要

macOS 向けの `PlatformRuntime` 実装。Lacaille（macOS 親指シフト）を参考にする。

## プラットフォーム API マッピング

| 機能 | Windows | macOS |
|------|---------|-------|
| キーボードフック | WH_KEYBOARD_LL | CGEventTap (kCGHeadInsertEventTap) |
| キー出力 | SendInput | CGEventPost (kCGHIDEventTap) |
| IME 検出 | TSF + IMM32 | TISCopyCurrentKeyboardInputSource |
| IME 制御 | ImmSetOpenStatus | TISSelectInputSource |
| トレイ | Shell_NotifyIconW | NSStatusBar + NSStatusItem |
| タイマー | SetTimer | CFRunLoopTimerCreate |
| フォーカス検出 | WinEventHook | NSWorkspace.didActivateApplicationNotification |
| メッセージループ | GetMessageW | CFRunLoopRun / NSApplication.run |
| プロセス情報 | GetWindowThreadProcessId | NSRunningApplication |
| 修飾キー状態 | GetAsyncKeyState | CGEventFlags / NSEvent.modifierFlags |

## キーボードフック: CGEventTap

```rust
// Core Graphics Event Tap
// カーネルレベルのイベント監視（アクセシビリティ権限必要）
let tap = CGEventTapCreate(
    kCGSessionEventTap,        // セッション全体を監視
    kCGHeadInsertEventTap,     // イベントチェーンの先頭に挿入
    kCGEventTapOptionDefault,  // イベントを変更可能
    event_mask,                // kCGEventKeyDown | kCGEventKeyUp | kCGEventFlagsChanged
    callback,
    user_info,
);
```

### アクセシビリティ権限

macOS ではグローバルキーボード監視に**アクセシビリティ権限**が必要:
- システム環境設定 → セキュリティとプライバシー → アクセシビリティ
- `AXIsProcessTrusted()` でチェック
- 未許可なら `AXIsProcessTrustedWithOptions()` でダイアログ表示

### キーコード

macOS は VK コードではなく **keycode** を使う:
- keycode は物理キー位置（scan code に相当）
- 日本語 JIS キーボードでは Windows と異なるレイアウト
- `scan_to_pos` の macOS 版が必要（keycode → PhysicalPos）

### 親指キー

| キー | Windows VK | macOS keycode |
|------|-----------|---------------|
| 英数 (Eisuu) | VK_NONCONVERT (0x1D) | 0x66 (kVK_JIS_Eisu) |
| かな (Kana) | VK_CONVERT (0x1C) | 0x68 (kVK_JIS_Kana) |

Apple JIS キーボードでは Eisuu / Kana キーが親指位置にある。

## キー出力: CGEventPost

```rust
let event = CGEvent::new_keyboard_event(source, keycode, is_keydown);
event.set_flags(flags);  // Shift 等の修飾キー
CGEventPost(kCGHIDEventTap, event);
```

Unicode 出力:
```rust
let event = CGEvent::new_keyboard_event(source, 0, true);
event.set_string_from_utf16("か");  // Unicode 文字を直接設定
CGEventPost(kCGHIDEventTap, event);
```

## IME 検出: TISCopyCurrentKeyboardInputSource

```rust
let source = TISCopyCurrentKeyboardInputSource();
let source_id = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
// "com.apple.inputmethod.Japanese" → 日本語 IME
// "com.apple.inputmethod.Japanese.HiraganaInputMode" → ひらがなモード
```

### モード判定

| InputSourceID | ImeMode |
|--------------|---------|
| `*.Hiragana*` | Hiragana |
| `*.Katakana*` | Katakana |
| `*.HalfwidthKana*` | HalfKatakana |
| `*.Roman*` | Alphanumeric |
| `com.apple.keylayout.*` | Off |

## トレイ: NSStatusBar

```rust
let status_bar = NSStatusBar::system_status_bar();
let item = status_bar.status_item_with_length(NSSquareStatusItemLength);
item.set_title("合");  // または画像アイコン
item.set_menu(menu);   // 右クリックメニュー
```

## フォーカス検出

```rust
// アプリ切替通知
NSWorkspace::shared_workspace()
    .notification_center()
    .add_observer(
        selector!(on_app_activated:),
        NSWorkspaceDidActivateApplicationNotification,
    );

// バンドルID で分類
let app = NSRunningApplication::current();
let bundle_id = app.bundle_identifier();  // "com.apple.Safari" 等
```

macOS では Window Class の代わりに **Bundle Identifier** でアプリを識別する。
force_bypass の設定例:
```toml
force_bypass = [
    { process = "com.apple.Spotlight", class = "" },
]
```

## Rust クレート依存

| 機能 | クレート |
|------|---------|
| Core Graphics | `core-graphics` |
| Core Foundation | `core-foundation` |
| Cocoa (NSStatusBar 等) | `cocoa` |
| Accessibility | `accessibility-sys` or raw FFI |
| Input Source | `core-text` or raw FFI |

## macOS 固有の考慮事項

1. **アクセシビリティ権限** — 初回起動時にダイアログを表示し、設定を促す
2. **App Sandbox** — サンドボックス環境では CGEventTap が使えない。非サンドボックスで配布
3. **Apple Silicon vs Intel** — Universal Binary でビルド
4. **Lacaille との比較** — Lacaille は InputMethodKit を使った入力メソッド方式。awase は CGEventTap 方式で、より低レベルだが互換性が高い
5. **Karabiner-Elements との共存** — Karabiner も CGEventTap を使う。イベントチェーンの順序に注意
