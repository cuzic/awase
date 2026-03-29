# キーボード配列エミュレータ 設計書

## 1. プロジェクト概要

### 1.1 目的

Windows 上で動作するキーボード配列エミュレータを Rust で開発する。
既存の「やまぶき」「DvorakJ」と同等の機能を持ち、NICOLA（親指シフト）を含む任意のキー配列をエミュレートできる常駐型ツールを目指す。

### 1.2 スコープ

- Windows 専用（Win32 API ベース）
- デスクトップ上のすべてのアプリケーション（UWP / WinUI / Win32 / ゲーム含む）に対して配列変換を適用
- 親指シフト（NICOLA）の同時打鍵判定に対応
- 設定ファイルによる配列定義の柔軟な切り替え

### 1.3 スコープ外（初期リリース時点）

- macOS / Linux 対応
- GUI による設定画面（初期は設定ファイル直接編集）
- Microsoft Store 配布（Win32 常駐アプリとして配布）
- IME 自体の実装（既存 IME と連携する前提）

---

## 2. 設計方針：シングルスレッド・イベント駆動

### 2.1 基本原則

本ツールは **シングルスレッド** で動作し、すべての処理を Win32 メッセージループ上のイベント駆動で行う。マルチスレッド・async ランタイム（tokio 等）は使用しない。

この方針を採る理由は以下の通り。

- `WH_KEYBOARD_LL` のフックコールバックは、`GetMessageW` を呼んでいるスレッドで実行される。フックコールバックとタイマー処理が同じスレッドで動けば、排他制御（Mutex / Arc）が一切不要になる。
- フックコールバックには約 300ms のタイムアウト制約がある。コールバック内では「受付」だけを行い、判定・出力はメッセージループ側で処理する構成にすることで、タイムアウトを回避する。
- `SetTimer` は OS カーネルのタイマーを利用し、時間経過後に `WM_TIMER` メッセージをメッセージキューに投入するだけなので、CPU 消費・メモリ消費ともにほぼゼロである。

### 2.2 メッセージループの全体像

```
アプリケーション起動
    │
    ▼
SetWindowsHookExW(WH_KEYBOARD_LL) でフック登録
    │
    ▼
┌─── メッセージループ（GetMessageW） ◄──────────────────┐
│       │                                              │
│       ├─ キーイベント（フックコールバック経由）         │
│       │   └─ engine.on_key_event() を呼ぶ            │
│       │       ├─ 即時確定 → output.send_key()        │
│       │       └─ 保留発生 → SetTimer(100ms) 起動     │
│       │                                              │
│       ├─ WM_TIMER（タイムアウト通知）                  │
│       │   └─ engine.on_timeout() を呼ぶ              │
│       │       └─ 保留キーを単独打鍵として確定          │
│       │          KillTimer() でタイマー停止            │
│       │                                              │
│       ├─ WM_HOTKEY（有効/無効切り替え）               │
│       │   └─ engine.toggle_enabled()                 │
│       │                                              │
│       └─ WM_QUIT                                     │
│           └─ ループ脱出 → フック解除 → 終了           │
│                                                      │
└──────────────────────────────────────────────────────┘
```

すべてのイベント（キー入力・タイマー・ホットキー・終了）が `GetMessageW` を通じて単一スレッドに届き、順次処理される。

---

## 3. システムアーキテクチャ

### 3.1 全体構成

