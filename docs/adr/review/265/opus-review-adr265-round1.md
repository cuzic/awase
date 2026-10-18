# ADR-265 レビュー round1(Opus、読み取りのみ)

対象: `docs/adr/265-mozc-readable-integration.md`(feat/mozc-readable-integration、HEAD `d9c3b9b8`)、
mozc-readable PR #1(head `5dd03f13`、`ci/release-automation`)、`origin/readable`(`0ea8fb8`)。
2026-10-17 実施。

注意: `/home/cuzic/mozc-readable-release/repo` は shallow clone(`git rev-parse --is-shallow-repository` → `true`、
`up/master` は 200 コミットだけ)で、`merge-base` や `--is-ancestor` が誤った答えを返す。祖先関係の確認は
scratch に blobless で取り直した(`git fetch --filter=blob:none https://github.com/google/mozc.git master 'refs/tags/3.34.6239:...'`)。
同じ clone で祖先を判定する作業がほかにあれば、その結論は疑うこと。

---

## 致命

### C1. 既定ブランチが `master` のままなので、D2 の自動化が一度も動かない
- 根拠: `gh api repos/cuzic/mozc-readable --jq .default_branch` → `master`。`readable-upstream-sync.yaml` の
  `schedule` と `readable-release.yaml` の `workflow_run` は、**既定ブランチにあるワークフロー定義でしか発火しない**
  (GitHub Actions の仕様)。PR #1 の 2 本は `readable`(PR #1 の base)に入るだけ。
- 失敗シナリオ: PR #1 をマージしても毎日の追随も Release 公開も起きない。エラーも出ないので「動いていない」ことに気付かない。
  awase.cc は「自動で追随して公開します」と書いている(下の S1)。
- 推奨: ADR に「既定ブランチをどれにするか」を決定として書く。`readable` を既定にするなら、その影響(fork の
  "Contribute"/compare の既定が readable になる、`master` をミラーとして残すか消すか)も書く。
  下の I1(履歴の持ち方)で「書き換えないブランチ」を既定にすると、この問題と force-push の問題が同時に消える。

### C2. TIP が無効化された後も共有メモリに最後の状態が残り、awase は「読めた」と判定する
- 根拠: `tip_state_publisher.cc`(readable)は `TipInputModeManager` の `On*` 6 つからしか書かない。
  TIP の `Deactivate`(Win+Space で同じスレッドの IME を MS-IME 等へ切り替えたとき)では何も書かない。
  マッピングは `thread_local ThreadSlot` が持ち、スレッドが生きている限り(DLL が解放されても、ハンドルは
  プロセスに残る)最後の値(例: `effective_open=1, mode=1`)のまま `magic`/`version` も正しい。
  ADR D3 の受理条件(開ける・magic/version 一致・seqlock 一致・`update_tick_ms` が新しい)はこれを弾けない。
- 失敗シナリオ: mozc-readable と MS-IME を両方有効にしている利用者が、ある窓で MS-IME に切り替えて IME を閉じる。
  awase は共有メモリから「開・ひらがな」を読み、belief を開に寄せる。belief の誤りが非対称な方向(開/閉の取り違え)
  に出るので、ADR-089 INV-45 の「推測値で分岐してよいのは誤っても被害が対称な選択だけ」に反する。
  awase 側からは、別スレッドのアクティブな TIP を確かめる手段が無い(`ITfInputProcessorProfileMgr::GetActiveProfile`
  は呼び出し元スレッドのみ、`GetKeyboardLayout(tid)` は TIP 系なら同じ `0x04110411`)。D3 の
  「`EnabledJaTip` に CLSID がある」は**有効化済み**を示すだけで、その窓で**いま使われている**ことを示さない。
- 推奨: publisher に `active` を足す(`TipTextService::Activate`/`Deactivate` で 1/0 を書いて seq を進め、
  version を 2 に上げる)。awase は `active==1` のときだけ読めたとする。これは publisher 層の差し込み点を増やすので、
  D4 の hooks 検査の対象(`TipTextService` 側)も増やすこと。

---

## 重要

### I1. 載せ替えのたびに `readable` を force-push する設計は避けられる(履歴の持ち方)
- 根拠: ADR D4「ブランチと載せ替え」は 15 コミットを 3〜4 にまとめ直す force-push と、以後の各 sync で `readable` を
  新しい履歴に置き換えることを前提にしている。PR #1 の `Advance readable`(`readable-release.yaml:92-107`)も
  `--force-with-lease="readable:${CUR}"` で上書きする。
