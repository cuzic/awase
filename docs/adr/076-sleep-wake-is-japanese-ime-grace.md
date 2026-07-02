# ADR-076: スリープ復帰後 is_japanese_ime 一時 false — grace 保護

## ステータス

採用済み（2026-07-02 実装）

## コンテキスト

### 症状

PC がスリープから復帰した直後に最初のキーを押すと、awase エンジンが
`NotJapaneseIme` 判定で非アクティブ化し、TsfNative アプリ（Windows Terminal 等）で
IME が OFF に固定される。

ユーザーは `conv=0x00000001`（日本語 IME がアクティブ）を確認できるが、
エンジンは PassThrough のまま復帰せず、`is_japanese_ime=false` が継続する。

### 原因

スリープ復帰後、`read_ime_state_fast`（フォーカスプローブの高速 IME 読み取り）は
一時的に `is_japanese_ime=false` を返すことがある。これは OS の IME 状態が
まだ完全に復元されていないためであり、トランジェントな誤読みである。

修正前の `apply_focus_probe` では `set_is_japanese_ime` が **grace チェックより前**
に無条件実行されていた:

```rust
// ← BUG: grace を計算する前に is_japanese_ime を上書き
self.platform_state.ime.set_is_japanese_ime(probe.is_japanese_ime);

let signals = compute_focus_probe_grace(...);  // shadow grace = active
// signals.any() は true → imc_open=false を抑制できるが、is_japanese_ime は保護できない
```

`imc_open=false` は `signals.any()` によって抑制されるが、`is_japanese_ime`
はすでに `false` に設定された後であるため保護されない。

### 波及効果

1. `is_japanese_ime=false` が確定 → 次のキー入力で `build_ctx()` が `NotJapaneseIme` を返す
2. エンジンが非アクティブ化 → `SetOpenRequest(false)` → `desired_open=false`, VK_IME_OFF 送信
3. `is_japanese_ime` を `true` に戻す機構がない（`idle-conv-check` は `conv` を読むが
   `is_japanese_ime` は更新しない）
4. ユーザーは永続的に IME OFF 状態に固定される

## 決定

`apply_focus_probe` 内で `compute_focus_probe_grace` を `set_is_japanese_ime` より
**前**に計算し、grace active 中は `false` へのダウングレードを抑制する:

```rust
// signals を先に計算
let signals = compute_focus_probe_grace(
    now_ms, probe_age_ms, warmup_ms, gji_last_io_ms, last_focus_change_ms, shadow_on,
);

// grace active 中は is_japanese_ime の false ダウングレードを抑制
// （true への更新はいつでも許可）
if probe.is_japanese_ime || !signals.any() {
    self.platform_state.ime.set_is_japanese_ime(probe.is_japanese_ime);
}
```

### なぜ `true` 側は無条件に更新するか

- `false → false`: grace なし → 更新
- `false → false`: grace あり → スキップ（保護）
- `true → true`: 無条件更新（問題なし）
- `true → false`: grace なし → 更新（本当に非日本語 IME に変わった場合）
- `true → false`: grace あり → **スキップ**（スリープ復帰後の誤読み保護）

grace (shadow sig3) が active になるのは `probe_age_ms` が極端に小さい場合（0ms 付近）—
スリープ復帰後の最初の fast probe が該当する。

### imc_open との対称性

`imc_open=false` と `is_japanese_ime=false` は同じ原因で誤る。
修正前は `imc_open` のみが grace で保護されており、`is_japanese_ime` だけが
保護されない非対称な状態だった。今回の修正で対称化される。

## 関連 ADR

- ADR-075: ImmCrossProbe による belief 補正（shadow grace / FocusProbe confidence の設計）
- ADR-032: IME 状態モデルの4階層 reducer アーキテクチャ（`is_japanese_ime` の役割）
