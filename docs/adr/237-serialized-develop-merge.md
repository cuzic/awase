---
id: ADR-237
title: |-
  develop へのマージ担当の手順を決める(古い base のままマージし、競合しない rebase をやめ、pre-push の `*_plan` 列挙を 1 パターンにする)
summary: |-
  2026-10-06 は 1 日に 46 本の PR(#492〜#537)が develop 向けに作られ、同時に開いていた PR は最大 9 本。マージは team-lead が 1 人で行っていた。
  force-push 29 回のうち 28 回は新しい develop への rebase で、再生して確かめられた 21 回のうち 10 回は競合し、11 回は競合していなかった
  (develop にブランチ保護が無く、古い base のままでもマージできるのに最新化していた)。
  マージ済み 38 本のうち 32 本は古い base のままマージされたが、develop の push で走った `CI` ワークフローの失敗は 4 回で、全て report-worker の外部 API 403。
  競合の原因はマージの並列ではなく開発の並列と、登録用の 1 行(pre-push の正規表現など)への書き込みの集中。マージはすでに直列なので、直列化では競合は減らない。
  決定: (b) マージ担当の手順を書く(古い base でも CONFLICTING でなければそのままマージ、検出は develop の push CI と revert に任せる、
  競合しない rebase を頼まない)。撤去対象は「競合しないのに最新化する rebase」。(c) pre-push の正規表現で `*_plan` の列挙を `[a-z_]+_plan` の 1 パターンにする。
  GitHub merge queue は個人所有のリポジトリでは文書上対象外の見込み。ブランチ保護(strict)は推奨しない。
status: |-
  提案(Opus round1 の Must 1〜4・Should 1〜6・Nit 1〜4 を反映、再確認前)
related_adr:
  - "ADR-158"
  - "ADR-162"
  - "ADR-229"
  - "ADR-236"
  - "ADR-239"
---

# ADR-237: develop へのマージ担当の手順を決める

## 背景(実測、2026-10-06、develop `1630e55e`)

### 1. 並列度と、rebase・競合の実数

数えた範囲は 2026-10-06(UTC)に作られた PR #492〜#537 の 46 本(マージ 38、オープン 6、クローズ 2)。

| 量 | 値 | 再確認のコマンド |
| --- | --- | --- |
| 同時に開いていた PR の最大 | 9 本(04:11 UTC) | 下の「再確認の手順」1 |
| PR が開いていた時間の中央値 | 26 分(最大 92 分) | 同上 |
| force-push(`HeadRefForcePushedEvent`) | 29 回 / 19 本(#528 が 5 回で最多) | 同 2 |
| そのうち新しい develop への rebase | 28 回(base が動かない amend は 1 回) | 同 3 |
| rebase を再生して競合の有無を確かめられたもの | 21 回 | 同 4 |
| そのうち実際に競合したもの(rebase 単位) | **10 回** | 同 4 |
| そのうち競合していなかったもの | **11 回** | 同 4 |

競合した 10 回の rebase で、競合したファイル(**ファイル単位の回数**。1 回の rebase に複数ファイルが入る):

| ファイル | 回数 | PR |
| --- | --- | --- |
| `.githooks/pre-push`(対象正規表現の 1 行、36 行目) | 5 | #517、#519、#528×2、#535 |
| `.claude/rules/fix-requires-evidence.md`(再発ファミリー表) | 3 | #519、#528、#535 |
| `crates/awase-windows/tests/layer_boundary_guard.rs` | 3 | #498(冒頭コメント)、#504(`CORE_MODULES`)、#519 |
| `crates/awase-windows/tests/architecture_guard.rs` | 1 | #517 |
| `.cargo/mutants-awase-windows.toml`(`examine_globs`) | 1 | #519 |
| `docs/tasks/fcis-layering-tasks-2026-10-06.md` | 1 | #528 |
| `docs/known-bugs/BUG-074.md` | 1 | #526 |
| 本番コード(`runtime/executor.rs`・`state/ime_set_open_plan.rs`、`state/drift_*.rs`) | 2 | #504、#532 |

再生できなかった 7 回(PR のコミット数が rebase の前後で変わり、新しい base を機械的に特定できなかったもの)は数に入れていない。
#528 の 07:33 の回では `architecture_guard.rs` と `.githooks/pre-push` の競合も出ているが、同じ理由で表から外した。
つまり 10 回は下限。

pre-push が競合した 5 回の rebase は、全て同じ rebase で別の登録ファイルも競合している(#517 は `architecture_guard.rs`、
#519 は fix-requires・`layer_boundary_guard.rs`・`examine_globs`、#528 の 2 回は fix-requires と `docs/tasks/...`、#535 は fix-requires)。
したがって pre-push の行の競合を消しても、**手で解く rebase の回数はほぼ減らない**。減るのは競合の hunk の数と、
#535 型のマーカーの消し忘れが起きうる場所の数(後述 §3)。

### 2. 競合が起きる構造(ファイルごと)

- **`.githooks/pre-push` 36 行目**: `local target='(...state/(ime|conv_mode|...|drift_plan|drift_correction|relay_plan|deferred_gate_plan|focus_classify_plan|focus_resolve_plan|msaa_role_plan)|...)'`
  の 602 文字の 1 行。今日この行は 6 コミットで書き換わった(`git log --oneline --since=2026-10-06T00:00 origin/develop -- .githooks/pre-push`)。
  足された名前は `relay_plan`(`bfdceb08`)・`deferred_gate_plan`(`fc8824de`)・`focus_classify_plan` と `msaa_role_plan`(`55753781`)・
  `drift_correction`(`2cd99845`)・`focus_resolve_plan`(`e133c730`)で、残る 1 つ(`1630e55e`)はマーカーの削除。オープン中の
  #519 は `focus_gen`、#528 は `warm_send_plan` を足している。6 回のうち 4 回、オープン中を含めると 7 回のうち 5 回が `*_plan`。
  `xtask-adr-evidence`(`crates/xtask-adr-evidence/src/main.rs:322-336`)は `local target=` で始まる最初の 1 行だけを読むので、
  行を複数行に分けるには xtask の変更も要る。
- **`fix-requires-evidence.md` の表**: 1 セルが 1 行で、長い行は 3,475 文字(147 行目)。モジュールを足す PR はセルの末尾に書き足す。
- **`CORE_MODULES`**(`layer_boundary_guard.rs:495-`): すでに 1 行 1 要素・昇順。#504 の衝突は隣り合う名前(`ime_read_strategy` の次の
  `ime_set_open_plan`)を 2 本が同時に足したため。
- **`examine_globs`**(`.cargo/mutants-awase-windows.toml:28-49`): 1 行 1 要素だが昇順でなく、末尾に足す運用。今日足された 8 行は全て末尾。
- **`state/mod.rs`**(今日 14 本が触った)と **`architecture_guard.rs`**(7,201 行、今日 14 本が触った): 再生で確認できた競合は
  それぞれ 0 回と 1 回。

### 3. 古い base のままのマージと、その実害

- マージ済み 38 本のうち **32 本は、マージした時点の develop より古い base のまま**マージされた(1〜26 コミット遅れ。#530 は squash なので ±1 本の誤差)。
  CI は最後の push の時点の `refs/pull/N/merge` で走るので、この 32 本はマージ後の develop と同じ内容では PR の CI を通っていない。
- 2026-10-06 の develop への push で走った `CI` ワークフロー 62 回のうち失敗は 4 回で、**4 回とも `report-worker` ジョブの
  GitHub API 403**(`latest release fetch failed UpstreamFetchError: ... 403`。run 37411947874・37416879966 などで、ほかのジョブは全て success)。
  `CI` の Rust のビルド・テスト・ガードが develop で落ちたことは 0 回。
- ただし `e2e-ime-smoke` は develop の push 37 回のうち 18 回が `cancelled`(後続のマージで取り消し)で、半分のマージ後の状態は e2e で検査されていない。
- 今日 develop に入った害は **#535(F5c)の diff3 マーカー 1 件**だけ: `.githooks/pre-push` と `fix-requires-evidence.md` に
  `||||||| parent of 9c9a1164 ...` の行と、**古い `local target=` 行がまるごと 1 行**残った(`1630e55e` で削除。マージ 07:30 UTC → 修正 07:41 UTC)。
  マーカーは PR の head `e133c730` にすでに入っていた(`git grep '^|||||||' e133c730 -- .githooks/pre-push .claude/rules/fix-requires-evidence.md` で 2 件)。
  原因は「競合を手で解いたときの消し忘れ」で、古い base とは関係がなく、merge queue でも防げない。CI の検出は ADR-236 が扱う。
  過去の例(`docs/known-bugs/BUG-172.md` に 09-29 の `ab368de8` から 6 日間残ったマーカー)は ADR-236 を参照。

### 4. 費用の見積もり

- PR の CI(`pull_request`)は 2026-10-06 に `CI` 119 回・`e2e-ime-smoke` 115 回。1 回あたりおよそ 6 分(729 分 / 119 回)と 5.8 分(669 分 / 115 回)。
- rebase 1 回で CI を 2 本ずつ再実行するので、rebase 28 回で **約 330 ランナー分**。そのうち**競合していない 11 回で約 130 ランナー分**。
- エージェントの時間(rebase して push し直すまで)は**未確認**。#528 は 06:29〜07:41 の 72 分に rebase 5 回(CI 待ちを含む)。

### 5. 誰がマージしているか

今日のマージは team-lead が 1 人で行った(team-lead の説明。ADR-239 の summary「実装エージェントが worktree ごとに PR を作り、Opus がレビューし、
team-lead がマージ」と同じ)。`gh` の記録では 38 本全てが `cuzic` アカウントで、誰がマージしたかは区別できない。

つまり**マージはすでに直列で、その状態で競合が 10 回起きた**。競合の原因はマージの並列ではなく、開発の並列(同時に最大 9 本)と、
登録用の 1 行への書き込みの集中。マージの直列化で競合は減らない。

自動マージ許可(メモリ `project_adr229_fcis_layering_2026_10_06.md`)の条件「mergeStateStatus=CLEAN」は、develop にブランチ保護が無い
(`gh api repos/cuzic/awase/branches/develop/protection` → 404)ため、base が最新であることを意味しない。古くても競合が無ければ CLEAN になり、
GitHub はそのままマージできる。それでも競合しない rebase が 11 回あったのは、習慣か指示によるもの(ADR-239 の決定 1「完了前チェック」の
1 項目目は「最新の `origin/develop` に rebase してあるか」)。

## 決定

### (b) マージ担当の手順(運用の約束)

1. マージは今どおり 1 人(team-lead)が行う。
2. **base が古くても、`gh pr view N --json mergeable` が `CONFLICTING` でなければそのままマージする。** 最新の develop での CI の再実行は求めない。
3. **検出は develop の push CI に任せる。** develop の push で `CI` の Rust のジョブが落ちたら、マージ担当は原因のマージを `git revert` して
   作者に戻す(develop で直さない)。
4. **競合しない rebase を頼まない。** 実装エージェントは `CONFLICTING`/`DIRTY` になったときだけ自分のブランチで `git rebase origin/develop` する。
   ADR-239 の完了前チェックの 1 項目目は「`CONFLICTING`/`DIRTY` でないこと」に絞る(ADR-239 側で決め、この ADR は参照にする)。
5. 競合を解いた後は、push の前に ADR-236 の衝突マーカーの検査を通す(検査の定義は ADR-236 の 1 か所に置き、ここには書かない)。

- 撤去対象: 「競合しないのに最新化する rebase」(今日 11 回、約 130 ランナー分)。
- 行数の増減: コードは 0 行(運用の約束のみ)。
- V1(`core-registry-consistency`)との関係: V1 は「PR が足した名前が mutants に載っているか」を PR の CI の時点の `origin/develop` と比べて見るだけなので、
  古い base のままマージしても判定は変わらない。V1 のために最新の base は要らない。

### (c) 第 1 段階: pre-push の正規表現で `*_plan` を列挙しない

`.githooks/pre-push` 36 行目の `state/(...)` の中の
`|drift_plan|drift_correction|relay_plan|deferred_gate_plan|focus_classify_plan|focus_resolve_plan|msaa_role_plan`
のうち `*_plan` の 6 つを、`|[a-z_]+_plan` の 1 つに置き換える(`drift_correction` は残す)。

- 撤去対象: 「新しい `state/*_plan.rs` を作るたびに pre-push の正規表現の 1 行を書き換える」手順。
- 効果: 競合は両側がこの行を書き換えたときだけ起き、今日の 5 回はどれも片側が `*_plan` を足していたので、pre-push の行の競合 hunk は 0 になる見込み。
  ただし §1 のとおり、同じ rebase で fix-requires の表などほかの登録ファイルも競合しているので、**手で解く rebase の回数はほぼ変わらない見込み**。
- 行数: 36 行目を 1 行書き換えるだけ(行数 ±0、文字数は約 70 文字減)。経緯のコメントを 2 行足す(+2 行)。
- 効果の副産物: 現在どこにも載っていない `state/focus_probe_plan.rs`(`grep -c focus_probe_plan .claude/rules/fix-requires-evidence.md` → 0、
  正規表現にも無い)も対象に入る。`ime_set_open_plan` は既に `state/(ime` の前方一致で入っている。
- xtask との関係: `xtask-adr-evidence` の TC3(`main.rs:312-383`)は「表に書いたパスが正規表現にアンカーなしで当たるか」を見る向きなので、
  正規表現を広げても落ちない(表の `_plan` のパス 7 つはいずれも `state/[a-z_]+_plan` に当たる)。V1(`core-registry`)は pre-push を見ない(`core_registry.rs:6`)。
- 誤検出の側: `[a-z_]` は `/` を含まないので `state/` 直下に限られる。前方一致なので `state/foo_planner.rs` も当たるが、pre-push は警告だけでブロックしない。

fix-requires の表(3 回)はこの段階では変えない。

## 却下した案と理由

- **マージ担当を新たに 1 人に絞る(初版の (b)-1)**: §5 のとおり、すでに 1 人だった。撤去対象が存在しない。
- **登録ファイルを触る PR だけ、最新の develop での CI green を待ってからマージする(初版の (b)-2)**: 今日のマージ済み 38 本のうち 25 本が登録ファイルを触り、
  そのうち 21 本が古い base だった(Opus round1 の再確認)。FCIS の PR はほぼ全部が登録ファイルを触るので、実質的に strict 保護と同じ型の費用
  (21 回 × CI 2 本 × 約 6 分 ≒ 250 ランナー分/日、加えてマージ担当の待ち行列 2〜4 時間)を、実害 0 件のまま課すことになる。
  「起きない保証は無い」を理由に機構を足すのは、実害の記録に結びつく項目だけ採る方針(メモリ `feedback_dont_pile_complexity_in_response_to_adversarial_review.md`)に反する。
  取りやめ条件の側に回す(段階の表の段階 0)。
- **GitHub merge queue**: GitHub の文書では、merge queue の対象は「組織が所有する公開リポジトリ、または GitHub Enterprise Cloud を使う組織の非公開リポジトリ」。
  `gh api repos/cuzic/awase --jq .owner.type` は `User` なので、**文書上は対象外**。所有者が設定画面に項目が無いことを確かめれば選択肢から外す。
  なお merge queue が減らすのは strict 保護と組み合わせたときの「競合しない rebase」で、それは保護を入れなければそもそも不要(決定 (b)-4 で消す)。
  テキストの競合そのものは減らさない。
- **「マージ前に base を最新にすることを必須にする」ブランチ保護(strict status checks)**: 個人リポジトリでも使える。ただし 32/38 本が古い base だったので、
  rebase と CI の再実行が 1 日あたり約 32 回増え(約 380 ランナー分)、その対価で防げた実害は今日 0 件。推奨しない。
- **pre-push の正規表現を 1 行 1 モジュールの配列に分ける**: 衝突の根は消えるが、TC3 が `local target=` の 1 行を前提にしている(`main.rs:327`)ので
  xtask の変更が要り、行数が数十行増える。
- **`state/(...)` の列挙全体を `state/` にする**: 以後 `state/` 直下のどのファイルを足してもこの行を書き換えずに済み、`drift_correction`・`focus_gen` 型の競合も消える。
  費用は警告の増加だけで、警告を読む人がいるか自体が未確認(リスクと限界)。ただし `state/` には再発ファミリーでないファイル(`scoped_latch.rs` など)も多く、
  fix-requires の表の「対象を絞る」という意図を崩す。第 1 段階は `*_plan` だけにとどめ、`*_plan` 以外の名前で競合が続いたら段階 2 の候補にする。
- **`architecture_guard.rs` の末尾追記をガードごとの小関数・別ファイルにする**: 再生で確認できた競合は 1 回だけで、ファイル分割は
  ADR-218 round1 で「新ファイルが CI の `--test` 指定と xtask から漏れる」と指摘された。費用に見合わない。
- **`examine_globs` を昇順にそろえる**: 今日の確認できた競合は 1 回。段階 2 の候補に回す。
- **`CORE_MODULES` を 1 行 1 要素にそろえる**: すでにそうなっている。何もしない。
- 宣言テーブル化・DSL 化・汎用ツールキットは ADR-218〜220・229 で却下済みなので扱わない。

## 段階

| 段階 | 内容 | 撤去対象 | 検証(CI・コマンドで確認できる形) | 取りやめ条件 |
| --- | --- | --- | --- | --- |
| 0 | マージ担当の手順(決定 (b))の運用開始 | 競合しない rebase | 次の 40 本で、再生して競合なしの rebase が 0〜2 回か(再確認の手順 3・4)。develop の push CI の Rust のジョブの失敗のうち、古い base が原因のものの件数を数える | develop の push CI で `CI` の Rust のジョブが古い base 由来で落ちたら、そのとき競合したファイルを触る PR に限って最新の base での CI を求める |
| 1 | pre-push 正規表現の `*_plan` を `[a-z_]+_plan` に | `*_plan` 追加ごとの 36 行目の書き換え | `adr-evidence-consistency` が green。次の 40 本で、新しい `state/*_plan.rs` を足した PR が `.githooks/pre-push` を変更していないか(`gh pr list --state merged --json files`)、再生での pre-push の競合 hunk が 0 回か(手順 4) | 再発ファミリーでない `*_plan.rs` への警告が出て作業の妨げになったら、`*_plan` の列挙に戻す |
| 2(候補、未決) | `examine_globs` の昇順化、または `state/(...)` の `state/` 化 | 末尾追記による必ず起きる衝突、`*_plan` 以外の名前の追加による衝突 | 段階 1 後の 40 本で、それぞれの競合が 2 回以上あったときだけ着手 | 段階 1 後に競合が 0〜1 回なら着手しない |

## リスクと限界

- 競合の数は「force-push の前後のコミットを `git merge-tree`(trivial merge)で再生して `<<<<<<<` が出るか」で数えた。PR のコミット数が
  rebase の前後で変わったもの 7 回は数えていない(10 回は下限、11 回の「競合なし」も 21 回の中での数)。
- 「古い base でも実害 0 件」は 1 日分の develop の `CI` 62 回からの推定で、`e2e-ime-smoke` は半分(18/37)が取り消されていて検査されていない。
  決定 (b)-3 で develop の push CI を検出器にするので、e2e の取り消しが多いと、古い base 由来の e2e の失敗を見逃しうる。
- 第 1 段階で消えるのは「`*_plan` を足すとき」の競合だけ。`drift_correction`・`focus_gen` のような `*_plan` 以外の名前を足す PR 同士は、これからも 36 行目で衝突する。
- **pre-push の警告を見ている人がいるかは未確認**。`core.hooksPath` は `.git/config` で `/home/cuzic/rust-nicola/.githooks` という絶対パスなので、
  どの worktree から push しても実行されるのはメインの作業ツリーの版。pre-push は 98〜106 行目で `cargo clean`・`cargo xwin check` を走らせるので、
  ローカルでビルドしない方針(メモリ `feedback_no_local_builds_use_github_actions_2026_10_04.md`)のもとでエージェントは `--no-verify` を使っている可能性が高い。
  そうなら、この正規表現を実際に使っているのは TC3 だけ。
- エージェントの時間の損失は計測していない(**未確認**)。費用の見積もりはランナー分のみ。

## 所有者に聞くこと

1. **ブランチ保護(strict)を使うか**
   - (i) 使わない。決定 (b) の手順だけ(**推奨**。今日の実測で古い base の実害が 0 件。費用は約 380 ランナー分/日)。
   - (ii) 「base を最新に」を必須にする。
   - merge queue は文書上個人所有のリポジトリでは使えない見込み。設定画面に項目が無いことの確認だけお願いしたい。
2. **並列度の上限**
   - (i) 上限を設けない(今日は最大 9 本)。第 1 段階の後に競合の数を測り直す(**推奨**。第 1 段階で減るのは pre-push の競合 hunk で、
     手で解く rebase の回数はほぼ減らない見込み。上限の適正値を決めるデータがまだ無い)。
   - (ii) 同時に開く PR を 5 本までにする。
   - (iii) 登録ファイル(pre-push・fix-requires・`layer_boundary_guard.rs`・`examine_globs`)を触る PR だけ同時 3 本までにする。

## 関連する既存文書への追記案(この ADR では書き換えない)

- メモリ `project_adr229_fcis_layering_2026_10_06.md` の「自動マージ許可」: 「CLEAN は base が最新であることを意味しない(develop にブランチ保護が無い)。
  古い base でも CONFLICTING でなければそのままマージし、develop の push CI で Rust のジョブが落ちたら revert する」を追記。
- ADR-239 の決定 1「完了前チェック」の 1 項目目: 「最新の `origin/develop` に rebase してあるか」を「`CONFLICTING`/`DIRTY` でないこと」に置き換える。
- `.claude/rules/fix-requires-evidence.md` の「自動チェック(pre-push)」節: 「`state/` 直下の `*_plan.rs` は正規表現の `[a-z_]+_plan` でまとめて拾う。
  新しい `*_plan.rs` を作っても pre-push の正規表現は変えなくてよい」を 1 文。

## 再確認の手順

```sh
# 1. 2026-10-06 の PR 数と同時に開いていた最大数
gh pr list --state all --limit 80 --search "created:>=2026-10-06T00:00" \
  --json number,state,createdAt,mergedAt,closedAt
# 2. PR ごとの force-push(before/after のコミット)。nodes を数えること:
#    timelineItems(itemTypes:[...]){totalCount} は filter を無視した値を返す。
gh api graphql -f query='{repository(owner:"cuzic",name:"awase"){pullRequest(number:528){
  timelineItems(first:100,itemTypes:[HEAD_REF_FORCE_PUSHED_EVENT]){nodes{... on HeadRefForcePushedEvent{
  createdAt beforeCommit{oid} afterCommit{oid}}}}}}}'
# 3. base が動いたか(before と after の develop との merge base を比べる)
gh api repos/cuzic/awase/compare/develop...<sha> --jq .merge_base_commit.sha
# 4. rebase を再生して競合の有無(git 2.39 の trivial merge。k = before 側の PR コミット数)
git merge-tree <before の merge base> <after>~<k> <before> | grep -c '^+<<<<<<<'
# 5. マージ時点で base が古かったか(マージコミットの第 1 親が PR の head の祖先か)
git merge-base --is-ancestor <mergeCommit>^1 <headRefOid> || echo STALE
# develop の push CI の結果
gh api "repos/cuzic/awase/actions/runs?branch=develop&event=push&created=>=2026-10-06&per_page=100" \
  --jq '.workflow_runs[]|[.name,.conclusion]|@tsv' | sort | uniq -c
```
