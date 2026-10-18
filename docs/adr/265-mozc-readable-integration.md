---
id: ADR-265
title: |-
  mozc-readable の配布・追随と、awase が共有メモリから IME 状態を読む(自動判定)
status: |-
  提案(2026-10-17)。段階 0(配布の自動化と awase.cc の欄)は着手済み、awase 側の読み取りは未実装
related_adr:
  - "ADR-191"
  - "ADR-261"
---

# ADR-265: mozc-readable の配布・追随と、awase が共有メモリから IME 状態を読む

## 背景

awase は、Win32 の IMM32 や TSF の compartment から IME の開閉・入力モードを読めない窓(`Imm32Unavailable`)で、belief を推測に頼っている。ADR-191 §2 の CI 実測で、外部プロセスが TSF compartment からフォーカス窓の IME 状態を読めないことは確認済み。

[cuzic/mozc-readable](https://github.com/cuzic/mozc-readable) は、Mozc の TIP から IME の開閉・モードを名前付き共有メモリへ書き出す fork である。
- 書き出し先: `Local\mozc-readable-ime-<スレッド ID>`(48 バイト、seqlock、`tip_state_publisher.h` の `TipSharedState`)。
- 通知: `Local\mozc-readable-ime-changed`(manual-reset イベントを SetEvent→ResetEvent で pulse)。
- 公式 Mozc とは別 CLSID・別 UpgradeCode・別製品名で、同時にインストールできる。
- 効くのは mozc 系だけ(MS-IME・配布版 GJI には効かない)。全 IME 向けの近似は ADR-261(字形)が担う。

## 決定

### D1. 配布は mozc-readable 側の GitHub Releases、awase.cc は最新版へのリンクだけ持つ

- MSI は `cuzic/mozc-readable` の Release に `v<upstream の版>-readable.<N>` として置く(SHA256SUMS.txt 付き)。
- `docs/index.html` / `index.en.html` の「インストール」に任意項目を置き、Releases API の最新の MSI へ張る(既存の awase の MSI と同じ方式)。awase の Release には同梱しない(版が独立し、署名・ライセンス表記が別になるため)。
- 署名は無し。SmartScreen の警告を明記する。SignPath Foundation は後付けで検討する。

### D2. upstream への追随は「リリースタグ」を基準に自動化する

- `readable-upstream-sync.yaml`: 毎日 `google/mozc` の `X.Y.Z` タグの最新を確認し、新しければ readable の差分をそのタグへ rebase した `sync/<タグ>` を作り、ビルドと検証(`readable-build.yaml`、インストール・共有メモリ・アンインストールの CI)を起動する。衝突したら Issue を立てて止まる(自動解決しない)。
- `readable-release.yaml`: その検証が成功したら Release を公開し、載せ替え中に readable が動いていなければ readable を進める(動いていたら Issue で止め、readable は上書きしない)。
- master 追随(nightly)は採らない: upstream が壊れている日の影響を受け、公開物の品質が日替わりになるため。

### D3. awase は「設定なし」で自動判定する。設定画面は状態表示とリンクだけ

- インストール済みかの判定: 日本語で有効な TIP の列挙(`EnabledJaTip`)に mozc-readable 専用の CLSID があるか。純関数 `identify_*` に足し、Linux で全数テストする(`ime_kind.rs` の `TipIdentity` に variant を足す形)。
- 窓ごとの状態の判定: 前面窓のスレッド ID の共有メモリが開け、`magic`/`version` が一致し、seqlock の読みが 2 回一致し、`update_tick_ms` が新しいときだけ「読めた」。それ以外は従来経路へ戻る(誤って採ると belief を壊すので、読めないことは無かったことにする)。
- belief への入れ方: 観測(Observe)として取り、`classify_*` の純関数を経て `ObserverReported`/`InputModeObserved` へ流す(`ime-belief-architecture.md`)。**信頼度は実機の測定で決める**(測定前は Medium 以下)。`dispatch_event` を直接呼ぶ近道は作らない。
- 通知イベントは「読み直す合図」にだけ使う(pulse なので取りこぼす前提で、起きるたびに前面窓の共有メモリを読み直す)。
- 設定画面: 検出状態(未インストール / インストール済み・共有メモリ読み取り中 / 読めない窓あり)の表示と、ダウンロードへのリンクのみ。ON/OFF スイッチは置かない(自動判定が外れた報告が出たら強制オプションを足す)。

### D4. upstream との衝突は「層」に分け、機械的な書き換えはスクリプトにする

readable の upstream との差分は、性質の違う 3 層に分かれる(2026-10-17 時点で `readable` と upstream master `0136c357` を比べた実測: 既存ファイル 8 本の変更 + 新規ファイル)。衝突しうるのは第 2 層だけにする。

| 層 | 中身 | 衝突 | 扱い |
| --- | --- | --- | --- |
| identity | 別名化: GUID 3 つ、製品名・表示名、IPC/イベント/サービス/レジストリ名(`const.h`・`system_util.cc`・`tsf_profile.cc`・`installer_oss_64bit.wxs`・`build_installer.py`) | 行が動く・整形されるとき | **スクリプトが毎回生成**(rebase も cherry-pick もしない) |
| publisher | `tip_state_publisher.{h,cc}`(新規)、`BUILD.bazel` の目標と依存、`tip_input_mode_manager.{h,cc}` の書き出しの呼び出し(`On*` 6 メソッドの先頭に `PublishOnExit`) | upstream が `TipInputModeManager` や `BUILD.bazel` を変えたとき | コミットとして持ち、cherry-pick。衝突するのはここだけ |
| ci | `readable/ci/`・`.github/workflows/readable-*.yaml` | 無い(新規のみ) | cherry-pick |

#### identity スクリプト(`readable/identity.py` + `readable/identity.toml`)

- 標準ライブラリだけの Python(`tomllib`)。`apply` と `check` の 2 モード。
- **GUID は 1 つの表から 3 つの書式を作る**: `{AFC9…}` の文字列、コメント、C の構造体 `0xafc94560, 0x63da, …`。表は「元の GUID → 新しい GUID」と、対象ファイルだけ持つ。新しい GUID は固定し、載せ替えで変えない(awase の TIP 同定の定数と一致させるため)。
- **文字列は「元の 1 行 → 新しい 1 行」の表**で、各エントリは「対象ファイルで丸ごと 1 回だけ一致」を条件にする(0 回・2 回以上は失敗)。部分一致で置換しないので、無関係な `Mozc` を巻き込まない。
- 冪等: 新しい行が既にあり元の行が無ければ何もしない。元も新しいも無ければ「アンカーが消えた」と、ファイル・エントリを名指しして失敗する。
- `check`: ①`apply` を当てても差分が出ない、②元の GUID 3 つが `src/` のどこにも残っていない(`system_util.cc` が CLSID を直書きしていた件を、検査で防ぐ)、③新しい GUID が表にあるファイル以外に出ていない、④新しい GUID が `identity.toml` の固定値と一致する。
- 失敗したら、直すのは `identity.toml` の該当エントリだけ(upstream の行に合わせる)。

#### publisher の見落とし検査(文面は衝突しないのに壊れる場合)

`tip_input_mode_manager.cc` の `TipInputModeManager::On*` の定義を一覧し、`identity.toml` と同じ形式の `readable/hooks.toml` にある「書き出しを呼ぶ」「呼ばない(状態を変えない)」の 2 つの一覧と突き合わせる。upstream が `On*` を追加・改名したら、一覧に無いものが出るので失敗し、レビューを強制する。ビルドと `verify.ps1` は書き出しの正しさ(開閉・モードを別プロセスから読んで一致するか)の最終判定として残す。

#### ブランチと載せ替え

- `readable` は、upstream のタグ + identity コミット + publisher コミット + ci コミットの順に積む。各コミットにトレーラー `Readable-Layer: identity|publisher|ci` を付ける(15 コミットの履歴を 3〜4 コミットにまとめ直す。force-push が要るので、mozc-readable の別セッションの作業が終わってから行う)。
- `sync/<タグ>`: タグを checkout → `identity.py apply` → コミット → `Readable-Layer: publisher` と `ci` を古い順に cherry-pick → `identity.py check` と hooks 検査 → ビルドと検証。

#### 衝突の分類と対応

| 分類 | 検知 | 対応 |
| --- | --- | --- |
| A. アンカー消失(identity) | `apply` が失敗 | `identity.toml` の該当エントリを直す小さな PR。判断は不要 |
| B. publisher の cherry-pick 衝突 | cherry-pick が失敗 | 衝突マーカー付きの `sync/<タグ>` を push して Draft PR を作る。解消後は CI が判定する |
| C. 一覧に無い `On*` | hooks 検査が失敗 | 呼ぶか呼ばないかを決めて `hooks.toml` を更新する Issue |
| D. 文面は通ったのにビルド・検証が失敗 | `readable-build.yaml` の失敗 | 失敗ログ付きの Issue |

- Issue は分類と上流の版(タグまたは master の SHA)をキーに 1 件だけ立て、再発したらコメントを足す(毎日の重複を防ぐ)。
- **早期検知**: 毎日 upstream の master に対して、push せずに同じ載せ替え(`apply`・cherry-pick・`check`・hooks 検査)だけを試し、失敗したら上の分類で Issue を立てる。タグは master から切られるので、リリースの数日〜数週間前に直せる。
- **解消の自動化の範囲**: A は人(または私)が直す。B は Draft PR まで。解消後の CI が緑でも、人が 1 回見てから公開する(`verify.ps1` が見るのは書き出しの正しさだけで、新プロセスの最初のキーが落ちる件のような症状は見えないため)。CI 上での AI による自動解消は、衝突の頻度を見てから決める。

## 未確認(段階 1 の前に実測する)

- Chrome 等の TSF ネイティブ、UWP/AppContainer で、`Local\` 名前空間の共有メモリが読めるか(AppContainer は分離される恐れ)。読めない窓は従来経路へ戻る設計だが、割合を測る。
- 公式 Mozc との first-key(新プロセスの最初のキーが落ちる件)の比較。
- 共有メモリの読み取りの信頼度(上の Medium 以下をどこまで上げてよいか)。
- 変換モード以外の状態(必要になれば `TipSharedState` に追記、version を上げる)。

## 段階

0. 配布と追随の自動化、awase.cc の欄(mozc-readable PR #1、本 ADR と同じブランチの docs)。PR #1 の sync は rebase 方式なので、D4 の層方式(identity スクリプト + cherry-pick + 早期検知)へ作り直す。
1. awase に TIP 同定と共有メモリ読み取りの Observe を足す(読むだけ、belief へは入れず journal に出して実機で一致を測る)。
2. 測定に基づき belief へ流す(信頼度を決める)。設定画面に状態表示。
3. awase.cc の記述「自動で判定します」は段階 2 のマージまで公開しない。
