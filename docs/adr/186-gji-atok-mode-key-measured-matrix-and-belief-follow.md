---
id: ADR-186
title: |-
  GJI(ATOK)のモードキー動作を実機で測定し、無変換/変換の開閉トグルを「タイマーでなくKeyUpで解決する」
  既存のdelegate経路で押下時点にbelief追随させる(実機E2Eと撤去実験で、必要な仕組みと不要な仕組みを確定)
summary: |-
  ADR-184/185は「ATOKの無変換はIME ONのまま半角英数に変える」を前提にしていたが、awase非依存のスパイク
  (`crates/awase-windows/examples/ime_key_matrix_spike.rs`)で測った結果、**公開Mozcの`atok.tsv`が実機GJIの
  動作**だった(素のEDITとRichEditで一致。状態は入力なし/変換前/変換中に分けて読む)。無変換/変換は入力なしの
  とき**IME開閉のトグル**、ひらがなキーはかな⇔半角英数のトグル、convは開閉遷移をまたいで保存、半角/全角は
  0xF3/0xF4ともトグル。ユーザー要件「かな=Engine ON、英数(半角英数・直接入力)=Engine OFF」を、押下直後から
  満たすため、実機で**実IME状態とEngine切り替えを自動検証するE2Eハーネス**(`tools/e2e/ime_key_matrix`、
  SendInput注入+awase debugログ照合、`AWASE_TEST_INJECTION=1`のテスト専用分岐)を作り、決定2の実装を実機で
  検証した。決定2が動かなかった**根本原因は1つ**: タイマー(100ms)で解決した親指の単独タップのSetOpenは、
  belief書き込み・明示意図の記録を持つキーボード経路を通らず、warrantが`Unwarranted`でOFFを拒否していた。
  修正は述語1つの変更(delegateを持つ親指もKeyUpで解決)。加えて変換の分類(ATOKでは古いcustom表を読まない)。
  撤去実験(各3回、実機)で、KeyUp解決・ATOK分類修正・物理キー後の20ms再読み取り・opt-inフラグは**必須**、
  eisu reset抑止(旧決定2の一部)は**不要**と確認し、削除した。残る問題: awase起動中は、まれに(1押下あたり約5%)
  押下がGJIに届かない回があり、GJI単体では起きない(別件、BUG起票)。
status: |-
  **v4(実装済み・実機E2Eで検証、撤去実験の結果を反映)**。実装ブランチ`feat/adr186-nonconvert-toggle-belief-follow`。
  E2Eの再現性: 条件が良いとき連続11回ALL PASS。押下の取りこぼしによる失敗が残る(下記「残る問題」)。
  GitHub Actionsの実機E2E(`.github/workflows/e2e-ime.yml`)で、撤去実験の8構成×3回が実機と同じ結果になることを確認済み
  (下記「CIでの再現」)。
related_adr:
  - "ADR-090"
  - "ADR-176"
  - "ADR-179"
  - "ADR-184"
  - "ADR-185"
---

# ADR-186: GJI(ATOK)モードキーの実測と、無変換/変換の押下時点belief追随

## 背景

ユーザー要件: **かなのときEngine ON、英数のときEngine OFFを徹底する**。英数は「IME ONの半角英数」と
「直接入力(IME OFF)」の両方(awaseの挙動はどちらでも同じ=Engine OFFでよい、ユーザー確認済み)。

現状の弱点: 無変換などの押下は生キーとしてGJIに渡されるだけで、awaseのbeliefは動かない。Engineが切り替わる
のは、次の打鍵の後に`idle-conv-check`が変換モードを読んで`ObservedEisu`と判定したとき(遅延観測)。
押下直後の最初の1打はEngine ONのままNICOLAで処理される。

## 実測

- 環境: dragonflyg4、Google日本語入力、`session_keymap = 1`(ATOK、`config1.db`をデコードして確認。オーバーレイ
  なし。同ファイルに残るMS-IME風の`custom_keymap_table`はプリセット選択時は読まれない)、awase停止。
