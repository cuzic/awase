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
  `[ImmCross, MsImeDirect]`にし、述語を`kind==MsIme`だけにする(同時にしか入れられない)。`KanjiToggle`(非冪等な機構)は到達不能になるので**同じ変更で撤去する**
  (ユーザー判断: VK_IME_ON/OFFはIME種別によらず同じ挙動で常に安全。`ImeKeyKind::KanjiToggle`=物理VK_KANJIキーの分類は別物で残す)。
status: |-
  **実装済み(未マージ)・CI実機E2Eで検証済み**。opus round1〜3で収束(round3: Blocker無し)、KanjiToggle撤去をユーザー判断で決定に追加。実装はfeb49ffd(決定1〜4)・ef2d73a7(FallbackSent削除)・c2163b69(CI判定窓)。CI実機(run 35545478699): sc-dbe/sc-shift-msime-native 各3/3 PASS、sc-kanji-msime-native 3回目はスパイク側のkが+6.9s遅れて判定窓を超えた「?」だったのでチェッカーを直した(窓の上限=次の手順の押下)。PR #231のCI(windows-build含む全ジョブ)PASS、`/code-review low`の指摘4件のうち実害のある2件(--seq不正トークン、欠番符号のテスト)を修正。実機(dragonflyg4)検証済み(下記「実機検証結果」)。CI検証済み(a8: run 35515406371、a9: run 35516320434)。実機(dragonflyg4)検証済み(下記「実機検証結果」)。
related_adr:
  - "ADR-063"
  - "ADR-089"
  - "ADR-117"
  - "ADR-186"
  - "ADR-189"
---

# ADR-190: MS-IMEのImmCross失敗後は冪等なVK_IME_ON/OFFへフォールバックする

関連: [BUG-152](../known-bugs/BUG-152.md)、レビュー: [round1](190-opus-review-round1.md)、[round2](190-opus-review-round2.md)、[round3](190-opus-review-round3.md)(Blocker無し)。

## 背景と症状

CI実機E2E(`.github/workflows/e2e-ime.yml`の`sc-*`構成、Microsoft IME本体+awase)で、直接入力からの**最初のひらがなキー(0xF2)**を押すと、実IMEは閉じたまま
(`open=0`)なのにawaseはEngine ON(`Engine activated`)になり、続く`k`がNICOLAのかな(`き`)として直接入力に出た(`sc-dbe/kanji/shift-msime-native`各3/3)。
awaseなし(`sc-*-noawase`)では同じキー列でF2はIMEを開き、F0/F3/F4もトグルする。

## 原因(ログとablationで確定)

1. Microsoft IME本体への**最初のImmCross set-open**(`WM_IME_CONTROL 0x0006`)が、CIランナーで約150msの`SendMessageTimeout`に収まらず
   `set_ime_open_for_target ... success=false send_elapsed=154ms`(`slow IMM call: 156ms`)。同じ環境でも別の呼び出しは成功する(`res2/result-sc-hz-msime-native-suppress-1/dist/awase.log:872`、`open=false`のset-open: `success=true send_elapsed=12ms`)。
2. `runtime/open_chain.rs::imm_cross_write`は`Failed`のとき`read_ime_state_fast().ime_on`を読み直すが、これも失敗して`None`になる。
   **`Some(open)`のときだけ`AlreadyMatched`、`None`(不明)も`Failed`**として次の機構へ落とす(`open_chain.rs:353-374`)。
   `fallback_write`のdoc(`:446-449`)は「`Failed`は実際に確認した場合だけ」と書き、実装と食い違う。
3. `ImmCross × MsIme`のチェーンは`[ImmCross, KanjiToggle]`(`state/app_ime_policy.rs:65`の`CHAIN_IMM_CROSS_THEN_KANJI`、ADR-089 §2.8)。
   `ms_ime_direct_applicable`(`state/key_sequence_policy.rs:60`)が`!can_use_imm32_cross_process()`を要求するので`MsImeDirect`は不適用
   (`fallback_write: mechanism=MsImeDirect not applicable → Failed`)、`KanjiToggle`が非冪等な`VK_KANJI`を送る。
