---
id: ADR-265
title: |-
  mozc-readable の配布・追随と、awase が共有メモリから IME 状態を読む(自動判定)
status: |-
  提案(2026-10-17、Opus round1 の指摘を反映済み・再確認待ち)。配布の自動化は PR #1 を作り直す前提、awase 側の読み取りは未実装
related_adr:
  - "ADR-191"
  - "ADR-261"
---

# ADR-265: mozc-readable の配布・追随と、awase が共有メモリから IME 状態を読む

## 背景

awase は、IMM32 や TSF の compartment から IME の開閉・入力モードを読めない窓(`Imm32Unavailable`)で、belief を推測に頼っている。ADR-191 §2 の CI 実測で、外部プロセスが TSF compartment からフォーカス窓の IME 状態を読めないことは確認済み。

[cuzic/mozc-readable](https://github.com/cuzic/mozc-readable) は、Mozc の TIP から IME の開閉・モードを名前付き共有メモリへ書き出す fork である。
- 書き出し先: `Local\mozc-readable-ime-<スレッド ID>`(48 バイト、seqlock、`tip_state_publisher.h` の `TipSharedState`)。通知: `Local\mozc-readable-ime-changed`(pulse)。
- 公式 Mozc とは別 CLSID・別 UpgradeCode・別製品名にしてある。
- 効くのは mozc 系だけ(MS-IME・配布版 GJI には効かない)。全 IME 向けの近似は ADR-261(字形)が担う。

Opus による round1 レビュー(`opus-review-adr265-round1.md`)で、致命 2・重要 15・軽微 7 の指摘を受けた。本版はそれを反映したものである。以下の「R1-xx」は同レビューの項目番号。

## 決定

### D1. 配布は mozc-readable 側の GitHub Releases、awase.cc は最新版へのリンクを持つ

- MSI は `cuzic/mozc-readable` の Release に `v<upstream の版>-readable.<N>` として置く(SHA256SUMS.txt 付き)。awase の Release には同梱しない(版・署名・ライセンス表記が別のため)。
- `docs/index.html` / `index.en.html` の「インストール」に任意項目を置く(既存の awase の MSI と同じ方式で最新の MSI へ張る)。
- 署名は無し(SmartScreen の警告を明記)。SignPath Foundation は後付けで検討する。
- **サイトに書く文は、事実になってから足す**(R1-S1)。現時点で書けるのは「mozc-readable の MSI へのリンク」「署名なし」「利用者プロファイルが公式 Mozc と分かれる(辞書は引き継がれない)」だけ。「自動で追随して公開します」は最初の Release が出て追随が動いてから、「awase が自動で判定します」は awase 側の読み取りがマージされてから、「公式 Mozc と干渉しません」は共存の CI 検査(D4)が通ってから足す。

### D2. upstream への追随は、公開済みの版より新しい「リリースタグ」だけを対象にする

- 対象タグは `^[0-9]+(\.[0-9]+){2,3}$`(4 区切りも拾う)。**直近に公開した Release の upstream 版より新しいもの**だけを選ぶ(R1-I2、古いタグへ「版を下げる」載せ替えをしない)。比較は版の順序(`sort -V`)で行い、`grep` ではタグの `.` を固定文字列として扱う(`grep -F`)。
- 初回は 3.34.6239(最新タグ)を対象にする。このタグは `readable` の土台の upstream master(`0136c357`)より 216 コミット古く、`git apply --check` で `tip_input_mode_manager.{h,cc}` の差分が当たらないことが分かっている(upstream `6b5b8c12` がタグより後にある)。**初回は必ず publisher の手直しが要る**。これを D4 の分類 B の予行演習として扱う。
- **readable 層だけの修正でも再リリースする**(R1-I4): publisher・identity・ci の層に変更が入ったら、現在の upstream 版のまま `-readable.<N+1>` を作る。
- master 追随(nightly)は採らない。

### D3. awase は設定なしで自動判定する。設定画面は状態表示とリンクだけ

**判定の材料**
- 有効化済みか: 日本語で有効な TIP の列挙に mozc-readable 専用の CLSID があるか。これは「使える」ことを示すだけで、**その窓でいま使われている**ことを示さない(R1-C2)。
- **いま使われているか**: publisher に `active`(その TIP が Activate 中は 1、Deactivate で 0)を足し、`TipTextService::Activate/Deactivate` で書いて seq を進める(`TipSharedState` の version を 2 に上げる)。awase は `active==1` のときだけ「読めた」とする。Win+Space で同じスレッドの IME を MS-IME に切り替えても共有メモリに「開・ひらがな」が残るため、`active` が無いと belief を開閉の取り違えという非対称な誤りへ寄せる(ADR-191・INV-45 に反する)。
- `TipIdentity` に variant は足さない(R1-I13)。`TipIdentity` は送信キーの戦略選択(ImmCross → GjiDirect → MsImeDirect の golden)の入力で、足すと観測と無関係にキー選択が変わる。「共有メモリを読める」は独立した能力(`readable_tip_enabled: bool`)として持つ。mozc 系を GJI と同じに扱うかは別 ADR。

**読み取りの規約**(R1-I9・I10・I11)
- 読むたびに Open → Map → 読む → Unmap → Close し、ハンドルを持ち続けない(プロセス終了後の古いブロックを保持しない)。
- seqlock: `seq1` を読み、偶数でなければ捨て、中身を読み、`seq2` を読み、`seq1 == seq2` のときだけ採る(Rust では `read_volatile` と `fence(Acquire)`)。`seq == 0` は未公開として捨てる(publisher 側は `magic` を最後に書くよう直す)。
- `process_id` が対象窓のプロセスの PID と一致しないものは捨てる(スレッド ID の再利用への対策)。
- `update_tick_ms` による新しさの判定はしない(publisher は変化がなければ書かないので、生存の印にならない)。生存は `process_id` 照合と `active` で見る。
- 読む対象のスレッドは、**キーボードフォーカスを持つ窓のスレッド**(`GetGUIThreadInfo` の `hwndFocus`、UWP は `CoreWindow` のスレッド。既存のフォーカス解決を使う)。前面窓のスレッドとは限らない。
- 信頼境界は同一ユーザー(同じセッションの別プロセスが先に同名のマッピングを作れば値を偽れる)。これは脅威として許容し、ADR に明記する(R1-M7)。

**belief への入れ方**(R1-I14)
- Observe → `classify_*` 純関数 → `ObserverReported`/`InputModeObserved`(ime-belief-architecture)。新しい `ObservationSource` を足すので、`architecture_guard.rs` の構築箇所の件数と `observation_source_guard` の対象を更新する。
- 信頼度は **Low(記録のみ)から始める**。Medium は belief に採られる側なので安全側ではない。段階 1 の journal で一致率が基準(件数・率は段階 1 の測定後にここへ書く)を超えたら Medium に上げる。
- 取るフィールドは `tsf_*`(compartment 相当)を既定にする。`effective_*`(入力スコープの上書き後)と食い違うとき(パスワード欄など)の扱いは段階 1 で測って決める。
- 通知イベントは「読み直す合図」にだけ使う(pulse なので取りこぼす前提)。

**AppContainer**(R1-I12): AppContainer 内のプロセスの `Local\` 名前空間は `AppContainerNamedObjects\<SID>\` へ分かれ、開けない見込みが高い(CI の AppContainer 検査は準備段階で失敗していて結論が無い)。段階 1 の前に、`OpenFileMappingW` で届くか・`NtOpenSection` が要るかを CI で測る。開けなければ「UWP は従来経路」と明記する。

**設定画面**: 検出状態(未インストール / 有効 / この窓で読めている)の表示と、ダウンロードへのリンクのみ。スイッチは置かない(自動判定が外れた報告が出たら強制オプションを足す)。

### D4. 履歴の持ち方と upstream との衝突

#### D4-1. 書き換えない「材料」ブランチ + 使い捨ての `build/<タグ>`(R1-I1、R1-C1)

- 既定ブランチ `readable` には **upstream のソースを置かない**。持つのは材料だけにする: `readable/identity.py`・`identity.toml`・`hooks.toml`、`readable/patches/publisher/*.patch`(`git format-patch` の series)、`readable/ci/`、`.github/workflows/`。`readable` は通常の PR と fast-forward だけで進む。force-push・`sync-base`・「載せ替え中に動いたら止める」の仕組みは不要になる。
- 各リリースは `build/<upstream タグ>-<N>` を使い捨てで生成する: タグを checkout → 材料ブランチの `identity.py apply`(M2: スクリプトは材料ブランチから動かす)→ `git am -3 patches/publisher/*` → ci を上書きコピー → ビルド → 検証 → Release のタグで固定する。`build/*` は Release のタグが指すので到達性は保たれる。
- 既定ブランチが `readable` になるので `schedule`・`workflow_run` が動く(C1)。fork の既定ブランチを `master` から `readable` へ変える作業は ADR の決定として扱う(`master` は upstream のミラーとして残すか、消すか。残す場合は fork の compare の既定が `readable` になる点だけが影響)。
- **現在の `readable`(C++ のソース入り、別セッションが作業中)からの移行**: 別セッションの作業が終わってから、`git diff` を identity / publisher / ci の 3 つに分けて材料ブランチを作る。現在の `readable` は `legacy/readable-src` として残す。

#### D4-2. 3 層

| 層 | 中身 | 扱い |
| --- | --- | --- |
| identity | 別名化。GUID、製品名・表示名・発行元、IPC/イベント/サービス/レジストリ名 | `identity.py` が毎回生成。衝突しない |
| publisher | `tip_state_publisher.{h,cc}`(新規)、`BUILD.bazel`、`tip_input_mode_manager`、`TipTextService` の Activate/Deactivate の呼び出し | patch series。`git am -3`。衝突しうるのはここだけ |
| ci | `readable/ci/` | 生成時に上書きコピー。衝突しない |

#### D4-3. identity スクリプト

- 標準ライブラリだけの Python(`tomllib`)。`apply` と `check`。
- GUID は 1 つの表から複数の書式を作る(R1-M1): `{AFC9…}` の文字列、コメント、C の構造体 `0xafc94560, 0x63da, …`、波括弧なしの `AFC9…`(`build_installer.py` の UpgradeCode)、小文字。新しい GUID は固定し、載せ替えで変えない(awase 側の CLSID の定数と一致させる)。
- 文字列は「元の 1 行 → 新しい 1 行」の表で、各エントリは対象ファイルで丸ごと 1 回だけ一致を条件にする(0 回・2 回以上は失敗)。冪等。元も新しいも無ければ、ファイル・エントリを名指しして失敗する(分類 A)。
- **置換表に入れるもの**(R1-I3): 既存の `const.h`・`system_util.cc`・`tsf_profile.cc`・`installer_oss_64bit.wxs`・`build_installer.py` に加え、`tip_resource.rc` の表示名(`IDS_IME_DISPLAYNAME`・`IDS_TEXTSERVICE_DISPLAYNAME_SYNONYM`、日本語 UI で言語の切り替えに見える名前)、`wxs` の `Manufacturer`(2 か所)・`Description`・`Feature Title`・エラー文言、`mozc_cache_service.rc`・`custom_action.rc`、`kSearchCandidateProviderName`、`kConfigurationDisplayname`。`wxs` の `SysProcs` コンポーネントが公式と同じキーパスを持ち、コンポーネント GUID が公式と同じになりうる(未確認)ので、製品ごとに分ける方法も検討する。
- 置換後の行が長くなって upstream の整形と合わなくなる(R1-M3): 新しい名前は元の長さ以下にできるものは短くする。
- `check`: ①`apply` を当てても差分が出ない、②元の GUID が `src/` のどこにも残っていない、③新しい GUID が `src/` の表にあるファイル以外に出ていない(`readable/ci/` は対象外)、④新しい GUID が `identity.toml` の固定値と一致する、⑤`src/win32` の `.rc`/`.wxs` に `"Mozc"` という完全一致の文字列が残っていない(許可リスト付き)。

#### D4-4. publisher の見落とし検査(R1-I15、R1-I2c)

- 検査の主軸は「`mozc_state_`・`tsf_state_` に代入する関数の集合 ⊆ 書き出しを呼ぶ関数の集合」(テキスト走査)。名前の一覧(`hooks.toml`)は補助。`TipTextService` の Activate/Deactivate も対象に含める。
- `static_cast<int>(conversion_mode)` の型の入れ替わりを黙って通さないよう、`static_assert` で値(0〜5)を固定し、型や値の変更をビルド失敗にする。

#### D4-5. 衝突の分類と対応

| 分類 | 検知 | 対応 |
| --- | --- | --- |
| A. アンカー消失(identity) | `apply` が失敗 | `identity.toml` を直す小さな PR |
| B. publisher の `git am` 衝突 | `git am -3` が失敗 | 衝突解消用のブランチと Draft PR を作る。patch を直す普通の PR になる |
| C. 書き出しを呼ばない状態の変更 | 見落とし検査が失敗 | 呼ぶか呼ばないかを決める Issue |
| D. 文面は通ったのにビルド・検証が失敗 | `readable-build` の失敗 | 失敗ログ付きの Issue |

- Issue は分類と upstream の版をキーに 1 件だけ立て、再発したらコメントを足す(`gh issue list --search "in:title <キー>" --state open` を見てから)。分類 D は release 側で `conclusion != success` かつ `build/*` のときに Issue を立てる。`build/<タグ>` が N 日以上残っていたら Issue を立てる(R1-I5、黙って止まらない)。
- 毎日 upstream の master に対して、push せずに同じ生成(apply・am・check・見落とし検査)だけを試す(早期検知)。
- 解消後は CI が緑でも、人が 1 回見てから公開する。CI 上での AI による自動解消は、衝突の頻度を見てから決める。

#### D4-6. ワークフローの安全(R1-I6・I7・I8)

- `${{ }}` を shell に直接展開せず、`env:` 経由で渡し、`[[ "$TAG" =~ ^[0-9]+(\.[0-9]+){2,3}$ ]]` で検証してから使う。`GH_TOKEN` は必要なステップにだけ置く。リポジトリ内のスクリプトを動かすジョブでは `persist-credentials: false` にし、push の直前だけ認証を付ける。
- `workflow_run` の公開ジョブは、`head_repository.full_name == github.repository` かつ `event in (push, workflow_dispatch)` を条件にする。
- `SYNC_TOKEN` は fine-grained PAT を `cuzic/mozc-readable` 1 つに限定し(Contents・Issues・Actions の read/write、有効期限付き)、材料ブランチ構成では生成ブランチにワークフローを含めないので Workflows 権限は付けない。設定の有無は未確認。

#### D4-7. 検証の強化(R1-I3、R1-M6)

- 公式 Mozc の MSI を先に入れ、readable を入れ、両方を切り替えて打鍵し、片方をアンインストールしてももう片方が動くことを CI で検査する(共存の主張の根拠。いまの `verify.ps1` は公式 Mozc が入っていない前提の検査しか無い)。
- Chrome の検査(今は Soft で PASS している)を Hard に上げる。自動公開の判定に効くのは Hard だけ。

## 未確認(段階 1 の前に実測する)

- AppContainer の名前空間(上記)、日本語 UI での TIP の表示名、`SysProcs` コンポーネント GUID の共有、同じプロセスでの 2 つの `mozc_tip64.dll`、サービス表示名の重複時の SCM の挙動。
- 新プロセスの最初のキーが落ちる件の、公式 Mozc との比較。
- `tsf_*` と `effective_*` の食い違い(パスワード欄など)。
- `SYNC_TOKEN` の設定有無。

## 段階

0. **配布と追随の作り直し**(mozc-readable 側): D4-1 の材料ブランチ構成へ移す(PR #1 は rebase 方式なので閉じて作り直す)。別セッションの作業が終わってから。初回は 3.34.6239 向けに publisher を手で直す。
1. mozc-readable に `active` を足す(version 2、`magic` を最後に書く)。awase に読み取りの Observe を足し、belief へは入れず journal に出して実機で一致を測る。AppContainer の測定も行う。
2. 測定に基づき belief へ流す(Low から始め、基準を満たしたら Medium)。設定画面に状態表示。
3. サイトの記述は D1 のとおり、事実になった文から足す。
