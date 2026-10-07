---
id: ADR-188
title: |-
  Chrome等(Imm32Unavailable/TsfNative)で、convだけを変えるモードキー(ひらがな、Shift+無変換)の後にEngineを追随させる
  (モードキー後の遅延conv読み取り、実機A/Bで4案を比較)
summary: |-
  BUG-149: ChromeでGJI(ATOK)のかな→半角英数(ひらがなキー/Shift+無変換)のあと、EngineがOFFにならず英数なのにNICOLAが動く
  (実機6/6失敗、awase停止の対照は24/24正常)。原因は、convを読む経路(`idle-conv-check`)がChrome(Imm32Unavailable)に入らず
  (ガード2が「TsfNativeのみ」)、20ms再読み取りも`SkipTyping`/`Blacklist`で読まないこと。実験4案(E1入口を広げる/E2モードキー後300msの
  強制チェック/E3=E2+Shiftガード中は再試行)を実機で比較し、E3が全ケースで最初の打鍵から正しい唯一の案だった。本ADRは実験の設計を
  実装に落とす前のレビュー対象で、フィールドの積み増しを最小にする形を探す。
status: |-
  追加計測完了・第1段の実装へ(2026-10-06): GJI の MS-IME プリセット・変換中・30 秒 idle 後のいずれも、最初の読みは 31〜110ms で、以後 300ms の窓内は値が一定(計 100 窓超)。変換中に読み取りの異常は出なかった。基準値なしの観測(R1〜R3・M2・M7 を満たす)を実装し、`sc-bug149-chrome-*-passthru` の A/B で確認する。BUG-186 は範囲外。
  旧(2026-10-04 更新前):
  **ドラフト(実験のみ、未実装)**。レビュー対象。実験パッチ: `188-measurements/e3-experimental.patch`(実験用、そのまま採用しない)。
related_adr:
  - "ADR-186"
  - "ADR-187"
  - "ADR-179"
---

# ADR-188: convだけを変えるモードキーの後にEngineを追随させる(TsfNative/Imm32Unavailable)

## 問題(BUG-149、実機再現)

`chrome_probe`(`spike/ime-key-matrix`の`crates/awase-windows/examples/chrome_probe.rs`、専用プロファイルのChromeで`k`,`a`を打って
出た文字でEngine/IMEの状態を判定)で、awase起動中(ADR-186実装+Shift修正入り)のChromeの8ケース×3周:

- 無変換/変換のON/OFFとShift+無変換のOFF中は18/18 PASS。
- **かな→半角英数(ひらがなキー0xF2、Shift+無変換)は6/6失敗**: IMEは半角英数になるがEngineがONのまま、`k`,`a`が`kiu`になる。
- awase停止の対照は24/24 PASS(GJI自身はChromeで正しく動く)。待ち2秒でも直らない。

## 原因(ソースとログで特定)

1. `idle-conv-check`(`runtime/key_pipeline.rs::kp_stage_idle_conv_check_inner`、次の打鍵で変換モードを読む唯一の機構)の入口
   `should_run_idle_conv_check`(`src/engine/idle_check.rs`)のガード2が`is_tsf_native`。Chromeは`profile=Imm32Unavailable`で、
   `AppImeProfile::is_effectively_tsf_native`は`is_tsf_native_window(class_name)`(WezTerm/Windows Terminal/XAML系の5クラスのみ)に
   委ねるため、Chrome(`Chrome_WidgetWin_1`)は黙ってfalse(ログ無し)。検証ログで`[idle-conv-check]`は0件。
2. モードキー押下の20ms後の再読み取り(`TIMER_IME_REFRESH`→`ir_decide_read_strategy`)は、押したキー自身が「入力中」を成立させ
   `SkipTyping`、入力中でなくても`skip_imm_query`のプロファイルは`Blacklist`(OsPollしない)。
3. Chromeのconvは読める(`[cold-diag] pre-send conv=0x00000019`、`WM_IME_CONTROL`経由)。読みに行く経路が無いだけ。

## 実験(実機、Chrome、awase起動、`188-measurements/chrome-ablation-summary.md`、`ablations/a8〜a10`)

