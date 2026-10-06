---
id: ADR-239
title: |-
  並列エージェント開発での指示の取り違えを減らす(一時停止・取り消し・順序)
summary: |-
  2026-10-06 の並列開発(実装エージェントが worktree ごとに PR を作り、Opus がレビューし、team-lead がマージ。この日に作られた PR は #492〜#537 の 46 本)で起きた、指示の取り違え・古い状態での報告・手順の漏れ 10 件を、PR の編集履歴・PR コメント・フックの中身・指示書で裏取りした。
  原因は 3 つに分かれる: (1) 共通の約束が、セッション限りの scratchpad の指示書(impl-brief.md)と、自動では読み込まれない docs/tasks/fcis-layering-tasks-2026-10-06.md の §0・V2 に散らばっていて、しかも指示書自身が誤りを含んでいた(「pre-push フックの cargo check は構わない」)。(2) 一時停止・取り消しの指示は、作業中のエージェントに届く前に PR 作成まで進むことがある。(3) 報告の直前に現在の状態を見直していない。
  決定: 新しい機構は足さない。`.claude/rules/agent-handoff.md`(1 ページ、40 行以内)に、完了前チェック・取り消しの書き方と受け取り方・報告前の状態確認・push の仕方を置き、docs/tasks の §0・V2 と scratchpad の指示書の重複行を撤去してそこへの参照に置き換える。pre-push フックの変更はこの ADR では決めない(所有者に聞く)。
status: |-
  提案(起草中、Opus レビュー前)
related_adr:
  - "ADR-229"
  - "ADR-237"
  - "ADR-238"
  - "ADR-158"
  - "ADR-162"
---

# ADR-239: 並列エージェント開発での指示の取り違えを減らす(一時停止・取り消し・順序)

## この ADR の位置づけ

これはコードの変更ではなく、AI エージェントとの運用の約束である。ADR にする価値は小さく、本体は `.claude/rules/` の規約 1 ページで足りる。それでも ADR の形で残すのは、(i) 置き場所と pre-push フックの扱いが所有者の判断であること、(ii) 「受信箱の仕組みを作る」「フックを書き換える」などの却下した案と理由を残さないと、次の事故のたびに同じ案が出ること、の 2 点のためである。規約が定着したら、この ADR は「規約の由来」として読むだけのものになる。

隣の ADR との分担(同時に起草中): マージの直列化と `mergeStateStatus` の扱いは ADR-237、主張に再確認コマンドを添える約束は ADR-238 が本体。この ADR は「指示を出す側と受ける側の約束」に絞り、重なる項目は 1 行で参照する。

## 背景(実測した事実)

### 事例 10 件と裏取り

この日に作られた PR は 46 本(`gh pr list --state all --limit 60 --search "created:>=2026-10-06"`、#492〜#537)。team-lead が挙げた 10 件を、確認できた範囲で次に示す。

| | 事例 | 裏取り | 実害 |
|---|---|---|---|
| a | impl-tf2-removal が、一時停止の指示が届く前に #524 を作った | #524 は 05:31 作成・05:48 マージ。本文に「所有者承認済み」。指示の到着時刻は team-lead のメッセージのみで、リポジトリからは確認できない | 小(所有者が承認しマージ) |
| b | impl-f5 が、F5d の取り消しが届く前に #536 を作った | #536 は 06:48 作成、07:09 に「チームリードの指示で F5d は見送り(取り消し)」とコメントして CLOSED(`gh pr view 536 --json comments,state`) | 小(PR 1 本の作成と CI 1 回分) |
| c | impl-dup-s1 が、#526 のレビュー指摘 S2・S3 を最初の返信で反映しなかった | PR にコメントは 0 件(レビューはメッセージで往復)。リポジトリからは確認できない | 往復 1 回 |
| d | impl-p2 が、共有の `/tmp/b.md` で #513 の本文を #514 の本文で上書きした | #513 の編集履歴: 04:30:33Z の版が「FCIS F4。…」(#514 の本文)、04:34:31Z に元へ戻した(`gh api graphql -f query='{repository(owner:"cuzic",name:"awase"){pullRequest(number:513){userContentEdits(first:10){nodes{editedAt diff}}}}}'`)。`/tmp/b.md` は今も存在し、scratchpad にも `b.md`・`body.md` という汎用の名前のファイルがある | 4 分間、誤った本文(レビュアーが読めば誤判断の元) |
| e | impl-wc が「#508 は未マージ」と報告した | #508 は 03:51:36Z にマージ済み(`gh pr view 508 --json mergedAt`) | 誤った前提での判断の危険 |
| f | impl-wa が、衝突(DIRTY)で CI が起動しないのに「CI を待っている」と報告した | #519 は今も `mergeable=CONFLICTING`・`mergeStateStatus=DIRTY`(`gh pr view 519 --json mergeable,mergeStateStatus`)。最後の push は 06:39 | 待ち時間の浪費 |
| g | impl-f4・impl-v1・impl-f3 の `git push` で、pre-push フックがローカルで `cargo check` を回した | `.githooks/pre-push` は、**新しいブランチの push では変更内容を見ずに必ず**チェックする(`remote_sha` が全 0 なら `needs_check=true`)。チェックは `cargo clean -p awase-windows` の後の `cargo xwin check`。指示書 `impl-brief.md` の 21 行目が「push 時の pre-push フックが cargo check を走らせることがあるのは構わない」と**許していた**。一方 CI の `windows-cross-check` は同じ対象をより広く(`cargo xwin build --tests`)検査する(`.github/workflows/ci.yml:63`) | ディスク逼迫(今 84%、`df -h /home`)。所有者の「ローカルでビルドしない」(メモリ 2026-10-04)と矛盾 |
| h | マージ時に `mergeStateStatus` が UNKNOWN のまま(#518) | マージ済みの PR は後から見ると全て UNKNOWN になる(#513・#514・#518・#524・#526・#530 で確認)ので、マージ時点の値は**確認できない** | 未確認。ADR-237 の範囲 |
| i | team-lead の指示書に「3 か所必須」と書いた誤り | #530 のタイトルは今も「3 か所登録漏れを検出する」。実装は 2 つ目のコミットで mutants の 1 か所に絞った(`docs/tasks/fcis-layering-tasks-2026-10-06.md` の「V1 の実態」節)。承認済みの V1 の行は最初から mutants だけだった | 実装のやり直し 1 回 |
| j | ガードに触る PR のたびに再確認のラウンドが発生 | ガードの追補 PR が 4 本(#502・#509・#513・#521)。ただしこれは指示の取り違えではなく、ガードの走査の穴(V3 の対象)と、所有者がすでに決めた「docs・テストだけの直しは再確認を省く」の範囲 | この ADR の範囲外 |

数え方の注意: c・e・f は team-lead とのメッセージの中だけで起きており、リポジトリ・PR からは件数を数えられない。

### 共通の約束が散らばっていた

この日の約束は 3 か所にあった。

1. scratchpad の指示書(`impl-brief.md` 40 行、`adr-brief.md`・`inv-brief.md`・`w0-brief.md`)。セッションが終われば消える。`impl-brief.md:21` は上の g の誤りを含む。
2. `docs/tasks/fcis-layering-tasks-2026-10-06.md` の「§0 全タスク共通のルール」8 項目と「完了前チェック(V2)」4 項目。FCIS のタスク表の中にあり、自動では読み込まれない。V2 の 2 項目目「本文が自分の PR のものか」は d の後に足されたもの。
3. メモリ(`feedback_verify_commit_landed_not_backgrounded_2026_10_06`・`feedback_worktree_per_session`・`feedback_forks_may_ignore_investigation_only_instructions`・`feedback_no_local_builds_use_github_actions_2026_10_04`)。team-lead の会話には読み込まれるが、起動された実装エージェントが読むとは限らない。

一方で、`.claude/rules/*.md` は全セッションで自動的に読み込まれる(CLAUDE.md の「Repo-specific workflow rules」節)。並列のエージェントにも同じ文が届く唯一の置き場所である。

### 取り消しが届かない理由(未確認を含む)

a・b は、team-lead が一時停止・取り消しを送った時点で、エージェントが長い一続きの作業(編集 → push → `gh pr create`)の途中にいた。エージェントに送ったメッセージが作業の途中で読まれるのか、区切りでしか読まれないのかは**未確認**(この ADR の起草では確かめていない)。どちらにしても、取り消しが PR 作成に間に合わないことはあり得る、という前提で約束を書く。b の実際の後始末(PR をコメント付きで閉じる)は、コストが小さく正しかった。

## 決定

### D1. 規約 1 ページを `.claude/rules/agent-handoff.md` に置く(40 行以内)

中身は下の「付録: 規約の文案」。次の 5 つだけを書く。

1. **完了前チェック**(PR 作成と「済み」の報告の前): 最新の `origin/develop` に rebase してあるか / 新しい純粋モジュールを `CORE_MODULES` と mutants の `examine_globs` に載せたか(V1 は PR の差分でしか見ないので、人の確認は残る)/ PR 本文は自分のタスク名の付いたファイルから書き、更新後に `gh pr view <N> --json body` で自分の本文か確かめたか / 共有の `/tmp/*.md`・汎用の名前(`b.md`・`body.md`)を使っていないか / `gh pr view <N> --json mergeable,mergeStateStatus` が CONFLICTING・DIRTY でないか(DIRTY だと `pull_request` の CI が起動しない)。
2. **取り消し・一時停止の書き方**(指示を出す側): 1 行目に「作業を止める。PR は作らない」(または「PR を閉じる」)、2 行目に対象のブランチ名と PR 番号。理由は 3 行目以降。
3. **取り消しの受け取り方**(受ける側): 受け取った時点で PR がもうあれば、理由をコメントして閉じる(#536 の形)。ブランチは消さない。区切り(PR 作成の前・新しい作業の開始の前)では、届いているメッセージを読んでから進む。
4. **報告の前に今の状態を見る**: PR の状態・CI・マージの有無を報告する前に `gh pr view <N> --json state,mergedAt,mergeStateStatus` と `gh pr checks <N>` を実行し、その出力に基づいて書く(主張にコマンドを添える詳細は ADR-238)。
5. **push の仕方**: エージェントの作業では `git push --no-verify origin <ブランチ>`。検査は CI の `windows-cross-check` に任せる(pre-push フックより広く検査する)。フック自体の変更は所有者の判断(下の「所有者に聞くこと」2)。

team-lead 自身への約束(i の対策)も同じページに 1 項目だけ置く: 指示書に仕様を書くときは、承認済みの文書の該当行を**そのまま引用**し、ファイル名と節を添える(要約し直さない)。

### D2. 撤去する重複(D1 と同じ PR)

- `docs/tasks/fcis-layering-tasks-2026-10-06.md` の「完了前チェック(V2)」の 1・2 項目を、`.claude/rules/agent-handoff.md` への 1 行の参照に置き換える(3・4 項目は FCIS 固有なので残す)。§0 の 1(ローカルでビルドしない)・6(CI の見方)のうち、規約と重なる文を参照に置き換える。**この ADR の PR ではタスク表を書き換えない**(並行 PR との衝突を避けるため。下の「追記案」に書く)。
- scratchpad の指示書(`impl-brief.md`)から、規約と重なる行(21 行目のフックの許可、37 行目のコミット確認、35 行目の本文更新の手順)を消し、「`.claude/rules/agent-handoff.md` に従う」の 1 行にする。指示書はタスク固有の内容だけにする。

行数の見込み: 規約 +40 行、タスク表 −8 行前後、指示書 −5 行前後(指示書はリポジトリ外)。リポジトリの行数は増えるが、増えた分は自動で読み込まれる場所への移動であり、読み込まれない場所の重複を消す。

## 却下した案と理由

- **受信箱を確認する仕組み・ロックファイル・「PR 作成の前に team-lead の許可を待つ」手順を作る**: a・b の実害は PR 1 本の作成と閉じる手間で小さい。許可待ちを全 PR に入れると、46 本すべてに往復 1 回の待ちが乗る。取り消しは「PR を閉じる」で十分に後始末できる。
- **pre-push フックをこの ADR で書き換える**(docs のみのときスキップ、環境変数でスキップ): フックは人のセッションの安全網でもあり、変えるかは所有者の判断。この ADR では選択肢を並べるだけにする。
- **ガードの再確認ラウンド(j)をこの規約で減らす**: 原因はガードの走査の穴で、V3(`strip_comments` の共通化)の対象。再確認を省く範囲は所有者がすでに決めている(docs・テストだけの直し)。
- **規約を docs/tasks/ に置く**: 自動で読み込まれず、今回 V2 がそこにあっても d・f・g が起きた。
- **チェックを CI のジョブにする**: PR 本文の取り違え(d)・古い報告(e)・DIRTY の放置(f)は、エージェントの報告と PR の外で起きるので CI では捕まらない。DSL 化・宣言テーブル化・汎用ツールキットは ADR-218〜220・229 で却下済みで、ここでも作らない。

## 段階

| 段階 | 内容 | 撤去対象 | 検証方法 | 取りやめ条件 |
|---|---|---|---|---|
| 1 | `.claude/rules/agent-handoff.md` を足す(docs のみの PR、develop へ) | なし(重複の撤去は段階 2) | 規約が 40 行以内(`wc -l`)。CLAUDE.md の rules 一覧に 1 行足す | 所有者が置き場所に別案を選ぶ |
| 2 | タスク表の V2 の 1・2 と §0 の重複文を参照に置き換え、指示書から重複行を消す | 上の D2 | `grep -n "agent-handoff" docs/tasks/fcis-layering-tasks-2026-10-06.md` が参照を返す。V2 の 1・2 の本文が残っていない | — |
| 3 | 次の 10 本のエージェント PR で、事例の型 a〜i の件数を数える | — | PR 本文の取り違え: 各 PR の `userContentEdits` に他 PR の本文が無い。取り消し: 「取り消し」「見送り」コメントで閉じた PR の数。DIRTY の放置: マージ前に CONFLICTING のまま 30 分以上経った PR の数。c・e・f は team-lead がメッセージを数える(リポジトリからは数えられない) | 10 本で a〜i の合計が 2 件以上なら、規約の文を見直す(足すのではなく、守られなかった項目を書き直す)。0〜1 件なら段階 3 を終える |

全体の取りやめ条件: 規約が 60 行を超えたら削る(足し続けると、自動で読み込まれるコストが全セッションにかかる)。並列のエージェント開発をしなくなったら、規約を消してこの ADR を「廃止」にする。

## リスクと限界

- 規約は守られるとは限らない。メモリ `feedback_forks_may_ignore_investigation_only_instructions` のとおり、明記した指示でも無視されることがある。段階 3 で数えるのはそのため。
- 自動で読み込まれる規約は、並列開発をしていない人のセッションにも読み込まれる。40 行の上限はこのコストを抑えるため。
- メッセージが作業の途中で読まれるか区切りでしか読まれないかは**未確認**。区切りでしか読まれないなら、D1 の 3(区切りで読む)は効かず、効くのは「受け取ったら PR を閉じる」だけになる。
- h(マージ時の `mergeStateStatus`)は後から確かめられないので、件数は**未確認**。
- c・e・f はリポジトリに痕跡がなく、件数は team-lead の記録に頼る。

## 所有者に聞くこと

1. **置き場所**: (A) `.claude/rules/agent-handoff.md`(自動で読み込まれる、推奨)/(B) `docs/` の手順書にして、指示書から参照させる(読み込まれないので、エージェントが読むかは指示書次第)/(C) リポジトリには置かず、team-lead の指示書(scratchpad)だけで続ける。推奨は A: 並列のエージェント全員に同じ文が届く唯一の場所で、今回 B 相当(docs/tasks の V2)では d・f・g を防げなかった。
2. **pre-push フック**: (A) エージェントの作業では `--no-verify` を使う、と規約に書くだけ(フックは変えない、推奨)/(B) フックに `AWASE_SKIP_PREPUSH_CHECK=1` のような環境変数でのスキップを足す(フックの変更、数行)/(C) フックの「新しいブランチは必ずチェック」をやめ、`crates/awase-windows/` の変更があるときだけにする(人のセッションの安全網も弱まる)。推奨は A: CI の `windows-cross-check` が同じ対象をより広く検査し、所有者の「ローカルでビルドしない」とも合う。`--no-verify` を許してよいか(fix-requires-evidence の警告も出なくなる)を確認したい。B・C を選ぶなら別の判断として扱う。

## 関連する既存文書への追記案

- `CLAUDE.md` の「Repo-specific workflow rules」節に 1 行: 「`agent-handoff.md` — 並列のエージェント開発での完了前チェック・取り消しの書き方・報告前の状態確認・push の仕方(ADR-239)。」
- `docs/tasks/fcis-layering-tasks-2026-10-06.md` の「完了前チェック(V2)」: 1・2 項目を「`.claude/rules/agent-handoff.md` の完了前チェックに従う」の 1 行に置き換える(段階 2)。
- 指示書 `impl-brief.md`(scratchpad): 21・35・37 行目を消し、「`.claude/rules/agent-handoff.md` に従う」を足す。21 行目(フックの cargo check を許す)は所有者の方針と矛盾しているので、段階 1 を待たずに直してよい。

## 付録: 規約の文案(`.claude/rules/agent-handoff.md`、段階 1 で置く)

```markdown
# 並列エージェント開発の約束(ADR-239)

適用: team-lead が実装・レビューのエージェントを並列に起動して PR を作るとき。1 人のセッションには適用しない。

## 完了前チェック(PR 作成・「済み」の報告の前)
1. 最新の origin/develop に rebase した。
2. 新しい純粋モジュールを CORE_MODULES と .cargo/mutants-awase-windows.toml の examine_globs に載せた(CI の V1 は PR の差分しか見ない)。
3. PR 本文は scratchpad の <タスク名>-pr-body.md から書いた。共有の /tmp/*.md や b.md・body.md のような名前は使わない。更新後に gh pr view <N> --json body で自分の本文か確かめた。
4. gh pr view <N> --json mergeable,mergeStateStatus が CONFLICTING・DIRTY でない(DIRTY だと CI が起動しない)。

## 報告の前
PR の状態・CI・マージの有無は、報告の直前に gh pr view <N> --json state,mergedAt,mergeStateStatus と gh pr checks <N> を実行し、その出力で書く。記憶で書かない。

## 取り消し・一時停止
- 出す側: 1 行目に「作業を止める。PR は作らない」(または「PR #N を閉じる」)、2 行目に対象のブランチ名と PR 番号。理由は 3 行目以降。
- 受ける側: PR がもうあれば、理由をコメントして閉じる。ブランチは消さない。PR 作成の前と新しい作業の開始の前に、届いたメッセージを読んでから進む。

## push
エージェントの作業では git push --no-verify origin <ブランチ>(検査は CI の windows-cross-check)。宛先のブランチを明示する。

## 指示を書く側(team-lead)
仕様は承認済みの文書の該当行をそのまま引用し、ファイル名と節を添える。要約し直さない。
```
