---
id: ADR-186
title: |-
  GJI(ATOKプリセット)のモードキー動作を実機で測定し、無変換/変換の開閉トグルだけを既存のToggle経路で
  押下時点にbelief追随させる(かな英数トグル・入力中・半角/全角は別扱い)
summary: |-
  ADR-184/185は「ATOKの無変換はIME ONのまま半角英数に変える」を前提にしていたが、awase非依存のスパイク
  (`crates/awase-windows/examples/ime_key_matrix_spike.rs`)で素のWin32 EDITとRichEdit 5.0を測った結果、
  **公開Mozcの`atok.tsv`が実機GJIの動作**だった(状態を「入力なし/変換前(Composition)/変換中(Conversion)」に
  分けて読めば矛盾しない)。無変換/変換は入力なしのとき**IME開閉のトグル**(ON中→OFF、直接入力→ON)。
  入力中(Composition)の無変換だけがONのまま半角英数(ADR-184の症状は、この状態では正しい)。
  ひらがなキーはかな⇔半角英数のトグルで開閉に触れない。**変換モード(conv)は開閉遷移をまたいで保存される**
  (IMEOnは直前のconvを復元する。session.cc:1023、keyevent_handler.cc:700)。半角/全角は0xF3/0xF4のどちらも
  `HANKAKU`に潰され、GJIでは両方**トグル**(ON中に0xF4を押すとOFFになる)。ユーザー要件は「かな=Engine ON、
  英数(半角英数・直接入力)=Engine OFF」。現状は押下でbeliefが動かず、遅延観測(`idle-conv-check`、
  次の打鍵後)でしかEngineが切り替わらない。opus round1(Blocker3件)を受け、本ADRの決定は
  **無変換/変換(入力なし)の押下時点のbelief追随だけ**に絞る。かな英数トグルは実機確認まで保留、
  入力中の無変換は現状維持、半角/全角のモデル誤りは別件。
status: |-
  **ドラフトv3(opus round2で収束、Blocker 0)**。round1のBlocker3件・Must-fix5件、round2のMust-fix2件・Should-fix3件を反映。実装可(下記の前提ブランチ待ち)。
  前提ブランチ: ADR-179/184(`feat/adr178-mode-key-actuation-and-tsfnative-rescue-teardown`の未追跡/未マージ
  ファイル)とADR-185(`feat/adr185-directinput-open-axis-write`)は本ブランチ(developの先端)に存在しない。
  実装は上記ブランチのマージ後に行う。
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

## 決定

**決定1 — 実測表を一次情報として固定する。** 本ADRと`186-measurements/`を、ATOKプリセットの動作の根拠とする。
「全セル一致」は撤回し、状態をComposition/Conversionに分けた表(上)を正とする。

**決定2 — 無変換/変換(入力なし)の押下時点のbelief追随は、既存のdelegate-to-open-axis経路を使う。**
実体は`src/engine/nicola_fsm.rs`の`resolve_pending_thumb_as_single`内の`special.delegate_to_open_axis`分岐
(Toggleのとき、awaseが自分のbeliefに従って明示ON/OFFをactuate。合成送出なし、物理キーは`Decision::Consume`
で中継されないため二重actuationにならない)。到達に必要なのは、**`gji_thumb_key_ime_toggle = true`(opt-in)**
と、**無変換/変換が親指キーとして設定されていること**。この経路はPassthrough(優先順位4)より優先(3)なので、
発火する状況では「Passthrough設定」は効かなくなる(入力中を除く)。ATOKは無変換と変換の両方をToggleにする。

