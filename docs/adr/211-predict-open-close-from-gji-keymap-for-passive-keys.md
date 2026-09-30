---
id: ADR-211
title: |-
  GJI の MS-IME/MOBILE プリセットの F13(閉状態からだけ IME を開く受動のキー)で、Engine が追随するよう打鍵時に「開く」と予測する。一般化(キーマップ全体からの予測)は需要が確認できるまで見送る
summary: |-
  GitHub Actions(windows-latest、GJI、実 Chrome=読めない窓、awase あり、run 36701610898、`sc-follow-chrome-*`、1構成2回)で、IME OFF から各キーを1回押したとき、
  MS-IME プリセットの F13 だけが「IME は開くが Engine が追随しない(`か`)」になった。F13 は Mozc の `ms-ime.tsv`/`mobile.tsv` で DirectInput の `IMEOn`(閉状態からだけ開き、
  開状態の行は無い)なので、ADR-199 のトグルの役割ではなく受動になり、(a)予測の表(`TableKey`、13キー)にも(b)通過マーク(`is_followed_mode_key`)にも無く、追随する手段が無い。
  Opus round1(2026-09-30)が Mozc の全文 TSV(`b4bbc42f`)で確認した結果、4プリセットで「受動で、表の13キーの外で、閉状態から開く」キーは **MS-IME/MOBILE の F13 だけ**
  (ATOK・KOTOERI には無い。英数・ひらがな・カタカナ・漢字・変換・無変換・半角/全角・`ON` は全て表のキー)。当初案(キーマップ全体の評価)は、CUSTOM 表だけで効く一般化で、
  設計上の穴が多く(`predict_with_override` のガードの後ろに置くと届かない、トグル形なのに能動でないキーが取りこぼされる、一部の開状態だけ Close のキーで偽 ON になる、ほか)、
  所有者の実機は MS-IME プリセット(古い表つき)で CUSTOM ではないため、需要が確認できていない。よって本 ADR は **F13 の1規則+通過マーク** に範囲を絞る。
status: |-
  草案 v2(2026-09-30。Opus round1 の指摘を受け、範囲を F13 の1規則に縮小)。実装は未着手。次に Opus round2(同じレビュアーへの再確認)。
related_adr:
  - "ADR-186"
  - "ADR-191"
  - "ADR-195"
  - "ADR-196"
  - "ADR-199"
  - "ADR-205"
  - "ADR-206"
  - "ADR-209"
---

# ADR-211: MS-IME/MOBILE プリセットの F13(閉状態からだけ開く受動のキー)への追随

## 経緯(v1 の縮小)
v1 は「GJI の実効キーマップから、受動のキーの開閉を一般に予測する」だった。Opus round1 が、(B1)決定4の順序で `predict_with_override` に差し込むと、
CUSTOM のキー行を理由にした既存のガード(`key_effect_predictor.rs` の `custom_table_overrides` の打ち切り)が先に `None` を返すので「閉じる」は全構成で届かず CUSTOM の「開く」も届かないこと、
(B2)トグル形なのに能動でないキー(候補外のキー・ON/OFF 行が欠けた表・自動リピート・`is_japanese_ime` が偽のとき)がどの経路にも乗らないこと、(M3)一部の開状態だけ Close のキーで偽 OFF を偽 ON に入れ替えること、
(M4)`CompositionModeHalfAlphanumeric` で開くのにモードを予測しないと Engine がかなで活性化すること、ほかを指摘した。
さらに4プリセットの TSV では、この一般化が**新しく動かすキーは F13(MS-IME/MOBILE)だけ**(下の実測と TSV の突き合わせ、Mozc `b4bbc42f` の全文)で、残りは CUSTOM 利用者のためだけにある。
本 ADR は範囲を縮め、CUSTOM 等の一般化は「再開の条件」(下)が満たされたときの設計上の制約として、Opus の指摘とともに記録するだけにする。

## 背景(事実)
**所有者の要望(2026-09-30)**: 受動ポリシーのモードずれは許容する。ただし GJI・MS-IME の設定で「内部状態によらず IME ON」や「トグル」になっているキーは、受動で動作しつつ、できる限り Engine が追随するようにする。

### 実測(GitHub Actions、実 Chrome=読めない窓、GJI、awase あり、IME OFF から各キー1回。run 36701610898、`sc-follow-chrome-*`、1構成2回)
| キー | ATOK プリセット | MS-IME プリセット |
|---|---|---|
| 半角/全角、`VK_IME_ON` | 開いて追随 | 開いて追随 |
| 変換 | 開いて追随 | 開いて追随(ADR-209) |
| 無変換 | 開いて追随 | 開かない |
| ひらがな、英数 | 開かない | 開いて追随 |
| **F13** | 開かない | **IME は開くが Engine は追随しない(`か`)** |
| F14 | 開かない | 開かない |

