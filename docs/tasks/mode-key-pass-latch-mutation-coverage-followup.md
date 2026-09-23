# ModeKeyPassLatch cargo-mutants調査の残作業（別セッション向け）

状態: **やること1完了（2026-09-23）、develop未反映のまま**。
`test/mode-key-pass-latch-mutation-followup`ブランチ（`feat/ime-sim-harness`起点、
[commit 789063ce](https://github.com/cuzic/awase/commit/789063ce)）で
drift.rs/refresh_plan.rsの直接単体テスト4件を追加済み。詳細は下記「やること1
実施結果」節参照。やること2（一時ファイル後片付け）は未着手のまま。

<details>
<summary>旧状態（未着手、2026-09-23起票）</summary>

親タスク[mode-key-pass-latch-mutation-coverage.md](mode-key-pass-latch-mutation-coverage.md)
完了・PR #248 developマージの副産物として切り出し。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

</details>

## 背景

[mode-key-pass-latch-mutation-coverage.md](mode-key-pass-latch-mutation-coverage.md)で、
develop採用済みの2ファイル（`state/force_guard.rs`/`state/mode_key_pass.rs`）に対する
cargo-mutants実測22件missedを、テスト追加で21件caught・1件等価変異体（除外）まで
解消した（PR #248、run
[35819375184](https://github.com/cuzic/awase/actions/runs/35819375184)）。

`.cargo/mutants-bug158-scope.toml`の`examine_globs`は元々4ファイルを対象にしていたが、
今回手を付けたのはdevelop採用済みの2ファイルのみで、`state/drift.rs`/
`state/refresh_plan.rs`（`feat/ime-sim-harness`限定、develop未マージ）は手つかずの
まま残っている。本ドキュメントはその残作業と、調査用一時ファイルの後片付け判断を
まとめたもの。

## やること1: drift.rs/refresh_plan.rsのmutation coverage（優先度低）

- 対象: `state/drift.rs`（`check_drift_correction`）・`state/refresh_plan.rs`
  （`next_refresh_ms`/`decide_imm_capability`）。両方とも`feat/ime-sim-harness`
  ブランチ限定（2026-09-23時点でdevelopにマージされていない、ブランチ自体は
  リモートに現存）。
- 親タスクと同様の手順:
  1. `gh workflow run mutants-scope-investigation.yml --ref feat/ime-sim-harness`
     で実測する（ワークフロー自体は既にwindows-latest化済み・developにマージ済み
     なので`--ref`をこのブランチに向けるだけでよいはず。ただし
     `.cargo/mutants-bug158-scope.toml`のexamine_globsやexclude_reは
     develop側の内容が使われる点に注意——`feat/ime-sim-harness`ブランチ自体は
     このファイルの2026-09-23更新〈windows-latest対応・158:28除外〉を持っていない
     可能性が高いので、まずそのブランチに同じ内容をcherry-pick/バックポートする
     必要があるかもしれない）。
  2. missed一覧を洗い出し、親タスクの「やること」節と同じ考え方
     （構造体メソッド自身のグルーコードを直接呼ぶ決定表テストを書く）で潰す。
  3. `feat/ime-sim-harness`はdevelop未マージのまま破棄予定
     （ADR-194の再挑戦条件検証〈2026-09-22実施〉は「条件不成立、破棄継続が妥当」
     という結論で終わっている）ため、**このテスト追加をどこにマージするかは
     自明ではない**。考えられる選択肢:
     - `drift.rs`/`refresh_plan.rs`相当のロジックがdevelopの別モジュールに
       既に存在するなら、develop側にテストを移植する。
     - 存在しないなら、このタスクの価値は「決定表網羅率の実測」だけで終わり、
       `feat/ime-sim-harness`自体を最終的に削除するときに一緒に破棄してよい
       （実装を採用する予定が無いコードにテストを積み増す投資対効果は低い）。
  4. **着手前に、まず`feat/ime-sim-harness`が本当にまだ「再挑戦の見込みなし」で
     確定しているかを`git log`とADR-194本体で再確認すること**
     （このドキュメント自体がstaleになっている可能性があるため）。もし既に
     ブランチが削除されていたら、このタスク自体が不要（クローズしてよい）。

過去の実測（2026-09-22、`--ref feat/ime-sim-harness`、131 mutants対象）は2回とも
約10分で原因不明の`interrupted`になり結果が取れていない
（`mode-key-pass-latch-mutation-coverage.md`の「補足」節参照）。windows-latest化後
なら解消しているかもしれないが未検証。再現するなら`--jobs 1`固定も試す価値がある。

### やること1 実施結果（2026-09-23）

上記手順1〜4を実施。手順4の再確認結果: ADR-194は`feat/ime-sim-harness`上でも
依然「草案・develop未マージ」のままで、2026-09-22の「条件不成立、破棄継続が妥当」
という結論に変化なし（本ドキュメントはstaleになっていなかった）。

1. **worktree/branch**: `test/mode-key-pass-latch-mutation-followup`
   （`~/rust-nicola-worktrees/mkpl-followup`、`origin/feat/ime-sim-harness`起点）。
2. **バックポート**（[commit df9f2029](https://github.com/cuzic/awase/commit/df9f2029)）:
   develop側の`.cargo/mutants-bug158-scope.toml`/
   `.github/workflows/mutants-scope-investigation.yml`
   （windows-latest化・158:28等価変異体除外込み）をこのブランチへ反映。
   さらに4ファイル対象化でdevelop実績の45分を超えたため`timeout-minutes`を
   60→120へ延長（[commit 3b7cacef](https://github.com/cuzic/awase/commit/3b7cacef)、
   最初の実行run
   [35849828981](https://github.com/cuzic/awase/actions/runs/35849828981)は
   ちょうど60分で`cancelled`だった）。
3. **実測**（run
   [35855782626](https://github.com/cuzic/awase/actions/runs/35855782626)、
   131 mutants tested in 85m: **10 missed**, 113 caught, 6 unviable, 2 timeouts）。
   missed 10件の内訳:
   - `drift.rs`: 46:57(`&&`→`||`)・51:24(`<`→`>`)・57:25(`>`→`>=`) の3件
   - `refresh_plan.rs`: 46:38(`||`→`&&`) の1件
   - `force_guard.rs`: 342:70(`*`→`+`、`send_failure_is_timeout`) の1件
   - `mode_key_pass.rs`: 133:18/133:28/133:31/139:21/139:43 の5件
     （このブランチの`mode_key_pass.rs`は develop 採用版〈4引数、
     `readable_at_arm`あり〉と異なる**旧版〈3引数〉**のままで、PR #248の
     修正が未反映。develop側は既にcaught済みのため無関係）。
4. **対応**（[commit 789063ce](https://github.com/cuzic/awase/commit/789063ce)）:
   本タスクの対象である`drift.rs`/`refresh_plan.rs`の4件を、親タスクと同じ
   「構造体/関数を直接呼ぶ決定表スタイル」のテストで解消。`ImeModel`の観測は
   `ObservationStore::record_replayed`（journal/fixture復元用の口）+
   `update_drift`で直接組み立てた（`ime_model.rs::fully_populated_model`の
   フィクスチャと同じ手法）。`force_guard.rs`/`mode_key_pass.rs`の残り6件は
   上記の通り develop 未同期の旧版コード由来のため対象外とした。
5. **検証**: `cargo test -p awase-windows --lib`（754 passed、追加4件含む）・
   `cargo fmt --check`・`cargo clippy --lib --tests`（追加2ファイルへの新規
   指摘なし。`architecture_guard.rs`等の既存clippy失敗はこのブランチが
   develop未同期であることに起因する既存不具合で、本タスクの変更前から
   存在する——本タスクのスコープ外）をローカルで確認。**CIでの
   missed→caught再確認（親タスクの受け入れ基準）はコスト対効果を鑑みて
   意図的に省略した**（4ファイル分で約85分かかる上、本タスク自体が
   「優先度低」「投資対効果が低い可能性がある」と明記された副次タスクの
   ため、ユーザー判断でローカル検証止まりとした、2026-09-23）。

**現状**: `test/mode-key-pass-latch-mutation-followup`ブランチは develop に
一切マージしていない（`drift.rs`/`refresh_plan.rs`自体がdevelopに存在しない
ため）。「やること1」冒頭の選択肢のうち「`feat/ime-sim-harness`が最終的に
削除される際に一緒に破棄してよい」を採用する。

## やること2: 調査用一時ファイルの後片付け判断

以下はADR-194再挑戦条件検証（2026-09-22起票）専用の一時ファイルで、各ファイルの
コメントに「役目が終わったら削除してよい」と明記されている:

- `.github/workflows/mutants-scope-investigation.yml`
- `.cargo/mutants-bug158-scope.toml`
- `docs/tasks/mode-key-pass-latch-mutation-coverage.md`（親タスク、完了済み）
- 本ドキュメント（`mode-key-pass-latch-mutation-coverage-followup.md`）

**「やること1」（drift.rs/refresh_plan.rs）が完了する、またはスコープ外と判断されて
クローズされるまでは削除しないこと**（`examine_globs`が4ファイルとも列挙している
ままなので、2ファイル分だけ終わった段階で消すと「やること1」の再現手順が失われる）。

両方終わったら、この一時ワークフロー一式を削除するコミットを一つ作ってよい
（`.github/workflows/*.yml`の削除はCI設定の変更にあたるため、念のため
`/code-review`を通してからマージすることを推奨する）。

## 関連

- 親タスク: [docs/tasks/mode-key-pass-latch-mutation-coverage.md](mode-key-pass-latch-mutation-coverage.md)
  （develop採用済み2ファイル分、完了・PR #248）
- 姉妹タスク（別セッションが同じcargo-mutants手法で見つけた、IME actuation合流点の
  テスト漏れ、本タスクとは独立に進行中）:
  [docs/tasks/actuation-confluence-already-matched-gap.md](actuation-confluence-already-matched-gap.md)
- ADR-194（IME時間依存ロジックの仮想時間シミュレーションハーネス、
  `feat/ime-sim-harness`、develop未マージのまま破棄方向）
