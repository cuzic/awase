# ADR-075: ImmCrossProbe による belief 補正 — Qt/GJI フォーカス時の IME 誤認識修正

## ステータス

採用済み（2026-07-02 実装）

## コンテキスト

### 症状

GJI（Google 日本語入力）使用中の LINE（Qt アプリ）にフォーカスを当てると、
awase エンジンが Engine OFF（ローマ字直接入力モード）になる不具合が報告された。

### 根本原因

`read_ime_state_fast` は `GetForegroundWindow()` で得た **top-level hwnd** の
`IMC_GETOPENSTATUS` を読む。しかし Qt アプリは各子ウィジェットが独立した
IMM32 コンテキストを持つため、top-level hwnd の状態は `false` のまま、
実際の入力フォーカスを持つ子 hwnd は `true`（GJI がアクティブ）という乖離が生じる。

`FocusProbe` の `ObservationConfidence::Medium` によって `derive_open()` が
`false` を正しい値と判断する設計上の穴があった。また、`effective_open()` が
観測プールを一切参照せず `desired_open` のみを返していたため、
後続の `ObserverPoll`（正しく `true` を返す）で修正できなかった。

### 設計の欠落

`ObservationStore` には `most_recent_trusted()`・`consensus()` メソッドが
実装されていたが、`derive_open()` が存在せず、`effective_open()` は
観測を無視して `desired_open` を返すだけだった。

## 決定

### 変更 1: `ObservationStore::derive_open()` 純粋決定関数を追加

観測プールから IME 開閉 belief を導出する純粋関数。判定順序:

1. **High confidence** — 単一ソースで即採用（`ImmCrossProbe`, `ImmGetOpenStatus`）
2. **Medium+ ソースの無競合多数決** — 1 ソース以上が一方向に揃えば採用
3. 競合または観測なし → `None`（呼び出し側が `desired_open` にフォールバック）

鮮度ウィンドウ: 3000ms を超えた観測は無視。

### 変更 2: `ImeModel::effective_open()` を観測 derived belief に変更

```rust
pub fn effective_open(&self) -> bool {
    let base = if self.has_user_explicit_intent() {
        self.desired_open
    } else {
        self.observations.derive_open(Instant::now()).unwrap_or(self.desired_open)
    };
    self.force_guards.effective_open(base)
}
```

- **ユーザー明示意図あり**（SyncKey / PhysicalImeKey / Command / Recovery）:
  `desired_open` を優先（観測で上書きしない）
- **ユーザー明示意図なし**（フォーカス変化直後、HwndCache 復元等）:
  `derive_open()` を使い、観測がなければ `desired_open` にフォールバック
- `HwndCache` 由来の intent は「明示意図なし」扱い（観測が優先）

### 変更 3: `FocusProbe` confidence を `Low` に下げる

`write_focus_probe()` の `ObservationConfidence::Medium` → `Low`。

`derive_open()` のステップ 2（Medium+ 無競合多数決）に `FocusProbe` が
参入しなくなり、top-level hwnd の誤読み取りが belief に影響しない。

### 変更 4: `ObservationSource::ImmCrossProbe` を追加（High confidence）

Qt/LINE 等の ImmCross アプリで、フォーカス変更直後に
`read_ime_state_full_async`（child hwnd + ImmCross 経由の正確な読み取り）を
非同期実行し、結果を `High confidence` 観測として記録する。

```rust
// focus_tracking.rs: on_focus_process_changed 末尾
if AppImeProfile::ImmCross && is_japanese_ime {
    win32_async::spawn_local(async move {
        let snap = read_ime_state_full_async().await;
        if let Some(open) = snap.ime_on {
            write_imm_cross_probe(open, tick_ms);  // High confidence
        }
    });
}

// key_pipeline.rs: apply_focus_probe 末尾（first-key トリガー、2 回目の補強）
if AppImeProfile::ImmCross && is_japanese_ime {
    win32_async::spawn_local(async move { ... });
}
```

### トリガーを 2 箇所設ける理由

| トリガー | タイミング | 役割 |
|---------|-----------|------|
| `on_focus_process_changed` | フォーカス変更直後 | 最初のキー入力前に確定させる（主要パス）|
| `apply_focus_probe` | first-key の async probe 完了後 | `on_focus_process_changed` の probe が失敗した場合の補強 |

## 修正フロー

**修正前（GJI + LINE フォーカス時の失敗パス）**:

1. HwndCache が `ime_on = false` を復元 → `desired_open = false`
2. FocusProbe: top-level hwnd → `false`（誤）
3. `effective_open()` = `desired_open = false` → Engine OFF
4. OsPoll が `true` を返しても `desired_open` を上書きできず放置

**修正後**:

1. HwndCache が `false` を復元 → `desired_open = false`, `last_intent = HwndCache`
2. `on_focus_process_changed` 末尾で ImmCrossProbe を非同期起動
3. ImmCrossProbe が child hwnd → `true` を返す → `ImmCrossProbe { open: true, confidence: High }` 記録
4. 最初のキー入力時: `has_user_explicit_intent() = false`（HwndCache は明示意図なし扱い）
   → `derive_open()` → High confidence `true` → `effective_open() = true` → Engine ON ✓

## 設計原則との整合

- **Observer は `desired_open` を直接書き換えない** — 変わらず維持。
  `write_imm_cross_probe` は `ObserverReported` を dispatch するだけ。
- **Observation を集めたうえで pure に判断して belief に反映** — `derive_open()` が
  この純粋決定の役割を担い、`effective_open()` から呼ばれる。
- **UserIntent が最優先** — `has_user_explicit_intent()` が true なら `desired_open` を使う。

## 関連 ADR

- ADR-032: IME 状態 Reducer 4 層モデル（`ImeModel` の設計）
- ADR-033: AppImeProfile（ImmCross / TsfNative / Imm32Unavailable の分類）
- ADR-038: DriftMonitor（shadow desync の継続的な検出・修復パターン）
- ADR-074: ObservedEisu 自動直接入力切替（ImmCross 環境での conv_mode 非検出）