### Mozc の TSV との突き合わせ(Opus round1 が全文で確認、私も再取得して DirectInput の行を確認)
DirectInput で `IMEOn` の行: ms-ime・mobile = Eisu・F13・Hankaku/Zenkaku・Hiragana・Kanji・Katakana・ON(`Henkan` は `Reconvert`)、atok = Hankaku/Zenkaku・Henkan・Kanji・Muhenkan・ON(`Shift Henkan` は `Reconvert`)、
kotoeri = Hankaku/Zenkaku・Kanji・ON。F14 はどのプリセットにも行が無い。実測表と TSV は、**MS-IME の変換(TSV=`Reconvert`、実測=開く。ADR-209)を除いて全て一致する**。
表のキー(`TableKey`)でないのは、この中では **F13 だけ**。

### 現状の追随手段(コードで確認)
1. **能動**(awase が書く): ADR-199 のトグルの役割のキー(半角/全角・F13〜F24・無変換/変換の単独タップ)と `keys.ime_on/off`。トグルは「DirectInput が Open かつ全ての開状態が Close」(決定4・決定11)。
2. **受動+表の予測**(打鍵時): `TableKey` の13キーだけ(学習/同梱の表は素の EDIT で測ったので TSF の窓で誤りうる。ADR-209)。
3. **受動+観測**: 通過マークのキー(`is_followed_mode_key`=`is_ime_mode_key_for_ime` から `VK_IME_ON/OFF`(0x16/0x1A)を除いたもの。0x15・0x17〜0x19・0xF0〜0xF6・変換/無変換)の後に IME を読み直す。
   読める窓ではこれで追随する。TsfNative には観測の手段が無い。Imm32Unavailable×GJI(実 Chrome)では、ADR-205 の watch(prefetch 済みの読みの変化を拾う)があるが、**外部注入のキーにしか立たない**(ADR-205 の将来課題に「物理キーにも広げる」がある)。

F13 は 1・2・3 のどれにも当たらない: MS-IME/MOBILE プリセットの F13 は DirectInput の `IMEOn` だけで開状態の行が無い(トグルでなく受動)、表に無い、通過マークの集合にも無い。

## 決定

1. **F13 の1規則を足す**(ADR-209 と同じ形の、純関数の中の規則)。次の全てを満たす打鍵は「IME を開く」と予測する(開閉だけ。モード・段階は予測しない。belief が `Unknown` のときだけ既存の種 `kana_mode()`。`track` はそのまま返す):
   - GJI(`config1.db` が読める)で、`session_keymap` が MS-IME(2)または MOBILE(4)、または不在/NONE(既定=MS-IME 相当)。
   - 閉状態の belief。無修飾の F13(0x7C)。overlay が無い。
   - 規則は**データで持つ**: 「プリセットごとの、表の外で DirectInput が `IMEOn` の受動のキー」の小さな定数表(現状は MS-IME/MOBILE に F13 の1行)。テストは、`role.rs` のテストと同じく **コミットハッシュ(`b4bbc42f`)を固定した TSV の抜粋**(Eisu・F13・Hiragana・Katakana・Hankaku/Zenkaku・Kanji・Henkan・Muhenkan・ON の DirectInput の行と、F13 の全状態の行を含む)から作った期待値と定数表が一致することを固定する。新しいプリセットや Mozc の更新で行が増えたら、テストが落ちる。
2. **新しいゲートを明示する**(`kp_stage_key_effect_track` の除外は今 `in_table &&` のときだけで、F13〜F24 のような表に無いキーには効かない。決定1を入れる前にこのゲートを足さないと、
   役割由来の Toggle・同期キー・エンジンが消費した打鍵でも予測が走って二重に効く。Opus M-c): 規則が当たるのは、KeyDown・**自動リピートでない(`!was_down`)**・非 injected・**`shadow_action` を持たない**・
   **`sync_direction` を持たない**・エンジンが消費していない打鍵だけ。**Shift 付きも当たらない**(`modifiers_suppress_prediction` は表に無いキーで Shift を許すので、F13 についてはこの規則の内側で Shift も止める。TSV では `Shift F13` は別の行)。
   役割はあるが awase が書かなかった打鍵(F13 の役割トグルなのに `is_japanese_ime` が偽、など)は `shadow_action` が付いたままになる。その判定は「`shadow_action` の有無」でなく「この打鍵で awase が実際に書いたか(`shadow_toggled`)」で行う。
