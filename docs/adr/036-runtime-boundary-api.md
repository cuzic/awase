# ADR-036: Runtime フィールド境界 API

## ステータス

採用済み（2026-05-30）

## コンテキスト

`Runtime` struct は `engine`, `executor`, `platform`, `platform_state`, `all_keymaps` など多数のフィールドを公開していた。`app/` モジュールが `runtime.executor.platform.xxx` のように深い参照をしていたため：

- `Runtime` 内部構造の移動・リネームが `app/` コードに波及した
- どこから何にアクセスしてよいかのルールが暗黙的だった
- ADR-032 で定義した「app/ は Runtime メソッド経由でのみアクセス」が実装で守られていなかった

## 決定

`Runtime` の全フィールドを `pub(crate)` 以下（または `private`）に変更し、外部（`app/`）からのアクセスは委譲メソッド経由のみとする。

```rust
pub struct Runtime {
    engine: Engine,                      // private
    executor: DecisionExecutor,          // private
    pub platform: WindowsPlatform,       // platform のみ crate 内公開
    platform_state: PlatformState,       // private
    // ...
}

impl Runtime {
    pub fn process_key_event(&mut self, event: RawKeyEvent, ...) -> CallbackResult { ... }
    pub fn on_timer(&mut self, timer_id: usize) { ... }
    pub fn apply_config_update(&mut self, config: ValidatedConfig) { ... }
    pub fn diagnostic_snapshot(&self) -> RuntimeDiagnosticSnapshot { ... }
    // ...
}
```

`app/` モジュールは `Runtime` の公開メソッドのみを呼び出す。内部フィールドへの直接アクセスはコンパイルエラーになる。

## 結果

- `Runtime` の内部リファクタリング（フィールドの移動・統合）が `app/` に波及しなくなった
- `docs/layer-boundaries.md` の B-1 ルール（`with_app` の呼び出し元制限）をコンパイル時に補強
- 公開 API の一覧が `Runtime` の `impl` ブロックを読むだけで分かるようになった

## 関連 ADR

- [ADR-004](004-appstate-orchestrator.md) — AppState orchestrator
- [ADR-014](014-observer-executor-runtime.md) — Observer/Executor/Runtime 分離
- [ADR-035](035-decision-executor-pure-state-machine.md) — DecisionExecutor 純粋化