- 失敗シナリオ:
  - PR #1 自体の base が `readable`(`gh pr view 1 --json baseRefName` → `readable`)。まとめ直しで PR #1 の差分が崩れる。以後も、
    `readable` に向けた PR は sync のたびに壊れる。
  - 手元の clone(`/home/cuzic/mozc-readable-release/repo` の `readable` など)で `git pull` すると、古い履歴と新しい履歴が
    マージされて同じ変更が二重になる。古い clone から `--force`(lease なし)で push すると、追随の成果が消える。
  - `sync-base/<タグ>` と「動いていたら Issue」の分岐は、force-push を安全にするためだけにある仕組み。
  - なお Release のコミットの到達性は問題ない(`gh release create --target` がタグを作るので、`readable` が先へ
    置き換わってもタグから辿れる)。
- 推奨(どれか 1 つを ADR に決定として書く):
  - **(推奨)書き換えない「材料」ブランチ + 使い捨ての生成ブランチ**: 既定ブランチ `readable` には upstream のソースを置かず、
    `readable/`(`identity.toml`・`hooks.toml`・`patches/publisher/*.patch`〈`git format-patch` の series〉・ci スクリプト)と
    `.github/workflows/` だけを持つ。各リリースは `build/<タグ>` = タグ + `identity.py apply` + `git am -3 patches/...` +
    ci の上書きコピーを生成し、Release タグで固定する。`readable` は通常の PR・fast-forward だけで進む。
    force-push・`sync-base`・「動いていたら止める」がまるごと不要になり、C1 も解消する。
    publisher の修正は patch を直す普通の PR になり、I4(同じタグの再リリース)にも自然につながる。
    代償は「patch のレビューがコミットのレビューより読みにくい」ことだけで、`git am -3` で 3-way マージは効く。
  - upstream タグごとの `readable/<ver>` を作り、`readable` は最新の `readable/<ver>` を指すだけにする案も成り立つが、
    `readable` の移動は結局 force になる。
  - 毎回 upstream タグを `readable` にマージする案は、identity 層の行がマージのたびに衝突し、D4 の「identity は毎回生成」と
    両立しないので勧めない。

### I2. 最初の追随は「版を下げる」載せ替えになり、publisher の差分が当たらない
- 根拠:
  - `readable` は upstream master `0136c357`(3.35.6354 の開発版)の上。最新の X.Y.Z タグは `3.34.6239`(2026-06-24)で、
    master より 216 コミット古い(blobless fetch で `git rev-list --count 3.34.6239..up/master` → `216`)。
  - タグは master の祖先(`git merge-base --is-ancestor 3.34.6239 up/master` → 真。`3.33.6133`・`3.33.6089` も同じ)。
    ADR の「タグは master から切られる」は正しい。
  - `git diff 0136c357 origin/readable -- src` を `3.34.6239` に `git apply --check` すると、
    `tip_input_mode_manager.{h,cc}` だけ失敗、残りの identity 5 ファイルは通る。原因は upstream `6b5b8c12`
    (`Remove TipInputModeManagerImpl::ConversionMode (#1526)`)がタグより後にあること。
- 失敗シナリオ: PR #1 を動かすと初回から衝突(D4 の分類 B)。さらに PR #1 の `Pick the upstream tag` は
  `readable` の現在の版との大小を比べないので、upstream が古いタグしか持たない状況では常に「版を下げる」sync を試みる。
- 推奨: (a) 最初の Release は 3.34.6239 向けに publisher を手で直して作る(分類 B の手順の予行演習になる)と ADR に書く。
  (b) sync は「直近に公開した Release の upstream 版より新しいタグ」だけを対象にする。
  (c) publisher の `static_cast<int>(mozc_state_.conversion_mode)` は enum の型が変わっても黙って通る
  (3.34.6239 では `TipInputModeManagerImpl::ConversionMode`、master では `commands::CompositionMode`。今は値が一致)。
  `static_assert` で 0〜5 の値を固定し、型や値の変更をビルド失敗にする。