```
┌─────────────────────────────────────────────────────┐
│                     OS (Windows)                     │
│                                                      │
│  ┌──────────┐    ┌────────────────────────────────┐  │
│  │ 他アプリ  │◄───│  キーボード配列エミュレータ      │  │
│  │(UWP/Win32│    │       （常駐プロセス）           │  │
│  │ /ゲーム) │    │                                │  │
│  └──────────┘    │  ┌──────────────────────────┐  │  │
│       ▲          │  │ メッセージループ (main)    │  │  │
│       │          │  │ GetMessageW              │  │  │
│  SendInput       │  │   ├ フックコールバック     │  │  │
│       │          │  │   ├ WM_TIMER             │  │  │
│       │          │  │   └ WM_HOTKEY            │  │  │
│       │          │  └───────────┬──────────────┘  │  │
│       │          │              │                  │  │
│       │          │  ┌───────────▼──────────────┐  │  │
│       │          │  │ engine（配列変換エンジン） │  │  │
│       │          │  │   状態機械 + 変換テーブル  │  │  │
│       │          │  └───────────┬──────────────┘  │  │
│       │          │              │                  │  │
│       │          │  ┌───────────▼──────────────┐  │  │
│       └──────────│──│ output（キー出力）        │  │  │
│                  │  │   SendInput + 状態追跡    │  │  │
│                  │  └──────────────────────────┘  │  │
│                  │              │                  │  │
│                  │  ┌───────────▼──────────────┐  │  │
│                  │  │ config（TOML 設定）       │  │  │
│                  │  └──────────────────────────┘  │  │
│                  └────────────────────────────────┘  │
│                                                      │
│  ┌──────────┐                                        │
│  │ keyboard │──── ハードウェア割り込み ─────────►     │
│  └──────────┘                                        │
└─────────────────────────────────────────────────────┘
```

### 3.2 処理フロー

```
キー押下（ハードウェア）
    │
    ▼
WH_KEYBOARD_LL フックコールバック呼び出し
    │
    ├─ dwExtraInfo == INJECTED_MARKER ?
    │   └─ YES → CallNextHookEx で素通し（無限ループ防止）
    │
    ├─ 変換対象外キー？（Ctrl+Alt+Del 等）
    │   └─ YES → CallNextHookEx で素通し
    │
    └─ 変換対象
        │
        ▼
    engine.on_key_event(event) を呼ぶ
        │
        ├─ Emit(actions)
        │   └─ 元キーを握りつぶし（LRESULT(1)）
        │      output.send_keys(actions) で注入
        │
        ├─ Pending
        │   └─ 元キーを握りつぶし（LRESULT(1)）
        │      SetTimer(TIMER_ID_PENDING, threshold_ms) 起動
        │      （次のキーイベント or WM_TIMER で確定する）
        │
        └─ PassThrough
            └─ CallNextHookEx で素通し

            ～ 時間経過 ～

    WM_TIMER 到着（メッセージループで受信）
        │
        ▼
    KillTimer(TIMER_ID_PENDING)
    engine.on_timeout() を呼ぶ
        │
        └─ 保留キーを単独打鍵として確定
           output.send_keys(actions) で注入
```

---

## 4. 同時打鍵判定の詳細設計

### 4.1 ハイブリッド判定方式

同時打鍵の判定には「イベント駆動優先 + SetTimer 安全網」のハイブリッド方式を採用する。

通常のタイピングでは、次のキーイベントが判定閾値（100ms）以内に到着するため、大半のケースはイベント駆動だけで判定が完了する。`SetTimer` は「しばらくキーを打たなかった場合に保留を掃除する」安全網としてのみ機能する。

```
通常のタイピング（99% のケース）:
  文字キー押下 → engine が保留 + SetTimer 起動
  30ms 後に次のキー押下 → engine が保留を遡及判定して確定
                          KillTimer（タイマーは発火せず）

最後の 1 文字（1% のケース）:
  文字キー押下 → engine が保留 + SetTimer 起動
  100ms 経過、次のキーなし → WM_TIMER 発火
                             engine.on_timeout() で単独確定
```

### 4.2 判定パターン

```
時間軸 →

【パターン1: 親指先行 → 即時確定】
  親指キー ████████████████████████
  文字キー         ████
  処理:    親指Down時   文字Down時:
           left_thumb    親指押下中なので
           = Some(t0)    即座にシフト面を出力
                         → Emit（タイマー不要）

【パターン2: 文字先行 → 時間内に親指 → イベント駆動で確定】
  文字キー ████████████████
  親指キー       ████████████████████
  処理:    文字Down時:   親指Down時:
           pending       保留あり & 時間内
           = Some(vk,t0) → 同時打鍵として確定
           SetTimer起動   KillTimer
                          → Emit

【パターン3: 文字単独 → タイムアウトで確定】
  文字キー ████████████
  処理:    文字Down時:       WM_TIMER 到着:
           pending           on_timeout()
           = Some(vk,t0)     → 単独打鍵として確定
           SetTimer起動       KillTimer
                              → Emit

【パターン4: 文字連打 → 前の保留をイベント駆動で確定】
  文字キー1 ████████████
  文字キー2       ████████████
  処理:     文字1 Down:   文字2 Down:
            pending        保留あり & 親指なし
            = Some(vk1,t0) → 文字1を単独確定
            SetTimer起動    → 文字2を新たに保留
                            KillTimer → SetTimer 再起動

【パターン5: 親指単独 → タイムアウトで親指キー機能を出力】
  親指キー ████████████
  処理:    親指Down時:       WM_TIMER 到着:
           pending           on_timeout()
           = Some(thumb,t0)  → スペース/変換/無変換を出力
           SetTimer起動       KillTimer
```

