---
id: ADR-237
title: |-
  develop への取り込みを直列化する(merge lane を運用で決め、競合の発生源を 1 つ減らす)
summary: |-
  2026-10-06 は 1 日に 46 本の PR が develop 向けに作られ、同時に開いていた PR は最大 9 本。force-push 29 回のうち 28 回は新しい develop への rebase で、
  再生して確かめられた 21 回のうち 10 回が実際に競合した。競合したファイルで一番多いのは `.githooks/pre-push` の対象正規表現の 1 行(5 回)で、
  新しい `state/*_plan.rs` を作るたびに `|xxx_plan` をこの 1 行の中に足すため、モジュールを足す PR 同士は必ず衝突する。
  一方、マージ済み 38 本のうち 32 本は develop より古い base のままマージされたが、develop の push CI で Rust のジョブが落ちたことは 0 回(落ちた 4 回は全て report-worker の外部 API 403)。
  決定: (b) マージ担当を 1 人にする merge lane を運用の約束として書き、(c) のうち pre-push の正規表現で `*_plan` を列挙をやめて `[a-z_]+_plan` の 1 パターンにする(第 1 段階)。
  GitHub merge queue と「最新の base を必須にする」ブランチ保護は所有者判断(今日の実測では古い base による実害が 0 件で、費用に見合う証拠が無い)。
status: |-
  提案(起草中、Opus レビュー前)
related_adr:
  - "ADR-158"
  - "ADR-162"
  - "ADR-229"
  - "ADR-236"
---

# ADR-237: develop への取り込みを直列化する

## 背景(実測、2026-10-06、develop `1630e55e`)

### 1. 並列度と、競合の実数

数えた範囲は 2026-10-06(UTC)に作られた PR #492〜#535 の 46 本(マージ 38、オープン 6、クローズ 1、ほか 1)。

| 量 | 値 | 再確認のコマンド |
| --- | --- | --- |
| 同時に開いていた PR の最大 | 9 本(04:11 UTC) | 下の「再確認の手順」1 |
| PR が開いていた時間の中央値 | 26 分(最大 92 分) | 同上 |
| force-push(`HeadRefForcePushedEvent`) | 29 回 / 19 本 | 同 2 |
| そのうち新しい develop への rebase | 28 回(base が動かない amend は 1 回) | 同 3 |
| rebase を再生して競合の有無を確かめられたもの | 21 回 | 同 4 |
| そのうち実際に競合したもの | **10 回** | 同 4 |

競合した 10 回のファイル(1 回に複数ファイルあり):

| ファイル | 回数 | PR |
| --- | --- | --- |
| `.githooks/pre-push`(対象正規表現の 1 行、36 行目) | **5** | #517、#519、#528×2、#535 |
| `.claude/rules/fix-requires-evidence.md`(再発ファミリー表) | 3 | #519、#528、#535 |
| `crates/awase-windows/tests/layer_boundary_guard.rs` | 3 | #498(冒頭コメント)、#504(`CORE_MODULES`)、#519 |
| `crates/awase-windows/tests/architecture_guard.rs` | 1 | #517 |
| `.cargo/mutants-awase-windows.toml`(`examine_globs`) | 1 | #519 |
| `docs/tasks/fcis-layering-tasks-2026-10-06.md` | 1 | #528 |
| `docs/known-bugs/BUG-074.md` | 1 | #526 |
| 本番コード(`runtime/executor.rs`・`state/ime_set_open_plan.rs`、`state/drift_*.rs`) | 2 | #504、#532 |

再生できなかった 7 回(PR のコミット数が rebase の前後で変わり、新しい base を機械的に特定できなかったもの)は数に入れていない。
#528 の残り 1 回(07:33)では `architecture_guard.rs` と `.githooks/pre-push` の競合も出ているが、同じ理由で表から外した。
つまり 10 回は下限。