### I3. identity の置換表に入っていない「Mozc」が残っていて、利用者からは公式 Mozc と区別できない
- 根拠(すべて `origin/readable`):
  - `src/win32/tip/tip_resource.rc:148-149` `IDS_IME_DISPLAYNAME "Mozc"`・`IDS_TEXTSERVICE_DISPLAYNAME_SYNONYM "Mozc"`。
    `tsf_profile.cc` の `GetDescriptionTextIndex()` がこの ID を返し、`tsf_registrar.cc` が
    `SetLanguageProfileDisplayName` で地域化された表示名として登録する(言語の切り替えに見える名前)。
    不変の説明は `kProductNameInEnglish`(= "Mozc Readable")だが、日本語 UI では "Mozc" が出るはず
    (実機の表示は**未確認**)。
  - `installer_oss_64bit.wxs:60-61` `Manufacturer="Google LLC"`(Package と SummaryInformation の 2 か所)。
    「アプリと機能」の発行元が Google LLC になる。awase.cc は「Google の名称・商標を使うものではありません」と書いている。
  - 同 `wxs` の `Description="Mozc インストーラー"`、`Feature Title="Mozc"`、エラー文言 4 か所。
  - `cache_service/mozc_cache_service.rc:73,84` `"Mozc Cache Service"`(サービス一覧に同名が 2 つ並ぶ。SCM の重複表示名
    エラーは、登録値が `@<パス>,-100` でパスが違うので起きないと見るが**未確認**)。
  - `custom_action.rc:73,86`、`tip_linguistic_alternates.cc:66` `kSearchCandidateProviderName = L"Mozc"`、
    `const.h:126` `kConfigurationDisplayname = L"Mozc Configuration"`(`tip_text_service.cc:851` が表示名として返すだけで、
    名前の検索には使われていない — `git grep FindWindow` は 0 件)。
- 衝突(動作上の干渉)の有無:
  - IPC・イベント・ミューテックス・サービス名・レジストリ(`kMozcRegKey`・Policies)・Run キー・cab 名・インストール先・
    UpgradeCode・CLSID・プロファイル GUID は別名化済み。旧 GUID は `src/` に残っていない
    (`git grep -i -E '10A67BC8|186F700C|DD94B570' origin/readable` → `readable/ci/verify.ps1` だけ)。
  - 利用者プロファイルは `kProductNameInEnglish` から作るので `...\Mozc Readable` に分かれる。公式 Mozc の
    ユーザー辞書は引き継がれない(干渉ではないが awase.cc に書く価値がある)。
  - `SysProcs` コンポーネント(`wxs:368-371`)は公式と同じレジストリ値をキーパスに持つ。WiX の自動 GUID は
    キーパスから作られるので、公式 Mozc と**同じコンポーネント GUID になり参照カウントを共有する**可能性が高い
    (**未確認**。値が同一なので実害は小さいが、MSI の設計としては製品ごとに分けたい)。
  - `mozc_tip64.dll` は同じ基本名。COM は InprocServer32 の完全パスで読むので別々に読まれるはずだが、両方を
    同じプロセスで切り替えたときの挙動は**未確認**。
  - `verify.ps1` は「公式 Mozc が入っていない」ことを確かめるだけで(`:26`・`:31`)、**公式 Mozc を入れた状態で共存する**
    検査が無い。ADR・awase.cc・Release 本文の「同時にインストールできる/干渉しません」は CI で裏付けられていない。
- 推奨: identity 表に `tip_resource.rc`・`wxs` の Manufacturer/Description/Title/文言・`mozc_cache_service.rc`・
  `custom_action.rc` を足す。`check` に「`src/win32` 配下の `.rc`/`.wxs` に `"Mozc"` という完全一致の文字列が残っていない」
  (許可リスト付き)を足す。CI に「公式 Mozc の MSI を先に入れ、readable を入れ、両方を切り替えて打鍵し、片方を
  アンインストールしてももう片方が動く」検査を足す。

### I4. readable 層だけの修正(publisher の不具合修正など)を同じ upstream 版で再リリースする経路が無い
- 根拠: `readable-upstream-sync.yaml:62-69` は `v<タグ>-readable.*` が 1 つでもあれば skip。
  `readable-build.yaml` の `push: [readable]` で走ったビルドは、`readable-release.yaml:45` が `sync/*` 以外を捨てる。
  N が 2 以上になるのは、人が手で `sync/<タグ>` を作り直すか `workflow_dispatch` で run_id を渡したときだけ。