### 4.3 engine の状態遷移

```rust
/// エンジンの内部状態
struct Engine {
    /// 配列定義
    layout: KeyLayout,

    /// 現在押下中の修飾キーの状態
    modifiers: ModifierState,

    /// 左親指キーが押下中か（押下時刻を保持）
    left_thumb_down: Option<Instant>,

    /// 右親指キーが押下中か（押下時刻を保持）
    right_thumb_down: Option<Instant>,

    /// 同時打鍵判定用：未確定の保留キー
    pending: Option<PendingKey>,

    /// 同時打鍵の判定閾値
    threshold: Duration,

    /// 物理キー → 注入済みキーの対応（KeyUp 時の整合性維持用）
    active_keys: HashMap<u16, u16>,

    /// エンジンの有効/無効
    enabled: bool,
}

struct PendingKey {
    vk_code: u16,
    scan_code: u32,
    timestamp: Instant,
    kind: PendingKind,
}

enum PendingKind {
    /// 文字キーが保留中（親指キーの到着を待っている）
    CharKey,
    /// 親指キーが保留中（文字キーの到着を待っている）
    ThumbKey { is_left: bool },
}
```

### 4.4 engine の主要メソッド

```rust
impl Engine {
    /// キーイベント到着時に呼ばれる（フックコールバックから）
    ///
    /// 戻り値:
    /// - Emit(actions): 即座に出力すべきアクション列
    /// - Pending: 保留が発生した（呼び出し側で SetTimer を起動する）
    /// - PassThrough: 元のキーをそのまま通す
    fn on_key_event(&mut self, event: RawKeyEvent) -> EngineOutput {
        if !self.enabled {
            return EngineOutput::PassThrough;
        }

        match event.event_type {
            KeyEventType::KeyDown | KeyEventType::SysKeyDown => {
                self.on_key_down(event)
            }
            KeyEventType::KeyUp | KeyEventType::SysKeyUp => {
                self.on_key_up(event)
            }
        }
    }

    /// タイムアウト時に呼ばれる（WM_TIMER ハンドラから）
    ///
    /// 保留中のキーを単独打鍵として確定し、出力アクションを返す。
    fn on_timeout(&mut self) -> Option<Vec<KeyAction>> {
        let pending = self.pending.take()?;

        match pending.kind {
            PendingKind::CharKey => {
                // 文字キーが単独 → 通常面の文字を出力
                let action = self.layout.normal.get(&pending.vk_code)?;
                Some(vec![action.clone()])
            }
            PendingKind::ThumbKey { .. } => {
                // 親指キーが単独 → スペース/変換/無変換を出力
                Some(vec![KeyAction::Key(pending.vk_code)])
            }
        }
    }

    fn on_key_down(&mut self, event: RawKeyEvent) -> EngineOutput {
        let is_thumb = self.is_thumb_key(event.vk_code);

        // ── 保留キーがある場合：遡及判定 ──
        if let Some(pending) = self.pending.take() {
            let elapsed = event.timestamp - pending.timestamp;

            if elapsed < self.threshold {
                match (&pending.kind, is_thumb) {
                    // 保留=文字, 到着=親指 → 同時打鍵
                    (PendingKind::CharKey, true) => {
                        let face = self.thumb_face(event.vk_code);
                        if let Some(action) = face.get(&pending.vk_code) {
                            return EngineOutput::Emit(vec![action.clone()]);
                        }
                    }
                    // 保留=親指, 到着=文字 → 同時打鍵
                    (PendingKind::ThumbKey { is_left }, false) => {
                        let face = if *is_left {
                            &self.layout.left_thumb
                        } else {
                            &self.layout.right_thumb
                        };
                        if let Some(action) = face.get(&event.vk_code) {
                            return EngineOutput::Emit(vec![action.clone()]);
                        }
                    }
                    // その他（文字+文字, 親指+親指）
                    _ => {
                        // 前の保留を単独確定してから、今回を処理
                        // （呼び出し側で前の分を先に send する）
                    }
                }
            }
            // 時間超過 or 同種キー → 前の保留を単独確定
            // ...
        }

        // ── 保留キーがない場合 ──

        // 親指キーが既に押下中なら即時同時打鍵
        if !is_thumb {
            if let Some(face) = self.active_thumb_face() {
                if let Some(action) = face.get(&event.vk_code) {
                    return EngineOutput::Emit(vec![action.clone()]);
                }
            }
        }

        // 新たに保留
        self.pending = Some(PendingKey {
            vk_code: event.vk_code,
            scan_code: event.scan_code,
            timestamp: event.timestamp,
            kind: if is_thumb {
                PendingKind::ThumbKey {
                    is_left: self.is_left_thumb(event.vk_code),
                }
            } else {
                PendingKind::CharKey
            },
        });
        EngineOutput::Pending
    }

    fn on_key_up(&mut self, event: RawKeyEvent) -> EngineOutput {
        // 親指キーのリリース追跡
        if self.is_thumb_key(event.vk_code) {
            if self.is_left_thumb(event.vk_code) {
                self.left_thumb_down = None;
            } else {
                self.right_thumb_down = None;
            }
        }

        // active_keys から対応する注入済みキーを探してリリース
        if let Some(injected_vk) = self.active_keys.remove(&event.vk_code) {
            return EngineOutput::Emit(vec![KeyAction::KeyUp(injected_vk)]);
        }

        EngineOutput::PassThrough
    }
}
```