- 方法: スパイクが`WH_KEYBOARD_LL`でキーを捕捉し、押下前・+400ms・+1500msの観測値を並べて記録
  (A=`ImmGet*`、B=`WM_IME_CONTROL`、T=TSFスレッドcompartment、G=TSFグローバルcompartment)。状態は
  変えず、案内に従ってユーザーがキーで作る。**A/B/Tは、フォーカスが入力欄から外れた無効レコード3件
  (`round1-edit.log:94-96`)を除いて全件一致**。Gは常に0で不採用(ADR-176の手法Cは
  `GetGlobalCompartment`のスコープの誤り)。
- 対象: 標準EDITとRichEdit 5.0(TSFネイティブ)。ただし**2ラウンドは別ビルドで走らせた**(round1は24ステップ版、
  round2は英数を除き、Shift併用を許可した20ステップ版)。両方で測れたセルは、次の表で一致。
- 注意: ログの時刻は記録完了時刻(押下の約1.5秒後)。押下が1.5秒未満の間隔で続いた箇所は、前のキーの効果が
  次の記録に混ざる。**+400ms値を優先して読む**(+1500msは後続の押下で汚染されやすい)。
  `tail`が入力欄以外(ログ欄、ウィンドウタイトル)になっている記録は無効(`round1:242`、`round2:294-318`)。
- 生ログ: `186-measurements/`。

### 結果(ATOKプリセット。クリーンな記録のみ。r1/r2=行番号)

| 状態 | 無変換(0x1D) | 変換(0x1C) | ひらがな(0xF2) | 半角/全角(0xF3/0xF4) |
|---|---|---|---|---|
| 直接入力 | IME ON(r1:9, r2:9) | IME ON(r1:19, r2:19) | 変化なし(r1:24, r2:29) | **0xF4**: IME ON(r1:56, r2:39)。0xF3: **有効な測定なし** |
| ON・入力なし・かな | **IME OFF**(r1:61, r2:44) | **IME OFF**(r1:66, r2:54) | ONのままconv 0x09→0x10(かな→半角英数)(r1:71, r2:64) | **0xF3**: IME OFF(r1:83, r2:75) |
| ON・変換前(Composition) | ONのままconv→0x10、未確定保持(r2:90) | 変換(comp か→下)(r2:95) | 半角英数トグル(r2/r1で未確認) | **0xF3**: IME OFF、未確定破棄(r1:160) |
| ON・変換中(Conversion) | **効果なし**(r1:108) | 次候補ページ(comp か→🉑)(r1:113) | conv 0x10→0x19(r1:143)※ | **0xF4**: IME OFF、未確定破棄(r2:142) |
| ON・入力なし・半角英数 | **IME OFF**(r1:175, r2:152) | **IME OFF**(r1:180, r2:162) | かなに戻る(conv 0x10→0x19)(r1:185) | 未測定 |

※ `atok.tsv`に`Conversion Kana`行は無いため、この効果はMozcではなくOS/IMM側のDBE効果の可能性がある。

- 決定5の根拠(0xF4がON中に届いてOFFにした実例)は、Conversion状態での1件(`round2:142`)。
- Shift+無変換: ON・入力なしで、かな⇔半角英数のトグル(conv 0x19⇄0x10)。両ラウンドで確認。
- **convは開閉遷移をまたいで保存される**。直接入力(conv 0x10)から変換でONにしたとき、conv 0x10のままONに
  なった(`round2:147-151`、同型3件: `147/157/284`)。`IMEOn`は`key.mode`(=直前のvisible conv)を復元する
  (`session.cc:1023-1034`、`win32/base/keyevent_handler.cc:700-705`)。
- 未測定: 英数(0xF0、物理キー無し)、直接入力×0xF3、ON・半角英数×半角/全角、Shift+ひらがな(0xF1、
  カタカナ)のON中の効果(`atok.tsv`に`Katakana`行が無いので、keymap上は未割当。OS/DBE側の効果は未確認)。