- 失敗シナリオ: publisher の誤り(例: C2)を直しても、次の upstream タグ(数か月後)まで配布物に入らない。
- 推奨: 「`readable` に publisher/identity/ci 層の変更が入ったら、現在の upstream 版で N+1 を作る」ことを ADR に決める。
  I1 の推奨構成なら、`readable` への push を契機に `build/<現行タグ>-<N+1>` を生成すればよい。

### I5. 衝突のたびに毎日 Issue が増える/ビルド失敗が黙って止まる
- 根拠:
  - 衝突時(`sync.yaml:89-93`)は `sync/<タグ>` を push せず、Issue を作って終わる。翌日は skip 条件(Release も
    `sync/<タグ>` も無い)に当たらないので、同じ衝突でもう一度 Issue を作る。
  - ビルド・検証が失敗した場合(分類 D)、`sync/<タグ>` と `sync-base/<タグ>` が残り、翌日以降は
    `sync/<タグ> already exists` で `exit 0`(`:70-72`)。`readable-release.yaml:46` も「nothing to do」で `exit 0`。
    **誰にも通知されないまま追随が止まる**。
- 推奨: ADR の「キーで 1 件、再発はコメント」を実装の要件にする(`gh issue list --search "in:title <キー>" --state open` を
  見てからコメント)。分類 D は release 側で `conclusion != success` かつ `sync/*` のときに Issue を立てる。
  `sync/<タグ>` が N 日以上残っていたら Issue を立てる。