---

## 5. SetTimer の運用ルール

### 5.1 タイマー ID の定義

```rust
const TIMER_ID_PENDING: usize = 1;  // 同時打鍵判定用タイマー
```

### 5.2 タイマーのライフサイクル

| タイミング | 操作 |
|---|---|
| `on_key_event` が `Pending` を返した | `SetTimer(HWND::default(), TIMER_ID_PENDING, threshold_ms, None)` |
| `on_key_event` が `Emit` を返した（保留が遡及判定で確定した） | `KillTimer(HWND::default(), TIMER_ID_PENDING)` |
| `WM_TIMER` を受信した | `KillTimer` → `engine.on_timeout()` → `output.send_keys()` |

### 5.3 タイマーの再起動

パターン4（文字連打）のように、前の保留をイベント駆動で確定した直後に新しい保留が発生する場合がある。この場合は `KillTimer` → `SetTimer` の順で再起動する。`SetTimer` を同じ ID で再度呼ぶと、既存のタイマーがリセットされるため、明示的に `KillTimer` しなくても動作上は問題ないが、意図を明確にするために `KillTimer` を先に呼ぶ。

### 5.4 SetTimer の精度について

`SetTimer` の精度は約 10〜16ms（Windows のタイマー解像度に依存）。同時打鍵の判定閾値が 100ms であれば、最大 16ms の誤差は許容範囲内である。より高精度が必要な場合は `timeSetEvent`（マルチメディアタイマー）への置き換えを検討するが、初期リリースでは不要と判断する。

### 5.5 メッセージループのコード

```rust
fn run_message_loop(engine: &mut Engine, output: &Output, threshold_ms: u32) {
    let mut msg = MSG::default();

    loop {
        let ret = unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) };
        if ret.0 <= 0 {
            break; // WM_QUIT or エラー
        }

        match msg.message {
            WM_TIMER if msg.wParam.0 == TIMER_ID_PENDING => {
                // タイムアウト：保留キーを単独打鍵として確定
                unsafe { KillTimer(HWND::default(), TIMER_ID_PENDING) };
                if let Some(actions) = engine.on_timeout() {
                    output.send_keys(&actions);
                }
            }
            _ => {
                unsafe { DispatchMessageW(&msg) };
            }
        }
    }
}
```

