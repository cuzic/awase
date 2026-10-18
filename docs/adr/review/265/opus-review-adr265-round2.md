# ADR-265 レビュー round2(Opus、読み取りのみ)

対象: `/home/cuzic/awase-mozc-readable` のコミット `a0ca35b0`(ADR-265 全面改訂、`docs/index.html`・`index.en.html`・`docs/adr/index.md`)。
2026-10-17 実施。round1 は `opus-review-adr265-round1.md`。

結論: round1 の致命 2 件は方針として解消した。ただし、新しく採った材料ブランチ構成(D4-1)の配線に**重要 3 件**の穴がある。
いちばん大きいのは N1(`build/*` を push する案と「Workflows 権限は要らない」「workflow_run の出どころ検査」が両立しない)。
N1 に対しては、`build/*` を push しない形を勧める。

---

## (1) round1 の各指摘の扱い

| ID | 判定 | 理由・残り |
| --- | --- | --- |
| C1 既定ブランチ | 解消(方針) | D4-1 で `readable` を既定にすると決めた。実際の切り替えは段階 0 の作業。 |
| C2 Deactivate 後の残留 | 解消(方針)、実装上の注意 2 点 | (a) publisher の `Unchanged()` は状態 4 つしか比べない(`tip_state_publisher.cc` の `ThreadSlot::Unchanged`)。`active` を比べる対象に入れないと、再び Activate したとき(`OnInitialize` の状態が前回と同じなら)`active=1` が書かれず、0 のまま残る。D3 に「`active` も比べる対象に入れる」と書くこと。(b) 読み手は **version 1 のブロックを読めないものとして捨てる**と書いていない(D3 には「version を 2 に上げる」としか無い)。旧版の mozc-readable を入れた利用者で C2 が再発する。「`version >= 2` のときだけ採る」を読み取りの規約に足すこと。 |
| I1 履歴の持ち方 | 解消、ただし配線に穴(N1〜N4) | 下の (2)。 |
| I2 版を下げる載せ替え | 解消 | (a) 初回の手直し、(b) 公開済みより新しいタグだけ、(c) `static_assert`(D4-4)がそろった。 |
| I3 identity の取りこぼし | 不足(順序) | 置換表に足すものは全部入った。ただし「**最初の Release は I3 の置換を当てた後に出す**」という順序が書かれていない。今の `readable` のまま 3.34.6239 を出すと、`Manufacturer="Google LLC"`・言語一覧の表示名 "Mozc" の MSI が公開される。サイトの「Google とは無関係の非公式の派生版」「別名で作っています」と食い違う。段階 0 の完了条件に入れること。 |
| I4 再リリース | 不足 | 「層に変更が入ったら N+1」は決めたが、何を契機に動かすか(`readable` への push の path フィルタ、承認の有無)が無い。N5 参照。 |
| I5 Issue 重複・黙った停止 | 解消(文面)、軽微な残り | 「N 日以上残っていたら」の N が未定。N1 の推奨どおり `build/*` を push しない形にすると「残存したブランチ」という検知手段自体が無くなるので、「最後に成功した生成から M 日、新しいタグがあるのに Release が無い」で検知する形に書き換えること。 |
| I6 式の直埋め込み | 解消 | |
| I7 workflow_run の出どころ | 不足 | 条件 `event in (push, workflow_dispatch)` は良いが、D4-5 の「`build/*` のとき」という絞り込みと両立しない(N1)。 |
| I8 PAT の権限 | 不足(前提の誤り) | 「生成ブランチにワークフローを含めないので Workflows 権限は付けない」は成り立たない見込み(N1)。 |
| I9 update_tick | 解消 | |
| I10 古いブロック・tid 再利用・seq==0 | 解消 | version 1 の扱いは C2(b)。 |
| I11 読むスレッド | 解消 | |
| I12 AppContainer | 解消(測定を段階 1 の前に置いた) | |
| I13 TipIdentity | 解消 | |
| I14 信頼度 | 解消 | |
| I15 見落とし検査 | 解消、軽微な注意 | `TipTextService` の Activate/Deactivate を publisher 層に足すと、upstream が頻繁に触る大きいファイル(`tip_text_service.cc`)に差分が入り、分類 B の頻度が上がる。差分は 1 行の呼び出しだけに留める(書き出しの本体は `tip_state_publisher.cc` に置く)と D4-2 に書くとよい。 |
| M1 GUID の書式 | 解消 | |
| M2 identity.py の出どころ | 解消 | |
| M3 整形 | 解消 | |
| M4 タグ | 解消、ただし新しい誤り E1 | |
| M5 heredoc | 対応不要(バグではない) | |
| M6 Soft 検査 | 解消 | |
| M7 乗っ取り | 解消(信頼境界を明記) | |
| S1 サイト | 解消、軽微な残り | 3 文は消えた。残りは E3。 |

---

## (2) 材料ブランチ構成で新しく生じた穴

