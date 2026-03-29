# キーボード配列エミュレータ 設計書

## 1. プロジェクト概要

### 1.1 目的

Windows 上で動作するキーボード配列エミュレータを Rust で開発する。
既存の「やまぶき」「DvorakJ」と同等の機能を持ち、NICOLA（親指シフト）を含む任意のキー配列をエミュレートできる常駐型ツールを目指す。

### 1.2 スコープ

- Windows 専用（Win32 API ベース）
- デスクトップ上のすべてのアプリケーション（UWP / WinUI / Win32 / ゲーム含む）に対して配列変換を適用
- 親指シフト（NICOLA）の同時打鍵判定に対応
- 拡張親指シフト、文字キー同時打鍵シフトに対応
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
    |
    v
SetWindowsHookExW(WH_KEYBOARD_LL) でフック登録
    |
    v
+--- メッセージループ（GetMessageW） <-----------------+
|       |                                              |
|       +- キーイベント（フックコールバック経由）        |
|       |   +- engine.on_key_event() を呼ぶ            |
|       |       +- 即時確定 -> output.send_key()        |
|       |       +- 保留発生 -> SetTimer(threshold) 起動 |
|       |                                              |
|       +- WM_TIMER（タイムアウト通知）                 |
|       |   +- engine.on_timeout() を呼ぶ              |
|       |       +- 保留キーを単独打鍵として確定          |
|       |          KillTimer() でタイマー停止            |
|       |                                              |
|       +- WM_HOTKEY（一時停止/再開切り替え）           |
|       |   +- engine.toggle_enabled()                 |
|       |                                              |
|       +- WM_QUIT                                     |
|           +- ループ脱出 -> フック解除 -> 終了          |
|                                                      |
+------------------------------------------------------+
```

すべてのイベント（キー入力・タイマー・ホットキー・終了）が `GetMessageW` を通じて単一スレッドに届き、順次処理される。

---

## 3. システムアーキテクチャ

### 3.1 処理フロー

```
キー押下（ハードウェア）
    |
    v
WH_KEYBOARD_LL フックコールバック呼び出し
    |
    +- dwExtraInfo == INJECTED_MARKER ?
    |   +- YES -> CallNextHookEx で素通し（無限ループ防止）
    |
    +- 変換対象外キー？（Ctrl+Alt+Del 等）
    |   +- YES -> CallNextHookEx で素通し
    |
    +- 一時停止中？
    |   +- YES -> CallNextHookEx で素通し
    |
    +- 変換対象
        |
        v
    engine.on_key_event(event) を呼ぶ
        |
        +- Emit(actions)
        |   +- 元キーを握りつぶし（LRESULT(1)）
        |      output.send_keys(actions) で注入
        |
        +- Pending
        |   +- 元キーを握りつぶし（LRESULT(1)）
        |      SetTimer(timer_id, threshold_ms) 起動
        |
        +- PassThrough
            +- CallNextHookEx で素通し

            ~ 時間経過 ~

    WM_TIMER 到着（メッセージループで受信）
        |
        v
    KillTimer(timer_id)
    engine.on_timeout(timer_id) を呼ぶ
        |
        +- 保留キーを単独打鍵として確定
           output.send_keys(actions) で注入
