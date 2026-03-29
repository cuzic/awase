# キーボード配列エミュレータ 設計書 v2

## 1. プロジェクト概要

### 1.1 目的

Windows 上で動作するキーボード配列エミュレータを Rust で開発する。
NICOLA（親指シフト）を含む任意のキー配列をエミュレートできる常駐型ツールを目指す。長期的には「やまぶき」「DvorakJ」と同等の互換性を目標とするが、初版では「壊れない最小構成」を優先する。

### 1.2 スコープ

**初版（MVP）の成功条件:**

- Windows 専用（Win32 API ベース）
- Win32 アプリケーション（メモ帳、ブラウザ等）で NICOLA 配列が安定動作する
- 親指シフトの同時打鍵判定が 5 パターンすべて正しく動作する
- 異常時は必ず PassThrough に倒れ、入力不能にならない
- TOML 設定ファイルによる配列定義

**段階的に拡張する項目:**

- UWP / WinUI アプリでの互換性検証・対応
- IME 状態検知（TSF + IMM32 ハイブリッド）に基づく出力方式の切り替え
- n-gram コーパスによる適応的閾値調整
- ゲームでの互換性（DirectInput / Raw Input 使用のものは対象外）

### 1.3 スコープ外（初版）

- GUI による設定画面（初期は設定ファイル直接編集）
- Microsoft Store 配布（Win32 常駐アプリとして配布）
- IME 自体の実装（既存 IME と連携する前提）

**macOS / Linux 対応**はスコープ外だが、プラットフォーム依存コードを抽象化し、将来のクロスプラットフォーム対応の基盤を初版から用意する（§6.9 参照）。

---

## 2. 設計方針：シングルスレッド・イベント駆動

### 2.1 基本原則

本ツールは **シングルスレッド** で動作し、すべての処理を Win32 メッセージループ上のイベント駆動で行う。マルチスレッド・async ランタイム（tokio 等）は使用しない。

この方針を採る理由は以下の通り。

- `WH_KEYBOARD_LL` のフックコールバックは、`GetMessageW` を呼んでいるスレッドで実行される。フックコールバックとタイマー処理が同じスレッドで動けば、排他制御（Mutex / Arc）が一切不要になる。
- フックコールバックには約 300ms のタイムアウト制約がある。ただしコールバック内の処理（HashMap 引き + `SendInput`）は数十マイクロ秒で完了するため、判定と出力をコールバック内で実行しても問題ない。コールバックは元キーを握りつぶすか素通しするかを戻り値で即座に決定する必要があるため、判定の後回しは原理的にできない。
- `SetTimer` は OS カーネルのタイマーを利用し、時間経過後に `WM_TIMER` メッセージをメッセージキューに投入するだけなので、CPU 消費・メモリ消費ともにほぼゼロである。
- n-gram テーブル引きは HashMap の 1 回引き（数百ナノ秒）であり、処理ループ内に組み込んでもレイテンシに影響しない。新しいイベントループは不要。

### 2.2 メッセージループの全体像

```
アプリケーション起動
    │
    ▼
SetWindowsHookExW(WH_KEYBOARD_LL) でフック登録
RegisterHotKey で有効/無効切替ホットキー登録
Shell_NotifyIconW でトレイアイコン追加
    │
    ▼
┌─── メッセージループ（GetMessageW） ◄──────────────────┐
│       │                                              │
│       ├─ キーイベント（フックコールバック経由）         │
│       │   └─ IME 状態チェック                         │
│       │       ├─ IME OFF/英数 → PassThrough           │
│       │       └─ かな入力 → engine.on_key_event()     │
│       │           ├─ 即時確定 → output.send_key()     │
│       │           └─ 保留発生 → SetTimer 起動          │
│       │                                              │
│       ├─ WM_TIMER（タイムアウト通知）                  │
│       │   └─ engine.on_timeout() を呼ぶ              │
│       │       └─ 保留キーを単独打鍵として確定          │
│       │          KillTimer() でタイマー停止            │
│       │                                              │
│       ├─ WM_HOTKEY（有効/無効切り替え）               │
│       │   └─ engine.toggle_enabled()                 │
│       │      tray.set_enabled()                       │
│       │                                              │
│       ├─ WM_APP（トレイアイコンイベント）              │
│       │   └─ 右クリック → コンテキストメニュー表示     │
│       │                                              │
│       ├─ WM_COMMAND（メニュー選択）                    │
│       │   ├─ 有効/無効切替                            │
│       │   ├─ 配列切替（layouts_dir 内の TOML）        │
│       │   └─ 終了                                    │
│       │                                              │
│       └─ WM_QUIT                                     │
│           └─ ループ脱出 → フック解除 → 終了           │
│                                                      │
└──────────────────────────────────────────────────────┘
```

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
│       │          │  │   ├ WM_HOTKEY            │  │  │
│       │          │  │   ├ WM_APP (トレイ)      │  │  │
│       │          │  │   └ WM_COMMAND (メニュー) │  │  │
│       │          │  └───────────┬──────────────┘  │  │
│       │          │              │                  │  │
│       │          │  ┌───────────▼──────────────┐  │  │
│       │          │  │ ime（IME 状態検知）       │  │  │
│       │          │  │   TSF + IMM32 ハイブリッド │  │  │
│       │          │  └───────────┬──────────────┘  │  │
│       │          │              │                  │  │
│       │          │  ┌───────────▼──────────────┐  │  │
│       │          │  │ engine（配列変換エンジン） │  │  │
│       │          │  │   状態機械 + 変換テーブル  │  │  │
│       │          │  │   + n-gram 適応閾値(Ph.2) │  │  │
│       │          │  └───────────┬──────────────┘  │  │
│       │          │              │                  │  │
│       │          │  ┌───────────▼──────────────┐  │  │
│       └──────────│──│ output（キー出力）        │  │  │
│                  │  │   SendInput + 状態追跡    │  │  │
│                  │  │   + サロゲートペア対応     │  │  │
│                  │  └──────────────────────────┘  │  │
│                  │              │                  │  │
│                  │  ┌───────────▼──────────────┐  │  │
│                  │  │ tray（システムトレイ）     │  │  │
│                  │  │   アイコン + メニュー      │  │  │
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
WH_KEYBOARD_LL フックコールバック呼び出し（hook.rs）
    │
    ├─ dwExtraInfo == INJECTED_MARKER ?
    │   └─ YES → CallNextHookEx で素通し（無限ループ防止）
    │
    ├─ 再入ガード発動？
    │   └─ YES → CallNextHookEx で素通し
    │
    ├─ コールバック関数（main.rs on_key_event_callback）を呼ぶ
    │   │
    │   ├─ scanCode → 物理位置 (行, 列) に変換
    │   │
    │   ├─ IME 状態チェック（ime.rs HybridProvider）
    │   │   ├─ IME OFF → PassThrough
    │   │   └─ IME ON → engine へ
    │   │
    │   └─ engine.on_event(event) を呼ぶ（engine.rs）
    │       │
    │       ├─ 修飾キー状態を更新（Ctrl/Alt/Shift）
    │       ├─ パススルー判定（修飾キー、Fキー、ナビゲーション等）
    │       ├─ Ctrl/Alt 押下中 → PassThrough（OS ショートカット保護）
    │       ├─ Shift 面の処理
    │       ├─ 親指シフト同時打鍵判定（保留・遡及判定・d1/d2 比較）
    │       │
    │       ├─ → Response { consumed: true, actions: [ローマ字列], timers: [Kill] }
    │       ├─ → Response { consumed: true, actions: [], timers: [Set(100ms)] }
    │       └─ → Response { consumed: false }
    │
    │   dispatch(response):
    │       ├─ タイマー命令実行（SetTimer / KillTimer）
    │       ├─ ローマ字列を VK コードに変換して SendInput
    │       └─ consumed に応じて LRESULT(1) or CallNextHookEx
    │
    └─ 戻り値
