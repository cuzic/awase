# ADR-074: ObservedEisu 自動直接入力切替 — IME ON 英数モードを idle-conv-check で自動 OFF

## ステータス

採用済み（2026-07-01 実装、commit 1ef82ca / 754a7a4）

## コンテキスト

### ObservedEisu とは

`InputModeState::ObservedEisu` は「IME が ON になっているが変換モードが半角英数
（`conv_mode = 0x0010` など、`IME_CMODE_NATIVE` ビットが 0）」という状態。

Windows では IME ON のまま半角英数入力ができる。しかし awase から見ると
これは IME ON（shadow=true）のはずなのに実際には英字が直接入力されており、
IME ON の状態で日本語入力を行おうとするとエンジンが誤動作する。

### 発生ケース

1. `VK_DBE_ALPHANUMERIC`（直接入力）などを直接押してしまった
2. MS-IME の半角英数モードに入った（`conv_mode = 0x0010`）
3. GJI で ROMAN フラグのみの状態（`conv_mode = 0x0012` など）に陥った

### 問題: awase エンジンが誤検出

`shadow=ON` のまま `ObservedEisu` が続くと、次のキー押下で awase は
「IME ON で日本語入力ができる」と仮定してエンジンを動かし続ける。
しかし OS は英字を直接出力するため、awase の出力とユーザーの期待がずれる。

### 既存対策の限界

`idle_conv_check` はすでに定期的に `conv_mode` を読んで `InputModeState` を更新していたが、
`ObservedEisu` を検出しても IME 状態を変更するアクションを起こしていなかった。

`SetOpen(true)` の後に `ObservedEisu` が残る場合（commit 754a7a4）も問題で、
`SetOpen(true)` 直後に `ObservedEisu` が残っていると engine が NotRomajiInput のまま
活性化できない症状があった。

## 決定

### 変更1: idle-conv-check で ObservedEisu → 自動直接入力切替

`idle_conv_check` 実行時に `input_mode == ObservedEisu` かつ `shadow=ON` だった場合、
`apply_ime_open_with_applied(false)` + `handle_engine_set_open(false)` を自動発行する。

```rust
// runtime/key_pipeline.rs
if new_mode == InputModeState::ObservedEisu {
    log!("[idle-conv-check] TsfNative: ObservedEisu 検出 → DirectInput");
    self.apply_ime_open_with_applied(false);
    self.platform_state.handle_engine_set_open(false);
}
```

- `apply_ime_open_with_applied(false)`: IME OFF を apply（shadow=OFF に更新）
- `handle_engine_set_open(false)`: `last_explicit_ime_action_ms` を更新し、
  `EXPLICIT_IME_SUPPRESS_MS`（1500ms）以内は再検出ループにならないよう抑制

### 変更2: SetOpen(true) + ObservedEisu → AssumedRomaji にリセット

`apply_ime_open_with_applied(true)`（IME ON 成功）後に `input_mode == ObservedEisu` が
残っている場合、VK_KANJI / VK_IME_ON の送信によって GJI がひらがなモードへ遷移するため
`ObservedEisu` は stale になる。これを `AssumedRomaji` にリセットして engine を即活性化する。

```rust
// SetOpen(true) 完了後のポスト処理
if effective == true
    && self.platform_state.ime.input_mode() == InputModeState::ObservedEisu
{
    self.platform_state.ime.set_input_mode(InputModeState::AssumedRomaji);
}
```

### 誤ループ防止

- `handle_engine_set_open(false)` が `last_explicit_ime_action_ms` を更新することで、
  1500ms 以内の `idle_conv_check` では ObservedEisu 検出による IME OFF が再発しない
- SetOpen(true) → ObservedEisu → AssumedRomaji リセットのパスは
  「IME ON 要求が成功した後」にのみ走るため、無限ループにならない

### TsfNative 環境限定の理由

`idle_conv_check` で `conv_mode` を読めるのは TsfNative 環境（TSF API が利用可能な
Chrome / WezTerm 等）のみ。ImmCross 環境では `conv_mode` が取得できないため
`ObservedEisu` 状態自体が検出されない。

## 検討した代替案

### ObservedEisu を検出しても IME 操作はせず、engine を停止させるだけ

→ 採用しなかった。engine を停止させることでユーザーは「awase が動いていない」と
  感じ、混乱する。直接入力に切り替える（IME OFF にする）ほうが、
  OS の表示とユーザーの体験が一致する。

### ObservedEisu を「通常の AssumedRomaji 相当」として扱う

→ 採用しなかった。`AssumedRomaji` は「ローマ字入力として awase が動く」を意味するが、
  `ObservedEisu` は「IME ON なのに直接入力」という矛盾状態。
  同列に扱うと conv_mode の乖離を放置することになる。

## 結果

- IME ON 半角英数モードに陥っても 500ms 程度の idle conv check サイクルで自動復帰する
- SetOpen(true) 後の ObservedEisu stale 残留で engine が動かない症状が解消した
- カタカナ + shadow=OFF でエンジン復帰しないバグ（`0f75b5b`）も同一サイクルで修正

## 関連 ADR

- ADR-068: JISかな・カタカナモード対応（ObservedEisu の概念導入）
- ADR-032: IME 状態 Reducer 4 層モデル（InputModeState の遷移規則）
- ADR-038: DriftMonitor（shadow desync の継続的な検出・修復パターン）
