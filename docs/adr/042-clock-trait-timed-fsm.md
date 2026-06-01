# ADR-042: Clock トレイト抽象化と timed-fsm のテスト可能性

## ステータス

採用済み（2026-05-30）

## コンテキスト

`timed-fsm` クレートは `Instant::now()` を直接呼び出していた。これにより：
- 実時間に依存するテストは `sleep()` を挿入するか、タイムアウトが発火するまで待機する必要があった
- `Instant::now()` が Windows の `windows` クレートの `SystemClock` 実装に暗黙的に依存し、クロスプラットフォーム化の妨げになっていた
- テストで「100ms 後にタイムアウトする」を検証するために実際に 100ms 待つ必要があった

## 決定

`timed-fsm` に `Clock` トレイトを導入し、時刻の取得方法を差し替え可能にする。

```rust
// timed-fsm/src/lib.rs
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

pub struct MonotonicClock;
impl Clock for MonotonicClock {
    fn now_ms(&self) -> u64 {
        // std::time::Instant ベース（Windows 非依存）
    }
}

// テスト用
pub struct ManualClock {
    current_ms: AtomicU64,
}
impl ManualClock {
    pub fn advance(&self, ms: u64) { ... }
}
impl Clock for ManualClock {
    fn now_ms(&self) -> u64 { self.current_ms.load(Relaxed) }
}
```

`SystemClock`（Windows epoch ベース、`windows` クレート依存）は `awase-windows` 内に残し、`timed-fsm` は `windows` クレートへの依存を持たない。

`TimedStateMachine` の実装は `Clock` 引数を受け取る形に変更：

```rust
impl ProbeFsm {
    pub fn tick(&mut self, clock: &dyn Clock) -> Option<ProbeFsmOutput> { ... }
}
```

## 境界の明確化

| クロック実装 | 用途 | 依存クレート |
|---|---|---|
| `MonotonicClock` | 本番（プラットフォーム非依存） | `std::time` |
| `ManualClock` | テスト時間注入 | なし |
| `SystemClock` | Windows epoch 計算 | `windows` crate |

`timed-fsm` が `windows` クレートへの依存を持たないことで、Linux/macOS でも `cargo test` が通るようになる。

## 結果

- `ProbeFsm` のテストで `ManualClock::advance(100)` で 100ms 進めてタイムアウトを即時テスト可能
- `timed-fsm` が純粋な `std` 依存クレートになり、crates.io 公開の障壁が下がった
- `SystemClock` が `awase-windows` にあることで「Windows epoch はプラットフォーム責務」が明確

## 関連 ADR

- ADR-022 (クロスプラットフォームクレート構成)
- ADR-019 (プラットフォーム非依存化)