- 初期convは0x09(NATIVE|FULLSHAPE、ROMANなし)。最初の半角英数往復の後は0x19(+ROMAN)になり戻らない。
- 上流`atok.tsv`(Mozc master `13c98988`)との照合: **一致する**(Conversion×無変換の行が無い=効果なし、を
  含む)。ただし「ON・入力中」を、Composition(変換前)とConversion(変換中、Space後)に分けて読む必要がある。
  round1のSTEP13がConversionだったのは、直前(`round1:103`)のSpace押下と、STEP14の結果(次候補ページ)から。
  Windowsのキー→Mozcキー名: 0x1C=HENKAN、0x1D=MUHENKAN、0xF2=KANA、**0xF3と0xF4はどちらもHANKAKU**
  (`keyevent_handler.cc:315-316`)。IME OFF中にMozcへ届くのは`DirectInput`行のキーのKeyDownだけ
  (`keyevent_handler.cc:680`)。

## 前提の訂正

1. ADR-184の症状「ATOKの無変換はIME ONのまま半角英数」は、**入力中(Composition)のときだけ正しい**。
   入力なし(Precomposition)や半角英数ONでは、IME OFFになる(`atok.tsv` Precomposition Muhenkan =
   `CancelAndIMEOff`)。過去の観測は、状態の取り違え、またはIME OFFとの見分けのつかなさによる。
2. 「直接入力での無変換は何も起きない」は、素のWin32/RichEditでは成立しない(ONになる)。メモ帳・
   Windows Terminalでの観測との差は未解明(未解決事項)。
3. `classify_mode_key_ime_action`のATOK分類: 無変換/変換=`Toggle`(開閉トグル)は入力なしのとき正しい。
   ひらがな=`None`も正しい。
4. ADR-185(半角英数を検出しても、awaseからIME OFFを送らない)は、この表と矛盾しない。
5. **awaseの既存モデルが誤りの箇所**: (a) `vk.rs`は0xF4=`TurnOn`(一方向)としているが、GJIでは0xF3/0xF4とも
   トグル(`round2:142`: ON中に0xF4でOFF)。(b) `key_pipeline.rs`の「ユーザーがIMEをONにした時点でIMEは
   ひらがなで再開するため、過去の英数観測はstale」(`eisu_reset_on_ime_on`、`PostSetOpenEisuReset`)は、
   GJI/ATOKでは成り立たない(convは保存される)。

## 決定(v4)

**決定1 — 実測表を一次情報として固定する。** 本ADRと`186-measurements/`を、ATOKプリセットの動作の根拠とする。
「全セル一致」は撤回し、状態をComposition/Conversionに分けた表(上)を正とする。

**決定2 — 無変換/変換(入力なし)の押下時点のbelief追随は、既存のdelegate-to-open-axis経路を使い、次の3点で成立させる。**
実体は`src/engine/nicola_fsm.rs`の`resolve_pending_thumb_as_single`内の`special.delegate_to_open_axis`分岐
(Toggleのとき、awaseが自分のbeliefに従って明示ON/OFFをactuate。合成送出なし、物理キーは`Decision::Consume`
で中継されないため二重actuationにならない)。この経路はPassthrough(優先順位4)より優先(3)なので、発火する
状況では「Passthrough設定」は効かなくなる(入力中を除く)。

- **(a) opt-in `gji_thumb_key_ime_toggle = true`が必要**(撤去実験E7b: falseだと`delegated`が0件で、手順5・6が
  3/3失敗)。無変換/変換が親指キーとして設定されている前提。ATOKは無変換と変換の両方をToggleにする。
