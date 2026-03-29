# キーボード配列エミュレータ 設計書

## 1. プロジェクト概要

### 1.1 目的

Windows 上で動作するキーボード配列エミュレータを Rust で開発する。
既存の「やまぶき」「DvorakJ」と同等の機能を持ち、NICOLA（親指シフト）を含む任意のキー配列をエミュレートできる常駐型ツールを目指す。

### 1.2 最優先設計原則

本ツールは **「絶対に入力不能にしない配列変換フィルタ」** として設計する。

「正しく変換する」よりも「入力を失わない」ことを優先する。常駐型の入力ツールである以上、不具合時にユーザーがキーボード操作不能に陥ることが最大のリスクである。すべての設計判断はこの原則に基づく。

- **正常時**: 配列変換を適用する
- **異常時**: 元の入力をそのまま通す（PassThrough フォールバック）
- **重大異常時**: 変換を自動停止し、トレイ通知でユーザーに知らせる
- **回復後**: ユーザーの明示的な操作で再開する

### 1.3 スコープ

- Windows 専用（Win32 API ベース）
- Win32 デスクトップアプリケーションおよび UWP / WinUI アプリケーションに対して配列変換を適用
- 親指シフト（NICOLA）の同時打鍵判定に対応
- 設定ファイルによる配列定義の柔軟な切り替え

### 1.4 対応対象の明示

#### Phase 1（MVP）で対応する環境

- 通常の Win32 デスクトップアプリケーション（メモ帳、ブラウザ、Office、エディタ等）
- UWP / WinUI アプリケーション（設定、電卓、ストアアプリ等）
- Windows 10 / 11（64-bit）

`WH_KEYBOARD_LL` はプロセス外フック（DLL インジェクションなし）のため、UWP のサンドボックスに制限されず、大半の UWP アプリで動作する。ただし IME 状態取得において、`ImmGetContext` が UWP ウィンドウ（`ApplicationFrameWindow` / `Windows.UI.Core.CoreWindow`）に対して無効なコンテキストを返す場合がある。この問題は Auto モード（IMM 失敗時に TSF フォールバック）で対処する（§7.4 参照）。

#### Phase 2 以降で対応を検討する環境

- 管理者権限で実行されているアプリケーション
- リモートデスクトップ（RDP）クライアント経由の入力

#### 対応対象外（将来含め非対応）

- DirectInput / Raw Input を直接使用するフルスクリーンゲーム
- アンチチート（EAC / BattlEye 等）を搭載したゲーム
- macOS / Linux
- IME 自体の実装（既存 IME と連携する前提）

### 1.5 スコープ外（初期リリース時点）

- GUI による設定画面（初期は設定ファイル直接編集）
- Microsoft Store 配布（Win32 常駐アプリとして配布）
- 拡張親指シフト（Phase 2）
- 文字キー同時打鍵シフト（Phase 2）

### 1.6 参考プロジェクト

本設計は以下のプロジェクトの実装・運用知見を参考にしている。

- **kanata**（https://github.com/jkehne/kanata）: Rust 製クロスプラットフォームキーリマッパ。LLHOOK バックエンドの実装パターン、スキャンコードベース I/O、チャネルによる入力受付と処理ループの分離、シミュレーションテスト基盤など、多くの設計判断を参考にしている。特に `platform-known-issues.adoc` に記録された Windows LLHOOK 固有の既知問題（§11 参照）は、本ツールの Guard 設計に直接反映している。
- **やまぶき**（http://yamakey.seesaa.net/）: Windows 向け親指シフトエミュレータ。NICOLA 配列の挙動仕様と設定体系の参考。
- **DvorakJ**: Windows 向けキーボード配列変更ツール。機能範囲の参考。

---

## 2. 設計方針

### 2.1 基本原則：観測・変換・注入の分離

本ツールのアーキテクチャは、処理を3つの独立した層に分離する。

```
[Capture 層]  キーイベントの観測とキューイング
      ↓         mpsc チャネル
[Engine 層]   状態機械による配列変換判定
      ↓
[Output 層]   SendInput によるキー注入
```

各層は明確な責務境界を持ち、どの層で障害が起きても PassThrough にフォールバックできる。

これに加え、すべての層を横断的に監視する **Guard 層** を設ける。Guard 層はエラー回数の監視、フック生存確認、状態の強制リセット、自動停止を担当する。

#### OsCode 抽象化層

Capture 層と Output 層の間に **OsCode 抽象化** を設ける。kanata の設計に倣い、OS 固有のキーコード表現を Engine から隠蔽する。

```rust
/// OS 非依存のキーコード。スキャンコード値で識別する。
/// Windows のスキャンコード（Make コード）をそのまま使用し、
/// 拡張キーは上位ビットで区別する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct OsCode(u16);

impl OsCode {
    /// 通常キー用コンストラクタ
    fn new(scan_code: u16) -> Self { Self(scan_code) }

    /// 拡張キー用コンストラクタ（E0 プレフィクス付き）
    fn extended(scan_code: u16) -> Self { Self(scan_code | 0xE000) }

    /// スキャンコード値を取得
    fn scan_code(&self) -> u16 { self.0 & 0x01FF }

    /// 拡張キーかどうか
    fn is_extended(&self) -> bool { (self.0 & 0xE000) == 0xE000 }

    /// Windows 仮想キーコードから変換（フォールバック用）
    fn from_vk(vk: u16, scan: u16, extended: bool) -> Self {
        if extended { Self::extended(scan) } else { Self::new(scan) }
    }
}
```

スキャンコードを主軸にする理由は以下の通り。

- **言語レイアウト非依存**: 仮想キーコード（VK）はキーボードレイアウト設定に依存して値が変わる（例: US 配列の VK_OEM_1 は JP 配列では別のキーに対応する）。スキャンコードは物理キー位置に対応し、レイアウト設定に左右されない。
- **kanata の実装知見**: kanata は `winIOv2` 実装でスキャンコード中心の I/O を採用し、ANSI/ISO/JIS 等の物理配列差異を安定的に吸収している。
- **VK のフォールバック**: 一部の特殊キー（IME 関連キーなど）ではスキャンコードが 0 または不正値になる場合がある。この場合は VK からの逆引きテーブルでフォールバックする。

### 2.2 入力受付スレッドと処理ループの分離

本ツールは **2 つのコンテキスト** で動作する。入力を受け付けるフックコールバックと、変換・出力を行う処理ループを `std::sync::mpsc` チャネルで分離する。

kanata の設計に倣い、フックコールバックではイベントを受け取ってチャネルに投入するだけに留め、処理ループ側で Engine 呼び出し・SendInput 出力・タイマー管理を行う。

この方針を採る理由は以下の通り。

- `WH_KEYBOARD_LL` のフックコールバックには約 300ms のタイムアウト制約がある。コールバック内の処理を最小化することで、タイムアウトリスクを排除する。
- Engine の状態機械や SendInput 呼び出しをフックコールバックから完全に切り離すことで、フック側の処理時間を決定的（数マイクロ秒以下）にできる。
- フックコールバック内でのブロック要因を完全に排除し、OS がフックを自動解除するリスクを最小化する。

#### コールバックの動作

フックコールバックでは以下の処理のみ行う。

1. `dwExtraInfo` で自己注入イベントを判別 → 自己注入なら素通し
2. 再入ガードチェック
3. Guard の自動停止状態チェック → 停止中なら素通し
4. `QueryPerformanceCounter` でタイムスタンプを取得
5. `InputEvent` を構築し、mpsc チャネルに `try_send`
6. 元キーを握りつぶし（`LRESULT(1)` を返す）

フックコールバックは **常にすべてのキーを握りつぶす**（自己注入・再入・停止中の場合を除く）。処理ループ側で Engine が `PassThrough` と判定した場合は、Output 層が元のキーと同じ入力を `SendInput` で再注入する。

この方式は kanata の LLHOOK 実装と同じアプローチである。フックコールバック内で判定結果を待つ必要がなくなるため、フックのタイムアウトリスクを完全に排除できる。ただし、再注入のため Input latency が若干増加する（SendInput 1 回分、通常 < 1ms）。

```
フックコールバック                    処理ループ
    |                                   |
    +- InputEvent 構築                  |
    +- channel.try_send(event)  ------> |
    +- LRESULT(1) を返す                +- event を受信
       （元キーを握りつぶし）            +- engine.process(event)
                                        |   +- PassThrough -> output.send_original(event)
                                        |   +- Emit(actions) -> output.send_keys(actions)
                                        |   +- Suppress -> 何もしない
                                        +- タイマー管理
                                        +- Guard チェック
```

#### チャネル満杯時のフォールバック

`try_send` が失敗した場合（チャネルバッファが満杯）、フックコールバックは元キーを素通しする（`CallNextHookEx` を呼ぶ）。これにより、処理ループが追いつかない場合でもユーザーの入力は失われない。

```rust
match tx.try_send(event) {
    Ok(_) => LRESULT(1),  // 握りつぶし（処理ループに委譲）
    Err(_) => {
        DIAGNOSTICS.dropped_events.fetch_add(1, Ordering::Relaxed);
        CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam)  // 素通し
    }
}
```

#### スレッド構成

