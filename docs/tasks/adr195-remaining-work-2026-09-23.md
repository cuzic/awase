# ADR-195: PR #250〜#258 develop統合と、レビューで見送った低優先度指摘の残作業

状態: 未着手（2026-09-23起票）。本ドキュメントは、2026-09-23セッションで
PR #250〜#258（ADR-195 T0/T2/T3/T4/T5/T6/T8/T9）を横断レビュー・修正した後に残った
作業をまとめたもの。実バグは全てpush済みだが、**PRはまだ1件もdevelopへマージされて
いない**。着手時は `.claude/rules/worktree-per-session.md` に従い専用worktree/branchを
切ること。

## 1. develop統合待ちのPR一覧と依存順序

2026-09-23時点でopenの全PRとbase branch（`gh pr list`の実測、下記は`git log <branch> -1`の
最新コミット）:

| PR | branch | base | 最新コミット |
| --- | --- | --- | --- |
| #261 | docs/adr-197-msime-legacy-custom-keymap-support | develop | （ADR-195と無関係、別系統） |
| #260 | feat/adr196-t3-bundled-table-versioning | develop | （ADR-196系、本ドキュメント対象外） |
| #259 | feat/adr196-t1-external-write-observation | develop | （ADR-196系、本ドキュメント対象外） |
| #258 | feat/adr195-t9-learning-output-binding | feat/awase-calibration | `2e5df7a1` |
| #257 | feat/adr195-t0-config-reading-integration | develop | `41633fe1` |
| #256 | feat/adr195-t4-runtime-loading | feat/adr195-t3-persistence | `74ed295e` |
| #255 | feat/adr195-t6-wizard-integration | feat/awase-calibration | `3fd54c45` |
| #253 | feat/adr195-t8-staleness-detection | feat/adr195-t3-persistence | `13c5b0f4` |
| #252 | feat/adr195-t5-mealy-minimization | feat/adr195-t2-self-verification | `929061f4` |
| #251 | feat/adr195-t3-persistence | feat/awase-calibration | `b9ae3af9` |
| #250 | feat/adr195-t2-self-verification | feat/awase-calibration | `23d80b02` |

土台の`feat/awase-calibration`（前提タスク、[adr195-t-rebase-calibration-branch.md](adr195-t-rebase-calibration-branch.md)）
は`3165d1d3`。

依存グラフ（GitHubのbase branchそのまま。T9は実際には T2+T3/T8+T6 を統合したブランチだが、
PRのbaseは`feat/awase-calibration`のまま）:

```
develop
 ├─ feat/adr195-t0-config-reading-integration (#257)  … developから独立、他への依存なし
 └─ feat/awase-calibration (前提タスク)
     ├─ feat/adr195-t2-self-verification (#250)
     │   └─ feat/adr195-t5-mealy-minimization (#252)
     ├─ feat/adr195-t3-persistence (#251)
     │   ├─ feat/adr195-t8-staleness-detection (#253)
     │   └─ feat/adr195-t4-runtime-loading (#256)
     ├─ feat/adr195-t6-wizard-integration (#255)
     └─ feat/adr195-t9-learning-output-binding (#258)  … T2+T3/T8+T6を統合済み
```

マージは依存の葉から順に、develop 1本化に向けてPRをまとめていく必要がある
（T9が実質的にT2/T3/T8/T6を包含しているため、develop統合の実務上は「T0を先にdevelopへ、
その後T9を軸に統合する」形が現実的——T9単体でdevelopへマージすると、T2/T3/T8/T6の
個別PRとの間で二重マージや解決済みコンフリクトの再発が起きうるため、統合順序は着手時に
再検討すること）。

## 2. 見送った低優先度レビュー指摘（実害なし・設計ノート）

2026-09-23の並列レビュー（code-reviewスキル×8PR、opus系ではなく通常レビュー）で
検出したが、正しさに関わるバグではなく設計上の重複・簡素化余地の指摘のため、
今回のセッションでは意図的に対応を見送った。次にこれらのファイルへ触れるセッションが
拾うか、まとめて着手するかは着手時に判断する。

### PR #250（feat/adr195-t2-self-verification、`crates/awase-keymap-learn/src/verify.rs`）

- `Class::Single`（観測1件のセル）が`classify_robust`で「完全に確信できる」扱いになっている。
  観測1件では観測誤りの可能性を排除できないため、`min_minority`の考え方からすると
  本来はリトライ対象になるべきという指摘。
- `min_minority == 0`のエッジケースの挙動が未検証。
- `classify_robust`（`verify.rs`）と`Table::majority`/`Table::class`（`table.rs`）で
  多数決タイブレークのロジックが重複している（後述、PR #258レビューでも再検出）。

