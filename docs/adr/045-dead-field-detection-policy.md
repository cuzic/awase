# ADR-045: Dead Field 検出方針とプレースホルダーフィールド禁止原則

## ステータス

採用済み（2026-06-04 実施）

## コンテキスト

### 段階的リファクタリングが生む dead field の蓄積

ADR-040（段階的遷移戦略）に沿って IME 状態モデルを Step 1〜7 に分けて
リファクタリングしてきた。この方式は安全性が高い反面、
「書き込む側だけ先に実装し、読み取る側は次ステップ以降」という状態が
一定期間続くため、write-only フィールドが蓄積しやすい。

git log を遡ると、dead code 撤去が繰り返し発生していることが確認できる：

| コミット | 撤去した dead code |
|---|---|
| `ac1bbd9` | 試行錯誤で蓄積した不要コード |
| `80f17fc` | `last_explicit_intent` フィールド（SSOT 集約後の残骸）|
| `0b61a32` | `LastAppliedImeState`（`CompositionState` に統合後の残骸）|
| `ac76f88` | `applied_open` ラッチ（executor へ責務移転後の残骸）|
| `066b70c` | `ImeRecoveryState`（`force_guards` + `drift_monitor` に統合後）|
| `dcf67fb` | `ImeTransition.optimistic_applied`（常に `false`、未参照）|
| `857574e` | `HookRoutingState`（空 struct）、`hook_config` dead field |
| `fcbcd8a` | `ImeModel.reduce_count`（診断用フィールドが未接続のまま）|
| `a42fe96` | `applied_*` 薄い委譲5本（移譲後の呼び出し元が存在しなかった）|

そして 2026-06-04 に今回のセッションで以下を一括撤去した：

- `RecordedIntent.at_seq` — `at_ms` は使われるが `at_seq` は書かれるだけ
- `ImeObservation.recorded_seq` — 書かれるだけ、参照なし
- `ImeDrift.desired / .observed / .first_drift_seq` — `drift_duration()` は `started_at` しか使わない
- `ObservationStore.suspicious` + `record_suspicious()` — 外部から一度も呼ばれない
- `AppImePolicy.observer_false_on_focus` — 対応する reducer ロジックが未実装
- `AppImePolicy.observer_poll_role` + `ObserverPollRole` enum — 全 observer が HealthChecker に統一済み
- `ImeTransition.requested_at` — `timeout_at` が既に計算済みで冗長
- `ImeTransition.actuator` — リトライ機構が未実装、`AppImePolicy.actuator_kind` から取得可能
- `WarmthContext.warm / .session_expired` — 同名のローカル変数と混同、struct field は未参照
- `ObservationFalsePolicy` enum — フィールド削除に伴い参照箇所消滅

### なぜ Rust の lint が検出できないか

Rust の `dead_code` lint には構造的な死角が二つある：

**1. pub フィールドはライブラリクレートで検査対象外**

```rust
// lib クレートの pub struct は「外部クレートが読むかもしれない」と
// コンパイラが保守的に扱うため、dead_code 警告が出ない。
pub struct ImeObservation {
    pub recorded_seq: u64,  // ← どこからも .recorded_seq は呼ばれない。が、警告なし。
}
```

**2. `#![cfg(windows)]` クレートは Linux CI で完全に透明**

```rust
// awase-windows/src/lib.rs の先頭
#![cfg(windows)]
```

Linux（CI 環境含む）では `awase-windows` クレート全体が空クレートとして
コンパイルされる。そのため dead field も broken test も全て素通りする。
実際、今回のセッションで `platform_state.rs` のテストが存在しない
メソッド `ps.ime_on()` / `ps.reset_stale_ime_on_for_tsf_native()` を
`PlatformState` に対して呼んでいることが判明した。
Windows CI が動いていれば即座にコンパイルエラーになっていた。

### write-only フィールドは「書き込み側だけが先行した」証拠

特に問題になるのは「常に `field_name: value` と書かれるが、
`.field_name` としては一度も読まれない」パターンである：

```rust
// struct 構築時に設定される
ImeTransition { requested_at: envelope.time.monotonic, ... }

// しかし is_timed_out() は timeout_at しか使わない — requested_at は読まれない
pub fn is_timed_out(&self, now: Instant) -> bool {
    now >= self.timeout_at
}
```

このパターンはコードを読むだけでは発見が難しい。
「書き込む側が先」は設計上の意図に見えるからである。

## 決定

### 1. `scripts/scan_dead_fields.py` をメンテナンスツールとして常備する

コーパス1回走査（O(n)）で全 struct フィールドの読み取り数を集計し、
`.field_name` が0件のフィールドを報告する。

**アルゴリズム:**

