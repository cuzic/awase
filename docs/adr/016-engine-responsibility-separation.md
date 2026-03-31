# ADR-016: Engine 内部の責務分離（5層構造）

## ステータス

採用

## コンテキスト

Engine（統合エントリポイント）と NicolaFsm（同時打鍵 FSM）がそれぞれ多くの責務を抱えていた。Engine は IME shadow 管理・ガード・特殊キー判定・Response 変換をすべて持ち、NicolaFsm は状態遷移・確定モード戦略・出力履歴・タイミング判定をすべて持っていた。

## 決定

Engine と NicolaFsm の内部を責務ごとにモジュール分割する。

### Engine 側の分離

| モジュール | 責務 |
|-----------|------|
| `engine.rs` | オーケストレーション（on_input / on_timeout / on_command） |
| `ime_coordinator.rs` | shadow IME 状態追跡、IME トグルガード、deferred key バッファ |
| `fsm_adapter.rs` | timed-fsm Response → Decision/Effect 変換 |

### NicolaFsm 側の分離

| モジュール | 責務 |
|-----------|------|
| `nicola_fsm.rs` | 状態遷移（decide_and_transition + step_*）|
| `confirm_policy.rs` | 5つの確定モード戦略（Wait/Speculative/TwoPhase/Adaptive/Ngram）|
| `timing.rs` | タイミング判定（is_simultaneous / three_key_pairing / should_speculate）|

### Engine の公開 API

```rust
on_input(event, &InputContext) → Decision
on_timeout(timer_id, &InputContext) → Decision
on_command(EngineCommand) → Decision
process_deferred_keys(&InputContext) → Vec<Decision>
```

### EngineCommand（外部コマンド、9バリアント）

```rust
ToggleEngine, InvalidateContext, SwapLayout, SyncImeState,
SetGuard, ClearDeferredKeys, ReloadKeys, UpdateFsmParams, SetNgramModel
```

## 結果

最終的なファイル構成:

```
engine/
├── mod.rs              (35行)   — re-export
├── engine.rs          (386行)   — オーケストレータ
├── nicola_fsm.rs     (1200行)   — パーサーFSM
├── timing.rs          (180行)   — タイミング判定集約
├── confirm_policy.rs  (160行)   — 確定モード戦略
├── ime_coordinator.rs (170行)   — IME 状態管理
├── fsm_adapter.rs     (127行)   — timed-fsm ↔ Decision
├── decision.rs        (262行)   — 公開 API 型
├── fsm_types.rs       (270行)   — FSM 内部型
├── observation.rs      (65行)   — 観測結果型
├── input_tracker.rs   (156行)   — 物理キー追跡
└── output_history.rs  (257行)   — 出力履歴
```

各ファイルが400行以下（nicola_fsm.rs を除く）。nicola_fsm.rs の1200行は decide_* メソッド群（アクションテーブル）であり、これ以上の分割は自然な境界がない。