3. **通過マークに、この規則のキー(プリセットで開くと分かっている表の外のキー=F13)を足す**(BUG-157 の揃え。ADR-205 の観測の読み直しも同時に走るので、読める窓では観測が予測を訂正する)。
   判定の関数を1つにし、立てる場所は**2つとも**同じ関数を呼ぶ: `kp_stage_mode_key_follow`(`key_pipeline.rs`)と、FSM の `SendKeys` 経由(`runtime/executor.rs`)。
   `is_followed_mode_key` 自体は広げない(`reinject_scan_code` と BUG-113 の軸と共有する関数なので。`schedule_ime_refresh(20)` の頻度増を警告するコメントもある)。**自動リピートでは立て直さない**(`!was_down`)。
   呼び出し順が `kp_stage_mode_key_follow` → `kp_stage_key_effect_track` なので、予測の結果を条件にせず、規則が当たる打鍵の条件(決定2と同じ)で立てる(順序の入れ替えをしない)。
4. **実装の前に測る(既存の不具合の可能性、Opus M-d)**: 読める窓(メモ帳/素の EDIT)× MS-IME プリセットで、明示の IME OFF の後に F13 を押すと、観測で開いたのに `desired_open=false` が残り、drift correction が閉じ直す可能性がある
   (BUG-157 と同じ経路)。CI(素の EDIT のスパイク、`--seq` に F13)で `[drift] correction` の件数を数える。確認できたら、これは本 ADR の副次ではなく既存のバグ(新しい BUG 番号、`docs/known-bugs/`)として記録し、決定3の通過マークがその修正になる。
5. **設定は増やさない**(ADR-209 の設定とは違い、規則の根拠は TSV と実測が一致した1キーで、窓の種類の誤分類に依らない。偽 ON が出たら、その時点で設定を足す)。新しいイベント・I/O・actuation の合流点・tuning 定数も作らない。
6. **効果の範囲を明記する**: 本規則が効くのは、awase が物理キーとして見る F13(QMK 等の F13 を出すキーボード、テストの注入)。**PowerToys・AutoHotkey 等の再割り当てで作られた F13 は injected**なので対象外(BUG-14。ADR-205 の watch が Imm32Unavailable×GJI では拾う)。
   親指キーに F13 を割り当てた構成は初期範囲外(同時打鍵では F13 が IME に届かない、単独タップの再注入は injected で除外される。ADR-206 が変換/無変換で決めた扱いに揃える判断を別途)。

## 検証方針
- **単体テスト**(`state/`・`awase-gji-config`、Linux で走る): 決定1の定数表と固定した TSV の抜粋の一致。規則が当たる/当たらない条件を全て(MS-IME・MOBILE・不在/NONE で当たる、ATOK・KOTOERI・CUSTOM・overlay ありで当たらない、開状態・Shift・Ctrl・リピート・`shadow_action`・`sync_direction`・消費済みで当たらない)。
  学習表がある(`override_table=Some`)とき、追跡の段階が `None` でないとき、`input_mode=Unknown` のときにも規則が届くこと(`predict_in_table` が表に無いキーで先に `Some` を返す分岐があるので、規則は `predict_with_override` の先頭に置く=ADR-209 と同じ)。
- **CI**: `sc-follow-chrome-msime-f13` が「開いて追随(`きう`)」になること(合否)。`--no-awase` の対照を同じ run に足す(IME が実際に開くこと)。古い表つき(`custom_table=true`)、Shift+F13 の負例、ATOK の F13(開かない=予測しない)も足す。
- **読める窓**(決定4): メモ帳/素の EDIT で `[drift] correction` の件数を、実装の前後で比べる。
- **実機(dragonflyg4)**: 実 Chrome・WT・メモ帳・Edge で、偽 ON が無いこと。F13 は実機に物理キーが無いので、注入(injected でない目印つき)で。