### N1(重要). `build/*` の push は Workflows 権限を要し、workflow_run の head_branch も `build/*` にならない
- 根拠:
  - `build/<タグ>` は upstream のタグのツリーをそのまま土台にするので、upstream の `.github/workflows/{android,lint,linux,macos,windows}.yaml` を含む
    (`gh api repos/cuzic/mozc-readable/actions/workflows` → この 5 本が fork にあり、いずれも `disabled_manually`)。
    `readable`(既定)のワークフロー一式とは中身が違う。GitHub はワークフローのファイルを作る・変える push を、`workflows` 権限の無い
    トークンから拒否する。新しいブランチの push がこれに当たるかは**未確認**だが、当たる見込みが高い。D4-6 の
    「Workflows 権限は付けない」と矛盾する。
  - `readable-build.yaml` は材料ブランチ(`readable`)にしか無い。`gh workflow run readable-build.yaml --ref build/x` は、
    その ref にワークフローの定義が無いと失敗する。`--ref readable` で動かして、入力で `build/x` を渡す形になる。
    すると run の `head_branch` は `readable` になり、D4-5 の「`build/*` のとき」の判定も、round1 の release 側の
    `case "$BRANCH" in sync/*` 型の判定も使えない。
  - upstream が今後のタグで**新しいワークフローのファイル**を足すと、そのパスは `disabled_manually` になっていないので、
    `build/*` の push で upstream の CI(macOS・Android など)が走る。
- 推奨: **`build/*` を push しない**。生成(タグ + `identity.py apply` + `git am -3` + ci のコピー)からビルド・検証までを
  `readable` の 1 本のワークフローの中で行う(またはジョブ間を artifact で渡す)。Release には
  「upstream のタグ・材料ブランチのコミット SHA・patch series の SHA」を本文に書き、生成したソースを `git archive` して
  Release の asset に付ける(BSD なのでソース公開の義務は無いが、再現と監査のため)。Release のタグは材料ブランチの
  コミットに付ける(`v3.34.6239-readable.1` → `readable` の SHA)。これで (a) upstream のツリーを push しないので
  Workflows 権限も upstream の CI の誤起動も無くなる、(b) 公開ジョブは同じワークフローの後段ジョブにでき、`workflow_run`
  自体が要らなくなる(R1-I7 の出どころ検査も不要)、(c) 公開の前に人が 1 回見る(D4-5)は `environment` の承認で実装できる。

### N2(重要). `git am -3` は patch の元の blob が手元に無いと 3-way にならない
- 根拠: `git am -3` は patch の `index <old>..<new>` 行の元 blob をリポジトリから探して仮の祖先を作る。無ければ
  「could not build fake ancestor」で 3-way をあきらめ、厳密な適用だけになる。ワークフローがタグだけを浅く取ると
  (`fetch-depth: 1`、`--filter`、`--depth`)、元 blob(patch を作った土台のもの)が無い。
- 失敗シナリオ: 本当は 3-way で自動で通る差分(upstream が周りの行を少し変えただけ)が、毎回分類 B の衝突として人に回る。
- 推奨: patch series の土台のコミット(`patches/publisher/BASE` に SHA を書く)を生成ジョブで必ず fetch する
  (blobless + 必要な blob の遅延取得でよい)。ADR に「series は `BASE` のファイルを持つ」と書く。

### N3(重要). patch series が 1 つだと「master への早期修正」と「現行タグでの再リリース」がぶつかる
- 根拠: D4-5 は毎日 master に対して生成を試し、失敗したらリリース前に直すとする。D2 は現行の upstream 版のまま N+1 を出すとする。
  series が 1 つしか無いと、master 向けに直した series は現行タグ(例: 3.34.6239)に当たらなくなる(実例: round1 で示した
  `6b5b8c12` の前後で `tip_input_mode_manager` の差分が当たらない)。master 向けに直すと現行タグの再リリースが壊れ、
  現行タグに合わせておくと早期修正ができない。
- 推奨: series を土台ごとに持つ(`patches/publisher/<土台>/`。例: `3.34.6239/` と `master/`)。新しいタグが出たら、そのタグに
  当たる series を選び、無ければ `master/` を試す。または「早期検知は失敗を Issue にするだけで、直すのはタグが出てから」と
  割り切る。どちらかを ADR に書くこと。

### N4(軽微). 衝突の解消手順と patch の作り直し
- D4-5 の分類 B「衝突解消用のブランチと Draft PR」は、`git am` が止まった状態のツリーを人がどう受け取り、どう series に
  戻すかが書かれていない。手元で再現できる 1 本の入口(`readable/tools/generate.sh <タグ>`: 生成して止まったところで終わる)と
  `readable/tools/refresh-patches.sh`(解消したツリーから `git format-patch <タグ>..` で series を作り直し、`BASE` も更新する)
  を材料ブランチに置く、と書くとよい。CI と手元で同じスクリプトを使うことが、`build/*` の生成を再現できる条件になる。

