# ADR 027: IME 状態リフレッシュと IME 制御キーの設計

## ステータス

承認済み（実装完了）

## コンテキスト

### 問題 1: IME 状態の更新タイマーが複数存在

`TIMER_IME_POLL`（500ms 固定周期）と `TIMER_FOCUS_DEBOUNCE`（50ms ワンショット）が独立して動作し、タイミングによってはフォーカス切替中にポーリングが中間ウィンドウの stale な IME 状態を拾ってしまう。

### 問題 2: IME 制御キー後の状態不整合

Ctrl+Muhenkan（IME OFF）を押した後、SetOpen Effect がメッセージループで実行される前にポーリングタイマーが発火すると、OS はまだ旧状態を返す。observer が `preconditions.ime_on` を元の値に上書きし、Engine が一瞬再起動する。

### 問題 3: Zoom で NICOLA 入力が動作しない

Zoom の `ConfMultiTabContentWndClass` は `conv=0x09`（NATIVE + FULLSHAPE、ROMAN ビットなし）を返す。romaji モードでも ROMAN ビットを報告しないため `is_romaji=false` になり、hook のかなバイパスで全キーが Engine をスキップしていた。

### 問題 4: クロスプロセス IME 検出のウィンドウ選択

`GetForegroundWindow()` はトップレベルウィンドウを返すが、wezterm のように入力ウィンドウ（`CASCADIA_HOSTING_WINDOW_CLASS`, conv=0x09）とメインウィンドウ（`org.wezfurlong.wezterm`, conv=0x19）で IME context が異なるアプリでは、誤ったウィンドウの状態を読み取る。

## 決定

### 1. 統合 IME リフレッシュタイマー (TIMER_IME_REFRESH)

`TIMER_IME_POLL` と `TIMER_FOCUS_DEBOUNCE` を 1 本のタイマー `TIMER_IME_REFRESH` に統合する。

```
トリガー           → タイマーリセット値
─────────────────────────────────────
通常ポーリング      → refresh 完了後に 500ms で自動再スケジュール
フォーカス変更      → Engine が 50ms にリセット（500ms poll を暗黙にキャンセル）
SetOpen 実行後     → 20ms で再スケジュール（安全ネット）
即時 refresh       → 直接呼出（スリープ復帰、言語切替等）→ 500ms で再スケジュール
```

フォーカス切替時にポーリングタイマーが中間ウィンドウの状態を拾う問題が構造的に解消される。

### 2. IME 制御キー: 即時更新 + タイマー停止

Sync key（半角全角等）と IME 制御キー（Ctrl+Muhenkan 等）は本質的に異なる:

| | Sync key | IME 制御キー |
|---|---|---|
| 誰が切り替える | OS | 自アプリ (SetOpen) |
| 結果がわかるタイミング | KeyUp 後 | Engine 判断時 |
| 必要なメカニズム | guard (buffer → refresh → replay) | 即時更新 (guard 不要) |

IME 制御キーの処理フロー:

```
Hook callback:
  1. Engine: check_special_keys → SetOpen(false) を返す
  2. preconditions.ime_on = false  ← 即時更新
  3. timer.kill(TIMER_IME_REFRESH) ← ポーリング停止
  4. SetOpen Effect をキューに入れる

Message loop:
  5. WM_EXECUTE_EFFECTS → set_ime_open_cross_process(false)
  6. post_ime_refresh → TIMER_IME_REFRESH を 20ms で再スケジュール
  7. 20ms 後: observer で OS 状態を確認 → 500ms で再スケジュール
```

タイマー停止（ステップ 3）が重要: SetOpen がまだ実行されていない間にポーリングが走り、stale な OS 状態で `preconditions.ime_on` を上書きするのを防ぐ。

### 3. is_romaji 判定: 前回値維持 + ROMAN ビット遷移検出

`detect_ime_state()` の `is_romaji` 判定を 3 段階にする:

1. **直接検出成功** (`Some(bool)`) → そのまま適用
2. **直接検出失敗、ROMAN ビット変化あり** → モード切替として検出
3. **直接検出失敗、ROMAN ビット変化なし** → `None`（前回値維持）

Zoom のように ROMAN ビットを報告しないアプリでは段階 3 が適用され、前のウィンドウで確定した `is_romaji` がそのまま引き継がれる。

wezterm 内でのかな→ローマ字切替（conv 0x19→0x09、ROMAN ビット消失）は段階 2 で検出される。

フォーカス変更時に `prev_conversion_mode` を 0 にリセットして、異なるウィンドウ間の偽の ROMAN 遷移検出を防ぐ。

### 4. hwndFocus ベースのクロスプロセス検出

`detect_ime_state()` と `set_ime_open_cross_process()` で `GetForegroundWindow()` の代わりに `GetGUIThreadInfo().hwndFocus` を使用する。実際のキーボードフォーカスウィンドウの IME context を対象にすることで、wezterm 等のマルチウィンドウアプリでの誤検出を防ぐ。

### 5. NonText バイパスとビルトインバイパスリスト

- hook callback に `focus_kind == NonText` のバイパスを追加
- ビルトインバイパスリストにフォーカス切替中の中間ウィンドウを追加:
  `ForegroundStaging`, `XamlExplorerHostIslandWindow`, `DesktopWindowContentBridge`, `InputSite`, `Shell_TrayWnd`, `TaskListThumbnailWnd`, `TopLevelWindowForOverflowXamlIsland`

## 結果

### 正の結果

- IME 制御キーの応答が即時（guard の遅延なし）
- Zoom で NICOLA 入力が動作
- フォーカス切替時の中間ウィンドウによる Engine 誤作動が解消
- タイマーが 1 本に統合され、重複実行が構造的に排除
- wezterm のかな/ローマ字切替検出と Zoom の ROMAN 未報告が両立

### 負の結果

- `set_ime_open` が失敗した場合、500ms のポーリングまで状態が収束しない
- 一部の IME/アプリで `GetGUIThreadInfo().hwndFocus` が期待と異なる hwnd を返す可能性
- `GetAsyncKeyState` による修飾キー検出にタイミング依存性がある（Ctrl+Henkan のコンボ検出漏れの可能性）

### 設計原則

- **確定情報は即座に反映**: IME 制御キーの結果は Engine 判断時に確定しているため、OS 確認を待たない
- **未確定情報は前回値維持**: direct check 失敗時に推測せず、確認できた最新値を保持
- **タイマーは 1 本**: 全てのリフレッシュトリガーを単一タイマーのリセットで統一
- **副作用で状態を守る**: override フラグではなくタイマー停止で observer の上書きを防止