```

---

## 4. 設定体系の設計（やまぶきR 互換）

やまぶきR の設定タブ構造に準拠し、以下の設定項目をサポートする。

### 4.1 親指シフト設定

| 設定項目 | 説明 | デフォルト値 |
|---|---|---|
| 左親指シフトキー | 左親指シフトに使用する物理キー | 無変換（VK_NONCONVERT） |
| 右親指シフトキー | 右親指シフトに使用する物理キー | 変換（VK_CONVERT） |
| 連続シフト（左） | 左親指キーを押し続けている間、連続してシフト面の文字を入力できる | OFF |
| 連続シフト（右） | 右親指キーを押し続けている間、連続してシフト面の文字を入力できる | OFF |
| 単独打鍵（左） | 左親指キーを単独で押した場合にキー本来の機能を出力する | 有効 |
| 単独打鍵（右） | 右親指キーを単独で押した場合にキー本来の機能を出力する | 有効 |
| キーリピート（左） | 左親指キーの長押し時にリピートを有効にする | ON |
| キーリピート（右） | 右親指キーの長押し時にリピートを有効にする | ON |
| 同時打鍵判定時間 | 親指シフトが同時打鍵と判定される時間範囲（ミリ秒） | 65 |

#### 連続シフトの動作

連続シフトが OFF の場合、親指キー＋文字キーで1文字出力するたびにシフト状態がリセットされる（通常の親指シフト動作）。

連続シフトが ON の場合、親指キーを押し続けている間は何文字でもシフト面の文字を連続入力できる。PC キーボードの Shift キーと同じ挙動になる。

#### 単独打鍵の動作

単独打鍵が「有効」の場合、親指キーを単独で押してタイムアウトすると、キー本来の機能（無変換→無変換、変換→変換）を出力する。

単独打鍵が「無効」の場合、親指キーを単独で押しても何も出力しない（シフト専用キーとして機能する）。

### 4.2 拡張親指シフト設定

通常の親指シフト（2面）に加え、追加のシフト面を定義できる。

| 設定項目 | 説明 | デフォルト値 |
|---|---|---|
| 拡張親指シフトキー1 | 拡張シフト面1に使用する物理キー | 無変換（VK_NONCONVERT） |
| 拡張親指シフトキー2 | 拡張シフト面2に使用する物理キー | なし |
| 連続シフト（拡張1） | 拡張1の連続シフト | ON |
| 連続シフト（拡張2） | 拡張2の連続シフト | ON |
| 単独打鍵（拡張1） | 拡張1の単独打鍵 | 無効 |
| 単独打鍵（拡張2） | 拡張2の単独打鍵 | 無効 |
| キーリピート（拡張1） | 拡張1のキーリピート | OFF |
| キーリピート（拡張2） | 拡張2のキーリピート | OFF |

拡張親指シフトは、通常の親指シフトとは別の追加シフト面として機能する。連続シフト ON・単独打鍵無効がデフォルトであり、修飾キー的な使い方を想定している。

### 4.3 文字キー同時打鍵シフト設定

文字キー同士の同時押しによるシフト機能を提供する。

| 設定項目 | 説明 | デフォルト値 |
|---|---|---|
| 連続シフト | 文字キー同時打鍵シフトの連続シフト | ON |
| 同時打鍵判定時間 | 文字キー同士が同時打鍵と判定される時間範囲（ミリ秒） | 65 |

文字キー同時打鍵シフトは、特定の文字キーの組み合わせを同時に押すと別の文字を出力する機能である。親指シフトの判定時間とは独立して設定できる。

### 4.4 動作モード設定

| 設定項目 | 説明 | デフォルト値 |
|---|---|---|
| 一時停止用ショートカットキー | エミュレータの一時停止/再開を切り替えるキー | Pause |
| IME アクセス API | IME の状態取得に使用する API の種類（IMM / TSF） | IMM |
| 日本語以外の入力言語での文字キー入れ替え | 入力言語が日本語以外の場合にも配列変換を適用するか | 有効 |
| 設定ファイル切り替え通知 | 設定ファイルが切り替わったときに通知を表示するか | ON |
| メニュー表示中のキー入れ替え | メニューが開いているときもキー配列の入れ替えを行うか | OFF |

#### IME アクセス API について

- **IMM（Input Method Manager）**: 従来型の IME API。`ImmGetContext` / `ImmGetConversionStatus` 等を使用する。Windows のほぼ全バージョンで安定動作し、従来型 IME（旧 Microsoft IME、ATOK、Google 日本語入力等）との相性がよい。
- **TSF（Text Services Framework）**: Windows Vista 以降で推奨されている新しい IME フレームワーク。`ITfThreadMgr` / `ITfCompartmentMgr` 等の COM インターフェースを使用する。Windows 10 以降の新しい Microsoft IME は TSF ベースで動作しており、IMM 経由では状態を正しく取得できない場合がある。

やまぶきR と同様に、ユーザーが IMM / TSF を設定で選択できるようにする。IME によって適切な API が異なるため、両方をサポートすることで幅広い環境に対応する。

#### IMM と TSF の使い分け指針

| 環境 | 推奨 API |
|---|---|
| Windows 10/11 の新しい Microsoft IME | TSF |
| 旧 Microsoft IME（互換モード） | IMM |
| ATOK | IMM |
| Google 日本語入力 | IMM |
| その他 TSF ベースの IME | TSF |

---

## 5. 同時打鍵判定の詳細設計

### 5.1 ハイブリッド判定方式

同時打鍵の判定には「イベント駆動優先 + SetTimer 安全網」のハイブリッド方式を採用する。

通常のタイピングでは、次のキーイベントが判定閾値以内に到着するため、大半のケースはイベント駆動だけで判定が完了する。`SetTimer` は「しばらくキーを打たなかった場合に保留を掃除する」安全網としてのみ機能する。

```
通常のタイピング（99% のケース）:
  文字キー押下 -> engine が保留 + SetTimer 起動
  30ms 後に次のキー押下 -> engine が保留を遡及判定して確定
                          KillTimer（タイマーは発火せず）