```
メインスレッド（フック登録 + メッセージポンプ）
    |
    +- SetWindowsHookExW(WH_KEYBOARD_LL)
    +- フックコールバック: InputEvent を mpsc チャネルに投入
    +- GetMessageW ループ: フックコールバックの実行に必要
    |
    +--- mpsc::sync_channel(128) ---+
                                    |
                                    v
処理スレッド（Engine + Output + タイマー）
    +- チャネルから InputEvent を recv_timeout で受信
    +- Engine に渡して Decision を得る
    +- Output で SendInput を呼ぶ
    +- タイマー管理（Instant ベース）
    +- Guard チェック
```

メインスレッドは `GetMessageW` ループを回し続ける必要がある（`WH_KEYBOARD_LL` のコールバック呼び出しに必要）。処理スレッドは `recv_timeout` でイベントを待ち、タイムアウト時に保留キーの掃除を行う。

#### 処理スレッドのタイマー管理

処理スレッドでは OS の `SetTimer` ではなく、`std::time::Instant` と `recv_timeout` を組み合わせてタイマーを管理する。

```rust
fn processing_loop(rx: mpsc::Receiver<InputEvent>) {
    let mut engine = Engine::new(config);
    let mut output = Output::new();
    let mut guard = Guard::new();
    let mut pending_deadline: Option<Instant> = None;

    loop {
        // 保留があれば deadline までの残り時間を timeout に設定
        let timeout = match pending_deadline {
            Some(dl) => dl.saturating_duration_since(Instant::now()),
            None => Duration::from_secs(1), // アイドル時は 1 秒ごとに Guard チェック
        };

        match rx.recv_timeout(timeout) {
            Ok(event) => {
                let decision = engine.process(event);
                handle_decision(decision, &mut output, &mut guard, &mut pending_deadline);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if pending_deadline.is_some() {
                    // 保留タイムアウト → 単独打鍵として確定
                    let actions = engine.on_timeout();
                    output.send_keys(&actions);
                    pending_deadline = None;
                }
                // 定期 Guard チェック
                guard.periodic_check();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}
```

この方式では `SetTimer` / `WM_TIMER` を使わないため、OS タイマーの精度ブレ（10-16ms）の影響を受けない。`recv_timeout` の精度も OS のスリープ精度に依存するが、同時打鍵判定の中核は QPC ベースのイベント駆動であるため、タイマーは掃除用途に過ぎず精度要件は緩い。

### 2.3 メッセージループの全体像

```
アプリケーション起動
    |
    v
Guard 初期化 + Diagnostics 初期化
    |
    v
mpsc::sync_channel(128) でチャネル作成
    |
    v
処理スレッドを spawn（rx を渡す）
    |
    v
SetWindowsHookExW(WH_KEYBOARD_LL) でフック登録
    |
    v
+--- メインスレッド: メッセージループ（GetMessageW） <--------+
|       |                                                     |
|       +- キーイベント（フックコールバック経由）               |
|       |   +- capture: InputEvent を構築                     |
|       |   +- tx.try_send(event)                             |
|       |   +- LRESULT(1) で握りつぶし                        |
|       |                                                     |
|       +- WM_HOTKEY（一時停止/再開切り替え）                  |
|       |   +- tx.try_send(ControlEvent::Toggle)              |
|       |                                                     |
|       +- WM_POWERBROADCAST / WM_WTSSESSION_CHANGE          |
|       |   +- tx.try_send(ControlEvent::SessionChange)       |
|       |                                                     |
|       +- WM_QUIT                                            |
|           +- ループ脱出 -> フック解除 -> 終了                 |
|                                                             |
+-------------------------------------------------------------+

+--- 処理スレッド: recv_timeout ループ <-----------------------+
|       |                                                      |
|       +- InputEvent 受信                                     |
|       |   +- engine.process(event) を呼ぶ                    |
|       |       +- PassThrough -> output.send_original(event)  |
|       |       +- Emit(actions) -> output.send_keys(actions)  |
|       |       +- Suppress -> pending_deadline を設定          |
|       |                                                      |
|       +- タイムアウト（pending_deadline 到達）               |
|       |   +- engine.on_timeout() を呼ぶ                      |
|       |       +- 保留キーを単独打鍵として確定                 |
|       |                                                      |
|       +- ControlEvent::Toggle 受信                           |
|       |   +- engine.toggle_enabled()                         |
|       |                                                      |
|       +- ControlEvent::SessionChange 受信                    |
|       |   +- engine.force_reset()                            |
|       |   +- guard チェック                                  |
|       |                                                      |
|       +- 定期 Guard チェック（アイドル時 1 秒ごと）           |
|       |   +- guard.periodic_check()                          |
|       |                                                      |
|       +- チャネル切断                                         |
|           +- ループ脱出 -> 終了                               |
|                                                              |
+--------------------------------------------------------------+
```

#### チャネルメッセージの型

```rust
enum ChannelEvent {
    /// キーボード入力イベント
    Key(InputEvent),
    /// 制御イベント（ホットキー、セッション変更等）
    Control(ControlEvent),
}

enum ControlEvent {
    /// 有効/無効切り替え
    Toggle,
    /// セッション変更（スリープ復帰、画面ロック解除等）
    SessionChange,
    /// フォアグラウンドウィンドウ変更
    ForegroundChange,
    /// 設定リロード要求
    ReloadConfig,
    /// 終了要求
    Quit,
}
```

---

## 3. システムアーキテクチャ

### 3.1 層構造と責務

#### Capture 層（oskbd/capture.rs）

責務: 生のキーイベントを受け取り、`OsCode` に変換してタイムスタンプを付与する。判定・変換は一切行わない。

```rust
struct InputEvent {
    seq: u64,              // 単調増加のシーケンス番号
    code: OsCode,          // スキャンコードベースのキーコード
    kind: KeyKind,         // Down / Up
    injected: bool,        // 自己注入イベントか
    ts_qpc: i64,           // QueryPerformanceCounter の値
    extra_info: usize,     // dwExtraInfo の生値
}

enum KeyKind {
    Down,
    Up,
    SysDown,
    SysUp,
}
```

フックコールバック内では以下だけを行う。

1. `dwExtraInfo` で自己注入イベントを判別 → 自己注入なら素通し
2. 再入ガードチェック → 再入なら素通し
3. Guard の自動停止状態チェック → 停止中なら素通し
4. `QueryPerformanceCounter` でタイムスタンプを取得
5. `KBDLLHOOKSTRUCT` から `OsCode` を構築（スキャンコード + 拡張フラグ）
6. `InputEvent` を構築し、mpsc チャネルに `try_send`
7. `LRESULT(1)` を返す（元キーを握りつぶし）

フックコールバック内で以下は **行わない**。

- 同時打鍵判定
- IME 状態の取得
- SendInput の呼び出し
- Engine の呼び出し
- ログ出力（デバッグビルドを除く）

##### OsCode の構築

```rust
fn os_code_from_hook(kb: &KBDLLHOOKSTRUCT) -> OsCode {
    let scan = kb.scanCode as u16;
    let extended = (kb.flags.0 & LLKHF_EXTENDED.0) != 0;

    // スキャンコードが有効な場合はそのまま使用
    if scan != 0 {
        if extended {
            OsCode::extended(scan)
        } else {
            OsCode::new(scan)
        }
    } else {
        // スキャンコードが 0 の場合は VK からフォールバック
        OsCode::from_vk(kb.vkCode as u16, scan, extended)
    }
}
```

#### Engine 層（engine.rs）

責務: 明示的状態機械（§5参照）による配列変換判定。入力イベントを受け取り、出力すべきアクション（Decision）を返す。OS 依存の処理は一切含まない。`OsCode` のみを入出力の単位とする。

```rust
enum Decision {
    /// 元キーをそのまま再注入する
    PassThrough,
    /// 元キーを破棄する（保留中、または出力で置き換え）
    Suppress,
    /// 変換結果を出力する
    Emit(Vec<OutputAction>),
}

enum OutputAction {
    /// OsCode の KeyDown を送信（スキャンコードベース）
    KeyDown { code: OsCode },
    /// OsCode の KeyUp を送信（スキャンコードベース）
    KeyUp { code: OsCode },
    /// KEYEVENTF_UNICODE で Unicode 文字を直接注入
    UnicodeChar(char),
    /// KEYEVENTF_UNICODE で Unicode 文字列を直接注入
    UnicodeString(String),
    /// 何も出力しない（握りつぶし）
    Suppress,
}
```

Engine は純粋な状態機械であり、テスト容易性が高い。OS API への依存がないため、単体テストで全パターンを検証できる。

#### Output 層（oskbd/output.rs）

責務: `SendInput` によるキー注入。自己注入マーカーの付与。注入失敗の検出と報告。

```rust
const INJECTED_MARKER: usize = 0x4B45_594D; // "KEYM"

struct Output {
    diagnostics: DiagnosticsHandle,
}

impl Output {
    /// 変換結果のキーを注入する
    fn send_keys(&self, actions: &[OutputAction]) -> Result<(), OutputError> {
        // OutputAction を INPUT 構造体に変換し、SendInput を呼び出す
        // 戻り値で注入成功数を確認し、失敗した場合は OutputError を返す
    }

    /// PassThrough 判定時に元キーをそのまま再注入する
    fn send_original(&self, event: &InputEvent) -> Result<(), OutputError> {
        // event.code から INPUT 構造体を構築し、SendInput で再注入
    }
}
```

