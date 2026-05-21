# ADR-031: win32-async クレートの設計

## ステータス

採用済み

## コンテキスト

awase-windows は Windows メッセージループの上で動作するため、以下の 2 つの
本質的に異なる非同期問題を抱えていた:

1. **メッセージループ上の協調的 async 実行**
   `TsfReadinessProbe::wait_until_ready` のような「メッセージループを動かし
   ながら待機する」処理を書きたい。`std::thread::sleep` を使うと、その間
   WinEvent コールバック（`observation_event_proc` 等）が発火しなくなり、
   composition の completion シグナルを取りこぼす。
2. **同期ブロッキング Win32 API のタイムアウト保護**
   `ImmGetContext`, `ImmGetConversionStatus`, `AccessibleObjectFromWindow`,
   UIA の COM 呼び出し等は対象プロセスがハングすると無期限ブロックする。
   `IMM32` には async バージョンが存在せず、UIA も完全な同期 COM API のため、
   これらを async 化することは原理的に不可能。

これら 2 つを 1 つのクレートにまとめるかは検討の余地があったが、いずれも
「Windows メッセージループの内側 / 外側」というスレッドモデルに関わる
低レベル基盤であり、責務として一体化させることにした。

## 決定

### クレートの位置づけ

`crates/win32-async` を「async executor ラッパー + ブロッキング Win32 API
隔離ユーティリティ」として独立クレート化する。awase-windows・lib・他の
将来クレートはすべてこのクレート経由でメッセージループ協調プリミティブに
アクセスする。

```
[package]
name = "win32-async"
version = "0.1.0"
edition = "2021"

[dependencies]
log = "0.4"

[target.'cfg(windows)'.dependencies]
winmsg-executor = "0.3"
```

### 提供する API

**async executor の薄いラッパー（再エクスポート）**

| 名前 | 元 | 役割 |
|------|----|------|
| `block_on` | `winmsg_executor::block_on` | Future をメッセージループを動かしながら完了まで実行 |
| `spawn_local` | `winmsg_executor::spawn_local` | 同スレッドのメッセージループに Future をスポーン |

**メッセージループ協調 async プリミティブ**

| 名前 | モジュール | 役割 |
|------|-----------|------|
| `sleep_ms` | `sleep` | `SetTimer` ベースの非同期 sleep（メッセージループを止めない） |
| `AtomicWatcher` | `atomic_watcher` | `AtomicU32` の変化を event-driven に待つ Future |
| `notify_all` | `atomic_watcher` | `AtomicWatcher` を起動するためのウェイカー |
| `WinEventStream` | `win_event` | WinEvent（OBJ_NAMECHANGE 等）の Stream 抽象 |

**ブロッキング Win32 API 隔離**

| 名前 | モジュール | 役割 |
|------|-----------|------|
| `run_with_timeout` | `thread_timeout` | 任意の処理をワーカースレッドで実行、タイムアウトで放棄して孤児スレッドリストに退避 |
| `SingleThreadCell<T>` | `single_thread_cell` | メッセージループ専用シングルスレッドセル（`Send` 不要なオブジェクトを安全に保持） |

### なぜ async 化できない API が残るか

`run_with_timeout` は async ではなく同期 + ワーカースレッド + タイムアウト
という構造を持つ。これは以下の API が同期コール以外に呼び出し手段を
持たないため:

- `ImmGetContext` / `ImmReleaseContext`: IMM32 はそもそも同期 API
- `ImmGetConversionStatus`: 内部で対象プロセスへ `SendMessage` 同期送信
- UIA COM 呼び出し: COM が同期で、しかも対象アプリの UI スレッドに依存
- `GetGUIThreadInfo`: 対象プロセスがハング中だと無期限ブロック

これらに対して `block_on` ベースの async 待機は無力（待機しても完了しない）
なため、別スレッドに隔離して `JoinHandle` を諦める形を取る。タイムアウト時
にはスレッドを **kill せず** 孤児リストに移し、次回呼び出し時に
`is_finished()` で刈り取る（GC）。孤児が 8 件で打ち止め（`LEAKED_THREAD_MAX`）。

### `windows` クレートと `windows-sys` クレートの共存

awase-windows は両方の Win32 バインディングクレートを使い分ける:

- **`windows` クレート**: COM オブジェクト（TSF の `ITf*` 系インタフェース）を
  扱う箇所。`IUnknown` を Rust の所有権モデルに統合した型を提供
- **`windows-sys` クレート**: raw Win32 API を呼ぶだけの箇所。
  バイナリが小さく依存ビルド時間も短い

`win32-async` は raw Win32（`SetTimer`, `MsgWaitForMultipleObjects`,
`SetWinEventHook` 等）しか触らないため `windows-sys` のみで実装。
COM が必要な上位クレート（awase-windows）が `windows` を別途追加する形。

### 移行の背景となるコミット

| コミット | 内容 |
|---------|------|
| `22f1cd8` | `win32-async` クレート追加 + `std::thread::sleep` を非同期化 |
| `2998a68` | `run_with_timeout` + `SingleThreadCell` を `win32-async` クレートに移動 |

## 結果

### メリット

- `TsfReadinessProbe` の `block_on` ネスト中もメッセージループが回り、
  composition completion シグナル（WinEvent）を取りこぼさなくなった
- 「async で書ける処理」と「ワーカースレッド隔離が必須の処理」が型と
  モジュールで区別され、誤って `block_on` を blocking API に使う事故が
  防げる
- 孤児スレッド GC により、ハング中の Win32 API を呼んでも `run_with_timeout`
  自体は即座に戻り、メッセージループが詰まらない
- awase-windows 以外のクレート（将来の awase-settings 等）も同じプリミティブを
  共有できる

### デメリット

- 孤児スレッドが 8 件で打ち止め後は `run_with_timeout` が `None` を返す
  ため、永続的にハングするアプリがあれば検出が一時停止する
- `winmsg-executor` 0.3 系に依存するため、上流クレートの破壊的変更を
  追従する必要がある
- `windows` と `windows-sys` の二重依存はビルド時間に若干の影響がある

### 影響を受けるファイル

| ファイル | 役割 |
|---------|------|
| `crates/win32-async/Cargo.toml` | クレートメタデータ |
| `crates/win32-async/src/lib.rs` | re-export と公開 API |
| `crates/win32-async/src/sleep.rs` | `sleep_ms`（SetTimer ベース） |
| `crates/win32-async/src/atomic_watcher.rs` | `AtomicWatcher` / `notify_all` |
| `crates/win32-async/src/win_event.rs` | `WinEventStream` |
| `crates/win32-async/src/thread_timeout.rs` | `run_with_timeout` + 孤児 GC |
| `crates/win32-async/src/single_thread_cell.rs` | `SingleThreadCell<T>` |
| `crates/awase-windows/src/ime.rs` 行 286-300 | `run_with_timeout` 使用箇所（`detect_ime_state_with_timeout`） |
| `crates/awase-windows/src/tsf/probe.rs` | `block_on` / `sleep_ms` / `AtomicWatcher` 使用箇所 |