最後の 1 文字（1% のケース）:
  文字キー押下 -> engine が保留 + SetTimer 起動
  65ms 経過、次のキーなし -> WM_TIMER 発火
                             engine.on_timeout() で単独確定
```

### 5.2 判定パターン

```
時間軸 ->

【パターン1: 親指先行 -> 即時確定】
  親指キー ========================
  文字キー         ========
  処理:    親指Down時   文字Down時:
           left_thumb    親指押下中なので
           = Some(t0)    即座にシフト面を出力
                         -> Emit（タイマー不要）

【パターン2: 文字先行 -> 時間内に親指 -> イベント駆動で確定】
  文字キー ================
  親指キー       ========================
  処理:    文字Down時:   親指Down時:
           pending       保留あり & 時間内
           = Some(vk,t0) -> 同時打鍵として確定
           SetTimer起動   KillTimer -> Emit

【パターン3: 文字単独 -> タイムアウトで確定】
  文字キー ============
  処理:    文字Down時:       WM_TIMER 到着:
           pending           on_timeout()
           = Some(vk,t0)     -> 単独打鍵として確定
           SetTimer起動       KillTimer -> Emit

【パターン4: 文字連打 -> 前の保留をイベント駆動で確定】
  文字キー1 ============
  文字キー2       ============
  処理:     文字1 Down:   文字2 Down:
            pending        保留あり & 親指なし
            = Some(vk1,t0) -> 文字1を単独確定
            SetTimer起動    -> 文字2を新たに保留
                            KillTimer -> SetTimer 再起動

【パターン5: 親指単独 -> 単独打鍵設定に従う】
  親指キー ============
  処理:    親指Down時:       WM_TIMER 到着:
           pending           on_timeout()
           = Some(thumb,t0)  -> 単独打鍵=有効: キー本来の機能を出力
           SetTimer起動       -> 単独打鍵=無効: 何も出力しない
                              KillTimer

【パターン6: 親指先行 + 連続シフト ON -> 複数文字連続入力】
  親指キー ========================================
  文字キー1     ========
  文字キー2              ========
  文字キー3                       ========
  処理:    連続シフト ON のため、親指キー押下中は
           すべての文字キーをシフト面で即時出力
           -> Emit, Emit, Emit（すべてタイマー不要）

【パターン7: 文字キー同時打鍵シフト】
  文字キーA ============
  文字キーB      ============
  処理:    文字A Down時:   文字B Down時:
           pending          保留あり & 時間内
           = Some(vkA,t0)   -> 文字キー同時打鍵テーブルを検索
           SetTimer起動      -> ヒット: 同時打鍵面の文字を出力
                              -> ミス: 文字Aを単独確定、文字Bを新たに保留
