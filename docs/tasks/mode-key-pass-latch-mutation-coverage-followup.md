# ModeKeyPassLatch cargo-mutants調査の残作業（別セッション向け）

状態: 未着手（2026-09-23起票、親タスク
[mode-key-pass-latch-mutation-coverage.md](mode-key-pass-latch-mutation-coverage.md)
完了・PR #248 developマージの副産物として切り出し）
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

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