- **(b) delegateを持つ親指の単独タップを、タイムアウトでなくKeyUpで解決する**(`defers_solo_until_release`の
  対象から`delegate_to_open_axis`持ちの除外を外す、述語1つの変更、コミット`2b93e185`)。
  **決定2が実機で動かなかった根本原因**: タイマー(しきい値100ms)で解決した単独タップの`SetOpen`は、非キーボード
  経路(`execute_from_loop`)で実行され、belief書き込み(`handle_engine_set_open`)・明示意図の記録・eisu resetを
  持つキーボード経路(`kp_stage_post_decision`)を通らない。押下が100msを超える通常のタップでToggle OFFが直前の
  明示ON意図(IntentStore、10秒TTL)に対するwarrantで`Unwarranted`となり実行されず、awaseがONを再送していた
  (実機ログ2026-09-20)。ADR-179のbelief追随もタイマー経由では同じ理由で動いていなかった。撤去実験E1で、
  この変更を戻すと3/3失敗(手順5のOFFが効かない)することを確認した。
- **(c) ATOKプリセットでは、古い`custom_keymap_table`を読まない**(`gji_charset_autodetect.rs`、コミット`bff621b5`)。
  表に`DirectInput Henkan IMEOn`が残っていても、ATOKのGJIは変換を`atok.tsv`どおり開閉トグルとして扱う。
  ADR-174のフォールスルーを全プリセットで有効にしておくと、変換が`On`(冪等・belief追随のみ・生キー素通し)と
  誤分類され、実IMEのOFFにbeliefが追随しなかった。MS-IMEはADR-174の実機根拠があるため変更しない。
  撤去実験E2(変換キー)で、この修正を戻すと3/3失敗(各5件)することを確認した。

**決定3 — かな英数トグル(ひらがなキー)の押下時点予測反転は、不要とする(実装しない)。**
実機E2Eで、ひらがなキー(かな⇔半角英数)の後、Engineは**押下の30〜70ms後**に追随した。仕組みは既存の
「物理IMEキー(`may_change_ime`)が通過した20ms後のIME再読み取り(`ObserverPoll`)」で、conv(0x19⇄0x10)を読んで
`ObservedEisu`⇔romajiのbeliefを更新する。この再読み取りを撤去すると手順7・9が3/3失敗した(撤去実験E5)。
**この20ms再読み取りは必須**。ただし、これはIMM32でconvを読めるアプリ(Win32 EDIT/RichEdit、非TsfNative)での
確認で、TsfNativeでは`ime_on=None`のため読めず、`idle-conv-check`(次の打鍵後)が担う(未検証、下記)。

**決定4 — 入力中の無変換は、現状維持とする。** `resolve_pending_thumb_as_single`は`composing`を引数に取り、
delegateはcomposing中に発火しないfail-closedになっている(誤ってfalseでToggle→OFFすると、compositionを破棄する)。
この保護は外さない。入力中の効果(ONのまま半角英数)へのEngine追随は、遅延観測のまま。

**決定5 — 半角/全角(0xF3/0xF4)のモデル誤りは別件として切り出す。** GJI使用時は0xF3/0xF4とも
`CancelAndIMEOff`/`IMEOn`のトグル(`atok.tsv`の全状態、`keyevent_handler.cc:315-316`)。awaseは
0xF3=TurnOff、0xF4=TurnOnとしており、物理キーはGJI時に常にSuppressされる(`transport.rs:405-418`)。修正箇所
(`vk.rs::shadow_effect`か`transport.rs`か)は別のBUG/ADRで決める。本ADRでは変更しない。

