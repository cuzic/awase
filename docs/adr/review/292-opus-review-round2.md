# ADR-292 Opus 敵対的レビュー round2

対象: `b32819c79`(ブランチ `docs/adr292-key-effect-prediction-vs-observation`)の ADR-292
round1 の指摘(R1〜R9)の反映は確認した。D1 の格下げ、F1・F2・G1・H1 の却下、位相による説明、R6 の 3 択の明記は妥当。以下は、**新しく書いた G3・P・R6 の 3 択そのものの弱点**と、まだ残っている誤りを挙げる。

結論を先に書く。

- **G3 は今の形では BUG-232 を再発させる**。`last_mode_key_tick_ms` は「書き込みを**要求**した時刻」で、awase が IME キーを実際に**送った**時刻ではない。今のローリング baseline は送った時刻を(偶然)拾っているので、G3 にするとその保護が消える(S1)。
- **P の「既存の仕組みをそのまま使える」は誤り**。warm の送信には送信前に読む場所が無い。保留の単位(ローマ字か物理キーか)、保留の解放条件、報告の経路もまだ決まっていない(S3〜S6)。
- **R6 の (ii) の「保留分を捨てる」は不可**。(i) は機構と二重の書き込みの危険が未検討(S7・S8)。
- 原因の特定(CI 計画の 1)は、CI より先に**手元の報告の journal・ログを読む**方が安く、確実(S9)。

---

## G3 の弱点

### S1(致命的): `last_mode_key_tick_ms` は書き込みの要求の時刻で、実際の送信の時刻ではない。BUG-232 の保護が消える