##### スキャンコード主軸の注入方式

`SendInput` 呼び出し時は **スキャンコードを主軸** とし、`KEYEVENTF_SCANCODE` フラグを使用する。kanata の `winIOv2` 実装と同じアプローチである。

```rust
fn os_code_to_input(code: OsCode, key_up: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if code.is_extended() {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),  // スキャンコードベースでは 0
                wScan: code.scan_code(),
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTED_MARKER,
            },
        },
    }
}
```

スキャンコードベースを主軸にする理由は以下の通り。

- VK は Windows のキーボードレイアウト設定に依存して値が変わるが、スキャンコードは物理キー位置に固定されるため、日本語/英語レイアウト切替時の誤動作を防げる。
- RDP / VM 環境で VK ベースの入力が正しく伝達されないケースがあるが、スキャンコードベースの方がロバストである（kanata の運用知見）。
- 一部のアプリケーション（ゲーム、リモートデスクトップクライアント等）はスキャンコードしか見ないため、VK のみの注入では動作しない場合がある。

##### Unicode 文字出力方式

日本語文字の出力には `SendInput` の `KEYEVENTF_UNICODE` フラグを使用し、Unicode 文字を直接注入する。

```rust
fn unicode_char_to_inputs(ch: char) -> Vec<INPUT> {
    let mut inputs = Vec::new();
    // サロゲートペアが必要な文字も処理
    let mut buf = [0u16; 2];
    for &unit in ch.encode_utf16(&mut buf) {
        // KeyDown
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: INJECTED_MARKER,
                },
            },
        });
        // KeyUp
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: INJECTED_MARKER,
                },
            },
        });
    }
    inputs
}
```

この方式は大半のアプリケーションで動作するが、以下の制約がある。

- 一部のゲーム・古いアプリケーションでは `KEYEVENTF_UNICODE` を正しく受け取れない場合がある。この場合は Phase 2 以降で IME 入力シーケンス送信方式を追加検討する。

##### SendInput 失敗時の対応

`SendInput` の戻り値が期待した注入数と一致しない場合、`OutputError` を返す。Guard 層がこれを検知し、連続失敗が閾値を超えた場合に自動停止する（§8参照）。

#### Guard 層（guard.rs）

責務: 異常検知・自動停止・フック生存確認・状態強制リセット。「壊れない設計」の本丸。

詳細は §8 フォールバックと Guard 設計 を参照。

#### Context 層（context.rs）

責務: IME 状態・フォアグラウンドウィンドウ・入力言語のスナップショット管理。

詳細は §7 IME 連携設計 を参照。

#### Diagnostics 層（diag.rs）

責務: ログ出力・メトリクス記録。

```rust
struct Diagnostics {
    dropped_events: AtomicU64,      // チャネル満杯でドロップしたイベント数
    queue_overflow_count: u64,
    output_fail_count: u64,
    state_reset_count: u64,
    hook_reregister_count: u64,
    passthrough_reinject_count: u64, // PassThrough で再注入した回数
    last_error: Option<String>,
}
```

### 3.2 処理フロー

```
キー押下（ハードウェア）
    |
    v
WH_KEYBOARD_LL フックコールバック呼び出し（メインスレッド）
    |
    +- dwExtraInfo == INJECTED_MARKER ?
    |   +- YES -> CallNextHookEx で素通し（無限ループ防止）
    |
    +- 再入ガードに引っかかる？
    |   +- YES -> CallNextHookEx で素通し
    |
    +- guard.is_suspended() ?（重大異常で自動停止中）
    |   +- YES -> CallNextHookEx で素通し
    |
    +- InputEvent を構築（OsCode + QPC タイムスタンプ付与）
    |
    +- tx.try_send(ChannelEvent::Key(event))
    |   +- 成功 -> LRESULT(1)（元キーを握りつぶし）
    |   +- 失敗 -> CallNextHookEx で素通し（フォールバック）
    |
    ~ チャネル経由 ~
    |
    v
処理スレッドが event を受信
    |
    +- engine.process(event) を呼ぶ（catch_unwind で保護）
        |
        +- Decision::Emit(actions)
        |   +- output.send_keys(actions)
        |      -> 成功: active_keys に記録
        |      -> 失敗: guard.on_output_error()
        |
        +- Decision::Suppress
        |   +- pending_deadline を設定（Instant::now() + threshold）
        |
        +- Decision::PassThrough
            +- output.send_original(event)（元キーを再注入）

キー解放（ハードウェア）
    |
    v
（同じフロー: フックで握りつぶし → チャネル → 処理スレッド）
    |
    +- active_keys に物理キーの記録がある？
    |   +- YES -> 記録されている出力キーの KeyUp を送信
    |             active_keys から削除
    |
    +- NO -> output.send_original(event)（元キーの KeyUp を再注入）

            ~ 時間経過 ~

    recv_timeout がタイムアウト（pending_deadline 到達）
        |
        v
    engine.on_timeout() を呼ぶ
        |
        +- 保留キーを単独打鍵として確定
           output.send_keys(actions) で注入
           active_keys に記録
           pending_deadline = None
```

### 3.3 KeyUp 追跡と active_keys の運用ルール

配列変換では、物理キーの KeyDown 時に変換後のキーを出力するが、KeyUp 時にも対応する変換後キーの KeyUp を送信しないと、キーが押しっぱなしになる。`active_keys` でこの対応関係を管理する。

#### 運用ルール

1. **KeyDown 出力時**: `active_keys` に `(OsCode, ActiveKeyEntry)` を記録する。
2. **KeyUp 受信時**: `active_keys` を参照し、記録されている出力キーの KeyUp を送信する。記録がなければ元キーの KeyUp を再注入する。
3. **同時打鍵で出力内容が変わるケース**: 親指シフトの有無で同じ物理キーの出力が変わるが、KeyDown 時の出力結果を `active_keys` に記録するため、KeyUp 時には「押した時点で何を出力したか」に基づいて正しい KeyUp を送信できる。
4. **保留中の KeyUp**: 物理キーが保留中（まだ出力していない）に KeyUp が来た場合、保留をキャンセルし、単独打鍵として即時確定した上で KeyUp を送信する。

```rust
struct ActiveKeyEntry {
    output_code: OsCode,
    is_unicode: bool,
    char_value: Option<char>,
}
```

### 3.4 修飾キー（Ctrl / Alt）との組み合わせ

Ctrl+A、Alt+F4、Ctrl+Shift+Z のような修飾キー付き入力は配列変換をバイパスし、物理キーをそのまま通す。

#### 方針

- **Ctrl または Alt が押されている場合**: 文字キーの配列変換を行わず、`PassThrough` を返す。ショートカットキーが期待通りに動作することを保証する。
- **Shift のみの場合**: Shift は配列の `shift` 面で定義された変換を適用する（配列変換の一部として扱う）。
- **Win キー**: 常に素通しする（OS 予約キー扱い）。

#### ModifierState の更新タイミング

修飾キー（Ctrl / Alt / Shift / Win）の KeyDown / KeyUp を `engine.process()` の冒頭で `ModifierState` に反映してから、変換判定を行う。修飾キー自体は常に `PassThrough` で素通しする。

```rust
fn process(&mut self, event: InputEvent) -> Decision {
    // 修飾キーの状態を先に更新
    if self.update_modifier_state(&event) {
        return Decision::PassThrough;  // 修飾キー自体は素通し
    }
    // Ctrl/Alt が押されていれば変換バイパス
    if self.modifiers.ctrl || self.modifiers.alt {
        return Decision::PassThrough;
    }
    // ... 以降、状態機械による変換処理
}
```

### 3.5 有効/無効切り替え時のキー残留対策

`toggle_enabled()` で変換を無効化する際、既に出力済みで物理キーがまだ押されているキーの処理が必要である。

```rust
fn toggle_enabled(&mut self) -> Vec<OutputAction> {
    self.enabled = !self.enabled;
    if !self.enabled {
        let mut cleanup = Vec::new();
        // 保留中のキーを単独打鍵として確定
        if let Some(pending) = self.state.take_pending() {
            cleanup.extend(self.resolve_as_standalone(pending));
        }
        // active_keys に残っているキーの KeyUp を送信
        for (_, entry) in self.active_keys.drain() {
            cleanup.push(OutputAction::KeyUp {
                code: entry.output_code,
            });
        }
        cleanup
    } else {
        vec![]
    }
}
```

---

## 4. 設定体系の設計

### 4.1 MVP 設定（Phase 1）

MVP では機能より安定性を優先し、設定項目を最小限に絞る。

| 設定項目 | 説明 | デフォルト値 |
|---|---|---|
| 左親指シフトキー | 左親指シフトに使用する物理キー | 無変換（VK_NONCONVERT） |
| 右親指シフトキー | 右親指シフトに使用する物理キー | 変換（VK_CONVERT） |
| 連続シフト（左） | 左親指キーの連続シフト | OFF |
| 連続シフト（右） | 右親指キーの連続シフト | OFF |
| 単独打鍵（左） | 左親指キーの単独打鍵 | 有効 |
| 単独打鍵（右） | 右親指キーの単独打鍵 | 有効 |
| キーリピート（左） | 左親指キーのキーリピート | ON |
| キーリピート（右） | 右親指キーのキーリピート | ON |
| 同時打鍵判定時間 | 同時打鍵と判定される時間範囲（ミリ秒） | 65 |
| 一時停止ホットキー | 一時停止/再開を切り替えるキー | Pause |
| 対象外アプリリスト | 配列変換を適用しないプロセス名のリスト | （空） |
| Windows 起動時に自動起動 | ログオン時に自動起動する | OFF |

