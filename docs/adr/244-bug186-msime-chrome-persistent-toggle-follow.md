---
id: ADR-244
title: |-
  実 Chrome × MS-IME 本体で、半角英数の持続トグル中の IME 側モードキーに Engine を追随させる(BUG-186)
summary: |-
  BUG-186: 左 Shift 単独タップで持続半角英数にしたあと、変換・英数・ひらがなを押すと IME はかなへ戻るが Engine は OFF のまま(`か`)。
  ADR-188 第1段(PR #539)の後も最新 develop で再現する。run 37704093235 のログでは、かなに戻った実状態は約 26〜45ms 後に正しく読めているのに、
  打鍵中扱い(TypingActive)で捨てられ、ADR-188 の直接観測は GJI 限定の述語で MS-IME 本体には一度も動いていない。
  本 ADR は現状の事実と、追加で測ること・決めることの順序を定める。修正方式は決めない。
status: |-
  ドラフト(2026-10-08、未実装、S1 の計測結果と S2 スパイクの CI 検証〈48/48 PASS〉を追記済み)。Opus レビュー 2 回(1 回目 Blocker 2・Must 4・Should 6、2 回目 Blocker なし・Must 2・Should 3)を反映済み。
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

## 決めること(順序、方式は未決)

- **S1(追加で測る)**: 上の読み(約 26〜45ms)は 1 標本群で済んでいる。追加で測るのは、窓内で値が一定か、30 秒 idle 後・変換中・無変換(閉)の後はどうか(ADR-188 追記 3 と同じ観点を MS-IME 本体で)。併せて `ro` 退行の機序(どの run のどのセルを基準にしたか)を調べる。
- **S2 と S3 は二者択一に近い**: `state/platform_state.rs::follow_direct_read_in_scope` は、打鍵時点の予測(`key_effect().at_ms >= armed_at`)が付いた打鍵では直接観測を採らない。S3(学習表に「開・C10」を足す)をすると S2 の観測経路はその打鍵で無効になり、逆に S2 を採れば S3 は不要になりうる。S1 の後に、どちらで行くかを決める。
- **S2 の問い**: (i) GJI 限定の述語を MS-IME 本体へ広げてよいか(ADR-188 追記 6 の `[key-effect-miss]` 対策との関係)。(ii) 観測で belief を `ObservedEisu` から外しても `gate.half_width_alnum.toggle_held` は true のまま残り、次の読みは `shift_conv_guard_active` で凍結され、次の Shift タップの意味も食い違う。S2 はトグルの解除に必ず触れる。解除を読み(NATIVE)でどう守るか。

## 範囲外

- 修正方式の選択(この ADR は決めない)。
- GJI MS-IME プリセットの `ro` の退行そのものの修正。