### I6. 式の直埋め込み(`${{ }}` を shell に展開)と入力の未検証
- 根拠: `sync.yaml:53` `TAG="${{ github.event.inputs.tag }}"`、`:66,80,102,110` `${{ steps.pick.outputs.tag }}`、
  `release.yaml:75-76,96-97` `${{ steps.run.outputs.upstream }}`(これは**ブランチ名** `sync/...` から作る)。
  git の ref 名は `$`・`(`・`)`・`;`・`` ` `` を許すので、`sync/$(…)` というブランチ名は作れる。
  実行時の環境には `GH_TOKEN=SYNC_TOKEN` がジョブ全体で入っている(`sync.yaml:31-32`、`release.yaml:29-30`)。
- 失敗シナリオ: 書き込み権限のある者(または将来の自動化)が作った異常なタグ入力・ブランチ名で、PAT 付きの任意コマンドが走る。
  入力が `master` や `3.34.6239-rc` でも検証されずに進む。
- 推奨: すべて `env:` 経由で渡し、`[[ "$TAG" =~ ^[0-9]+(\.[0-9]+){2,3}$ ]]` で検証してから使う。`GH_TOKEN` は必要なステップだけに置く。
  `actions/checkout` に PAT を渡すと `.git/config` に残る(`persist-credentials` 既定 true)ので、D4 で identity.py・
  hooks 検査など**リポジトリ内のスクリプトを動かすジョブでは** `persist-credentials: false` にし、push の直前だけ
  認証を付ける。

### I7. `workflow_run` の出どころを確かめていない(将来の pwn request)
- 根拠: `release.yaml:41-46` は run の `head_branch` が `sync/*` で `conclusion == success` かだけを見る。
  `head_repository`・`event` を見ない。今の `readable-build.yaml` は `push`/`workflow_dispatch` だけなので現時点では
  到達しないが、ビルドに `pull_request` を足した瞬間、fork の PR(ブランチ名 `sync/x` は fork 側で自由に付けられる)の
  成果物を PAT で Release として公開できるようになる。
- 推奨: `head_repository.full_name == github.repository` かつ `event in (workflow_dispatch, push)` を条件に足す。

### I8. SYNC_TOKEN の権限
- 根拠: `sync.yaml:8` は「contents・workflows・issues の書き込みができる PAT」とだけ書く。classic PAT の `workflow`
  スコープは利用者の全リポジトリのワークフローを書き換えられる。`gh api repos/cuzic/mozc-readable/actions/secrets` は
  空(設定済みかは**未確認**、権限不足で見えていないだけの可能性もある)。
- 推奨: fine-grained PAT を `cuzic/mozc-readable` 1 つに限定(Contents・Workflows・Issues・Actions〈artifact 取得〉の
  read/write、有効期限付き)と ADR に書く。I1 の構成にすれば `.github/workflows/` を生成ブランチへ push する必要が
  無くなり、Workflows 権限ごと外せる可能性がある(生成ブランチにワークフローを含めないなら)。

### I9. D3 の `update_tick_ms` が新しいとき」という条件は publisher の意味と合わない
- 根拠: `tip_state_publisher.cc` は状態が変わらないときは何も書かない(`Unchanged()` で return、コミット `340b910`)。
  `update_tick_ms` は「最後に**変わった**時刻」であって生存の印ではない。
- 失敗シナリオ: 閾値を置くと、1 時間 IME の状態を変えていない窓を「読めない」とする(取りこぼしが増える)。
  閾値を置かないなら条件の意味が無い。どちらにしても、プロセス終了後の古いブロック(下の I10)は弾けない。
- 推奨: 条件から外し、生存の判定は I10 の `process_id` 照合と C2 の `active` で行う。

### I10. 古い共有メモリとスレッド ID の再利用
- 根拠: マッピングはページファイル裏付けで、最後のハンドルが閉じると消える。awase がハンドルを保持し続けると、
  TIP のプロセスが終わってもブロックが残り、値は最後のまま。スレッド ID は再利用されるので、別プロセスの新しい
  スレッドが同じ番号になると、awase は古いブロックを新しい窓の状態として読む。逆に TIP 側が先にあるブロックを
  開いた場合(`CreateFileMappingW` が既存を返す)、最初の書き込みまで古い値が見える。
  また `ThreadSlot::Get()` は `magic`/`version` を書いた時点で `seq=0`(偶数)・中身 0 のブロックを公開する。
  最初の `Publish` の `InterlockedIncrement` までの間に読むと「閉・DIRECT」が正しく読めてしまう。
- 推奨(awase 側の読み取り規約として D3 に書く): (a) 読むたびに Open→Map→読む→Unmap→Close し、ハンドルを持ち続けない。
  (b) `process_id` が前面窓(下の I11 のスレッド)の PID と一致しないものは捨てる。(c) `seq == 0` は「未公開」として捨てる
  (publisher 側で `magic` を最後に書くよう直すのがより良い)。(d) seqlock の手順を正確に書く
  (`seq1` を読む→偶数でなければ捨てる→中身→`seq2`→`seq1==seq2` のときだけ採る。ADR の「読みが 2 回一致」は
  中身を 2 回読むとも取れる)。Rust 側は `read_volatile` と `fence(Acquire)` が要る。

### I11. どのスレッドの共有メモリを読むか
- 根拠: ADR D3 は「前面窓のスレッド ID」。TSF の状態は**キーボードフォーカスを持つ窓のスレッド**ごと
  (ADR-191 の `191-gji-state-scope-spec.md:25`)。前面窓とフォーカス窓のスレッドが違う窓がある
  (UWP は前面が `ApplicationFrameHost` の窓で、入力は別プロセスの `CoreWindow` のスレッド。別スレッドの子窓を持つアプリ)。
  CI では Chrome と Notepad は前面窓のスレッドで読めた(run `39093102390` の `chrome: shared block exists for the
  foreground thread` が PASS)。
- 推奨: awase が既に持っているフォーカスの解決(`GetGUIThreadInfo` の `hwndFocus`、UWP の `CoreWindow` の扱い)を使い、
  そのスレッドを読む、と D3 に書く。

### I12. AppContainer では名前空間が分かれるので、`Local\` で開けない見込みが高い
- 根拠: AppContainer 内のプロセスが作る `Local\...` は `\Sessions\<n>\AppContainerNamedObjects\<AppContainer SID>\` へ
  振り分けられる(Windows の仕様)。CI の AppContainer 検査は準備段階で失敗しており結論が無い
  (run `39093102390`: `NO appcontainer: window became foreground`・`exit 2`)。通知イベントも同じく分かれるので、
  AppContainer の TIP の pulse は awase に届かない。
- 推奨: 段階 1 の前に、対象プロセスのトークンの AppContainer SID を取り、その名前空間のパスで開けるか(`OpenFileMappingW`
  で届くのか `NtOpenSection` が要るのか — **未確認**)を CI で測る。開けなければ「UWP は従来経路」と ADR に明記する。

### I13. `TipIdentity` に variant を足すと、観測と無関係なキー選択まで変わる
- 根拠: `ime_kind.rs:70-76` の `TipIdentity` は `Gji`/`MsImeNative`/`Other`。公式 Mozc も今は `Other`。
  D3 は「`TipIdentity` に variant を足す形」。`TipIdentity` は送信キーの戦略選択(`ime_key_sequence_golden.rs` が固定する
  ImmCross → GjiDirect → MsImeDirect)の入力でもあり、新 variant を足すと網羅 match のすべてで扱いを決める必要がある。
- 推奨: 「共有メモリを読める」は `TipIdentity` と独立した能力(例: `readable_tip_enabled: bool`)として持ち、
  キー選択は今まで通り `Other` 扱いのまま(または mozc を GJI 系として扱う判断は別 ADR)にする。

### I14. D3 の信頼度「測定前は Medium 以下」は安全側になっていない
- 根拠: `.claude/rules/ime-belief-architecture.md:28,50`: `derive_open()` は Medium+ の合意で採り、`InputModeObserved` は
  Medium+ で `input_mode` を上書きする。Medium は「採られる」側。
- 推奨: 段階 2 の初期値は Low(記録のみ)と書き、Medium に上げる条件(段階 1 の journal で一致率何%・何件)を
  先に決める。新しい `ObservationSource` を足すと `architecture_guard.rs` の構築箇所数と `observation_source_guard` の
  対象になる点も段階 2 の作業に書く。どのフィールド(`effective_*` = 入力スコープの上書き後、`tsf_*` = compartment の値)
  を belief のどの軸に入れるかも決まっていない。awase の belief は compartment 相当なので `tsf_*` が素直だが、
  パスワード欄などで `effective` と食い違うときの扱いを決めておく。

### I15. publisher の見落とし検査は名前の一覧だけでは足りない
- 根拠: 現在の非公開 `On*` は `OnDissociateContext`・`OnTestKey`・`OnKey`・`OnMoveFocusedWindow`(書き出しなし)、
  書き出しありは 6 つ。状態(`mozc_state_`/`tsf_state_`)を書くのは今はこの 6 つだけ
  (`git show origin/readable:src/win32/tip/tip_input_mode_manager.cc | grep -n '_state_.*='`)。しかし upstream が
  `On` で始まらない変更メソッド(`Reset`・`SetConfig` 等)を足した場合や、既存の「書き出しなし」メソッドに状態の書き込みを
  足した場合は、名前の一覧は変わらず検査を通る。C2 の対策を入れると `TipTextService` 側も対象になる。
- 推奨: 検査を「`mozc_state_`/`tsf_state_` に代入する関数の集合 ⊆ 書き出しを呼ぶ関数の集合」にする(テキスト走査で足りる)。
  名前の一覧は補助として残す。

---

## 軽微

### M1. identity の GUID の書式は 3 つでは足りない
- `build_installer.py:134` の UpgradeCode は波括弧なしの `'B5E1D5C5-...'`。ADR の「`{AFC9…}` の文字列・コメント・C の構造体」の
  3 書式に入らない。波括弧なし(と、念のため小文字)を 4 つ目の書式にする。
- 検査 ③「新しい GUID が表にあるファイル以外に出ていない」は、範囲を書かないと `readable/ci/verify.ps1:6-7`
  (新 CLSID・プロファイル)で落ちる。範囲を `src/` に限ると書く。

### M2. `identity.py` をどのツリーから動かすか
- `sync/<タグ>` の手順は「タグを checkout → `identity.py apply`」だが、タグのツリーには `readable/identity.py` が無い。
  `readable` の別 worktree(または I1 の材料ブランチ)から動かすと書く。

### M3. 置換後の行が upstream の clang-format に合わない
- `const.h:117` `kMessageReceiverClassName ... L"mozcreadable.renderer.window";` は 80 桁を超える。生成物なので実害は無いが、
  「元の 1 行 → 新しい 1 行」の表は upstream が整形し直したとき(1 行が 2 行に割れる)にアンカー消失(分類 A)になる。
  分類 A の想定どおりだが、頻度を下げるなら新しい名前を元の長さ以下にする(`MozcR` など)手もある。

### M4. タグの選び方
- `gh api repos/google/mozc/tags` と `git ls-remote` で確認: 最新は `3.34.6239`、`3.33.6133`、`3.33.6089`、その前は
  4 区切り(`2.32.5994.102` …)、ほかに `2018-02-26`。今の正規表現 `^[0-9]+\.[0-9]+\.[0-9]+$` で正しく `3.34.6239` を選ぶ。
  ただし 4 区切りへ戻ると拾えない。`^[0-9]+(\.[0-9]+){2,3}$` にし、I2(b) の「公開済みより新しいもの」と組み合わせる。
- `sync.yaml:67` の `grep -q "^v${TAG}-readable\."` はタグの `.` を正規表現のまま使っている(`grep -F` か、`.` をエスケープ)。

### M5. heredoc は問題ない
- `release.yaml:83-87` の `cat > notes.md <<EOF` は、YAML のブロック(`run: |`)がすべての行から同じ 10 桁の字下げを
  取り除くので、本文も `EOF` も行頭になる。バグではない(依頼の観点に挙がっていたので確認結果だけ記す)。

### M6. 検証の「Soft」項目は Release の判定に効いていない
- `verify.ps1:104-106,129-131` の Chrome・Notepad・AppContainer は `Soft`(失敗しても exit 1 にならない)。
  ADR D4 は「`verify.ps1` は書き出しの正しさの最終判定」と書くが、自動公開の判定に効くのは Notepad 相当の
  `ui_host` の Hard 項目(`:72-84`)だけ。Chrome を Hard に上げる(今は PASS している)と書く。

### M7. 共有メモリの乗っ取り
- 同じセッションのどのプロセスでも、TIP より先に `Local\mozc-readable-ime-<tid>` を作れば、TIP は既存のものを開いて
  書くので、作った側が偽の値を書ける(`CreateFileMappingW` が `ERROR_ALREADY_EXISTS` を返しても publisher は続行する)。
  同一ユーザーの攻撃者は他の手段もあるので脅威としては小さいが、`ERROR_ALREADY_EXISTS` のとき作成者の SID/所有者を
  確かめるか、ADR に「信頼境界は同一ユーザー」と書いておく。

---

## awase.cc の記述と実装のずれ(観点 5)

### S1(重要). 3 つの文がまだ事実ではない
- 根拠: `docs/index.html:694,696`(英語版 `index.en.html:694,696` も同じ)。
  - 「upstream の新しいリリースに自動で追随して公開します」: PR #1 は未マージ、既定ブランチが `master`(C1)、
    Release は 0 件(`gh release list -R cuzic/mozc-readable` → 出力なし)。
  - 「awase は mozc-readable を使っていれば自動で判定します。設定は不要です。」: awase 側は未実装。ADR 自身が
    「段階 3: 段階 2 のマージまで公開しない」と書いているのに、同じブランチのコミット `84ed1cd7` がこの文を入れている
    (ADR と同じブランチの中で矛盾している)。
  - 「公式の Mozc とは別名でインストールされ、同時に入れても干渉しません」: 共存の CI 検査が無い(I3)。表示名は "Mozc" のまま。
- 今は公開されていない(`git branch -r --contains 84ed1cd7` → 空。develop に入っていない)。
- 推奨: このブランチを develop に入れる前に、「自動で判定」の行を消す(段階 2 で戻す)、「自動で追随」は C1・I5 が直って
  最初の Release が出てから足す、「干渉しません」は共存の CI 検査が通ってから足す。公開物の「発行元: Google LLC」(I3)を
  直すまでは「Google の名称・商標を使うものではありません」と矛盾する。利用者プロファイルが分かれる(公式 Mozc の辞書は
  引き継がれない)ことを書く。

---

## 確認したこと・していないこと

- 確認した: 上に書いたコマンドの出力すべて。readable の最新 CI(run `39093102390`、`0ea8fb8`)の Hard 項目はすべて PASS、
  Chrome・Notepad の Soft も PASS、AppContainer は準備段階で失敗。
- 未確認: 日本語 UI での TIP 表示名、`SysProcs` コンポーネント GUID の共有、同一プロセスでの 2 つの `mozc_tip64.dll`、
  AppContainer 名前空間の開き方、`SYNC_TOKEN` の設定有無、サービス表示名の重複時の SCM の挙動。