#### 連続シフトの動作

連続シフトが OFF の場合、親指キー＋文字キーで1文字出力するたびにシフト状態がリセットされる（通常の親指シフト動作）。

連続シフトが ON の場合、親指キーを押し続けている間は何文字でもシフト面の文字を連続入力できる。PC キーボードの Shift キーと同じ挙動になる。

#### 単独打鍵の動作

単独打鍵が「有効」の場合、親指キーを単独で押してタイムアウトすると、キー本来の機能（無変換→無変換、変換→変換）を出力する。

単独打鍵が「無効」の場合、親指キーを単独で押しても何も出力しない（シフト専用キーとして機能する）。

### 4.2 Phase 2 以降の追加設定

以下は MVP で安定性が確認された後に追加する。設定が増えるほど状態遷移の組み合わせ爆発が起きるため、慎重に追加する。

- **拡張親指シフト設定**: 拡張親指シフトキー（最大2つ）、連続シフト、単独打鍵、キーリピート
- **文字キー同時打鍵シフト設定**: 有効/無効、連続シフト、判定時間
- **IME アクセス API 手動選択**: IMM / TSF / Auto（MVP では Auto 固定）
- **日本語以外の入力言語での文字キー入れ替え**: 有効/無効
- **設定ファイル切り替え通知**: ON / OFF
- **メニュー表示中のキー入れ替え**: ON / OFF

### 4.3 設定ファイルの構造（TOML）

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

# === 動作モード設定 ===
[behavior]
pause_hotkey = "Pause"
auto_start = false

# === 対象外アプリ ===
[behavior.exclude]
processes = []
# processes = ["game.exe", "rdp_client.exe"]

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
```

### 4.4 設定バリデーション

設定ファイル読み込み時にバリデーションを行い、不正な設定値はデフォルト値にフォールバックする。設定ファイル全体が読み込み不能な場合はエラーログを出力し、前回の有効な設定で動作を継続する。

```rust
fn load_config(path: &Path) -> Result<ValidatedConfig, ConfigError> {
    let raw: RawConfig = toml::from_str(&std::fs::read_to_string(path)?)?;
    raw.validate()  // 各フィールドの値域チェック、依存関係チェック
}
```

---

## 5. 状態機械の設計

### 5.1 明示的状態機械

Engine の中核は明示的な有限状態機械（enum で閉じた状態集合）で構成する。散在する `Option<PendingKey>` や `bool` フラグの組み合わせではなく、状態を enum で閉じることで、想定外の状態遷移を型レベルで排除する。

```rust
enum EngineState {
    /// 待機中。キー入力を待っている。
    Idle,

    /// 文字キーが1つ保留中。親指キーとの同時打鍵を待っている。
    PendingChar {
        key: KeyCode,
    },

    /// 親指キーが1つ保留中。文字キーとの同時打鍵を待っている。
    PendingThumb {
        thumb: ThumbSide,
        key: KeyCode,
    },

    /// 親指キーが押下中（連続シフトモード）。
    /// 文字キーが来るたびにシフト面で即時出力する。
    ThumbHeld {
        thumb: ThumbSide,
    },

    /// 文字キー同時打鍵の候補が2つ揃った状態（Phase 2）。
    ChordCandidate {
        first: KeyCode,
        second: KeyCode,
    },

    /// 変換が自動停止されている状態。Guard から遷移する。
    Suspended,
}

/// キー押下情報。OsCode とタイムスタンプのペア。
struct KeyCode {
    code: OsCode,
    ts_qpc: i64,
}

enum ThumbSide { Left, Right }
```

### 5.2 Engine の全体構造

```rust
struct Engine {
    state: EngineState,
    config: EngineConfig,
    keymap: KeyLayout,
    context: InputContext,
    modifiers: ModifierState,
    active_keys: HashMap<OsCode, ActiveKeyEntry>,
    enabled: bool,
    qpc_frequency: i64,
    diagnostics: Diagnostics,
}

struct EngineConfig {
    thumb: ThumbShiftConfig,
    behavior: BehaviorConfig,
}

struct ThumbShiftConfig {
    left_key: OsCode,
    right_key: OsCode,
    left_continuous_shift: bool,
    right_continuous_shift: bool,
    left_standalone: StandaloneMode,
    right_standalone: StandaloneMode,
    left_key_repeat: bool,
    right_key_repeat: bool,
    threshold_ms: u32,
}

struct BehaviorConfig {
    pause_hotkey: OsCode,
    auto_start: bool,
    exclude_processes: Vec<String>,
}

enum StandaloneMode { Enabled, Disabled }

struct ModifierState {
    shift: bool,
    ctrl: bool,
    alt: bool,
    win: bool,
    left_thumb: bool,
    right_thumb: bool,
}

struct ActiveKeyEntry {
    output_code: OsCode,
    is_unicode: bool,
    char_value: Option<char>,
}
```

### 5.3 状態遷移表

```
現在の状態         | イベント              | 条件                     | 次の状態          | 出力
---               | ---                  | ---                      | ---               | ---
Idle              | 文字キー Down        | 親指キー非押下           | PendingChar       | Suppress（保留）
Idle              | 文字キー Down        | 親指キー押下中           | Idle or ThumbHeld | Emit（シフト面）
Idle              | 親指キー Down        | 連続シフト OFF           | PendingThumb      | Suppress（保留）
Idle              | 親指キー Down        | 連続シフト ON            | ThumbHeld         | Suppress（保留）
PendingChar       | 親指キー Down        | 時間内（QPC差 ≤ 閾値）   | Idle              | Emit（同時打鍵）
PendingChar       | 親指キー Down        | 時間外（QPC差 > 閾値）   | PendingThumb      | Emit（先行文字を単独確定）
PendingChar       | 別の文字キー Down    | -                        | PendingChar（新）  | Emit（先行文字を単独確定）
PendingChar       | タイムアウト          | -                        | Idle              | Emit（単独確定）
PendingChar       | 保留文字の KeyUp     | -                        | Idle              | Emit（単独確定 + KeyUp）
PendingThumb      | 文字キー Down        | 時間内（QPC差 ≤ 閾値）   | Idle              | Emit（同時打鍵）
PendingThumb      | 文字キー Down        | 時間外（QPC差 > 閾値）   | PendingChar       | 親指を単独 or 無視
PendingThumb      | タイムアウト          | 単独打鍵=有効            | Idle              | Emit（キー本来の機能）
PendingThumb      | タイムアウト          | 単独打鍵=無効            | Idle              | （何も出力しない）
ThumbHeld         | 文字キー Down        | -                        | ThumbHeld         | Emit（シフト面）
ThumbHeld         | 親指キー Up          | -                        | Idle              | （何もしない）
Suspended         | ユーザー再開操作      | -                        | Idle              | 状態リセット
any               | Ctrl/Alt/Win Down    | -                        | （変更なし）       | PassThrough
any               | 状態リセットトリガー  | -                        | Idle              | active_keys の全 KeyUp
```

### 5.4 状態リセットトリガー

以下のイベントで状態を `Idle` に強制リセットし、`active_keys` に残っているキーの KeyUp をすべて送信する。

- フォアグラウンドウィンドウの変更（`EVENT_SYSTEM_FOREGROUND`）
- Alt+Tab / Win キー操作
- IME モードの変更
- スリープ復帰（`WM_POWERBROADCAST`）
- セッション切替（`WM_WTSSESSION_CHANGE`）
- 画面ロック解除
- Ctrl+Alt+Delete
- `active_keys` のエントリ数が異常に多い場合（20個以上）
- 同一キーの KeyDown が KeyUp なしに連続した場合

```rust
impl Engine {
    fn force_reset(&mut self) -> Vec<OutputAction> {
        let mut cleanup = Vec::new();
        // 保留をフラッシュ
        if let Some(pending) = self.state.take_pending() {
            cleanup.extend(self.resolve_as_standalone(pending));
        }
        // active_keys を全クリア
        for (_, entry) in self.active_keys.drain() {
            cleanup.push(OutputAction::KeyUp {
                code: entry.output_code,
            });
        }
        self.state = EngineState::Idle;
        self.diagnostics.state_reset_count += 1;
        cleanup
    }
}
```

### 5.5 変換テーブルの構造

```rust
struct KeyLayout {
    name: String,
    normal: HashMap<OsCode, KeyAction>,
    shift: HashMap<OsCode, KeyAction>,
    left_thumb: HashMap<OsCode, KeyAction>,
    right_thumb: HashMap<OsCode, KeyAction>,
    /// Phase 2 で追加
    ext_thumb1: HashMap<OsCode, KeyAction>,
    /// Phase 2 で追加
    ext_thumb2: HashMap<OsCode, KeyAction>,
    /// Phase 2 で追加。key: (OsCode, OsCode) のソート済みタプル
    char_simultaneous: HashMap<(OsCode, OsCode), KeyAction>,
}