## 一般化(見送り)と、再開の条件・設計上の制約
**再開の条件**: 所有者の実機、または実利用者の config1.db で、CUSTOM(または F13 以外)に「受動の開閉キー」(トグル形でないのに閉状態から開く/開状態で閉じる)が実際に使われていることが確認できたとき
(`gji_charset_autodetect` の診断が CUSTOM の on/off/toggle を bug report に出している)。確認できていない一般化は、`feedback_verify_symptom_and_claims_before_designing` の教訓に反する。
**再開するときに守る設計上の制約**(Opus round1 が指摘した、v1 の穴):
- (B1)設定由来の規則は `predict_in_table` の表に無いキーの分岐と、表のキーでセルが無い場合の両方で開閉を合成し、`custom_table_overrides` の打ち切りより**前**に置く(CUSTOM のユーザーの割り当てを読む規則なので、ガードの理由が当たらない。ADR-195 段階4 B3 と同じ理屈)。
- (B2)判定式は状態ごとに独立した2規則にする(閉状態で DirectInput が Open→開く、開状態で6つの開状態がすべて Close→閉じる)。トグル形も両方に当たる(段階に依らないので確か)。除外は形でなく「この打鍵で awase が書いたか(`shadow_toggled`)」で決める。
- (M2)「全ての開状態」は6状態(ZeroQuerySuggestion・Suggestion・Prediction を含む。`role.rs` の `KeyStates::closes_in_all_open_states` を再利用)。
- (M3)一部の開状態だけ Close のキーは、1回目に開く予測をすると、開いた直後の2回目で偽 ON になる。開く側も予測しない(「開状態に Close の行が1つでもあるキーは対象外」)か、段階(`track.stage==None && !composing`)で判定するかの二択を決める。
- (M4)`CompositionModeHalfAlphanumeric`/`FullAlphanumeric`(旧名 `InputMode*`)は Open に数えるが、モードを予測しないと Engine がかなで活性化する。モードも予測するか、対象外にする。ADR-091 の F15〜F19 の残骸の行がある利用者で起きうる。
- (M5)13キーの除外は、キー単位でなく**セル単位**(表にその状態・キーのセルがあるときだけ表を優先し、無ければ設定由来へ落ちる)。`KeymapPreset::Custom` は同梱表が空なので、CUSTOM の利用者が最もよく書き換えるキーが対象から外れる。
- (M6)テストの「一致」は、規則が答えたセルで表と同じ答えになること(規則が沈黙するセルは問わない)。
- (M7)通過マークの条件は「予測が動かしたか」でなく「設定上、開閉を動かしうるキーを通したとき」。
- (m4)開く/閉じるで `KeyTrack.conv`/`stage` をどうするか(閉じたら `Stage::None`、開いたら conv を捨てる等)を決める。
- (m6)`awase-gji-config/src/keymap.rs::extract_ime_keys`(継承規則を持たない3つ目の評価器)と `role.rs` の規則が割れないよう、一本化するか注記する。

## 代替案
- **A: 学習表に F13〜F24 を足す**: 学習は素の EDIT(IMM32)で測るので、TSF の窓で誤る(変換で実証、ADR-209)。
- **B: `keys.ime_detect` に手で書く/設定から自動生成する**: 設定の二重管理に加え、`keys.ime_detect` は同期キーの明示意図(`IntentKind::SyncKey`)を作り、`check_drift_correction` はそれを閾値0で即時補正する。**TSV と実挙動が食い違うと awase が IME に書く**ことになり、ADR-191 決定1に反する。
- **C: 読めない窓で打鍵ごとに実 IME を読む**: 読めない(TsfNative)ので不可。
- **D: awase が F13 を能動に昇格する(Suppress して絶対指定で書く)**: ADR-199 が能動を「トグルの役割」に限った所有者決定に反する。
- **β(ADR-205 の watch を、awase が通した「設定上開閉しうる」物理キーにも立てる)**: TSV を信じずに実際の遷移で追随するので、TSV と実挙動の食い違い(変換の前例)による偽 ON が構造的に起きない。Imm32Unavailable×GJI の窓に限られ(TsfNative は対象外)、
  遷移を見るまで(〜300ms)は追随しないので直後の最初の文字を取りこぼしうる。Opus は本 ADR の規則(α)と併用を推奨した。**本 ADR では採らない**(α だけで実測で失敗した唯一のセルは直る。β は ADR-205 の適用範囲の拡張として、必要が出たら別 ADR)。

## リスク
1. **偽 ON**: TSV と実挙動の食い違い(変換が前例)。F13 は TSV(MS-IME/MOBILE の `IMEOn`)と CI の実 Chrome の実測が一致しているが、他の窓の種類(WT・UWP・設定アプリの検索欄など)は測っていない。読める窓では観測が訂正する。読めない窓の偽 ON は自動で直せない。
2. **設定の適用遅れ**: `KeymapCache::RECHECK_MS`(2秒)。config1.db の変更から awase が読み直すまで、キーマップが古い(表に無いキーでは新しい種類の誤り)。
3. **BUG-157 の退行/既存不具合**: 決定3・4。通過マークを立てる場所を1つだけ直して2つ目を忘れる(ADR-119 型)ことに注意する。
