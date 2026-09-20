---
id: ADR-190
title: |-
  Microsoft IMEでImmCross(WM_IME_CONTROL)が失敗したとき、非冪等なVK_KANJIトグルではなく冪等なVK_IME_ON/OFF(MsImeDirect)へフォールバックする
summary: |-
  CI実機E2E(`sc-*`)で、Microsoft IME本体(Win32 Edit、ImmCrossプロファイル)にawaseを起動すると、直接入力からの最初のひらがなキーでIMEが開かず
  Engineだけ ON になる(実IME OFF + Engine ON、3/3)。awaseなしなら開く。原因: (1)MS-IME本体への最初のImmCross set-open(0x0006)がCIで約150msの
  タイムアウトで`success=false`、(2)`imm_cross_write`は事後読み取りが`None`(不明)でも`Failed`にする、(3)`ImmCross × MsIme`のチェーンは
  `[ImmCross, KanjiToggle]`(ADR-089)で、物理F2(ImmCrossでもAllowされOSに届く)が既に開けたIMEを、awaseの非冪等なVK_KANJIが閉じる(BUG-46型の二重actuation)。
  検証: KanjiToggle撤去(a8)で各3/3 ALL PASS、MsImeDirectへ差し替え(a9)で実IMEが全手順で正しい。決定: `ImmCross × MsIme`のチェーンを
  `[ImmCross, MsImeDirect]`にし、述語を`kind==MsIme`だけにする(同時にしか入れられない)。`KanjiToggle`は到達不能になり、ATOK等をMS-IMEと誤推定した環境の
  「Win32 Edit × ImmCross失敗」時のフォールバックは無くなる(受容、削除は実機確認後の別ADR)。
status: |-
  **ドラフト v2(未実装)**。opus round1(Blocker2/Must-fix4/Should-fix6)を反映。CI検証済み(a8: run 35515406371、a9: run 35516320434)。実機(dragonflyg4)・ATOK未検証。
related_adr:
  - "ADR-063"
  - "ADR-089"
  - "ADR-117"
  - "ADR-186"
  - "ADR-189"
---

# ADR-190: MS-IMEのImmCross失敗後は冪等なVK_IME_ON/OFFへフォールバックする

関連: [BUG-152](../known-bugs/BUG-152.md)、レビュー: [round1](190-opus-review-round1.md)。

## 背景と症状

CI実機E2E(`.github/workflows/e2e-ime.yml`の`sc-*`構成、Microsoft IME本体+awase)で、直接入力からの**最初のひらがなキー(0xF2)**を押すと、実IMEは閉じたまま
(`open=0`)なのにawaseはEngine ON(`Engine activated`)になり、続く`k`がNICOLAのかな(`き`)として直接入力に出た(`sc-dbe/kanji/shift-msime-native`各3/3)。
awaseなし(`sc-*-noawase`)では同じキー列でF2はIMEを開き、F0/F3/F4もトグルする。

## 原因(ログとablationで確定)

1. Microsoft IME本体への**最初のImmCross set-open**(`WM_IME_CONTROL 0x0006`)が、CIランナーで約150msの`SendMessageTimeout`に収まらず
   `set_ime_open_for_target ... success=false send_elapsed=154ms`(`slow IMM call: 156ms`)。同じ環境でも2回目以降は成功する(`sc-hz-msime-native-suppress`の
   step5: `success=true send_elapsed=12ms`)。
2. `runtime/open_chain.rs::imm_cross_write`は`Failed`のとき`read_ime_state_fast().ime_on`を読み直すが、これも失敗して`None`になる。
   **`Some(open)`のときだけ`AlreadyMatched`、`None`(不明)も`Failed`**として次の機構へ落とす(`open_chain.rs:353-374`)。
   `fallback_write`のdoc(`:446-449`)は「`Failed`は実際に確認した場合だけ」と書き、実装と食い違う。
3. `ImmCross × MsIme`のチェーンは`[ImmCross, KanjiToggle]`(`state/app_ime_policy.rs:65`の`CHAIN_IMM_CROSS_THEN_KANJI`、ADR-089 §2.8)。
   `ms_ime_direct_applicable`(`state/key_sequence_policy.rs:60`)が`!can_use_imm32_cross_process()`を要求するので`MsImeDirect`は不適用
   (`fallback_write: mechanism=MsImeDirect not applicable → Failed`)、`KanjiToggle`が非冪等な`VK_KANJI`を送る。