```

### 5.3 3 種類のタイマーと判定閾値

親指シフト・拡張親指シフト・文字キー同時打鍵シフトのそれぞれに独立した判定閾値を持つ。タイマー ID は判定種別ごとに分ける。

```rust
const TIMER_ID_THUMB: usize = 1;     // 親指シフト同時打鍵判定用
const TIMER_ID_EXT_THUMB: usize = 2; // 拡張親指シフト判定用
const TIMER_ID_CHAR_SIM: usize = 3;  // 文字キー同時打鍵判定用
```

### 5.4 engine の状態構造

```rust
struct Engine {
    layout: KeyLayout,
    config: EngineConfig,
    modifiers: ModifierState,
    left_thumb_down: Option<Instant>,
    right_thumb_down: Option<Instant>,
    ext_thumb1_down: Option<Instant>,
    ext_thumb2_down: Option<Instant>,
    pending: Option<PendingKey>,
    active_keys: HashMap<u16, u16>,
    enabled: bool,
}

struct EngineConfig {
    thumb: ThumbShiftConfig,
    ext_thumb: ExtThumbShiftConfig,
    char_simultaneous: CharSimultaneousConfig,
    behavior: BehaviorConfig,
}

struct ThumbShiftConfig {
    left_key: u16,
    right_key: u16,
    left_continuous_shift: bool,
    right_continuous_shift: bool,
    left_standalone: StandaloneMode,
    right_standalone: StandaloneMode,
    left_key_repeat: bool,
    right_key_repeat: bool,
    threshold_ms: u32,
}

struct ExtThumbShiftConfig {
    key1: Option<u16>,
    key2: Option<u16>,
    key1_continuous_shift: bool,
    key2_continuous_shift: bool,
    key1_standalone: StandaloneMode,
    key2_standalone: StandaloneMode,
    key1_key_repeat: bool,
    key2_key_repeat: bool,
}

struct CharSimultaneousConfig {
    enabled: bool,
    continuous_shift: bool,
    threshold_ms: u32,
}

struct BehaviorConfig {
    pause_hotkey: u16,
    ime_api: ImeApi,
    non_japanese_key_swap: bool,
    config_switch_notify: bool,
    menu_key_swap: bool,
}

enum StandaloneMode { Enabled, Disabled }
enum ImeApi { Imm, Tsf }

struct PendingKey {
    vk_code: u16,
    scan_code: u32,
    timestamp: Instant,
    kind: PendingKind,
}

enum PendingKind {
    CharKey,
    ThumbKey { is_left: bool },
    ExtThumbKey { key_index: u8 },
    CharSimultaneous,
}

struct ModifierState {
    shift: bool,
    ctrl: bool,
    alt: bool,
    left_thumb: bool,
    right_thumb: bool,
    ext_thumb1: bool,
    ext_thumb2: bool,
}
```

### 5.5 変換テーブルの構造

```rust
struct KeyLayout {
    name: String,
    normal: HashMap<u16, KeyAction>,
    shift: HashMap<u16, KeyAction>,
    left_thumb: HashMap<u16, KeyAction>,
    right_thumb: HashMap<u16, KeyAction>,
    ext_thumb1: HashMap<u16, KeyAction>,
    ext_thumb2: HashMap<u16, KeyAction>,
    /// key: (vk1, vk2) のソート済みタプル
    char_simultaneous: HashMap<(u16, u16), KeyAction>,
}

