---
id: ADR-244
title: |-
  実 Chrome × MS-IME 本体で、半角英数の持続トグル中の IME 側モードキーに Engine を追随させる(BUG-186)
summary: |-
  BUG-186: 左 Shift 単独タップで持続半角英数にしたあと、変換・英数・ひらがなを押すと IME はかなへ戻るが Engine は OFF のまま(`か`)。
  ADR-188 第1段(PR #539)の後も最新 develop で再現する。run 37704093235 のログでは、かなに戻った実状態は約 26〜45ms 後に正しく読めているのに、
  打鍵中扱い(TypingActive)で捨てられ、ADR-188 の直接観測は GJI 限定の述語で MS-IME 本体には一度も動いていない。
  本 ADR は事実・CI での検証(S1 の計測、S2 のスパイク 48/48 PASS)と、S2 を本番に落とす方針(決定 D1〜D7)を定める。
status: |-
  実装済み・CI 検証済み・実機未確認(2026-10-08、PR #556): S2 採用・S3 不採用。方針 D1〜D7 は Opus の計画レビュー(Blocker 0・Must 6・Should 6)を反映して確定し、実装は Opus のコードレビュー(Blocker 0・Must 5・Should 5)を反映した。Opus レビュー 2 回(1 回目 Blocker 2・Must 4・Should 6、2 回目 Blocker なし・Must 2・Should 3)を反映済み。
related_adr:
  - "ADR-188"
  - "ADR-107"
  - "ADR-191"
---

# ADR-244: 持続トグル中の IME 側モードキーへの追随(BUG-186)

## 現状の事実(2026-10-07)

確かめたコマンド(`develop` 先端 `2a178d5b`、PR #539 を含む):

```sh
gh workflow run e2e-ime.yml --ref develop -f only='sc-table-msime-*'   # run 37704093235
gh run view 37704093235 --log | grep -E "RESULT (PASS|FAIL)|SUMMARY"
gh run download 37704093235 -n result-sc-table-msime-passthru-1       # 構成ごとの awase.log(suppress も同形)
```

- 2 構成(`sc-table-msime-passthru`・`-suppress`)× 2 周 × 24 セル。各構成の SUMMARY は `PASS=42 FAIL=6`。
- 各構成・各周とも、直接入力・かな・半角英数 × 6 キーは 18/18 PASS(計 72/72)。
- 「Shift単独タップ後」は無変換・IME_ON・IME_OFF が PASS、**変換・英数・ひらがなが FAIL**(`か`)。FAIL は 3 キー × 2 周 × 2 構成 = 12 セル。キー後 2 回目の確認も `か`のままで、待ち不足ではない。
- FAIL の 12 セルすべてで、`[mode-key-follow]` の約 26〜45ms 後(中央値 約 33ms)に `[skip-typing-read] ime_on=Some(true) conv=0x00000019`(かな)が読めていた(例: passthru の awase.log、23:48:41.437526Z の `vk=0x1C` の 34.3ms 後)。トグルON〜かな入力へ復元の区間の `[mode-key-follow]` と次の `[skip-typing-read]` の時刻差を awk で計算した。直前に `Skipping observer/SSOT write: typing active (idle=32ms)` が出て、読みは捨てられている。
- 同じ run の awase.log で `[direct-follow]` と `[external-change]` は 0 件。`profile=Imm32Unavailable … active_ime_kind=MicrosoftIme` が出ている。
- 無変換(トグル中)は `ime_on=Some(false) conv=0x00000000`(閉)と読め、これも捨てられている。プローブは `ka` なので PASS だが、IME は閉・トグルは保持という組み合わせが残る(範囲外、S1 で観察する)。

## 原因の整理(コードで確認、2026-10-07)

不具合の**根**は次の 3 つ。

- **a. 読みを捨てる直接の理由は `TypingActive`**: `state/ime_read_strategy.rs::decide_read_strategy` は打鍵中の判定を先に評価する。Chrome は `skip_imm_query` なので `explicit_verify` で迂回できない。
- **b. ADR-188 の直接観測は GJI 限定**: `runtime/mod.rs::external_change_watch_applies_for` が `Imm32Unavailable` かつ `active_ime_kind() == GoogleJapaneseInput` を要求し、窓を開く 2 か所(`runtime/key_pipeline.rs::kp_stage_mode_key_follow`、`runtime/executor.rs` の FSM 再送出)と、照合する 1 か所(`runtime/ime_refresh.rs::ir_follow_direct_mode_key_read`)がこれで入口を閉じる。述語を広げるときは 3 か所をそろえて変える。MS-IME 本体ではトグルの有無に関係なく一度も動かない。
- **c. トグル中の凍結(2 段目の関門)**: `shift_conv_guard_active` は、打鍵中判定を通り抜けた後で読みを止める。トグル開始は awase 自身の `conv=0x0000` 書き込み(`[shift-conv-guard] … トグルON (conv=0x0000 書き込み)`)で、`ime_read_strategy.rs` のコメントは「awase 自身が設定した状態なので観測として belief に入れない」という意図。凍結だけを外しても a が残る。

修正前に確認すること(根ではない): MS-IME 本体の conv 読みが窓内で一定か(ADR-188 の追加計測は GJI の MS-IME プリセットのもの)、MS-IME 本体の学習表に「開・C10(生の値は 0x00)」のセルがないこと(`no prediction`)。

## 却下済みの案と、その範囲

`024ca336`(素通しのモードキーが来たらトグルを OS 書き込みなしで手放す)は `6b1e91b8` で戻した。ただし:

- `か`のまま変わらなかった理由は、トグルを手放した後も同じ読みが根 a で捨てられたためと読める(当時は直接観測も無かった)。「手放すこと自体が誤り」の証拠にはなっていない。
- 戻した実質的な理由は、GJI MS-IME プリセットで Shift タップ後の無変換に `ro` が新しく見えた退行(run 37234927086)。どのセルで、何と比べて「新しく」かの記録が薄く、`6b1e91b8` の本文も自動生成のみ(experiment-logging 規約の 3 点が無い)。
- 禁止するのは「**キー押下だけを根拠に、無条件で**手放す」こと。「**観測された読み**(NATIVE ビットが立つ等、awase 自身の `conv=0` 書き込みと区別できる値)で手放す」は未判断。

## S1 の結果(2026-10-07、MS-IME 本体 × 実 Chrome、観測のみ)

計測用スパイク(使い捨て、`develop` にはマージしない): ブランチ `ci/adr244-s1-trace`(`4b59999b`、`9cd78810`)。素通しのモードキーの後 300ms の間 60ms ごとに refresh を回し、`[mode-key-trace] read t= since_arm= open= conv= toggle_active=` を出す(GJI 限定の述語は使わない、挙動は変えない)。

```sh
gh workflow run e2e-ime.yml --ref ci/adr244-s1-trace -f only='sc-table-msime-*'                    # run 37706872040
gh workflow run e2e-ime.yml --ref ci/adr244-s1-trace -f only='sc-bug149-trace-compose-msimebody-*,sc-bug149-trace-idle-msimebody-*'   # run 37710113306
gh run download <run> -n result-<構成名>-1                                                         # awase.log の [mode-key-trace]
```

(`only` のワイルドカードは末尾の `*` だけ有効。途中の `*` は何にも一致しない。)

- **窓内の値は一定**: `sc-table-msime-*`(passthru・suppress)の 86 窓で、窓内の `(open, conv)` が変わった窓は 0 件。最初の読みは 31〜32ms(1 窓だけ 78ms)。
- **トグル中(`toggle_active=true`)の読みは実状態を示す**(passthru・suppress 各 2 窓、同じ値): 変換 `open=true conv=25`、英数 `open=true conv=25`、ひらがな `open=true conv=25`(いずれもかな)、無変換 `open=false conv=0`(閉)。FAIL の 3 キー(変換・英数・ひらがな)はすべて NATIVE ビットが立つ `conv=25` で、トグル中に awase が書いた `conv=0` とは区別できる。無変換の `conv=0` は `open=false` と併せて見る必要がある(`conv=0` だけでは awase 自身の書き込みと区別できない)。
- **変換中**(`compose`)と **30 秒 idle の後**(`idle`)、変換キー: 最初の読みは 31〜47ms で、`open=true conv=25`、窓内は一定。Shift+無変換は `open=true conv=24〜25`。
- **読みの失敗が 1 窓**: compose × Shift+無変換の 1 窓で、78ms に `open=true conv=25`、250ms に `open=None conv=None`(読めなかった)。値が別の状態へ変わったのではなく読み失敗。起きる条件は未調査(試行数が少ない)。
- **制限**: compose/idle は各 1 試行 × 3 回(窓 3〜4 件)と少ない。無変換単独は親指キーの扱いで compose/idle 構成では arm されず(窓は setup の F2 のみ)、無変換の読みは `sc-table` の分だけ。変換中に候補窓が長く開いた場合や負荷は測っていない。

**判断**: 根「読みの信頼性が未測定」は、少なくともこの範囲では解消した(窓内一定・最初の読み 31〜47ms・NATIVE ビットで awase 自身の `conv=0` と区別できる)。S1 の残りは、読み失敗(`None`)の頻度と、`ro` 退行の機序(未調査のまま)。次は S2 と S3 のどちらで行くかの決定(`follow_direct_read_in_scope` の予測優先との関係)。

## 現行の仕組みの調査(2026-10-07、コードと S1 のログで確認)

- **トグル中に「予測なし」になる理由**: `state/key_effect_predictor.rs::predict_in_table` は、入力モードが `ObservedEisu` だと変換モードを `Conv::C10` と見なして表を引く。`MSIME_NATIVE`(`state/key_effect_table.rs`)の「開いている」セルは `C19`・`C1B` の 2 種類だけで `C10` が無いので、該当セルが見つからない。S1 のログでは、トグル中の変換・無変換・英数・ひらがなが 8/8 すべて `[key-effect-predict] … no prediction`(トグルなしの状態は予測が付く)。
- **S2 と S3 の関係**: `follow_direct_read_in_scope`(`state/platform_state.rs`)は、その打鍵に予測が付いていると採用を見送る。トグル中は予測が付かないので S2 の経路は使える。S3(表に `C10` のセルを足す)をすると予測が付いて S2 の経路がふさがる。二者択一に近い。
- **読みを捨てる関門は 2 段**: `decide_read_strategy`(`state/ime_read_strategy.rs`)の `TypingActive`、次に `ShiftConvGuard`。ただし直接観測の照合 `ir_follow_direct_mode_key_read` は `ir_stage_observe` の中で読み方針の決定より前に呼ばれるため、どちらの関門もすり抜けて照合に届く。GJI 限定の述語 `external_change_watch_applies_for`(`runtime/mod.rs`)が止めているのは、窓を開く 2 か所(`runtime/key_pipeline.rs::kp_stage_mode_key_follow`、`runtime/executor.rs` の FSM 再送出)と照合 1 か所の計 3 か所。同じ述語は ADR-205 の外部変化の監視(`ir_follow_external_change`)も止めている。
- **読みの経路**: トレースの読みは `spawn_ime_refresh` の prefetch(`read_ime_state_full_async` → `ime.rs::detect_ime_open_for_hwnd`)で、`GetGUIThreadInfo().hwndFocus` → `ImmGetDefaultIMEWnd` → `WM_IME_CONTROL`(`IMC_GETOPENSTATUS`、50ms)。ADR-205 が「MS-IME では開いていても 0」と書いた `CrossProcess(hwndFocus)` と同じ経路。高速版 `read_ime_state_fast` は別経路(`Imm32Unavailable` では `ime_on=None`)で、トレースには使っていない。
- **ADR-205 の観測との食い違い**: ADR-205 の run 36548761653 は、同じ表の中で「9〜10 回は IME が閉じず」とも書いており、IME が実際に開いていたかが疑わしい(その構成は現在の `e2e-ime.yml` に無く再確認できない)。S1 では 18 セル(直接入力・かな・半角英数 × 6 キー)で `open` の読みと結果(`ka`/NICOLA 文字)の食い違いが 0 件だった。原因はセットアップの違いと考えるのが自然だが、推測。
- **条件を広げる場合に残る課題**: `toggle_held` が true のまま残る(観測で belief を戻しても凍結と次の Shift タップの意味は食い違う)。`ro` の退行の機序(GJI の MS-IME プリセット側)は未調査。

## S2 スパイクの検証結果(2026-10-08、CI、使い捨て)

使い捨てのブランチ `ci/adr244-s2-spike`(`845cd75f`・`55252fbe`、`develop` にはマージしない)で、次の 2 点を足して `sc-table-msime-*` を回した。

1. ADR-188 の直接観測(窓を開く 2 か所と照合 1 か所)を、GJI に加えて MS-IME(`active_ime_kind() == MicrosoftIme`)にも適用する(新しい述語 `direct_mode_key_watch_applies_for`。ADR-205 の外部変化の監視は GJI 限定のまま)。
2. 直接観測で「かな(NATIVE の読み)」または「閉」へ追随したら、持続トグルを OS 書き込みなしで手放す(`HalfWidthAlnumState::abandon_for_mode_key`)。

```sh
gh workflow run e2e-ime.yml --ref ci/adr244-s2-spike -f only='sc-table-msime-*'   # 1 のみ: run 37713583320 / 1+2: run 37714287200
```

| 構成 | 「Shift単独タップ後」の変換・英数・ひらがな・無変換 | `SUMMARY`(2 構成とも同じ) |
|---|---|---|
| `develop`(`2a178d5b`、run 37704093235) | 変換・英数・ひらがなが FAIL(`か`)、無変換は PASS | `PASS=42 FAIL=6` |
| スパイク 1 のみ(run 37713583320) | 変換・英数が PASS(NICOLA 文字)。ひらがな・無変換は **INVALID**(`持続半角英数にならなかった`) | `PASS=44 FAIL=0 INVALID=4` |
| スパイク 1+2(run 37714287200) | 4 キーとも PASS(変換・英数・ひらがな = NICOLA 文字、無変換 = `ka`) | **`PASS=48 FAIL=0 INVALID=0`** |

- **1 のみで INVALID が出た理由**: 追随の後も `toggle_held` が true のまま残り、次のケースの setup の Shift タップが「開始」でなく「解除」(`ひらがなモード復元`)として扱われた。S2 の課題として予想していたとおり。2 を足すと解消した。
- **直接観測が働いたのは対象の窓だけ**: 1 のみの run の awase.log で `[direct-follow]` は 4 件(変換・英数 × 2 周、`eisu=Some(false)` `conv=25`)。他の窓での誤った追随は無かった。
- **回帰の確認(同じ構成で `develop` と比較)**:
  - MS-IME 本体 × Chrome の `sc-adr209-chrome-msime*`・`sc-adr211-chrome-msime-f13*`・`sc-reopen-tsf-msime-gap600`・`sc-startup-msime-chrome-on-*`・`sc-driftrecovery-*`(計 49 ジョブ、run 37714886987 と 37714889571): 終了コードが FAIL(1)で食い違ったジョブは 0。食い違い 4 件はすべて INVALID(3)と PASS(0)の出入りで、スパイク側・`develop` 側の両方に出た(既存の setup の不安定)。
  - GJI の遷移表 `sc-table-gji-*`(run 37716008111 と 37716011228): 4 構成とも PASS/FAIL/INVALID の件数が `develop` と同じ(例: `gji-msimepreset-passthru` は PASS 28・FAIL 4・INVALID 16)。INVALID になるセルが 1 つずれた箇所(IME_ON とひらがな)が `gji-msimepreset` にあるが、件数は変わらない。`ro` の退行(GJI の MS-IME プリセットの Shift タップ後の無変換)に当たる FAIL の増加は無い。

**確定したこと**: BUG-186 は、直接観測を MS-IME 本体へ広げ、追随時にトグルを手放す 2 点で、CI の再現(`sc-table-msime-*`)では直る。S2 を採る(S3 は採らない)。

**未確定・次の課題**:
- 本番実装ではない。スパイクは `external_change_watch_applies_for` の GJI 限定の理由(ADR-205)を、直接観測の分だけ外したにすぎず、単体テスト・`architecture_guard`・`PLATFORM_STATE_PUB_FNS` 等のガードは見ていない。
- 手放す条件(NATIVE の読みまたは閉)は最小の案。ADR-188 の R3(awase 自身の書き込みの後は採らない)や M2(変換中は arm しない)との関係、トグルを手放す責務の置き場所(`ir_follow_direct_mode_key_read` の中か、`ImeStateHub` か)は設計が要る。
- `[mode-key-follow]` が実機の Chrome × MS-IME 本体で同じに働くかは未確認(CI のみ)。
- `GJI` の MS-IME プリセットの `ro` の機序は未調査のまま(今回のスパイクでは再現しなかった)。

## 決定(2026-10-08、Opus の計画レビューを反映)

**D1. S2 を採り、S3 は採らない。** 根拠: S2 スパイクが `sc-table-msime-*` で 48/48 PASS(run 37714287200)。S3(表に `C10` のセルを足す)は予測が付いて S2 の経路をふさぐ。

**D2. 述語は「`Imm32Unavailable` かつ GJI または同定済みの MS-IME 本体」。** スパイクの `active_ime_kind() == MicrosoftIme` は「GJI 以外」の意味で ATOK・Japanist・未知 TIP・IMM32 HKL のみ・起動直後の未検出(`tsf_active_kind == 0`)も含む(`tsf/observer.rs` の `active_ime_kind`・`ms_ime_native_identified` の doc)ので使わない。既存の `table_ime_kind().is_some()` を使う(GJI → `Some(Gji)`、同定済み本体 → `Some(MsIme)`、それ以外と起動直後 → `None`)。CI で本体が同定されていることは `[tip-detect] initial IME kind: MicrosoftIme (MsImeNative)`(run 37713583320 の awase.log)と BUG-179 の修正(`3e3a4ca9`・`518b62ee`)で確認した。述語の論理は `state/` の純関数(`ImeKindId` を受ける)に置き、殻は呼ぶだけにして Linux で単体テストする(ATOK・未検出・HKL のみが `false`)。ADR-205 の外部変化の監視は GJI 限定のまま(その理由は撤回しない。直接観測は基準値を持たず窓内の現在値だけを見る点で違う)。

**D3. トグル解除の置き場所と範囲。** `DirectFollow` には載せず、`state/half_width_alnum.rs` の純関数(例: `should_abandon_on_observed_follow`)で判定し、殻 `ir_follow_direct_mode_key_read` から呼ぶ(`PLATFORM_STATE_PUB_FNS` を増やさない)。条件は「観測された読みが NATIVE(`eisu=Some(false)`)」。スパイクが飛ばしていた通常の解除経路 `kp_restore_kana_from_half_width` の副作用は次のとおり決める。
- `note_explicit_ime_action` は**呼ばない**(呼ぶと同じ窓の後続の読みが R3 で捨てられ、誤追随を窓内で直せなくなる)。
- `ime_mode_fsm.unconfirm`・確認ゲートの期限延長(`SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS` = 5000ms)の解除は、スパイクの run 37714287200 の awase.log で、手放した直後の最初のかな送信は `[msime-ready] IME mode 未確認` → `IMC ポーリング: NATIVE 確認 → 終了` で毎回(8/8)解決し、出力は NICOLA 文字だった。よって unconfirm は不要と見るが、期限延長の解除(`bump_shift_conv_guard_gen` と override のクリア)を揃えるかは実装時にフォーカス変更の経路(`output/mod.rs`)と見比べて決め、決めた理由を PR に書く。
- 誤って解除した場合(読みが NATIVE だが実状態は半角英数)は、復元の書き込みが二度と走らず、リテラルのローマ字が出る。読みが嘘になるのは ADR-205 が言う MS-IME の「開いているのに 0」の型で、NATIVE の読み(`conv=25`)側では観測されていない(S1 の 18 セルで食い違い 0)。

**D4. MS-IME 本体では英数の軸だけを採り、解除も本体に限る(GJI の挙動は変えない)。** 失敗していた 3 キー(変換・英数・ひらがな)は英数の軸(`open=Some(true)` かつ NATIVE)だけで直り、無変換は `develop` でも PASS。開閉の軸を採らないことで、ADR-205 の「開いているのに 0」の懸念(トグル中に閉と誤読して追随し、Engine が OFF のまま、IME_ON でもリテラルのローマ字になる)を本体側で踏まない。解除を本体に限ることで、既に直接観測が動いている GJI の MS-IME プリセット(`ro` の退行があった領域)の挙動を変えない。代償は「IME は閉・トグルは保持」の組み合わせが残ること(範囲外)と、GJI の追随で `toggle_held` が残る潜在課題を BUG-186 の範囲外として残すこと。スパイクは両軸・GJI 含みで検証したので、**この狭い形でも CI で 4 キー PASS を再検証する**(D6)。

**D5. 予測が付くと S2 は黙って無効になる。** `MSIME_NATIVE` に開・`C10` のセルが無いことを固定する Linux テストを `state/key_effect_table.rs` に足す(変換・英数・ひらがな・無変換、「足すと ADR-244 の追随経路がその打鍵で無効になる」とコメント)。利用者の機械で学習した表(`use_learned_keymap_table`)に `C10` のセルが入ると、予測が belief を動かす一方でトグルが残る。今回は注記にとどめ、将来案として解除の契機を「`toggle_held` の間に belief が awase 自身の書き込み以外で `ObservedEisu` から外れた」に一般化する。

**D6. テストと CI。**
- (a) 回帰テスト: 述語の純関数、解除の純関数(「窓 arm → トグル開始 → NATIVE の読み」では解除しない〈R3〉、Shift 付き・全角英数〈`conv=24`〉・閉の読みでは解除しない、変換中は窓を開かない)、`MSIME_NATIVE` の C10 欠落の固定。
- `tests/architecture_guard.rs::external_change_watch_is_limited_to_imm32_unavailable_and_gji` は、executor の唯一の呼び出しを新しい述語に替えると落ちる。ADR-205 の 3 か所は GJI 限定の述語のまま、ADR-188 の 3 か所(`kp_stage_mode_key_follow`・executor の再送出・`ir_follow_direct_mode_key_read`)は新しい述語、新しい述語の本体に `Imm32Unavailable` と `table_ime_kind` が含まれる、を固定する形に書き直す。コメント(executor・`key_pipeline.rs`・`platform_state.rs`・`ime_refresh.rs` の「GJI × Imm32Unavailable」)も追随する。
- CI: `sc-table-*` は `observe` で既定の実行からも外れているため、そのまま `pass` に格上げしない(48 セルは setup の INVALID の出入りで赤くなりうる)。同定済み MS-IME 本体 × 実 Chrome の「Shift単独タップ → 変換・英数・ひらがな・無変換」の 4 セルだけの専用構成を `expect=pass` で既定の実行に入れる。併せて、トグル外のセルの `[direct-follow]` が 0 件であること(FSM 再送出の窓がかな入力中の単独変換/無変換のたびに開くため)と、`sc-table-gji-*` のセル単位(キー × 周 × 出力文字)の比較を PR に載せる。

**D7. 進め方。** 専用 worktree(`develop` から)で実装し、Opus コードレビュー(同じレビュアーで収束確認)→ CI green → PR → `develop`。実機確認(Chrome × MS-IME 本体)はマージの条件にしない。BUG-186 は「修正済み(CI 検証済み・実機未確認)」と書く(BUG-149/150 と同じ扱い)。v1 へは backport しない(ADR-188 が v1 に無い。PR 本文に 1 行書く)。コミット本文に、戻した案(`024ca336`/`6b1e91b8`)との違い(キー押下ではなく観測された読みを根拠にする。`ro` はセル比較で再現しなかった、run を添える)を書き、`docs/experiments.md` に 1 行足す。ADR-205 に 1 行足す(「直接観測(ADR-188)は ADR-244 で `table_ime_kind` の範囲へ広げた。外部変化の監視は GJI 限定のまま」)。

## 実装と検証(2026-10-08、PR #556)

実装は D1〜D7 のとおり。Opus のコードレビューで次を直した: 専用構成を `--strict`(INVALID・RECOVER も赤)にし、手放す処理の呼び出しと確認ゲートの期限延長の解除を `architecture_guard` で固定した(M-1)/ 手放す条件に「追随後に belief が英数でない」を足した(予測の fence に追随を捨てられたとき手放さない、S-2)/ 確認ゲートの期限延長(`SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS` = 5000ms)の解除と世代更新をフォーカス変更(`on_ime_mode_focus_changed`)と同じ形で行う(M-3、D3 の宿題)/ `runtime/mod.rs` で新しい関数が既存の doc と `#[must_use]` を奪っていたのを直した(M-5)/ 全角英数 `conv=0x18` と MS-IME 本体の R3 の単体テストを足した(S-1)/ `chrome_probe` の `--table-state`・`--table-keys` の書き間違いを終了コード 2 で止める(S-3)。

CI(すべて PR の head `a0a0bdb5` 以降):
- 専用構成 `sc-bug186-msime-shift-toggle-{suppress,passthru}`(`--strict`): 2 構成とも `PASS=8 FAIL=0 INVALID=0`(run 37736666172)。
- `sc-table-msime-*`: 2 構成とも `PASS=48 FAIL=0 INVALID=0`(run 37736669264、修正前は `PASS=42 FAIL=6`)。この run の `[direct-follow]` は 1 構成あたり 6 件(変換・英数・ひらがな × 2 周、すべて `open=None eisu=Some(false) conv=25`)で、トグル外の 18 セルでは 0 件(S-4)。手放しのログも 6 件で対応する。
- `sc-table-gji-*`(4 構成): 本体の追随だけの段階(run 37736672041)では `develop`(run 37716011228)とセル単位(キー × 周 × 出力文字)で差分 0。D8 を足した最終版(run 37739968036)は下記。
- MS-IME 本体 × Chrome の回帰セット 49 ジョブ(run 37734121794、`develop` は 37714889571): FAIL(rc=1)の食い違いは `sc-startup-msime-chrome-on-norefocus2` の試行 5 と 8 の入れ替わりだけで、同じ構成は `develop` の run どうしでも結果が入れ替わる(既存の不安定)。他は INVALID と PASS の出入り。
- PR の CI(`fmt`・`clippy`・`test`・`windows-build`・`dylint`・`smoke` ほか): 最終 head の run で PR #556 に記録する(途中の head `a0a0bdb5` 以降、`architecture_guard` の手放し配線の固定が rustfmt の折り返しで落ち、`b4a10afa` で直した)。

**M-2 の機序(予測付きのモードキーを先に押すと `track.conv` がトグル開始後も残り、トグル中にも予測が付いて直接観測が見送られる)は実在した。** 観測のみの `sc-bug186-pre-{1C,F2}-*`(`chrome_probe --table-pre=`)と、Opus 再レビュー(round 2、`sc-bug186-pre-F2-*` の awase.log)で確かめた。
- トグル開始(`InputModeApplied`)の reducer は `input_mode` だけを書き、追跡 `key_track` に触れない。ひらがな(0xF2)を先に押すと `track.conv=Some(C19)` がトグル中も残り、`predict_in_table` は追跡値を `ObservedEisu`→`C10` より優先して `(open, C19)` の行を引く。
- 無変換(トグル中): 予測が belief を「開・ローマ字」へ動かす(`mode=Some(AssumedRomaji)`)。実 IME は閉じる(S1: `open=false conv=0`)のに Engine だけが ON になり、閉じた IME へ打って入力が空になる(`食い違い=空`)。予測が付いた打鍵では直接観測が見送られ、持続トグルも残る。
- ひらがな(トグル中): 予測で Engine が ON になり出力は PASS だが、直接観測が見送られてトグルが残り、次のケースの setup が INVALID になる。英数は 2 周とも INVALID で検証できていなかった。
- 変換キーを先に押す場合は、変換が追跡を汚さないので影響しない(修正後 8/8 PASS、修正前は `PASS=2 FAIL=3 INVALID=3`〈run 37737446039、修正の本体だけ `develop` に戻した版〉)。
- この症状が [BUG-192](../known-bugs/BUG-192.md)(ひらがな直後のトグル → 無変換で入力が空)。`develop` でも再現していた。

**D8(追加の決定): トグル中は古い追跡を捨てる。** `KeyTrack::without_conv_while_half_width_alnum`(`state/key_effect_predictor.rs`)を `kp_predict_key_effect` の `PredictInput` に当てる。トグル中は追跡の `conv` を捨て(`stage` は残す)、追跡が空の場合と同じ「予測なし」にする。reducer(`ime_model.rs`)は触らず、トグル開始時の追跡の消去(案 b)は採らない(belief の書き込み点を増やさない)。回帰は `sc-bug186-pre-F2-{suppress,passthru}`(`--strict`、`expect=pass`、既定の実行に入れる)と単体テスト(`key_track_forgets_conv_only_while_half_width_alnum_toggle_is_active`・`msime_native_toggle_keys_have_no_prediction_once_stale_conv_is_dropped`)。

**D8 の副作用(GJI の MS-IME プリセットが良くなる)**: 追跡を捨てる範囲はトグル中の全 IME に及ぶ。`sc-table-gji-*`(run 37739968036、修正前 `develop` は run 37716011228)で、`gji-atok-*` はセル単位で差分 0、`gji-msimepreset-*` は「Shift単独タップ後 → 変換」が FAIL(`ローマ字のまま〈英数なのに Engine ON=未追随〉`)から PASS(`ka`)になり、無変換は INVALID(`持続半角英数にならなかった`)から PASS になった(passthru: `PASS=28 FAIL=4 INVALID=16` → `PASS=32 FAIL=2 INVALID=14`、suppress: `PASS=30 FAIL=2 INVALID=16` → `PASS=34 FAIL=0 INVALID=14`)。PASS から FAIL へ悪化したセルは無い。BUG-186 の症状欄が書いていた「GJI の MS-IME プリセットでは変換でローマ字のまま」も、同じ原因(トグル中の古い追跡による予測)だったと読める。

**本体のトグル外でも英数の軸の追随が新しく効く(Opus コードレビュー S-5)**: `classify_direct_read_for(MsIme)` はトグルの有無に関係なく英数の軸を採る。例: かなで Engine ON 中の Shift+無変換(予測なし、S1 で `conv=24` = 0x18 = 全角英数、NATIVE なし)が NATIVE なし → `ObservedEisu` → Engine OFF になる。実状態への追随なので方向は正しい。develop には無かった挙動で、ADR-188 追記 6 型の「処理前の読み」の懸念は、S1 の最初の読みが 31〜47ms で窓内一定という範囲では問題にならない(上の `sc-table-msime-*` でトグル外の 18 セルに誤った追随は無い)。

## 残る課題・範囲外

- **変換中にトグルした後の変換キー**: ADR-188 M2(変換中は窓を開かない)のため、この修正では直らない(ADR-107 はトグル開始を変換中でも許す)。
- 学習表の上書きで `C10` のセルが入った場合にトグルが残ること(D5)。
- GJI の追随で `toggle_held` が残る潜在課題(D4)。
- GJI MS-IME プリセットの `ro` の退行そのものの修正。
- S1 の残り: `None` の読み失敗の頻度、`ro` の機序。