enum KeyAction {
    /// スキャンコードベースのキーを送信
    Key { code: OsCode },
    /// KEYEVENTF_UNICODE で Unicode 文字を直接注入
    Char(char),
    /// KEYEVENTF_UNICODE で Unicode 文字列を直接注入
    String(String),
    /// キーを握りつぶす（何も出力しない）
    Suppress,
}
```

---

## 6. 同時打鍵判定の詳細設計

### 6.1 QPC ベースのイベント駆動判定

同時打鍵の判定は **`QueryPerformanceCounter`（QPC）ベースの時刻差比較** を主体とする。タイマーは判定の中核ではなく、未確定入力の掃除（ハウスキーピング）専用とする。

#### 判定の流れ

1. フックコールバック内で `QueryPerformanceCounter` を呼び、`InputEvent.ts_qpc` に記録する。
2. Engine が `process()` 内で、保留中のキーと新しいキーの `ts_qpc` 差分を計算する。
3. 差分が閾値以内なら同時打鍵と判定し、閾値を超えていれば単独打鍵として確定する。
4. 通常のタイピングでは次のキーイベントが閾値内に到着するため、大半のケースはイベント駆動だけで判定が完了する。

```rust
impl Engine {
    fn is_within_threshold(&self, ts_a: i64, ts_b: i64) -> bool {
        let diff_ticks = (ts_b - ts_a).abs();
        let diff_ms = diff_ticks * 1000 / self.qpc_frequency;
        diff_ms <= self.config.thumb.threshold_ms as i64
    }
}
```

#### QPC を使う理由

- OS タイマーの精度は約 10-16ms でブレがあり、VM / RDP / 高負荷時にさらに悪化する。
- `QueryPerformanceCounter` は単調増加で精度が高く（通常 1μs 以下）、環境に左右されにくい。
- イベント間の時刻差で判定するため、タイマー精度に依存しない決定的な判定が可能になる。

#### タイマーの役割（掃除専用）

処理スレッドの `recv_timeout` を利用して、「最後のキー入力後、しばらく何も来なかった場合に保留を掃除する」。

```
通常のタイピング（99% のケース）:
  文字キー押下 -> engine が保留 + pending_deadline 設定
  30ms 後に次のキー押下 -> QPC 差分で即時判定
                          pending_deadline クリア

最後の 1 文字（1% のケース）:
  文字キー押下 -> engine が保留 + pending_deadline 設定
  閾値 + α 経過、次のキーなし -> recv_timeout がタイムアウト
                                 engine.on_timeout() で単独確定
                                 pending_deadline クリア
```

`recv_timeout` のタイムアウト値は判定閾値より若干長めに設定する（例: 判定閾値 65ms に対してタイムアウトは 80ms）。これはタイムアウト精度のブレを考慮し、イベント駆動判定の機会を最大化するためである。

### 6.2 判定パターン

```
時間軸 ->

【パターン1: 親指先行 -> 即時確定】
  親指キー ========================
  文字キー         ========
  処理:    親指Down時      文字Down時:
           ThumbHeld or    親指押下中なので
           PendingThumb    即座にシフト面を出力
                           -> Emit（タイマー不要）

【パターン2: 文字先行 -> 時間内に親指 -> QPC 差分で即時確定】
  文字キー ================
  親指キー       ========================
  処理:    文字Down時:   親指Down時:
           PendingChar   QPC差 ≤ 閾値
           deadline設定  -> 同時打鍵として確定
                          deadline クリア -> Emit

【パターン3: 文字単独 -> タイムアウトで確定】
  文字キー ============
  処理:    文字Down時:       recv_timeout:
           PendingChar      on_timeout()
           deadline設定     -> 単独打鍵として確定
                             deadline クリア -> Emit

【パターン4: 文字連打 -> 前の保留を QPC 差分で確定】
  文字キー1 ============
  文字キー2       ============
  処理:     文字1 Down:   文字2 Down:
            PendingChar   QPC差: 親指なし
            deadline設定  -> 文字1を単独確定
                           -> 文字2を新たに保留
                           deadline 再設定

【パターン5: 親指単独 -> 単独打鍵設定に従う】
  親指キー ============
  処理:    親指Down時:       recv_timeout:
           PendingThumb     on_timeout()
           deadline設定     -> 単独打鍵=有効: キー本来の機能を出力
                             -> 単独打鍵=無効: 何も出力しない
                             deadline クリア

【パターン6: 親指先行 + 連続シフト ON -> 複数文字連続入力】
  親指キー ========================================
  文字キー1     ========
  文字キー2              ========
  文字キー3                       ========
  処理:    連続シフト ON のため ThumbHeld 状態
           すべての文字キーをシフト面で即時出力
           -> Emit, Emit, Emit（すべてタイマー不要）

【パターン7: 文字キー同時打鍵シフト（Phase 2）】
  文字キーA ============
  文字キーB      ============
  処理:    文字A Down時:   文字B Down時:
           PendingChar    QPC差 ≤ 閾値
           deadline設定   -> 文字キー同時打鍵テーブルを検索
                           -> ヒット: ChordCandidate -> Emit
                           -> ミス: 文字Aを単独確定、文字Bを新たに保留
```

---

## 7. IME 連携設計

### 7.1 設計方針：スナップショット方式

IME の状態取得を毎打鍵の判定経路に入れるのは危険である。IME は内部状態が非同期で変化するため、フックコールバック中に IME 状態を取得すると、タイミングによって不正確な結果を返す場合がある。また、変換中にキーを奪うと候補ウィンドウが壊れるリスクがある。

本設計では **スナップショット方式** を採用する。IME 状態やフォアグラウンドウィンドウ情報はイベント駆動で更新し、Engine はそのスナップショットを参照するだけとする。

### 7.2 InputContext

```rust
struct InputContext {
    ime_mode: ImeMode,
    fg_window: HWND,
    fg_process_name: String,
    keyboard_layout: u32,
    last_updated: i64,  // QPC
}

enum ImeMode {
    Off,
    Hiragana,
    Katakana,
    HalfKatakana,
    Alphanumeric,
    Unknown,  // 取得失敗時のフォールバック
}
```

### 7.3 更新タイミング

`InputContext` は以下のイベントで更新する。キーイベントごとには更新しない。

- フォアグラウンドウィンドウの変更（`SetWinEventHook` で `EVENT_SYSTEM_FOREGROUND` を監視）
- IME モード変更の通知
- 定期更新（処理スレッドのアイドル時、例: 500ms ごと）

```rust
impl InputContext {
    fn refresh(&mut self, ime_provider: &dyn ImeProvider) {
        let hwnd = unsafe { GetForegroundWindow() };
        self.fg_window = hwnd;
        self.fg_process_name = get_process_name(hwnd);
        self.keyboard_layout = unsafe { GetKeyboardLayout(0) } as u32;
        self.ime_mode = ime_provider.get_mode()
            .unwrap_or(ImeMode::Unknown);
        self.last_updated = qpc_now();
    }

    fn is_japanese_input(&self) -> bool {
        (self.keyboard_layout & 0xFFFF) == 0x0411
    }

    fn is_excluded_app(&self, exclude_list: &[String]) -> bool {
        exclude_list.iter().any(|name| {
            self.fg_process_name.eq_ignore_ascii_case(name)
        })
    }
}
```

### 7.4 IME 状態取得の実装

#### ImeProvider トレイト

```rust
trait ImeProvider {
    fn get_mode(&self) -> Result<ImeMode, ImeError>;
    fn is_enabled(&self) -> Result<bool, ImeError>;
}
```

#### Auto モードの実装

MVP では Auto モード（IMM を試し、失敗したら TSF にフォールバック）を使用する。

```rust
struct AutoImeProvider {
    imm: ImmProvider,
    tsf: Option<TsfProvider>,
}

impl ImeProvider for AutoImeProvider {
    fn get_mode(&self) -> Result<ImeMode, ImeError> {
        match self.imm.get_mode() {
            Ok(mode) if mode != ImeMode::Unknown => Ok(mode),
            _ => {
                if let Some(tsf) = &self.tsf {
                    tsf.get_mode()
                } else {
                    Ok(ImeMode::Unknown)
                }
            }
        }
    }
}
```

#### IMM 実装

```rust
struct ImmProvider;

impl ImeProvider for ImmProvider {
    fn get_mode(&self) -> Result<ImeMode, ImeError> {
        unsafe {
            let hwnd = GetForegroundWindow();
            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                return Ok(ImeMode::Unknown);
            }
            let mut conversion = 0u32;
            let mut sentence = 0u32;
            ImmGetConversionStatus(himc, &mut conversion, &mut sentence);
            ImmReleaseContext(hwnd, himc);

            if (conversion & IME_CMODE_NATIVE) == 0 {
                return Ok(ImeMode::Alphanumeric);
            }
            if (conversion & IME_CMODE_KATAKANA) != 0 {
                if (conversion & IME_CMODE_FULLSHAPE) != 0 {
                    Ok(ImeMode::Katakana)
                } else {
                    Ok(ImeMode::HalfKatakana)
                }
            } else {
                Ok(ImeMode::Hiragana)
            }
        }
    }
}
```

#### TSF 実装

```rust
struct TsfProvider {
    thread_mgr: ITfThreadMgr,
}

