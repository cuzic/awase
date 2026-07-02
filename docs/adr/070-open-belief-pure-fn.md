# ADR-070: `reduce_open_belief` — 観測値を純粋関数で単一ビリーフに還元する

## ステータス

採用済み（2026-07-01 実装、commit 1e09e90 / 8c34984）

## コンテキスト

### 問題: ad-hoc な IME apply 判定

IME を目標状態へ適用する際、「現在の IME 状態は何か」「本当に送信が必要か」を判断する
ロジックが `output/mod.rs` と `executor` に散在していた。

具体的には以下の boolean が各所で個別に評価されていた：

- `shadow_on`: 目標状態
- `candidate_visible` / `candidate_was_seen`: 候補ウィンドウの有無
- `conv_mode: Option<u32>`: OS から直接読み取った変換モード（あればより正確）
- `gji_monitor_ok`: GJI モニターが active かどうか
- `can_imm32_cross_process`: ImmCross が使えるか
- `applied.is_confirmed()`: 直前の apply が Confirmed だったか

これらを組み合わせて `already_matched`（今の状態と目標が一致→送信不要）を判定していたが、
`kanji_needs_context_override`（KanjiToggle 系で確信がない場合は強制 apply）のような
例外ロジックが別途分岐していた。同じ観測値セットが複数箇所で二重計算されていた。

### なぜ純粋関数か

- テスト可能性: 観測値 struct を渡すだけで任意のケースを単体テストできる
- SSOT: `OpenBeliefInputs` という型が「IME apply 判定に必要な観測値の全体」を明示する
- `executor` → `ime_apply_planner` のレイヤー境界: executor は観測値を収集・渡すだけ、
  判定は planner 内の純粋関数が行う

## 決定

`OpenBeliefInputs` 構造体と `reduce_open_belief` 純粋関数を `output/ime_apply_planner.rs` に追加する。

```rust
pub(crate) struct OpenBeliefInputs {
    pub shadow_on: bool,
    pub applied: AppliedImeState,
    pub candidate_visible: bool,
    pub candidate_was_seen: bool,
    pub gji_monitor_ok: bool,
    pub conv_mode: Option<u32>,
    pub can_imm32_cross_process: bool,
    pub is_engine_intent: bool,
    pub now_ms: u64,
}

pub(crate) struct OpenBelief {
    pub effective_open: bool,
    pub confident: bool,
}

pub(crate) fn reduce_open_belief(inputs: &OpenBeliefInputs, desired_open: bool) -> OpenBelief
```

### `effective_open` の計算規則

1. `conv_mode` が取得できた場合（ImmCross / TsfNative で直接読取り可能）を ground-truth とする
   - `desired_open=true` のとき: `conv & 0x0001 != 0`（`IME_CMODE_NATIVE` ビット）
   - `desired_open=false` のとき: `conv != 0`（DirectInput=0 以外は IME ON とみなす）
2. `conv_mode` が None の場合は shadow + candidate 観測で推定する
   ```
   shadow_on || candidate_visible || (!desired_open && candidate_was_seen)
   ```

### `confident` の計算規則

`confident=false` は「`already_matched` を強制 false」＝「必ず apply する」を意味する。

KanjiToggle 系（Chrome / TsfNative で ImmCross も GJI も使えない環境）でのみ
`confident=false` になり得る。条件：

```
is_engine_intent
    && !can_imm32_cross_process
    && !gji_monitor_ok
    && conv_mode.is_none()
    && !(shadow_on == desired_open
         && applied.is_confirmed()
         && (now_ms - applied.confirmed_at_ms()) < 300)
```

つまり「300ms 以内に Confirmed な apply があり shadow と目標が一致」している場合のみ
確信あり。それ以外は必ず送信する（トグルキーが状態不明なため冪等性がない）。

### 旧 `kanji_needs_context_override` との比較

旧実装では `kanji_needs_context_override()` が `confident=false` 相当の役割を担っていたが、
`executor` 側に散在し `ImeApplyContext::from_view` が事後的に上書きしていた。
`reduce_open_belief` に統合することで判定の責務が一箇所に集約された。

### 命名の変遷

実装当初は `ApplyBelief` / `reduce_apply_belief` という名前だったが、
「"apply" は動詞・動作のニュアンスが強く、"open" のほうが状態を表す」として
`OpenBelief` / `reduce_open_belief` に改名した（commit 8c34984）。

## 検討した代替案

### 既存の `ImeApplyContext::from_view` にロジックを押し込む

→ 採用しなかった。`from_view` はすでに多数のフィールドを受け取っており、
  ここにさらに判定ロジックを足すと God Object になる。純粋関数に分離することで
  `ImeApplyContext` は「計画データの保持」、`reduce_open_belief` は「観測値の還元」と
  責務が明確になる。

### `effective_open` を省略して `shadow_on` のみ使う

→ 採用しなかった。`candidate_was_seen=true` の候補ウィンドウ経由で IME が ON に
  なっているケースや、`conv_mode` が `shadow` と乖離しているケースで誤適用が生じた
  実績があり、多観測値の統合が必要。

## 関連 ADR

- ADR-035: Decision/Executor 純粋ステートマシン（executor が観測値を収集する役割の根拠）
- ADR-069: 凝集性リファクタ H-5-d（`ImeApplyPlanner` の新設）
- ADR-044: AppliedImeState の確信度（`applied.is_confirmed()` の意味）
