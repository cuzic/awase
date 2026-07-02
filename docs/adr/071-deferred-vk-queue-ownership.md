# ADR-071: deferred VK キューの所有権を probe machine から TsfWarmupCoordinator へ移管

## ステータス

採用済み（2026-07-01 実装、commit 15ac8d8）

## コンテキスト

### バグ: 「にゅうりょく → にうりょく」のような打鍵消失

TSF cold-start probe 実行中（`GjiWarmupCoro` / `TsfProbeCoro` 等が動いている間）に
後続の同時打鍵が届いた場合、その VK が消えるというバグがあった。

例: 「にゅうりょく」と入力しようとすると「にうりょく」になる。

### 根本原因

**原因1: StepCoro の self-priming なし**

`timed_fsm::coro::StepCoro` は「最初の `step()` 呼び出しを受けたとき、
その input を即消費せずにコルーチン本体を最初の `yield` まで進める」という設計のため、
生成直後〜最初の `tick` の前に push された deferred VK が握り潰されていた。

```
probe 生成 → tick 前の短い窓 → deferred VK push → 最初の step() で消費されず消える
```

**原因2: deferred キューが probe machine の所有物だった**

各 probe FSM（`GjiWarmupCoro` / `SacrificialWarmupCoro` 等）が自身の
`pending_deferred: Vec<DeferredVk>` を持っており、probe が別の FSM に切り替わると
キューの中身ごと drop されていた。

```
probe A (deferred: [VK_X]) → probe B に切り替え → VK_X が消える
```

### deferred VK の役割

TSF cold-start probe は GJI context を warmup するために一時的にキー入力を「先送り」する。
probe 完了後に溜まったキーを再送することで、ユーザーには probe が透過的に見える。
このキューが消えると probe 中に打ったキーが無効になる。

## 決定

deferred VK キューの所有権を各 probe machine から `TsfWarmupCoordinator` へ移管する。

### 変更1: probe machine から `push_deferred` / `pending_deferred` を全廃

各 probe FSM の `pending_deferred` フィールドを削除する。
probe 側は「deferred に積む」という操作を認識しなくなる。

### 変更2: TsfWarmupCoordinator が単一キューを保持

```rust
pub(crate) struct TsfWarmupCoordinator {
    /// probe 進行中に届いた後続 VK の単一キュー。
    /// probe machine の生存期間に依存しない単一の書き込み先。
    pending_deferred: RefCell<Vec<DeferredVk>>,
    // ...
}
```

dispatch 側（`Output::vk_send.rs`）が実送信直前に `coordinator.drain_deferred()` で
取り出す。probe machine の入れ替わりやキューの drop と完全に切り離される。

### 変更3: StepCoro に construction 時の self-priming tick を追加

`GjiWarmupCoro` / `TsfProbeCoro` / `SacrificialWarmupCoro` のコンストラクタで
`step(initial_event)` を 1 回呼ぶ（self-priming）。これにより「生成〜最初の tick まで」の
空白窓が消える。

```rust
impl GjiWarmupCoro {
    pub fn new(params: GjiWarmupParams) -> Self {
        let mut coro = Self { ... };
        coro.step(GjiWarmupEvent::Start);  // self-priming
        coro
    }
}
```

## 検討した代替案

### probe machine にキューを残し、FSM 切り替え時にキューを引き継ぐ

→ 採用しなかった。引き継ぎロジックの実装コストに加え、「誰が引き継ぐか」という
  所有権の暗黙的な契約が残り、将来の FSM 追加時にバグが再発しやすい。
  coordinator が「唯一の書き込み先」になることで構造的に不可能にするほうが安全。

### deferred キューをグローバル / thread-local に置く

→ 採用しなかった。coordinator はすでに probe 状態の SSOT であり、
  deferred キューを同じ場所に置くことで「probe 完了時に drain」の流れが自然。
  グローバルにすると probe と無関係なコードが誤って push/pop できてしまう。

### tick を増やして空白窓を縮める

→ 採用しなかった。タイミング勝負の回避策であり、負荷が高い環境では依然として
  窓が開く可能性がある。self-priming は構造的に窓を消す。

## 結果

- 「にゅうりょく」→「にうりょく」のような probe 中打鍵消失が再現しなくなった
- probe FSM の実装が `push_deferred` / `pending_deferred` への依存から解放され、
  probe FSM の追加・変更時に deferred キューの扱いを考慮する必要がなくなった
- `TsfWarmupCoordinator` が probe 関連状態の真の SSOT になった

## 関連 ADR

- ADR-047: TickableFsm・IME warmup 戦略の設計（probe FSM の基盤）
- ADR-053: StepCoro コルーチンパターン（self-priming の実装基盤）
- ADR-069: 凝集性リファクタ H-5-c（TsfWarmupCoordinator の新設）