```
1. 全 .rs ファイルを収集（target/, .claude/, .worktree/ 等を除外）
2. コメント・文字列リテラルをブランク化（ライフタイム 'a は保持）
3. 単一正規表現で corpus を走査:
   - \.identifier  → dot_reads[identifier]++   (読み取り)
   - identifier:   → colon_writes[identifier]++ (struct 定義 or リテラル書き込み)
4. 全 struct フィールドを抽出し、dot_reads[field_name] == 0 を報告
```

**実行方法:**

```bash
python3 scripts/scan_dead_fields.py
python3 scripts/scan_dead_fields.py /path/to/project
```

**既知の限界（false positive / false negative 源）:**

| 種別 | 内容 |
|---|---|
| FP | pub フィールドを持つ lib クレート — 外部クレートからの読み取りは検出不可 |
| FP | `let S { field: var } = x` 形式の destructuring — `field:` が write としてカウントされる |
| FP | serde / derive マクロによるフィールド参照 |
| FN | `let S { field }` shorthand destructuring — `.field` も `field:` も現れない |
| FN | メソッド名がフィールド名と一致する場合 |

### 2. プレースホルダーフィールドを禁止する

「将来のステップで読む予定」というフィールドを事前に追加しない。

**禁止:**

```rust
// NG: 読む側が未実装なのにフィールドだけ追加する
pub struct AppImePolicy {
    pub observer_false_on_focus: ObservationFalsePolicy, // Step 2B で使う予定
}
```

**許可:**

```rust
// OK: 読む側が実装されてから同時にフィールドも追加する
// OK: RAII のために保持するフィールドは #[allow(dead_code)] + コメントで明示する
#[allow(dead_code)] // RAII: Drop で OUTPUT_GATE.active=false を実行する
pub guard: OutputActiveGuard,
```

**理由:**

1. 「書き込む側が先」は static analysis では設計意図に見える
2. 読む側が実装されるまでの間、フィールドはコードを読む人を誤解させる
3. 将来の実装時に git log から意図を復元できる（コミットメッセージが設計資料になる）

### 3. Windows CI を整備して `#![cfg(windows)]` コードを継続的に検証する

Linux CI だけでは `awase-windows` の dead field も broken test も検出できない。
Windows 環境での `cargo test` がない期間、構造的な問題が蓄積し続けた。

Windows CI（GitHub Actions `windows-latest`）で最低限以下を実行する：

```yaml
- run: cargo check --workspace
- run: cargo test --workspace
```

## 実施した変更（2026-06-04）

### dead field 撤去

| ファイル | 撤去したフィールド / 型 |
|---|---|
| `state/ime_model.rs` | `RecordedIntent.at_seq` |
| `state/observation_store.rs` | `ImeObservation.recorded_seq` |
| `state/observation_store.rs` | `ImeDrift.desired / .observed / .first_drift_seq` → `started_at` のみに縮小 |
| `state/observation_store.rs` | `ObservationStore.suspicious` + `record_suspicious()` + `SUSPICIOUS_CAPACITY` |
| `state/observation_store.rs` | `update_drift()` の `seq: u64` 引数 |
| `state/app_ime_policy.rs` | `AppImePolicy.observer_false_on_focus` / `.observer_poll_role` |
| `state/app_ime_policy.rs` | `ObservationFalsePolicy` enum / `ObserverPollRole` enum |
| `state/transition.rs` | `ImeTransition.requested_at` / `.actuator` + `use ImeActuatorKind` |
| `state/ime_model.rs` | `ImeTransition` 構築時の `requested_at:` / `actuator:` フィールド |
| `output/mod.rs` | `WarmthContext.warm` / `.session_expired` |

### broken test 修正

`platform_state.rs` テストが `PlatformState` に存在しないメソッドを呼んでいた
（`#![cfg(windows)]` のため Linux では長期間未検出）：

```rust
// Before — PlatformState にこのメソッドは存在しない
ps.reset_stale_ime_on_for_tsf_native();
assert!(!ps.ime_on());

// After — ImeStateHub 経由で正しく呼ぶ
ps.ime.reset_stale_ime_on_for_tsf_native();
assert!(!ps.ime.effective_open());
```

### その他の dead code 修正（同セッション）

- `awase-macos/src/output.rs` — `KeyAction::KeySequence` match arm 欠落（コンパイルエラー）
- `tests/scenarios.rs` — `fn key_up()` 未使用、`VK_CONVERT` の不要 `#[allow(dead_code)]`
- `tests/e2e_windows.rs` — `fn send_keydown_to_edit()` 未使用
- `focus/uia.rs` — `#[allow(unused_variables)]` → `_hwnd` 命名規則に変更

## 関連

- ADR-040: 段階的リファクタリング戦略（dead field 蓄積の根本原因）
- ADR-032: IME 状態 reducer 4 層モデル（多くの dead field が生まれた refactor の中心）
- `scripts/scan_dead_fields.py`: 実装されたスキャナースクリプト
- `docs/layer-boundaries.md`: grep audit による設計原則チェック（同様の継続的検証アプローチ）