impl TsfProvider {
    fn new() -> Result<Self, ImeError> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .map_err(|e| ImeError::ComInit(e))?;
            let thread_mgr: ITfThreadMgr = CoCreateInstance(
                &CLSID_TF_ThreadMgr,
                None,
                CLSCTX_INPROC_SERVER,
            ).map_err(|e| ImeError::TsfInit(e))?;
            Ok(Self { thread_mgr })
        }
    }

    fn get_compartment_value(&self, guid: &GUID) -> Option<u32> {
        unsafe {
            let mgr: ITfCompartmentMgr = self.thread_mgr.cast().ok()?;
            let compartment = mgr.GetCompartment(guid).ok()?;
            let value = compartment.GetValue().ok()?;
            Some(value.Anonymous.Anonymous.Anonymous.ulVal)
        }
    }
}

impl ImeProvider for TsfProvider {
    fn get_mode(&self) -> Result<ImeMode, ImeError> {
        let open = self.get_compartment_value(
            &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE
        ).unwrap_or(0);

        if open == 0 {
            return Ok(ImeMode::Off);
        }

        let conversion = self.get_compartment_value(
            &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION
        ).unwrap_or(0);

        if (conversion & IME_CMODE_NATIVE) == 0 {
            return Ok(ImeMode::Alphanumeric);
        }
        if (conversion & IME_CMODE_KATAKANA) != 0 {
            if (conversion & IME_CMODE_FULLSHAPE) != 0 {
                Ok(ImeMode::Katakana)
            } else {
                Ok(ImeMode::HalfKatakana)
            }
        } else {
            Ok(ImeMode::Hiragana)
        }
    }
}
```

#### TSF 使用時の注意事項

- TSF は COM ベースであるため、使用するスレッドで `CoInitializeEx` を事前に呼ぶ必要がある。
- `ITfThreadMgr` のインスタンスはアプリケーション起動時に1回だけ生成し、以降は使い回す。
- TSF の初期化に失敗した場合は IMM にフォールバックし、エラーログを記録する。
- TSF の Compartment GUID は IME の実装によって対応状況が異なる。一部の IME では TSF 経由でも状態を正しく取得できない場合があるが、`ImeMode::Unknown` を返してフォールバックする。

---

## 8. フォールバックと Guard 設計

### 8.1 Guard 層の責務

Guard 層は「壊れない設計」の中核であり、以下を担当する。

```rust
struct Guard {
    hook_handle: Option<HHOOK>,
    error_count: u32,
    output_fail_count: u32,
    suspended: AtomicBool,  // フックコールバックからも参照するため Atomic
    diagnostics: DiagnosticsHandle,
}
```

`suspended` フィールドは `AtomicBool` とする。フックコールバック（メインスレッド）から素通し判定のために読み取り、処理スレッドから書き込むため、スレッド間で安全に共有する必要がある。

### 8.2 フォールバック一覧

#### Panic Guard

Engine 内で panic 相当のエラーが起きた場合:

1. 変換を即時停止（`suspended = true`）
2. フックは素通しのみを返すようにする
3. トレイアイコンで通知
4. エラーログを保存
5. ユーザーの明示的な再開操作を待つ

```rust
impl Guard {
    fn on_engine_panic(&mut self, error: &str) {
        self.suspended.store(true, Ordering::SeqCst);
        self.diagnostics.log_error(error);
        tray::notify("配列変換を緊急停止しました");
    }
}
```

#### Output Failure Guard

`SendInput` が失敗した場合:

1. 1回だけリトライ
2. リトライも失敗した場合、`output_fail_count` をインクリメント
3. `output_fail_count` が閾値（5回連続）を超えた場合、自動停止

```rust
impl Guard {
    fn on_output_error(&mut self) -> GuardAction {
        self.output_fail_count += 1;
        if self.output_fail_count >= 5 {
            self.suspended.store(true, Ordering::SeqCst);
            tray::notify("出力エラーが連続したため停止しました");
            GuardAction::Suspend
        } else {
            GuardAction::Continue
        }
    }

