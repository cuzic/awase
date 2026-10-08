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
  ドラフト(2026-10-07、未実装)。Opus レビュー 2 回(1 回目 Blocker 2・Must 4・Should 6、2 回目 Blocker なし・Must 2・Should 3)を反映済み。
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

## 決めること(順序、方式は未決)

- **S1(追加で測る)**: 上の読み(約 26〜45ms)は 1 標本群で済んでいる。追加で測るのは、窓内で値が一定か、30 秒 idle 後・変換中・無変換(閉)の後はどうか(ADR-188 追記 3 と同じ観点を MS-IME 本体で)。併せて `ro` 退行の機序(どの run のどのセルを基準にしたか)を調べる。
- **S2 と S3 は二者択一に近い**: `state/platform_state.rs::follow_direct_read_in_scope` は、打鍵時点の予測(`key_effect().at_ms >= armed_at`)が付いた打鍵では直接観測を採らない。S3(学習表に「開・C10」を足す)をすると S2 の観測経路はその打鍵で無効になり、逆に S2 を採れば S3 は不要になりうる。S1 の後に、どちらで行くかを決める。
- **S2 の問い**: (i) GJI 限定の述語を MS-IME 本体へ広げてよいか(ADR-188 追記 6 の `[key-effect-miss]` 対策との関係)。(ii) 観測で belief を `ObservedEisu` から外しても `gate.half_width_alnum.toggle_held` は true のまま残り、次の読みは `shift_conv_guard_active` で凍結され、次の Shift タップの意味も食い違う。S2 はトグルの解除に必ず触れる。解除を読み(NATIVE)でどう守るか。

## 範囲外

- 修正方式の選択(この ADR は決めない)。
- GJI MS-IME プリセットの `ro` の退行そのものの修正。
