# ADR-265 レビュー round4(Opus、読み取りのみ、最終確認)

対象: `/home/cuzic/awase-mozc-readable` のコミット `bcc85446`(ADR-265 の D4-1・D4-5・D4-6・未確認・段階 0、`docs/adr/index.md`)。
2026-10-17 実施。前回までは `opus-review-adr265-round1.md`〜`round3.md`。

## 結論: 収束

致命・重要はもう無い。残りは軽微 7 件で、どれも段階 0 の実装のときに ADR へ 1〜2 行足せば済む(方針の変更は要らない)。

---

## (1) round3 の各指摘の扱い

| ID | 判定 | 備考 |
| --- | --- | --- |
| F1 schedule の 60 日無効化 | 解消、軽微な残り G1・G2 | 自己有効化(F1a)と awase 側の監視(F1b)を併用。 |
| F2 承認待ちが concurrency を塞ぐ | 解消、軽微な残り G3 | `detect`/`release` に分け、concurrency は公開ジョブだけ・版入りのグループ。 |
| F3 force-push | 解消 | 材料は孤立ブランチ `main`、`readable` は凍結・保護、PR #1 は閉じる。force-push は 0 回。 |
| F4 series をタグ名へ固定 | 解消、軽微な残り G4 | |
| F5 N+1 の契機 | 解消、軽微な残り G5 | 「使う series と `identity.toml` の SHA が、直近の同じ版の Release と違うときだけ」。 |
| F6 生成の場所 | 解消 | Windows ジョブ内で Git Bash、`core.autocrlf=false`、渡すのは MSI と tar だけ。 |
| F7 Source code zip の注記・同じ SHA の N+1 | 解消 | |
| F8 権限 | 解消、軽微な残り G3・G4(足りない権限 2 つ) | |
| E1 format-patch の土台 | 解消 | `git format-patch <BASE>..`。 |
| E2 force-push の矛盾 | 解消 | |
| E3 既定ブランチ名 | 解消 | ADR 中の `readable` は凍結ブランチの意味でだけ使われている(`grep -n 'readable`' ` で確認。37 行目の「`readable` の土台」は凍結ブランチの土台の説明なので正しい)。 |

---

## (2) `main` 構成で残る軽微な穴

### G1(軽微). 自己有効化しているのは `detect` だけで、`release` の schedule は 60 日で止まりうる
- 根拠: D4-1「毎回の最後に**自分自身の**ワークフローを有効化し直す」。`release` も `schedule` を持つ。
- 影響: `release` だけが止まった場合、`detect` の停止の検知(「新しいタグがあるのに M 日 Release が無い」)が Issue を立てるので、黙っては止まらない。
- 推奨: `detect` の最後で `detect` と `release` の両方を有効化し直す、と書く。

### G2(軽微). 60 日で無効化された「後」は、自己有効化では戻せない(依頼の論点)
- 自己有効化は、無効化される前に走っていることが前提。`detect` は毎日走るので、自己有効化の API 呼び出しが
  60 日の計時を戻すなら止まらない。**API の有効化が計時を戻すかどうかは GitHub の仕様として明示されていない**(未確認)。
  戻さない場合は、60 日目に `detect` も止まる。
- このときの受け皿が F1b(awase 側の監視)。ADR は「awase 側の定期ジョブ」とだけ書いていて、どのワークフローに足すかが無い。
  awase リポジトリは毎日動きがあるので、awase 側の schedule は止まらない。mozc-readable は公開リポジトリなので、
  ワークフローの `state` と最後の run の日付はトークン無しで読める(`gh api repos/cuzic/mozc-readable/actions/workflows`)。
- 推奨: 段階 0 の作業に「awase の既存の定期ワークフロー(または新規)に、mozc-readable の `detect`/`release` の `state != active` か、
  最後の run が 3 日以上前なら awase 側に Issue を立てるステップを足す」と書く。どの仕組みで止まっても最後はここで拾える。
  60 日の計時が API の有効化で戻るかは、段階 0 の後、実際に 60 日たってから確かめれば足りる(F1b があるので急がない)。

### G3(軽微). 古い版の承認待ちを取り消すには `actions: write` が要る
- 根拠: D4-1「新しい版の `release` が始まったら、承認待ちの古い版の run を取り消す」。concurrency のグループ名に版が入っているので、
  GitHub は自動では取り消さない(グループが違う)。`gh run cancel` は `actions: write` が要るが、D4-1 の権限の段落は
  `actions: write` を自己有効化のステップにしか付けていない。
- 推奨: 取り消しのステップにも `actions: write` を付ける、と権限の段落に足す。

### G4(軽微). series をコピーする PR には `pull-requests: write` と、リポジトリの設定が要る
- 根拠: D4-1「公開ジョブが…コピーする PR を作る」。`GITHUB_TOKEN` で PR を作るには `pull-requests: write` と、リポジトリの
  Settings → Actions の「Allow GitHub Actions to create and approve pull requests」の有効化が要る(既定では無効)。
  権限の段落は `contents: write` だけ。
- 推奨: 権限の段落と段階 0 の作業(リポジトリ設定)に足す。

### G5(軽微). N+1 の判定に使う「Release に記録した値」は、本文の文字列ではなく機械可読な asset にする
- 根拠: D4-1「本文には…使った series の SHA を書く」「直近の同じ upstream 版の Release に記録した値と違うときだけ N+1」。
  本文を後から人が編集すると判定が壊れる。
- 推奨: `build-info.json`(`upstream_tag`・`main_sha`・`series_dir`・`series_tree_sha`(`git rev-parse HEAD:readable/patches/publisher/<dir>`)・
  `identity_sha`)を Release の asset に付け、判定はそれを読む、と書く。series の「SHA」はディレクトリの tree の SHA だと明記する。

### G6(軽微). 凍結する `readable` から `main` へ材料を移す手順が、改訂で抜けた
- 根拠: round2 の版には「`git diff` を identity / publisher / ci の 3 つに分けて材料ブランチを作る」があったが、`bcc85446` には無い
  (`grep -n '3 つに分け' docs/adr/265-mozc-readable-integration.md` → 0 件)。依頼にある「別セッションの `readable-build.yaml` と
  `readable/ci` を `main` へ移す手順」もここに当たる。
- 推奨: 段階 0 の①の前に次を書く。
  (a) 別セッションの作業が終わった時点の `readable` の SHA を記録し、`readable` を保護する(①より先に保護すると、別セッションの
  push が拒否されて作業の終わりが明確になる)。
  (b) その SHA から `git diff 0136c357 <SHA>` を 3 つに分ける: identity の差分 → `identity.toml` の表、publisher の差分 →
  `git format-patch` で `patches/publisher/master/`(`BASE`=`0136c357`)、`readable/ci/` と `readable-build.yaml` の中身 → `main` の
  `readable/ci/` と `release` ワークフローの検証ジョブ。
  (c) 確認: `generate.sh 0136c357`(master の土台)で作ったツリーと `readable` の SHA のツリーが、`readable/ci/`・ワークフロー以外で
  一致すること(`git diff --stat` が空)。これで移行に取りこぼしが無いことを機械的に示せる。
  (d) `main` のワークフローのファイル名は `readable-build.yaml` と違う名前にする。GitHub は有効/無効の状態をファイルのパスで持つので、
  同名だと凍結ブランチ側の定義と状態が混ざる。

### G7(軽微). 段階 0 の順序は「既定ブランチを先に切り替えてから試す」で足りる(依頼の論点)
- ADR は「試すなら同名のワークフローを一時的に `master` に置く」。`master` は消してよいブランチで、既定ブランチを `main` に切り替えて
  困る利用者はいない(Release は 0 件、サイトの欄は `hidden`)。切り替えは Settings でいつでも戻せる。
- 推奨: 順序を「①`main` を作る → ②既定ブランチを `main` に変える → ③`workflow_dispatch` で `detect` と `release`(公開ジョブは承認で止める)を
  試す → ④PR #1 を閉じる」にし、`master` に一時的なワークフローを置く案は外す。一時ファイルを置くと、消し忘れたときに
  そのワークフローが既定でないブランチに残り、混乱の元になる。

---

## (3) 改訂で入った誤り

- 新しい誤りは見つからなかった。D4-6 の「トークンが要るのは Issue の作成、Release の公開、series をコピーする PR の作成、自己有効化だけ」は、
  G3(古い承認待ちの取り消し)が抜けているだけで、事実の誤りではない。

## 確認したこと・していないこと

- 確認した: `git show bcc85446`、ADR の D4-1・D4-5〜D4-7・未確認・段階、`docs/adr/index.md` の行(「Opus round3 反映済み」)、
  移行の記述が無いこと(G6 の grep)。
- 未確認(GitHub の仕様): 有効化の API が 60 日の計時を戻すか(G2)、`GITHUB_TOKEN` で PR を作る設定の既定値(G4)。