| 構成 | 内容 | 8ケース×3周 | Shift 700ms長押し |
|---|---|---|---|
| base | 変更なし | 18 PASS、6 FAIL(ケース3・7が3/3失敗) | ― |
| E1 | ガード2を`cannot_verify_real_ime_state`に広げる(1行) | 18 PASS、**6 RECOVER**(最初の打鍵は誤り、次の打鍵で追随) | ― |
| E2-150ms | モードキー(KeyDown、物理)の150ms後にガード3/4/5を無視して`idle-conv-check`を1回強制 | 21 PASS、3 RECOVER(ケース7が回復のみ) | ― |
| E2-300ms | 同、300ms後 | 24/24 PASS | 14 PASS、**2 RECOVER**(Shift長押しで凍結) |
| **E3-300ms** | E2 + 強制チェックが`half_width_alnum`ガード中なら100msごとに再試行(最大30回) | **24/24 PASS** | **16/16 PASS** |

- E1は、次の打鍵で読む方式なので最初の1〜2文字が誤り。
- E2でShift長押しが負けるのは、強制チェックが`kp_stage_shift_conv_guard`(`half_width_alnum.is_guard_pending()/is_toggle_active()`)で
  凍結されるため(E3の再試行ログがShift700msで10回、40msで0回)。
- Win32 EDITの通常10手順の回帰(倍速12回、Engine判定含む): E2-300ms 12/12 PASS。E3は実行中(結果が出たら追記)。
- 強制読み取りは、物理のモードキー1押下につき1回(24ケースで66回)。

## 実験の実装(`188-measurements/e3-experimental.patch`、採用形ではない)

- `lib.rs`に`TIMER_MODEKEY_CONV = 110`。`GateStore`に`pending_modekey_event: Option<RawKeyEvent>`・`force_conv_check: bool`・
  `modekey_retries: u8`の3フィールド。
- `kp_stage_idle_conv_check`(毎キー呼ばれる)で、KeyDown・`is_ime_mode_key`・`!injected`のとき、イベントを保存して300msタイマー武装。
- `run_modekey_conv_check`(タイマー発火時): Shift関連ガード中なら100ms後に再試行、そうでなければ保存イベントを
  `is_ime_mode_key=false`にして`force_conv_check=true`で`kp_stage_idle_conv_check_inner`を呼ぶ。
- `..._inner`は`force`のとき、`output_idle_ms`と`explicit_age`を`u64::MAX`にし、`is_tsf_native`を強制`true`にする。

## 関連する他セッションの設計(重複の可能性)

`origin/adr/187-atok-passthrough-belief-follow`のADR-187は、ATOK+パススルー(opt-in無し)で、実IMEの開閉にEngineが追随しない問題を
「生キー通過後に読み直してbeliefを追随」する観測型で検討し、`IntentStore`/`ImeModel::last_intent`の二重固定を外すのに新しい
`ImeEvent` variantが要り重いとして見送った(status欄に「followのスパイクが成功したため撤回候補」とある)。
本ADRの「モードキー後に読み直す」は、機構が重なる可能性がある。

## レビューしてほしい点(批判的に)

1. 新しいタイマー+GateStore 3フィールドの積み増しは最小か。既存の機構(`FocusResyncGate`、`TIMER_IME_REFRESH`、`idle_conv_check_in_flight_since_ms`、
   ADR-187のfollow方式)を再利用して、フィールドを減らせないか。ADR-184の反省(型・フィールドを積まず最小の配線変更で)に照らして。
2. `force`で`is_tsf_native`を`true`にすると、**Standardプロファイルの通常アプリ(Win32 EDIT等)でも**、モードキーの後にTsfNative用の
   `idle-conv-check`適用経路(`apply_idle_conv_check`)が走る。副作用は無いか(実機のWin32 EDITの回帰は12/12だが、網羅か)。
3. BUG-113(読み取りとactuationの時間的近接で「@」)、BUG-34(`SendMessageTimeoutW`のブロック)、BUG-33/37(Chrome等でImmGet*が不安定)の
   再発リスク。モードキー1押下ごとに追加のクロスプロセス読み取りが入る。`is_ime_mode_key`のキー集合(無変換/変換/ひらがな/VK_IME_ON等)全てで良いか。
4. 競合: 強制チェックが保留中に、フォーカス変更、別のモードキー、`Engine`のON/OFF、ユーザーの打鍵が来たとき。`pending_modekey_event`が古い
   イベントを使い回す/フォーカスが変わったのに読む、を防げているか。`apply_idle_conv_check`のepoch/hwnd/`explicit_action_ms`照合は足りるか。
