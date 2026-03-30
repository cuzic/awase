# ADR-009: データ付き enum による FSM 状態表現

## ステータス

採用

## コンテキスト

エンジンの FSM は `phase: EnginePhase`（タグのみの enum）と `pending_char: Option<PendingKey>` / `pending_thumb: Option<PendingThumbData>` の3フィールドで状態を表現していた。各状態ハンドラは `self.pending_char.expect("PendingChar phase requires pending_char")` のように実行時にデータの存在を仮定しており、23箇所の `expect()` が存在した。

phase とデータの整合性は手動で維持する必要があり、`go_idle()` で3フィールドをリセットし忘れるとパニックの原因になる。

## 決定

`EnginePhase` + 2つの `Option` フィールドを、データ付き enum `EngineState` に統合する。

```rust
enum EngineState {
    Idle,
    PendingChar(PendingKey),
    PendingThumb(PendingThumbData),
    PendingCharThumb { char_key: PendingKey, thumb: PendingThumbData },
    SpeculativeChar(PendingKey),
}
```

## 結果

- 23箇所の `expect()` が全て不要に（不正な状態がコンパイル時に表現不可能）
- `go_idle()` が3行 → `self.state = EngineState::Idle` の1行に
- `match self.state` でデータと状態タグを同時に取得
- exhaustive match でバリアント追加時にコンパイラが未処理を検出
