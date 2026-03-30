# ADR-011: RAII ガードによる Win32 リソース管理

## ステータス

採用

## コンテキスト

Win32 リソース（キーボードフック、ホットキー、タイマー、トレイアイコン、WinEvent フック）は手動で cleanup() 内で解放していた。

- `uninstall_hook()` の呼び忘れリスク
- WinEventHook のハンドルが保存されておらず `UnhookWinEvent` が呼ばれない（リーク）
- panic 時にクリーンアップが保証されない

## 決定

各 Win32 リソースに対応する RAII ガード構造体を作成し、`Drop` トレイトで自動解放する。

| ガード | リソース | Drop で呼ぶ API |
|--------|---------|----------------|
| `HookGuard` | キーボードフック | `UnhookWindowsHookEx` |
| `HotKeyGuard` | ホットキー | `UnregisterHotKey` |
| `TimerGuard` | タイマー | `KillTimer` |
| `WinEventHookGuard` | WinEvent フック | `UnhookWinEvent` |
| `SystemTray` (Drop impl) | トレイアイコン | `Shell_NotifyIconW(NIM_DELETE)` |

## 結果

- `cleanup()` は `APP.clear()` + ログのみに簡素化
- WinEvent フックのリークを修正
- panic 時もスタック巻き戻しでリソースが解放される
- リソースの寿命が変数のスコープで明示される