```

### 3.3 キー識別と出力方式

**入力側（キーの識別）:** スキャンコードで物理キー位置を識別する。OS のキーボードレイアウト設定（JIS/US 等）に非依存。

**出力側（案 D: ローマ字固定）:** .yab のローマ字列を常に VK コードとして SendInput する。IME はローマ字入力モードで使用する前提。入力モードの自動検知は行わない。

```
物理キー A + 左親指同時打鍵 → 「を」を入力したい

  → engine: 物理位置(2,0) + Face::LeftThumb → .yab から "wo" を取得
  → output: SendInput(VK_W), SendInput(VK_O)
  → IME: "wo" → 「を」
```

| IME 状態 | 出力方式 |
|---|---|
| IME OFF | 配列変換をバイパスし、元のキーを素通し |
| IME ON | .yab のローマ字列を VK コードとして SendInput → IME が変換 |

**将来のかな入力対応（案 C）:** .yab に `[かなシフト無し]` セクションを追加し、セクション名で出力方式を決定する拡張を予定。IME 入力モードの自動検知は行わない。

---

## 4. 同時打鍵判定の詳細設計

### 4.1 ハイブリッド判定方式

同時打鍵の判定には「イベント駆動優先 + SetTimer 安全網」のハイブリッド方式を採用する。

通常のタイピングでは、次のキーイベントが判定閾値以内に到着するため、大半のケースはイベント駆動だけで判定が完了する。`SetTimer` は「しばらくキーを打たなかった場合に保留を掃除する」安全網としてのみ機能する。

Phase 1 では固定閾値（100ms）を使用する。Phase 2 では n-gram コーパスによる適応的閾値調整を導入する（§6.3 参照）。

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

### 4.3 Engine の状態遷移

```rust
use std::collections::{HashMap, VecDeque};

/// プラットフォーム非依存のタイムスタンプ（マイクロ秒）
/// テスト時に任意の値を注入可能。Instant に依存しない。
pub type Timestamp = u64;

/// 配列変換エンジン（状態機械 + 同時打鍵判定）
struct Engine {
    /// 配列定義
    layout: KeyLayout,

    /// 左親指キーが押下中か（押下時刻を保持）
    left_thumb_down: Option<Timestamp>,

    /// 右親指キーが押下中か（押下時刻を保持）
    right_thumb_down: Option<Timestamp>,

    /// 同時打鍵判定用：未確定の保留キー
    pending: Option<PendingKey>,

    /// 同時打鍵の判定閾値（マイクロ秒）
    threshold_us: u64,

    /// 物理キー → 注入済みキーの対応（KeyUp 時の整合性維持用）
    active_keys: HashMap<u16, KeyAction>,

    /// エンジンの有効/無効
    enabled: bool,

    /// 修飾キー状態
    ctrl_down: bool,
    alt_down: bool,
    shift_down: bool,

    /// n-gram モデル（Phase 2、None なら固定閾値にフォールバック）
    ngram_model: Option<NgramModel>,

    /// 直近の出力文字履歴（n-gram 判定用、最大 3 文字）
    recent_output: VecDeque<char>,
}

struct PendingKey {
    scan_code: u32,
    timestamp: Timestamp,
    kind: PendingKind,
}

enum PendingKind {
    /// 文字キーが保留中（親指キーの到着を待っている）
    CharKey,
    /// 親指キーが保留中（文字キーの到着を待っている）
    ThumbKey { is_left: bool },
}
```

### 4.4 Engine の主要メソッド

```rust
impl Engine {
    /// キーイベント到着時に呼ばれる（フックコールバックから）
    pub fn on_key_event(&mut self, event: RawKeyEvent) -> EngineOutput {
        // 修飾キー状態を更新
        self.update_modifier_state(&event);

        if !self.enabled {
            return EngineOutput::PassThrough;
        }

        match event.event_type {
            KeyEventType::KeyDown | KeyEventType::SysKeyDown => self.on_key_down(&event),
            KeyEventType::KeyUp | KeyEventType::SysKeyUp => self.on_key_up(&event),
        }
    }

    /// タイムアウト時に呼ばれる（WM_TIMER ハンドラから）
    pub fn on_timeout(&mut self) -> Option<Vec<KeyAction>> { /* ... */ }

    /// 配列を動的に差し替える。保留中のキーがあればタイムアウトとして確定する。
    pub fn swap_layout(&mut self, layout: KeyLayout) -> Option<Vec<KeyAction>> {
        let timeout_actions = self.on_timeout();
        self.layout = layout;
        self.active_keys.clear();
        self.recent_output.clear();
        timeout_actions
    }

    /// 有効/無効を切り替える
    pub fn toggle_enabled(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.recent_output.clear();
        self.enabled
    }

