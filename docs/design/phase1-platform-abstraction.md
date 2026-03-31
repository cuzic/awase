# Phase 1 設計書: プラットフォーム抽象化レイヤー

## 目的

既存の Windows 実装を壊さずに、プラットフォーム抽象化トレイトを整備する。
Phase 2 (macOS) / Phase 3 (Linux) で実装を差し替えるだけで動く土台を作る。

## 現状

- `platform.rs` に `KeyboardHook`, `KeySender`, `ImeDetector` トレイトが定義済み
- `executor.rs` が直接 Win32 API を呼んでいる（抽象化されていない）
- `main.rs` のイベントループが Win32 メッセージループに直結

## 追加するトレイト

### 1. PlatformRuntime（全プラットフォーム機能の統合）

```rust
/// プラットフォーム固有の機能を束ねるトレイト。
/// 各プラットフォーム（Windows/macOS/Linux）が1つの struct で実装する。
pub trait PlatformRuntime {
    // ── キーボード ──
    fn install_keyboard_hook(&mut self, callback: KeyCallback) -> anyhow::Result<()>;
    fn send_keys(&mut self, actions: &[KeyAction]);
    fn reinject_key(&mut self, event: &RawKeyEvent);

    // ── タイマー ──
    fn set_timer(&mut self, id: usize, duration: Duration);
    fn kill_timer(&mut self, id: usize);

    // ── IME ──
    fn detect_ime_state(&self) -> Option<bool>;  // CrossProcess 相当
    fn set_ime_open(&mut self, open: bool) -> bool;
    fn request_ime_refresh(&mut self);  // 非同期リフレッシュ要求

    // ── トレイ ──
    fn update_tray(&mut self, enabled: bool);
    fn show_balloon(&mut self, title: &str, message: &str);

    // ── フォーカス ──
    fn get_foreground_info(&self) -> Option<ForegroundInfo>;
    fn classify_focus(&self, info: &ForegroundInfo) -> FocusKind;
    fn read_modifier_state(&self) -> ModifierState;

    // ── イベントループ ──
    fn run_event_loop(&mut self, handler: &mut dyn EventHandler);
}
```

### 2. EventHandler（イベントループからのコールバック）

```rust
pub trait EventHandler {
    fn on_timer(&mut self, timer_id: usize);
    fn on_focus_changed(&mut self, info: ForegroundInfo);
    fn on_hotkey(&mut self, hotkey_id: i32);
    fn on_session_change(&mut self, locked: bool);
    fn on_power_change(&mut self, resumed: bool);
    fn on_tray_command(&mut self, command: TrayCommand);
    fn on_config_reload(&mut self);
    fn on_ime_refresh(&mut self);
}
```

### 3. ForegroundInfo（プラットフォーム非依存のフォーカス情報）

```rust
pub struct ForegroundInfo {
    pub process_id: u32,
    pub class_name: String,
    pub window_title: String,
    // macOS: bundle identifier, Linux: WM_CLASS
}
```

## executor.rs の変更

現在:
```rust
fn execute_effects(&mut self, effects: Vec<Effect>) {
    match effect {
        Effect::Timer(TimerEffect::Set { id, duration }) => {
            unsafe { SetTimer(HWND::default(), id, ms, None); }  // 直接 Win32
        }
    }
}
```

変更後:
```rust
fn execute_effects(&mut self, effects: Vec<Effect>, platform: &mut dyn PlatformRuntime) {
    match effect {
        Effect::Timer(TimerEffect::Set { id, duration }) => {
            platform.set_timer(id, duration);  // トレイト経由
        }
    }
}
```

## main.rs の変更

```rust
fn main() -> Result<()> {
    let mut platform = create_platform_runtime()?;  // cfg(target_os) で分岐
    let mut runtime = Runtime::new(&config, &mut platform)?;
    platform.run_event_loop(&mut runtime);
}

#[cfg(target_os = "windows")]
fn create_platform_runtime() -> Result<impl PlatformRuntime> {
    WindowsRuntime::new()
}

#[cfg(target_os = "macos")]
fn create_platform_runtime() -> Result<impl PlatformRuntime> {
    MacOsRuntime::new()
}

#[cfg(target_os = "linux")]
fn create_platform_runtime() -> Result<impl PlatformRuntime> {
    LinuxRuntime::new(&config)  // config でバックエンド選択
}
```

## ファイル構成（Phase 1 完了時）

```
src/
├── platform.rs          # トレイト定義（拡張）
├── main.rs              # cfg(target_os) で分岐
├── runtime.rs           # Runtime（PlatformRuntime を使う）
├── executor.rs          # PlatformRuntime 経由で Effect 実行
├── platform_windows/    # NEW: Win32 実装
│   ├── mod.rs
│   ├── hook.rs          # 現 hook.rs を移動
│   ├── output.rs        # 現 output.rs を移動
│   ├── ime.rs           # 現 ime.rs を移動
│   ├── tray.rs          # 現 tray.rs を移動
│   ├── focus/           # 現 focus/ を移動
│   └── observer/        # 現 observer/ を移動
```

## 検証

- Windows で `mise run pre-push` 全通過
- 動作変更なし（トレイト経由になるが出力は同じ）
- macOS / Linux ではコンパイルエラー（未実装）→ Phase 2/3 で対応
