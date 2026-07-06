# ADR-053: StepCoro — タイマー駆動コルーチンによる FSM チェーン置換

## ステータス

採用済み（2026-06-27 実装）

## コンテキスト

### 従来の enum FSM の課題

awase-windows の TSF cold-start パスでは、複数の逐次フェーズ（GJI probe → FreshF2 送信 → NameChangeWait → Transmit → LiteralDetect）を enum FSM で実装していた。

この設計には 2 つの構造的な問題があった。

**① 状態爆発と match アームの肥大化**

フェーズが増えるたびに enum バリアントと `match` アームが増える。`GjiWarmupFsm` は最終的に `Phase1Probe` / `Phase2SendFreshF2` / `Phase3NameChangeWait` / `Phase4Transmit` などの enum を持ち、各 `tick()` は全状態に対する分岐を記述しなければならなかった。

**② 複数 FSM 間の SwitchMachine パターン**

「このフェーズが終わったら次の FSM に切り替える」という `SwitchMachine` アクションで FSM を連鎖させていた。`GjiWarmupFsm` → `LiteralDetectFsm`、`SacrificialWarmupFsm` → `ChromeGjiReinitFsm` のように機械切り替えが多発し、切り替え時にコンテキスト（ローマ字文字列、BS カウントなど）を `struct` フィールドや `SwitchMachine` のペイロードとして受け渡す必要があった。

これらの問題から、コルーチンスタイルへの移行を決定した。

## 決定

### StepCoro 基盤の追加（commit e548e63 → 118b3cf）

`crates/timed-fsm/src/coro.rs` に `StepCoro<I, Y>` を実装した。`unsafe`・nightly・外部クレートを一切使わず `std` のみで動作する。

```rust
// コルーチン本体の記述例（yield_step で出力を渡し、次 tick の入力を受け取る）
async fn phase_body(ch: Rc<Channel<TickInput, Vec<ProbeAction>>>) {
    // Phase 1: probe ループ
    loop {
        let input = yield_step(ch.clone(), vec![]).await;
        if probe.check_outcome(total_max_ms).is_some() { break; }
    }
    // Phase 2: FreshF2 送信（局所変数として保持、struct フィールド不要）
    let f2_input = yield_step(ch.clone(), vec![ProbeAction::SendFreshF2 { .. }]).await;
    // Phase 3: NameChangeWait（同じ関数スコープ内でそのまま記述）
    loop {
        let nc_input = yield_step(ch.clone(), vec![]).await;
        if nc_input.env.gji_candidate_visible { break; }
    }
}
```

タイマー駆動のため `Waker::noop()` を使用し、`step(input)` を 1 回呼ぶと future を次の `yield_step` まで進めて `CoroStep::Yielded(output)` を返す。コルーチン完了時は `CoroStep::Complete`。

`StepCoro` は `TickableFsm` トレイトを実装するラッパー struct（`GjiWarmupCoro` など）に格納し、毎 tick の `tick()` から `coro.step(input)` を呼ぶ。

```rust
impl TickableFsm for GjiWarmupCoro {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        let input = TickInput { env: *env, .. };
        match self.coro.step(input) {
            CoroStep::Yielded(actions) => actions,
            CoroStep::Complete => vec![ProbeAction::Done],
        }
    }
}
```

### 置き換えた FSM 一覧

| コルーチン | 置き換えた FSM | コミット |
|-----------|--------------|---------|
| `GjiWarmupCoro` | `GjiWarmupFsm` + `LiteralDetectFsm`（cold パス） | ccd4711 |
| `SacrificialWarmupCoro` | `SacrificialWarmupFsm` + `ChromeGjiReinitFsm` | 2c756dd |
| `TsfProbeCoro` | `TsfProbeMachine` | d1d6d17 |

合計 **-1055 行** の削減。最終的に `step_coro` モジュールを awase-windows ローカルから撤去し `timed_fsm::coro` として公開した（118b3cf）。

### 補足（P4-2, 2026-07-06）: LiteralDetect ロジックの単一所在地化

ccd4711 が畳み込んだのは **cold パス**（`GjiWarmupCoro` の inline Phase 6）であり、
**warm パス**の `LiteralDetectFsm`（`send_romaji_as_tsf_warm` が `pending_tsf` に install）は
別実装として残っていた。その結果、literal 検出の判定ロジック（`check_now` → partial literal
判定 → 回収アクション生成）が cold/warm の 2 箇所に**重複**していた（上表の「置き換えた」は
cold パス限定の意味であり、`LiteralDetectFsm` 型自体は撤去されていなかった）。

P4-2 でこの重複を解消し、判定ロジックを `literal_detect_fsm::LiteralDetectCore` に集約した:

- `LiteralDetectCore` が literal 検出の**単一所在地**。`is_partial_literal` / `emit_recovery_actions`
  もここに集約。
- cold パス（`GjiWarmupCoro` Phase 6）は coro 本体内で `LiteralDetectCore::poll` を駆動する。
- warm パス（`LiteralDetectFsm`）は `LiteralDetectCore` + `OutputActiveGuard` の薄いラッパー。

cold/warm は依然として別の install 経路（互いに排他）を持つが、**判定ロジックは 1 箇所**になった。
挙動（検出条件・タイミング・BS 数・回収アクション）は変更していない。

## なぜこの設計か / 検討した代替案

**async/await ランタイム（tokio 等）の使用**
タイマー駆動の単一スレッドモデルでは外部ランタイムは不要かつ重い。`Waker::noop` と `Pin<Box<dyn Future>>` だけで十分なコルーチン機構が実現できる。

**generator クレート / coroutine nightly 機能**
nightly 依存または外部クレート依存になる。std のみで実装できる `async fn` + `SuspendOnce` future のパターンを選択した。

**FSM のまま継続**
フェーズ間コンテキストを `struct` フィールドに持ち続ける設計は拡張のたびにフィールドが増え、コードの読解コストが高い。局所変数で自然に表現できるコルーチンが優れる。

## 結果

- 多段フェーズ FSM が単一の async 関数として直線的に読めるようになった
- フェーズ間コンテキスト（ローマ字文字列、BS カウント、コールバック等）を struct フィールドから局所変数に降格できた
- `SwitchMachine` による FSM チェーンパターンを撤去し `StartLiteralDetect` → `Continue` の inline 処理に統一した
- `StepCoro` が `timed_fsm::coro` として公開 API に追加され、`HoldingGate` / `TimedStateMachine` と並ぶ timed-fsm の第三の抽象となった

## 関連 ADR

- ADR-042: Clock トレイト抽象化と timed-fsm のテスト可能性（crates.io 公開方針）
- ADR-047: TickableFsm と IME warmup ストラテジー（`GjiWarmupCoro` が実装するトレイト）
- ADR-048: SacrificialWarmup Chrome cold-start 修正（`SacrificialWarmupCoro` の対象問題）
- ADR-051: HoldingGate の timed-fsm クレートへの移植