10 回のうち 8 回は「本番コードの中身」ではなく、**登録のための 1 行を同じ場所に足す**ことで起きている(pre-push の正規表現、
fix-requires の表のセル、`CORE_MODULES`、`examine_globs`、タスク表)。本番コードの意味の衝突は 2 回(#504・#532)だけ。

### 2. 競合が起きる構造(ファイルごと)

- **`.githooks/pre-push` 36 行目**: `local target='(...state/(ime|conv_mode|...|drift_plan|drift_correction|relay_plan|deferred_gate_plan|focus_classify_plan|focus_resolve_plan|msaa_role_plan)|...)'`
  の 602 文字の 1 行。今日だけでこの行は 6 コミットで書き換わった(`git log --oneline --since=2026-10-06T00:00 origin/develop -- .githooks/pre-push`)。
  新しい `state/*_plan.rs` を足す PR は全てこの 1 行を書き換えるので、**同時に 2 本あれば必ず衝突する**。
  `xtask-adr-evidence`(`crates/xtask-adr-evidence/src/main.rs:322-336`)は `local target=` で始まる最初の 1 行だけを読むので、
  行を複数行に分けるには xtask の変更も要る。
- **`fix-requires-evidence.md` の表**: 1 セルが 1 行で、長い行は 3,475 文字(147 行目)。モジュールを足す PR はこのセルの末尾に書き足す。
- **`CORE_MODULES`**(`layer_boundary_guard.rs:495-`): すでに 1 行 1 要素・昇順。#504 の衝突は隣り合う名前(`ime_read_strategy` の次に
  `ime_set_open_plan`)を 2 本が同時に足したため。昇順にしても隣接すれば衝突する。
- **`examine_globs`**(`.cargo/mutants-awase-windows.toml:28-49`): 1 行 1 要素だが昇順でなく、末尾に足す運用。今日足された 8 行
  (`drift_plan` 〜 `deferred_gate_plan`)は全て末尾に並んでいる。末尾追記は 2 本あれば必ず衝突する。
- **`state/mod.rs`**: 今日 14 本の PR が触った(最多タイ)。ただし再生では `state/mod.rs` 由来の競合は数えられなかった(宣言の位置が分散していたため)。
- **`architecture_guard.rs`**(7,201 行): ガードを末尾に足す運用。今日 14 本が触ったが、再生で確認できた競合は 1 回。

### 3. 古い base のままのマージと、その実害

- マージ済み 38 本のうち **32 本は、マージした時点の develop より古い base のまま**マージされた(develop から 1〜26 コミット遅れ)。
  CI は最後の push の時点の `refs/pull/N/merge` で走るので、この 32 本はマージ後の develop と同じ内容では CI を通っていない。
- それでも、2026-10-06 の develop への push で走った CI 62 回のうち失敗は 4 回で、**4 回とも `report-worker` ジョブの
  GitHub API 403(`latest release fetch failed UpstreamFetchError: ... 403`)**。Rust のビルド・テスト・ガードが develop で落ちたことは 0 回。
- 古い base のマージで実際に develop に入った害は、**diff3 の衝突マーカーが残った 2 件**だけ(ADR-236 の対象):
  - `5de4b395`: `docs/known-bugs/BUG-172.md` に `|||||||` 行が 1 行残った。
  - `1630e55e`: #535(F5c)のマージで `.githooks/pre-push` と `fix-requires-evidence.md` に `||||||| parent of 9c9a1164 ...` の行と、
    **古い `local target=` 行がまるごと 1 行**残った。pre-push はシェルの構文エラーになり、xtask は先頭の(古い方の)`local target=` を
    読み得た。CI はどちらも検出しなかった(Markdown とシェルはコンパイルされない)。マージ 07:30 UTC → 修正 07:41 UTC。
  これは「古い base」ではなく「競合を手で解いたときの消し忘れ」が原因で、merge queue でも防げない(競合解消は PR 側の rebase で起きる)。

### 4. 費用の見積もり

- PR の CI(`pull_request`)は 2026-10-06 に `CI` 119 回・`e2e-ime-smoke` 115 回。1 回あたりの実行時間はおよそ 6 分(合計 729 分 / 119 回)と 5.8 分(669 分 / 115 回)。
- rebase 28 回はそれぞれ CI を 2 本ずつ再実行させるので、**約 28 × 12 分 ≒ 330 ランナー分**が rebase 由来。そのうち競合を伴った 10 回が約 120 分。
- エージェントの時間(競合を解いて push し直すまでの時間)は**未確認**。#528 は 06:29〜07:41 の 72 分に rebase 5 回(CI 待ちを含む)。
- develop への push で `e2e-ime-smoke` は 37 回中 18 回が `cancelled`(後続のマージで取り消し)。直列化してもマージの頻度が同じなら変わらない。