enum KeyAction {
    Key(u16),
    KeyUp(u16),
    String(String),
    Char(char),
    Suppress,
}
```

### 5.6 engine の主要メソッド

```rust
impl Engine {
    fn on_key_event(&mut self, event: RawKeyEvent) -> EngineOutput {
        if !self.enabled {
            return EngineOutput::PassThrough;
        }
        if !self.config.behavior.menu_key_swap && is_menu_active() {
            return EngineOutput::PassThrough;
        }
        if !self.config.behavior.non_japanese_key_swap
            && !is_japanese_input_language()
        {
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

    fn on_timeout(&mut self, timer_id: usize) -> Option<Vec<KeyAction>> {
        let pending = self.pending.take()?;
        match pending.kind {
            PendingKind::CharKey | PendingKind::CharSimultaneous => {
                let action = self.layout.normal.get(&pending.vk_code)?;
                Some(vec![action.clone()])
            }
            PendingKind::ThumbKey { is_left } => {
                let standalone = if is_left {
                    &self.config.thumb.left_standalone
                } else {
                    &self.config.thumb.right_standalone
                };
                match standalone {
                    StandaloneMode::Enabled => {
                        Some(vec![KeyAction::Key(pending.vk_code)])
                    }
                    StandaloneMode::Disabled => Some(vec![]),
                }
            }
            PendingKind::ExtThumbKey { key_index } => {
                let standalone = match key_index {
                    1 => &self.config.ext_thumb.key1_standalone,
                    _ => &self.config.ext_thumb.key2_standalone,
                };
                match standalone {
                    StandaloneMode::Enabled => {
                        Some(vec![KeyAction::Key(pending.vk_code)])
                    }
                    StandaloneMode::Disabled => Some(vec![]),
                }
            }
        }
    }

    fn toggle_enabled(&mut self) {
        self.enabled = !self.enabled;
        if !self.enabled {
            self.flush_all_pending();
        }
    }

    /// 保留キーに対応するタイマー ID と閾値を返す
    fn pending_timer_info(&self) -> (usize, u32) {
        match &self.pending {
            Some(p) => match p.kind {
                PendingKind::CharKey | PendingKind::ThumbKey { .. } => {
                    (TIMER_ID_THUMB, self.config.thumb.threshold_ms)
                }
                PendingKind::ExtThumbKey { .. } => {
                    (TIMER_ID_EXT_THUMB, self.config.thumb.threshold_ms)
                }
                PendingKind::CharSimultaneous => {
                    (TIMER_ID_CHAR_SIM,
                     self.config.char_simultaneous.threshold_ms)
                }
            },
            None => (TIMER_ID_THUMB, 65),
        }
    }
}
```

---

## 6. SetTimer の運用ルール

### 6.1 タイマーのライフサイクル

| タイミング | 操作 |
|---|---|
| `on_key_event` が `Pending` を返した | `SetTimer(HWND, timer_id, threshold_ms, None)` |
| `on_key_event` が `Emit` を返した | `KillTimer(HWND, timer_id)` |
| `WM_TIMER` を受信した | `KillTimer` -> `engine.on_timeout(timer_id)` -> `output.send_keys()` |

### 6.2 SetTimer の精度

`SetTimer` の精度は約 10-16ms（Windows のタイマー解像度に依存）。やまぶきR のデフォルト判定時間 65ms に対して十分な精度である。

### 6.3 メッセージループのコード

```rust
fn run_message_loop(engine: &mut Engine, output: &Output) {
    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) };
        if ret.0 <= 0 { break; }
        match msg.message {
            WM_TIMER => {
                let timer_id = msg.wParam.0;
                unsafe { KillTimer(HWND::default(), timer_id) };
                if let Some(actions) = engine.on_timeout(timer_id) {
                    output.send_keys(&actions);
                }
            }
            WM_HOTKEY => {
                engine.toggle_enabled();
            }
            _ => {
                unsafe { DispatchMessageW(&msg) };
            }
        }
    }
}
```

### 6.4 フックコールバックのコード

```rust
unsafe extern "system" fn hook_callback(
    ncode: i32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    if ncode >= 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

        // 自己注入チェック
        if kb.dwExtraInfo == INJECTED_MARKER {
            return CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam);
        }

        let event = RawKeyEvent::from_hook(wparam, kb);

        match ENGINE.on_key_event(event) {
            EngineOutput::Emit(actions) => {
                KillTimer(HWND::default(), TIMER_ID_THUMB);
                KillTimer(HWND::default(), TIMER_ID_EXT_THUMB);
                KillTimer(HWND::default(), TIMER_ID_CHAR_SIM);
                OUTPUT.send_keys(&actions);
                return LRESULT(1);
            }
            EngineOutput::Pending => {
                let (timer_id, threshold) = ENGINE.pending_timer_info();
                SetTimer(HWND::default(), timer_id, threshold, None);
                return LRESULT(1);
            }
            EngineOutput::PassThrough => {}
        }
    }
    CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam)
}
```

---

## 7. モジュール設計

### 7.1 モジュール一覧

| モジュール | ファイル | 責務 |
|---|---|---|
| `hook` | `hook.rs` | フック登録・解除、コールバック定義 |
| `engine` | `engine.rs` | 配列変換エンジン（状態機械 + 同時打鍵判定） |
| `output` | `output.rs` | SendInput によるキー注入、キー状態追跡 |
| `config` | `config.rs` | TOML 設定ファイルの読み込み・パース |
| `types` | `types.rs` | 共通の型定義 |
| `main` | `main.rs` | エントリポイント、メッセージループ、SetTimer 管理 |
| `ime` | `ime.rs` | IME 状態検知（IMM / TSF） |
| `tray`（Phase 7） | `tray.rs` | システムトレイアイコン |

### 7.2 output モジュール

#### 自己注入の識別

```rust
const INJECTED_MARKER: usize = 0x4B45_594D; // "KEYM"
```

#### キーリピートの処理

キーリピートの有効/無効は、フックコールバック内で連続した KeyDown イベント（オートリピート）を検出し、キーリピートが無効の場合は2回目以降の KeyDown を握りつぶすことで実現する。

### 7.3 ime モジュール

IME の状態取得を IMM と TSF の両方で実装し、設定に応じて切り替える。

#### アーキテクチャ

```rust
/// IME 状態取得のトレイト
trait ImeProvider {
    /// 現在の IME モードを取得する
    fn get_mode(&self) -> ImeMode;
    /// IME がオンかどうか
    fn is_enabled(&self) -> bool;
}