4. **物理ひらがなキーはImmCrossプロファイルでもOSに届く**(`PhysicalKeyDisposition::plan`のF2分岐は`is_tsf_mode && f2_warmup_owned`のときだけSuppress、
   `transport.rs:279`付近。CIでは`is_tsf_mode=false`でAllow、`[reinject] vk=0xf2`)。よってMS-IME自身がF2でIMEを開き、awaseのVK_KANJIがそれを閉じる。
   本質は**物理キーとawaseの両方がactuateするBUG-46型の二重actuation**で、2本目が「非冪等キー」なので衝突が実害になる。

検証(CI実機、各3回):

| 実験 | 結果 | 限界 |
|---|---|---|
| 再注入の`scan=0`が原因か(`sc-probe-vk-msime-native`) | **否定**。scan=0のF2でもMS-IME本体は開く | — |
| a8: `fallback_write`でKanjiToggleを送らない | `sc-dbe`/`sc-kanji-msime-native` 各3/3 ALL PASS | **物理F2が既にIMEを開けていたので通った**(awaseが何も送らなくても開く場面) |
| a9: `ms_ime_direct_applicable`の`!can_use_imm32_cross_process()`を外す | `sc-dbe`/`sc-shift`各3/3 ALL PASS、`sc-kanji`は実IMEが全手順で正しい | **`VK_IME_ON`が閉じたIMEを開けることの証明ではない**(step1で送った時点で物理F2により既に開いていた可能性が高い)。冪等キーが開いたIMEを壊さない、は示せる |

a9の`sc-kanji`2/3のFAILは判定窓の問題(実IMEは正しい): step1の押下から最初の`k`までが常に+1.1〜1.6sかかり、まれに+3.7s/+4.6sに跳ねる
(run2: +3.72s、run3: +4.63s、いずれも`decision=Consume`=Engine ON)。`check_consistency.engine_after`の判定窓+2500msに元々マージンが薄い。
なお`Unwarranted`件数(2件)は起動直後(`elapsed_ms=197`)のもので手順と無関係。

`MsImeDirect`は`VK_IME_ON`(0x16)/`VK_IME_OFF`(0x1A)を`SendInput`する冪等キーで`conv`を変えない(`ime_controller.rs`の`MsImeDirectStrategy`のdoc、2026-08-06〜)。
ADR-063の「`VK_DBE_HIRAGANA`/`VK_DBE_ALPHANUMERIC`」の記述と`ime_controller.rs`冒頭のモジュールdocの「冪等VK_DBE_*」は古い。
Chromeが`VK_IME_ON/OFF`を受け付けない(`docs/experiments.md` 2026-05-22)のはChrome×GJIの話で、Chromeは`Imm32Unavailable`として今日既に
`[MsImeDirect]`=`VK_IME_ON/OFF`を使っている(`app_ime_policy.rs`)ので本件の反証にならない。

## なぜ`MsImeDirect`が今まで入っていなかったか

ADR-089 §2.8の表は「`ImmCross × MsIme`に`MsImeDirect`を足すと、現行が到達しない経路を新設することになる」を理由に`KanjiToggle`を置いた。
これは**当時の実装の書き写し**で、「`VK_IME_ON`がImmCrossプロファイルで危険」という実測ではない(この組で試した記録は`docs/experiments.md`にも無い)。
ADR-089自身が「`KanjiToggle`が到達するのは`ImmCross × MsIme`の1組だけ」と明記していた——今回のバグはその1組そのもの。

## 決定

1. **`ImmCross × MsIme`(Standard/Plain/Unknown)のチェーンを`[ImmCross, MsImeDirect]`にする**(`CHAIN_IMM_CROSS_THEN_KANJI`→`CHAIN_IMM_CROSS_THEN_MS_IME`)。
2. **`ms_ime_direct_applicable`を`kind==MsIme`だけで判定する**(`!can_use_imm32_cross_process()`を外す)。引数`profile`は未使用になるが、`transport.rs:386`
   (物理キーSuppress/Allow、BUG-46/52/116ファミリー)に差分を出さないため**シグネチャは残し`_profile`にする**(`#[track_caller]`も残す)。削除は決定3の後続で。
   **決定1と2はセットでしか入らない**: 非同期チェーン(`run_open_chain_async`)は`caps(p,k).chain`を使わず`WriteMechanism::ALL`を走査して`is_applicable`を再評価する
   (`open_chain.rs`モジュールdoc、`chain_len=4`)ので効くのは決定2だけ。同期チェーン(`ImeController::apply`→`caps_chain_for`→`run_chain`)は
   チェーン定数を使うので決定1が要る。片方だけだと`caps_chain_matches_legacy_all_scan`(`ime_controller.rs:1006`)が落ちる(ALL走査とcapsの不一致を検出する安全網)。
   `transport.rs:386`は`can_use_imm32_cross_process()`が真の腕を先に処理する`else`内なので判定結果は変わらない(検証済み)。