4. **物理ひらがなキーはImmCrossプロファイルでもOSに届く**(`PhysicalKeyDisposition::plan`のF2分岐は`is_tsf_mode && f2_warmup_owned`のときだけSuppress、
   `transport.rs:292-298`。CIでは`is_tsf_mode=false`でAllow、`[reinject] vk=0xf2`)。よってMS-IME自身がF2でIMEを開き、awaseのVK_KANJIがそれを閉じる。
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
2. **`ms_ime_direct_applicable`を`kind==MsIme`だけで判定する**(`!can_use_imm32_cross_process()`を外す)。`profile`引数は未使用になるので**引数ごと消す**
   (`#[track_caller]`も消す: この属性は`can_use_imm32_cross_process`の`actuation_choke_point`が呼び出し元を記録するためのもので、述語がそれを呼ばなくなれば伝える先が無い。
   副作用として`[actuation-record] can_use_imm32_cross_process called from ime_controller.rs:146`のログが消え、ADR-158の呼び出し元棚卸しが1件減る)。
   `transport.rs:386`は`ms_ime_direct_applicable(kind)`に直す。`.githooks/pre-push`は警告のみでブロックしないので、BUG-46/52/116ファミリーのファイルに差分が出ること自体はコストではない
   (判定結果は不変、下記の検証済み)。
   **決定1と2はセットでしか入らない**: 非同期チェーン(`run_open_chain_async`)は`caps(p,k).chain`を使わず`WriteMechanism::ALL`を走査して`is_applicable`を再評価する
   (`open_chain.rs`モジュールdoc、`chain_len=4`)ので効くのは決定2だけ。同期チェーン(`ImeController::apply`→`caps_chain_for`→`run_chain`)は
   チェーン定数を使うので決定1が要る。片方だけだと`caps_chain_matches_legacy_all_scan`(`ime_controller.rs:1006`)が落ちる(ALL走査とcapsの不一致を検出する安全網)。
   `transport.rs:386`は`can_use_imm32_cross_process()`が真の腕を先に処理する`else`内なので判定結果は変わらない(検証済み)。
