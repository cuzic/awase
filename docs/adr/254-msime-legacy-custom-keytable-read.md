---
id: ADR-254
title: |-
  MS-IME 互換モード(旧UI)の Custom 表の扱い——第一段は「keystyle が既定でないとき MSIME_NATIVE の予測を止める」、表を読む第二段は検証が通ってから
summary: |-
  互換モードで旧UIのキー表をカスタムしている利用者は、v2 では `MSIME_NATIVE`(新UI前提の既定表)で予測されて belief がずれる。
  2026-10-10 の CI スパイク(ブランチ ci/msime-legacy-keytable-spike、windows-latest 26100、各セル n=1。再検証は n=3〜5、実機〈dragonflyg4〉は n=3)で、旧UIの表について次が観測できた。
  (a) 表が効くのは keystyle=Custom のときだけで、名前付きスタイル(NATURAL 等)の表をレジストリで書き換えても、閉じた状態の実験では無視された。
  (b) IME が閉じている間のキーは Custom\S4key が決め、key だけを書いても閉じた状態では無反応。ADR-197 の実験は閉じた状態で key だけを書いたため無反応に見えた。
  (c) key の列は旧UIの列見出し(No Input・Only Input・Converted・Showing・Changing・Char Input)と1対1で、1列目は「開いている・入力なし」。
  最初の開いた状態の掃引は、ハーネスの人工物で「入力中」になっていたため無効(Opus レビュー B1)。ハーネスを直した再検証で 1 列目の対応(実測5)・ND の陽性対照(実測6)などが取れた。
  本 ADR は2段に分ける。第一段: 互換 ON かつ keystyle=Custom、または keystyle が名前付き(ATOK 等)のときは MSIME_NATIVE の予測を全キーで止め(互換 OFF の Custom は新エンジンが読まないので予測する)、スタンプと指紋に keystyle を足す(新しい意味づけを持ち込まない)。
  第二段: 表を読んで閉・開の効果を予測に使う。ただし第二段は、UI が実際に書くコード(V2)・そのコードを Custom の 1 列目に置いた試行・エンジンの同定(V3c)・実機での V1 が通ってから。