    fn on_output_success(&mut self) {
        self.output_fail_count = 0;
    }
}
```

#### Hook Alive Guard

フックが OS によって自動解除されていないかを定期確認する。処理スレッドのアイドル時に実行する。

```rust
impl Guard {
    fn ensure_hook_alive(&mut self) -> bool {
        if !self.is_hook_valid() {
            match self.reregister_hook() {
                Ok(handle) => {
                    self.hook_handle = Some(handle);
                    self.diagnostics.hook_reregister_count += 1;
                    true
                }
                Err(e) => {
                    self.suspended.store(true, Ordering::SeqCst);
                    self.diagnostics.log_error(
                        &format!("フック再登録失敗: {}", e)
                    );
                    tray::notify("フックの再登録に失敗しました");
                    false
                }
            }
        } else {
            true
        }
    }
}
```

`WM_POWERBROADCAST`（スリープ復帰）や `WM_WTSSESSION_CHANGE`（セッション切替・画面ロック解除）を受信した場合も、`ControlEvent::SessionChange` をチャネル経由で処理スレッドに通知し、フック生存確認と再登録を行う。

#### State Desync Guard

§5.4 の状態リセットトリガーに加え、以下の状況で `active_keys` と `EngineState` を強制リセットする。

- `active_keys` のエントリ数が異常に多い場合（20個以上）
- 同一キーの KeyDown が KeyUp なしに連続した場合

### 8.3 フックコールバックのコード

```rust
unsafe extern "system" fn hook_callback(
    ncode: i32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    // 異常時は必ず素通し
    if ncode < 0 {
        return CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam);
    }

    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

    // 自己注入チェック
    if kb.dwExtraInfo == INJECTED_MARKER {
        return CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam);
    }

    // 再入ガード
    static REENTRANT: AtomicBool = AtomicBool::new(false);
    if REENTRANT.swap(true, Ordering::SeqCst) {
        return CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam);
    }

    // Guard による自動停止中
    if GUARD.suspended.load(Ordering::SeqCst) {
        REENTRANT.store(false, Ordering::SeqCst);
        return CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam);
    }

    // InputEvent を構築（OsCode + QPC タイムスタンプ付与）
    let mut qpc: i64 = 0;
    QueryPerformanceCounter(&mut qpc);
    let event = InputEvent {
        seq: next_seq(),
        code: os_code_from_hook(kb),
        kind: KeyKind::from_wparam(wparam),
        injected: false,
        ts_qpc: qpc,
        extra_info: kb.dwExtraInfo,
    };

    // チャネルに投入
    let result = match TX.try_send(ChannelEvent::Key(event)) {
        Ok(_) => LRESULT(1),  // 握りつぶし
        Err(_) => {
            DIAGNOSTICS.dropped_events.fetch_add(1, Ordering::Relaxed);
            CallNextHookEx(HOOK_HANDLE, ncode, wparam, lparam)  // 素通し
        }
    };

    REENTRANT.store(false, Ordering::SeqCst);
    result
}
```

### 8.4 処理スレッドのコード

```rust
fn processing_loop(rx: mpsc::Receiver<ChannelEvent>) {
    let config = load_config_or_default();
    let mut engine = Engine::new(config);
    let mut output = Output::new();
    let mut guard = Guard::new();
    let mut pending_deadline: Option<Instant> = None;

    loop {
        let timeout = match pending_deadline {
            Some(dl) => dl.saturating_duration_since(Instant::now()),
            None => Duration::from_secs(1),
        };

        match rx.recv_timeout(timeout) {
            Ok(ChannelEvent::Key(event)) => {
                let result = std::panic::catch_unwind(
                    std::panic::AssertUnwindSafe(|| engine.process(event))
                );
                let decision = match result {
                    Ok(d) => d,
                    Err(_) => {
                        guard.on_engine_panic("engine.process() panicked");
                        continue;
                    }
                };
                handle_decision(
                    decision, &event, &mut engine, &mut output,
                    &mut guard, &mut pending_deadline,
                );
            }
            Ok(ChannelEvent::Control(ControlEvent::Toggle)) => {
                let cleanup = engine.toggle_enabled();
                if !cleanup.is_empty() {
                    let _ = output.send_keys(&cleanup);
                }
            }
            Ok(ChannelEvent::Control(ControlEvent::SessionChange)) => {
                let cleanup = engine.force_reset();
                if !cleanup.is_empty() {
                    let _ = output.send_keys(&cleanup);
                }
                guard.ensure_hook_alive();
            }
            Ok(ChannelEvent::Control(ControlEvent::ForegroundChange)) => {
                engine.context.refresh(&engine.ime_provider);
                let cleanup = engine.force_reset();
                if !cleanup.is_empty() {
                    let _ = output.send_keys(&cleanup);
                }
            }
            Ok(ChannelEvent::Control(ControlEvent::ReloadConfig)) => {
                if let Ok(new_config) = load_config_or_default() {
                    engine.reload_config(new_config);
                }
            }
            Ok(ChannelEvent::Control(ControlEvent::Quit)) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if pending_deadline.is_some() {
                    let actions = engine.on_timeout();
                    if !actions.is_empty() {
                        let _ = output.send_keys(&actions);
                    }
                    pending_deadline = None;
                }
                guard.periodic_check();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle_decision(
    decision: Decision,
    event: &InputEvent,
    engine: &mut Engine,
    output: &mut Output,
    guard: &mut Guard,
    pending_deadline: &mut Option<Instant>,
) {
    match decision {
        Decision::Emit(actions) => {
            *pending_deadline = None;
            match output.send_keys(&actions) {
                Ok(_) => guard.on_output_success(),
                Err(_) => {
                    if guard.on_output_error() == GuardAction::Suspend {
                        return;
                    }
                }
            }
        }
        Decision::Suppress => {
            let threshold = Duration::from_millis(
                engine.cleanup_timer_ms() as u64
            );
            *pending_deadline = Some(Instant::now() + threshold);
        }
        Decision::PassThrough => {
            // 元キーをそのまま再注入
            if let Err(_) = output.send_original(event) {
                guard.on_output_error();
            }
        }
    }
}
```

### 8.5 メインスレッドのメッセージループ

```rust
fn main_message_loop(tx: mpsc::SyncSender<ChannelEvent>) {
    // フォアグラウンド変更監視
    unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_callback),
            0, 0,
            WINEVENT_OUTOFCONTEXT,
        );
    }

    // ホットキー登録
    unsafe {
        RegisterHotKey(HWND::default(), 1, MOD_NOREPEAT, VK_PAUSE.0 as u32);
    }

    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) };
        if ret.0 <= 0 {
            break;
        }
        match msg.message {
            WM_HOTKEY => {
                let _ = tx.try_send(ChannelEvent::Control(ControlEvent::Toggle));
            }
            WM_POWERBROADCAST | WM_WTSSESSION_CHANGE => {
                let _ = tx.try_send(
                    ChannelEvent::Control(ControlEvent::SessionChange)
                );
            }
            _ => {
                unsafe { DispatchMessageW(&msg) };
            }
        }
    }

    // 終了時
    let _ = tx.try_send(ChannelEvent::Control(ControlEvent::Quit));
}
```

---

## 9. 無限ループ防止の設計

### 9.1 対策（3 重の安全弁）

1. **dwExtraInfo マーカー**: 注入時に `INJECTED_MARKER`（`0x4B45_594D`）を設定し、フック冒頭でチェック。
2. **OS 予約キーの除外**: Ctrl+Alt+Delete, Win キー等は常に素通し。
3. **再入ガード**: `AtomicBool` による再入フラグで多重呼び出しを検出。

### 9.2 全握りつぶし方式の安全性

本設計ではフックコールバックで常にすべてのキーを握りつぶし、処理スレッドで再注入する方式を採用している（§2.2 参照）。この方式では以下のリスクが生じるが、対策を講じている。

- **処理スレッドの異常停止**: 処理スレッドが停止するとキー入力が完全に失われる。`Guard.suspended` が `true` になると、フックコールバックは即座に素通しモードに切り替わる。処理スレッド側で `catch_unwind` により panic を捕捉し、`suspended` を `true` に設定する。
- **チャネル満杯**: `try_send` が失敗した場合はフックコールバックが即座に素通しする。これにより処理スレッドが追いつかない場合でも入力は失われない。

---

## 10. エラーハンドリング方針

### 10.1 起動時のエラー

| エラー | 対応 |
|---|---|
| フック登録失敗 | エラーメッセージを表示して終了。管理者権限での再実行を案内する。 |
| TSF（COM）初期化失敗 | IMM にフォールバック。ログに警告を記録。 |
| 設定ファイル読み込み失敗 | デフォルト設定で起動。ログにエラーを記録。 |
| トレイアイコン登録失敗 | ログに警告を記録して続行（トレイなしで動作）。 |
| チャネル作成失敗 | エラーメッセージを表示して終了。 |
| 処理スレッド起動失敗 | エラーメッセージを表示して終了。 |

### 10.2 実行時のエラー

| エラー | 対応 |
|---|---|
| Engine panic | Guard が検知し自動停止。フックは素通しモードに移行。 |
| SendInput 失敗 | 1回リトライ。連続5回失敗で自動停止。 |
| フック自動解除 | Guard が検知し再登録。再登録失敗なら自動停止。 |
| チャネル満杯 | フックコールバックが素通し。Diagnostics にドロップ数を記録。 |
| IME 状態取得失敗 | `ImeMode::Unknown` を返す。配列変換は継続。 |
| 設定リロード失敗 | 前回の有効な設定で動作を継続。ログにエラーを記録。 |

### 10.3 共通原則

- **すべてのエラーパスで、ユーザーの入力が失われないこと**を保証する。
- エラー時は常にログを記録する。
- 自動停止した場合は、トレイアイコンの変化とバルーン通知でユーザーに知らせる。
- ユーザーが明示的に「再開」操作を行うまで、自動で復帰しない。

---

## 11. 技術的な制約・リスク

### 11.1 フックのタイムアウト

Windows はフックコールバックが約 300ms 以内に戻らない場合、フックを自動解除する（`LowLevelHooksTimeout` レジストリ値で制御、デフォルト 300ms）。本設計ではフックコールバック内の処理をチャネル投入のみ（数マイクロ秒）に限定しているため、通常条件ではタイムアウトは発生しない。Guard 層がフックの自動解除を検知して再登録する安全機構を備える。

### 11.2 LLHOOK の既知問題（kanata の運用知見）

kanata の `platform-known-issues.adoc` および issue tracker から、Windows LLHOOK に固有の問題が多数報告されている。以下は本ツールに関連する主要な問題である。

#### 11.2.1 スリープ復帰後のフック無効化

Windows がスリープから復帰した後、LLHOOK が無言で無効化されるケースが報告されている。OS がフックチェーンを再構築する際に、常駐アプリのフックが脱落することがある。

- **対策**: `WM_POWERBROADCAST` / `WM_WTSSESSION_CHANGE` を監視し、セッション変更時にフック生存確認と再登録を行う（§8.2 Hook Alive Guard）。

#### 11.2.2 高負荷時のフック自動解除

CPU 負荷が高い状態では、フックコールバックの処理時間が OS のタイムアウト閾値を超え、フックが自動解除されることがある。特に VM 環境やリモートデスクトップセッションで発生しやすい。

- **対策**: フックコールバック内の処理をチャネル投入のみに限定し、処理時間を決定的に短くする。定期的なフック生存確認で解除を検知し再登録する。

#### 11.2.3 SendInput の UIPI 制限

User Interface Privilege Isolation (UIPI) により、自プロセスより高い整合性レベル（Integrity Level）で動作するプロセスに対して `SendInput` が失敗する。具体的には、通常ユーザー権限で実行している本ツールから、管理者権限で実行されているアプリケーション（例: タスクマネージャ、レジストリエディタ）にキーを注入できない。

- **対策**: Phase 1 ではこの問題を許容し、管理者権限アプリは対象外とする（§1.4）。SendInput の戻り値で失敗を検知し、Guard が連続失敗を監視する。Phase 2 以降で「管理者権限で自プロセスを実行する」オプションの追加を検討する。

#### 11.2.4 AltTab / Win キー操作との干渉

Alt+Tab や Win キーの操作中にキーを握りつぶすと、タスクスイッチャーが正常に動作しなくなるケースがある。特に Alt の KeyUp を握りつぶした場合、Alt キーが「押しっぱなし」状態になる問題が報告されている。

- **対策**: 修飾キー（Ctrl / Alt / Win）は常に `PassThrough` で素通しする（§3.4）。Alt+Tab / Win 操作を検知した場合は状態を強制リセットする（§5.4）。

#### 11.2.5 RDP / VM 環境での KeyUp 消失

リモートデスクトップや仮想マシン環境では、キーイベントの到着順序が乱れたり KeyUp が消失したりするケースがある。これにより `active_keys` にエントリが残り続け、キーが「押しっぱなし」になる問題が生じる。

- **対策**: `active_keys` のエントリ数が異常に多い場合（20個以上）に強制リセットを行う（§5.4）。同一キーの KeyDown が KeyUp なしに連続した場合も強制リセットする。

#### 11.2.6 日本語キーボードの NumLock 状態と VK の不一致

日本語キーボードで NumLock が ON の場合、テンキーのスキャンコードに対して VK が変化する（NumLock ON: VK_NUMPAD0〜9、NumLock OFF: VK_INSERT 等）。VK ベースの判定に依存すると、NumLock 状態によって挙動が変わってしまう。

- **対策**: OsCode（スキャンコードベース）を主軸にすることで、NumLock 状態に影響されない判定を行う（§2.1）。

### 11.3 セキュリティソフトとの競合

グローバルキーボードフックをブロックするセキュリティソフトがある。フック登録失敗時に適切なエラーメッセージを表示する。

### 11.4 管理者権限

UAC 環境下で管理者権限アプリに対してはフックは動作するが SendInput が失敗する（§11.2.3 参照）。

### 11.5 ゲーム・特殊アプリケーションとの互換性

DirectInput / Raw Input を直接使用するゲームではフックが効かないケースがある。これは §1.4 で明示的にスコープ外としている。

### 11.6 TSF の COM 初期化

TSF は COM ベースであるため、使用スレッドで `CoInitializeEx` を事前に呼ぶ必要がある。TSF 初期化失敗時は IMM にフォールバックする。

### 11.7 VM / RDP 環境

仮想マシンやリモートデスクトップ環境では、キーイベントのタイミングが通常と異なる場合がある。QPC ベースの判定はタイマーベースよりロバストだが、判定閾値の調整が必要になる可能性がある。KeyUp 消失のリスクについては §11.2.5 を参照。

### 11.8 全握りつぶし方式の Input Latency

フックで全キーを握りつぶし処理スレッドで再注入する方式（§2.2）では、チャネル経由の受け渡しと SendInput 呼び出しの分、Input latency が若干増加する。通常条件では 1ms 未満の増加であり体感上は問題ないが、レイテンシに極めて敏感なアプリケーション（音ゲー等）では知覚される可能性がある。このようなアプリケーションは §1.4 の「対象外アプリリスト」で除外することを想定する。

---

## 12. 使用クレート

| クレート | バージョン | 用途 |
|---|---|---|
| `windows` | 0.58+ | Win32 API |
| `toml` | 0.8+ | 設定ファイルパース |
| `serde` / `serde_derive` | 1.x | デシリアライズ |
| `log` / `env_logger` | 0.4+ / 0.11+ | ログ出力 |
| `anyhow` | 1.x | エラーハンドリング |
| `trayicon` | 0.5+ | システムトレイアイコン |
| `winit` | 0.30+ | イベントループ（trayicon の依存） |

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
    "Win32_System_Performance",
    "Win32_System_Threading",
    "Win32_Globalization",
]
```