3. **`WriteMechanism::KanjiToggle`(非冪等な`VK_KANJI`トグル機構)を撤去する。** 決定1・2の後は同期(chainに現れない)・非同期(`GjiDirect`/`MsImeDirect`が
   必ずapplicableなので`Failed`にならず`KanjiToggle`の腕に入らない)のどちらでも到達しない(`ImeKindId`は`Gji`/`MsIme`の2値のみ)。到達不能のまま残すと
   「保険に見えて実際は死んでいる」(opus round1 B1)ので、同じ変更で消す。**ユーザー判断(2026-09-20): `VK_IME_ON/OFF`はどんなIMEでも同じ挙動で、
   `VK_KANJI`トグルの代わりに送るのは常に安全**。したがって「ATOK等をMS-IMEと誤推定した環境でフォールバックが消える」懸念(round1 B1の帰結)は採らない。
   **撤去するもの**(実測: 該当語を含むのは`crates/`の.rs 29ファイル、`lints/`1、docs等45。コード実体の変更が要るのは13前後、残りはdocコメント):
   - 機構: `KanjiToggleStrategy`/`KANJI_STRATEGY`/`strategy_for`の腕、`WriteMechanism::KanjiToggle`(`WriteMechanism::ALL`は4→3)、`MechanismCommand::PostKanjiToggle`と
     `decide_attempt`の腕、`ime::post_kanji_toggle_to_focused`と`apply_mechanism`の腕。
   - `lints/actuation_call_guard/src/lib.rs:77`の`RESTRICTED_CALLS`(`send_input_safe`)の許可呼び出し元`"post_kanji_toggle_to_focused"`(actuation合流点の許可リストから1件減る)。
   - `ImeOpenOutcome::FallbackSent`: 唯一の生成元(`ime_controller.rs:357`の`PostKanjiToggle`の腕)が消えるので到達不能なvariantになる。**同じ変更で消す**(別コミット):
     **`runtime/message_handlers.rs`の`encode_outcome`/`decode_outcome`は`FallbackSent => 1`の行だけ消し、2〜7は動かさない(1を欠番にする)**: `decode_outcome`は数値matchでcatch-all(`other=>UnsafeToToggle`)を持つため、
     番号を詰めて片方だけ直すとコンパイルは通り、実行時に全outcomeが黙って`UnsafeToToggle`(送っていない扱い)に倒れる。`encode_decode_outcome_roundtrips_for_all_variants`(`:2090`付近)のリストから`FallbackSent`を1件削る。
     コア`src/platform.rs`(定義と`wrote_open_state`等。`should_send_accompanying_warmup`(`:249-254`)の腕削除はBUG-113/ADR-149の随伴warmupファミリーに触れるが**挙動不変**(到達不能アームの削除のみ))、`crates/awase-windows/src/platform.rs`、`runtime/message_handlers.rs`(`ImeOpenOutcome`↔u8のワイヤ符号化、プロセス内なので互換性問題なし)、
     `state/ime_event.rs`/`platform_state.rs`/`executor.rs`/`journal.rs`/`gji_direct_mechanism.rs`/`actuation_chain.rs`の`may_return_failed`表と`ALL_OUTCOMES`。
   - `state/actuation_decision_record.rs`の`MAX_WRITE_MECHANISMS`(4→3)と、境界テスト`deserialize_rejects_chain_longer_than_max_write_mechanisms`のフィクスチャ
     (`"KanjiToggle"`が未知variantになって「長さ超過」ではなく「デシリアライズ不能」でerrになり**恒真化する**ので、`["ImmCross","GjiDirect","MsImeDirect","ImmCross"]`(4件>3)に書き換える)、
     同ファイルの「残り3スロットはnull」のコメント。
   - `state/ime_profile_driver.rs`の`invariant_2_kanji_owning_profiles_do_not_lead_with_kanji_toggle`(`WriteMechanism::KanjiToggle`を参照するのでコンパイル不能、**テストごと削除**)。
     置き換えの根拠: 非冪等機構が存在しないことは`WriteMechanism`のvariant集合そのものが固定する(型で保証される)。同ファイルの「`(ImmCross, MsIme)`のchainは`[ImmCross, KanjiToggle]`」のdocも直す。
   - `state/actuation_chain.rs`のユニットテスト4本: `unsafe_to_toggle_stops_the_chain_before_kanji_toggle`(名前・chainを再定義)、`inapplicable_mechanisms_are_skipped`、
     `all_failed_yields_failed`(`[Failed; 4]`・`calls.len()==4`→3)、`mechanism_names_match_strategy_names`(4件→3件)。
   - `architecture_guard.rs`: `kanji_toggle_fallback_sends_expected_vk_codes`(`:1702-1721`)は**テストごと撤去**、`:2438-2447`の宣言存在検査の配列から`"struct KanjiToggleStrategy"`を除く
     (除かないとpanicで落ちる。`apply_mechanism(`/`fallback_write(`の件数は不変)。
   - `win32.rs:222-223`の制約「`send_ime_mode_key`と`post_kanji_toggle_to_focused`が同じマーカーを使うので区別できない」は、送信元が1つになり**制約自体が解消**する。
   - `state/actuation_chain.rs:205-210`/`:173-196`と`state/app_ime_policy.rs:44-50`の「`UnsafeToToggle`を`falls_through`に含めない理由」(非冪等な`VK_KANJI`を送る新経路になる、と書いている)は、
     守る対象が消えるので**理由を書き換える**: 「`UnsafeToToggle`は`applied_snapshot`をラッチさせないための未適用シグナルであり、次の機構へ進む根拠にならない」(BUG-16追補、`ime_controller.rs:330-341`)。
     規則(`UnsafeToToggle`はフォールスルーしない)自体は維持する。放置すると「非冪等キーはもう無いからフォールスルーしてよい」と読まれる。
   - `runtime/focus_tracking.rs:1043`のコメントは「chainが`KanjiToggle`を含むのでこのpre-syncはStandardでも必要」という**根拠**。撤去後にpre-syncが必要かを再導出してから文言を直す(文言だけ直すと根拠を失った処理が残る)。
   **残すもの(別物)**: `vk.rs`の`ImeKeyKind::KanjiToggle`(`:96`、`0x19 => Some(Self::KanjiToggle)`:127、`ShadowImeEffect::Toggle`:149=物理`VK_KANJI`キーの分類、shadow-toggleの入力側)、
   `crates/awase-settings/src/main.rs:4846`(設定GUIのdoc、無変更)、`AppImeProfile::uses_kanji_toggle()`(`platform.rs:1302`のmode-key送信スキップ判定。名前が古いだけ、改名は別件)、
   **`tests/e2e_windows.rs`の`e2e_gji_vk_kanji_toggle_hazard_interactive`/`e2e_msime_vk_kanji_toggle_hazard_interactive`**(`WriteMechanism`を使わず生のVK_KANJIを送って「非冪等」を実機で示す。
   撤去後は`VK_KANJI`が非冪等だという唯一の実行可能な証拠になるので掃除で巻き込まない)。
   **再生フィクスチャ**: `tests/journals/`に`KanjiToggle`を含むものは**確認済みで0件**(`WriteMechanism`が載るのは`ime_apply/adr108-focus-crossing-success.json`の`ImmCross`/`MsImeDirect`のみ)。
   一方`WriteMechanism`は`Serialize/Deserialize`で、`ActuationDecisionRecord`はbug reportの`journal_json`に相乗りする(ADR-095)ので、**撤去前に収集された`"KanjiToggle"`(と、`ImeOpenOutcome`も同じくserde derive(`src/platform.rs:152`)で`AttemptRecord.outcome`として載るので`"FallbackSent"`)を含む旧reportは再生できなくなる**
   (現コーパス`bug-131-report-*.json`は両方0件、確認済みで実害なし)。`#[serde(other)]`相当の受け口は型を足すので採らず、旧reportの再生は諦める。
   なお`GjiDirect`と`MsImeDirect`はどちらも`VK_IME_ON/OFF`を送る冪等キーになり、差は適用条件(GJI検出/MS-IME推定)だけになる。統合は別ADRの候補(今回はやらない)。
   **tripwire**: 「`KanjiToggle`が不要」という結論は**`ImeKindId`が2値**(`GjiDirect⟺kind==Gji`、`MsImeDirect⟺kind==MsIme`)で全(profile,kind)に少なくとも1機構がapplicableであることに依存する。
   3値目を足すと、非同期チェーンで適用可能な機構が無く無音で`Failed`になる。`ImeKindId::ALL`のテスト(`caps_chains_match_the_adr089_table`)に落ちる先を残す。
   **前提が外れた場合の検出口**: 「`VK_IME_ON/OFF`はIME種別によらず効く」が外れると、症状はBUG-152と同型(ImmCrossが失敗したときだけIMEが開かない/Engineだけ ON)で再来し、
   ログ上の差は`[apply-ime] MS-IME direct: send 0x0016`の後に実IMEが開かないこと。