### N5(重要). `readable` への push のたびに 1〜2 時間のビルドと N+1 の Release が出る
- 根拠: いまの `readable-build.yaml` は `push: branches: [readable]` で起動する(`origin/readable` の同ファイル 1〜7 行目)。
  材料ブランチ構成では README・ci スクリプト・ワークフローの修正も `readable` への push になる。D2 の「層に変更が入ったら N+1」を
  そのまま push に結び付けると、MSI の中身が変わらない変更でも Release が増える。
- 推奨: N+1 を作る契機は `readable/patches/**`・`identity.toml` の変更に限る(path フィルタ)。ci だけの変更は検証だけ走らせて
  公開しない。公開は N1 の `environment` 承認を通す。

### N6(軽微). 既定ブランチがソースを持たないことの影響
- fork のトップページはソースの無いツリー(材料)になる。clone した人が「何をビルドすればよいか」が分からないので、
  `readable` の README に「`readable/tools/generate.sh <タグ>` で生成してビルドする」と書く。
- GitHub の "Sync fork" は表示中のブランチと upstream の同名ブランチを同期する機能なので、`readable` では出ない。
  `master` を upstream のミラーとして残すなら、ミラーの更新は自前のジョブ(または手動)になる。早期検知は upstream を
  直接 fetch するので `master` のミラーは不要。消してよい。
- upstream へ PR を出す導線: publisher の変更を upstream に提案する場合は、`master`(または upstream の最新)から作業ブランチを
  切って series を当てればよく、既定ブランチの中身は関係しない。fork の compare の既定が `google/mozc:master...cuzic:readable`
  になり、ツリー全体が消える差分に見えるが、実害は表示だけ。
- `readable` を孤立した(upstream と履歴を共有しない)ブランチにするか、`legacy/readable-src` の上に「ソースを消すコミット」を
  積むかを決めておく。後者は upstream との関係が残るが、fork の「N commits ahead/behind」が意味のない数字になる。どちらでも
  動作は同じなので、ADR に一言書けば足りる。

---

## (3) 改訂で入った誤り

### E1(軽微). `grep -F` と行頭の `^` は一緒に使えない
- 根拠: D2「`grep` ではタグの `.` を固定文字列として扱う(`grep -F`)」。round1 の対象は `grep -q "^v${TAG}-readable\."`
  だった。`-F` にすると `^` も文字として扱われ、一致しなくなる(常に「未公開」と判定して同じ版を毎日作り直す)。
- 推奨: `gh release list --json tagName --jq` で名前を取り、`jq` の `startswith("v\($tag)-readable.")` で比べる。
  または `grep -E "^v${TAG//./\\.}-readable\."`。

### E2(軽微). 実測していない挙動を事実として書いている
- 根拠: D3「Win+Space で同じスレッドの IME を MS-IME に切り替えても共有メモリに『開・ひらがな』が残るため」は、round1 で
  ソースを読んで推論したもので、実機・CI では測っていない(publisher が `Deactivate` で書かないことはソースで確認済み、
  残った値を awase 側が読むことは未実測)。
- 推奨: 「ソース上は残る(未実測)」と書き、段階 1 の CI に「readable で開にする → MS-IME に切り替える → `active==0` になる」を足す。

### E3(軽微). サイトの「別名で作っています」と、今の配布物の名前
- 根拠: `docs/index.html` 新 690 行目「公式の Mozc とは別名(別の識別子・別のインストール先)で作っています」。今の
  `readable` から作った MSI は、言語の切り替えの表示名が "Mozc"、発行元が "Google LLC" のまま(round1 I3)。
  ダウンロードのボタンは、Release が 0 件(`gh release list -R cuzic/mozc-readable` → 出力なし)なので、Releases の空ページへ飛ぶ。
- 推奨: このブランチを develop に入れるのを、最初の Release(I3 の置換を当てたもの)が出るまで待つ。先に入れるなら、
  欄ごと `hidden` にしておく。

### E4(軽微). `index.md` の行
- 「Opus round1 反映済み・再確認待ち。実装は未着手」は正しい。round2 の反映後にステータスを更新すること(指摘ではなく手順の確認)。

---

## 確認したこと・していないこと

- 確認した: `git show a0ca35b0`(ADR・サイト・index の差分)、`gh api repos/cuzic/mozc-readable/actions/workflows`
  (upstream 5 本は `disabled_manually`、`readable build (x64 MSI)` だけ active)、`gh run list -R cuzic/mozc-readable`
  (直近 15 件中 14 件が `readable build` の push 起動)、`tip_state_publisher.cc` の `Unchanged()`。
- 未確認: 新しいブランチの push に upstream のワークフローのファイルが含まれる場合に `workflows` 権限が要るか(N1)、
  `git am -3` が blobless clone で遅延取得により元 blob を取れるか(N2。取れる見込みだが試していない)。
