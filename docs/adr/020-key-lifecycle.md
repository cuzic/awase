# ADR-020: KeyLifecycle による Down/Up ペア追跡

## ステータス

採用

## コンテキスト

修飾キー（Shift, Ctrl 等）が押下状態で残る不具合が発生していた。根本原因は KeyDown を Engine が Consume した場合に、対応する KeyUp が OS に漏れることだった。

- IME ガードが KeyDown のみバッファし、KeyUp を素通しさせていた
- フォーカス変更時に deferred keys を破棄し、修飾キーの KeyUp も消えていた
- KeyDown と KeyUp がバラバラに扱われ、ペアとしての整合性が保証されていなかった

## 決定

`KeyLifecycle` 構造体を導入し、Engine が Consume した全 KeyDown を追跡する。

### 動作

1. **KeyDown が Consume された場合**: `on_key_down_consumed()` で記録
2. **KeyUp 到着時**: `on_key_up()` で対応する KeyDown が Consume 済みなら `true` を返す → Engine は KeyUp も自動的に Consume する（OS に渡さない）
3. **コンテキスト変更時**: `flush_pending_key_ups()` が Consume 済みで KeyUp が来ていないキーの KeyUp イベントを生成 → OS に再注入して状態を整合させる

### IME 切替後の修飾キー同期

`SyncModifiers` コマンドを追加。IME トグル直後に `GetAsyncKeyState` で OS の実際の修飾キー状態を読み取り、Engine の内部状態と比較して不整合を修正する。

### IME ガードの修正

ガード中は KeyDown だけでなく KeyUp もバッファするように変更。KeyDown が Consume されているのに KeyUp だけ OS に渡る問題を解消。

## 結果

- 修飾キーが押下状態で残る不具合を構造的に防止
- Engine の Phase 0 で KeyUp 自動追跡が入るため、全てのキーイベントパスで整合性が保証される
- IME 切替時の修飾キー不整合も自動修復