4. **`imm_cross_write`の`None`=`Failed`は変えない**。`MsImeDirect`は冪等なので、不明を「開いていない」と扱っても逆転しない。`fallback_write`のdocは実装に合わせて直す。
5. **ROMAN補完の挙動差分を受容する(実測を残す)。** `apply_mechanism`は先頭で`romaji_pre_write`を呼び、`decide_needs_romaji_pre_write`は
   `open && {ImmCross, MsImeDirect} && kind==MsIme && belief!=ObservedKana`で真。変更前のfallback(`KanjiToggle`)では偽だったが、変更後(`MsImeDirect`)は真になり、
   `SendMessageTimeout`ベースの同期ブロッキング往復が`with_app`を握ったまま走る。a9ログの実測: `VK_IME_ON`の送信が**62.7ms**遅れ、`ROMAN 補完 Failed`
   (`res6/result-sc-kanji-msime-native-a9-2/dist/awase.log` 14:27:25.997168、GETの`probe_ime_control`が50msのタイムアウトを超えて`None`→SET側は走らず失敗)。
   最悪は**`get_focused_hwnd`(30ms)+`GetConversionMode`(50ms)+`SetConversionMode`(50ms)≒130ms、最大3往復**(`ime.rs`の`capture_blocking`/`modify_conv_mode`)。ImmCrossが既に約150msブロックした直後の稀な失敗経路への追加。
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

