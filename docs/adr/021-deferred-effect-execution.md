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

---

## Phase 2 (2026-05-28): キュー構造の精密化

初期実装では「単一 `Vec<Effect>` キュー」「単一 `Vec<RawKeyEvent>` 入力退避」の
2 つだけで運用していたが、実機運用で以下の 3 つの欠陥が露出した。

1. **OUTPUT_GUARD 待ちの Effect を queue 先頭に `push_front` で park** していた
   結果、queue が「純粋 FIFO」ではなくなり、新規 Effect とのインターリーブで
   reinject 順序が崩れる
2. **`INPUT_DEFER` が無制限 `Vec`**。drain 詰まり時に無限に肥大化し、
   復帰後の drain ストームでフックタイムアウトを引き起こす
3. **`Mutex` poison 時に `if let Ok(...) = lock()` で silent drop**。
   入力キーがログ無しで消える

これらに対し、以下の構造的修正を適用した。

### Queue を 3 つの責務に分離（`43c3c15`）

`DecisionExecutor` の Effect 保持を以下 3 構造に分ける。

| 構造 | 型 | 用途 |
|------|----|------|
| `queue` | `VecDeque<Effect>` | 通常の FIFO キュー。`push_back` / `pop_front` のみ |
| `guard_held` | `Option<Effect>` | OUTPUT_GUARD 待ちで park された 1 個 |
| `pending_apply_events` | `Vec<PendingApplyEvent>` | sync IME apply の outcome record（後述） |

**不変条件**: `guard_held.is_some()` ⟺ `TIMER_OUTPUT_GUARD` が登録済み。

`drain_deferred` は「slot を先に試す → 通過したら queue に進む」の 2 段構え。
`park_in_guard` / `output_guard_remaining` という private helper に隔離して、
`push_front` 経路を構造的に消す（将来 `push_front` の追加が
コードレビューで明示的なレッドフラグになる）。

冗長な `guard_timer_active: bool` フラグは撤去（slot の有無が SSOT）。

### 入力退避を bounded ring 化（`b61240c` / `4bad97a`）

`INPUT_DEFER` (`InputDeferQueue`) を以下に変更。

- `Vec<RawKeyEvent>` → `Mutex<VecDeque<RawKeyEvent>>`
- `MAX_CAPACITY = 1024` を associated const として定義
- 容量到達時は **最古の event を `pop_front` で drop**、新規を末尾に追加
- `overflow_count: AtomicUsize` で drop 累積数を追跡、`take_all` でリセット
- warn ログは spam 抑制のため初回 + 2 のべき乗ごとに出力

通常は 0〜数件で運用される想定なので 1024 ヒットは drain 詰まりの兆候。
`push_with_cap` ヘルパーに集約して `defer_during_output` と `replay_later` の
両経路で同じ ring セマンティクスを保証する。

#### Poison 復元（silent drop の根絶）

`Mutex::lock()` が poison したとき、従来は `if let Ok(...)` で握りつぶしており、
silent drop が発生していた。修正:

```rust
let mut q = match self.queue.lock() {
    Ok(q) => q,
    Err(e) => e.into_inner(),  // poison でもデータ自体は健全。復元して使う。
};
```

入力キーが消えるよりは状態を疑いつつでも処理を続ける方が安全という判断。

#### `pending_len_nonblocking` の意味論明確化

`try_lock` ベースで「`None` = ロック競合中で不明」を返すように変更し、
3 callsite で `unwrap_or(0)` を撤去。呼び出し側は保守的に
「pending あり」として扱うルールを doc に明記する。

### Sync apply 完了の record 化（`33257fb` / `c6b143c`）

ImmCross 以外（同期 IME 設定）の apply outcome を、
`DecisionExecutor` から `Runtime` に伝達するための record:

```rust
pub struct PendingApplyEvent {
    pub target: bool,
    pub outcome: awase::platform::ImeOpenOutcome,
}
```

これは「キュー」(FIFO 退避) ではなく「未配送 event の素材」である。
`DecisionExecutor` が `PlatformState` を直接持たない（reducer 依存を断つ）ため、
sync path では outcome を pending record に溜め、
`Runtime::flush_pending_apply_events` が `shadow_model.pending.generation` と
照合してから `ImeApplyRequested/Succeeded/Failed` event を dispatch する。

outcome → event の写像は `ImeEvent::from_apply_outcome` に single source of truth として
集約し、async path (ImmCross の `spawn_local`) と sync path (`flush_pending_apply_events`)
で重複していた `match` arm を 1 箇所に統合した。

### 命名規約

| 旧 | 新 |
|----|----|
| `sync_apply_outcomes` | `pending_apply_events` |
| `drain_sync_apply_outcomes` | `drain_pending_apply_events` |
| `flush_sync_apply_events` | `flush_pending_apply_events` |

「sync」を「pending」に置換することで、実体（PlatformState にアクセス不可な
executor が main thread に渡す pending record）と名前が一致するようになった。

## Phase 2 で得られた教訓

- **「キュー」と呼ぶ前に責務を分解する** — 同一 `VecDeque` に「次に実行する」
  ものと「条件待ちで park された」ものを混ぜると順序保証が壊れる
- **Bounded structure には overflow tracker を付ける** — drop 数の累積が
  drain 詰まりの早期警告になる
- **`Mutex` poison は silent drop の温床** — `into_inner()` で復元するか、
  少なくとも log に残す
- **構造の名前は「役割」を反映させる** — `sync_apply_outcomes` は
  キューに見えるが実体は「次回 flush 時の event 素材」だった