### 5.6 フックコールバックのコード

```rust
unsafe extern "system" fn hook_callback(
    ncode: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if ncode >= 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

        // ── 自己注入チェック（無限ループ防止）──
        if kb.dwExtraInfo == INJECTED_MARKER {
            return CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam);
        }

        let event = RawKeyEvent::from_hook(wparam, kb);

        // ── engine に渡して判定 ──
        match ENGINE.on_key_event(event) {
            EngineOutput::Emit(actions) => {
                // 即時確定：保留が遡及判定で解消された場合はタイマーも止める
                KillTimer(HWND::default(), TIMER_ID_PENDING);
                OUTPUT.send_keys(&actions);
                return LRESULT(1); // 元キーを握りつぶす
            }
            EngineOutput::Pending => {
                // 保留発生：タイマー起動
                SetTimer(HWND::default(), TIMER_ID_PENDING, THRESHOLD_MS, None);
                return LRESULT(1); // 元キーを握りつぶす
            }
            EngineOutput::PassThrough => {
                // 何もしない（元キーをそのまま通す）
            }
        }
    }

    CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam)
}
```

---

## 6. モジュール設計

### 6.1 モジュール一覧

| モジュール | ファイル | 責務 |
|---|---|---|
| `hook` | `hook.rs` | フック登録・解除、コールバック定義 |
| `engine` | `engine.rs` | 配列変換エンジン（状態機械 + 同時打鍵判定） |
| `output` | `output.rs` | SendInput によるキー注入、自己注入マーカー管理、キー状態追跡 |
| `config` | `config.rs` | TOML 設定ファイルの読み込み・パース |
| `types` | `types.rs` | 共通の型定義 |
| `main` | `main.rs` | エントリポイント、メッセージループ、SetTimer 管理 |
| `tray`（Phase 5） | `tray.rs` | システムトレイアイコン |

### 6.2 output モジュール

#### 自己注入の識別

`dwExtraInfo` にマジックナンバーを設定して注入し、フック側でチェックする。

```rust
const INJECTED_MARKER: usize = 0x4B45_594D; // "KEYM"

fn send_key(vk: u16, is_keyup: bool) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                dwExtraInfo: INJECTED_MARKER,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_INPUT_FLAGS::default()
                },
                ..Default::default()
            },
        },
    };
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
}
```

#### キー状態追跡（KeyUp 整合性）

物理キーと注入キーの対応を `HashMap` で保持し、物理キーの KeyUp 時に正しい注入キーの KeyUp を送る。

```rust
struct KeyStateTracker {
    /// key: 物理 vkCode, value: 注入した vkCode
    active_keys: HashMap<u16, u16>,
}

impl KeyStateTracker {
    fn on_key_down(&mut self, physical_vk: u16, injected_vk: u16) {
        self.active_keys.insert(physical_vk, injected_vk);
    }

    fn on_key_up(&mut self, physical_vk: u16) -> Option<u16> {
        self.active_keys.remove(&physical_vk)
    }
}
```

### 6.3 config モジュール

#### 設定ファイルの構造