## 撤去前後の挙動(全組、opus round2で検証)

同期(`ImeController::apply`→`caps_chain_for`→`run_chain`)・非同期(`run_open_chain_async`→`WriteMechanism::ALL`走査→`fallback_write`)とも、**変わるのはStandard × MsImeだけ**で意図どおり
(ImmCross失敗後: `VK_KANJI`→`VK_IME_ON/OFF`)。`attempts`は3件(実ログの`attempts_len=3`と一致)。他は不変:
Standard×Gji `[ImmCross,GjiDirect]`、Imm32Unavailable/TsfNative×両 `[GjiDirect]`/`[MsImeDirect]`、InputRelay(`decide_gate`が`NotOwned`で`caps_chain_for`の前にreturn、非同期も`fallback_write`冒頭で停止)、
Plain/Unknown(構造的に到達不能)。`imm_cross_is_first_applicable`は全組で不変。`may_return_failed`が真なのは撤去後も`ImmCross`だけでINV-44のガード
(`caps_chains_have_no_unreachable_trailing_element`)は成立。終端は撤去前後とも`Failed`(全機構が`with_app`で`None`のときだけ)。

## `MsImeDirect`がImmCrossプロファイルで成り立つことの根拠(コードから確認済み)

- `decide_attempt`は`MsImeDirect`で`shadow_on`を参照せず無条件に`SendVk`(`ime_actuation_decision.rs`)。`fallback_write`の`shadow_on=None`上書き(BUG-113)は無害
  (`architecture_guard.rs`のコメントが既に明言)。`decide_gate`のInputRelay判定は機構に依らず`fallback_write`冒頭で効く。
- `VK_IME_ON`はopen軸のみで`conv`を触らない。ROMAN補完は`set_ime_romaji_mode_for_hwnd`が`conv | IME_CMODE_ROMAN`のread-modify-writeで`KATAKANA`ビットを落とさず、
  `belief==ObservedKana`のときはそもそも発火しない。よって「開いているカタカナのIMEを壊さないか」は実機確認項目ではなく既知事実。実機で確認するのは決定5のレイテンシ。

## 更新するもの(この変更と同じPRで)

- テスト/golden: `crates/awase-windows/tests/golden/ime_key_sequences.txt`(`MS-IME	Standard	async_fallback	KanjiToggle`→`MsImeDirect`、本文の「`!can_use_imm32_cross_process()`」説明、
  KanjiToggle節の「稀にしか到達しない」)、`tests/ime_key_sequence_golden.rs:211-215`(**`#![cfg(windows)]`で、Linuxのtestジョブでは0 tests。更新漏れはwindows-build CIまで気付けない**)、
  `state/key_sequence_policy.rs:205-222`の4アサーション、`state/app_ime_policy.rs`の`caps_chains_match_the_adr089_table`と定数名、`ime_controller.rs:1006`の`caps_chain_matches_legacy_all_scan`。
- 直すdoc: `ime_controller.rs`冒頭(13-14/24-28行、「冪等VK_DBE_*」の記述も)、`open_chain.rs`の`fallback_write`のdoc(`:434-437`、`:446-449`、`:465-468`)、
  `app_ime_policy.rs:60-62`、`focus/class_names.rs`の`uses_kanji_toggle`のdoc、`focus/tracker.rs:204`・`runtime/key_pipeline.rs:1682-1689`・`runtime/focus_tracking.rs:1043`の
  KanjiToggle言及、`transport.rs`の`plan`doc(ImmCross×F2の例外)。`KanjiToggleStrategy`とそのdoc、`architecture_guard.rs`の`post_kanji_toggle_to_focused`ガードは**撤去**。
