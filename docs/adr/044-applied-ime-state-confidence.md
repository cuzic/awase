# ADR-044: AppliedImeState と decide_kanji_apply — 保守性改善

## ステータス

採用済み（実装完了）

実装メモ: `decide_kanji_apply` は `kanji_needs_context_override` という名前で
`runtime/executor.rs` に実装。`AppliedImeState` は `state/ime_model.rs` に定義、
`state/mod.rs` から re-export。

## コンテキスト

### 問題 1: 確信度がセンチネル値に隠れている

`DecisionExecutor.applied_snapshot` は `Option<(bool, u64)>` 型だが、タプルの
第2要素（`at_ms`）に意味上の3状態が混在している。

```rust
None                 // フォーカス直後・起動時 — 実 IME 状態が完全に不明
Some((v, 0))         // 楽観更新 — ImmCross async が async 完了前に事前書き込み。未確認。
Some((v, ts))  ts>0  // 確認済み — 実 apply 完了後。信頼できる状態。
```

`ts == 0` はセンチネル値だが、その意味はコメントでしか表現されていない
（executor.rs:826–827, 835, 839）。このパターンを変更・追加するたびに
コメントとコードの対応を追い直す必要があり、混入リスクが高い。

### 問題 2: 6-C 判断ロジックが OS 呼び出しと混在して単体テスト不可能

executor.rs:818–857（約40行）は「KanjiToggle / GjiDirect を強制送信するか
スキップするか」の純粋な判断だが、OS 呼び出し（`current_tick_ms`、
`current_app_profile`、`gji_monitor_healthy` 等）と同じ関数内に書かれているため
単体テストが書けない。

判断に関わる条件の組み合わせは6軸あり、回帰テストなしに変更すると
デグレードのリスクが高い：

- EffectOrigin（EngineIntent か AutoApply か）
- IMM32 クロスプロセス可否（`can_use_imm32_cross_process`）
- GJI ヘルス（`gji_monitor_healthy`）
- 適用方向（ON / OFF）
- `AppliedImeState` の確信度（Unknown / Optimistic / Confirmed）
- 経過時間（300ms ウィンドウ）

## 決定

### 1. `AppliedImeState` enum を導入する

```rust
/// IME apply 結果の確信度。
/// `Option<(bool, u64)>` の暗黙のセンチネル値（ts=0）を型で置き換える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedImeState {
    /// フォーカス直後・起動時。実 IME 状態が不明。
    Unknown,
    /// ImmCross async の楽観的事前更新。OS ではまだ未確認。
    Optimistic(bool),
    /// 実 apply 完了・確認済み。`applied_at_ms > 0` に相当。
    Confirmed { open: bool, at_ms: u64 },
}
```

**遷移規則:**

```
起動 / フォーカス変更直後   → Unknown
ImmCross async 事前書き込み → Optimistic(open)
apply 完了（sync path）     → Confirmed { open, at_ms: now_ms }
apply 完了（async path）    → Confirmed { open, at_ms: now_ms }
mirror_applied_open(v)      → Confirmed { open: v, at_ms: now_ms }
mirror_applied_open_with_ts(v, 0) → Optimistic(v)   ← 従来の ts=0 パス
```

**効果:**

```rust
// Before: センチネル値の意味を追う必要がある
shadow_on == open && applied_at_ms > 0

// After: 型が意図を語る
matches!(state, AppliedImeState::Confirmed { open: s, .. } if *s == open)
```

```rust
// Before: ts=0 を "楽観更新" の意味で使う
self.applied_snapshot = Some((open, 0));

// After: 意図が名前に現れる
self.applied_state = AppliedImeState::Optimistic(open);
```

### 2. `decide_kanji_apply` 純粋関数を抽出する

executor.rs:818–857 のロジックを OS 呼び出しを含まない純粋関数に切り出す。

```rust
/// KanjiToggle / GjiDirect の apply を実行すべきか判断する。
/// OS 呼び出しを一切含まないため単体テスト可能。
///
/// workaround 6-C の実装: フォーカス変更直後の shadow desync 対策。
pub(crate) fn decide_kanji_apply(
    desired_open: bool,
    applied: AppliedImeState,
    shadow_on: bool,
    origin: EffectOrigin,
    can_imm32_cross_process: bool,
    gji_monitor_healthy: bool,
    now_ms: u64,
) -> KanjiApplyDecision

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KanjiApplyDecision {
    /// shadow が一致しており確認済みのためスキップ
    Skip,
    /// 通常 apply（apply_context をそのまま使う）
    Apply,
    /// desync の可能性があるため apply_context を !open に上書きして強制 apply
    ForceApply,
}
```

**配置場所:** `runtime/executor.rs` 内のモジュールプライベート関数
（または独立した `runtime/kanji_apply_policy.rs`）。

## 変更範囲

| ファイル | 変更内容 |
|---|---|
| `state/ime_model.rs` | `applied_open: Option<bool>` + `applied_at_ms: u64` → `applied: AppliedImeState` |
| `state/platform_state.rs` | `mirror_applied_open_with_ts` の更新、`applied_pair()` → `applied_state()` |
| `runtime/executor.rs` | `applied_snapshot` 型変更、40行ブロック → 関数呼び出し、~5箇所の `applied_at_ms > 0` パターン更新 |
| その他参照箇所 | `applied_pair()` の呼び出し（~3箇所） |

動作変更はない（純粋リファクタリング）。

## 期待するテストケース

| # | applied | origin | can_imm32 | gji | 期待値 | 説明 |
|---|---|---|---|---|---|---|
| 1 | Unknown | EngineIntent | false | false | ForceApply | フォーカス直後 desync（6-C 主ケース）|
| 2 | Optimistic(false) | EngineIntent | false | false | ForceApply | 楽観更新のみ = 不確定 |
| 3 | Confirmed{false,500} | EngineIntent | false | false | Skip | 確認済み OFF + 目標 OFF → 永続スキップ |
| 4 | Confirmed{false,800} | EngineIntent | false | false | Skip | 確認済み + 300ms 以内（now=1000）|
| 5 | Confirmed{false,500} | EngineIntent | false | false | ForceApply | 300ms 超過（now=1000）→ 再試行許容 |
| 6 | Unknown | EngineIntent | true | false | Apply | IMM32 使用可 → override 不要 |
| 7 | Unknown | EngineIntent | false | true | Apply | GJI 健全 → override 不要 |
| 8 | Unknown | AutoApply | false | false | Apply | EngineIntent でない → override 不要 |

## 実装順序

1. `AppliedImeState` enum を定義（`state/ime_model.rs`）
2. `ImeStateHub` / `platform_state.rs` の `mirror_applied_open_*` を更新
3. `executor.rs` の `applied_snapshot` を `AppliedImeState` に置き換え
4. `decide_kanji_apply` を抽出
5. unit test を追加

## 関連

- ADR-035: DecisionExecutor の純粋状態機械化（applied_snapshot 導入の経緯）
- ADR-032: IME 状態 reducer 4 層モデル
- workarounds.md: 6-C（shadow desync 強制送信）、6-E（物理 KANJI 迂回）