5. 300msと100ms×30回(最大3秒)の根拠(実測が不足)。`tuning.rs`の規約(`#[measured]`、実測ms)にどう載せるか。
6. Shift+無変換のShift長押し以外の凍結要因(`half_width_alnum`の`toggle_active`が長く続く場合)で、再試行が無限/長時間になる懸念。
7. メモ帳・Windows Terminal・Edgeでは未検証。Chromeでだけ効く実装になっていないか(`is_ime_mode_key`、プロファイル依存)。
8. もっと単純な代案があるか(例: モードキーのKeyUp/Shift解放を契機にする、既存の`schedule_ime_refresh`経路でconvを読む、E1+RECOVERを許容して
   最初の1〜2文字の誤りを許す、決定3の予測反転)。


## 2026-10-06 追記: 最新 develop での再確認と重複整理(実装前の方針)

詳細な根拠(ファイル:行・コミット)は読み取り専用の重複分析によるもので、本ADRの実装前に CI で効果確認が要る。

**症状の再確認(d0d42be6、`sc-bug149-*`)**: 既定設定(Suppress)の 10/24 FAIL のうち約 9 件は、無変換・変換・Shift+無変換が GJI に届かないための期待値のずれ(Engine と IME は整合)で BUG ではない。本物は、素通し設定(`-passthru`、df5c96d1 で 3/3 再現)の「無変換/変換で IME 閉・Engine ON」「Shift+無変換で IME 英数・Engine ON」と、ひらがなの追跡ずれ(1/3)。`-passthru` は 2026-10-06 の再実行(run 37427296256)で、かな→無変換/変換/Shift+無変換が各 3/3 FAIL(ひらがなは 3/3 PASS)と再現を確認した。

**機序(現行コード)**: 実 Chrome(`Imm32Unavailable`)×GJI では 20ms 後の prefetch が開閉と conv を実際に読んでいるのに、読み取り方針が `SkipTyping`/`Blacklist` でその値を捨て、その後の読み直しも予約しない。例外は ADR-205 の窓だけで、外部注入キーでしか arm せず開閉しか見ない。予測(ADR-191 決定3)は Shift 付きと、FSM が保留後に再送出した素通しの親指キーを対象にしない。

**構成要素ごとの整理**: E1(ガード2拡大)は単独で弱い(次の打鍵で読む)。E2・E3・新タイマー・GateStore 3 フィールドは ADR-205 の監視窓(300ms 窓・60ms 再読み・ShiftConvGuard より前の照合)と重複。ADR-187 は ADR-191 決定2 に吸収済みで、本文の「他セッションの設計」節は古い。ADR-206 は既定 Suppress が (a) の見かけの FAIL の原因。ADR-208・212・213 は競合なし。ADR-227 の give-up arm が「外部注入以外で窓を arm する」前例。

**最小配線案(GJI×`Imm32Unavailable` 限定、新型・新タイマー・新 GateStore フィールドなし)**:
1. 監視窓の arm 条件を物理モードキー通過にも広げる(`key_pipeline.rs` の `kp_arm_external_change_watch`、Shift 付きも。`executor.rs` の FSM 再送出でも)。観測だけで書き込み・意図への昇格はしないので BUG-14 の型には当たらない。
2. 照合値を開閉から「開閉+conv の NATIVE ビット」に広げる(`ir_follow_external_change`)。awase 自身の conv=0 書き込み(左 Shift 単独タップの確定)直後は外部変化として採らない条件が要る(`conv_mutation::current()` を arm 時に控える、フィールド 1 個は増える設計判断)。
3. `reschedule_ime_refresh` は窓が生きている間は既に予約するため変更不要の見込み。
限界: GJI 限定、追随は最大 ~80ms 遅れ(E3 の「最初の打鍵から正しい」より弱い可能性)、基準値が古いと誤追随の恐れ。

**範囲**: BUG-149 と BUG-150 の Chrome 版は含める(表題を「読めない窓で、物理モードキー通過後の開閉・conv の変化に追随する」に広げる)。BUG-186(MS-IME 本体×実 Chrome の持続トグル)は含めない。根が 3 つ重なる(トグル中の ShiftConvGuard 凍結、MS-IME の読みの信頼性が未測定、本体の表に「開・C10」のセルなし)ため、MS-IME の conv の読みを CI で測ってから別 ADR にする。