- `ImeApplyRequested` は `handle_engine_set_open`(`platform_state.rs:1200-1209`)などで、**書き込みの要求を受けた時点**で dispatch される。`last_mode_key_tick_ms` が進むのはこの時刻だ(`ime_model.rs:893-896`)。
- 実際の書き込みはその後に起きる。executor のキュー、`run_open_chain_async` の gate のポーリング、`fallback_write` の待ちを通ってからで、数十〜数百 ms 遅れる。Mozc は GJI 系に分類されている(PR #663)ので、送る手段は `GjiDirect` だ。これは `apply_mechanism` → `ime::send_ime_mode_key`(`ime_controller.rs:112-115`)で **SendInput でキーを注入する**。この送信時刻は `imm_cross_write_tick_ms` には入らず、`hook_alive_tick_ms` にだけ入る(自己注入キーもフックを通るため)。
- G3 で baseline を `max(last_mode_key_tick_ms, imm_cross_write_tick_ms)` にすると、次の順で BUG-232 が再発する。
  1. T0: 明示 OFF の意図が記録される。
  2. T0+X: GjiDirect が OFF のキーを注入する(X は数十〜数百 ms)。
  3. T0+170+ε: Mozc がまだ注入キーを処理していない古い `open=true` の読みが、T0 から 170ms を過ぎているので `Use` になる。
  - X が 170ms を超える場面(cold の GJI の F キーは約 750ms 遅れる、`experiment-logging.md` の `098c663` の例)では確実に起きる。
  - 今のローリング baseline は T0+X の注入をフックで拾っているので、この穴が無い。BUG-232 の単体テスト(baseline=明示 OFF の時刻)は G3 でも通ってしまうので、**単体テストではこの退行を検出できない**。
- **要求**: G3 の起点に「awase が IME の開閉・モードを変える目的で実際に送った時刻」を加える。`note_imm_cross_write` と同じ形で、`send_ime_mode_key` などの注入の直後に打つ `note_ime_actuation_send` を作り、baseline を `max(last_mode_key, imm_cross_write, ime_actuation_send)` にする。打刻の対象は次のものを全部洗い出す。
  - `apply_mechanism` の GjiDirect・MsImeDirect の両方
  - drift correction の再送
  - ADR-245 の半角英数トグルの注入
  - ADR-255 の Space 置換(Space が開閉を動かさないなら不要。その根拠を書く)
  - `ime_profile_driver` 系
  - 物理キーの通過(`ModeKeyPassedThrough`)
  
  ADR の「要検証」の箇条(214-217 行)は、検証項目ではなく**設計の必須要素**に格上げする。

### S2(中): G3 を計算する場所。`mozc_shm_shadow.rs` では計算できない

- `architecture_guard.rs:8680` の `mozc_shm_reader_touches_belief_only_through_report_mozc_shm` は、`mozc_shm_shadow.rs` に `KeyEffect`・`ImeEvent` などの語を書くことを禁じている。今 baseline を作っているのは殻の `mozc_shm_shadow.rs:157` だ。G3 の起点を作る `last_mode_key_tick_ms` は belief の中にあるので、起点の合成は `report_mozc_shm`(`platform_state.rs:2409`)の中で行う必要がある。
- 殻からは S1 の打刻(hook 側の Atomic)だけを渡す。`decide_use` は「一致する読み用の baseline(今のまま)」と「食い違う読み用の baseline(G3)」の 2 つを受け取る形になる。どちらの baseline がどちらの判定に効くかを単体テストの表で固定すること。
- `last_mode_key_tick_ms` は envelope の `tick_ms` だ。本番でこれが `GetTickCount64` と同じ時計か(`HubClock`・`TickMs` の出所)を確かめて書くこと。`decide_use` の `now_ms`・`update_tick_ms` は `GetTickCount64`。

### S2'(小): `FocusChanged` で `None` になったときの規則が無い

- フォーカス直後は `last_mode_key_tick_ms = None` になる(`ime_model.rs:898`)。その場合の起点を ADR に書くこと。round1 で示したとおり、今の `last_operation_tick_ms` に戻すのが安全。

---

## P の弱点

### S3(重大): warm の送信には送信前に読む場所が無い。「既存の仕組みをそのまま使える」は誤り

- `mozc_compose_precheck` が呼ばれるのは、cold の probe 経路の `ProbeAction::Transmit`(`probe_io.rs:507`)の 1 か所だけだ。warm の送信(`send_romaji_batch_immediate` など)には送信前に読む場所が無い。BUG-233 では `ko` が cold の経路を通ったかさえ未確認(ADR 188-190 行)なので、**P がこの事例に効くかは、どちらにしても新しい配線次第**になる。
- P を warm の送信にも掛けるなら、出力の層(`output/`)が belief の状態(予測が未照合か)を知る必要がある。`output/` は `platform_state` を直接は読めない(層の境界)。runtime 側(`key_pipeline` の送信を決める段)で判断し、その結果を出力に渡す形にすること。置き場所の候補は、Engine の出力を送る直前の `runtime/key_pipeline.rs` の段。

### S4(重大): 何を保留するか。ローマ字ではなく物理キーを保留すべき

- P は「ローマ字を送らずに保留」と書いている。ローマ字は Engine が belief=開 を前提に変換した**結果**だ。保留の後に R6 で (iii)(belief を閉に戻す)を選んだ場合、保留したローマ字を元の物理キーに戻す手段は無い。NICOLA の同時打鍵の判定は済んでおり、`ko` を `k`・`o` の生キーとして出すのは利用者の打鍵ではない。
- **保留の単位は Engine に入る前の物理キーにする**こと。既にある `focus_switch_hold`(PR #809 M1)・`resume_hold`(ADR-245 追補 2 R2)と同じく、取り込み口で保留し、解放時に `INPUT_DEFER.replay_later` で Engine に流し直す。こうすれば解放時の belief に従って正しく変換される。この形だと P は「送信前の照合」ではなく、**「予測が未照合の間の打鍵の保留(第 3 の hold)」**になる。ADR の P の定義を書き直す必要がある。
- 再生の際に、親指シフトの同時打鍵の判定のための打鍵時刻の間隔を保つか(既存の hold の再生がどう扱っているか)を確かめて書く。

### S5(重大): 保留の解放条件と読みの周期が無い

- P の「閉なら保留」には、いつ解放するかが無い。
  - 開いた → 解放して送る
  - 閉で確定 → R6 の行動
  - 期限 → ?
- 確かめるための読みが 500ms のタイマー任せだと、保留は最大 500ms 以上になる。保留中だけ短い周期(16〜30ms、ADR-270 の確認の FSM と同程度)で読む必要がある。
- 保留中の読みも `decide_use` を通すこと。モードキーから 170ms 以内の「閉」の読みは、Mozc がまだ処理していない古い読みかもしれない(round1 の R4' と同じ理由)。P が「閉と読んだら即座に閉と確定」すると、速く打つ利用者(モードキーから 50〜100ms で次の打鍵)で G1 と同じ退行が起きる。P の判定は「開と読めた(一致する読み、高速路で可)→ 解放」「起点(S1 の G3 の起点)から settle を過ぎて閉 → 確定」「それ以外 → 保留を続ける」の 3 値にする。
- 新しい保留は、`fix-requires-evidence.md` の「defer/replay キューの解放条件」のファミリーに入る。フォーカス変更・panic・設定の再読み込み・期限・Engine の非活性化の、全ての解放の窓口に配線すること(ADR-123→128 の回帰と同型の漏れを防ぐ)。既存の 2 つの hold と同時に保留した場合の順序も決める。

### S6(中): 観測の報告の経路

- P は「この読みを観測として報告する」と書いている。mozc-shm の読みを belief に入れる経路は `report_mozc_shm` の 1 つに限られている(上の guard と ADR-268)。P の読みも `report_mozc_shm` を通すこと(Held/Use の判定と `[key-effect-miss]` の照合が同じ規則になる)。`output/` や hold の殻から直接 `ObserverReported` を出さないことを明記する。

### S6'(中): 逆向きの予測(open=false)が対象外

- P の条件は「予測が開」の場合だけだ。予測が閉(IME OFF のキー)で Mozc が実は開いたままなら、Engine は非活性になり、打鍵は生のまま開いた Mozc に入って、qwerty のローマ字として変換される。症状は逆向きで同型だ。Engine が非活性の打鍵はローマ字の送信を通らないので、P の「送信前」では捕まえられない。S4 のように取り込み口で保留する形なら、この向きも同じ仕組みで扱える。対象に含めるか、含めないなら範囲外と明記すること。

---

## R6 の 3 択の弱点

### S7(重大): (ii) の「保留分を捨てる」は不可

- リテラルは目に見えるので、利用者は消して打ち直せる。保留した打鍵を黙って捨てると、利用者は気づかないまま文字が消える(BUG-074「give-up で文字消失」と同じ型。memory の 2026-10-04 の項目)。期限の後は「belief を閉にして保留分を Engine に流し直す」(S4 の物理キーの保留なら可能)にすべきで、捨てる選択肢は ADR から消すこと。

### S8(重大): (i) 開き直しの未検討のリスク

1. **機構**: モードキーが Zed に届かなかったのが原因なら、awase の注入キー(GjiDirect の VK)も同じ窓に同じ形で届く。同じ理由で効かない可能性が高い。ImmCross は `Imm32Unavailable` だ。(i) が成り立つかは原因(S9)次第で、効く機構が無ければ (i) は空の選択肢になる。
2. **二重の書き込み**: Mozc が単に遅く(例: 600ms 後に)開いた場合、awase の開き直しと重なる。送るキーがトグル(VK_KANJI など)なら閉じてしまう。冪等な「開く」キー(VK_IME_ON、または Mozc の keymap で IMEOn に割り当てたキー)に限ると明記すること。
3. **合流点の規約**: 開き直しは IME への新しい書き込みの入口になる。ADR-119 の合流点の表(`fix-requires-evidence.md`)の 3 点、すなわち `claim_press_write`・`with_press`・`explicit_press_applied_pair` と、`RESTRICTED_CALLS` の許可リスト、`architecture_guard` の `press_id_is_claimed_and_carried_at_every_order_issuing_entry` に従う必要がある。押下の ID は、素通しにしたモードキーのものを使うのか、押下に由来しない(drift correction と同じ `press=None`)のかを決めること。
4. **IntentStore**: ADR 262-265 行の注意は正しい。加えて、`report_mozc_shm` の食い違いの追随は `pass_through_observed(true, true)` で `desired_open` を閉に揃える。(i) を選ぶなら、開き直しの判断は**この追随より前**に行わないと、開く意図の根拠が消える。

### S8'(中): 選択肢 (iv) が抜けている

- 「belief を閉に直す + 保留分を流し直す + 利用者に知らせる(バルーンやインジケーター)」。開き直しの機構が無い窓での最善は、たぶんこれだ。memory の 2026-10-23 の方針(「バルーンは残す」)とも合う。

---

## 原因の特定と CI 計画

### S9(中): CI の前に、手元の報告を読む

- CI 計画の 1(モードキーがなぜ効かなかったか)は、まず BUG-233 の報告 journal・awase.log で分かる可能性がある。
  - `[diagnostics] mozc_shm_shadow` の記録が有効なら、20.634 と 23.134 の読みの v5 ブロック(`key_seq`)が残っている。両者で `key_seq` が進んでいれば TIP はキーを見ていた。進んでいなければ、モードキーも ko/re も TIP に届いていない。後者なら、Zed で Mozc の TIP が効いていない(打ったローマ字がそのまま出たことと合う)ことになり、「フォーカス直後」ではなく「その窓では TIP が入力を受けていない」可能性が出る。
  - ADR-270 の確認が張られていれば、`[mozc-compose]` の行に `key_seq` が出る(`literal_facts.rs:103` の `mozc_key_seq`)。
- CI で Zed を使うなら Zed の Windows 版で再現できるかをまず確かめる。`herdr-app.exe` が手に入らないなら、それを前提に書く。
- 実機のバイナリが `752b35ec5` だという根拠(awase.log の起動行など)を ADR に 1 行書くこと。コミット時刻(2026-11-12 03:15 UTC)は事象(11:24 UTC)より前なので矛盾は無いが、今の ADR には確かめた手段が書かれていない(`agent-handoff.md` 3)。

### S10(小): CI 計画の 4 に足す項目

- S1 の退行の確認: 明示 OFF の後、cold の GjiDirect の送信が 170ms 以上遅れる状況で、古い開の読みが `Use` されないこと。単体テストでは検出できないので、`tickchurn` の変種で、送信の遅れを入れた CI のシナリオとして足す。
- P(hold)を採るなら、保留中にフォーカス変更・panic・設定の再読み込みが来たときに、保留分が失われず二重にも再生されないこと。

---

## 小さな指摘

- `opus-review-adr292-round1.md` がリポジトリの最上位にコミットされている(`b32819c79` の stat)。develop にマージする前に撤去するか `docs/` 配下へ移すこと(`one-off-verification-workflows.md` の撤去の考え方と同じ)。ADR の summary はこのファイルを参照しているので、撤去するなら参照も PR 番号かコミットに替える。
- ADR 125 行の `reconcile_key_effect_open` の行番号は 800-831(793 は `impl` の行)。
- summary 42 行の「F2/G1/G2/G3 はこれしかしない」の F2・G1 は既に却下・除外済み。G3 だけにする。

## まとめ

| 対象 | 判定 |
| --- | --- |
| G3 | 方向は正しいが、実際の送信の打刻を起点に加えないと BUG-232 が再発する(S1)。計算は `report_mozc_shm` の中で(S2) |
| P | 「送信前の照合」から「予測が未照合の間の物理キーの保留」へ定義し直す。解放条件・読みの周期・報告の経路・defer のファミリーの配線を設計に含める(S3〜S6) |
| R6 | (ii) の「捨てる」を削除(S7)。(i) は機構・冪等性・合流点の規約・追随との順序を明記(S8)。(iv)「直す+流し直す+知らせる」を追加(S8') |
| 原因の特定 | CI より先に手元の journal の `key_seq` を確かめる(S9) |
