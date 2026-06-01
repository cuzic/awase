# ADR-041: フック再入時の修飾キー整合性保証

## ステータス

採用済み（2026-05-26）

## コンテキスト

`WH_KEYBOARD_LL` フックコールバックが再入した場合（`OUTPUT_GATE` active 時にフックが呼ばれる）、キーイベントは `InputDeferQueue` に退避される。この退避の際、修飾キー（Ctrl/Shift/Alt）の KeyUp イベントが deferred queue に入ると以下の問題が発生する：

**症状:** IME OFF コマンド送信直後に次のキーを押すと Ctrl+key として誤認識される。

**原因の連鎖:**
1. Ctrl+無変換↓ で IME OFF を実行 → OUTPUT_GATE active
2. 直後の Ctrl↑ が OUTPUT_GATE active 中に到着 → InputDeferQueue に退避
3. IME OFF 完了後に OUTPUT_GATE 解除 → WM_DRAIN で Ctrl↑ が replay
4. しかし drain のタイミングまでに次の文字キー↓ が OS に届き、Ctrl 押下状態のまま処理される

## 決定

修飾キーの KeyUp イベントは、`OUTPUT_GATE` active 中でも **deferred queue に入れずに即座に OS に passthrough** する。

```rust
// hook.rs
if output_gate.is_active() {
    if event.is_modifier_key_up() {
        // 修飾キー KeyUp は defer せず即時 passthrough
        return CallNextHookEx(...);
    }
    // その他は defer
    input_defer.push(event);
    return LRESULT(1);
}
```

加えて、`DecisionExecutor` の OUTPUT_GATE 待ちスロット（`guard_held`）は純粋な FIFO キューとは別管理にし、修飾キー UP が guard_held から抜け出して FIFO を追い越せないようにする（ADR-021 Phase 2 の純粋 FIFO 保証）。

## なぜ修飾キーだけ例外扱いか

修飾キーは「状態キー」であり、その UP イベントが遅れると OS 全体の修飾キー状態が awase の認識と乖離する。文字キーの UP が遅れても「1文字出力が遅れる」だけだが、修飾キーの UP が遅れると「以後の全キー入力が誤動作」する。

修飾キーの UP を即時 passthrough することで OS の修飾キー状態が正しく保たれ、drain 後の文字入力が正しく処理される。

## 結果

- Chrome/Edge での「IME OFF 直後に Ctrl が残留」症状が解消
- `GetAsyncKeyState` で読み取れる修飾キー状態と awase の内部状態の乖離が防止
- `HeldModifiers::restore()` が修飾キー状態を確実に復元できる

## 関連 ADR

- ADR-021 (deferred effect execution)
- ADR-031 (win32-async)
- ADR-037 (キーマップ再割当設計)