### PR #251（feat/adr195-t3-persistence、`crates/awase-keymap-learn/src/persist.rs`）

- `from_json`が「1件でも不正なセルがあればファイル全体を拒否」する設計になっており、
  T4側のドキュメントが謳う「行単位の寛容さ」の主張と食い違う（アーキテクチャ上のギャップ、
  現状T4は該当パスを実際には使っていないため実害なし）。
- 派生`PartialEq`が`Vec<PersistedCell>`の順序に依存する（現状どの呼び出し元も順序に
  依存していないため無害だが、将来のバグの種）。
- pretty-print出力のアロケーション効率、`const fn`が実質不要、テスト本体の重複。
- `schema_version`の不一致判定が「二値の拒否」のみで、ADR196-T5が導入する
  「要再検証」段階的判定へは未対応（[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
  側のスコープ、本PR側の対応は不要）。
- golden fixtureとしてのJSON固定テストが無い（構造体からのラウンドトリップテストのみ）。

### PR #252（feat/adr195-t5-mealy-minimization、`crates/awase-keymap-learn/src/minimize.rs`）

- 決定性チェックの重複、不要なアロケーション、変数名`m`のシャドーイングなど、
  小さなクリーンアップ余地のみ（正しさは実測トレースで確認済み）。

### PR #255（feat/adr195-t6-wizard-integration、`crates/awase-settings/src/main.rs`）

- `keymap_learn_rx`/`keymap_learn_progress`/`keymap_learn_status`/`keymap_learn_child`の
  4つの`Option`フィールドを、状態を表すenum 1本にまとめられるのではという指摘。
- `Arc<Mutex<Child>>`が本当に必要か（単一スレッドからのアクセスに絞れないか）の再検証提案。
- `should_recommend_learning`が`#[allow(dead_code)]`のまま未配線（T4の「同梱表との
  セル突き合わせ」判定が実装されてから配線する設計、意図的な未配線であり要修正ではない）。
- `stdout().flush()`の呼び出しが冗長な箇所がある。

### PR #258（feat/adr195-t9-learning-output-binding）横断レビューで再検出した重複

- `verify::classify_robust`（`verify.rs:53`付近）と`Table::majority`（`table.rs:94`）が、
  「同数タイなら先に現れた方を採用する」多数決ロジックを独立に実装している。
  コード中のコメントで「table.rs::majority()と同じ規則」と明記されているが、
  共有関数化はされていない——将来どちらか一方だけ規則を変更すると気づかれずにdriftする。
  入力の形（`Vec<Outcome>` vs `Vec<Observation>`・スコープ（グループ内 vs セル全体）が
  異なるため、共有化には`impl Iterator<Item = Outcome>`受け取りへの一般化など
  多少の設計判断が要る。
- `ResultLineArgs`（`crates/awase-keymap-learn-win/src/main.rs`）が呼び出し元1箇所のみの
  9フィールド構造体で、`#[allow(clippy::too_many_arguments)]`1行で足りる場面に
  構造体を導入している。既存コメントでclippy対策と明記されており、対応不要と判断。

## 3. T10（RealImeDriver実機観測不良）の残作業

[adr195-t10-realimedriver-ci-observation-failure.md](adr195-t10-realimedriver-ci-observation-failure.md)
に記載の4方向の究明（フォアグラウンド確保ロジックの可視化、`settle_setup`/`observe_imm`の
失敗理由の可視化、GitHub-hosted runner固有の環境差の切り分け、dragonflyg4実機での再検証）が
未着手のまま。run [35840828329](https://github.com/cuzic/awase/actions/runs/35840828329)
（待機時間延長版）の完了を2026-09-23に確認済みだが、結果はT10記載のものと完全一致
（presses=0, cells=0）で新たな手がかりは得られていない。

## 完了条件

- 上記PR群が依存順にdevelopへ統合される（コンフリクト解消・CI green・/code-review通過）。
- 2節の指摘のうち着手したものは、対応してPRへ追随コミットするか、見送りと判断した理由を
  このファイルへ追記する。
- T10の究明が1つでも進展したら、T10ファイル自体を更新する（本ファイルではなくT10側に書く）。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md)
- [adr195-t10-realimedriver-ci-observation-failure.md](adr195-t10-realimedriver-ci-observation-failure.md)
- [adr195-t-rebase-calibration-branch.md](adr195-t-rebase-calibration-branch.md)（前提タスク`feat/awase-calibration`）
- [ADR-196](../adr/196-keymap-learn-truth-priority.md)関連タスク（PR #259・#260、本ドキュメントの対象外・別セッション担当）