- ADR/文書: **言及の差し替えで済まないもの**: `docs/ime-control-overview.md`の図(`:35`)・戦略リスト(`:188-190`)・独立節「`KanjiToggleStrategy`のconfidence gateと300msウィンドウ」(`:244`〜、
  `output/ime_apply_planner.rs`の`safely_confirmed`ロジックの説明でロジックは残る。「KanjiToggle系(Chrome/TsfNative等)」の呼称は今日既に誤り)は**節ごと書き直す**、
  `docs/workarounds.md`の独立項目「4-1. `post_kanji_toggle_to_focused`」(`:116`〜)と`:194`/`:218`は**項目を削除/歴史へ移す**。言及の更新: `docs/windows-api-constraints.md`・
  `docs/app-onboarding-checklist.md`・`CLAUDE.md`・`.claude/rules/fix-requires-evidence.md`。
  **触らない(履歴)**: `docs/experiments.md`、`docs/known-bugs/BUG-*.md`(BUG-110/113等)、過去のADR群(033/034/044/063/070/081/087/088/089/090/095/097/098/108/114/117/121/130/133/135/138/139/153/158/159/160/163/167/168/171)。
- ADR-089 §2.8: 表と「入れない理由」節は**削除せず「2026-09-20、BUG-152により覆した。当時の理由は実測ではなく実装の書き写しだった」と経緯を残す**(追記済み)。
- CI: `sc-dbe/kanji/shift-msime-native`を`observe`→`pass`、`check_consistency.py`の判定窓。

## 検証計画

- **`#[cfg(windows)]`で隠れる範囲(実装時の罠)**: `ime_controller`(`lib.rs:50-51`)・`ime`・`runtime`(`transport.rs`/`message_handlers.rs`/`open_chain.rs`)・`platform`・`journal`・`win32`・`output`・`imm`は
  Linuxで1行もコンパイルされない。よって**決定2の安全網`caps_chain_matches_legacy_all_scan`(`ime_controller.rs:1006`)はwindows-build CIでしか作動しない**。
  撤去差分の大半はhost targetの`cargo check`では検証できないので、`cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib`(リンカ不要)を必ず通す。
  Linuxで走る(壊れれば即検出される)のは`state/`配下: `actuation_chain.rs`の4本、`app_ime_policy.rs`のcaps全数テスト、`actuation_decision_record.rs`の境界テスト、
  `ime_profile_driver.rs`の不変条件テスト、`gji_direct_mechanism.rs`、コア`src/platform.rs`。`ALL_OUTCOMES`(`actuation_chain.rs:665` 7→6、`gji_direct_mechanism.rs:239` 6→5と`:279-284`の4→3)は長さ注記があるのでLinuxのコンパイルで必ず検出される。
  **windows-build CIまで気付けない**: `ime_key_sequence_golden.rs`、`ime_controller.rs`の全テスト、`message_handlers.rs`のroundtripテスト、`runtime/transport.rs`の`plan`テスト(決定2で呼び出し形を変える先)。

- 回帰テスト: 上記のgolden/単体テスト(`ImmCross × MsIme`のImmCross失敗後が`MsImeDirect`)。
- CI実機E2E: 本変更のビルドで`sc-dbe/kanji/shift-msime-native`が各3/3 PASS。`sc-dbe-msime-native-noawase`との一致。
- 実機(dragonflyg4、Microsoft IME): (a)**物理キーを伴わないopen**(engine起点)でImmCrossを失敗させ、`VK_IME_ON`だけで開くか(a9が示せなかった点)、
  (b)決定5のレイテンシ、(c)`KanjiToggle`が到達しなかった既存経路(Chrome/Edge等)が撤去前後で同じ挙動か(golden)。