## 2026-10-06 追記2: Opus 実装前レビューの反映(方針の段階化)

先の「最小配線案」を実装前に敵対的レビューした結果(Blocker 2・Must 6・Should 5)、そのままでは動かないと判明した。主な指摘と対応:

- **B1 arm 箇所が違う**: `kp_arm_external_change_watch` は `may_change_ime` のキーでしか呼ばれず、無変換/変換(0x1C/0x1D)はそこに含まれない。Shift+無変換は arm も 20ms の再読み予約も通らない(`reschedule_ime_refresh` 変更不要の前提も崩れる)。→ arm と最初の予約は `kp_stage_mode_key_follow` の Shift の早期 return の前、executor の再送出の箇所に別に置く。
- **B2 基準値が古い**: 明示意図があると読めない窓ではポーリングが止まり、基準値が awase 自身の書き込み前のまま残る。最初の変化で窓を閉じる仕様と重なると、取りこぼしと逆追随(意図の削除)が起きる。→ `Changed` の後も窓を閉じず照合を続ける。awase が書いたら直近の読みを無効にし belief を基準にする(`conv_mutation` の控えは、FSM が変換を再送出するたびに追随を止めるため使わない)。
- **M1** 予測の fence(170ms、`KEY_EFFECT_SETTLE_MS`)内の `Changed` は採用されない。予測が出たキーには arm しない。**M2** 変換中(composition)は arm しない(候補窓中の読み取り増加は BUG-113/34 のファミリー)。**M3** `shadow_action`/`sync_direction` を持つキーは arm の対象から外す(述語は `kp_stage_mode_key_follow` と同じ)。**M5** conv の追随は純関数を通し、hub 内から `InputModeObserved(ConvBitsInference)` で書く(ROMAN ビットから Kana を作らない)。**M6** 1 回の変化で開閉と英数の両軸を反映する。
- **代案(採用)**: 無変換/変換(Shift なし)は FSM が再送出する時点で予測(ADR-191 決定3)を当てる。ひらがなが 3/3 PASS しているのと同じ仕組みで、conv も窓も基準値も要らない。監視窓の拡張は、予測表が修飾なしのキーだけのため予測が効かない **Shift+無変換だけ**に絞る。

**段階**:
- 第0段(ログのみ、挙動不変): GJI×Imm32Unavailable で、物理モードキー/再送出の後の窓内の prefetch の `(t_ms, open, conv)` の列を `[mode-key-trace]` として出す。測るのは、(a) 何 ms で値が変わるか、(b) 遷移中の値が出るか、(c) arm 時点の最後の読みの古さ、(d) 前提状態のセットアップで awase の明示書き込みが入るか。使い捨ての `ci/adr188-trace` ブランチで行い、develop にはマージしない。
- 第1段: 結果が良ければ executor の再送出で予測を当てる(`sc-bug149-chrome-atok-passthru` の無変換/変換が 3/3 PASS になるか)。
- 第2段: Shift+無変換だけ監視窓を拡張(B2・M1〜M6 を満たす)。
合格条件は修正前後の A/B で、既定 Suppress と awase なしの対照の FAIL の集合が変わらないこと、MS-IME 構成で `[external-change]` が 0 件、素通し設定で「打鍵→変換→確定」を繰り返しても変換中の追随が 0 件であること。回帰テストは `state/external_change_watch.rs` の純関数テストと `tests/closed_loop_scenarios.rs`、`tests/architecture_guard.rs` に置く。


## 2026-10-06 追記3: 第0段(計測)の結果と、基準値なしの観測案

`ci/adr188-trace`(`13c23c54`、挙動不変のログのみ)を `sc-bug149-chrome-atok-passthru` で実行(run 37437413624、実 Chrome・GJI の ATOK・素通し・`profile=Imm32Unavailable`)。物理モードキー(素通し)と FSM 再送出の後、300ms の窓で 60ms ごとに prefetch の `(open, conv)` を出した。30 窓:

