---
paths:
  - "crates/awase-windows/src/**/*.rs"
---

# IME belief アーキテクチャルール

## Observe → pure decision → belief の三層分離

IME 状態（ON/OFF・input_mode）の belief 更新は必ず以下の流れを守ること。

```
Observe     Win32 API / async probe → ImeSnapshot / raw value
Pure        classify_* 関数 → ImeUpdate / Option<InputModeState>  ← 副作用ゼロ
Apply       dispatch_event → reduce()  ← belief の唯一の書き込み点
```

## input_mode の変更ルール

### ✅ 正しいパターン

観測結果から input_mode を更新するときは `classify_fetched_snapshot()` を経由する。

```rust
// ImmCrossProbe・FocusProbe 等で snap が手に入った場合
let update = crate::observer::ime_observer::classify_fetched_snapshot(
    &snap,
    tick_ms.0,
    app.platform_state.ime.effective_open(),
    app.platform_state.ime.is_force_on_guard_active(),
    app.platform_state.ime.input_mode(),
    app.platform_state.ime.belief.prev_conversion_mode(),
);
if let Some(mode) = update.new_input_mode {
    app.platform_state.ime.dispatch_event(
        ImeEvent::InputModeObserved { mode, source, at: tick_ms },
        tick_ms,
    );
}
```

### ❌ 禁止パターン

インラインで input_mode を判定して `dispatch_event(InputModeObserved)` を直接呼ぶ。

```rust
// NG: classify_* を経由せず直接判定している
if !ConvMode::from_u32(conv).is_eisu()
    && matches!(app.platform_state.ime.input_mode(), InputModeState::ObservedEisu)
{
    app.platform_state.ime.dispatch_event(
        ImeEvent::InputModeObserved { mode: InputModeState::AssumedRomaji { .. }, .. },
        tick_ms,
    );
}
```

`classify_ime_snapshot` / `classify_fetched_snapshot` はその判定ロジックを純粋関数として集約するために存在する。同じ判定を外部で再実装しない。

## ON/OFF belief の変更ルール

- **High confidence（ImmCross/FocusProbe）**: `write_imm_cross_probe(open)` / `write_focus_probe(open)` を使う
- **Medium confidence（定期 poll）**: `apply_ime_update(&update)` 経由（`poll_and_classify_ime` の戻り値）
- `dispatch_event(ImeEvent::ObserverReported { .. })` を直接呼ぶのは上記メソッドの内部に限る

## belief の書き込み点

`ImeModel::reduce()` in `state/ime_model.rs` が唯一の書き込み点。
`input_mode` フィールドへの直接代入は `reduce()` 以外では禁止。