```toml
[general]
# 同時打鍵の判定閾値（ミリ秒）
simultaneous_threshold_ms = 100

# 親指キーの割り当て
left_thumb_key = "VK_MUHENKAN"    # 無変換
right_thumb_key = "VK_CONVERT"     # 変換

# 有効/無効を切り替えるホットキー
toggle_hotkey = "Ctrl+Shift+F12"

[layout]
name = "NICOLA"

# 通常面
[layout.normal]
VK_Q = "。"
VK_W = "か"
VK_E = "た"
VK_R = "こ"
VK_T = "さ"
VK_Y = "ら"
VK_U = "ち"
VK_I = "く"
VK_O = "つ"
VK_P = "，"
VK_A = "う"
VK_S = "し"
VK_D = "て"
VK_F = "け"
VK_G = "せ"
VK_H = "は"
VK_J = "と"
VK_K = "き"
VK_L = "い"
VK_OEM_PLUS = "ん"
VK_Z = "．"
VK_X = "ひ"
VK_C = "す"
VK_V = "ふ"
VK_B = "へ"
VK_N = "め"
VK_M = "そ"
VK_OEM_COMMA = "ね"
VK_OEM_PERIOD = "ほ"
VK_OEM_2 = "・"

# 左親指シフト面
[layout.left_thumb]
VK_Q = "ぁ"
VK_W = "え"
VK_E = "り"
VK_R = "ゃ"
VK_T = "れ"
VK_Y = "ぱ"
VK_U = "ぢ"
VK_I = "ぐ"
VK_O = "づ"
VK_P = "ぴ"

# 右親指シフト面
[layout.right_thumb]
VK_Q = "ぅ"
VK_W = "が"
VK_E = "だ"
VK_R = "ご"
VK_T = "ざ"
VK_Y = "よ"
VK_U = "に"
VK_I = "る"
VK_O = "ま"
VK_P = "ぇ"

# Shift 面
[layout.shift]
VK_Q = "ぁ"
# ... 以下同様
```

### 6.4 types モジュール

```rust
/// フックから受け取る生のキーイベント
struct RawKeyEvent {
    vk_code: u16,
    scan_code: u32,
    event_type: KeyEventType,
    extra_info: usize,
    timestamp: Instant,
}

enum KeyEventType {
    KeyDown,
    KeyUp,
    SysKeyDown,
    SysKeyUp,
}

/// エンジンが返す出力指示
enum EngineOutput {
    /// 即座に出力すべきアクション列
    Emit(Vec<KeyAction>),
    /// 保留が発生した（呼び出し側で SetTimer を起動する）
    Pending,
    /// 元のキーをそのまま通す
    PassThrough,
}

/// 出力アクション
enum KeyAction {
    /// 単一の仮想キーコードを押下
    Key(u16),
    /// 単一の仮想キーコードをリリース
    KeyUp(u16),
    /// 文字列（ローマ字列など）を出力
    String(String),
    /// Unicode 文字を直接出力（SendInput の KEYEVENTF_UNICODE）
    Char(char),
    /// 何もしない（キーを握りつぶす）
    Suppress,
}
```

---

## 7. 無限ループ防止の設計

### 7.1 問題

SendInput でキーを注入 → フックが再度捕捉 → 再変換 → 再注入 → ... の無限ループが発生しうる。

### 7.2 対策（3 重の安全弁）

1. **dwExtraInfo マーカー**: 注入時に `INJECTED_MARKER` を設定し、フック冒頭でチェック。一致すれば即座に `CallNextHookEx` で素通し。
2. **OS 予約キーの除外**: Ctrl+Alt+Delete, Win キー等の OS レベルのキーコンビネーションは常に素通し。
3. **再入ガード**: フックコールバック内で `static` な再入フラグを持ち、多重呼び出しを検出した場合は即座に素通し。

---

## 8. IME 連携

### 8.1 方針

初期リリースでは「かな入力モード固定」を推奨構成とする。ローマ字入力モードでの動作も設定により可能とするが、優先度は低い。

### 8.2 IME 状態の検知

`windows` クレートの `Win32_UI_Input_Ime` feature で IMM32 API を利用する。

```rust
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetConversionStatus, ImmReleaseContext,
};

fn get_ime_mode() -> ImeMode {
    unsafe {
        let hwnd = GetForegroundWindow();
        let himc = ImmGetContext(hwnd);
        let mut conversion = 0u32;
        let mut sentence = 0u32;
        ImmGetConversionStatus(himc, &mut conversion, &mut sentence);
        ImmReleaseContext(hwnd, himc);
        // conversion フラグから IME_CMODE_NATIVE, IME_CMODE_KATAKANA 等を判定
    }
}
```

### 8.3 出力方式の切り替え

| IME モード | 出力方式 |
|---|---|
| かな入力 ON | `SendInput` で仮想キーコード送信（IME がかなに変換） |
| ローマ字入力 ON | `SendInput` でローマ字列を送信（例: "ka" → か） |
| IME OFF | 配列変換をバイパスし、元のキーを素通し |

