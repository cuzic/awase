# ADR-265 レビュー round3(Opus、読み取りのみ)

対象: `/home/cuzic/awase-mozc-readable` のコミット `305c7177`(ADR-265 の D2・D3・D4・段階、`docs/index.html`・`index.en.html` の欄を `hidden`、`docs/adr/index.md`)。
2026-10-17 実施。前回までは `opus-review-adr265-round1.md`・`round2.md`。

結論: round2 の指摘は全件、方針として反映された。致命は無い。1 本のワークフロー構成で新しく**重要 3 件**(F1 schedule の 60 日停止、F2 承認待ちが concurrency を塞ぐ、F3 孤立ブランチ化は結局 1 回の force-push)が出た。
どれも ADR に数行足せば閉じる。F3 は、材料ブランチを別の名前にすれば force-push が 1 回も要らなくなる。

---

## (1) round2 の各指摘の扱い

| ID | 判定 | 備考 |
| --- | --- | --- |
| C2a `Unchanged()` に active | 解消 | D3。 |
| C2b version 1 を捨てる | 解消 | D3。 |
| I3 最初の Release は置換後 | 解消 | D4-7・段階 0 の完了条件。 |
| I4/N5 N+1 の契機 | 不足(F5) | path フィルタ `readable/patches/**` が広すぎる。 |
| I5 停止の検知 | 解消、ただし F1 | 検知するジョブ自体が schedule で動くので、schedule が止まると検知も止まる。 |
| I7/I8 | 解消 | `workflow_run`・PAT を廃止。`GITHUB_TOKEN` で足りるかは未確認のまま(段階 0)と正しく書いてある。 |
| N1 build/* を push しない | 解消 | |
| N2 BASE | 解消 | 段階 0 で試すと明記。 |
| N3 土台ごとの series | 解消、ただし F4 | 新しいタグが `master/` で通ったとき、その series をタグ名のディレクトリへ固定する手順が無い。 |
| N4 generate.sh / refresh-patches.sh | 解消、ただし E1 | |
| N6 README・孤立ブランチ | 解消、ただし F3 | |
| E1 jq startswith | 解消 | |
| E2 未実測の明記 | 解消 | |
| E3 欄を hidden | 解消 | 両言語とも `hidden` 属性(`git show 305c7177 -- docs/index.html docs/index.en.html`)。 |

---

## (2) 1 本のワークフロー構成で新しく生じた穴

### F1(重要). 公開リポジトリの schedule は、60 日リポジトリに動きが無いと GitHub が自動で無効にする
- 根拠: GitHub Actions の仕様(公開リポジトリでは、60 日間リポジトリにアクティビティが無いと schedule のワークフローが無効になる)。
  材料ブランチ構成では、`readable` が動くのは patch や identity を直したときだけになる。upstream のリリースは 3〜4 か月おき
  (タグの日付: 3.33.6089 → 3.33.6133 〈2026-03-12〉→ 3.34.6239 〈2026-06-24〉)なので、静かな期間が 60 日を超えるのは普通に起きる。
  D4-5 の停止の検知(「M 日 Release が無い」)も同じ schedule で動くので、一緒に止まる。
- 失敗シナリオ: 次の upstream タグが出たときに追随ジョブが無効になっていて、誰にも知らされない。round1 の C1 と同じ「静かに動かない」状態に戻る。
- 推奨: 次のどちらかを ADR に書く。(a) ワークフローの最後に `gh api -X PUT repos/{owner}/{repo}/actions/workflows/<id>/enable` を
  呼ぶ(`GITHUB_TOKEN` に `actions: write`)。無効化の前に毎回有効化し直すので止まらない(無効化された後では動けないので、
  60 日未満の周期で必ず走ることが前提)。(b) awase リポジトリ側(動きがある)の定期ジョブから、mozc-readable の
  ワークフローの状態(`state`)と最後の run の日付を見て、止まっていたら Issue を立てる。(a) と (b) の併用が確実。

### F2(重要). 承認待ちの run が concurrency のグループを塞ぎ、毎日の early detection と次の追随を止める
- 根拠: GitHub の concurrency は、同じグループの実行中 run が 1 本、待ちが 1 本までで、新しい待ちは古い待ちを取り消す。
  `environment` の承認待ち(最長 30 日)の run は「実行中」としてグループを持ち続ける。PR #1 のワークフローは
  `concurrency: group: readable-upstream-sync / cancel-in-progress: false` の形だったので、同じ形を 1 本のワークフローに付けると、
  承認を置いている間、毎日の schedule の run は待ちになり、翌日の run がそれを取り消す。早期検知(master への生成)も止まる。
- 推奨: (a) 早期検知と、新しいタグの追随・公開を**別のワークフロー**に分ける(早期検知は公開ジョブも `environment` も持たない)。
  (b) concurrency は公開ジョブにだけ付け、グループ名に upstream の版を入れる(`readable-publish-<タグ>`)。
  (c) 承認待ちのまま次の版が来た場合の扱い(古い版の承認待ちを取り消すか)を書く。
- 補足: `environment` の承認(required reviewers)は、公開リポジトリなら無料プランでも使える。承認待ちの間はジョブの
  `timeout-minutes` は進まない。承認者は 30 日以内に承認しないと run が失敗する。ビルドの artifact は既定で 90 日残るので、
  承認期限より先に消えることはない(いずれも GitHub の仕様、このリポジトリでの実測ではない)。

### F3(重要). `readable` を孤立ブランチとして作り直すのは、1 回の force-push そのもの
- 根拠: D4-1 は「force-push は不要」としつつ、「`readable` は upstream と履歴を共有しない孤立ブランチとして作り直す」と書く。
  同じ名前のブランチを、履歴を共有しないコミットへ置き換えるのは force-push(round1 I1 で挙げた危険がそのまま 1 回起きる)。
  - PR #1(base `readable`)は、base の履歴が入れ替わると差分が壊れる。
  - 別セッションの worktree や clone(`/home/cuzic/mozc-readable-release/repo` の `readable` など)が `origin/readable` を追跡していると、
    次の `git pull` で、関係のない 2 つの履歴をマージしようとする(`--allow-unrelated-histories` が無いので失敗はするが、
    混乱の元になる)。
  - `readable` への push で起動する今の `readable-build.yaml`(`push: branches: [readable]`)は、置き換えの直後に材料だけのツリーで走って失敗する。
- 推奨: **材料ブランチを別の名前で作る**(例: `main`)。`readable` は今の場所のまま凍結し、`legacy/readable-src` への改名もしない。
  手順は「`main` を孤立ブランチとして作る → 既定ブランチを `main` に変える → PR #1 を閉じる → `readable` を保護(push 禁止)する」。
  force-push が 1 回も要らず、別セッションの作業も PR #1 の扱いも、`readable` を触らない限り壊れない。
  ADR の「既定ブランチ `readable`」と「`legacy/readable-src`」の記述は、この名前に合わせて直す。
- 移行の順序で気を付けること: `workflow_dispatch` と `schedule` は既定ブランチにあるワークフローでしか起動できない
  (`gh workflow run --ref <ブランチ>` は、そのワークフローのファイルが既定ブランチにもあることが前提)。新しい構成のワークフローは、
  既定ブランチを切り替えてからでないと手で試せない。切り替え前に試すなら、同じファイル名のワークフローを今の既定ブランチ(`master`)に
  一時的に置く必要がある。どちらの順序にするかを段階 0 に書く。

### F4(軽微). 新しいタグが `master/` の series で通ったら、その series をタグ名のディレクトリへ固定する
- 根拠: D4-1「そのタグ名のディレクトリがあればそれを、無ければ `master/` を試す」「早期検知は `master/` を直す」。新しいタグを
  `master/` で公開した後に `master/` を(upstream の次の変更に合わせて)直すと、その版の再リリース(N+1)では `master/` が
  もう当たらないことがある。round2 N3 の衝突が 1 段先で再発する。
- 推奨: 公開に使った series を `patches/publisher/<タグ>/` へコピーする PR を、公開後に作る(公開ジョブが PR を作る。
  `GITHUB_TOKEN` で作った PR では、その PR の CI が起動しない点に注意)、と書く。

### F5(重要). N+1 の path フィルタ `readable/patches/**` は、`master/` の修正でも現行タグの Release を出す
- 根拠: D4-1「N+1 を作る契機は `readable/patches/**` と `identity.toml` の変更に限る」。早期検知のための `master/` の修正も
  `readable/patches/**` に当たるので、現行タグの再リリースが起動する。そのときに使われるのは現行タグのディレクトリ
  (または `master/`)。MSI の中身が変わらない(または、意図しない series で作った)Release が出る。
- 推奨: 契機は「現行タグに使う series(`patches/publisher/<現行タグ>/`。無ければ `master/`)と `identity.toml` の変更」とし、
  ワークフローの中で、どの series が使われるかを決めてから、その差分の有無で N+1 を決める。`environment` の承認があるので
  実害は承認者の手間だけだが、判定は ADR に正しく書いておく。

### F6(軽微). artifact の引き渡しと、どこで生成するか
- 生成したツリー(upstream の `src/` と `update_deps.py` が落とす third_party を含む)は大きい。生成を Linux のジョブで行い、Windows のビルドへ
  artifact で渡すと、容量・時間のほか、改行コード(`git am` を Linux、ビルドを Windows)や実行ビットで差が出うる。
- 推奨: 生成は Windows のビルドジョブの中で `generate.sh`(Git Bash)を呼んで行い、後段に渡すのは MSI と `git archive` の tar だけにする。
  早期検知(ビルドしない)は Linux で同じ `generate.sh` を使う。`core.autocrlf` は生成の前に `false` にする。

### F7(軽微). Release のタグが材料のコミットを指すので、GitHub が自動で付ける "Source code (zip)" が材料だけになる
- 根拠: D4-1「Release のタグは材料ブランチのコミットに付ける」。GitHub は Release のタグのツリーを "Source code (zip/tar.gz)" として
  自動で付け、消せない。利用者はこれを mozc-readable のソースだと思って落とす。
- 推奨: `git archive` の asset を `mozc-readable-<版>-source.tar.gz` のような分かる名前にし、Release 本文に
  「自動の Source code は生成スクリプト(材料)だけで、ビルドしたソースは `...-source.tar.gz`」と書く。
- 同じ SHA で N+1 を作れるか: 作れる。git のタグは名前が違えば同じコミットに何本でも付けられ、`gh release create --target <SHA>`
  も同じ SHA を受け付ける。同じ材料の SHA で N+1 を出すのは、ビルド環境の都合で作り直すときだけなので、手動の
  `workflow_dispatch` に限る、と書くとよい(F5 の判定では、材料に変更が無ければ N+1 は起きない)。

### F8(軽微). `GITHUB_TOKEN` の権限はジョブ単位で絞る
- D4-6 は「`GITHUB_TOKEN`(`contents: write`・`issues: write`)で足りる見込み」。ワークフロー全体に `contents: write` を付けると、
  upstream のソースを生成してビルドするジョブ(upstream のスクリプト `update_deps.py`・Bazel を動かす)も書き込み権限を持つ。
  ワークフローの既定を `contents: read` にし、`contents: write` は公開ジョブ、`issues: write` は Issue を立てるジョブ、`actions: write` は F1 の
  有効化のステップにだけ付ける、と書く。

---

## (3) 改訂で入った誤り

### E1(軽微). `refresh-patches.sh` の土台は `<タグ>` ではなく `BASE`
- D4-1「解消したツリーから `git format-patch <タグ>..` で series を作り直し」。`master/` の series の土台はタグではなく master のコミット。
  `git format-patch <BASE>..` とし、作り直したら `BASE` を新しい土台に更新する、と書く。

### E2(軽微). 「force-push・sync-base…の仕組みは不要」と「`readable` を孤立ブランチとして作り直す」の矛盾
- F3 のとおり。名前を変えれば矛盾は消える。

### E3(軽微). 既定ブランチ名の記述が F3 の推奨と合わない
- D4-1・段階 0 の「既定ブランチ `readable`」「`legacy/readable-src` に残し」は、F3 を採るなら「既定ブランチ `main`(材料)」
  「`readable` は凍結」に直す。採らないなら、1 回の force-push とその手順(PR #1 を先に閉じる、別セッションへの連絡、
  `readable-build.yaml` の push 起動を先に外す)を段階 0 に書く。

---

## 確認したこと・していないこと

- 確認した: `git show 305c7177`(ADR・サイト・index)、ADR の残りに `sync/`・`sync-base`・`SYNC_TOKEN`・`workflow_run` の古い記述が
  残っていないこと(`grep -n -E 'sync/|sync-base|SYNC_TOKEN|workflow_run' docs/adr/265-mozc-readable-integration.md` → 「使わない」の
  文脈のみ)、upstream のタグの日付(round1 で取得)。
- 未確認(GitHub の仕様として書いたもの): schedule の 60 日の無効化、concurrency の待ち 1 本の規則、environment の承認期限 30 日、
  artifact の既定保持 90 日、`workflow_dispatch` が既定ブランチのファイルを前提とすること。どれもこのリポジトリでは試していない。