### 5. 誰がマージしているか

38 本全てが `cuzic` アカウント(`gh` 経由)でマージされた。実際には複数のエージェントがそれぞれ自分の PR をマージしており、
マージの順番を決める担当はいない。自動マージ許可(メモリ `project_adr229_fcis_layering_2026_10_06.md`)の条件
「mergeStateStatus=CLEAN」は、develop にブランチ保護が無い(`gh api repos/cuzic/awase/branches/develop/protection` → 404)ため、
**base が最新であることを意味しない**(古くても競合が無ければ CLEAN になる)。

## 決定

### (b) merge lane(運用の約束)

1. **develop へのマージはマージ担当 1 人(team-lead のエージェント、または所有者)が順番に行う。** 実装エージェントは自分の PR をマージしない。
   準備ができたら担当に「PR #N マージ可(Opus 再確認済み・CI green)」と送る。
2. マージ担当は、マージの直前に `gh pr view N --json mergeStateStatus,baseRefOid` と `git merge-base --is-ancestor origin/develop <head>` で
   base が最新かを見る。**最新でなくても、次の「登録ファイル」を触っていなければそのままマージしてよい**(実測で古い base の実害は 0 件):
   `.githooks/pre-push`、`.claude/rules/fix-requires-evidence.md`、`crates/awase-windows/tests/{architecture_guard,layer_boundary_guard}.rs`、
   `.cargo/mutants-awase-windows.toml`、`crates/awase-windows/src/state/mod.rs`。
   これらを触っていて最新でなければ、PR の作者に rebase を頼み、最新の develop での CI green を待ってからマージする。
3. マージ担当は、登録ファイルを触る PR をマージしたら、同じファイルを触っているオープン中の PR の作者に「#N が入った。rebase を」と知らせる
   (`gh pr list --state open --json number,files` で引く)。作者は自分のブランチで `git rebase origin/develop` する。develop 側で直さない。
4. 競合を解いた後は push の前に `git grep -nE '^(<<<<<<<|\|\|\|\|\|\|\||=======|>>>>>>>)( |$)'` が 0 件であることを確かめる
   (CI での検出は ADR-236 に任せる。ここでは手順だけ)。
5. docs だけのコミットを develop に直接 push する運用(メモリ `project_adr229_*` の How to apply)は続けてよい。ただし登録ファイルの
   docs(`fix-requires-evidence.md`)は直接 push せず PR に乗せる(他の PR の rebase を増やすため)。

撤去対象: 「各エージェントが自分の PR をマージする」運用。行数の増減: コードは 0 行(運用の約束のみ。ここに書く)。

### (c) 第 1 段階: pre-push の正規表現で `*_plan` を列挙しない

`.githooks/pre-push` 36 行目の `state/(...)` の中の
`|drift_plan|drift_correction|relay_plan|deferred_gate_plan|focus_classify_plan|focus_resolve_plan|msaa_role_plan`
のうち `*_plan` の 6 つを、`|[a-z_]+_plan` の 1 つに置き換える(`drift_correction` は残す)。

- 撤去対象: 「新しい `state/*_plan.rs` を作るたびに pre-push の正規表現の 1 行を書き換える」手順と、それによる衝突(今日 5 回)。
- 行数: 36 行目を 1 行書き換えるだけ(行数 ±0、文字数は約 70 文字減)。経緯のコメントを 2 行足す(+2 行)。
- 効果の副産物: 現在どこにも載っていない `state/focus_probe_plan.rs`(`grep -c focus_probe_plan .claude/rules/fix-requires-evidence.md` → 0、
  正規表現にも無い)も対象に入る。`ime_set_open_plan` は既に `state/(ime` の前方一致で入っている。