enum ImeMode {
    Off,
    Hiragana,
    Katakana,
    HalfKatakana,
    Alphanumeric,
}

/// 設定に応じて IMM or TSF の実装を返す
fn create_ime_provider(api: ImeApi) -> Box<dyn ImeProvider> {
    match api {
        ImeApi::Imm => Box::new(ImmProvider),
        ImeApi::Tsf => Box::new(TsfProvider::new()),
    }
}
```

#### IMM 実装

従来型の IME API。`ImmGetContext` / `ImmGetConversionStatus` を使用する。

```rust
struct ImmProvider;

impl ImeProvider for ImmProvider {
    fn get_mode(&self) -> ImeMode {
        unsafe {
            let hwnd = GetForegroundWindow();
            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                return ImeMode::Off;
            }
            let mut conversion = 0u32;
            let mut sentence = 0u32;
            ImmGetConversionStatus(himc, &mut conversion, &mut sentence);
            ImmReleaseContext(hwnd, himc);

            if (conversion & IME_CMODE_NATIVE) == 0 {
                return ImeMode::Alphanumeric;
            }
            if (conversion & IME_CMODE_KATAKANA) != 0 {
                if (conversion & IME_CMODE_FULLSHAPE) != 0 {
                    ImeMode::Katakana
                } else {
                    ImeMode::HalfKatakana
                }
            } else {
                ImeMode::Hiragana
            }
        }
    }

    fn is_enabled(&self) -> bool {
        !matches!(self.get_mode(), ImeMode::Off | ImeMode::Alphanumeric)
    }
}
```

#### TSF 実装

COM インターフェースを使用する。`ITfThreadMgr` でスレッドマネージャを取得し、`ITfCompartmentMgr` 経由で IME の状態を読み取る。

```rust
use windows::Win32::UI::TextServices::{
    ITfThreadMgr, ITfCompartmentMgr, ITfCompartment,
    CLSID_TF_ThreadMgr,
    GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};

struct TsfProvider {
    thread_mgr: ITfThreadMgr,
}

impl TsfProvider {
    fn new() -> Self {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
            let thread_mgr: ITfThreadMgr = CoCreateInstance(
                &CLSID_TF_ThreadMgr,
                None,
                CLSCTX_INPROC_SERVER,
            ).expect("TSF ThreadMgr の初期化に失敗");
            Self { thread_mgr }
        }
    }

    /// Compartment から値を読み取るヘルパー
    fn get_compartment_value(&self, guid: &windows::core::GUID) -> Option<u32> {
        unsafe {
            let mgr: ITfCompartmentMgr = self.thread_mgr.cast().ok()?;
            let compartment: ITfCompartment = mgr.GetCompartment(guid).ok()?;
            let value = compartment.GetValue().ok()?;
            // VARIANT から u32 を取り出す
            Some(value.Anonymous.Anonymous.Anonymous.ulVal)
        }
    }
}