## 実機検証結果(dragonflyg4、2026-09-21、`ci/e2e-scenarios`ビルド、`tools/e2e/ime_key_matrix/device/`)

実機のIMEはGJI(実環境)とMicrosoft IME本体(スパイクが`--msime`でアクティブ化、終了後にGJIへ復帰)。awaseはユーザーの`config.toml`(`gji_thumb_key_ime_toggle=true`等)を渡して別worktreeのバイナリを起動。

| シナリオ | 結果 | 備考 |
|---|---|---|
| GJI × DBE(suppress/passthrough)、漢字/IME ON・OFF、Shift単独 | **全PASS** | 退行なし(GJIのチェーンは不変) |
| MS-IME本体 × 漢字/IME ON・OFF、Shift単独 | **全PASS** | |
| MS-IME本体 × DBE(英数/カタカナ/ひらがな) | 2手順FAIL(5・9手目) | **本PRの経路ではない**: awaseは`shadow-toggle no-op`(belief既にON)で物理F2をPassThroughし、実MS-IMEがF2で閉じた(apply-imeの発行なし)。実機のMS-IMEは`conv=0x09`(かな入力)で、キー割り当て等の実機固有の可能性。観測を正とする設計(ADR-191)の領域 |
| GJI × 半角/全角(`--hz`) | 4/8 FAIL(F3が反転しない) | GJIのチェーンは不変で本PR無関係。開始時にbelief ON(true→false)と実IME OFFがずれた状態でF3を押す形。別件(ADR-189の初期状態の同期) |
| **a10(ImmCrossを強制失敗)** × MS-IME本体 | フォールバックが実機で動作 | 下記 |

a10(`ImmCross`を常に`Failed`にする実験ビルド、`open_chain.rs`のミューテーション)で、検証計画(a)「物理キーを伴わないopen(engine起点)で`VK_IME_ON`だけで開くか」を確認した:
- ログ: `ImmCross failed → GjiDirect not applicable → MsImeDirect: [apply-ime] MS-IME direct: send 0x0016 (IME ON)`、`ImmCross failed`から`send`まで約**1.6ms**(`romaji_pre_write`は`belief=ObservedKana`のため発火せず、決定5の最悪130msは今回の実機では未発生)。
- `sc-kanji`の2・3手目: 物理`VK_KANJI`はImmCrossプロファイルでSuppressされる(OSに届かない)ので、awaseの`VK_IME_OFF`だけで閉じ、続いて`VK_IME_ON`だけで開いた(実IME open 1→0→1)。**冪等キーが、物理キー無しで実機のMS-IMEを開閉できる**ことを確認。
- a10でEngineが実IMEに追随しない手順が出たのは、a10がImmCrossの`ROMAN補完`(conv書き込み)を丸ごと飛ばすため、実機のMS-IME(`conv=0x09`、ROMANなし)で`Inactive(NotRomajiInput)`になる人工的な副作用で、本PRの通常経路ではない。ただし副産物として、この状態(Engine非活性)で物理`VK_IME_ON`がSuppressされ、awaseも代替actuationを行わず、IMEが閉じたままになる手順を観測した(a10 step6)。ImmCrossが成功する通常経路(baseline)では`ROMAN補完`でEngineが活性化し発生しない。BUG-46型のファミリーとして記録に留める。

未実施: 対照(PR前のビルドでの同シナリオ)。上記の失敗2件はログ上、本PRの変更箇所を通らないことを確認した(実行したコードパス: `shadow-toggle no-op`→PassThrough、GJIのチェーン)。

## 残る限界

- MS-IME本体の半角/全角(0xF3/0xF4)は静的モデル(F3=OFF、F4=ON)のままで、同じVKの連続で反転しない(ADR-189が対象外にした範囲、別件)。
- `GjiDirect`/`MsImeDirect`の統合(どちらも`VK_IME_ON/OFF`)は別ADRの候補。
- `uses_kanji_toggle()`の名前が古い(機構撤去後は意味とずれる)。改名は別件。