- xtask との関係: `xtask-adr-evidence` の TC3 は「表に書いたパスが正規表現でカバーされているか」を見る向きなので、正規表現を広げても落ちない
  (表のパス `state/msaa_role_plan.rs` などは `[a-z_]+_plan` でカバーされる)。V1(`core-registry`)は pre-push を見ない
  (`core_registry.rs:6`)。
- 誤検出の側: pre-push は警告だけでブロックしない。`state/` 直下で `_plan` を含むが再発ファミリーでないファイルが将来できても、警告が 1 回増えるだけ。

fix-requires の表(3 回)はこの段階では変えない。表のセルにモジュールを書き足すかは V2(人の判断)のままで、書き足すときは PR に乗せる(決定 (b)-5)。

## 却下した案と理由

- **(a) GitHub merge queue を今入れる**: ブランチ保護(またはルールセット)と、全ワークフローへの `on: merge_group` の追加が要る。
  - リポジトリの設定変更は所有者が行う。この ADR では設定を変えない。
  - GitHub の文書では merge queue は組織所有の公開リポジトリが対象とされ、個人所有の `cuzic/awase` で使えるかは**未確認**(所有者が設定画面で確認)。
  - merge queue が防ぐのは「古い base のまま入って develop が壊れる」ことだが、今日の実測ではその実害が 0 件。競合そのもの(rebase 28 回)は減らない
    (競合した PR は queue に入る前に作者が rebase する)。
  - `core-registry-consistency`(V1)は `if: github.event_name == 'pull_request' && github.base_ref == 'develop'` なので、
    `merge_group` では動かない。必須チェックにしない方針(所有者決定済み)なので壊れはしないが、queue で検査が 1 つ減る。
  - queue は CI を 1 回ずつ直列に走らせるので、`CI`(約 6 分)と `e2e-ime-smoke`(約 6 分)の長い方で 1 本あたり約 6 分かかり、1 日 38 本なら約 4 時間ぶんの待ち行列になる。
- **「マージ前に base を最新にすることを必須にする」ブランチ保護(strict status checks)**: 個人リポジトリでも使える。ただし 32/38 本が古い base だったので、
  rebase と CI の再実行が 1 日あたり約 32 回増え(約 380 ランナー分)、その対価で防げた実害は今日 0 件。所有者判断とし、推奨しない。
- **pre-push の正規表現を 1 行 1 モジュールの配列に分ける**: 衝突の根は消えるが、xtask の TC3 が `local target=` の 1 行を前提にしている
  (`main.rs:327`)ので xtask の変更が要り、行数が数十行増える。`[a-z_]+_plan` の 1 パターンの方が費用が小さい。
- **`architecture_guard.rs` の末尾追記をガードごとの小関数・別ファイルにする**: 再生で確認できた競合は 1 回だけで、ファイル分割は
  ADR-218 round1 で「新ファイルが CI の `--test` 指定と xtask から漏れる」と指摘された。今回の証拠では費用に見合わない。
- **`examine_globs` を昇順にそろえる**: 末尾追記の「必ず衝突」は消えるが、今日の確認できた競合は 1 回。第 2 段階の候補に回す。
- **`CORE_MODULES` を 1 行 1 要素にそろえる**: すでにそうなっている(`layer_boundary_guard.rs:495-`)。何もしない。
- 宣言テーブル化・DSL 化・汎用ツールキットは ADR-218〜220・229 で却下済みなので扱わない。

## 段階

| 段階 | 内容 | 撤去対象 | 検証(CI・コマンドで確認できる形) | 取りやめ条件 |
| --- | --- | --- | --- | --- |
| 0 | merge lane の運用開始(この ADR の承認で開始) | 各エージェントの自己マージ | 次の 40 本で、マージした PR の「登録ファイルを触り、かつ古い base」の件数が 0 か(再確認の手順 5) | 担当の待ちで PR が開いている時間の中央値が今日(26 分)の 2 倍を超えたら、登録ファイルを触らない PR は自己マージに戻す |
| 1 | pre-push 正規表現の `*_plan` を `[a-z_]+_plan` に | `*_plan` 追加ごとの 36 行目の書き換え | `adr-evidence-consistency` が green。次の 40 本で、新しい `state/*_plan.rs` を足した PR が `.githooks/pre-push` を変更していないか(`gh pr list --state merged --json files` で確認)、再生での pre-push の競合が 0 回か(手順 4) | 再発ファミリーでない `*_plan.rs` への警告が出て作業の妨げになったら、`*_plan` の列挙に戻す |
| 2(候補、未決) | `examine_globs` を昇順にそろえ、末尾追記をやめる | 末尾追記による必ず起きる衝突 | 段階 1 後の 40 本で `examine_globs` の競合が 2 回以上あったときだけ着手。V1 の `in_mutants` は順序を見ないので落ちない | 段階 1 後に `examine_globs` の競合が 0〜1 回なら着手しない |