**決定6 — 実機E2Eハーネスとテスト専用の注入分岐を導入する。**
`tools/e2e/ime_key_matrix/`(`run.sh`/`check.py`/`ablate.sh`/`run_experiments.sh`/`stat_runs.sh`)。
スパイク(`--auto`)が`SendInput`で手順のキーを注入し、実IME(ImmGet\*/WM_IME_CONTROL/TSF compartment)を押下前・
+100/+400/+1500msで記録、awaseのdebugログ(Engine activated/deactivated、`delegated`、`Unwarranted`)と
時刻で照合して期待表に対しPASS/FAILを出す。注入キーは`dwExtraInfo=0x5350494B`の目印付き。**注入は物理キーと
実IMEの動きは同じだが、awaseでは`LLKHF_INJECTED`扱いとなり押下時点のbelief追随を通らない**(実測: Engine追随が
+1ms→+546ms/欠落、`check.py`がFAILにする)ため、`hook.rs::is_test_injection`で**環境変数`AWASE_TEST_INJECTION=1`
かつ目印一致のときだけ**`is_injected=false`として扱う。本番は環境変数が無く動作は変わらない
(コミット`ac46c20c`)。フォーカスが途中で外れた回・物理入力が混入した回は`INVALID`として判定に使わない。

## 撤去・統合の実験(実機、Win32 EDIT×GJI ATOK、押下保持180ms、各構成3回)

各実験は、実装ブランチにコード撤去を当て(`ablations/`のミューテーター)、Windowsへデプロイし、E2E10手順を実行。

| # | 撤去・変更 | 結果 | 結論 |
|---|---|---|---|
| E0h | (基準、変換キーで検証) | 3/3 ALL PASS | 変換キーでも無変換と同じに動く |
| E1 | KeyUp解決(決定2b)を戻す | **3/3 FAIL**(各3件) | **必須** |
| E2 | ATOK分類修正(決定2c)を戻す(変換) | **3/3 FAIL**(各5件) | **必須** |
| E3 | eisu reset抑止(旧決定2の一部)を撤去 | 3/3 ALL PASS | **不要**、削除(下記) |
| E4 | eisu resetの全経路(3種)を撤去 | 3/3 ALL PASS | Win32では不要。**検証できない領域あり**(下記) |
| E5 | 物理キー後の20ms再読み取りを撤去 | **3/3 FAIL**(手順7・9) | **必須**(決定3) |
| E6 | idle-conv-checkを無効化 | 3/3 ALL PASS | Win32では使われない(TsfNative限定)ため**検証できない** |
| E7b | `gji_thumb_key_ime_toggle=false` | **3/3 FAIL**(手順5・6、`delegated`=0) | opt-inは**必須**(決定2a) |

- **E3の削除**: 旧決定2は、直接入力(半角英数のまま)から無変換でONにしたとき`PostSetOpenEisuReset`がEngineをONにして
  実IMEが半角英数になる事態を防ぐ`eisu reset`抑止(`f5f78dfb`)を含んでいた。E2E全実行で抑止のログ(`reset を抑止`)は
  一度も発火せず、外しても通った。delegateが動くのはEngineがONのときで、ObservedEisuは定義上Engineがinactiveの
  ときに立つため、この抑止が働く場面は成立しない(半角英数中の無変換は生キーがGJIへ素通りする)。**デッドコードとして
  削除**した(`3f9b313e`、新しい型・フィールドは増やさず、コードは減った)。
- **E4・E6で検証できない領域**: eisu resetの3経路(`PostSetOpenEisuReset`/`UserImeOnEisuReset`/`UserTurnOnEisuReset`)は
  ObservedEisu循環デッドロック(2026-07-06 MS Edge、Imm32Unavailable=IMMでconvを読めないアプリ)の対策を兼ねる。
  Win32 EDITではIMMの再読み取りが同じ役割を果たすため、E4は通る。**Chrome/Edge/TsfNative(メモ帳、Windows
  Terminal)は、このスパイクでは測れない**。統合(撤去)は、これらのアプリで同じE2Eを回せるようになるまで**しない**。

## CIでの再現(GitHub Actions、Windowsランナー)