    fn on_key_down(&mut self, event: &RawKeyEvent) -> EngineOutput {
        let is_thumb = self.is_thumb_key(event.vk_code);

        // 修飾キー等はパススルー
        if Self::is_passthrough_key(event.vk_code) {
            return EngineOutput::PassThrough;
        }

        // Ctrl/Alt 押下中はパススルー（OS ショートカット保護）
        if self.is_os_modifier_held() {
            return EngineOutput::PassThrough;
        }

        // Shift 面の処理
        if self.shift_down && !is_thumb {
            if let Some(&ch) = self.layout.shift.get(&event.vk_code) {
                return EngineOutput::Emit(vec![KeyAction::Char(ch)]);
            }
            return EngineOutput::PassThrough;
        }

        // 保留キーの遡及判定 → 同時打鍵 or 単独確定
        // ...（既存の保留・同時打鍵判定ロジック）
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

パターン4（文字連打）のように、前の保留をイベント駆動で確定した直後に新しい保留が発生する場合がある。この場合は `KillTimer` → `SetTimer` の順で再起動する。`SetTimer` を同じ ID で再度呼ぶと既存のタイマーがリセットされるため、明示的に `KillTimer` しなくても動作上は問題ないが、意図を明確にするために `KillTimer` を先に呼ぶ。

### 5.4 SetTimer の精度について

`SetTimer` の精度は約 10〜16ms（Windows のタイマー解像度に依存）。同時打鍵の判定閾値が 100ms であれば、最大 16ms の誤差は許容範囲内である。

---

## 6. 時間依存ステートマシンフレームワーク

### 6.1 現在の設計の問題

現在の `Engine` は以下が密結合している:

- **状態管理**: `pending`, `left_thumb_down`, 修飾キーフラグ
- **遷移判定**: 時間差比較、配列面の選択
- **タイマー制御**: 呼び出し側（main.rs）が `SetTimer`/`KillTimer` を `EngineOutput` に応じて手動管理
- **出力アクション生成**: `KeyAction` の構築と `active_keys` の管理

特にタイマー制御が main.rs に漏れ出している点が問題で、「Emit が返ったら KillTimer → has_pending なら SetTimer」というロジックがフレームワーク化されていない。

### 6.2 設計目標

1. **純粋な状態遷移ロジック**をプラットフォーム非依存に保つ（テスト容易性）
2. **タイマー管理を宣言的**にする（遷移結果にタイマー命令を含める）
3. **副作用の実行をランタイム層に分離**する（Win32 固有処理の隔離）
4. **汎用ライブラリとして抽出可能**な設計にする

### 6.3 フレームワーク層の設計

#### コアトレイト

```rust
use std::time::Duration;

/// 時間依存ステートマシンのコアトレイト
///
/// 一般的な FSM と異なり、遷移結果にタイマー命令を含む。
/// これにより「一定時間内に次のイベントが来なければ別の遷移をする」
/// というパターンを宣言的に表現できる。
pub trait TimedStateMachine {
    /// ステートマシンに入力されるイベントの型
    type Event;

    /// 遷移時に出力されるアクションの型
    type Action;

    /// タイマーの識別子の型
    type TimerId: Copy + Eq;

    /// イベントを処理し、遷移結果を返す
    fn on_event(&mut self, event: Self::Event) -> Response<Self::Action, Self::TimerId>;

    /// タイマーのタイムアウトを処理する
    fn on_timeout(&mut self, timer_id: Self::TimerId) -> Response<Self::Action, Self::TimerId>;
}
```

#### 遷移結果（Response）

```rust
/// ステートマシンの遷移結果
///
/// ランタイムはこの値を解釈して副作用（タイマー操作、アクション実行）を行う。
/// ステートマシン自身は副作用を持たない。
#[derive(Debug)]
pub struct Response<A, T> {
    /// 元のイベントを消費したか（true: 握りつぶす、false: 素通し）
    pub consumed: bool,

    /// 出力アクション列（順序保証）
    pub actions: Vec<A>,

    /// タイマー操作命令列
    pub timers: Vec<TimerCommand<T>>,
}

#[derive(Debug, Clone, Copy)]
pub enum TimerCommand<T> {
    /// 指定 ID でタイマーを起動（既存の同一 ID は上書き）
    Set { id: T, duration: Duration },
    /// 指定 ID のタイマーを停止
    Kill { id: T },
}
```

#### Response ビルダー

```rust
impl<A, T> Response<A, T> {
    /// イベントを消費し、アクションを出力する
    pub fn emit(actions: Vec<A>) -> Self {
        Self { consumed: true, actions, timers: vec![] }
    }

    /// イベントを消費するが、アクションはまだ出さない（保留）
    pub fn consume() -> Self {
        Self { consumed: true, actions: vec![], timers: vec![] }
    }

    /// イベントを素通しする
    pub fn pass_through() -> Self {
        Self { consumed: false, actions: vec![], timers: vec![] }
    }

    /// タイマー起動命令を追加
    pub fn with_timer(mut self, id: T, duration: Duration) -> Self {
        self.timers.push(TimerCommand::Set { id, duration });
        self
    }

    /// タイマー停止命令を追加
    pub fn with_kill_timer(mut self, id: T) -> Self {
        self.timers.push(TimerCommand::Kill { id });
        self
    }
}
```

### 6.4 ランタイム層の設計

ランタイムはプラットフォーム固有の副作用を実行する薄いレイヤー。

```rust
/// ランタイムが実装するタイマー操作インターフェース
pub trait TimerRuntime {
    type TimerId;
    fn set_timer(&mut self, id: Self::TimerId, duration: Duration);
    fn kill_timer(&mut self, id: Self::TimerId);
}

/// ランタイムが実装するアクション実行インターフェース
pub trait ActionExecutor {
    type Action;
    fn execute(&self, actions: &[Self::Action]);
}

/// Response を解釈して副作用を実行する汎用ディスパッチャ
pub fn dispatch<A, T: Copy + Eq>(
    response: &Response<A, T>,
    timers: &mut impl TimerRuntime<TimerId = T>,
    executor: &impl ActionExecutor<Action = A>,
) {
    for cmd in &response.timers {
        match *cmd {
            TimerCommand::Set { id, duration } => timers.set_timer(id, duration),
            TimerCommand::Kill { id } => timers.kill_timer(id),
        }
    }
    if !response.actions.is_empty() {
        executor.execute(&response.actions);
    }
}
```

Win32 実装:

```rust
/// Win32 メッセージループ上のタイマーランタイム
struct Win32TimerRuntime;

impl TimerRuntime for Win32TimerRuntime {
    type TimerId = usize;
    fn set_timer(&mut self, id: usize, duration: Duration) {
        unsafe { SetTimer(HWND::default(), id, duration.as_millis() as u32, None); }
    }
    fn kill_timer(&mut self, id: usize) {
        unsafe { KillTimer(HWND::default(), id); }
    }
}

/// SendInput ベースのアクション実行
struct SendInputExecutor;

impl ActionExecutor for SendInputExecutor {
    type Action = KeyAction;
    fn execute(&self, actions: &[KeyAction]) {
        output.send_keys(actions);
    }
}
```

### 6.5 NicolaEngine の設計

#### レイヤー分離できない理由

n-gram 適応閾値では、同時打鍵の閾値を計算するために候補文字が必要になる:

```
文字キー A 保留中 → 親指キー到着（d1 = 60ms）

候補文字 = layout.left_thumb[VK_A] = 'を'
recent_output = ['し', 'て']
bigram('て', 'を') = 高頻度 → 閾値を緩めて 110ms
d1 < 110ms ? → YES → 同時打鍵確定
```

「どの面か」を決めるために「その面だったら何の文字か」を配列テーブルから引く必要がある。したがって**タイミング判定と文字決定は単一の timed-fsm 内で行う**。中間層（ResolvedKey 等）への分離は行わない。

ただし**内部のメソッドレベルでは関心を分離**する:

```rust
impl NicolaEngine {
    /// タイミング判定: d1/d2 比較、n-gram 閾値調整
    fn resolve_timing(&self, pending_vk: u16, event: &RawKeyEvent) -> TimingResult {
        let candidate = self.lookup_candidate(pending_vk, event);
        let threshold = self.adjusted_threshold(candidate);
        let elapsed = event.timestamp.duration_since(pending.timestamp);
        if elapsed < threshold { TimingResult::Simultaneous }
        else { TimingResult::Standalone }
    }

    /// 文字決定: 面 + 物理位置 → .yab からローマ字列取得 → 出力形式変換
    fn produce_action(&self, pos: PhysicalPos, face: Face) -> Vec<KeyAction> {
        let romaji = self.layout.get(face, pos);  // .yab の値（例: "wo"）
        self.format_output(romaji)
    }

    /// IME 入力方式に応じた出力形式変換
    fn format_output(&self, romaji: &str) -> Vec<KeyAction> {
        match self.input_method {
            InputMethod::Romaji => {
                // ローマ字入力: VK コードとして送信 → IME が変換
                romaji.chars()
                    .map(|ch| KeyAction::VkCode(ascii_to_vk(ch)))
                    .collect()
            }
            InputMethod::Kana => {
                // かな入力: ローマ字→かな逆引き → Unicode 直接送信
                self.romaji_to_kana.get(romaji)
                    .map(|&ch| vec![KeyAction::Char(ch)])
                    .unwrap_or_default()
            }
        }
    }
}
```

#### 状態の明示化

```rust
/// NICOLA エンジンのメイン状態
#[derive(Debug)]
enum NicolaState {
    /// 待機中（保留なし）
    Idle,
    /// 文字キーが保留中（親指キーの到着を待っている）
    PendingChar { scan_code: u32, timestamp: Timestamp },
    /// 親指キーが保留中（文字キーの到着を待っている）
    PendingThumb { scan_code: u32, is_left: bool, timestamp: Timestamp },
}

/// 修飾キーの追跡状態（メイン状態と直交）
#[derive(Debug, Default)]
struct ModifierState {
    ctrl: bool,
    alt: bool,
    shift: bool,
    left_thumb: Option<Timestamp>,
    right_thumb: Option<Timestamp>,
}
```

#### Engine 構造体

```rust
struct NicolaEngine {
    layout: KeyLayout,
    state: NicolaState,
    modifiers: ModifierState,
    active_keys: HashMap<u16, KeyAction>,
    threshold: Duration,
    enabled: bool,
    /// 現在の IME 入力方式（キーイベントごとに IME から取得して更新）
    input_method: InputMethod,
    /// ローマ字→かな逆引きテーブル（かな入力モード時に使用）
    romaji_to_kana: HashMap<String, char>,
    // Phase 2
    ngram_model: Option<NgramModel>,
    recent_output: VecDeque<char>,
}
```

#### フレームワーク上の実装

```rust
impl TimedStateMachine for NicolaEngine {
    type Event = RawKeyEvent;
    type Action = KeyAction;
    type TimerId = usize;

    fn on_event(&mut self, event: RawKeyEvent) -> Response<KeyAction, usize> {
        self.modifiers.update(&event);
        if !self.enabled { return Response::pass_through(); }

        match event.event_type {
            KeyEventType::KeyDown | KeyEventType::SysKeyDown => self.on_key_down(&event),
            KeyEventType::KeyUp | KeyEventType::SysKeyUp => self.on_key_up(&event),
        }
    }

    fn on_timeout(&mut self, _: usize) -> Response<KeyAction, usize> {
        let prev = std::mem::replace(&mut self.state, NicolaState::Idle);
        match prev {
            NicolaState::PendingChar { vk_code, .. } => {
                let action = self.produce_action(vk_code, Face::Normal);
                Response::emit(vec![action]).with_kill_timer(TIMER_PENDING)
            }
            NicolaState::PendingThumb { vk_code, .. } => {
                Response::emit(vec![KeyAction::Key(vk_code)]).with_kill_timer(TIMER_PENDING)
            }
            NicolaState::Idle => Response::pass_through(),
        }
    }
}
```

### 6.6 main.rs の簡素化

```rust
/// フックコールバック — Response を受け取って副作用を実行するだけ
unsafe fn on_key_event_callback(event: RawKeyEvent) -> CallbackResult {
    let Some(engine) = ENGINE.get_mut() else {
        return CallbackResult::PassThrough;
    };

    // IME ガード（フレームワーク外の前処理）
    if !check_ime_active() {
        return CallbackResult::PassThrough;
    }

    let response = engine.on_event(event);
    dispatch(&response, &mut WIN32_TIMERS, &SEND_INPUT_EXECUTOR);

    if response.consumed { CallbackResult::Consumed }
    else { CallbackResult::PassThrough }
}
```

`KillTimer` / `SetTimer` / `has_pending()` のロジックが完全に消え、`dispatch` に集約される。

### 6.7 テスト設計

テスト容易性は本設計の主要な設計目標の一つ。以下の 3 つの仕組みで実現する。

#### 6.7.1 タイムスタンプの注入可能性

`Instant::now()` に依存するとテストが非決定的になる。`RawKeyEvent` のタイムスタンプを `u64`（マイクロ秒）にすることで、テストから任意の時刻を注入できる:

```rust
/// プラットフォーム非依存のタイムスタンプ（マイクロ秒）
pub type Timestamp = u64;

pub struct RawKeyEvent {
    pub scan_code: u32,
    pub event_type: KeyEventType,
    pub timestamp: Timestamp,  // Instant ではなく u64
}
```

実行時は `QueryPerformanceCounter` 等から `Timestamp` を生成するが、テストでは自由な値を渡せる:

```rust
let t0: Timestamp = 1_000_000; // 任意の起点
let event1 = RawKeyEvent { scan_code: SC_A, timestamp: t0, .. };
let event2 = RawKeyEvent { scan_code: SC_THUMB, timestamp: t0 + 30_000, .. }; // 30ms 後
```

#### 6.7.2 Response ベースのアサーション

timed-fsm の Response にタイマー命令が含まれるため、**副作用なしで全ての振る舞いを検証できる**:

```rust
#[test]
fn test_simultaneous_keystroke() {
    let mut engine = make_engine();
    let t0 = 0u64;

    // 文字キー押下 → 保留 + タイマー起動
    let r = engine.on_event(key_down(SC_A, t0));
    assert!(r.consumed);
    assert!(r.actions.is_empty());
    assert_timer_set(&r, TIMER_PENDING);

    // 30ms 後に親指キー → 同時打鍵確定 + タイマー停止
    let r = engine.on_event(key_down(SC_THUMB_L, t0 + 30_000));
    assert!(r.consumed);
    assert_eq!(r.actions, vec![KeyAction::String("wo".into())]);
    assert_timer_killed(&r, TIMER_PENDING);
}

// ヘルパー関数
fn assert_timer_set(r: &Response<..>, id: usize) {
    assert!(r.timers.iter().any(|t| matches!(t, TimerCommand::Set { id: i, .. } if *i == id)));
}
```

検証可能な項目:
- 出力アクション（文字、ローマ字列、特殊キー）
- タイマー命令（起動/停止/再起動）
- consumed フラグ（キーの握りつぶし/素通し）
- 状態遷移の正しさ（連続イベントの結果を追跡）

#### 6.7.3 プラットフォームトレイトのモック

§6.9 のプラットフォームトレイトにより、統合テストでもプラットフォーム API を呼ばずにテスト可能:

```rust
/// テスト用のモックランタイム
struct MockRuntime {
    timer_log: Vec<TimerCommand<usize>>,
    sent_actions: Vec<KeyAction>,
    ime_state: ImeState,
}

impl TimerRuntime for MockRuntime {
    type TimerId = usize;
    fn set_timer(&mut self, id: usize, duration: Duration) {
        self.timer_log.push(TimerCommand::Set { id, duration });
    }
    fn kill_timer(&mut self, id: usize) {
        self.timer_log.push(TimerCommand::Kill { id });
    }
}

impl KeySender for MockRuntime {
    fn send_vk_sequence(&self, vk_codes: &[u16]) { /* 記録 */ }
    fn send_unicode(&self, ch: char) { /* 記録 */ }
}

impl ImeDetector for MockRuntime {
    fn get_state(&self) -> ImeState { self.ime_state }
}
```

#### 6.7.4 テストレベルと対象

| レベル | 対象 | 手法 | プラットフォーム依存 |
|---|---|---|---|
| **ユニット** | NicolaEngine の状態遷移 | Response 検査 | なし |
| **ユニット** | .yab パーサー | 文字列入力 → 構造体検査 | なし |
| **ユニット** | scanmap 変換 | スキャンコード → 物理位置の全キー検証 | なし |
| **ユニット** | ローマ字→かな逆引き | テーブル全エントリの往復検証 | なし |
| **プロパティ** | Engine の不変条件 | proptest でランダムイベント列を投入 | なし |
| **統合** | Engine + MockRuntime | シナリオテスト（実際の文章入力を模擬） | なし |
| **E2E** | 実 OS 上での動作 | 手動 or AutoHotKey スクリプト | Windows のみ |

#### 6.7.5 proptest による不変条件の検証

```rust
proptest! {
    #[test]
    fn engine_never_panics(events in vec(arb_key_event(), 0..200)) {
        let mut engine = make_engine();
        for event in events {
            let _ = engine.on_event(event);
        }
    }

    #[test]
    fn timers_are_balanced(events in vec(arb_key_event(), 0..100)) {
        let mut engine = make_engine();
        let mut active_timers: HashSet<usize> = HashSet::new();
        for event in events {
            let r = engine.on_event(event);
            for cmd in &r.timers {
                match cmd {
                    TimerCommand::Set { id, .. } => { active_timers.insert(*id); }
                    TimerCommand::Kill { id } => { active_timers.remove(id); }
                }
            }
        }
        // Idle 状態ならタイマーはすべて停止しているはず
    }

    #[test]
    fn consumed_implies_action_or_pending(event in arb_key_event()) {
        let mut engine = make_engine();
        let r = engine.on_event(event);
        if r.consumed {
            // consumed なら、アクションが出力されたか、タイマーが起動されたはず
            assert!(!r.actions.is_empty() || r.timers.iter().any(|t| matches!(t, TimerCommand::Set { .. })));
        }
    }
}
```

#### 6.7.6 シナリオテスト

実際の文章入力を模擬し、期待されるローマ字列が出力されることを検証:

```rust
#[test]
fn test_typing_scenario_watashi() {
    let mut engine = make_nicola_engine();
    let mut output = Vec::new();
    let mut t = 0u64;

    // 「わたし」を入力: W(通常面) → E(通常面) → S(通常面)
    for &(scan, delay_ms) in &[(SC_W, 0), (SC_E, 80), (SC_S, 160)] {
        t += delay_ms * 1000;
        let r = engine.on_event(key_down(scan, t));
        output.extend(r.actions.iter().cloned());
        // 前のキーの保留確定分も回収
    }
    // タイムアウトで最後のキーを確定
    let r = engine.on_timeout(TIMER_PENDING);
    output.extend(r.actions.iter().cloned());

    assert_eq!(
        output,
        vec![
            KeyAction::String("wa".into()),   // わ
            KeyAction::String("ta".into()),   // た
            KeyAction::String("si".into()),   // し
        ]
    );
}
```

### 6.8 クレート分離

外部クレートとして切り出すのは `timed-fsm` のみ。他のモジュール（yab パーサー、romaji-kana テーブル、scanmap 等）はアプリ内部のモジュールとして管理する。

```
keyboard-hook/
├── crates/
│   └── timed-fsm/              # 汎用クレート（crates.io 公開可能）
│       ├── src/lib.rs           # TimedStateMachine, Response, TimerCommand
│       └── Cargo.toml           # 依存: なし（no_std 対応可能）
├── src/
│   ├── lib.rs                   # NicolaEngine + config + scanmap + romaji_kana
│   └── main.rs                  # Win32 固有 (hook, output, ime, tray)
```

**`timed-fsm` だけをクレート化する理由:**
- 依存ゼロ、`no_std` 対応可能な完全に汎用的なフレームワーク
- crates.io に時間依存 FSM の既存クレートがない
- キーボード入力以外（MIDI、ゲーム入力、プロトコル、デバウンス）にも適用可能

**他をクレート化しない理由:**
- `romaji-kana`: HashMap 1 つ分のデータ。クレートにするほどの量がない
- `yab-parser`: やまぶきユーザー限定のニッチな形式
- `scanmap`: 定数テーブル数十行
- `nicola-core`: 再利用者が自プロジェクトのみ

### 6.9 プラットフォーム抽象化

初版は Windows 専用だが、将来の macOS / Linux 対応に備えてプラットフォーム依存コードをトレイトで抽象化する。

#### プラットフォーム依存の境界

| 機能 | Windows | macOS | Linux |
|---|---|---|---|
| キーフック | `WH_KEYBOARD_LL` | `CGEventTap` | `libinput` / `evdev` |
| キー注入 | `SendInput` | `CGEventPost` | `uinput` |
| タイマー | `SetTimer` / `WM_TIMER` | `CFRunLoopTimer` | `timerfd` / `epoll` |
| IME 状態取得 | TSF / IMM32 | `TISGetInputSourceProperty` | `IBus` / `Fcitx` D-Bus |
| トレイアイコン | `Shell_NotifyIconW` | `NSStatusItem` | `libappindicator` |
| イベントループ | `GetMessageW` | `CFRunLoop` | `epoll_wait` |

#### トレイト定義

```rust
/// キーボードフックの抽象化
pub trait KeyboardHook {
    fn install(&mut self, callback: Box<dyn FnMut(RawKeyEvent) -> bool>) -> Result<()>;
    fn uninstall(&mut self);
}

/// キー注入の抽象化
pub trait KeySender {
    fn send_vk_sequence(&self, vk_codes: &[u16]);
    fn send_unicode(&self, ch: char);
    fn send_key(&self, vk: u16, is_keyup: bool);
}

/// IME 状態取得の抽象化
pub trait ImeDetector {
    fn get_state(&self) -> ImeState;
}

/// TimerRuntime と ActionExecutor は §6.4 で既に定義済み
```

#### プラットフォーム別バイナリ構成

```
keyboard-hook/
├── crates/
│   └── timed-fsm/           # 汎用 FSM（プラットフォーム非依存）
├── src/
│   ├── lib.rs               # NicolaEngine, config, scanmap, types（プラットフォーム非依存）
│   ├── main.rs              # Windows バイナリ
│   ├── hook.rs              # impl KeyboardHook for Win32Hook
│   ├── output.rs            # impl KeySender for Win32Sender
│   ├── ime.rs               # impl ImeDetector for Win32Ime
│   └── tray.rs
├── src-macos/               # macOS バイナリ（将来）
│   └── ...
└── src-linux/               # Linux バイナリ（将来）
    └── ...
```

**設計方針:**
- `lib.rs` 以下（engine, config, scanmap, types）はプラットフォーム非依存。`timed-fsm` と `std` のみに依存
- NicolaEngine はスキャンコード（物理位置）で入力を受け、ローマ字列/かな文字を出力する。プラットフォーム API を一切呼ばない
- プラットフォーム固有のバイナリが `KeyboardHook` / `KeySender` / `ImeDetector` を実装し、Engine に注入する
- スキャンコード→物理位置のマッピングは OS 間で共通（USB HID 標準）

---

## 7. モジュール設計

### 7.1 モジュール一覧

| モジュール | ファイル | クレート | 責務 |
|---|---|---|---|
| `config` | `config.rs` | lib | やまぶき互換設定ファイル (.yab) のパース、スキャンコード↔物理位置変換、ホットキーパース |
| `engine` | `engine.rs` | lib | 配列変換エンジン（timed-fsm 実装、同時打鍵判定 + 修飾キー追跡 + n-gram） |
| `types` | `types.rs` | lib | 共通の型定義（`RawKeyEvent`, `KeyAction`, `Response`） |
| `hook` | `hook.rs` | bin | フック登録・解除、コールバック定義、再入ガード |
| `output` | `output.rs` | bin | `SendInput` によるキー注入（ローマ字列→VK コード送信、特殊キー対応） |
| `ime` | `ime.rs` | bin | IME 状態検知（TSF 優先 + IMM32 フォールバック） |
| `tray` | `tray.rs` | bin | システムトレイアイコン、コンテキストメニュー |
| `main` | `main.rs` | bin | エントリポイント、メッセージループ、グローバル状態管理 |
| `scanmap` | `scanmap.rs` | lib | スキャンコード → 物理位置 (行, 列) のマッピングテーブル |
| `ngram`（Phase 2） | `ngram.rs` | lib | n-gram コーパスの読み込みと閾値計算 |

### 7.2 output モジュール

#### 自己注入の識別

`dwExtraInfo` にマジックナンバー `0x4B45_594D`（"KEYM"）を設定して注入し、フック側でチェックする。

#### ローマ字列の VK コード送信

エンジンが決定したローマ字列（例: `"ka"`）を、各文字に対応する VK コードとして `SendInput` する。IME がこれを受けてかなに変換する。

```rust
fn send_romaji(&self, romaji: &str) {
    for ch in romaji.chars() {
        let vk = ascii_to_vk(ch);  // 'k' → VK_K, 'a' → VK_A
        self.send_key(vk, false);   // KeyDown
        self.send_key(vk, true);    // KeyUp
    }
}
```

#### 特殊値の送信

設定ファイルの特殊キーワード（`後`, `逃` 等）はそれぞれ対応する VK コードを送信する:

| 設定値 | 送信する VK コード |
|---|---|
| `後` | `VK_BACK` |
| `逃` | `VK_ESCAPE` |
| `入` | `VK_RETURN` |
| `空` | `VK_SPACE` |
| `消` | `VK_DELETE` |

#### リテラル文字の送信

シングルクォートで囲まれた値（例: `'．'`）は `KEYEVENTF_UNICODE` で直接送信する（IME をバイパス）。

### 7.3 n-gram コーパスによる適応的閾値調整（Phase 2）

#### 基本アイデア

固定閾値（例: 100ms）ではなく、「今入力しようとしている文字の並びがどれくらいありそうか」で閾値を伸縮させる。

```
固定閾値: 100ms で一律判定

適応的閾値:
  直前が「わ」→ 候補「が」(2-gram "わが" は高頻度) → 閾値を緩める (110ms)
  直前が「を」→ 候補「ぱ」(2-gram "をぱ" は極低頻度) → 閾値を締める (80ms)
```

n-gram による判定は Engine の `is_within_threshold()` 内に組み込む。理由:

- **同期的に決定が必要**: 保留キーに親指キーが来た瞬間に即座に判定しなければならない
- **計算コストが極めて小さい**: HashMap の 1 回引き（数百ナノ秒）
- **データは読み取り専用**: コーパスの頻度テーブルは起動時にロードして以降は不変
- **Engine の状態に依存**: `recent_output` を参照するため密結合

新しいイベントループやスレッドは不要。

#### Engine への追加構造

```rust
struct NgramModel {
    /// 2-gram 頻度: key=(前の文字, 現在の文字), value=対数確率
    bigram: HashMap<(char, char), f32>,

    /// 3-gram 頻度: key=(2つ前, 1つ前, 現在), value=対数確率
    trigram: HashMap<(char, char, char), f32>,

    /// 基準閾値（設定ファイルの threshold_ms）
    base_threshold_ms: u32,

    /// 閾値の調整幅（例: ±20ms）
    adjustment_range_ms: u32,

    /// 2-gram ごとの典型的な打鍵間隔（Phase 2 後期）
    bigram_typical_interval: HashMap<(char, char), TypicalInterval>,
}

struct TypicalInterval {
    mean_ms: f32,
    stddev_ms: f32,
}
```

#### 判定ロジック

```rust
impl Engine {
    /// n-gram モデルに基づいて閾値を動的に調整する
    fn adjusted_threshold(&self, candidate_char: char) -> Duration {
        let model = match &self.ngram_model {
            Some(m) => m,
            None => return self.threshold, // フォールバック: 固定閾値
        };

        let base = model.base_threshold_ms as f32;
        let range = model.adjustment_range_ms as f32;

        // 直前の文字との n-gram スコアを取得
        let score = match self.recent_output.back() {
            Some(&prev) => {
                // 3-gram が使えるなら優先
                let tri_score = self.recent_output.iter().rev()
                    .nth(1)
                    .and_then(|&pp| {
                        model.trigram.get(&(pp, prev, candidate_char))
                    });
                tri_score
                    .or_else(|| model.bigram.get(&(prev, candidate_char)))
                    .copied()
                    .unwrap_or(0.0) // コーパスに無い組み合わせは中立
            }
            None => 0.0, // 履歴なし → 中立
        };

        // score: 正=高頻度（閾値を緩める）、負=低頻度（閾値を締める）
        // tanh で [-1, 1] にマップし、adjustment_range を乗じる
        let adjustment = score.tanh() * range;
        let adjusted = (base + adjustment).clamp(30.0, 120.0);
        Duration::from_millis(adjusted as u64)
    }

    /// 同時打鍵の判定（Phase 2 で拡張）
    fn is_within_threshold(
        &self,
        elapsed: Duration,
        candidate_char: Option<char>,
    ) -> bool {
        let threshold = match candidate_char {
            Some(ch) => self.adjusted_threshold(ch),
            None => self.threshold,
        };
        elapsed < threshold
    }
}
```

#### タイミング情報の活用（Phase 2 後期）

打鍵間隔パターンも n-gram と組み合わせると精度が上がる。

```rust
impl NgramModel {
    /// 観測された打鍵間隔が、この n-gram の典型的な間隔に
    /// どれだけ合致するかのスコアを返す
    fn timing_score(
        &self, prev: char, current: char, observed_ms: f32,
    ) -> f32 {
        match self.bigram_typical_interval.get(&(prev, current)) {
            Some(typical) => {
                let z = (observed_ms - typical.mean_ms) / typical.stddev_ms;
                -z.abs() // 典型的な間隔に近いほど高スコア
            }
            None => 0.0,
        }
    }
}
```

#### `recent_output` の管理ルール

```rust
// Engine が文字を出力確定するたびに更新
fn emit_char(&mut self, ch: char) {
    self.recent_output.push_back(ch);
    if self.recent_output.len() > 3 {
        self.recent_output.pop_front();
    }
}

// 状態リセット時にクリア
fn toggle_enabled(&mut self) -> bool {
    self.recent_output.clear();
    // ...
}

fn swap_layout(&mut self, layout: KeyLayout) -> Option<Vec<KeyAction>> {
    self.recent_output.clear();
    // ...
}
```

#### 処理フローへの影響

既存の処理フローの変更箇所は閾値比較 1 箇所のみ。

```
PendingChar + 親指キー Down 到着
    |
    +- 候補文字を特定（keymap から引く）
    +- adjusted_threshold(候補文字) で閾値を計算  ← Phase 2 で追加
    +- 経過時間 ≤ 調整済み閾値 ?
        +- YES → 同時打鍵確定、recent_output に追加
        +- NO  → 単独打鍵確定
```

処理ループの構造やチャネル構成には一切変更がない。

#### コーパスデータの形式

```toml
# data/ngram_hiragana.toml（アプリに同梱）
[bigram]
"あい" = 2.3    # 対数頻度スコア
"かく" = 1.8
"をぱ" = -1.5   # 低頻度

[trigram]
"ありが" = 3.1
"ですか" = 2.8

[timing]
# Phase 2 後期（オプション）
"あい" = { mean_ms = 120.0, stddev_ms = 30.0 }
"かく" = { mean_ms = 95.0, stddev_ms = 25.0 }
```

---

## 8. IME 連携

### 8.1 方針

TSF（Text Services Framework）を優先し、IMM32（Input Method Manager）にフォールバックするハイブリッド方式で IME の **ON/OFF のみ** を検知する。

IME の入力方式（ローマ字/かな）の自動検知は行わない（案 D）。IME はローマ字入力モードで使用する前提。将来のかな入力対応は .yab の `[かなシフト無し]` セクション追加（案 C）で対応する。

| プロバイダ | 方式 | 対応 IME |
|---|---|---|
| `TsfProvider` | COM: `ITfThreadMgr` → `ITfCompartmentMgr` → `ITfCompartment` | MS-IME (Windows 10/11 新版) |
| `ImmProvider` | `ImmGetContext` → `ImmGetConversionStatus` | Google 日本語入力, ATOK, 旧 MS-IME |
| `HybridProvider` | TSF を試み、Off なら IMM32 で再確認 | 全 IME |

### 8.2 IME 状態の検知

検知するのは **ImeMode（ON/OFF + 変換モード）のみ**:

```rust
/// IME の変換モード
enum ImeMode {
    Off,           // IME OFF（直接入力）
    Hiragana,      // ひらがなモード
    Katakana,      // カタカナモード
    HalfKatakana,  // 半角カタカナモード
    Alphanumeric,  // 英数モード
}
```

判定ロジック（TSF/IMM32 共通）:

| フラグ | 値 | 判定 |
|---|---|---|
| `IME_CMODE_NATIVE` なし | — | `Alphanumeric` |
| `IME_CMODE_KATAKANA` + `IME_CMODE_FULLSHAPE` | 0x0002 + 0x0008 | `Katakana` |
| `IME_CMODE_KATAKANA` のみ | 0x0002 | `HalfKatakana` |
| それ以外（`IME_CMODE_NATIVE` あり） | — | `Hiragana` |

### 8.3 IME 状態の取得タイミング

キーイベントごとに IME の ON/OFF 状態を取得する。`ImmGetConversionStatus` / TSF 共に数十マイクロ秒で完了するため、パフォーマンスへの影響はない。

### 8.4 将来のかな入力対応（案 C）

初版はローマ字固定出力（案 D）。将来かな入力対応が必要になった場合は、.yab ファイルに `[かなシフト無し]` セクションを追加する方式（案 C）で対応する。IME の入力方式（`IME_CMODE_ROMAN`）の自動検知は行わない。ユーザーがトレイメニューまたは .yab ファイルの切替で出力方式を選択する。

---

## 9. 無限ループ防止の設計

### 9.1 問題

SendInput でキーを注入 → フックが再度捕捉 → 再変換 → 再注入 → ... の無限ループが発生しうる。

### 9.2 対策（3 重の安全弁）

1. **dwExtraInfo マーカー**: 注入時に `INJECTED_MARKER` を設定し、フック冒頭でチェック。一致すれば即座に `CallNextHookEx` で素通し。
2. **OS 予約キーの除外**: 修飾キー（Ctrl, Alt, Shift, Win）、ファンクションキー、ナビゲーションキー等は常にパススルー。Ctrl/Alt が押下中の文字キーもパススルー。
3. **再入ガード**: フックコールバック内で再入フラグを持ち、多重呼び出しを検出した場合は即座に素通し。

---

## 10. 技術的な制約・リスクと実装難所

### 10.1 フックのタイムアウト

Windows はフックコールバックが約 300ms 以内に戻らない場合、フックを自動的に解除する。本設計ではコールバック内で `engine.on_key_event()` を呼ぶだけ（HashMap ルックアップ＋状態更新＋n-gram テーブル引き）なので、数マイクロ秒で完了する。

### 10.2 「全部握りつぶして再注入」の互換性コスト

設計は元キーを握りつぶし `SendInput` / `KEYEVENTF_UNICODE` で再注入する方式を採る。これはフックの安全性には効くが、アプリごとの差異は避けられない。特に一部アプリでは `KEYEVENTF_UNICODE` が不安定になり得る。初版では Win32 アプリでの安定動作を優先し、UWP / WinUI の互換性検証は段階的に進める。

### 10.3 親指シフトの真の難所は KeyUp 整合性

同時打鍵の時間判定（QPC ベース）自体はシンプルだが、実装で事故りやすいのは以下の整合性:

- `active_keys` と保留中キーの KeyUp の対応
- 連続シフト（親指キーを押しっぱなしで文字キーを連打）
- 単独確定後の状態リセット
- 修飾キー混在時の優先順位

状態機械のテストを厚くしないと崩れるため、シミュレーション基盤と判定パターンテストが必須（Phase 1 で 44 テスト実装済み）。

### 10.4 IME 連携の地雷

TSF + IMM32 ハイブリッドは合理的だが、Windows の入力基盤で最も COM / ネイティブ色が濃い領域。

- `ImmGetContext` は UWP 系ウィンドウで無効になる場合がある
- TSF の `ITfCompartment::GetValue` が返す `VARIANT` のハンドリングが windows クレートのバージョンで変わりうる
- IME モード取得自体がフォアグラウンドウィンドウに依存するため、ウィンドウ切替時に状態がずれる可能性がある

IME 連携を早く入れすぎると MVP 全体が遅くなるため、まず IME なし（固定かな入力前提）で安定動作を確認してから統合する。

### 10.5 セキュリティソフトとの競合

一部のセキュリティソフトやアンチチートツールは、グローバルキーボードフックを不審な動作として検出・ブロックする場合がある。

### 10.6 管理者権限

UAC が有効な環境では、管理者権限で動作しているアプリケーションに対しては、非管理者権限のフックが効かない場合がある。

### 10.7 ゲームとの互換性

DirectInput や Raw Input を直接使用しているゲームでは、`WH_KEYBOARD_LL` フックが効かないケースがある。

---

## 11. 設定ファイル

### 11.1 やまぶき互換形式 (.yab)

配列定義ファイルはやまぶき互換の .yab 形式を採用する。これにより既存のやまぶきユーザーが設定をそのまま流用でき、物理キー位置ベースの配列定義が自然に実現できる。

#### 基本構文

```
;コメント行

[セクション名]
行1の値（カンマ区切り）
行2の値
行3の値
行4の値
```

- コメント: `;` で始まる行
- セクション: `[名前]` で始まる
- 各セクション内の4行が、キーボードの物理行に対応:
  - 行1: 数字行（1〜0, -, ^, ¥ — JIS 配列で13キー）
  - 行2: Q行（Q〜P, [, ] — 12キー）
  - 行3: A行（A〜;, :, ] — 12キー）
  - 行4: Z行（Z〜/, _ — 11キー）
- 値はカンマ区切り。行内の位置が物理キー位置に対応

#### セクション一覧

| セクション名 | 用途 |
|---|---|
| `[配列]` | メタ情報（配列名、バージョン、URL） |
| `[ローマ字シフト無し]` | 通常面（IME ON 時） |
| `[ローマ字左親指シフト]` | 左親指同時打鍵面 |
| `[ローマ字右親指シフト]` | 右親指同時打鍵面 |
| `[ローマ字小指シフト]` | Shift キー面 |
| `[英数シフト無し]` | IME OFF / 英数モード時 |

#### 値の形式

| 記法 | 意味 | 例 |
|---|---|---|
| 全角ローマ字 | ローマ字を VK コードとして SendInput → IME が変換 | `ｋａ` → "ka" → IME が「か」に変換 |
| `'文字'` | 文字を Unicode 直接送信（IME バイパス） | `'．'` → 「．」を KEYEVENTF_UNICODE で送信 |
| `無` | 割り当てなし（PassThrough） | |
| `後` | Backspace | |
| `逃` | Escape | |
| `入` | Enter | |
| `空` | Space | |
| `消` | Delete | |

#### NICOLA 配列の例

```
;NICOLA配列
;http://nicola.sunicom.co.jp/spec/kikaku.htm

[ローマ字シフト無し]
１,２,３,４,５,６,７,８,９,０,'－',無,無
．,ｋａ,ｔａ,ｋｏ,ｓａ,ｒａ,ｔｉ,ｋｕ,ｔｕ,'，',，,無
ｕ,ｓｉ,ｔｅ,ｋｅ,ｓｅ,ｈａ,ｔｏ,ｋｉ,ｉ,ｎｎ,後,逃
'．',ｈｉ,ｓｕ,ｆｕ,ｈｅ,ｍｅ,ｓｏ,ｎｅ,ｈｏ,／,無

[ローマ字左親指シフト]
？,'／',～,［,］,'［','］',（,）,｛,｝,無,無
ｌａ,ｅ,ｒｉ,ｌｙａ,ｒｅ,ｐａ,ｄｉ,ｇｕ,ｄｕ,ｐｉ,無,無
ｗｏ,ａ,ｎａ,ｌｙｕ,ｍｏ,ｂａ,ｄｏ,ｇｉ,ｐｏ,ｙｏ,後,逃
ｌｕ,－,ｒｏ,ｙａ,ｌｉ,ｐｕ,ｚｏ,ｐｅ,ｂｏ,'゛',無

[ローマ字右親指シフト]
？,'／',～,［,］,'［','］',（,）,｛,｝,無,無
'゜',ｇａ,ｄａ,ｇｏ,ｚａ,ｙｏ,ｎｉ,ｒｕ,ｍａ,ｌｅ,無,無
ｖｕ,ｚｉ,ｄｅ,ｇｅ,ｚｅ,ｍｉ,ｏ,ｎｏ,ｌｙｏ,ｌｔｕ,後,逃
無,ｂｉ,ｚｕ,ｂｕ,ｂｅ,ｎｕ,ｙｕ,ｍｕ,ｗａ,ｌｏ,無

[ローマ字小指シフト]
！,",＃,＄,％,＆,',（,）,：,＝,～,｜
Ｑ,Ｗ,Ｅ,Ｒ,Ｔ,Ｙ,Ｕ,Ｉ,Ｏ,Ｐ,',｛
Ａ,Ｓ,Ｄ,Ｆ,Ｇ,Ｈ,Ｊ,Ｋ,Ｌ,＋,＊,｝
Ｚ,Ｘ,Ｃ,Ｖ,Ｂ,Ｎ,Ｍ,＜,＞,？,＿
```

#### ローマ字の綴り

設定ファイル内のローマ字は訓令式（やまぶきのデフォルト）:

| ひらがな | 設定値（訓令式） | ヘボン式との違い |
|---|---|---|
| し | `ｓｉ` | ヘボン式: `shi` |
| ち | `ｔｉ` | ヘボン式: `chi` |
| つ | `ｔｕ` | ヘボン式: `tsu` |
| ふ | `ｆｕ` | ヘボン式: `hu` (やまぶきでは `fu`) |

ユーザーが IME のローマ字テーブルに合わせて自由に設定できる。

### 11.2 アプリケーション設定（TOML）

配列定義とは別に、アプリケーションの動作設定は TOML で管理する:

```toml
[general]
# 同時打鍵の判定閾値（ミリ秒）
simultaneous_threshold_ms = 100

# 有効/無効を切り替えるホットキー
toggle_hotkey = "Ctrl+Shift+F12"

# 配列ファイルの格納ディレクトリ
layouts_dir = "layout"

# デフォルトの配列ファイル
default_layout = "nicola.yab"

# n-gram コーパスファイル（Phase 2、オプション）
# ngram_file = "data/ngram_hiragana.toml"
# ngram_adjustment_range_ms = 20
```

---

## 12. 使用クレート

| クレート | バージョン | 用途 |
|---|---|---|
| `windows` | 0.58+ | Win32 API（フック, SendInput, SetTimer, IME, TSF, Shell, COM） |
| `toml` | 0.8+ | 設定ファイルパース |
| `serde` / `serde_derive` | 1.x | 設定ファイルのデシリアライズ |
| `log` / `env_logger` | 0.4+ / 0.11+ | ログ出力 |
| `anyhow` | 1.x | エラーハンドリング |

`windows` クレートの features:

```toml
[target.'cfg(windows)'.dependencies.windows]
version = "0.58"
features = [
    "Win32_Foundation",
    "Win32_UI_WindowsAndMessaging",
    "Win32_UI_Input_KeyboardAndMouse",
    "Win32_UI_Input_Ime",
    "Win32_UI_TextServices",
    "Win32_UI_Shell",
    "Win32_System_Console",
    "Win32_System_Com",
    "Win32_System_Variant",
    "Win32_System_LibraryLoader",
    "Win32_Graphics_Gdi",
]
```

---

## 13. ディレクトリ構成

```
keyboard-hook/
├── Cargo.toml                   # ワークスペース定義
├── rustfmt.toml
├── clippy.toml
├── config.toml                  # アプリケーション設定（TOML）
├── layout/
│   ├── nicola.yab               # NICOLA 配列定義（やまぶき互換）
│   └── (その他の .yab ファイル)
├── data/                         # Phase 2
│   └── ngram_hiragana.toml      # n-gram コーパスデータ
├── crates/
│   └── timed-fsm/               # 唯一の外部クレート（crates.io 公開可能）
│       ├── src/lib.rs
│       └── Cargo.toml
├── src/
│   ├── lib.rs                   # ライブラリクレート（プラットフォーム非依存）
│   ├── config.rs                # TOML 設定 + .yab パーサー
│   ├── scanmap.rs               # スキャンコード ↔ 物理位置
│   ├── engine.rs                # 配列変換エンジン（impl TimedStateMachine）
│   ├── types.rs                 # 共通型定義
│   ├── romaji_kana.rs           # ローマ字↔かな変換テーブル
│   ├── ngram.rs                 # n-gram モデル（Phase 2）
│   ├── main.rs                  # エントリポイント（Windows 専用）
│   ├── hook.rs                  # フック登録・コールバック
│   ├── output.rs                # SendInput キー注入
│   ├── ime.rs                   # IME 状態検知（TSF + IMM32）
│   └── tray.rs                  # システムトレイアイコン
├── tests/
│   └── scenarios.rs             # シナリオテスト（実際の文章入力を模擬）
├── tools/                        # Phase 2
│   └── build_ngram.py           # コーパスから n-gram テーブルを構築
└── docs/
    └── design-2.md              # 本設計書
```

---

## 14. 実装フェーズ

### 設計方針

「壊れない最小構成」を最優先とする。機能の広さよりも、異常時に PassThrough に倒れて入力不能にならないことを重視する。IME 連携や UWP 互換性など Windows 入力基盤の泥臭い部分は、基盤が安定してから段階的に統合する。

### MVP-0: 壊れないキー透過器（実装済み）

**成功条件**: フックが全キーを捕捉し、自己注入マーカーで無限ループを防ぎ、異常時は素通しする。

- `WH_KEYBOARD_LL` フック + `GetMessageW` メッセージループ
- `dwExtraInfo` による自己注入マーカー + 再入ガード
- 修飾キー（Ctrl/Alt）押下中のパススルー（OS ショートカット保護）

### MVP-1: 単純配列変換器（実装済み → やまぶき形式移行で再実装予定）

**成功条件**: Win32 アプリ（メモ帳等）でやまぶき互換設定に基づくキー変換が正しく動作し、KeyUp 整合性が壊れない。

- やまぶき互換 .yab 設定ファイルのパーサー
- スキャンコード → 物理位置 (行, 列) のマッピング（`scanmap.rs`）
- ローマ字列の VK コード送信（IME 経由でかなに変換）
- Shift 面の実装
- 配列定義にないキーの即時パススルー（不要な遅延防止）

### MVP-2: NICOLA 最小版（実装済み）

**成功条件**: 親指シフトの 5 判定パターンすべてが正しく動作し、状態機械のテストが十分に厚い。

- 親指シフト同時打鍵判定（5 パターン + SetTimer/WM_TIMER ハイブリッド）
- NICOLA 標準配列データ完全化（通常面・左右親指面・数字行・OEM キー）
- 状態機械のユニットテスト（44 テスト）

### Phase 1: 常駐化・IME 連携（実装済み → やまぶき形式移行で一部再実装予定）

**成功条件**: トレイアイコンで常駐し、IME OFF 時にバイパスが正しく動作する。

- IME 状態検知（TSF + IMM32 ハイブリッド）
- IME ON/OFF に基づくバイパス制御
- ホットキーによる有効/無効切替（`RegisterHotKey` + `WM_HOTKEY`）
- システムトレイアイコン（右クリックメニュー：有効/無効、配列切替、終了）
- 配列の動的切替（`layouts_dir` 内の .yab を起動時スキャン）

### Phase 2: 互換性拡張 + n-gram 適応的閾値

**2a: 互換性検証・拡張**:
- Win32 アプリでの広範な実地テスト（ブラウザ、エディタ、Office 等）
- UWP / WinUI アプリでの互換性調査と対応
- `KEYEVENTF_UNICODE` が不安定なアプリへの回避策（VK コード送信へのフォールバック等）
- IME ウィンドウ切替時の状態ずれ対策

**2b: n-gram 適応的閾値（前期・頻度ベース）**:
- `ngram.rs` モジュール作成
- コーパスデータ（`ngram_hiragana.toml`）の構築（`tools/build_ngram.py`）
- `NgramModel` の読み込みと `adjusted_threshold()` の実装
- `recent_output` の管理と `is_within_threshold()` の拡張
- 固定閾値との A/B 比較検証

**2c: n-gram 適応的閾値（後期・タイミングベース）**:
- `TypicalInterval` の導入
- `timing_score()` による打鍵間隔パターンの考慮
- 頻度スコアとタイミングスコアの重み付き合成

### Phase 3: クロスプラットフォーム

- `nicola-core` クレートの抽出（プラットフォーム非依存）
- macOS 対応（`CGEventTap` + `CGEventPost`）
- Linux 対応（`evdev` + `uinput`）

### Phase 4: 発展機能

- ユーザーの打鍵ログから個人 n-gram モデルを学習
- 設定 GUI
- DvorakJ 互換の設定形式サポート