さらに決定2に含める: **ATOKプリセットのこの経路では、open遷移時のeisu reset(`PostSetOpenEisuReset`、
`eisu_reset_on_ime_on`)を抑止する**(ObservedEisuを消さない)。convは開閉をまたいで保存されるため、
直接入力(半角英数のまま)から無変換でONにしたとき、resetするとEngine ONのまま実IMEが半角英数になり、
要件の真逆になる。これは既存分岐への条件追加であり、新しい型・フィールドは足さない。
**入れ場所は`crates/awase-windows/src/runtime/key_pipeline.rs`の`eisu_reset_on_ime_on(applied && new_ime_on, ..)`呼び出し(1911-1915付近)の1箇所**。
条件は「このイベントが無変換/変換(`VK_NONCONVERT`/`VK_CONVERT`)の修飾なし単独タップのとき」(同スコープの
`event.vk_code`と`event.modifier_snapshot`で判定できる。IME-ONコンボのCtrl+変換とは区別できる)。
**「GJI/ATOKなら常に抑止」にしてはならない**: `eisu_reset_on_ime_on`はデッドロック解除でもあり
(`state/eisu_recovery.rs:3-16`)、Imm32Unavailableアプリ(Chrome/Edge)ではObservedEisuを訂正する観測経路が無く、
一律に抑止するとEngineが永久にinactiveになる既知バグ(2026-07-06 MS Edge)を再発させる。`SetOpenOrigin`での区別も不可
(delegateもIME-ON/OFFコンボも同じ`ExplicitUserAction`で、区別には新variantが要る)。
抑止した後の脱出口: stale なObservedEisuに嵌ったときは、Ctrl+変換(IME-ONコンボ)が従来どおりresetする。
`state/eisu_recovery.rs`の対応表と`tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset`
にGJI/ATOK例外を明記する。

**決定3 — かな英数トグル(ひらがな、Shift+無変換)の押下時点予測反転は、保留する。**
物理0xF2がGJIに届くか未確認(`transport.rs:197-206`、GJI戦略で`f2_warmup_owned`のとき物理キーはSuppress)。
届かなければ、予測反転は常に外れる。実機のdebugログで、ひらがなキー押下時にGJIのconvが実際に変わるかを
確認してから決める。実装する場合も、新variantは足さず、既存の`InputModeApplyStrategy::
UserHalfWidthAlnumToggle`(openを動かさずkana⇔eisuのbeliefだけを書く)を再利用し、既存の片方向
`UserTurnOnEisuReset`を**置き換える**(併存させない)。

**決定4 — 入力中の無変換は、現状維持とする。** `resolve_pending_thumb_as_single`は`composing`を引数に取り、
delegateはcomposing中に発火しないfail-closedになっている(誤ってfalseでToggle→OFFすると、composition
を破棄する)。この保護は外さない。入力中の無変換は、ModeKeyConfig(Passthrough/Suppress)に委ねる。
入力中の効果(ONのまま半角英数)へのEngine追随は、決定3と同じ遅延観測のまま。

**決定5 — 半角/全角(0xF3/0xF4)のモデル誤りは別件として切り出す。** GJI使用時は0xF3/0xF4とも
`CancelAndIMEOff`/`IMEOn`のトグル(`atok.tsv`の全状態、`keyevent_handler.cc:315-316`)。awaseは
0xF3=TurnOff、0xF4=TurnOnとしており、物理キーはGJI時に常にSuppressされる(`transport.rs:405-418`)ため、
0xF4が届いたときはno-opのままになる。IME種別依存なので`vk.rs`の静的表では表現できない。修正箇所
(`vk.rs::shadow_effect`か`transport.rs`か)は別のBUG/ADRで決める。本ADRでは変更しない。

## 非決定(やらないこと)

- 新しい型・軸の区別・`InputModeApplyStrategy`の新variantは作らない。
- 英数キー・カタカナ・MS-IMEなど他プリセットは、未測定のため対象外。
- ADR-179のマージ前TODO(Passthrough実験の撤去)とは衝突しない(決定2はPassthroughを前提としない)。

## 期待される結果と残る限界(決定2のみ実装した場合)