- **最初の読みは 31〜46ms(中央値 31ms)で、以後 300ms の窓内は値が一定**(窓内で値が変わった窓は 0 件)。ただし 20ms のタイマーは約 31ms に丸められ、それより前の値は分からないので「遷移途中の値が出ない」とは言えない(訂正、Opus r2 M8)。
- 値は意味が通る: かな=conv 25(0x19)、半角英数=16(0x10)、無変換/変換→IME OFF の FSM 再送出は `open=false`。Shift+無変換(`vk=0x1D shift=true`)は `open=true conv=16`(半角英数)。直接入力→無変換/変換は `open=true` で conv は 25 が多いが、6 窓中 1 窓は `conv=9`(ROMAN ビットなし、実際はローマ字入力で probe は PASS。R2)。
- 追随が要る状況が実在する: FSM 再送出の窓(かな→無変換=IME OFF)6 件はすべて `open=false` だが `belief_open=true intent=Some(true)`。Shift+無変換の 3 件は `conv=16` だが belief は `ObservedEisu` でなくかな扱い(`belief_open=true`)。つまり**打鍵の約 31ms 後には実状態を読めており、読まれた値と awase の belief が食い違っている**。
- 測れていない: arm 時点の最後の読み(基準値)の古さ(今回は基準値を持たない設計のため)。前提状態のセットアップでの awase の明示書き込みの有無は `intent=Some(true)` の多さから、明示意図が残る状況は日常的に起きる。

**第1段の再設計案(基準値なし、Opus 再レビュー待ち)**: 先の案(窓の基準値との差分で追随)は、B2(基準値が古いと取りこぼしと逆追随)の弱点を持つ。しかし最初の読みが 31ms で既に遷移後の状態であり、遷移途中の値が出ないため、**基準値を持たず、窓内の読み(GJI×`Imm32Unavailable`、フォーカス・世代が同じ)を直接の観測として採る**ことができる見込み。すなわち、物理モードキー通過/FSM 再送出の後の窓内の prefetch を、`SkipTyping` で捨てる代わりに既存の観測(開閉は `write_observer_poll` 相当、conv は `InputModeObserved(ConvBitsInference)`)として classify を通して採用する。これなら無変換/変換・Shift+無変換・ひらがなを同じ経路で扱え、予測側の配線(executor からの `kp_predict_key_effect`)も不要になる。守る条件: M1(予測の fence 170ms との干渉)、M2(変換中は arm しない)、M3(`shadow_action`/`sync_direction` 付きは対象外)、M5(純関数・ROMAN から Kana を作らない)、M6(両軸を 1 回で反映)、BUG-14(意図への昇格をしない)。


## 2026-10-06 追記4: Opus r2 の反映(第1段の確定設計)

追記3 の基準値なしの観測案を再レビューした(Blocker 2・Must 4・Should 6)。案は採用してよいが、次を満たさないと動かない・退行する。

- **R1(Blocker)**: 開閉を `write_observer_poll` だけで書いても Engine は OFF にならない(`effective_open_at` は IntentStore の意図を観測より優先する、`state/platform_state.rs:835-845`。計測の FSM 再送出 6 窓はすべて `intent=Some(true)`)。ADR-205 の `follow_external_change_in_scope` と同じ 3 つの副作用(`ObserverPoll` → `intent_store.remove` → `ModeKeyPassedThrough{align_desired:true, demote_applied:true}`)が要る。
- **R2(Blocker)**: 既存の classify(ROMAN ビットを見る)を通すと、いま PASS のケースが FAIL になる。計測の「直接入力→無変換=かな ON」(`t=303031`)は `open=true conv=9`(ROMAN なし)が 300ms 続いたが実際はローマ字入力。**英数かどうかは NATIVE ビットだけで決め、開いていないときは conv を見ない**。`ObservedKana`/`ObservedRomaji` は作らず、`AssumedRomaji`/`ObservedEisu` のみ。
- **R3(Must)**: 窓内で awase 自身が IME へ書いた(左 Shift の `VK_DBE_ALPHANUMERIC`、明示の SetOpen 等)後に、GJI の処理前の読みを採ると belief を逆戻しする(BUG-51 型)。書いたら窓を閉じる。`conv_mutation` は使わない(変換の再送出のたびに発火するため)。「arm 時刻 < `last_explicit_ime_action_ms` なら採らない」でフィールドを増やさずに書ける(全経路が更新していることを実装時に確認)。
- **M2** 変換中(`ime_composition_active_now()`)は arm しない。**M7** 追随でも `last_external_change_ms` を更新する(Blacklist の `observe_gji_after_focus` に打ち消されないため)。**M5** GJI の MS-IME プリセット・変換中・長い idle 後の計測を実装前に行う。