status: |-
  第一段を実装(2026-10-10、PR #585 `feat/adr254-stage1-legacy-keystyle-stop`)。Opus レビュー r1〜r3(Blocker 3・Must 12・Should 9)と実装 PR のレビューを反映済み。
  CI スパイク(ブランチ `ci/msime-legacy-keytable-spike`、未マージ)と実機(dragonflyg4、互換 ON/OFF とも)で測定。
  第二段(Custom の表を読んで予測に使う)は未着手(着手条件: UI が書くコードの確認〈V2〉、報告者の実環境での確認〈V1〉)。
related_adr:
  - "ADR-197"
  - "ADR-191"
  - "ADR-196"
  - "ADR-199"
  - "ADR-195"
---

# ADR-254: 旧UIの Custom 表の扱い(第一段=予測を止める、第二段=表を読む)

## 背景

MS-IME「以前のバージョンの Microsoft IME を使う」互換モードの利用者が旧UIでキーをカスタムしていると、v2 の awase は期待どおり動かないという報告が散発している。v1 では動いていた。
v2 は IME 設定から役割を逆算して打鍵時に IME の開閉を予測する(ADR-191・199)。互換モードでは役割判定は受動になる(`msime_native_key_role`、`key_effect_predictor.rs:772,781`)が、
**予測は互換モードでも `MSIME_NATIVE` で行われている**(`predict_with_override`、`:841-885` は新UIの DWORD 由来の `henkan_reassigned`/`muhenkan_reassigned` でしか打ち切らない)。
つまり旧UIのカスタムと食い違う予測が、読めない窓(Chrome・Windows Terminal)でそのまま belief に入る(Opus r1 M4)。

## 実測(CI スパイク、各セル n=1、windows-latest 1 環境)

詳細と生データの場所: `docs/tasks/msime-legacy-keytable-findings-2026-10-10.md`。

信頼してよいもの(閉じた状態の実験):
1. `keystyle=Custom` のとき、閉じた状態で押したキーは `StyleList\Custom\S4key` が決める。`key` だけを書いても閉じた状態では無反応(c7、`S4key` の変換行=`00` で変換が開かなくなる/無変換行=`87`・`CE` で開く)。
2. 名前付きスタイルの `S4key` を書き換えても閉じた状態では無反応(s6)。名前付きスタイルの切り替えは `IMJPUEXC.EXE setkeytemplate <Microsoft_IME|IME_Standard|ATOK|VJE|WX>`。
   閉じた状態の変換キーの実効果は、各テンプレートの `S4key` の 変換 行の有無と一致した(Microsoft_IME・IME_Standard・WX は開く、ATOK・VJE は開かない)。
3. `SerialNo` の更新・ctfmon の再起動は効果と無関係(V2/V3 の旧スパイク)。UI の Apply は `keystyle`・`IMEUserName`・`option2`・`SerialNo` などを書く。
4. `key` の列は旧UIの列見出しと 1 対 1(`u5/advanced.png`、`spike1` の `SPACE=81 06 2D 30 07 06` ↔ SpaceWidth・ConvAll・Extended・DetThenNext・ConvPhrase)。
   **1 列目=開いている・入力なし、2 列目以降=入力中・変換済み・候補表示中など**。1 列目と 2〜6 列目では使われるコードの空間が違う(1 列目に 0x80 未満は無い)。

再検証(2026-10-10、同スパイクの第2ラウンド。`--no-probe` ハーネス、各ステップの「前」の状態を記録、ND=1 は n=5・ND=0 は n=3、全試行で結果が一致):
5. **列の対応(V0b)**: `key` の 1 列目は「開・入力なし」にだけ効く。1 列目=`97`(他 `00`)で、開・入力なしの無変換は conv→0x1B、入力中(`か`)は変化なし。1 列目=`00`(他 `CD`)では、開・入力なしも入力中(`か`)も変化なし。2〜6 列目の 1 列だけに `CD` を置いても(V0c)、かな未確定(`か`)・変換済み(`か`+Space)のどちらにも効かない。一方、dragonflyg4 型(1 列目=`CE`・他=`CD`)の行は入力中の状態に効く(実測12)が、これは単独の列ではなく行の組み合わせで決まる。2〜6 列目と状態の対応は未確定。第二段では読まない。
6. **ND の陽性対照(V0)**: 1 列目=`D5` の開・入力なしの無変換は、ND=1 で IME が閉じる(conv→0x10)、ND=0 で開いたまま conv→0x10。ND は効果を変える。一方、NATURAL の 半角/全角(F3・19)の開・入力なしは ND=1/0 とも閉じる(コードによって ND に依る/依らないが違う)。UI のチェックボックス(id=5004)は実際に `MSIME\NoDirectInputMode` を 1→0 に書く(`option1`/`option2` ではない)。
7. **互換モードのフラグを変えても `keystyle` の効果は変わらなかった(V3・V3b。windows-latest 1 環境、エンジンの同定なし)**: `Custom\S4key` の無変換=`87` は、`NoTsf3Override2`・`DisableNewIME` が (1,1)・(1,不在)・(不在,1)・(不在,不在)・(0,0) のどれでも閉じた状態で IME を開く。名前付き ATOK の閉・変換が開かないこと、`Custom\S4key` の変換=`00` で開かなくなることも、フラグ不在・(0,0) で同じ。ただし windows-latest ではフラグに関係なく旧エンジンが動いているだけ、という説明(r3 M3-1 の (ii))を、まだ否定できていない。新UIの割り当てを入れた陽性対照(`v3c`)で確かめる。
8. **`keystyle` の値が無いとき**は NATURAL と同じ(閉・変換が開く)。互換 ON でもフラグ不在でも。
9. **名前付きスタイルの開・入力なし(V4)**: Microsoft_IME・IME_Standard・WX は無変換で conv→0x1B で同じ。名前付き NATURAL の `key` を `無変換=D5…` に書き換えても結果は変わらない(内蔵表が使われ、レジストリの `key` は無視される)。ATOK・VJE は変換で開けず(閉・変換が開かない)、`1C` で開・入力なしを作れなかったため開側は未測定。
10. **dragonflyg4 型の構成(`key` の無変換行=`CE CD CD CD CD CD`、`S4key` は NATURAL の写し)**: 閉の無変換は何もしない(NATURAL の `S4key` に無変換行が無いため)。開・入力なしの無変換も何もしない(1 列目 `CE`)。入力中(ローマ字の途中・かな未確定の両方、実測12)では、ND=1 で半角英数、ND=0 で確定して閉じる。これは「実機で無変換が IME ON にならない」観測と閉・開・入力なしについては整合する。ただし dragonflyg4 の実際の `Custom\S4key` は読んでいない(ADR-197 は `key` の 2184 バイトだけ)ので、実機の構成の代表とは言えない(V1 で確かめる)。`MSIME_NATIVE` の既定は開・入力なしの無変換を conv→0x1B と予測するので、この構成では予測が外れる(決定1の動機)。

第3ラウンド(`v3c`・`v0d`・`v0e`、ND=1 は n=5、ND=0 は n=3、全試行で結果が一致。Probe の「前」の取り違えとログの上書きを直した版):
11. **エンジンの陽性対照は取れなかった(v3c)**: 新UIの割り当て(`IsKeyAssignmentEnabled=1`・`KeyAssignmentMuhenkan=0`=IME-オン、ADR-199 T12 で実機確認済みの値)を入れても、`NoTsf3Override2`・`DisableNewIME` の 5 通り(1,1)(1,不在)(不在,1)(不在,不在)(0,0)のどれでも閉・無変換は開かなかった(各 n=5、対照の割り当て無しも変化なし)。windows-latest では、フラグに関係なく旧エンジンが動いている(または新UIの割り当てを読まない)と読むのが自然で、**windows-latest で測った結果が新エンジンにも当てはまるかは分からない**。同梱表 `MSIME_NATIVE` を CI で測ったときも旧エンジンだった可能性があり、ADR-191 の校正の前提に関わるので、別に確かめる(実機〈Windows 11 クライアント〉での V1 の依頼文の 3)。読み込まれた DLL の比較は ctfmon から取れなかった。
12. **dragonflyg4 型の行(1 列目=`CE`・他=`CD`)は、組み合わせで効く(v0d)**: ローマ字の途中(`ｋ`)でも、かな未確定(`か`)でも、ND=1 で conv→0x10(半角英数、未確定は残る)、ND=0 で未確定を確定して閉じる。1 列目=`00`・他=`CD`、1 列目=`CE`・他=`00`、2〜6 列目の 1 列だけ `CD`、のどれでも効かない。つまり実測5・実測10の「ローマ字の途中だけ」は、`ｋｋｋ`(3 回注入)の人工物による誤読で、状態の違いではなく**行の組み合わせ(`CE` + `CD`)で決まる**。開・入力なしでは、この行は何もしない。
13. **1 列目だけにコードを置いた開・入力なしの効果(v0e、ND=1/0 で同じ)**: `CD`・`B3`=IME が閉じる、`A4`=閉じて conv→0x10、`C9`=conv→0x1B(全角カタカナ)、`97`=conv→0x1B、`CA`・`CE`・`00`=変化なし。`D5` だけは ND で向きが変わる(実測6)。

14. **設定アプリを読んでも、CI の互換モードは判別できなかった(`msime-settings-engine`、`ms-settings:regionlanguage-jpnime` を UI Automation で読んだ)**: ランナーは Windows Server 2025 Datacenter(26100)。Microsoft IME の「General」ページには互換性のチェックボックス(「以前のバージョンの Microsoft IME を使う」)が無く、「Key & touch customization」ページには「キーの割り当て」セクションが無い(ADR-199 の記録では、このセクションは互換モード ON のとき非表示)。**`NoTsf3Override2`・`DisableNewIME` の 5 通り(不在・(1,不在)・(1,1)・(0,0)・(不在,1))で、どちらのページも完全に同一**(ログのハッシュが一致)。また、実測11のとおり新UIの割り当てをレジストリに直接書いても効かない(ADR-199 T12: 設定アプリの UI を操作する必要がある)ので、陽性対照としては設計が誤りだった。結論: **windows-latest(Windows Server)では、フラグを書いても互換モードが切り替わったと確認できず、CI の測定は「この環境の MS-IME」(旧エンジンの可能性が高い)の結果と呼ぶべき**。これは、互換モード ON の利用者(調査対象)にとっては都合のよい前提(旧エンジンの表を測れている)だが、新エンジンの利用者や、`MSIME_NATIVE` の CI 校正(ADR-191)には当てはまるかが分からない。互換 ON/OFF の切り替えと新エンジンの挙動の確認は、実機(Windows 11 クライアント)でしかできない。

15. **実機(dragonflyg4、Windows 11 Pro 22631)の測定(clipwire 経由、harness `--no-probe`、各セル n=3、全試行一致)**: 現状は `NoTsf3Override2=0`(互換 OFF)・`keystyle=NATURAL`・新UIのマスタースイッチ OFF で、`StyleList\Custom` には ADR-148 の実験の表(無変換・変換=`CE CD CD CD CD CD`)が残っている。互換 OFF(新エンジン)での結果:
    - `keystyle=NATURAL`: 閉・変換→開く(conv 0x09)、閉・無変換→変化なし、開・入力なしの無変換→conv 0x0B(全角カタカナ)、かな未確定の無変換→カタカナ変換。CI と同じ。
    - `keystyle=ATOK`(レジストリ書換のみ): **新エンジンも `keystyle` を読む**が、内蔵の表は旧エンジンと違う。閉・変換→開く(CI の旧エンジンでは開かない)、閉・無変換→開く(NATURAL では変化なし)、開・入力なしの無変換→**閉じる**、かな未確定の無変換→conv 0x00。
    - `keystyle=Custom`: **NATURAL と完全に同一**。新エンジンは `Custom` の表を読まない(残っている `CE CD…` は無影響)。
    - したがって、**名前付きスタイル(ATOK 等)の結果は旧エンジンと新エンジンで違い、CI(旧エンジンの疑い)の結果は新エンジンに転用できない**。`keystyle` が既定でない新エンジンの利用者も `MSIME_NATIVE` の予測が外れるので、決定1が「互換フラグを条件にしない」のは正しい。
16. **互換 ON の実機測定(dragonflyg4、設定アプリの互換トグルを UI Automation で On にし、確認ダイアログの「OK」を押して確定。各セル n=3、全試行一致)**:
    - 互換トグルは `DialogToggle` で、押すと確認ダイアログ(OK=`PrimaryButton`・キャンセル=`SecondaryButton`)が出る。**OK を押すと `NoTsf3Override2=1` が書かれる**(OK を押さずに閉じると書かれず、挙動も変わらない)。レジストリ値を直接書くだけでは IME が応答しなかった(テスト不能)。つまり awase が読む `NoTsf3Override2` は、UI の互換トグルの実状態を正しく表す。
    - 互換 ON の `NATURAL`: 閉・変換→開く(conv 0x19)、閉・無変換→変化なし、開・入力なしの無変換→conv 0x1B、かな未確定の無変換→カタカナ変換。**CI の旧エンジンと同じ**(新エンジンの conv は 0x09/0x0B)。
    - 互換 ON の `Custom`(ADR-148 の実験で残っていた表、無変換・変換=`CE CD CD CD CD CD`): 閉・変換→開く、**閉・無変換→開く**(NATURAL では何もしない)、開・入力なしの無変換→変化なし、かな未確定の無変換→conv 0x10(半角英数、未確定は残る)。**CI の dragonflyg4 型の測定(実測12・13)と完全に一致**。互換 OFF では `Custom` は無視される(実測15)ので、`Custom` の表は互換 ON のときだけ効く。
    - 互換 ON の `ATOK`: IME が一度も開かない(閉・変換でも)。CI の旧エンジンの ATOK と同じ。互換 OFF の `ATOK`(新エンジンの内蔵表)とは別の挙動。
    - 結論: **windows-latest の CI は、実機の互換 ON(旧エンジン)と同じ挙動を再現している**(実測7〜14のうち `Custom`/名前付きスタイルに関するものは、互換 ON の実機で裏付けられた)。互換フラグは条件ではなく、**互換 ON のときだけ旧UIの表が効く**。
17. **互換 OFF + `keystyle=Custom` は `NATURAL` と同じ(実測15)**: 決定1の止める条件は、`NoTsf3Override2=1`(互換 ON)かつ `keystyle∉{NATURAL, 不在}`、または `keystyle` が名前付き(ATOK・VJE・WX・IME_Standard)のときに絞れる。互換 OFF の `Custom` は `NATURAL` 扱い(予測する)でよい。

信頼してはいけないもの(Opus r1 B1・B3・M1・M3):
- 「開いた状態の効果表」「D5 だけが閉じる」「CE は開いた状態で変化なし」は無効。開いた状態の掃引は、未確定の `ｋ` が残った「入力中」で取られていた(ハーネスの ESC が未確定を消せていない)。また 6 列すべてに同じコードを書いたため、1 列目用のコードを入力中の列に置く、実際の設定に無い組み合わせだった。
- 閉じた状態の 57 コードの効果表は、実際の `S4key` に現れるコード(`87`・`CE`)に比べて過大。大半のコードで「開く」になったのは、「`S4key` に行がある=IME が閉じている間にそのキーを横取りし、まず IME を開いて機能を実行する」という別の機序で説明がつく。コード固有の意味とは言えない。
- (解決済み)「直接入力モードを使用しない」に依らない、は陽性対照が無く言えなかった。実測6で、ND は効果を変えること(`D5`)と、チェックボックスの実体が `NoDirectInputMode` であることが分かった。
- 名前付きスタイルが表を無視するかは、閉じた状態でしか確かめていない(`spike4` は `key` を書いて閉じた状態で押したため、`key` が引かれない状態では当然無反応)。

## ADR-197 の訂正

ADR-197 に次を追記する(r1 観点3):
1. `key` の 1 列目は「開いている・入力なし」(旧UIの列見出しとレジストリの `SPACE` 行が根拠)。閉じた状態の割り当ては `S4key`。ADR-197 の「1列目 CE=直接入力→ON方向」は列の読み違い。
2. ADR-197 の CI 7 パターンと dragonflyg4 の物理キー確認は、いずれも閉じた状態で無変換を押している。閉じた状態で `key` を書いても効かないのは今回の実験と整合する。`key` の 1 列目(開・入力なし)の効果は今回の再検証で測った(実測5)。dragonflyg4 型の `無変換=CE CD CD CD CD CD` は、閉の `S4key` が NATURAL の写しなら、閉・開・入力なしのどちらでも無変換は何もしない(実測10。dragonflyg4 の実際の `S4key` は未確認)。
3. ADR-197 の「NATURAL `無変換=97` → IME OFF」も誤り。97 は開・入力なしでのかな切替(同梱表 `MSIME_NATIVE` の C19→C1B と一致)。
4. 「表の内容は挙動と相関しない」の撤回は、V0・V1 が通ってから行う。今言えるのは「閉じた状態は `S4key` と相関する」まで。

## 決定(提案)

### 決定1(第一段): 互換モードでカスタム表の可能性があるときは、`MSIME_NATIVE` の予測を全キーで止める

条件(実機の測定15〜17で絞った。実機〈互換 ON・OFF とも〉の裏付けあり): MS-IME 本体で、次のいずれかのとき。
- `keystyle` が名前付き(ATOK・VJE・WX・IME_Standard)のとき(互換 ON/OFF とも。互換 OFF の新エンジンも ATOK を読むが、内蔵表は旧エンジンと違う〈実測15・16〉ので `MSIME_NATIVE` の予測が外れる)。
- 互換 ON(`NoTsf3Override2=1`)かつ `keystyle=Custom`(旧UIの表が効く)。
- `keystyle` が未知の名前、または読み取り失敗。
予測する(従来どおり)のは次のとき: `keystyle` の値が無い(NATURAL と同じ)、`keystyle=NATURAL`、互換 OFF かつ `keystyle=Custom`(新エンジンは `Custom` の表を読まず `NATURAL` と同一、実測15)。
互換フラグの検出は、UI の互換トグルが `NoTsf3Override2` を書くこと(実測16)から、既存の `read_legacy_compat_mode_enabled()` を使う。
対象キーは列挙せず、**全キー**(r2 M2-3)。`MSIME_NATIVE` の `TableKey` には無変換・変換・半角/全角・英数・ひらがな・カタカナのほか、Kanji・ImeOn・ImeOff・Space・Enter・Esc・Bs があり、Custom はこれらも書き換えられる。列挙すると ADR-197 の「CE だけを見た」誤りと同じ漏れを作る。
置き場所は `predict_with_override`(`key_effect_predictor.rs:844`)の中で、**学習表の参照(`override_table`)の後、`predict(self.preset, …)` の前**。前に置くと ADR-196 の学習が戻せなくなる。決定1の「学習で戻せる」はこの置き場所が前提。
下流: `kp_predict_key_effect` は予測が `None` なら何もしない(確認済み)。役割(`thumb_role_open_actions`)は互換モードで元から受動なので、予測が消えても下流が「予測あり」を前提に動く箇所は見つからなかった(grep 済み)。

止めたときは、止めた理由(`keystyle` の値と、止めたこと)を journal と不具合報告に 1 行残す(この副作用を受けた人の報告を、ほかの原因と区別するため。`bug_report.rs` の既存の互換モード・`keystyle` の欄に足す)。
位置づけ: これは「誤予測を止める安全側の変更」であり、報告の症状の修正とは称さない(r2 M2-4)。止めると、読めない窓で既定のまま使っている変換(閉→開)の予測も消える。報告が直るかは V1 で確かめる。
`key_effect_predictor.rs` は fix-requires-evidence の IME belief ファミリー(`KeyEffectPredicted` が belief を直接動かす)なので、(a) 回帰テスト(Linux で走る単体テスト)を付ける。最低限:
  - 互換 ON・`keystyle=Custom` → `None`
  - 互換 OFF・`keystyle=Custom` → `MSIME_NATIVE` と同じ(新エンジンは Custom を読まない)
  - `keystyle=ATOK`(互換 ON・OFF とも)→ `None`
  - `keystyle` 不在・`NATURAL` → `MSIME_NATIVE` と同じ
  - 互換 ON・`keystyle=Custom`・学習表あり → 学習表

### 決定2(第一段): スタンプと指紋

- 版スタンプ(`native_assignment_stamp`、`msime_key_assignment.rs`)は、既存の値に **`keystyle` の種別・名前、`Custom` のときは `key` と `S4key` の中身のハッシュ**を混ぜる(実装: 2 つ目の要素を不透明なハッシュにした。最終書き込み時刻の API は使わない)。第二段で使うなら`NoDirectInputMode`の値も足す。`MSIME` キーの最終書き込み時刻は使わない(`keystyle` 以外に IME 自身が書く値が多く、UI の Apply 1 回で 10 個近く変わるため、読み直しが頻発する、r3 S3-5)。表のハッシュ(指紋用)は、`key` と `S0key`〜`SFkey` の固定の 17 名を読んで作る(実装。観測した `S*key` の集合はこの範囲に収まる。`RegEnumValueW` による全列挙にはしなかった)。版スタンプは `key` と `S4key` の 2 つだけを見る(打鍵の経路で 2 秒ごとに呼ばれるため)ので、**ほかの `S*key` だけを編集した場合、awase を再起動するまで指紋は古いまま**になる(第一段は予測の可否に影響しないので許容。第二段で見直す)。`KeymapCache::get` は 2 秒ごとに `stamp()` を呼び、呼び出し元は打鍵の経路(`kp_predict_key_effect`)なので、表のバイト列を 2 秒ごとに最大 15 個読むハッシュは避ける(r2 S2-2)。バイト列のハッシュは `load` と指紋の側で作る。
- 指紋(`msime_native_keymap_fingerprint`、`fingerprint.rs:110-120`)は、`keystyle` が NATURAL でも不在でもないときに限り `(keystyle, Custom なら key と全 S*key のハッシュ)` を足す。それ以外(NATURAL・不在=大多数の利用者)の指紋は不変(全員の学習表を一斉に失効させない)。ND の値(`NoDirectInputMode`)は、効果を変える(実測6)ので、第二段で指紋に足すかを決める。

**既存の学習表への影響(実装 PR のレビュー B-doc2)**: 指紋が変わるのは、予測を止める構成(互換 ON の `Custom`・名前付き・未知)のときだけ。**この構成で、すでに学習を済ませていた利用者の学習表は、指紋の不一致(`Staleness::FingerprintMismatch`)で失効し、再学習が要る**(止める判断自体は妥当なのでコードは変えない。該当者は少ないと見ているが、MS-IME 本体で学習が完走するかが未確認なので数は不明)。救う方法は後続: 止める構成では旧指紋の学習表も受け入れる。`keystyle` が不在・NATURAL・互換 OFF の `Custom` の大多数の利用者の指紋は変わらない(golden テストで固定)。

### 決定3(第二段、V0・V0′・V2・V3 が通ってから): 表を読んで予測に使う

- 対象: MS-IME 本体・互換 ON(`NoTsf3Override2=1`)・`keystyle=Custom`(互換 OFF では新エンジンが `Custom` を読まない、実測15・16)。名前付きスタイルの定数表は、V4 で内蔵表の効果が必要と分かったときだけ作る(実測9: 名前付きは内蔵表で、`key` の書換は効かない)。
- 閉じた状態(`Custom\S4key` の対象キーの行、r2 M2-1): 1 列目が `87` か `CE` → 開く(c7 で ND=0/1 の 2 回確認)。行が無い・1 列目が `00`/`80`/`FF` → 変化なし。**それ以外のコードは `不明`**(A2・A4・D0・D5 は閉じたまま conv だけ変わり、B3 は変化なし、94 はダイアログが出た。テキストを挿入するコード 81〜83・88 と未確定を消すコード B0・B1・F0・F1 も `不明`)。`S4key` 自体が読めなければ `不明`。
- 開いた状態: `key` の 1 列目(開・入力なし、実測5)について、効果が確かめられたコードだけを予測に使う。確かめたのは(実測13)`97`・`C9`(conv→0x1B)、`CD`・`B3`(閉じる)、`A4`(閉じて conv→0x10)、`00`・`CA`・`CE`(変化なし)。**`D5` は第二段の最初の範囲から外す**(ND による向きの違いが説明できず、ND の読み取りの時期とずれると閉/開の向きを誤る。全テンプレートで `D5` が現れるのは ATOK の `変換` の 1 列目だけ。戻す条件は V5〈ND を書き換えた後、起動中のアプリで向きが変わる時期〉と実機 1 回)。それ以外は `不明`。**第二段の着手条件**: V2(UI が無変換・変換の 1 列目と閉の `S4key` に書くコードの一覧)、実機での V1(dragonflyg4 の全表と、無変換・変換・ATOK テンプレートでの挙動)、エンジンの同定(実測11)。Custom の 1 列目に `CD` などを置いた ND=1/0 の試行(`v0e`)は済んだ(実測13)。2〜6 列目(入力中・変換中)は読まない(列と状態の対応が未確定、実測5)。
- ND(`NoDirectInputMode`)を入力に取る。`D5` のように ND で効果が変わるコードは、ND を読めたときだけ予測し、読めなければ `不明`。

### 決定4(第三段、実機で確認できたら): 役割(ADR-199)への接続

役割が `ImeToggle` になると awase が自分から IME を動かす(ADR-206 `role_open_action`)ので、予測より強い証拠を要する。第一・二段には含めない。

### 決定5: 失敗は安全側

読めない・コードが未実測・行が重複・`keystyle` が未知のいずれも `不明`(予測しない)。第二段の `不明` は決定1の「止める」と同じく、学習表があればそちらを使う。

## 非目的

- awase が表を書き換える、旧UIを操作する。新UI(`KeyAssignment*`)と旧UIの同時カスタムの調停。修飾付き(Ctrl/Shift)・F キー・第二段範囲外のキー。

## 代替案

- A′(決定1): 第一段として採用。変更は `for_msime_native` への引数 1 つと `predict_with_override` の打ち切り 1 か所程度。
- B(決定3): 表が正確ならゼロ設定で効く。A′ で報告の症状が直るのか、B まで要るのかは、報告 journal 1 件(どのキーで belief がどうずれたか)が示されるまで不明(V1)。

## 検証計画(Opus r1 の V0〜V7 を採用、各セル n≥5、ABAB、前提状態を満たさない試行は破棄して件数を記録)

順序: V0′ → V0 → V1 → V2/V2′ → V3 → V5 → V4・V6・V7。結果次第で第二段を止める判定基準は各項目に書く。
**V1 は第一段のマージ条件にしない**(第一段は新しい意味づけを持ち込まず、ADR-196 の「誤予測より予測しない」の範囲内。V1 で決まるのは第二段に進むかどうか)。

検証スクリプトの前提(Opus r2 T1〜T8):
- 開・入力中は `k` 単独(子音だけの未確定)ではなく `4B,41`(か)で作る。`k` 単独は別セルとして残す(T1)。
- 試行ごとにハーネスを起動し直し、起動時の閉から 1 打で目的の状態へ行く。ESC で入力なしへ戻さない(T2)。`1C`(S4key 変換=87)で開けない表のセルでは、`VK_IME_ON`(0x16)で開けるかを「前」のスナップショットで毎回確かめる。
- V0 の列判別には `97`(1 列目で conv→0x1B、入力中の列も 0x1B と効果が分かっている)を使う。`D5` は 2 段目(T3)。
- ABAB はプロセスをまたいで並べ、4 本目が 2 本目と同じかを判定に入れる(IME 側の表のキャッシュ漏れの見張り、T4)。
- 破棄規則: 各ステップの「前」(`open`・`conv`・`comp`)とヘッダの `状態=`、次の試行の「前」の `tail` が入力欄以外(`&No`・バナーの断片)なら捨て、件数を記録する(T5・T7)。`--no-probe` では +1500ms を最終状態として使ってよい。
- V0′ の UI 操作(BM_CLICK id=5004 + Apply)は `keystyle` をコンボの選択値で書き直す等の副作用がある(T6)。Apply の前後で `MSIME` と `StyleList\Custom` の全値の差分を取り、試行の前に `keystyle` を書き戻す。「ND=0/1 で同じ」は、レジストリ直書きと UI 経由の両方で変わらないときだけ「その行の効果は ND に依らない」と書く。UI 経由でだけ変わるなら `option1`/`option2` のビットが実体。
- 起動時の閉の状態(open・conv)は ND で違うかもしれないので、同じ「前」の試行どうしだけを比べる(T8)。

| ID | 確かめること | 捨てる条件 | 結果(CI 1 環境) |
| --- | --- | --- | --- |
| V0′ | ND の陽性対照: NATURAL の半角/全角を開・入力なしで押し、ND=1 と ND=0(書換と、UI のチェックボックスの実際の書込み)で比べる | 結果が変わらなければ ND の主張を取り下げる | 実測6: ND は効果を変える(`D5`)。半角/全角は変えない。UI のチェックボックスの実体は `NoDirectInputMode` |
| V0 | `key` の列と状態の対応: 無変換行を「1 列目=D5・他=00」と「1 列目=00・他=D5」にして、開・入力なしと開・入力中で押す | 1 列目設定で入力なしが閉じ入力中が無反応なら B2 が正しい。どちらでもなければ第二段を止める | 実測5・12・13: 1 列目=開・入力なし。dragonflyg4 型の行は組み合わせで入力中に効く。2〜6 列目と状態の対応は未確定(第二段では読まない) |
| V1 | 報告の再現: dragonflyg4 の `StyleList\Custom` 全値(`S*key` を含む)で、開・入力なし・開・入力中・閉の 3 状態、awase の予測と比べる | 報告の症状が A′ だけで消えるなら第二段は保留 | 未実施(dragonflyg4 の全表が必要。実測10 は既存観測と整合) |
| V2/V2′ | UI が保存する Custom の形(`S4key` を書くか)。自動化できなければ所有者が dragonflyg4 で `reg export` | UI が `S4key` を書かない/`87`・`CE` だけなら閉側は「行の有無」に縮める | UI の Apply が書く値は取得。Custom の `S4key` を UI が書くかは未実施(Advanced の操作) |
| V3 | 互換モード検出: `NoTsf3Override2`・`DisableNewIME` の 4 通り × Custom(新UI のとき Custom が効かないことも) | 効く組み合わせを決定1・3の条件にする | 実測7・8・11: windows-latest ではフラグに関係なく効くが、陽性対照が取れず旧エンジンのみの可能性が高い。keystyle 不在=NATURAL |
| V4 | 名前付きスタイルの開状態と、名前付き `key` を書換えて開・入力なしで押す | 書換が効くなら「内蔵表」の前提を捨てる | 実測9: 名前付きは内蔵表(`key` 書換は無視)。ATOK・VJE の開側は未測定 |
| V5 | 生きているプロセスへの反映(起動中アプリ・ctfmon 再起動・アプリ再起動) | プロセス起動時だけ読むなら、表が変わった直後は `不明` | 未実施 |
| V6 | アプリの種類(Chrome または Edge、Windows Terminal) | 結果が違えば窓の種類ごとに扱いを分け、分けられなければ読めない窓では予測しない | 未実施 |
| V7 | ND=0 の利用者で V0・V1 | 3 つ組の表にするか ND=0 を `不明` にする | 実測6 に含む(ND=0 は n=3) |

## 実装の分け方(第一段、Opus r3 の案。**実際は 1 つの PR にまとめた**)

- **PR 1(純粋な部分、Linux でテストが走る)**: `KeyEffectKeymap::for_msime_native` に `keystyle` の読み取り結果(`Absent`/`Natural`/`Other(LegacyKeyStyle)`/`Unreadable`)と、Custom の表のハッシュ(`Option<u64>`)を渡す。新フィールド `legacy_table_unknown: bool`。`predict_with_override` で、学習表の参照の後・`custom_table` の打ち切りの前に `if self.legacy_table_unknown { return None; }`。
- **PR 2(Windows の殻、`cargo check --target x86_64-pc-windows-msvc`)**: 既存の `keystyle` 読み取りの結果を `Absent`/`Unreadable` を区別したまま渡す。Custom の全値を `RegEnumValueW` でハッシュ。スタンプ(決定2)。予測を止めた理由を journal(tracing)へ1行(実装済み)。不具合報告(`bug_report.rs`)へ出すのは後続(既存の報告には互換モードと `keystyle` の欄がある)。
- **PR 3(文書)**: ADR-197 への訂正の追記、ADR-254 の整備、findings の訂正。
- 単体テスト(PR 1、`#[cfg(windows)]` の外):(1) `Other(Custom)` で、`MSIME_NATIVE` にセルがある全 `TableKey` の VK × 開/閉 × 段階の全組み合わせが `None`(表から VK 集合を作ってループ)。(2) `Absent`/`Natural` は今の既定と全組み合わせで一致。(3) `Other(Custom)` に学習表を渡すと学習表の予測。(4) `Unreadable`・未知の名前で `None`。(5) 互換フラグ `Some(true)`/`Some(false)`/`None` × `Other(Custom)` がすべて `None`。(6) 指紋: `Absent`/`Natural` の指紋が変更前の値と一致(定数の golden)、Custom のハッシュが違えば指紋が違う、ATOK と VJE で違う。(7) `msime_native_key_role` が `keystyle` で変わらない。(8) `Other(Custom)` かつ `input.unreadable=true` で `None`。
- fix-requires-evidence の (a) は PR 1 のテストで満たす。(b) の `docs/known-bugs/` は V1 で症状が確定してから起票する。
- 「学習で戻せる」の前提: MS-IME 本体で学習が CI で完走するかを、決定1の根拠に書く前に確かめる(過去に conv 0x0001 未対応で失敗した記録がある)。完走しなければ「戻せない」と書く。指紋は `current_fingerprint_probe`(`key_effect_runtime.rs:442-454`)→`read_key_effect_keymap_native().fingerprint()` の 1 か所から作られ、学習プロセスも同じ関数を使うので、ここで `keystyle` を足せば予測側と学習側で一致する(確認済み)。

## V1 の依頼文(所有者向け、案)

dragonflyg4(互換モード ON のまま)で、次の 2 点をお願いします。どちらも読み取りだけで、設定は変えません。3 は任意です(一時的に設定を変えて、最後に戻します)。

1. レジストリの書き出し(PowerShell。`msime.reg` には `IMEUserName`〈Windows のユーザー名〉と辞書ファイルのパスが入るので、気になれば該当行を消してから):
   ```
   reg export "HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME" msime.reg /y
   reg export "HKCU\Software\Microsoft\IME\15.0\IMEJP\StyleList" stylelist.reg /y
   reg export "HKCU\SOFTWARE\Microsoft\Input\TSF\Tsf3Override" tsf3.reg /y
   ```
2. 今の設定のまま、メモ帳で、それぞれ IME の表示(あ/A/カ など)がどう変わったかを教えてください。(a) IME オンで何も入力していない状態で無変換を 1 回 (b) 「k」を 1 回だけ打った状態(ｋ が下線付きで残っている)で無変換を 1 回 (c) 「か」を打った状態で無変換を 1 回 (d) IME オフで無変換を 1 回。
3. (任意・エンジンの確認。互換モード ON の利用者の調査には必須ではない)「以前のバージョンの Microsoft IME を使う」のチェックを外して新しい Microsoft IME に戻し、`& "$env:windir\System32\IME\IMEJP\IMJPUEXC.EXE" setkeytemplate ATOK` を実行して、メモ帳を開き直してください。IME をオフにして変換を 1 回押し、IME がオンになるかを教えてください。終わったら `setkeytemplate Microsoft_IME` を実行し、チェックを元に戻してください。これで分かるのは「`keystyle` が新しい IME にも効くか」だけで、決まるのは第一段の止める条件に互換モードの ON/OFF を含めるかどうかです(CI は互換モードの ON/OFF に関係なく旧エンジンが動いているように見え、判定できない)。

また、`StyleList\Custom` を旧UIで作ったのか、スクリプトで書いたのかを覚えていれば教えてください。報告者本人にも同じ 1 と 2 を頼めると、V1 の材料として一番よい。
