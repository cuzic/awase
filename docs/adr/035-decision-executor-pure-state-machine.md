# ADR-035: DecisionExecutor の純粋状態機械化

## ステータス

採用済み（2026-05-30）

## コンテキスト

初期の `DecisionExecutor` は `WindowsPlatform`, `Engine`, `layouts`, `FocusTracker` を直接フィールドとして所有していた。このため：

- `DecisionExecutor::execute()` が副作用（IME 制御・キー送信）と意思決定を混在させていた
- `DecisionExecutor` に依存するコードが WindowsPlatform の API に直接アクセスするため、層境界が曖昧になった
- ユニットテスト時に大量のモック（WindowsPlatform 全体）が必要だった

ADR-014（Observer/Executor/Runtime 分離）では「Executor は決定を実行する」と定義されていたが、実装では Executor が判断・状態保持・副作用を全て担っていた。

## 決定

`DecisionExecutor` から大型フィールドを全て撤去し、「Decision を受け取り Effect を発行する純粋な変換器」に徹する。

**Before:**

```rust
struct DecisionExecutor {
    platform: WindowsPlatform,
    engine: Engine,
    layouts: Vec<LayoutEntry>,
    focus_tracker: FocusTracker,
}
```

**After:**

```rust
struct DecisionExecutor {
    applied_snapshot: Option<(bool, u64)>,  // apply 済み IME 状態キャッシュのみ
}
```

大型フィールドは全て `Runtime` に昇格。`execute()` は `&self` ではなく必要なものを引数で受け取る形に変更。

## 結果

- `executor.rs` のユニットテストが `DecisionExecutor` 単体で書けるようになった
- `self.platform.xxx` 二段階アクセスが `runtime.platform.xxx` 一段階に短縮
- borrow checker が executor/platform の所有関係違反をコンパイル時に検出

## 関連 ADR

- [ADR-014](014-observer-executor-runtime.md) — Observer/Executor/Runtime 分離
- [ADR-036](036-runtime-boundary-api.md) — Runtime フィールド境界 API