| ユーザー操作 | ATOK実動作 | awaseの結果 |
|---|---|---|
| かな・入力なしで無変換/変換 | IME OFF | belief OFF→Engine OFF(押下時点)。**満たす** |
| 半角英数ON・入力なしで無変換/変換 | IME OFF | 同上。**満たす** |
| 直接入力で無変換/変換 | IME ON(convは直前値を復元) | belief ON。**ObservedEisuが立っていれば**Engine OFF(満たす)。立っていない場合(直前にひらがなキー/Shift+無変換で半角英数に入った等、決定3を保留したため beliefが`AssumedRomaji`のまま)は、抑止してもEngine ONになり**満たさない**(実IMEは半角英数ON)。今日は無変換でbelief openが動かずEngine OFFのままなので、このケースだけ**退行**。 |
| 入力中(Composition)で無変換 | ONのまま半角英数 | delegate発火せず。遅延観測まで満たさない(既知) |
| ひらがなキー | かな⇔半角英数 | 満たさない(決定3を保留)。遅延観測のまま |
| 半角/全角 | ON/OFFトグル | 0xF3は正しい。0xF4はno-op(決定5) |
| フォーカス遷移直後(settling中) | — | `SetOpen`が2段フィルタ(`executor.rs:156-172`、`platform_state.rs:289-309`)で落ち、物理キーも`Decision::Consume`で中継されないため、**誰も切り替えない完全な空振り**。今日は生キーがGJIに届くので、退行 |
| belief誤予測時 | — | Toggleなので**逆方向へactuate**。訂正はTsfNative限定(`idle_check.rs`のガード2)で、非TsfNativeでは訂正が来ない。TsfNativeでも、次に500ms以上手が止まり、最後の明示IME操作から1500ms経過するまで続く |

## リスク(BUG-115が挙げた却下理由と本ADRの実測の関係)

`src/config.rs:410-422`の却下理由4点のうち、本ADRで潰せたのは「4. GJIが本家`atok.tsv`と一致する保証がない」だけ。
残る「1. Toggleの非冪等性」「2. 親指キー2本への露出倍増」は、opt-in(`gji_thumb_key_ime_toggle`)で緩和するが
残る。特に`config.rs:452-455`が警告する「TSFネイティブアプリ(`FeedbackPolicy::Blind`)では実IME状態を読み戻せない
ため、beliefがズレると逆方向へ切り替わる」は、決定2の中心的リスクで、ユーザー原則「IME ON/OFFは安定して観測
できない」と直結する。実機A/Bでbeliefのズレを確認すること。
また、無変換/変換の単独タップごとに`record_explicit_intent`(`UserIntentSource::Command`)が走り、
`EXPLICIT_ON_INTENT_TTL_MS = 10_000`(`tuning.rs:467`)の間、open意図がIntentStoreに固定されてdrift correctionより
優先される(今日はPassthroughのため記録されない新しい露出)。親指キーの単独タップは日常的に起きる。

## 検証計画

1. 実機A/B(メモ帳・Windows Terminal、awase起動・debug、`gji_thumb_key_ime_toggle = true`):
   無変換/変換の押下から`Engine activated/deactivated`までの時間を、ログの押下時刻で測る(現状=次の打鍵後)。
   直接入力(半角英数のまま)からの無変換で、Engineが誤ってONにならないこと。
   **加えて、ひらがなキー(またはShift+無変換)で半角英数にした直後に、無変換を2回押して(OFF→ON)も、
   Engineが誤ってONにならないこと**(決定3を保留したことで生じる退行窓。再現したら、決定2を
   **ON→OFF方向だけ**に限定する: beliefがONのときだけToggleを採用し、OFFのときは今日どおり生キーをGJIに通す。
   Engineの安全上重要な方向は「英数のときOFF」で、「かなのときON」の失敗は1打の遅延に留まるため)。
2. ゴールデン/ユニットテスト: 表の「入力なし」セルについて期待するbelief遷移を固定。eisu reset抑止の
   回帰テスト(`architecture_guard::user_ime_on_paths_are_paired_with_eisu_reset`は`write_*(`の出現数を数えるだけで
   eisu resetの条件変更を検出しないため、**この回帰テストが唯一の保護**)(`fix-requires-evidence`の再発ファミリー: IME belief/キー選択に該当するため必須)。
3. 決定3の前提確認: ひらがなキー押下時、GJIのconvが実際に変わるかをdebugログで確認する。

## 未解決事項

- メモ帳・Windows Terminal(TSFネイティブ)での実測(スパイクはRichEditまで)。
- 直接入力×0xF3の有効な測定(現状は0件)、ON・半角英数×半角/全角、Conversion×ひらがなの由来(Mozcか、OS/DBEか)。
- 決定2の到達条件(無変換/変換が親指キーとして設定されている前提)が、ユーザーの現在の設定で満たされるか。