**実装(最小、既存の variant で足りる)**: (1) `state/external_change_watch.rs` の `Armed` に種別(`Baseline`=ADR-205、`Direct`=ADR-188)を足し、`Direct` は基準値を使わず窓内なら読みを返して窓を閉じない。(2) 純関数 `classify_direct_mode_key_read(open, conv)`(上記 R2 の規則、ungated)。(3) hub に `follow_external_change_in_scope` の隣のメソッド(開閉の食い違いは R1 の 3 副作用、conv は `InputModeObserved{ConvBitsInference, Medium}`、両軸を 1 回で)。(4) `ir_follow_external_change` に `snap.conversion_mode` も渡す。(5) arm は `kp_stage_mode_key_follow` の Shift の早期 return の前と executor の再送出の 2 箇所、条件は `external_change_watch_applies()` かつ変換中でない、20ms の予約付き。

**合格条件(A/B)**: `sc-bug149-chrome-{atok,msime}-passthru` で無変換/変換/Shift+無変換が 3/3 PASS、**かつ「直接入力→無変換/変換=かな ON」が 3/3 PASS のまま**(R2 の回帰検知)。`--settle` を短くした構成で追随の遅れによる最初の文字の誤りを数値で残す(既知の限界)。`sc-table-{atok,msime}`(Shift 単独タップ後、R3)、既定 Suppress・`-noawase`・ADR-205 の構成は FAIL の集合が不変、MS-IME 本体で追随ログ 0 件。**回帰テスト**: 純関数の表(`(Some(false), 9|25)`→None、`(Some(true), 9)`→AssumedRomaji、`(Some(true), 16)`→ObservedEisu、`(Some(true), 25)`→AssumedRomaji、conv が None)、`Direct` 窓の単体テスト、`tests/closed_loop_scenarios.rs` の 5 シナリオ(意図が残ったまま再送出後に open=false を読む等)、`tests/architecture_guard.rs`。


## 2026-10-06 追記5: 追加計測(M5)の結果

使い捨てスパイク `ci/adr188-trace`(`1d85e501`)で、素通し設定の実 Chrome×GJI に対し、窓内の `(open, conv)` を測った(run 37464007197=MS-IME プリセット 27 窓、run 37470606468=変換中・30 秒 idle 後の 12 構成)。

- **GJI の MS-IME プリセット**(27 窓): 窓内で値が変わった窓は 0 件。最初の読みは 31〜47ms。conv は 25/27/16 で、ROMAN ビットの有無は ATOK と同様にばらつく(NATIVE ビットだけで判定する R2 の方針を支持)。
- **変換中**(`--compose-key`、k,a で未確定文字を作ってから 変換/無変換/Shift+無変換): 窓内の値は一定(開閉 true、conv 9=変換 / 16=無変換・Shift+無変換)。最初の読みは 31〜110ms(1 窓だけ 110ms)。読み取りの異常・ブロックは見えなかった。ただし測ったのは 1 試行あたり最大 5 窓で、候補窓が長く開いた場合や BUG-34/113 型の負荷は測れていない。**M2(変換中は arm しない)は、安全側の条件として残す**(`ime_composition_active_now()`、コストが小さい)。
- **30 秒 idle の後**(`--idle-key`): 最初の読みは 31〜47ms で、すでに遷移後の状態(かな→無変換/変換=`open=false`、Shift+無変換=`conv=16`)。idle が最初の読みを遅らせる様子は無い。修正前の probe は、ATOK の無変換/変換/Shift+無変換が「ローマ字のまま(未追随)」、MS-IME プリセットの変換は「NICOLA 文字(Engine ON)」(変換は IME ON を保つので正しい)。
- 補足: MS-IME プリセットの無変換で conv=27/19(0x1B/0x13、全角カタカナ)が出た。NATIVE ビットは立つので `AssumedRomaji` になり、R2 の方針で矛盾しない。probe の setup が `setup_kana=false` になる試行が ATOK の compose で 2 件あった(IME 状態の準備の失敗で、本件と無関係)。

結論: 基準値なしの観測案(追記4)で第1段の実装に進んでよい。