実機を占有せずに撤去実験を並列で回すため、`.github/workflows/e2e-ime.yml`でE2Eを`windows-latest`上で実行する。
`plan`(構成表)→`build`(構成ごとに撤去を当てビルド、ソースのハッシュで成果物をキャッシュ)→`e2e`(構成×3回、別ランナー)
→`summary`(有効回のPASS/FAILを期待と照合)。GJIはchocolateyで入れ(約30秒)、入力言語をja-JP+GJIにし、ATOKプリセットの
`config1.db`(field 41=1)を書いて変換サーバーを再起動する。全体で約4分(ビルド成果物のキャッシュが効いた回)。

| 構成 | 期待 | 結果(run 35485279828) |
|---|---|---|
| baseline / baseline-henkan | PASS | 3/3 PASS ×2 |
| a1 KeyUp解決を撤去 | FAIL | 3/3 FAIL |
| a2 ATOK分類修正を撤去(変換、古いcustom表あり) | FAIL | 3/3 FAIL |
| a4 eisu reset全経路を撤去 / a6 idle-conv-checkを無効化 | PASS | 3/3 PASS ×2 |
| a5 20ms再読み取りを撤去 | FAIL | 3/3 FAIL |
| e7 `gji_thumb_key_ime_toggle=false` | FAIL | 3/3 FAIL |

実機の結果(E1/E2/E5/E7bが必須、E4/E6は不変)と全構成で一致した。CI固有の前提(実機では暗黙に満たされていた):

- a2は、`config1.db`に古い`custom_keymap_table`(`DirectInput Henkan IMEOn`)を入れた構成でだけ差が出る
  (実機には過去のCUSTOM設定の表が残っている。CIの素の`config1.db`では撤去しても壊れず、3/3 PASSした)。
- awase起動時のbelief(ON推定)と実状態(OFF)をそろえるため、起動後にVK_IME_OFFを1回注入する(スパイクの`--activate-gji`)。
- スパイクの初期化中にawaseがIMMをプローブすると`Edit`を「IMM不可」と誤学習するため、初期化後にawaseを起動し、
  学習済みキャッシュ(`cache.toml`)を事前投入する。
- LLフックは後から張ったものが先に呼ばれるため、スパイクのフックはawase起動後に遅延インストールする。
- **BUG-148**: awase起動時に既に対象アプリが前面にあると`current_focus`が最初のプロセス切替まで`None`のままで、
  委譲SetOpenが全て`Unwarranted`になり無変換がGJIに届かなかった。CIで発見し、起動時の初期フォーカスで
  `current_focus`を設定して修正した([BUG-148](../known-bugs/BUG-148.md)、`9ac77696`)。修正後は、以前必要だった
  「notepadを起動してフォーカスを移す」回避策なしでbaselineが3/3 PASSする。

## 残る問題

1. **押下の取りこぼし(awase起動中のみ)**。同じE2Eで、**GJI単体(awase停止)は5/5で全手順の実IMEが期待どおり**だったのに、
   awase起動中は、まれに1押下がGJIに届かない回がある(1回の実行で1手順だけ、失敗する手順は毎回違う: 無変換のOFF、
   ひらがなの切り替え)。失われた押下のログでは、awaseは`PassThrough`→`[reinject] vk=0x1d down`と正常に再注入
   しているがGJIが反応していない。有効な回の失敗率: 最新実装(抑止なし) 4/8(INVALID 4件除く)。条件が良いとき
    (Windowsを放置)は、連続11回ALL PASSした。原因は未特定(awaseの通過→再注入経路、TSF warmup/cold、
   フォーカス移動時のcold化のいずれか)。ADR-186の変更が原因かの切り分けとして、eisu reset抑止あり/なしの比較を
   取った(下記)。別件として[BUG-147](../known-bugs/BUG-147.md)に起票した(CIでは再現していない)。
2. **Shift+無変換が開閉トグルとして横取りされる**(`gji_thumb_key_ime_toggle=true`のとき、`action=Toggle`)。
   GJIのShift+無変換はかな⇔半角英数トグル。opt-in有効時は使えなくなる。
3. **TsfNative(メモ帳、Windows Terminal)・Chrome/Edgeは未検証**(上記)。