impl ImeProvider for TsfProvider {
    fn get_mode(&self) -> ImeMode {
        // GUID_COMPARTMENT_KEYBOARD_OPENCLOSE: IME のオン/オフ
        let open = self.get_compartment_value(
            &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE
        ).unwrap_or(0);

        if open == 0 {
            return ImeMode::Off;
        }

        // GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION: 変換モード
        let conversion = self.get_compartment_value(
            &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION
        ).unwrap_or(0);

        // IME_CMODE_NATIVE 等のフラグで判定（IMM と同じビットフラグ体系）
        if (conversion & IME_CMODE_NATIVE) == 0 {
            return ImeMode::Alphanumeric;
        }
        if (conversion & IME_CMODE_KATAKANA) != 0 {
            if (conversion & IME_CMODE_FULLSHAPE) != 0 {
                ImeMode::Katakana
            } else {
                ImeMode::HalfKatakana
            }
        } else {
            ImeMode::Hiragana
        }
    }

    fn is_enabled(&self) -> bool {
        !matches!(self.get_mode(), ImeMode::Off | ImeMode::Alphanumeric)
    }
}
```

#### TSF 使用時の注意事項

- TSF は COM ベースであるため、使用するスレッドで `CoInitializeEx` を事前に呼ぶ必要がある。本ツールはシングルスレッドなので `COINIT_APARTMENTTHREADED` で初期化する。
- `ITfThreadMgr` のインスタンスはアプリケーション起動時に1回だけ生成し、以降は使い回す。毎回の `CoCreateInstance` は不要。
- TSF の Compartment 値の読み取りは軽量な COM メソッド呼び出しであり、フックコールバックのタイムアウト制約（300ms）に対して十分高速に完了する。

#### 共通ユーティリティ

```rust
fn is_japanese_input_language() -> bool {
    let layout = unsafe { GetKeyboardLayout(0) };
    (layout.0 as u32 & 0xFFFF) == 0x0411
}

fn is_menu_active() -> bool {
    // クラス名 "#32768" のウィンドウが存在するかを判定
    false // TODO
}
```

---

## 8. 設定ファイルの構造

```toml
# === 親指シフト設定 ===
[thumb_shift]
left_key = "VK_NONCONVERT"
right_key = "VK_CONVERT"
threshold_ms = 65

[thumb_shift.left]
continuous_shift = false
standalone = "enabled"
key_repeat = true

[thumb_shift.right]
continuous_shift = false
standalone = "enabled"
key_repeat = true

# === 拡張親指シフト設定 ===
[ext_thumb_shift]
key1 = "VK_NONCONVERT"
key2 = ""

[ext_thumb_shift.key1]
continuous_shift = true
standalone = "disabled"
key_repeat = false

[ext_thumb_shift.key2]
continuous_shift = true
standalone = "disabled"
key_repeat = false

# === 文字キー同時打鍵シフト設定 ===
[char_simultaneous]
enabled = true
continuous_shift = true
threshold_ms = 65

# === 動作モード設定 ===
[behavior]
pause_hotkey = "Pause"
ime_api = "IMM"
non_japanese_key_swap = true
config_switch_notify = true
menu_key_swap = false

# === 配列定義 ===
[layout]
name = "NICOLA"

[layout.normal]
VK_Q = "。"
VK_W = "か"
VK_E = "た"
# ... 以下同様

[layout.left_thumb]
VK_Q = "ぁ"
VK_W = "え"
# ... 以下同様

[layout.right_thumb]
VK_Q = "ぅ"
VK_W = "が"
# ... 以下同様

[layout.shift]
# ... 必要に応じて定義

[layout.ext_thumb1]
# ... 拡張親指シフト面1

[layout.ext_thumb2]
# ... 拡張親指シフト面2

