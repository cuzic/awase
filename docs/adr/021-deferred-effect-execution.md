# ADR-021: Effect 遅延実行によるフックタイムアウト防止

## ステータス

採用

## コンテキスト

Windows の WH_KEYBOARD_LL フックコールバックには 300ms のタイムアウト制限がある。11回連続でタイムアウトするとフックが OS に強制解除される。

従来の実装ではフックコールバック内で Engine の判断（1-5ms）と Effect の実行（20-100ms: SendInput, IME クロスプロセス操作, トレイ更新等）を同期実行しており、合計 21-105ms かかっていた。高負荷時に 300ms を超えてフックが解除される問題が発生していた。

ウォッチドッグによる再インストールは対症療法であり、根本解決ではなかった。

## 決定

### フックコールバックと Effect 実行の完全分離

フックコールバックでは Engine の判断（consume/passthrough）のみ行い、即座に OS に返す。全ての Effect はキューに溜め、メッセージループで実行する。

```
フックコールバック（1-5ms で返る）:
  Engine.on_input() → Decision
  execute_from_hook(decision) → Effects をキューに入れるだけ
  PostMessage(WM_EXECUTE_EFFECTS)
  → OS に consume/passthrough を返す

メッセージループ（時間制約なし）:
  WM_EXECUTE_EFFECTS → drain_deferred() → 全 Effects を FIFO で実行
```

### 2エントリポイント API

`execute()` + `drain_deferred()` の呼び忘れを構造的に防止するため、呼び出しコンテキストに応じた2つの API を提供する。

- `execute_from_hook(decision) → HookResult`: フックコールバック用。Effects はキューに溜めるだけ。`HookResult.has_pending` で PostMessage の必要性を通知。
- `execute_from_loop(decision) → CallbackResult`: メッセージループ用。全 Effects を即座に実行。drain 忘れが構造的に不可能。

### HookResult 型

```rust
pub struct HookResult {
    pub callback: CallbackResult,  // OS に返す consume/passthrough
    pub has_pending: bool,         // PostMessage が必要か
}
```

## 検討した代替案

### 案1: urgent/deferred の2キュー分類

Timer や ReinjectKey はタイミング敏感なので即座実行、SendKeys 等は遅延実行する案。

**却下理由**: フックコールバック内で OS API（SetTimer, SendInput）を呼ぶことになり、レイヤー境界を破る。同時打鍵判定のタイミング情報は RawKeyEvent.timestamp に既に含まれているため、Timer の遅延は判定結果に影響しない。

### 案2: ウォッチドッグの強化

タイムアウト閾値を短縮し、再インストールを高速化する案。

**却下理由**: 対症療法。フックが解除されてから復旧するまでの間、キー入力が失われる。

## 結果

- フックコールバックの実行時間が 21-105ms → 1-5ms に短縮
- OS の 300ms タイムアウトでフックが解除されるリスクを根本的に排除
- フックコールバック内で OS API を一切呼ばない（PostMessage は呼び出し元の hook 層で実行）
- Effect の実行順序は FIFO キューで保証