3. **`KanjiToggle`は到達不能になる。これを受容する**(削除はしない)。`ImeKindId`は`Gji`/`MsIme`の2値のみで、決定1・2の後は同期(chainに現れない)・非同期
   (`GjiDirect`/`MsImeDirect`が必ずapplicableなので`Failed`にならず、`KanjiToggle`の腕に入らない)のどちらでも到達しない。**フォールバックが消える影響**:
   ATOK等(`ActiveImeKind`はGJI非検出=MS-IMEと*推定*)が**Win32 Edit(Standard)でImmCrossがタイムアウトしたとき**、今日届いている`VK_KANJI`が届かなくなる。
   ATOKが`Imm32Unavailable`/`TsfNative`のアプリを使う場合は今日既に`[MsImeDirect]`なので退行面はこの1組に限る。実機でATOK+`VK_IME_ON/OFF`を確認するまでドラフトのままにする。
   `KanjiToggleStrategy`/`WriteMechanism::KanjiToggle`/`PostKanjiToggle`/`post_kanji_toggle_to_focused`/`architecture_guard.rs`のガードの**削除は別ADR**
   (到達不能になったことを実機で確認した後)。この変更では**古くなるdocだけ直す**(下記)。
4. **`imm_cross_write`の`None`=`Failed`は変えない**。`MsImeDirect`は冪等なので、不明を「開いていない」と扱っても逆転しない。`fallback_write`のdocは実装に合わせて直す。
5. **ROMAN補完の挙動差分を受容する(実測を残す)。** `apply_mechanism`は先頭で`romaji_pre_write`を呼び、`decide_needs_romaji_pre_write`は
   `open && {ImmCross, MsImeDirect} && kind==MsIme && belief!=ObservedKana`で真。変更前のfallback(`KanjiToggle`)では偽だったが、変更後(`MsImeDirect`)は真になり、
   `SendMessageTimeout`ベースの同期ブロッキング往復が`with_app`を握ったまま走る。a9ログの実測: `VK_IME_ON`の送信が**62.7ms**遅れ、`ROMAN 補完 Failed`
   (`res6/result-sc-kanji-msime-native-a9-2`)。ImmCrossが既に約150msブロックした直後の稀な失敗経路への追加で、最悪でも`SendMessageTimeout`の上限(~150ms)。
   別の抑止(`DecisionSite`で分岐)は型・分岐を足すので採らず、実測値を記録して受容する。
6. **`check_consistency.py`の判定窓(`engine_after`の+2500ms)を本変更と同時に広げる**(step1が+1.1〜1.6s、まれに+4.6s)。「別途」にしない。

## 検討して採らなかった案

- **a8: `KanjiToggle`を送らない(`None`のときは何もしない)。** 止血としては十分に見える(3/3 ALL PASS)が、それは**物理F2がIMEを開けていた**から。
  物理キーの無い経路(engineの判断起点のopen、shadow-toggleのOFF)ではImmCrossが失敗したとき開閉する手段が無くなる。冪等キーが使える以上、送らない理由が無い。
- **`imm_cross_write`の`None`を`Failed`と区別する新しいoutcomeを足す。** 型・分岐が増える(ADR-184の教訓)。冪等キーなら`None`の扱いを変えずに解決する。
- **ImmCrossのタイムアウトを延ばす。** CIランナーの遅さへの対症(tuning-constants規約に反する)。問題は「失敗を非冪等キーで補う」こと。
- **`Standard × MsIme`ではawaseがactuateせず観測に追随する(follow方式、ADR-186/187)。** 機構を1つ減らす方向で削除量の観点では最も望ましく、a8の3/3ALL PASSが
  実現可能性の証拠でもある。今回採らない理由: engine起点のopen(フォーカス直後のforce-on、drift補正)には対応する物理キーが無く追随できない。
  将来、物理キー起点のactuationを畳む別ADRの候補として残す。
- **`transport.rs`の物理F2 Allowを変える。** ImmCrossでもF2がAllowされる(`plan`のdocは「ImmCross: KANJI関連キーはDown/Up共にSuppress」と書くが、F2は早期returnで例外)。
  Suppressに変えるとBUG-116ファミリーの再発リスク。今回は**docの食い違いだけ直す**。

## 影響範囲(再発ファミリー)と対象外