[layout.char_simultaneous]
# "VK_F+VK_J" = "特殊文字"
```

---

## 9. 無限ループ防止の設計

### 9.1 対策（3 重の安全弁）

1. **dwExtraInfo マーカー**: 注入時に `INJECTED_MARKER` を設定し、フック冒頭でチェック。
2. **OS 予約キーの除外**: Ctrl+Alt+Delete, Win キー等は常に素通し。
3. **再入ガード**: `static` な再入フラグで多重呼び出しを検出。

---

## 10. 技術的な制約・リスク

### 10.1 フックのタイムアウト

Windows はフックコールバックが約 300ms 以内に戻らない場合、フックを自動解除する。本設計ではコールバック内の処理は数マイクロ秒で完了する。

### 10.2 SetTimer の精度

約 10-16ms。判定時間 65ms に対して十分。

### 10.3 セキュリティソフトとの競合

グローバルキーボードフックをブロックするセキュリティソフトがある。

### 10.4 管理者権限

UAC 環境下で管理者権限アプリに対してはフックが効かない場合がある。

### 10.5 ゲームとの互換性

DirectInput / Raw Input を直接使用するゲームではフックが効かないケースがある。

### 10.6 TSF の COM 初期化

TSF は COM ベースであるため、使用スレッドで `CoInitializeEx` を事前に呼ぶ必要がある。本ツールはシングルスレッド設計のため `COINIT_APARTMENTTHREADED` で初期化するが、他の COM コンポーネントとの共存時にスレッドモデルの競合が発生する可能性がある。また、TSF の Compartment GUID は IME の実装によって対応状況が異なるため、一部の IME では TSF 経由でも状態を正しく取得できない場合がある。その場合は IMM にフォールバックすることをユーザーに案内する。

---

## 11. 使用クレート

| クレート | バージョン | 用途 |
|---|---|---|
| `windows` | 0.58+ | Win32 API |
| `toml` | 0.8+ | 設定ファイルパース |
| `serde` / `serde_derive` | 1.x | デシリアライズ |
| `log` / `env_logger` | 0.4+ / 0.11+ | ログ出力 |
| `anyhow` | 1.x | エラーハンドリング |

```toml
[dependencies.windows]
version = "0.58"
features = [
    "Win32_Foundation",
    "Win32_UI_WindowsAndMessaging",
    "Win32_UI_Input_KeyboardAndMouse",
    "Win32_UI_Input_Ime",
    "Win32_UI_TextServices",
    "Win32_System_Com",
    "Win32_Globalization",
]
```

---

## 12. ディレクトリ構成

```
keyboard-hook/
+-- Cargo.toml
+-- config/
|   +-- nicola.toml
+-- src/
|   +-- main.rs
|   +-- hook.rs
|   +-- engine.rs
|   +-- output.rs
|   +-- config.rs
|   +-- types.rs
|   +-- ime.rs
+-- docs/
    +-- design.md
```

---

## 13. 実装フェーズ

### Phase 1: 最小キーフック（ログ出力のみ）

- `WH_KEYBOARD_LL` フックで全キーイベントを取得し、コンソールにログ出力する。
- `GetMessageW` によるメッセージループの基本形を確立する。

### Phase 2: 単純キー置換 + 無限ループ防止

- 特定キーを別キーに置き換える。
- `dwExtraInfo` による自己注入マーカーと無限ループ防止を確立する。
- `KeyStateTracker` による KeyUp 整合性を実装する。

### Phase 3: TOML 設定ファイル対応

- TOML から配列定義と設定を読み込む。
- 単純配列のエミュレートを確認する。

### Phase 4: 親指シフト対応

- `SetTimer` / `WM_TIMER` によるハイブリッド同時打鍵判定を実装する。
- 連続シフト・単独打鍵・キーリピートの各設定を実装する。
- 7 つの判定パターンの動作を検証する。

### Phase 5: 拡張親指シフト + 文字キー同時打鍵シフト

- 拡張親指シフトキー（最大2つ）の同時打鍵判定を追加する。
- 文字キー同士の同時打鍵シフトを実装する。

### Phase 6: 動作モード + IME 連携

- 一時停止用ホットキーを実装する。
- IME 状態検知を実装する（IMM / TSF 両対応、`ImeProvider` トレイトで切り替え）。
- 入力言語判定、メニュー表示中の判定を実装する。

### Phase 7: 常駐化・利便性向上

- システムトレイアイコン。
- 配列の動的切替。
- 設定ファイルの監視と自動リロード。
