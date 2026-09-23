# ADR-195: PR #250〜#258 develop統合と、レビューで見送った低優先度指摘の残作業

状態: **1節(develop統合)は完了(2026-09-23)。残るは2節(低優先度指摘、対応不要と判断)・
3節(T10究明、未着手)のみ。** 本ドキュメントは、2026-09-23セッションでPR #250〜#258
（ADR-195 T0/T2/T3/T4/T5/T6/T8/T9）を横断レビュー・修正した後に残った作業をまとめた
ものとして起票したが、同セッション内でdevelop統合まで完了した。着手時は
`.claude/rules/worktree-per-session.md` に従い専用worktree/branchを切ること。

## 1. develop統合(完了、2026-09-23)

PR #250〜#258は全てdevelopへ統合済み。実際の統合順序と結果:

1. **T0(#257)**: developとの間で`crates/awase-windows/src/state/mod.rs`のモジュール宣言
   リストに実コンフリクトが発生(develop側で別PRが追加した`state_dependent_key_warning`と
   競合)。解消・CI green確認後、developへマージ(`97b11166`)。
2. **T9(#258)**: T8(#253)・T6(#255)の最新修正コミットを`git merge`で取り込み(T2(#250)・
   T3(#251)は元々ancestorとして含んでいた)、base branchをdevelop直接へretarget
   (`gh api ... -X PATCH -f base=develop`、`gh pr edit`はGraphQL「Projects (classic)」
   エラーで失敗するため回避)。retargetだけではCIがトリガーされない
   (`pull_request`の既定typesに`edited`が含まれないため)ため、developの取り込みマージを
   1コミットとして追加push。fmtジョブが初めて実行され、本セッション中の手動編集による
   フォーマット崩れを`cargo fmt`で機械的に解消。CI green確認後developへマージ(`8f037c53`)。
3. **PR #250・#251・#253・#255**: T9が全コミットを祖先として含むため、個別マージ不要と
   判断しsupersededとしてコメント付きでクローズ(削除はしていない)。
4. **T5(#252)**: base(T2)がdevelopに統合済みのため、`git rebase --onto develop
   origin/feat/adr195-t2-self-verification HEAD`でdevelop直上へ付け替え
   (`lib.rs`のdocコメント1箇所のみ軽微な衝突、即解消)。base retarget後、CIトリガーのため
   PRを一旦close→reopen(`pull_request`の既定typesは`opened`/`synchronize`/`reopened`の
   み)。CI green確認後developへマージ(`588cbd59`)。
5. **T4(#256)**: base(T3)がT8由来のコミット(`3613707e`/`f5d53047`)を含んでおり、それが
   develop側にも(T9経由で)別経路で既に入っていたため、素朴なrebaseは`persist.rs`/
   `staleness.rs`で衝突した。`git rebase --onto develop f5d53047 HEAD`(T8由来コミット
   そのものを再適用対象から除外し、T4固有のコミットだけを replay)で無衝突に解消。
   この過程で、T4が`crates/awase-windows/Cargo.toml`に`awase-windows→awase-keymap-learn`
   の依存を**初めて**追加したことが判明し、それによりwindows-build CIジョブの
   `cargo clippy -p awase-windows`が`awase-keymap-learn`も初めてリント対象に含めるように
   なった結果、以前は一度もCIに引っかからなかった`graph.rs::cpp_plan`のcognitive
   complexity超過(23/15、ADR-195の初期実装から存在、T4/T5いずれの新規コードでもない)が
   表面化・CI失敗。`ime_controller.rs::apply`等の既存の同種判断に倣い
   `#[allow(clippy::cognitive_complexity)]`を付与して解消。CI green確認後developへ
   マージ(`d7e0df17`)。

**教訓**: ローカルのLinuxサンドボックスで`cargo clippy -p awase-windows`を`--target
x86_64-pc-windows-msvc`無しで実行すると、`#[cfg(windows)]`配下のコードが丸ごとコンパイル
対象から外れ、実際のCI(windows-latestネイティブ実行)とは全く別のコード経路をリントして
しまう(本件では大量の偽`dead_code`警告が出た一方、実在する`cpp_plan`の指摘は出なかった)。
awase-windows関連のclippy挙動をローカルで裏取りする際は必ず`--target
x86_64-pc-windows-msvc`を付けること(CLAUDE.mdの既存注意と同じ理由)。

最終確認: developの最新コミット(`d7e0df17`)で`cargo fmt --all -- --check`・
`cargo test --workspace --lib`(1022+α passed)・`cargo test -p awase-windows --lib`
(795 passed)・`architecture_guard`/`layer_boundary_guard`(8 passed)・Windows target
`cargo check`(awase/awase-windows/awase-settings/awase-keymap-learn-win)を実施し、
全てgreenを確認済み。

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

- [x] 上記PR群が依存順にdevelopへ統合される（コンフリクト解消・CI green）。2026-09-23完了。
- [ ] 2節の指摘のうち着手したものは、対応してPRへ追随コミットするか、見送りと判断した理由を
  このファイルへ追記する（現時点は全件見送りのまま、対応の要否は次にファイルへ触れる
  セッションが判断する）。
- [ ] T10の究明が1つでも進展したら、T10ファイル自体を更新する（本ファイルではなくT10側に書く）。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md)
- [adr195-t10-realimedriver-ci-observation-failure.md](adr195-t10-realimedriver-ci-observation-failure.md)
- [adr195-t-rebase-calibration-branch.md](adr195-t-rebase-calibration-branch.md)（前提タスク`feat/awase-calibration`）
- [ADR-196](../adr/196-keymap-learn-truth-priority.md)関連タスク（PR #259・#260、本ドキュメントの対象外・別セッション担当）