---

## 9. 技術的な制約・リスク

### 9.1 フックのタイムアウト

Windows はフックコールバックが約 300ms 以内に戻らない場合、フックを自動的に解除する。本設計ではコールバック内で `engine.on_key_event()` を呼ぶだけ（HashMap ルックアップ＋状態更新）なので、数マイクロ秒で完了する。`SetTimer` の呼び出し自体もノンブロッキングで即座に返る。

### 9.2 セキュリティソフトとの競合

一部のセキュリティソフトやアンチチートツールは、グローバルキーボードフックを不審な動作として検出・ブロックする場合がある。

### 9.3 管理者権限

UAC が有効な環境では、管理者権限で動作しているアプリケーションに対しては、非管理者権限のフックが効かない場合がある。必要に応じて管理者として実行するオプションを用意する。

### 9.4 ゲームとの互換性

DirectInput や Raw Input を直接使用しているゲームでは、`WH_KEYBOARD_LL` フックが効かないケースがある。この場合は対象外とする。

---

## 10. 使用クレート

| クレート | バージョン | 用途 |
|---|---|---|
| `windows` | 0.58+ | Win32 API（フック, SendInput, SetTimer, IME） |
| `toml` | 0.8+ | 設定ファイルパース |
| `serde` / `serde_derive` | 1.x | 設定ファイルのデシリアライズ |
| `log` / `env_logger` | 0.4+ / 0.11+ | ログ出力 |
| `anyhow` | 1.x | エラーハンドリング |

`windows` クレートの有効化する feature:

```toml
[dependencies.windows]
version = "0.58"
features = [
    "Win32_Foundation",
    "Win32_UI_WindowsAndMessaging",    # Hook, SetTimer, GetMessageW
    "Win32_UI_Input_KeyboardAndMouse", # SendInput, GetAsyncKeyState
    "Win32_UI_Input_Ime",              # ImmGetContext 等（Phase 4 以降）
]
```

---

## 11. ディレクトリ構成

```
keyboard-hook/
├── Cargo.toml
├── config/
│   └── nicola.toml           # NICOLA 配列定義サンプル
├── src/
│   ├── main.rs               # エントリポイント、メッセージループ、SetTimer 管理
│   ├── hook.rs               # フック登録・コールバック
│   ├── engine.rs             # 配列変換エンジン（状態機械 + 同時打鍵判定）
│   ├── output.rs             # SendInput によるキー注入、キー状態追跡
│   ├── config.rs             # TOML 設定ファイル読み込み
│   └── types.rs              # 共通型定義
└── docs/
    └── design.md             # 本設計書
```

---

## 12. 実装フェーズ

### Phase 1: 最小キーフック（ログ出力のみ）

- `WH_KEYBOARD_LL` フックで全キーイベントを取得し、コンソールにログ出力する。
- `GetMessageW` によるメッセージループの基本形を確立する。
- 検証: メモ帳・ブラウザ・UWP アプリでキーイベントが捕捉できることを確認。

### Phase 2: 単純キー置換 + 無限ループ防止

- 特定キーを別キーに置き換える（A→B など）。
- `dwExtraInfo` による自己注入マーカーを実装し、無限ループ防止を確立する。
- `KeyStateTracker` による KeyUp 整合性を実装する。
- 検証: 全アプリで変換後のキーだけが入力されることを確認。

### Phase 3: TOML 設定ファイル対応

- TOML から配列定義を読み込み、任意の単純配列をエミュレートする。
- 検証: QWERTY → DvorakJP の変換が正しく動作することを確認。

### Phase 4: 親指シフト対応（SetTimer による同時打鍵判定）

- `SetTimer` / `WM_TIMER` を使ったハイブリッド同時打鍵判定を実装する。
- イベント駆動優先 + SetTimer 安全網の方式を実装する。
- 5 つの判定パターンすべてが正しく動作することを検証する。
- IME 状態検知と出力方式の切り替えを実装する。

### Phase 5: 常駐化・利便性向上

- システムトレイアイコン。
- ホットキーによる有効/無効切り替え（`RegisterHotKey` + `WM_HOTKEY`）。
- 配列の動的切替。
