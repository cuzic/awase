# ADR-013: 統一 Effect モデル（Decision / Effect パターン）

## ステータス

採用

## コンテキスト

Engine（NICOLA FSM）は timed-fsm の `Response` で副作用を宣言的に記述し、呼び出し側が実行する「判断と実行の分離」を実現していた。しかし Engine の外側（IME ガード、特殊キー判定、IME 制御）は Win32 API（PostMessageW, ImmSetOpenStatus）を直接呼び出す命令的なスタイルだった。

この「一部だけ Pure Architecture」により:
- Engine 外のロジック（7フェーズ中6フェーズ）がテスト不能
- 副作用が main.rs と app_state.rs に散在
- dispatch() が複数箇所から呼ばれる（バグの温床）
- find_match のフォールスルーバグなど、テストで防げたはずの問題が発生

## 決定

**入力パイプライン全体を Effect モデルで統一する。**

1. 全副作用を `Effect` enum で宣言的に表現
2. Engine が `Decision { consumed, effects: Vec<Effect> }` を返す
3. `execute_decision()` が唯一の副作用実行ポイント

```rust
enum Effect {
    SendKeys(Vec<KeyAction>),
    ReinjectKey(RawKeyEvent),
    SetTimer { id, duration },
    KillTimer(usize),
    SetImeOpen(bool),
    RequestImeCacheRefresh,
    UpdateTray { enabled },
}
```

### 構造

```
main.rs (OS アダプタ、4行)
  → Engine::on_input(event, ctx) → Decision  (pure、テスト可能)
  → AppState::execute_decision(decision)      (副作用はここだけ)
```

```
Engine (全判断の入り口、Win32 非依存)
├── InputTracker (物理キー追跡)
├── ImeState (shadow、guard、special keys)
└── NicolaFsm (timed-fsm ベースの同時打鍵 FSM)
```

### 主な変更

- 旧 `Engine` → `NicolaFsm` にリネーム（内部変更なし）
- 新 `Engine` が NicolaFsm + InputTracker + IME 状態 + ガード + 特殊キーを包む
- `matches_key_combo` が `GetAsyncKeyState` → `InputTracker::modifiers()` に（pure 化）
- timed-fsm `Response` は Engine 内部で `Effect` に分解
- `dispatch()` は main.rs から消滅（`Effect::SetTimer` / `Effect::SendKeys` が代替）

## 結果

- **テスト可能性**: Engine::on_input の全7フェーズが純粋関数としてテスト可能
- **副作用の集約**: Win32 API 呼び出しが `execute_decision` 1箇所に
- **311行の純減**: main.rs と app_state.rs が大幅にスリム化
- **バグ防止**: IME/エンジントグルの判定ロジックをユニットテストで検証可能
- **timed-fsm 非依存**: timed-fsm の Response は Engine 内部の実装詳細に
