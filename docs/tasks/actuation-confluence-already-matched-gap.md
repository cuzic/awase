# imm_cross_write の AlreadyMatched 判定にテストの穴がある（要修正）

状態: 対応済み（2026-09-22起票、PR [#247](https://github.com/cuzic/awase/pull/247)でテスト追加）
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

`imm_cross_reobservation_already_matches`として`state/ime_actuation_decision.rs`
（windows-ungated）へ判定を抽出し、一致/不一致/未知(`None`)の3ケースを
ユニットテストで固定した（`cargo test -p awase-windows --lib`でLinux上でも
検証可能）。残作業: マージ後に`gh workflow run
mutants-actuation-confluence-windows.yml --ref develop`を再実行し、
`open_chain.rs:356`相当のmutantが`missed`→`caught`に変わったことを確認する
（下記「やること」3番）。

## 背景

IME actuation 合流点（[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)
の「IME actuation 合流点」表）のうち現存する4関数

- `ImeController::apply`（`crates/awase-windows/src/ime_controller.rs`）
- `run_open_chain_async` / `fallback_write` / `imm_cross_write`（`crates/awase-windows/src/runtime/open_chain.rs`）
- `DecisionExecutor::dispatch_ime_set_open`（`crates/awase-windows/src/runtime/executor.rs`）

に対して、「片方の欠陥をもう片方の冗長経路が隠す」問題の棚卸しとして cargo-mutants を
回した（`.github/workflows/mutants-actuation-confluence-windows.yml`、GitHub-hosted
`windows-latest`、対象13 mutants）。2026-09-22 の実行（run
[35816447154](https://github.com/cuzic/awase/actions/runs/35816447154)）で3件の
mutant が生存（既存テストで検出できず）した:

1. `ime_controller.rs:640`（`ImeController::apply`）— `if outcome == Failed { warn!(...) }`
2. `executor.rs:1055`（`dispatch_ime_set_open`）— `if outcome == Failed { warn!(...) }`
3. `open_chain.rs:356`（`imm_cross_write`）— `if actual == Some(open) { AlreadyMatched } else { .. }`

1と2は**戻り値に一切影響しないログ専用分岐**（等価変異体、`.cargo/mutants.toml`が
既に別件で除外登録しているパターンと同型）で、新しい問題ではない。**3だけが本物の穴**。

## 問題の分岐（`open_chain.rs::imm_cross_write`、348〜365行付近）

```rust
ActuationOutcome::Failed => {
    // SAFETY: `read_ime_state_fast` は Win32 IMM API を呼ぶ。
    let actual = unsafe { crate::ime::read_ime_state_fast() }.ime_on;
    post_failed_reobservation = Some(actual);
    if actual == Some(open) {
        // ImmCross の書き込みは失敗と報告されたが、実際は既に desired と一致
        ImeOpenOutcome::AlreadyMatched
    } else {
        // フォールバックへ進む
        ...
    }
}
```

ImmCross の書き込みが `Failed` を返した直後、Win32 を直接読み直して実際の IME 状態が
既に望む状態と一致しているかを判定している。この `==` を `!=` に反転させても、
既存テスト（golden / architecture_guard / journal_replay 等）は誰も気づかない。

反転した場合の実害:
- 「一致しているのに一致していないと誤判定」→ 不要な `fallback_write` 呼び出し →
  ADR-149（半角状態の物理IMEキー単独タップでVK_IME_ONが3回重複送信）と同種の
  **冗長VK送信**が起きうる。
- 「一致していないのに一致していると誤判定」→ 必要な `fallback_write` をスキップ →
  実際にはOSがまだ desired 状態でないのに `AlreadyMatched` を返す欠落。

近い過去のインシデント（BUG-113追補、`architecture_guard.rs::
fallback_write_bypasses_gji_shadow_on_via_none_override`）は「`fallback_write`内部の
古いshadow値によるAlreadyMatched誤判定」を扱っており、こちらは`shadow_on = None`で
bypassする設計になっている。**今回の`imm_cross_write:356`はそれとは別物**——shadow値
ではなく`read_ime_state_fast()`によるその場のWin32直接再読み取りなので、bypass設計の
対象外であり、素で未テストのまま残っている。

## やること

[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)の
「IME actuation合流点」は再発ファミリー対象なので、fixには (a) 回帰テスト か
(b) `docs/known-bugs/BUG-NNN.md` のどちらかが必須。

1. **(a) を優先して検討する**: `imm_cross_write`の`ActuationOutcome::Failed`分岐を
   直接テストできる場所を探す/作る。既存の journal replay 基盤
   （`journal.rs`起点、[fix-requires-evidence.mdのテストの置き場所節](../../.claude/rules/fix-requires-evidence.md#テストの置き場所このリポジトリの既存資産)参照）や
   `tests/golden/`が使えないか検討する。「ImmCross書き込みFailed + 直後の
   `read_ime_state_fast()`がdesiredと一致/不一致」という2ケースを、それぞれ
   `AlreadyMatched`/フォールバック続行という異なる`ImeOpenOutcome`で区別して
   固定する。
2. Win32実機依存で(a)が組めない場合は(b): `docs/known-bugs/BUG-NNN.md`
   （次の連番、[docs/known-bugs/index.md](../../docs/known-bugs/index.md)参照）に
   症状・再現手順・本ドキュメントへのリンクを記録する（本文目安30行以内、
   [docs-frontmatter-convention.md](../../.claude/rules/docs-frontmatter-convention.md)参照）。
3. 修正後、`gh workflow run mutants-actuation-confluence-windows.yml --ref develop`
   を再実行し、`open_chain.rs:356`のmutantが`missed`→`caught`に変わったことを確認する。

## 補足（このタスクの範囲外）

`fallback_write`と`run_open_chain_async`は、今回生成された変異（戻り値まるごと
Default::default()への差し替え）がいずれも型に`Default`未実装でビルド不能
（`unviable`）となり、有効なシグナルが得られなかった。この2関数のガード条件が
実際どれだけテストされているかは今回の実行だけでは分からない——「クリーン」と
誤解しないこと。別の変異戦略（`examine_re`を関数内の演算子レベルに絞る、
`mutate_traits`調整等）が要るかもしれないが、これは本タスクの範囲外。

## 関連

- ワークフロー: `.github/workflows/mutants-actuation-confluence-windows.yml`
- スコープ設定: `.cargo/mutants-actuation-confluence-scope.toml`
- 実行結果アーティファクト: `gh run download 35816447154 --repo cuzic/awase -n mutants-report-actuation-confluence`
  （GitHub Actionsのartifact保存期限切れで取得できない場合は、上記コマンドで
  ワークフローを再実行すれば同じ結果が再現できるはず）
- [docs/tasks/actuation-confluence-inventory.md](actuation-confluence-inventory.md) —
  同じ調査から派生したもう一方のタスク（合流点の統合候補棚卸し）