## リスクと限界

- 競合の数は「force-push の前後のコミットを `git merge-tree`(trivial merge)で再生して `<<<<<<<` が出るか」で数えた。PR のコミット数が
  rebase の前後で変わったもの 7 回は数えていない(10 回は下限)。手で解いた競合と、rebase の途中で `git rerere` が自動で解いたものは区別できない。
- 「古い base でも実害 0 件」は 1 日分の 62 回の develop CI からの推定。Rust のガード同士の意味の衝突(2 本がそれぞれ通るが合わせると落ちる)は
  今日は起きなかったが、起きない保証は無い。merge lane の 2 で「登録ファイルを触る PR は最新の base での CI」としたのはこのため。
- マージ担当が 1 人になると、その担当が止まっている間はマージが止まる。今日のマージ間隔は数分おき(#521 05:06 → #517 05:09 など)で、
  担当の負荷は**未確認**。
- エージェントの時間の損失は計測していない(**未確認**)。費用の見積もりはランナー分のみ。
- 第 1 段階は今日の 5 回の pre-push 競合の原因を消すが、fix-requires の表の 3 回は残る。

## 所有者に聞くこと

1. **GitHub merge queue(またはブランチ保護)を使うか**
   - (i) 使わない。merge lane の運用だけ(**推奨**。今日の実測で古い base の実害が 0 件で、queue は競合そのものを減らさない)。
   - (ii) ブランチ保護で「base を最新に」を必須にする(1 日あたり rebase と CI の再実行が約 32 回増える)。
   - (iii) merge queue(個人リポジトリで使えるかを先に設定画面で確認。使えるなら全ワークフローに `on: merge_group` を足す PR が要る)。
2. **並列度の上限**
   - (i) 上限を設けない(今日は最大 9 本)。第 1 段階のあとに競合の数を測り直す(**推奨**。今日の競合 10 回のうち 5 回は第 1 段階で消える見込みで、
     上限の適正値を決めるデータがまだ無い)。
   - (ii) 同時に開く PR を 5 本までにする。
   - (iii) 登録ファイル(決定 (b)-2 の 6 ファイル)を触る PR だけ同時 3 本までにする。
3. マージ担当を誰にするか(team-lead のエージェントか、所有者自身か)。推奨は team-lead のエージェント(自動マージ許可の条件の確認を担当に集約できる)。

## 関連する既存文書への追記案(この ADR では書き換えない)

- メモリ `project_adr229_fcis_layering_2026_10_06.md` の「自動マージ許可」: 「マージは merge lane の担当が行う。CLEAN は base が最新であることを
  意味しない(develop にブランチ保護が無い)。登録ファイルを触る PR は最新の develop での CI green を確認」を追記。
- `.claude/rules/fix-requires-evidence.md` の「自動チェック(pre-push)」節: 「`state/` 直下の `*_plan.rs` は正規表現の `[a-z_]+_plan` で
  まとめて拾う。新しい `*_plan.rs` を作っても pre-push の正規表現は変えなくてよい」を 1 文。
- `docs/tasks/fcis-layering-tasks-2026-10-06.md` の V2(完了前チェック): 「競合を解いた後、`git grep` で衝突マーカーが 0 件か」を 1 項目。

## 再確認の手順

```sh
# 1. 2026-10-06 の PR 数と同時に開いていた最大数
gh pr list --state all --limit 80 --search "created:>=2026-10-05T23:00" \
  --json number,createdAt,mergedAt,closedAt
# 2. PR ごとの force-push(before/after のコミット)
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
