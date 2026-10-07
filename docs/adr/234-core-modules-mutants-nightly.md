---
id: ADR-234
title: |-
  純粋な核(CORE_MODULES)の mutants の生き残りをテストに変える(読む人を先に決め、定期実行は後で網として足す)
summary: |-
  `.cargo/mutants-awase-windows.toml` の `examine_globs` は FCIS の F 系 PR で 10/05〜06 に 9 ファイル増えたが、mutants は PR では skipping で、増えた分は一度も回っていない。一方で、9/13・9/23・9/23(8 shard)の 3 回は最後まで走り、ほぼ同じファイルに missed 52〜57 件が残ったまま、誰もテストに変えなかった。足りないのは回す回数ではなく読む人。mutants が本番の不具合を見つけた記録は 0 件で、テストの不足を埋めた実績は 2 回(7/24・9/23)。どちらも変更した人がその場で回して読んだときだけ。
  決定の順序: (S0') 既存の成果物(run 35920696097)の CORE の missed 22 件を、CI の費用なしでテスト化・理由つき除外する。やる人がいなければ本 ADR を取りやめ、mutants の登録の義務ごと撤去する案(Q1 (d))へ進む。(S1) `ci.yml` の `mutants-awase-windows` を workflow_dispatch 専用の `mutants-scheduled.yml` へ移し、新しい核を `examine_globs` に載せた PR の作者がブランチで 1 回回して読むことを完了前チェックに入れる。`mode_key_pass`・`explicit_press` を足し、役目を終えた `mutants-scope-investigation.yml`・`mutants-bug158-scope.toml` を撤去する。(S2) S0'・S1 が回り出してから、週 1 回の schedule を取りこぼしの網として足す。CORE のファイルに missed が 1 件でもあれば赤、CORE 以外は Summary に出すだけ。
  PR ごとの自動実行と Issue の自動起票はしない。V1b(`examine_globs` を `CORE_MODULES` に揃える)は S3。
status: |-
  提案(起草中、Opus round1〈Blocker 2・Must 5・Should 6・Nit 4〉を反映、再レビュー前)
related_adr:
  - "ADR-229"
  - "ADR-158"
  - "ADR-162"
  - "ADR-194"
  - "ADR-218"
  - "ADR-219"
  - "ADR-220"
  - "ADR-240"
  - "ADR-236"
  - "ADR-237"
  - "ADR-243"
---

# ADR-234: 純粋な核(CORE_MODULES)の mutants の生き残りをテストに変える

## 目的

FCIS(ADR-229)で `state/` に切り出した純粋関数は、`CORE_MODULES` と `.cargo/mutants-awase-windows.toml` の両方に載せる決まりになっている(タスク表 V1・V2)。しかし、載せたあとで回す人も、回した結果を読む人もいない。そのため、載せても何も検査していない。これを「生き残りが人に読まれて、テストか理由つきの除外に変わる」状態にする。それができないと分かったら、載せる義務のほうを撤去する。新しい検査機構は足さない。既存の手動ジョブの置き場所を移すことと、役目を終えた一時ワークフローを撤去することを対にする。

## 背景(実測した事実)

数値は develop `1630e55e`(2026-10-06)時点。

### 1. 今の mutants の構成

| 何 | どこ | 対象 | トリガー | 実行環境 |
| --- | --- | --- | --- | --- |
| `mutants` ジョブ | `.github/workflows/ci.yml:528` | ルート `awase` crate(`.cargo/mutants.toml`) | `if: github.event_name == 'workflow_dispatch'` | ubuntu-latest、4 shard |
| `mutants-awase-windows` ジョブ | `ci.yml:564-605` | awase-windows の許可リスト 29 ファイル(`.cargo/mutants-awase-windows.toml`) | 同上 | windows-latest、8 shard、`timeout-minutes: 90` |
| `mutants-windows.yml` | 単独 | awase-windows 全体(`--jobs 8`) | workflow_dispatch のみ | self-hosted(GCP Spot) |
| `mutants-actuation-confluence-windows.yml` | 単独 | 合流点 5 関数(13 mutants) | workflow_dispatch のみ | windows-latest。冒頭に「棚卸しが終わったら削除してよい」 |
| `mutants-scope-investigation.yml` | 単独 | `.cargo/mutants-bug158-scope.toml`(ADR-194 の再挑戦条件の検証用 4 ファイル) | workflow_dispatch のみ | windows-latest。冒頭に「検証が終わったら削除してよい」 |
| `mise run mutants` | `mise.toml:30` | ルート crate | 手動 | ローカル |

- PR で `mutants`・`mutants-awase-windows` が skipping になるのは、上の `if` が workflow_dispatch 以外を除いているため。設計どおりで、`ci.yml:525-527` に「フル実行は数十分かかるため、通常の push/PR ゲートには含めない」とある。確認: `gh pr view 528 -R cuzic/awase --json statusCheckRollup --jq '.statusCheckRollup[]|select(.name|test("mutants"))|[.name,.conclusion]|@tsv'` → 2 ジョブとも `SKIPPED`。
- awase-windows の mutants を ubuntu で回せないのは、`crates/awase-windows/examples/*.rs` の一部が `windows` crate を cfg なしで参照していて、ベースラインのビルドが E0433 で落ちるため(`ci.yml:567-578`、`mutants-scope-investigation.yml` の冒頭のコメント)。9/13 の ubuntu の run が通ったのは、この examples が足された 9/20 より前だったから。
- schedule トリガーを持つワークフローは今 1 本も無い(`grep -rn 'schedule:' .github/workflows/` → 0 件)。デフォルトブランチは `develop`、リポジトリは public(`gh repo view cuzic/awase --json defaultBranchRef,isPrivate`)。develop に branch protection は無い(`gh api repos/cuzic/awase/branches/develop/protection` → 404)。そのため「必須チェック」という仕組みは無く、あるのは「ジョブが赤になる」ことだけ。
- cargo-mutants は 27.1.0(成果物の `outcomes.json` の `cargo_mutants_version`)。テストランナーは既定の `cargo test`(`ci.yml:361-364`)。

### 2. CORE_MODULES と mutants の登録の差

```sh
sed -n '/^const CORE_MODULES/,/^];/p' crates/awase-windows/tests/layer_boundary_guard.rs | grep -c '^    "'   # 57
grep -cE '^\s+"crates/awase-windows/src/state/[a-z_]+\.rs"' .cargo/mutants-awase-windows.toml           # 22
```

- `CORE_MODULES`(`layer_boundary_guard.rs:495-553`)は 57 件。そのうち `examine_globs` に `state/<名前>.rs` の完全一致で載っているのは **19 件**で、載っていないのは **38 件**。タスク表 V1b の「52 件のうち 15 件」は develop `bbb70f29` の時点の数。
- 載っていない 38 件のうち、本番コードが大きいもの(先頭の `#[cfg(test)]` までの行数、コメント込み): `platform_state` 1626 行、`explicit_press` 1191 行、`actuation_chain` 688 行、`key_effect_table` 657 行、`ime_actuation_decision` 378 行、`mode_key_pass` 304 行、`open_warrant` 242 行。
- 9/23 以降に `examine_globs` へ足されたファイルは 9 件(`physical_disposition`・`drift_plan`・`drift_correction`・`ime_set_open_plan`・`msaa_role_plan`・`focus_classify_plan`・`focus_resolve_plan`・`relay_plan`・`deferred_gate_plan`)。コミットは `71b7bbff`〜`e133c730` で、10/05〜10/06。確認: `diff <(git show $(git rev-list -1 --before=2026-09-24 origin/develop):.cargo/mutants-awase-windows.toml | grep -oE '"crates/[^"]+"') <(grep -oE '"crates/[^"]+"' .cargo/mutants-awase-windows.toml)`。この 9 件は**一度も変異テストを受けていない**。#528(F6b、未マージ)の PR 本文にも「mutants は CI で skipping のため未実行」とある。

### 3. 回したが読まなかった: 最後まで走った 3 回の run

ci.yml の workflow_dispatch の run を全部見ると(`gh api 'repos/cuzic/awase/actions/workflows/253455717/runs?event=workflow_dispatch&per_page=100'`)、`mutants-awase-windows` が最後まで走った run は 3 回ある。

| run | 日時(UTC) | ブランチ / runner | shard | 結果 |
| --- | --- | --- | --- | --- |
| 34733775117 | 09-13 | develop `baf7dea0` / ubuntu-latest | 1 | 588 件、missed **57**、43 分 |
| 35879891879 | 09-23 15:12 | `ci/mutants-parallel-jobs` / windows-latest | 4 | 624 件、missed **52**、1 shard 56〜60 分 |
| **35920696097** | 09-23 21:09 | `ci/mutants-parallel-jobs` `e78fc582`(8 shard 化のコミット) / windows-latest | 8 | 624 件(1 shard 78 件)、missed **53**。ジョブ全体で 1 shard 24〜37 分 |

- 10-06 の run 37422836301(`experiment/monad-style-decision`)は、全 shard が開始から約 5 分で cancelled になった。手で止めたもので、時間切れではない。
- missed は 3 回とも、ほぼ同じファイルに出ている。run 35920696097(成果物は `expired: false`)の内訳: `state/hook_state.rs` 10、`focus/cache.rs` 9、`single_thread_cell.rs` 8、`state/observation_store.rs` 7、`state/ime_model.rs` 6、`state/probe_admission.rs` 5、`state/belief.rs` 4、`lifetime_counter.rs` 2、`state/conv_mode.rs` 1、`focus/class_names.rs` 1。**CORE_MODULES に入るファイルの分は 22 件**(hook_state・observation_store・belief・conv_mode)。例: `focus/cache.rs:69:50: replace < with <= in FocusCache::get`(TTL の境界)。
  確認: `gh run download 35920696097 -R cuzic/awase -p 'mutants-report-awase-windows*' && cat */missed.txt | sed -E 's/:.*//' | sort | uniq -c`
- **10 日間に 3 回回して、この missed をテストに変えたコミットは無い**(`git log origin/develop --since=2026-09-13 -i --grep='missed\|35920696097\|35879891879'` に該当なし)。同じ 9/23 に #248 で mutation の作業をしていた人もいたが、そちらは自分で選んだ範囲(`mode_key_pass`)だけを読んだ。
- 全 shard のジョブは、missed が 0 だった 1 つ(35920696097 の shard 1、`success`)を除いて `failure`。

### 4. mutants の実績: 本番の不具合 0 件、テストの不足を埋めたのは 2 回

`git log origin/develop --oneline -i --grep=mutant | wc -l` → 71 件。このうち、実際にテストを足したのは次の 2 回。

- 2026-07-24 初回のフル実行(run 30070606789、661 mutants、missed 127)→ triage して `def786eb`・`28315441` で回帰テストを追加した。ログ専用の 19 件と等価変異は、理由つきで `exclude_re` へ(`.cargo/mutants.toml` のコメント)。
- 2026-09-23 `ModeKeyPassLatch` の missed 22 件 → テスト 7 件で 1 件まで減らした。残った 1 件は等価変異として確定した(`mode_key_pass.rs:158:28`、PR #248、`d8653da5`)。

どちらも、**変更した人が自分で対象を決めて回し、その場で読んだ**ときだった。mutants が本番の不具合を見つけた記録は無い。BUG-41(2026-07-25)は `mutants-windows.yml` の**ベースライン**、つまり Windows で `cargo test` を初めて走らせたときに見つかったもので、変異テストの成果ではない(`ci.yml:345-352`、`docs/known-bugs/BUG-041.md`)。この経路は今は `windows-build` ジョブが PR ごとに走らせている。

「toml が dispatch のときしか読まれない」ことの実害に近い記録は 1 件ある。ADR-236 の表の #6 で、10-05 に `.cargo/mutants-awase-windows.toml` に衝突マーカーの基底行が残った。PR #517 のブランチの上で気づいたので、develop には届かなかった。

### 5. 件数と実行時間

`cargo mutants --list` はビルドをせず、構文解析だけで数秒で件数を出す。Opus のレビューが develop `1630e55e` で数えた結果は次のとおり(本 ADR の起草者は再計数していない。再確認のコマンド: `cargo mutants -p awase-windows --config .cargo/mutants-awase-windows.toml --list | wc -l`)。

- 今の 29 ファイル: **768 件**(9/23 の 624 件 + 9 ファイルの分で +144 件)
- `mode_key_pass.rs`: **59 件**、`explicit_press.rs`: **143 件** → S1 の合計は **970 件**
- CORE に入っていて許可リストに無い 38 件の合計: **908 件** → CORE 全体を足すと **1,676 件**

1 件あたりの時間は、run 35920696097(8 shard、1 shard 78 件で 24〜37 分、ベースラインのビルド込み)から 18〜28 秒。S1 の 970 件を 8 shard で回すと、1 shard 121 件で **36〜57 分**になり、90 分以内に収まる。CORE 全体の 1,676 件だと 1 shard 210 件で **63〜98 分**になり、90 分を超えうる。ルート crate(ubuntu)は 1 shard 約 300 件で 7 分前後で、Windows より 1 桁速い。public なので、GitHub-hosted runner の費用はかからない。

### 6. 限界: 人手のレビューが見つけた穴は、mutants の守備範囲の外

今日の Opus のレビューが見つけた穴は、mutants では見つからない(これは本 ADR の効果の根拠ではなく、限界として書く)。

- #513 の `strip_comments`(`/*` を含むコメントで本番コードが消える): 穴は `tests/architecture_guard.rs` にある。`examine_globs` が `tests/` を含まないので、そもそも対象外。
- #528 の引数の取り違え・定数の差し替え: 穴は殻(`output/vk_send.rs`、許可リスト外)にある。それに、引数の入れ替えは cargo-mutants の変異の種類に無い(変異の種類は、関数本体を既定値に置き換える、二項演算子・単項演算子、match の腕と guard、構造体リテラルのフィールドの削除)。#528 の核 `state/warm_send_plan.rs` の境界(`>` と `>=`)は mutants で見つかる種類の穴だが、このファイルは develop に無く(PR は OPEN)、許可リストにも無い。見つかるのは、マージされて許可リストに載り、そのあと回したときに限られる。
- #530 の xtask(`crates/xtask-adr-evidence`)も許可リストの外。

mutants が埋めるのは、核の純粋関数の境界・分岐に対するテストの不足で、レビューとは別の穴。Opus のレビューを減らす根拠にはならない。

## 決定

### D0(S0'): 既存の成果物の CORE の missed 22 件を、先に片づける

- run 35920696097 の成果物にある CORE の missed 22 件(hook_state 10・observation_store 7・belief 4・conv_mode 1)を、次のどちらかにする。
  - (a) テストを足す(当該ファイルの `#[cfg(test)]`。#248 と同じ型)。
  - (b) 等価変異・ログ専用なら、`.cargo/mutants-awase-windows.toml` の `exclude_re` に理由のコメントつきで載せる。
- CI の費用はかからない(成果物は既にある)。確認は、ブランチで `gh workflow run ci.yml --ref <branch>` を 1 回回し、4 ファイルの missed が 0 になったことで見る。9/23 以降にコードが変わって行番号がずれた変異もあるので、確認の run の結果を正とする。
- **これをやる人が現れなければ、定期実行にしても同じ結果になる**(背景 3)。その場合は本 ADR の D1 以降を取りやめ、Q1 (d)(登録の義務ごと撤去)へ進む。

### D1(S1): 新しい核を載せた PR の作者が、ブランチで 1 回回して読む

- 背景 4 で唯一うまくいった型(変更した人がその場で回して読む)を、そのまま手順にする。`examine_globs` にファイルを足した PR の作者は、その PR のブランチで mutants を 1 回回し、足したファイルの missed をテスト化するか、理由つきで除外してからマージする。
- 置き場所は完了前チェック。ADR-243 の完了前チェック(243 の 68 行目・129 行目の「CORE_MODULES と examine_globs に載せた」)と、タスク表 V2 の 1 項目目に 1 文を足す。別の場所に新しい規則は作らない。
- 回す手段: S1 のあとは `gh workflow run mutants-scheduled.yml --ref <branch>`。ファイルをデフォルトブランチの develop に置くので、どのブランチでも `--ref` で起動できる。S1 の前は `gh workflow run ci.yml --ref <branch>`(CI 全体と一緒に回る)。
- PR ごとに**自動では**回さない(D5)。作者が手で 1 回起動する。

### D2(S1): `mutants-awase-windows` を `mutants-scheduled.yml` へ移す(この段階では schedule を持たない)

- 新しい `.github/workflows/mutants-scheduled.yml` を作り、`ci.yml` の `mutants-awase-windows` ジョブ(`ci.yml:564-605`、コメント込みで約 45 行)を**移す**。これは移動で、行は減らない。S1 のトリガーは `workflow_dispatch` だけにする。
- ルート crate の `mutants` ジョブは `ci.yml` に残す(`ci.yml` の workflow_dispatch は、ブランチで CI 全体を回す用途にも使われている。例: run 37422836301)。
- ランナー、shard 数(8)、`CARGO_INCREMENTAL=1`、成果物の名前は今のジョブのまま。`timeout-minutes: 90` も据え置く(S1 の 970 件で 1 shard 36〜57 分、背景 5)。
- 同じ PR で、`cargo mutants -p awase-windows --config .cargo/mutants-awase-windows.toml --list` を既存の ubuntu の `test` ジョブに 1 step 足す(ビルドしないので数秒で終わる)。これで、toml の構文が壊れた場合(ADR-236 #6)と、`examine_globs` のファイルが改名・削除されて 0 件になった場合を、PR のたびに止められる(cargo-mutants は存在しない glob を黙って 0 件として扱う。`mutants-scope-investigation.yml` の冒頭のコメント)。0 件の検出は、許可リストの各ファイルが `--list` の出力に 1 回以上現れることを shell で数えて行う(10 行程度の見込み)。

### D3(S2): 週 1 回の schedule は、D0・D1 が回り出してから取りこぼしの網として足す。CORE に missed があれば赤

- 足す条件: D0 が済み、かつ D1 のとおり作者がブランチで回した PR が 2 本以上あること。
- 判定は、cargo-mutants の終了コードでは行わない。終了コードの割り当て(missed とタイムアウトのどちらが優先されるか)は未確認で、変異 1 件の時間切れは無限ループを作る変異ではふつうに起きる(9/23 のルート crate の run に 1 件あった)。代わりに、集計の step で次の 2 つを見る。
  - `outcomes.json` のベースラインの結果が `Success` であること。そうでなければ赤。
  - **CORE_MODULES のファイルに missed が 1 件でもあれば赤**。CORE 以外のファイル(`scanmap.rs`・`focus/cache.rs`・`single_thread_cell.rs` 等)の missed は、`$GITHUB_STEP_SUMMARY` に一覧を出すだけにする。
  - ジョブが `timeout-minutes` を超えた場合は、GitHub が cancelled にする(何もしなくても赤になる)。
- 基準を 0 件にできるのは、D0 で CORE の既存分を片づけ、D1 で新しいファイルをマージ前に片づけるから。ADR が却下した「しきい値を上げ続ける」問題にはならない。
- 赤にする理由: schedule で起動した workflow が失敗すると、GitHub は cron の行を最後に変えたユーザーにメールを送る。常に緑にすると、人に届く知らせが 0 になる。これは、9/23 に成果物が読まれなかったのと同じ形になる。
- 集計の step(各 shard の `missed.txt` を集め、`CORE_MODULES` の名前と照合し、Summary に出す)は 30〜40 行の見込み。前回との比較は、行:列を落とした「ファイル名 + 関数名 + 変異の説明」の文字列で行う。

### D4: Issue は自動起票しない

- 9/23 の成果物は 3 回分とも残っていたが、読まれなかった。1 件 1 Issue にしても、読まれない Issue が増えるだけになる。知らせは D3 の赤(メール)と、D1 の完了前チェックで足りる。

### D5: PR ごとには自動で回さない

- Windows で 1 件 18〜28 秒、S1 で 970 件。8 shard でも 1 shard 36〜57 分かかる。FCIS の PR は全チェックが終わってからマージする運用なので(メモリ `project_ci_bug_repro_campaign_2026_10_04.md`)、必須でないジョブでもマージを遅らせる。D1 の「作者が手で 1 回」は、足したファイルを読む人がいるときだけ回すので、これとは矛盾しない。
- ADR-240(85 行目)が「既にある確認」として挙げている `cargo mutants --in-diff` は、「手で回せる」という意味で、PR ごとの自動実行のことではない。

### D6: V1(#530)との関係と V1b

- V1 は「PR で `CORE_MODULES` に足した名前が `examine_globs` にも載っているか」だけを見る。D1 で、載せたものが実際に回って読まれるようになり、V1 に意味が生まれる。V1 を当面必須チェックにしないという決定は変えない。
- V1b(`examine_globs` を `CORE_MODULES` に揃える)は S3 で行う。揃い切ったら、V1 の差分検査(`core_registry.rs` 174 行、CI ジョブ `core-registry-consistency`、base の取得)を、`layer_boundary_guard.rs` の「`CORE_MODULES` の全名が `examine_globs` に載っている」という全数テスト(20 行程度の見込み)に置き換えられる。差分を読まないので、develop への直接 push も拾える。ただし `test` ジョブの中に入るので、載せ忘れると `test` ジョブが赤になる(Q4)。

## 段階

| 段階 | やること | 撤去対象 | 検証方法(CI で確認できる形) | 取りやめ条件 |
| --- | --- | --- | --- | --- |
| **S0'**(D0。CI の費用なし) | run 35920696097 の CORE の missed 22 件をテスト化・理由つき除外 | なし(テストが増える) | ブランチで `gh workflow run ci.yml --ref <branch>` を 1 回回し、hook_state・observation_store・belief・conv_mode の missed が 0 | **2026-10-20 までに誰も着手しなければ**、S1 以降を取りやめて Q1 (d) へ |
| **S1**(D1・D2) | ジョブを `mutants-scheduled.yml` へ移す(workflow_dispatch のみ)。`examine_globs` に `state/mode_key_pass.rs`・`state/explicit_press.rs` を足す。`mode_key_pass.rs:158:28` の等価変異の除外を `.cargo/mutants-bug158-scope.toml` から移す(行:列で書く。同じ関数の 164:53 にも `&& → ||` の変異があるので関数名では絞れない、とコメントに書く)。`--list` の step を足す。完了前チェック(ADR-243・V2)に D1 の 1 文を足す。撤去する 2 ファイルを名指ししているコメント(`ci.yml:574,577`、`mutants-actuation-confluence-windows.yml:28`)と文書(`docs/tasks/mode-key-pass-latch-mutation-coverage{,-followup}.md`)を直す | **撤去**: `.github/workflows/mutants-scope-investigation.yml`(61 行)、`.cargo/mutants-bug158-scope.toml`(52 行)。toml の `drift.rs`・`refresh_plan.rs` は develop に存在せず、`force_guard.rs` は本体の許可リストにある。残る `mode_key_pass.rs` を本体へ移せば役目が無くなる。**移動**: `ci.yml` の `mutants-awase-windows`(約 45 行、増減 0)。**追加**: `--list` の step 約 10 行。差し引き約 100 行減(Q6 を足せばさらに約 105 行減) | `gh workflow run mutants-scheduled.yml --ref <S1 のブランチ>` を 1 回回し、(1) 8 shard が 90 分以内に終わる、(2) `mode_key_pass.rs` の missed が 0(#248 以降このファイルは変わっていない。`git log --since=2026-09-23T05:00 -- crates/awase-windows/src/state/mode_key_pass.rs` で 0 件)、(3) `explicit_press.rs` の missed を同じ PR でテスト化・除外する(D1 を S1 自身に当てる)。`--list` の step は、許可リストの 1 行を存在しないファイル名に変えた一時コミットで赤になることを確かめる | `explicit_press` の missed を S1 の PR で片づけられない(30 件を超える、など)→ `explicit_press` を外し、`mode_key_pass` だけにする |
| **S2**(D3) | S0' が済み、D1 のとおり回した PR が 2 本以上になったら、`mutants-scheduled.yml` に週 1 回の `schedule` と集計の step(30〜40 行)を足す | なし | `workflow_dispatch` で 1 回回し、CORE の missed 0 で緑、CORE のファイルに missed を 1 件わざと作った一時ブランチで赤になる。次の schedule の run が自動で起動したことを Actions の一覧で確かめる | (1) schedule の run が**赤のまま 2 回続き、その間に missed をテスト化・除外したコミットが 0 件**なら、schedule を外して workflow_dispatch のみに戻す(赤が無視されている)。(2) `examine_globs` にファイルを足した PR が、D1 の run をせずに **3 本**マージされたら、D1 が守られていないので Q1 (d) を所有者に改めて聞く |
| **S3** | V1b: 残りの CORE 36 件を、数回に分けて `examine_globs` に足す(大きい順)。toml の冒頭のコメント(`:6-21`、「Linux でコンパイルされるファイルだけを許可する。`platform_state.rs` は `#[cfg(windows)]` なので対象外」)を、windows-latest で回している現状に合わせて書き直す。`platform_state`(137 件)を足すときに必要になる。足す時期と順序は、ADR-237 の段階 2(`examine_globs` を昇順にそろえる)と衝突しないように合わせる | V1 の差分検査(`core_registry.rs`・`core-registry-consistency` ジョブ)を全数テストに置き換える(Q4 で承認された場合のみ。全数テストは `#[cfg(windows)]` のファイルを許す前提で書く) | `CORE_MODULES` ⊆ `examine_globs` の全数テストが green。1 shard が 90 分以内(CORE 全体では 1 shard 63〜98 分の見込みなので、16 shard にするか、CORE だけの config に分ける) | shard を増やしても 1 shard が 90 分を超える → 大きいファイル(`platform_state` 等)を外したところで止める |

## 却下した案と理由

- **定期実行を先に入れる(初版の順序)**: 背景 3 のとおり、3 回回して誰も読まなかった。読む人が決まらないまま回す回数を増やしても、同じ結果がくり返されるだけ。
- **PR ごとにフルで回す**: D5 のとおり。
- **PR で `cargo mutants --in-diff` を自動で回す**: ベースラインのビルドだけで 131 秒(`ci.yml:594`)かかる。FCIS の PR は核の新しいファイル(数百行)を含むことが多い。D1 の「作者が手で 1 回」で同じ効果が得られ、読む人がいない PR では回らない。
- **生き残りを Issue に自動起票する**: D4 のとおり。
- **missed を全部(CORE 以外も)赤にする / しきい値つきで落とす**: CORE 以外には、既に 31 件(53 − 22)の missed がある。しきい値を決めても、毎回赤になるか、しきい値を上げ続けるかのどちらかになる。D3 は CORE だけを 0 件の基準で見る。
- **cargo-mutants の終了コードで赤と緑を分ける(初版の D2)**: 終了コードの割り当てが未確認で、変異 1 件の時間切れ(よくある)で赤になるおそれがある。D3 の `outcomes.json` を見る方式に替えた。
- **ubuntu-latest に戻して速く回す**: awase-windows は examples のビルドで落ちる(背景 1)。examples を `required-features` で外せば通るかもしれないが、未確認で、コードの変更が要る。S3 で shard が足りなくなったときの選択肢として残す。
- **self-hosted(`mutants-windows.yml`)で回す**: runner の起動が手作業(GCP Spot、トークンの発行)なので、定期実行にできない。本 ADR では触らない。
- **除外リストや例外の印を新しく作る**: ADR-218〜220 と同じ型になる。除外は既存の `exclude_re` に理由のコメントつきで書くだけにする。

## リスクと限界

- 背景 5 の件数(768・59・143・908)は Opus のレビューが `cargo mutants --list` で数えたもので、起草者は再計数していない(このサンドボックスには cargo-mutants が入っていない)。S1 の PR で `--list` の step が最初に回ったときに確かめる。
- cargo-mutants の終了コードの割り当ては未確認。D3 はこれに頼らない設計にした。
- 9/23 の CORE の missed 22 件のうち、今も残っている件数は未確認(その後テストが増えたファイルもある)。S0' の確認の run で分かる。
- mutants が見るのは、核の境界・分岐のテストの不足だけ。ガードの検査コード、殻の配線、引数の取り違えは見ない(背景 6)。
- D1 は人(またはエージェント)が守る手順で、機械では強制しない。S2 の取りやめ条件 (2) で、守られていないことを検出する。
- schedule は、public リポジトリで 60 日間活動が無いと GitHub が止める。今の開発ペースでは当たらない。
- 8〜16 shard の Windows ジョブが、無料枠の同時実行数の上限に当たり、同じ時刻の他の CI を待たせる可能性がある。schedule は利用の少ない時間帯に置く。
- `cargo test` を使うので、`TSF_OBS_TEST_LOCK` に頼る並列の汚染の防御は今と同じ(`ci.yml:359-364`)。変えない。

## 所有者に聞くこと

- **Q1 進めるか、撤去するか**: (a) 毎晩回す、(b) 週 1 回回す、(c) 手動のまま、(d) **mutants の登録の義務ごと撤去する**(V1 の xtask `core_registry.rs` 174 行と `core-registry-consistency` ジョブ、タスク表 V2 の該当の文、ADR-243 の完了前チェックの該当の項目)。**推奨: まず S0' をやってみる。読む人がいると分かったら (b) を S2 で。S0' が 10/20 までに進まなければ (d)**。「載せても誰も回さない・読まない」状態を解消する手段は、回して読むことと、載せるのをやめることの 2 つで、撤去を主目的とする方針からすると (d) を落とす理由は無い。
- **Q2 対象の範囲**: (a) S1 で 2 モジュール(`mode_key_pass`・`explicit_press`)だけ足す、(b) 最初から CORE 全体を足す(V1b を同時に。1,676 件、8 shard では 90 分を超えうる)、(c) 今の 29 ファイルのまま。**推奨 (a)**。970 件で、1 shard 36〜57 分。
- **Q3 誰が読むか**(一番大事な質問): (a) 載せた PR の作者が、ブランチで回して読む(D1)、(b) 所有者が週 1 回、Summary を読む、(c) 定期セッションのエージェントが読む。**推奨 (a)**。背景 4 で実績がある唯一の型だから。S0' の 22 件は、(b) か (c) のどちらがやるかを決めてほしい。
- **Q4 V1 の置き換え**: V1b が揃ったら、V1 の差分検査を `layer_boundary_guard.rs` の全数テストに置き換え、xtask 側(`core_registry.rs` 174 行・CI ジョブ)を撤去するか。全数テストは `test` ジョブに入るので、載せ忘れると `test` ジョブが赤になる。**推奨: S3 の時点で改めて判断**。
- **Q5 ルート crate の `mutants` ジョブ**: 定期実行に含めるか。9/23 の時点で missed 101 件。ubuntu で 1 shard 7 分と安い。**推奨: S2 で CORE の運用が回ってから**。
- **Q6 `mutants-actuation-confluence-windows.yml` の撤去**: 冒頭に「棚卸しが終わったら削除してよい」とあり、棚卸しは 2026-09-23 に完了している(`docs/tasks/actuation-confluence-inventory.md`)。ただし `docs/tasks/corpus-discard-impact-2026-10-06/README.md:150` が「② の後半に流用できる」と書いている。**推奨: コーパスの置き換えの計画で使わないと決まったら、S1 で一緒に撤去**。
- **Q7 赤のメールの宛先**: D3 の赤は、cron の行を最後に変えたユーザーにメールで届く。S2 の PR を作るのがエージェントでも、push したアカウント(所有者)に届く。受け入れるか。受け入れない場合は D3 を常に緑にすることになるが、そのときは誰が・どの曜日に Summary を開くかを Q3 の答えとして決めるまで、S2 を入れない。

## 関連する既存文書への追記案

(本 ADR の起草では、他の文書を書き換えない。採用されたら、各段階の PR で次を追記する)

- `docs/tasks/fcis-layering-tasks-2026-10-06.md` の V1b 行: 「件数は develop `1630e55e` で CORE 57 件・登録 19 件。穴埋めは ADR-234 の S3」。
- 同じタスク表の V2 の完了前チェック 1 と、ADR-243 の完了前チェック(68 行目・129 行目)を 1 か所にまとめて: 「`examine_globs` に足したファイルは、ブランチで `gh workflow run mutants-scheduled.yml --ref <branch>` を 1 回回し、missed をテスト化か理由つき除外にしてからマージする」。
- `ci.yml` の workflow_dispatch のコメント(`ci.yml:8-9`): `mutants-awase-windows` が `mutants-scheduled.yml` へ移ったことと、ブランチで回す手順(`gh workflow run mutants-scheduled.yml --ref <branch>`)。
- `.cargo/mutants-awase-windows.toml` の冒頭: 「回し方は `mutants-scheduled.yml`。生き残りは ADR-234 D0・D1 の手順で扱う」(S3 では冒頭の理由そのものを書き直す)。
- `docs/tasks/mode-key-pass-latch-mutation-coverage{,-followup}.md`: `mutants-scope-investigation.yml` と `mutants-bug158-scope.toml` を S1 で撤去したことを 1 行。
- ADR-240 の 85 行目: `cargo mutants --in-diff` は「手で回せる」の意味で、PR ごとに自動で回してはいない、と書き分ける。
