---
id: ADR-234
title: |-
  純粋な核(CORE_MODULES)の mutants を定期実行し、生き残った変異をテストに変える
summary: |-
  `.cargo/mutants-awase-windows.toml` の `examine_globs` は FCIS の F 系 PR のたびに増えている(2026-10-05〜06 に 9 ファイル)が、mutants は PR では skipping で、最後に完走した run は 2026-09-23 の 1 回だけ。追加された 9 ファイルは一度も変異テストを受けていない。その 9/23 の run で生き残った 52 件(awase-windows 側)も、誰もテストに変えていない。登録の検査(V1、#530)はあるのに、登録したものを回して読む仕組みが無い。
  決定: `ci.yml` の `mutants-awase-windows` ジョブを新しい `mutants-scheduled.yml`(schedule + workflow_dispatch)へ移し、定期実行にする。生き残りは Issue を自動起票せず、shard の `missed.txt` をまとめた一覧を Step Summary と成果物に出し、人(またはエージェント)がテストに変えるか、等価変異として理由つきで `exclude_re` に載せる。missed があってもジョブは赤にしない(赤にするのはベースライン失敗とタイムアウトだけ)。第 1 段階は既存の 29 ファイルに `mode_key_pass`・`explicit_press` の 2 つを足すだけにし、実行時間と生き残りの数を測る。撤去対象: `ci.yml` の `mutants-awase-windows` ジョブ、役目を終えた一時ワークフロー `mutants-scope-investigation.yml` と `.cargo/mutants-bug158-scope.toml`。
  PR では回さない(Windows で 1 変異 20 秒前後、8 shard でも 1 時間近い)。`examine_globs` を `CORE_MODULES` 全体に揃える(V1b)のは第 2 段階で、揃ったら V1 の差分検査を「CORE_MODULES ⊆ examine_globs」の全数テストに置き換えて xtask 側を撤去する案を所有者に聞く。
status: |-
  提案(起草中、Opus レビュー前)
related_adr:
  - "ADR-229"
  - "ADR-158"
  - "ADR-162"
  - "ADR-194"
  - "ADR-218"
  - "ADR-219"
  - "ADR-220"
---

# ADR-234: 純粋な核(CORE_MODULES)の mutants を定期実行し、生き残った変異をテストに変える

## 目的

FCIS(ADR-229)で `state/` に切り出した純粋関数は、`CORE_MODULES` に名前を載せ、`.cargo/mutants-awase-windows.toml` にも載せる決まりになった(タスク表 V1・V2)。しかし載せたあとに mutants を回す人も仕組みも無いので、載せることが何も検査していない。これを「定期的に回り、生き残りが人の目に届き、テストに変わる」状態にする。新しい検査機構は足さず、既存の手動ジョブを定期実行へ移すことと、役目を終えた一時ワークフローの撤去を対にする。

## 背景(実測した事実)

数値は develop `1630e55e`(2026-10-06)時点。

### 1. 今の mutants の構成

| 何 | どこ | 対象 | トリガー | 実行環境 |
| --- | --- | --- | --- | --- |
| `mutants` ジョブ | `.github/workflows/ci.yml:528` | ルート `awase` crate(`.cargo/mutants.toml`) | `if: github.event_name == 'workflow_dispatch'` | ubuntu-latest、4 shard |
| `mutants-awase-windows` ジョブ | `ci.yml:579` | awase-windows の許可リスト 29 ファイル(`.cargo/mutants-awase-windows.toml`) | 同上 | windows-latest、8 shard、`timeout-minutes: 90` |
| `mutants-windows.yml` | 単独 | awase-windows 全体(`--jobs 8`) | workflow_dispatch のみ | self-hosted(GCP Spot) |
| `mutants-actuation-confluence-windows.yml` | 単独 | 合流点 5 関数(13 mutants) | workflow_dispatch のみ | windows-latest。冒頭コメントに「棚卸しが終わったら削除してよい」 |
| `mutants-scope-investigation.yml` | 単独 | `.cargo/mutants-bug158-scope.toml`(ADR-194 の再挑戦条件検証用 4 ファイル) | workflow_dispatch のみ | windows-latest。冒頭コメントに「検証が終わったら削除してよい」 |
| `mise run mutants` | `mise.toml:30` | ルート crate | 手動 | ローカル |

- PR で `mutants`・`mutants-awase-windows` が skipping になるのは、上の `if` で workflow_dispatch 以外を除いているため(設計どおり。`ci.yml:525-527` のコメント「フル実行は数十分かかるため、通常の push/PR ゲートには含めない」)。例: `gh pr view 528 -R cuzic/awase --json statusCheckRollup --jq '.statusCheckRollup[]|select(.name|test("mutants"))|[.name,.conclusion]|@tsv'` → 2 ジョブとも `SKIPPED`。
- awase-windows の mutants を ubuntu で回せないのは、`crates/awase-windows/examples/*.rs` の一部が `windows` crate を cfg なしで参照し、ベースラインのビルドが E0433 で落ちるため(`ci.yml:567-578` のコメント、`mutants-scope-investigation.yml` 冒頭のコメント、メモリ `project_mode_key_pass_latch_mutation_coverage_task_2026_09_23.md` で再確認済み)。
- schedule トリガーを持つワークフローは今は 1 本も無い(`grep -rn 'schedule:' .github/workflows/` → 0 件)。デフォルトブランチは `develop`(`gh repo view cuzic/awase --json defaultBranchRef`)なので、schedule は develop で動く。リポジトリは public(`isPrivate: false`)で、GitHub-hosted runner は無料。
- cargo-mutants は 27.1.0(9/23 の成果物の `outcomes.json` の `cargo_mutants_version`)。テストランナーは既定の `cargo test`(`test_tool` 指定なし。`ci.yml:361-364`)。

### 2. CORE_MODULES と mutants の登録の差

```sh
sed -n '/^const CORE_MODULES/,/^];/p' crates/awase-windows/tests/layer_boundary_guard.rs | grep -c '^    "'   # 57
grep -cE '^\s+"crates/awase-windows/src/state/[a-z_]+\.rs"' .cargo/mutants-awase-windows.toml           # 22
```

- `CORE_MODULES`(`layer_boundary_guard.rs:495-553`)は 57 件。うち `examine_globs` に `state/<名前>.rs` の完全一致で載るのは **19 件**、載っていないのは **38 件**(タスク表 V1b の「52 件のうち 15 件」は develop `bbb70f29` 時点の数で、その後の F 系 PR で両方増えた)。
- 載っていない 38 件のうち本番コードが大きいもの(先頭の `#[cfg(test)]` までの行数。コメント込み): `explicit_press` 1191 行(ADR-208 の押下 ID、合流点ファミリー)、`actuation_chain` 688 行、`key_effect_table` 657 行、`ime_actuation_decision` 378 行、`mode_key_pass` 304 行(fix-requires-evidence の物理キーラッチ・belief ファミリー)、`open_warrant` 242 行、`platform_state` 1626 行(`ImeStateHub`、belief ファミリー)。38 件の合計は約 9,200 行。
- 逆に `examine_globs` にあって `CORE_MODULES` に無い `state/` は `ime_event`・`ime_model`・`probe_admission`(いずれも `NOT_CORE_MODULES` か対象外。`state/` 以外の `scanmap.rs`・`focus/cache.rs` 等 7 件も CORE ではない)。

### 3. 定期的に回っていない: 最後の完走は 9/23 の 1 回

```sh
gh api repos/cuzic/awase/actions/runs/35879891879/jobs --jq '.jobs[]|select(.name|test("mutants"))|[.name,.conclusion,.started_at,.completed_at]|@tsv'
git log origin/develop --format='%h %ad %s' --date=short --since=2026-09-23 -- .cargo/mutants-awase-windows.toml
```

- `mutants-awase-windows` が完走したのは run 35879891879(2026-09-23、ブランチ `ci/mutants-parallel-jobs`、4 shard)だけ。2026-10-06 の workflow_dispatch(run 37422836301、`experiment/monad-style-decision`)は全 shard `cancelled`。8 shard 化(`e78fc582`)後に完走した run は無い。
- 9/23 以降に `examine_globs` へ足されたファイルは 9 件(`physical_disposition`・`ime_set_open_plan`・`drift_plan`・`msaa_role_plan`・`relay_plan`・`deferred_gate_plan`・`focus_classify_plan`・`focus_resolve_plan`・`drift_correction`。コミット `71b7bbff`〜`e133c730`、10/05〜10/06)。**これらは一度も変異テストを受けていない。** V1(#530)は「載せたか」を検査するが、載せたものを回す仕組みが無い。
- #528(F6b)の PR 本文にも「mutants は CI で skipping のため未実行」とある。

### 4. 9/23 の run の結果と、その後の扱い

成果物(`gh run download 35879891879 -R cuzic/awase -p 'mutants-report*'`)の `outcomes.json` の集計:

| 対象 | 総数 | caught | missed | timeout | unviable | 1 shard の所要 |
| --- | --- | --- | --- | --- | --- | --- |
| awase-windows(当時の許可リスト 20 ファイル、4 shard) | 624 | 502 | **52** | 0 | 70 | 56〜60 分 |
| ルート awase crate(4 shard) | 1194 | 908 | **101** | 1 | 184 | 6.5〜8 分 |

- awase-windows の missed 52 件の内訳(ファイル別): `state/hook_state.rs` 10、`focus/cache.rs` 9、`single_thread_cell.rs` 8、`state/observation_store.rs` 7、`state/probe_admission.rs` 5、`state/ime_model.rs` 5、`state/belief.rs` 4、`lifetime_counter.rs` 2、`state/conv_mode.rs` 1、`focus/class_names.rs` 1。**CORE_MODULES に入るものは 22 件**(hook_state・observation_store・belief・conv_mode)。例: `focus/cache.rs:69:50: replace < with <= in FocusCache::get`(TTL の境界)、`single_thread_cell.rs:40:9: replace SingleThreadCell<T>::set with ()`。
- 全 shard のジョブは `failure`。cargo-mutants は missed があると終了コード 2 を返すため(cargo-mutants の文書による。下の「リスク」参照)、missed がある限り毎回赤になる。
- **この 52 件(と root 側 101 件)をテストに変えたコミットは無い**(`git log origin/develop --since=2026-09-23T16:00 -i --grep='missed\|35879891879'` に該当なし)。成果物は出たが、読む人と手順が無かった。

### 5. 過去に mutants が穴を見つけた実績

`git log origin/develop --oneline -i --grep=mutant | wc -l` → 71 件。実際にテストを足したものは次の 3 回。

- 2026-07-24 初回フル実行(run 30070606789、661 mutants、missed 127)→ triage して `def786eb`・`28315441` で回帰テスト追加、ログ専用の 19 件と等価変異を理由つきで `exclude_re` へ(`.cargo/mutants.toml` のコメント)。
- 2026-07-25 `mutants-windows.yml` のベースラインで、Windows で初めて走ったテストの不具合 15 件(実装バグ 1 件 = BUG-41)が見つかった(`ci.yml:345-352` のコメント)。
- 2026-09-23 `ModeKeyPassLatch` の missed 22 件 → テスト 7 件で 1 件(等価変異として確定、`mode_key_pass.rs:158:28`)まで減らした(PR #248、`d8653da5`、run 35819375184、95 mutants、45 分)。続く `drift.rs`/`refresh_plan.rs` の 4 ファイル・131 mutants は `timeout-minutes: 60` ちょうどで cancelled(run 35849828981。メモリ `project_mode_key_pass_latch_mutation_followup_2026_09_23.md`)。**教訓: 対象を増やすと時間は mutants の数に比例して伸びる。timeout は件数から見積もる。**

どの回も「人が対象を決めて手動で回し、その場で読んだ」ときだけテストに変わっている。

### 6. 今日の Opus レビューが人手で見つけた穴は、mutants で見つかったか(推定)

| PR | 人手で見つけた穴 | 置き場所 | mutants で見つかるか(推定) |
| --- | --- | --- | --- |
| #513 | コメント除去の字句走査 `strip_comments` で、`/*` を含むコメントがあると本番コードが消えてガードが素通りする(再確認 M1)、接頭辞つき生文字列 | `tests/architecture_guard.rs`(統合テスト) | **見つからない**。cargo-mutants が変異させるのはライブラリ・バイナリのソースで、`tests/` の検査コード自体は変異させない(推定。cargo-mutants の対象選択の仕様による) |
| #528 | 殻の呼び出しで `is_post_unicode_pending` 等の引数を取り違える・定数を差し替える | 殻 `output/vk_send.rs`(`#[cfg(windows)]`、許可リスト外) | **見つからない**。引数の入れ替えは cargo-mutants の変異の種類に無い(関数本体の置き換え・二項演算子・単項演算子・match の腕・構造体フィールドの削除など)。殻は許可リストの外でもある |
| #528 | `plan_warmth` の境界(`>` と `>=`、`T`/`T+1`) | 核 `state/warm_send_plan.rs` | **見つかる**(`replace > with >=` が典型)。PR は手で境界値テストを書いて固定済みなので、mutants は「その手書きが足りているか」の機械的な再確認になる |
| #530 | `CORE_MODULES` ブロックの構文解析の取りこぼし | `crates/xtask-adr-evidence/src/core_registry.rs`(bin crate) | 一部は見つかる可能性がある(比較演算・早期 return の削除)。ただし xtask は許可リストの外で、今回の対象でもない |

結論(推定): 今日の穴の多くは**検査する側のコード**(ガード・xtask)と**殻の配線**にあり、mutants の守備範囲の外。mutants が埋めるのは**核の純粋関数の境界・分岐に対するテスト不足**で、レビューとは重ならない別の穴。人手のレビューの代わりにはならない。

### 7. 実行時間とコスト

- Windows: 9/23 は 4 shard × 156 mutants で 1 shard 56〜60 分 → `--jobs 2` で 1 mutant あたり約 22 秒(ベースラインのビルドだけで 131 秒。`ci.yml:595`)。
- ルート crate(ubuntu): 1 shard 約 300 mutants で 7 分前後(`CARGO_INCREMENTAL=1`)。Windows より 1 桁速い。
- 第 1 段階の見積もり(推定): 9/23 の 624 件 + その後の 9 ファイル(件数未確認、0.1 件/行で数十〜100 件程度)+ `mode_key_pass` 約 59 件(9/23 の scope run で force_guard と合わせ 95 件、full run の force_guard 36 件を引いた数)+ `explicit_press` 約 70〜120 件(本番コードのうちコメントを除く約 700 行 × 0.1〜0.17 件/行)= **約 800〜900 件**。8 shard なら 1 shard 100〜115 件 × 22 秒 ≒ **40〜45 分**、合計 runner 時間 6 時間弱(public なので課金なし)。
- `CORE_MODULES` 全体(38 件・約 9,200 行を追加)まで広げると、追加分だけで推定 900 件前後、合計 1,700 件前後。8 shard では 1 shard 80 分前後で 90 分上限に近い → 16 shard が要る(推定)。

## 決定

### D1: `mutants-awase-windows` ジョブを定期実行専用のワークフローへ移す

- 新しい `.github/workflows/mutants-scheduled.yml` を作り、`ci.yml` の `mutants-awase-windows` ジョブ(`ci.yml:564-605`、コメント込み約 45 行)を**移す**(`ci.yml` からは消す)。トリガーは `schedule`(頻度は所有者に聞く。推奨は週 1 回、日曜の日本時間早朝)と `workflow_dispatch`。`push`/`pull_request` は持たない。
- ルート crate の `mutants` ジョブは `ci.yml` に残す(`ci.yml` の workflow_dispatch はブランチで CI 全体を回す用途にも使われている。例: run 37422836301)。定期実行に含めるかは所有者に聞く。
- ランナー・shard 数・`CARGO_INCREMENTAL=1`・成果物名は今のジョブのまま。`timeout-minutes` は第 1 段階の件数見積もり(1 shard 115 件 × 22 秒 ≒ 42 分)から 90 分のまま据え置く。

### D2: missed があってもジョブは赤にしない。赤はベースライン失敗とタイムアウトだけ

- cargo-mutants の終了コードで分ける: 0(全 caught)と 2(missed あり)は成功扱い、それ以外(ベースライン失敗・タイムアウト・使い方の誤り)は失敗。今のままだと missed がある限り毎回赤になり(9/23 は全 shard `failure`)、赤が「壊れた」と「いつもの生き残り」の区別を失う。
- shard の `missed.txt` を集めるジョブを 1 つ足し、ファイル別の件数と全行を `$GITHUB_STEP_SUMMARY` に出し、1 つの成果物 `mutants-missed-summary` にまとめる。行番号は変わるので、前回との比較は「ファイル名 + 関数名 + 変異の説明」(行:列を落とした文字列)で行う。比較は第 2 段階まで人がやる。

### D3: 生き残りの扱い: Issue を自動起票しない。人がテストに変えるか、理由つきで除外する

- 自動起票しない理由: 9/23 の 52 件は成果物として残っていたのに誰も読まなかった。件数が多いときに 1 件 1 Issue にすると、読まれない Issue が増えるだけになる。
- 運用: 定期 run の Step Summary を見て、**CORE_MODULES のファイルの missed を優先して**、1 回につき上限 N 件(所有者に聞く。推奨 10 件)を次のどちらかにする。
  - (a) テストを足す(`state/` の当該ファイルの `#[cfg(test)]` に。#248 と同じ型)。
  - (b) 等価変異・ログ専用なら、`.cargo/mutants-awase-windows.toml` の `exclude_re` に理由のコメントつきで載せる(`.cargo/mutants.toml` の既存の書き方。行番号つきの除外は行がずれると効かなくなるので、関数名で絞れるときは関数名で)。
- CORE でないファイル(`scanmap.rs`・`focus/cache.rs`・`single_thread_cell.rs` 等)の missed は後回しにしてよい。

### D4: PR では回さない

- 理由: Windows で 1 mutant 約 22 秒、第 1 段階で約 850 件。8 shard でも 1 shard 40 分以上かかり、PR のたびに回すと待ちが長い。FCIS の PR は「全チェック完了後にマージ」の運用(メモリ `project_ci_bug_repro_campaign_2026_10_04.md`)なので、必須でないジョブでもマージを遅らせる。
- 差分だけを回す `cargo mutants --in-diff` を PR で使う案は「却下した案」に書く(第 3 段階で再検討の余地あり)。

### D5: V1(#530)との関係と V1b

- V1 は「PR で `CORE_MODULES` に足した名前が `examine_globs` にも載っているか」だけを見る。本 ADR の定期実行で、**載せたものが実際に回る**ようになり、V1 の検査に意味が生まれる。V1 は当面必須チェックにしない(所有者の決定済み)を変えない。
- V1b(`examine_globs` を `CORE_MODULES` に揃える)は第 2 段階で行う。第 1 段階で実行時間と生き残りの数を測ってから決める。
- 揃い切ったら、V1 の差分検査(`core_registry.rs` 174 行 + CI ジョブ `core-registry-consistency` + base の取得)を、`layer_boundary_guard.rs` に足す「`CORE_MODULES` の全名が `examine_globs` に載っている」という全数テスト(20 行程度の見込み)に置き換えられる。差分を読まないので develop への直接 push も拾え、V1 の穴(タスク表 V1 行「develop への直接 push を拾えない」)が消える。ただしこれは `test` ジョブの中に入るので、事実上の必須化になる。所有者に聞く(下記 Q4)。

## 段階

| 段階 | やること | 撤去対象 | 検証方法(CI で確認できる形) | 取りやめ条件 |
| --- | --- | --- | --- | --- |
| **S0**(コード変更なし) | 今の develop で `gh workflow run ci.yml --ref develop` を 1 回起動し、8 shard の `mutants-awase-windows` の完走時間と missed を測る(9/23 以降の 9 ファイルを初めて回す) | なし | 8 shard とも完了(`failure` は missed ありの意味で可)、成果物 8 個、`outcomes.json` の `timeout` 0 | 1 shard が 90 分を超えて cancelled → shard 数を見直してから S1 |
| **S1**(最小の第 1 段階) | D1・D2 を実装。`examine_globs` に `state/mode_key_pass.rs` と `state/explicit_press.rs` を足す。`mode_key_pass.rs:158:28` の等価変異の除外を `.cargo/mutants-bug158-scope.toml` から移す | `ci.yml` の `mutants-awase-windows` ジョブ(約 45 行)、`.github/workflows/mutants-scope-investigation.yml`(61 行)、`.cargo/mutants-bug158-scope.toml`(52 行)。両ファイルは冒頭に「検証が終わったら削除してよい」とあり、対象の検証は #248・`f54c058a` で完了済み。差し引き行数は減る | `workflow_dispatch` で 1 回起動し、(1) missed があってもジョブが green、(2) Step Summary にファイル別件数が出る、(3) `mode_key_pass.rs` の missed が 0(9/23 の scope run で 1 件まで減らし、その 1 件は除外済みのため)。次の schedule の run が自動で起動することを Actions の一覧で確認 | 1 shard 70 分超、または `explicit_press` の missed が 30 件を超えて 1 回の運用(D3 の上限)で減らせない → `explicit_press` を外して `mode_key_pass` だけで続ける |
| **S2** | 定期 run を 4 回(週次なら 1 か月)続け、D3 の運用で CORE の missed を減らす。9/23 の CORE 分 22 件(hook_state・observation_store・belief・conv_mode)のうち今も残るものを先に片づける | なし(テストが増える) | 4 回の Step Summary で CORE の missed の件数が単調に減る | 4 回たっても CORE の missed が減らない(誰も読んでいない)→ 定期実行をやめ、ワークフローを workflow_dispatch のみに戻す(9/23 と同じ状態に戻るだけで、害はない) |
| **S3** | V1b: 残りの CORE 36 件を数回に分けて `examine_globs` に足す(大きい順。shard を 16 に増やすか、CORE だけの別 config に分けるかは S1・S2 の実測で決める) | V1 の差分検査(`core_registry.rs`・`core-registry-consistency` ジョブ)を全数テストに置き換える(所有者が承認した場合のみ) | `CORE_MODULES` ⊆ `examine_globs` の全数テストが green。定期 run の 1 shard が 90 分以内 | 1 shard が 90 分を超え、shard を増やしても下がらない → 大きいファイル(`platform_state` 等)を外したまま止める |

## 却下した案と理由

- **PR ごとにフルで回す**: D4 のとおり、Windows で 1 shard 40 分以上。マージを遅らせる。
- **PR で `cargo mutants --in-diff` だけを回す**: 変更直後に作者が読めるので、「読む人がいない」問題には効く。しかしベースラインのビルドだけで 131 秒かかり、FCIS の PR は核の新ファイル(数百行)を含むことが多いので 1 PR で数十件・10〜20 分になる(推定)。FCIS の PR は全チェック完了を待つ運用で、待ちが増える。S2 で定期 run の生き残りが減ったあと、まだ読まれていない場合に再検討する。
- **生き残りを Issue に自動起票する**: D3 のとおり。9/23 の 52 件は成果物があっても読まれなかった。起票しても同じになる見込み。
- **missed が出たら赤にする(しきい値つきで落とす)**: 既に生き残りが 52 件ある状態では、しきい値を決めても毎回赤か、しきい値を上げ続けるかのどちらかになる。tuning 定数の「とりあえず上げる」と同じ型になる。
- **ubuntu-latest に戻して速く回す**: ルート crate では Windows より 1 桁速い。しかし awase-windows は examples のビルドで落ちる(背景 1)。examples を `required-features` で外せば通るかもしれないが未確認で、コードの変更が要る。S3 で shard が足りなくなったときの選択肢として残す。
- **self-hosted(`mutants-windows.yml`)で回す**: runner の起動が手作業(GCP Spot、トークン発行)で、定期実行にできない。最後の run(36979815700、10/02)は cancelled。本 ADR では触らない。
- **除外リストや例外の印を新しく作る**: ADR-218〜220 と同じ型になるので作らない。除外は既存の `exclude_re` に理由のコメントつきで書くだけ。

## リスクと限界

- cargo-mutants の終了コード(0 = 全 caught、2 = missed あり、3 = タイムアウト、4 = ベースライン失敗)は文書の記憶によるもので、**未確認**。S1 の実装時に cargo-mutants 27.x の文書で確かめる。9/23 の全 shard が `failure` だったことは「missed で非 0」と矛盾しない。
- 9/23 以降に足された 9 ファイルの mutants 件数は**未確認**(S0 で分かる)。第 1 段階の 40〜45 分は推定。
- 8 shard の構成は一度も完走していない(背景 3)。S0 で確かめる。
- 9/23 の missed 52 件が今も残っているかは**未確認**(その後テストが増えたファイルもある)。
- mutants は核の境界・分岐のテスト不足しか見ない。ガードの検査コード、殻の配線、引数の取り違えは見ない(背景 6、推定)。Opus のレビューを減らす根拠にはならない。
- schedule は、public リポジトリで 60 日間活動が無いと GitHub が止める。今の開発ペースでは当たらない。
- `cargo test` を使うので、`TSF_OBS_TEST_LOCK` に頼る並列の汚染の防御は今と同じ(`ci.yml:359-364`)。変えない。
- windows-latest の同時実行数の上限(無料枠)に 8〜16 shard が当たり、同時刻の他の CI が待たされる可能性がある。schedule を利用の少ない時間帯に置く。

## 所有者に聞くこと

- **Q1 頻度とコスト**: (a) 毎晩、(b) 週 1 回、(c) 手動のまま(本 ADR をやめる)。**推奨 (b)**。public なので費用はかからないが、読む側の手が週 1 回分しかない見込み。毎晩回しても、コードが変わらない日は同じ結果が出るだけ。
- **Q2 対象の範囲**: (a) 第 1 段階は 2 モジュール(`mode_key_pass`・`explicit_press`)だけ足す、(b) 最初から CORE 全体(V1b を同時に)、(c) 今の 29 ファイルのまま定期化だけ。**推奨 (a)**。実行時間と生き残りの数を測ってから広げる。`mode_key_pass` は 9/23 に 1 件まで減らした実績があり、仕組みの検証に使える。
- **Q3 生き残りの扱い**: (a) D3 のとおり、Step Summary を人(またはエージェント)が見て 1 回 N 件までテスト化・除外、(b) Issue 自動起票、(c) 見るだけで義務にしない。**推奨 (a)、N = 10**。誰が見るか(所有者か、定期セッションのエージェントか)も決めてほしい。
- **Q4 V1 の置き換え**: V1b が揃ったら、V1 の差分検査を `layer_boundary_guard.rs` の全数テストに置き換えて xtask 側(`core_registry.rs` 174 行・CI ジョブ)を撤去するか。全数テストは `test` ジョブに入るので事実上の必須化になる(V1 を必須にしないという決定に触れる)。**推奨: S3 の時点で改めて判断**。
- **Q5 ルート crate の `mutants` ジョブ**: 定期実行に含めるか。9/23 に missed 101 件。ubuntu で 1 shard 7 分と安い。**推奨: S2 で CORE の運用が回ってから**。
- **Q6 `mutants-actuation-confluence-windows.yml` の撤去**: 冒頭に「棚卸しが終わったら削除してよい」とあり、棚卸しは 2026-09-23 に完了(`docs/tasks/actuation-confluence-inventory.md`)。ただし `docs/tasks/corpus-discard-impact-2026-10-06/README.md:150` が「② の後半に流用できる」と書いている。**推奨: コーパス置き換えの計画が使わないと決まったら S1 で一緒に撤去**。

## 関連する既存文書への追記案

(本 ADR の起草では他の文書を書き換えない。採用されたら、各段階の PR で次を追記する)

- `docs/tasks/fcis-layering-tasks-2026-10-06.md` の V1b 行: 「件数は develop `1630e55e` で CORE 57 件・登録 19 件。穴埋めは ADR-234 の S3」。
- 同 V2 の完了前チェック 1: 「`examine_globs` に載せたファイルは、次の定期 run(`mutants-scheduled.yml`)の Step Summary で missed を確認する」。
- `ci.yml` の `workflow_dispatch` のコメント(`ci.yml:8-9`): `mutants-awase-windows` が移ったことを反映。
- `.cargo/mutants-awase-windows.toml` の冒頭: 「定期実行は `mutants-scheduled.yml`。生き残りは ADR-234 D3 の手順で扱う」。
- `docs/tasks/mode-key-pass-latch-mutation-coverage-followup.md`: `mutants-scope-investigation.yml` と `mutants-bug158-scope.toml` を S1 で撤去したことを 1 行。