## 期待される結果(決定2〜4を実装した場合)

| ユーザー操作 | ATOK実動作 | awaseの結果(実機E2Eで確認) |
|---|---|---|
| かな・入力なしで無変換/変換 | IME OFF | delegate→false(KeyUp解決)、belief OFF、Engine OFF |
| 半角英数ON・入力なしで無変換/変換 | IME OFF | 生キー素通し、Engineは非活性のまま |
| 直接入力で無変換/変換 | IME ON(convは直前値を復元) | かななら押下時点(+1〜5ms)でEngine ON、半角英数のままならEngine OFFのまま |
| ひらがなキー(かな⇔半角英数) | conv 0x19⇄0x10 | 押下の30〜70ms後にEngineが追随(20ms再読み取り) |
| 入力中(Composition)で無変換 | ONのまま半角英数 | delegateは発火しない(遅延観測) |
| 半角/全角 | ON/OFFトグル | 決定5(別件) |

## リスク(BUG-115が挙げた却下理由と本ADRの関係)

`src/config.rs:410-422`の却下理由4点のうち、本ADRの実測で潰せたのは「4. GJIが本家`atok.tsv`と一致する保証がない」だけ。
残る「1. Toggleの非冪等性」「2. 親指キー2本への露出倍増」は、opt-in(`gji_thumb_key_ime_toggle`)で緩和するが残る。
`config.rs:452-455`が警告する「TSFネイティブ(`FeedbackPolicy::Blind`)では実IME状態を読み戻せないため、beliefが
ズレると逆方向へ切り替わる」は、決定2の中心的リスクで、ユーザー原則「IME ON/OFFは安定して観測できない」と直結する
(TsfNativeは未検証)。無変換/変換の単独タップごとに`record_explicit_intent`(`UserIntentSource::Command`)が走り、
`EXPLICIT_ON_INTENT_TTL_MS = 10_000`の間、open意図がIntentStoreに固定されてdrift correctionより優先される
(今日はPassthroughのため記録されない新しい露出)。フォーカス遷移直後(settling中)は、`SetOpen`が2段フィルタで落ち、
物理キーも`Decision::Consume`で中継されないため誰も切り替えない空振りになる(今日は生キーがGJIに届くので退行)。

## 検証(再現手順)

1. WindowsでawaseをADR-186実装ブランチのビルドで、`AWASE_TEST_INJECTION=1`・`RUST_LOG=debug`・
   `gji_thumb_key_ime_toggle=true`で起動する(ATOKプリセットのGJI)。
2. `tools/e2e/ime_key_matrix/clipwire-targets.example.toml`のターゲットを登録し、`./run.sh`(約50秒。実行中はWindows機の
   キーボード・マウスに触らず、ロックさせない)。`ALL PASS`(終了コード0)が合格。
3. 撤去・統合の実験: `run_experiments.sh`(結果は`results/SUMMARY.md`)、`stat_runs.sh <label> <N>`(有効回N回の集計)。
   GitHub Actionsでは`e2e-ime`ワークフロー(`workflow_dispatch`の`only`入力で構成を絞れる)。
4. ユニット/ガード: `cargo test --lib`、`cargo test -p awase-windows --lib --test architecture_guard`、
   `nicola_fsm`の`delegate_to_open_axis_solo_tap_resolves_at_key_up_not_at_timeout`(KeyUp解決の契約)、
   `classify_atok_session_keymap_ignores_stale_custom_table`(ATOK分類)。

## 未解決事項

- メモ帳・Windows Terminal・Chrome/Edgeでの同じE2E(TsfNative/Imm32Unavailable)。eisu reset3経路とidle-conv-checkの
  統合可否は、これが済んでから判断する。
- 「残る問題1」(押下の取りこぼし)の原因特定(BUG-147、別セッションで調査中)。
- Shift+無変換の横取りへの対処(delegateを修飾キー付きで発火させない、など)。