`fix-requires-evidence.md`の「キー選択」「IME actuation合流点」ファミリー(`ime_controller.rs`、`runtime/open_chain.rs`、`state/app_ime_policy.rs`、`state/key_sequence_policy.rs`)。
- **対象**: 同期チェーン(`ImeController::apply`)と非同期チェーン(`fallback_write`)。
- **対象外(明記)**: `runtime/ime_refresh.rs::ir_apply_drift_correction`→`platform.rs::set_ime_open`。チェーンを通らず、`can_use_imm32_cross_process()`が偽なら`false`を
  返すだけで**非冪等キーは送らない**(ImmCrossが失敗してもfire-and-forgetで代替機構へ落ちない)。CIログでも並走して失敗していたが、本バグの原因ではない。
- **Winキー押下中の挙動差分**: `MsImeDirect`は`send_ime_mode_key`失敗(Winキー押下中)で`UnsafeToToggle`を返し、`falls_through`が偽なのでチェーンは止まる。
  変更前の`KanjiToggle`は`post_kanji_toggle_to_focused`を無条件に送っていた。「Winキー押下中は今まで`VK_KANJI`が飛んでいたが、今後は何も飛ばない」(安全側)。

## `MsImeDirect`がImmCrossプロファイルで成り立つことの根拠(コードから確認済み)

- `decide_attempt`は`MsImeDirect`で`shadow_on`を参照せず無条件に`SendVk`(`ime_actuation_decision.rs`)。`fallback_write`の`shadow_on=None`上書き(BUG-113)は無害
  (`architecture_guard.rs`のコメントが既に明言)。`decide_gate`のInputRelay判定は機構に依らず`fallback_write`冒頭で効く。
- `VK_IME_ON`はopen軸のみで`conv`を触らない。ROMAN補完は`set_ime_romaji_mode_for_hwnd`が`conv | IME_CMODE_ROMAN`のread-modify-writeで`KATAKANA`ビットを落とさず、
  `belief==ObservedKana`のときはそもそも発火しない。よって「開いているカタカナのIMEを壊さないか」は実機確認項目ではなく既知事実。実機で確認するのは決定5のレイテンシ。

## 更新するもの(この変更と同じPRで)

- テスト/golden: `crates/awase-windows/tests/golden/ime_key_sequences.txt`(`MS-IME	Standard	async_fallback	KanjiToggle`→`MsImeDirect`、本文の「`!can_use_imm32_cross_process()`」説明、
  KanjiToggle節の「稀にしか到達しない」)、`tests/ime_key_sequence_golden.rs:211-215`(**`#![cfg(windows)]`で、Linuxのtestジョブでは0 tests。更新漏れはwindows-build CIまで気付けない**)、
  `state/key_sequence_policy.rs:205-222`の4アサーション、`state/app_ime_policy.rs`の`caps_chains_match_the_adr089_table`と定数名、`ime_controller.rs:1006`の`caps_chain_matches_legacy_all_scan`。
- 古くなるdoc: `ime_controller.rs`冒頭(13-14/24-28行)、`KanjiToggleStrategy`のdoc、`open_chain.rs`の`fallback_write`のdoc(`:434-437`、`:446-449`、`:465-468`)、
  `app_ime_policy.rs:60-62`、`focus/class_names.rs`の`uses_kanji_toggle`のdoc、`architecture_guard.rs`の「生きている`post_kanji_toggle_to_focused`」、
  `transport.rs`の`plan`doc(ImmCross×F2の例外)。
- ADR-089 §2.8: 表と「入れない理由」節は**削除せず「2026-09-20、BUG-152により覆した。当時の理由は実測ではなく実装の書き写しだった」と経緯を残す**(追記済み)。
- CI: `sc-dbe/kanji/shift-msime-native`を`observe`→`pass`、`check_consistency.py`の判定窓。

## 検証計画

- 回帰テスト: 上記のgolden/単体テスト(`ImmCross × MsIme`のImmCross失敗後が`MsImeDirect`)。
- CI実機E2E: 本変更のビルドで`sc-dbe/kanji/shift-msime-native`が各3/3 PASS。`sc-dbe-msime-native-noawase`との一致。
- 実機(dragonflyg4、Microsoft IME): (a)**物理キーを伴わないopen**(engine起点)でImmCrossを失敗させ、`VK_IME_ON`だけで開くか(a9が示せなかった点)、
  (b)決定5のレイテンシ、(c)ATOK等で`VK_IME_ON/OFF`が効くか(決定3の受容の妥当性)。

## 残る限界

- MS-IME本体の半角/全角(0xF3/0xF4)は静的モデル(F3=OFF、F4=ON)のままで、同じVKの連続で反転しない(ADR-189が対象外にした範囲、別件)。
- `KanjiToggle`関連コードは到達不能のまま残る(削除は別ADR)。