---

## 13. ディレクトリ構成

```
keyboard-layout-emulator/
+-- Cargo.toml
+-- config/
|   +-- nicola.toml          # NICOLA 配列定義
+-- src/
|   +-- main.rs              # エントリポイント、スレッド起動
|   +-- engine.rs            # 状態機械（配列変換判定）※ OS 非依存
|   +-- rules.rs             # キーマップ・配列定義 ※ OS 非依存
|   +-- config.rs            # TOML 設定ファイル読み込み・バリデーション
|   +-- context.rs           # IME / フォアグラウンド / 言語スナップショット
|   +-- guard.rs             # フォールバック・異常監視・自動停止
|   +-- diag.rs              # ログ出力・メトリクス記録
|   +-- tray.rs              # システムトレイアイコン
|   +-- types.rs             # 共通の型定義（OsCode, InputEvent 等）
|   +-- oskbd/               # OS 固有コードの隔離（kanata の oskbd に倣う）
|   |   +-- mod.rs           # oskbd モジュールルート
|   |   +-- capture.rs       # フック登録・解除、InputEvent 構築
|   |   +-- output.rs        # SendInput によるキー注入
|   |   +-- scancode.rs      # OsCode ⇔ VK / スキャンコード変換テーブル
|   |   +-- message_loop.rs  # メインスレッドのメッセージループ
+-- tests/
|   +-- engine_test.rs       # 状態機械の単体テスト（全パターン）
|   +-- sim/                 # シミュレーションテスト基盤（kanata に倣う）
|   |   +-- mod.rs           # シミュレーション実行環境
|   |   +-- nicola_test.rs   # NICOLA 配列のシナリオテスト
+-- docs/
    +-- design.md
```

#### シミュレーションテスト基盤（tests/sim/）

kanata のシミュレーションテスト基盤に倣い、OS API を使わずに Engine の動作を検証するテスト環境を構築する。

```rust
/// テスト用の仮想時刻管理
struct SimClock {
    current_qpc: i64,
}

impl SimClock {
    fn advance_ms(&mut self, ms: u32) {
        self.current_qpc += ms as i64 * 10_000; // 10MHz 想定
    }
}

/// テスト用のキー入力シーケンスビルダー
struct SimInput {
    clock: SimClock,
    events: Vec<InputEvent>,
}

impl SimInput {
    fn press(&mut self, code: OsCode) -> &mut Self {
        self.events.push(InputEvent {
            seq: self.events.len() as u64,
            code,
            kind: KeyKind::Down,
            injected: false,
            ts_qpc: self.clock.current_qpc,
            extra_info: 0,
        });
        self
    }

    fn release(&mut self, code: OsCode) -> &mut Self {
        self.events.push(InputEvent {
            seq: self.events.len() as u64,
            code,
            kind: KeyKind::Up,
            injected: false,
            ts_qpc: self.clock.current_qpc,
            extra_info: 0,
        });
        self
    }

    fn wait_ms(&mut self, ms: u32) -> &mut Self {
        self.clock.advance_ms(ms);
        self
    }
}
```

このシミュレーション基盤を使うことで、以下のテストが OS 非依存で実行可能になる。

- 同時打鍵の全パターン（§6.2 のパターン 1〜7）
- 状態遷移の網羅テスト
- タイムアウト挙動の検証
- 修飾キーとの組み合わせテスト
- 連続シフト・単独打鍵設定の動作テスト
- 対象外アプリ切替時の状態リセットテスト

---

## 14. 実装フェーズ

### Phase 1: MVP（壊れない最小構成）

#### Phase 1a: 最小キーフック + Guard + チャネル基盤

- `WH_KEYBOARD_LL` フックで全キーイベントを取得し、ログ出力する。
- mpsc チャネルによるフックコールバックと処理スレッドの分離を確立する。
- 処理スレッドの `recv_timeout` ループを実装する。
- Guard 層を実装する（フック生存確認、自動停止、再入ガード）。
- `dwExtraInfo` による自己注入マーカーと無限ループ防止を確立する。
- `catch_unwind` による panic 捕捉を組み込む。
- `Guard.suspended`（`AtomicBool`）によるフック素通しモードの切替を実装する。
- チャネル満杯時の素通しフォールバックを実装する。
- Diagnostics（ログ出力・メトリクス）を整備する。

#### Phase 1b: スキャンコードベース I/O + 単純キー置換

- `OsCode` 型を定義し、スキャンコード ⇔ VK 変換テーブルを構築する。
- フックコールバックで `KBDLLHOOKSTRUCT` から `OsCode` を構築する。
- Engine の状態機械を `Idle` のみで実装し、単純なキー置換を行う。
- `active_keys` による KeyUp 追跡を実装する。
- 修飾キー（Ctrl / Alt）のバイパスを実装する。
- Output 層の `SendInput` 実装（`KEYEVENTF_SCANCODE` 主軸、拡張キーフラグ対応）。
- `PassThrough` 時の元キー再注入を実装する。
- SendInput 失敗検知と Guard 連携を実装する。
- `KEYEVENTF_UNICODE` による Unicode 文字注入を実装する。

#### Phase 1c: TOML 設定ファイル対応

- TOML から配列定義と設定を読み込む。
- 設定バリデーションを実装する。
- 対象外アプリリスト機能を実装する。

#### Phase 1d: 親指シフト対応

- 状態機械に `PendingChar` / `PendingThumb` / `ThumbHeld` を追加する。
- QPC ベースの同時打鍵判定を実装する。
- `recv_timeout` ベースの掃除タイマーを実装する（`pending_deadline` 管理）。
- 連続シフト・単独打鍵・キーリピートの各設定を実装する。
- シミュレーションテスト基盤を構築し、7 つの判定パターンの単体テストを作成・通過させる。

#### Phase 1e: 常駐化

- システムトレイアイコン（`trayicon` クレートを使用。状態表示・一時停止/再開・終了のメニュー）。
- 一時停止用ホットキー（`RegisterHotKey` + `WM_HOTKEY` → チャネル経由で処理スレッドに通知）。
- 有効/無効切り替え時のキー残留対策。
- スリープ復帰・セッション切替の検知と再登録（`WM_POWERBROADCAST` / `WM_WTSSESSION_CHANGE` → チャネル経由）。
- フォアグラウンドウィンドウ変更の検知と状態リセット（`SetWinEventHook` → チャネル経由）。
- Windows 起動時の自動起動（レジストリ `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`）。

### Phase 2: 拡張機能

- 拡張親指シフトキー（最大2つ）の同時打鍵判定。
- 文字キー同時打鍵シフト（`ChordCandidate` 状態の追加）。
- IME アクセス API の手動選択（IMM / TSF / Auto）。
- 日本語以外の入力言語での動作設定。
- メニュー表示中の動作設定。
- 設定ファイル変更の自動検知とリロード（`FindFirstChangeNotification` → チャネル経由）。
- TCP / named pipe サーバーによる外部連携（状態通知、リモート制御）。kanata の TCP サーバー方式を参考にする。

### Phase 3: 互換性拡大

- 管理者権限アプリへの対応検討（自プロセスの管理者実行オプション）。
- RDP 環境での動作検証。
- アプリケーションごとの互換性テーブル。

---

## 15. 壊れない設計のルール 10 箇条

1. フックでは InputEvent を構築してチャネルに投入するだけ。Engine 呼び出し・SendInput を入れない。
2. 同時打鍵判定は QPC の単調時刻差ベース。タイマー精度に依存しない。
3. タイマーは未確定入力の掃除専用。判定の中核に使わない。
4. 状態は enum で閉じる。散在する Option / bool の組み合わせにしない。
5. 異常時は常に PassThrough。入力を失わせない。
6. フォアグラウンド変更・IME 切替・セッション変更で状態を強制リセットする。
7. SendInput 連続失敗で自動停止する。
8. 対象外アプリを明示的に除外できる。
9. 診断ログを必ず残す。
10. MVP では機能より安定性を優先する。
