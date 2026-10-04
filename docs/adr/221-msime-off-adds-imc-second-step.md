---
id: ADR-221
title: |-
  MS-IME の OFF は VK_IME_OFF の後に IMC_SETOPENSTATUS(0) を同じ機構の第2ステップとして補う
summary: |-
  TSF の入力先(実 Chrome 等)に未確定 composition が残っている間に MS-IME が VK_IME_OFF(0x1A)を受けると、閉じずに conv が 25→16 になるだけで開いたままになる(OS への単独注入=awase なしでも同じ、確定。docs/tasks/msime-chrome-off-rca-2026-10-04.md)。IMC_SETOPENSTATUS(0) は composition が有っても 10/10 閉じる。`MsImeDirect` の OFF だけ、VK 送信後に同期 IMC(OFF) を足す。新しい合流点・新しい許可呼び出し元は増やさない。ADR-208 決定4 例外(a)の理由(環境制約)は誤りで、本 ADR で訂正する。
status: |-
  採用(2026-10-04、所有者決定: v2.0.0 に含める)。実装済み(`ime_controller.rs::followup_after_vk`)。Opus レビュー(重大3・中6)を反映済み。CI は PR 参照。実機(Windows 11)での「打っている途中で無変換」の確認は未実施。
related_adr:
  - "ADR-208"
  - "ADR-163"
  - "ADR-180"
  - "ADR-089"
  - "BUG-184"
---

# ADR-221: MS-IME の OFF に IMC(OFF) の第2ステップを足す

## 背景(確定した原因)

詳細な実験表は [docs/tasks/msime-chrome-off-rca-2026-10-04.md](../tasks/msime-chrome-off-rca-2026-10-04.md)。要点:

- MS-IME は、TSF の入力先(実 Chrome、RichEdit の TSF 窓)に**未確定の composition が残っている間に `VK_IME_OFF`(0x1A)を受け取ると閉じず**、conv を 25→16(半角英数)に変えるだけで開いたままにする。`IMC_GETOPENSTATUS` は 1 のまま。
- awase なしの OS 注入でも同じ(awase のせいではない)。二重送信(0〜400ms 間隔)でも閉じない。composition が無ければ 10/10 閉じる。IMM32 のみの EDIT 窓は composition が有っても閉じる。
- `IMC_SETOPENSTATUS(0)` は実 Chrome でも composition が有っても 10/10 閉じる。GJI では IMC は効かない(API は閉になるが打鍵は `か` のまま)。GJI は VK_IME_OFF だけで composition 有りでも 10/10 閉じる。
- 試作(VK の後に同期 IMC(OFF))で CI の MS-IME × Chrome/tsf の OFF 系が全収束し、MS-IME の `sc-*` 36 構成の比較で回帰なし(補完が走った 9 ジョブの範囲)。

## 決定

1. **`MsImeDirect` の OFF(`ImeOperation::Close`)・明示キー押下由来の起案・chain が ImmCross を含まない窓(Imm32Unavailable/TsfNative)だけ**、`VK_IME_OFF` の送信が成功した直後に、同じ機構の第2ステップとして `IMC_SETOPENSTATUS(0)` を同期で書く。ON 方向(VK_IME_ON は composition が有っても開く)・GJI(IMC が効かない)・**押下に由来しない起案(`ActuationOrder::press()==None`: drift correction・リピート)と、ImmCross を先に試した fallback(Standard)には入れない**。`explicit_press` は ADR-208 L1 の押下台帳の ID(二重送信防止が本来の目的)を「composition を消してよいか」の判断に流用している。**`press` の付け方を変える(L3' 等で載せる起案が増減する)と補完の範囲も黙って変わる**(`press_id_is_claimed_and_carried_at_every_order_issuing_entry` は「載っていること」しか見ない)。
2. **補完するかは純粋関数 `state::key_sequence_policy::post_vk_followup(機構, 向き, 明示押下か)`** が決め、`ime_controller.rs::followup_after_vk` が **`SendVk` の両アーム(GjiDirect・MsImeDirect)から実引数で呼ぶ**(決定表が実行時に配線されている。GJI のアームに IMC を足しても、`post_vk_followup` を変えても、テストと golden が検知する)。Win32 呼び出しは `followup_after_vk` の 1 箇所。`explicit_press` は `ImeController::apply` と `run_open_chain_async` が `order.press().is_some()` を `order` 消費前に取り、`SyncChainWriter`/`AsyncChainWriter` 経由で `apply_mechanism` へ渡す。
3. **同期経路(`SyncChainWriter::write`)と非同期経路(`fallback_write`)はどちらも `apply_mechanism` を通る**ので同じ決定になる(`open_chain.rs` に分岐を足さない)。**MS-IME × Standard は、ImmCross が `Failed` を返したあとに MsImeDirect へ fallback する**(golden: `MS-IME Standard async_fallback → MsImeDirect`)。ここで補完すると、たった今ワーカースレッドで失敗した IMC をメインスレッドで `with_app` を握ったまま同期に再送することになり(効く見込みが低く、ブロックだけが増え、`SendHealth` ゲートもこの 1 回は抜ける)、**補完しない**(`post_vk_followup` の `imm_cross_in_chain` = `imm_cross_applicable(profile)`、golden に固定)。ImmCross が成功する窓は MsImeDirect を通らない。
4. **ブロックの扱い(既存の非同期 ImmCross より弱い)**: 既存の非同期 ImmCross は `offload` でワーカースレッドから `SendMessageTimeoutW` を呼ぶが、本補完はメインスレッド(フック/メッセージループ)で同期に呼ぶ。同等なのは `romaji_pre_write` と同期 ImmCross(`SetOpenCrossProcessSync`)だけ。上限は 150ms だが、`SMTO_ABORTIFHUNG` は呼び出し中に相手がハングし始めると最大 ~5s まで止まりうる(`send_health.rs`「初回の ~5s ブロックは防げない」)。`SendHealth::blocking_allowed` のゲートは「100ms 超が連続 2 回」で作動するので弱く、間に速い呼び出しがあると作動しない。待っている間は自スレッド宛て送信メッセージを処理する(再入、`fallback_write` の `with_app` は再入時に失敗する)。`romaji_pre_write` と違い OFF のたびに走るので頻度が高い。**この弱さを受容する**(RCA の `send_elapsed` は 0〜29ms)。ゲートで見送った場合は、`romaji_pre_write` がゲートを外した理由(再試行が無く静かに固着する)がそのまま当てはまり、修正前と同じ「半角英数に取り残される」状態で outcome は `Applied` になる。
5. **offload を採らない理由**(訂正): 遅れて着弾する点は同期(`SendMessageTimeoutW` はタイムアウトしても取り消されず IME 窓が後で処理する)でも同じで、却下理由にならない。実際の理由は (a) outcome を同期で返したい(`ImeOpenOutcome::Applied` が VK 送信を意味する)、(b) `ActuationTarget`(HWND)を `Send` にする必要があり、offload 後の再入・世代照合の窓を新設する(BUG-34 型)ため。メインスレッドを止める代償は上記の受容。
6. **合流点・許可リスト**: `RESTRICTED_CALLS` は変更しない(補完は `set_ime_open_for_target` = `actuate_ime_control` の許可呼び出し元経由)。ただし**新しい IMC_SETOPENSTATUS の書き口として `ime.rs::set_ime_open_for_actuation_target` を 1 つ足した**ので、`architecture_guard` で `capture_blocking(` の件数ピンを 1→2(`romaji_pre_write` + `followup_after_vk`)に更新し、2 本目の所在と `set_ime_open_for_actuation_target(` の呼び出し元(1 箇所)を関数単位で固定した。`apply_mechanism` の呼び出し元(2 箇所)は不変。
7. **宛先の同一性は検証しない**: `ActuationTarget` は `capture_blocking`(`GetGUIThreadInfo` 30ms → `GetForegroundWindow`)が返す HWND と `focus_gen` を持つだけで、`set_ime_open_for_actuation_target` は照合しない(`focus_gen` は恒真の照合ですらない)。VK の着弾先と IMC の宛先は、Alt+Tab 直後などフォーカス遷移と重なれば別の窓になりうる(低頻度)。INV-14 を満たしているとは読まないこと。
8. **記録**: 補完は `(mechanism, open, explicit_press)` から決まるが、**実際に IMC が届いたか**は `blocking_allowed`・`capture_blocking`・書き込みの成否で変わり、outcome(常に `Applied`)にも `AttemptRecord`/journal にも載らない(tracing の `[apply-ime] MS-IME direct: IMC_SETOPENSTATUS(0) 補完 ok=` ログのみ)。replay は `MechanismCommand`(`SendVk`)の再生差分 0 までしか証明しない。journal だけからは「VK のみで終わった」と「IMC まで届いた」を区別できない。
9. 複雑性予算: 追加は純粋関数 1・enum 1・関数 1・引数 1(`explicit_press`)。新しい合流点・tuning 定数は 0。

## 副作用と影響範囲(「打っている途中で無変換」)

- IMC 経由の閉じは **composition を取り消す**(`compositionupdate`→`compositionend`、`text_post=''`)。従来の VK だけの OFF は composition を残して半角英数にしていた(ただし IME は開いたままで OFF にならない)。**打っている途中で OFF を押すと、未確定の文字が消える**。
- **BUG-184 との関係**: BUG-184 は MS-IME × Chrome で「OFF で未確定文字が消える。確定してから OFF にしてほしい」という**実際の利用者の未対応の報告**(ImmCross 経路)で、本 ADR はその症状と**逆向き**の要望に対し、同じ症状を ImmCross を使わない窓(Imm32Unavailable/TsfNative)にも広げる。「戻す条件」(利用者が困ったら外す)は BUG-184 の報告で既に部分的に満たされていると読める。RCA の `text_post=''` は BUG-184 の「未確認: ImmSetOpenStatus(FALSE) で未確定が破棄されること」の証拠。**『確定してから OFF』の次の一手(`ImmNotifyIME(CPS_COMPLETE)`)は HIMC が他プロセスから取れず、`WM_IME_CONTROL` に確定コマンドも無いので、awase からはそのままでは実行できない見込み**(別 ADR の前段に置く逃げ道は現実的でない)。所有者が v2.0.0 に含めると決めた取捨はこの前提で再確認が要る。
- **影響する起案の全列挙**(`apply_mechanism` の MsImeDirect の OFF に至る経路): `press` が載るのは、Engine の特殊キーのコンボ(`stamp_set_open_press`)・`keys.ime_*`・単独タップ(`request.press`)・`kp_shadow_actuate`(トグルの OFF・無変換単独タップ・Ctrl+無変換・D4)だけで、これらは**補完する**。**エンジン切替のホットキー(`toggle_engine`)とトレイの「エンジン切替」の OFF は利用者が明示的に押しても `press=None`**(`transition_activation` が `SetOpen{press:None}`)なので**補わない=MS-IME × Chrome では修正前どおり半角英数に取り残される(BUG-185 が残る経路)**。補う(別フラグで「明示操作」を印す)かは複雑化を避けて見送った。`runtime/ime_refresh.rs` の drift correction は押下に由来しない(`press=None`)ので**補完しない**(誰も押していないのに打っている途中の未確定文字を消さない。修正前と同じく VK のみ)。
- **直らない経路**: (a) トレイメニューは最初から IMC。(b) 物理キーを Allow で OS へ通す OFF(英数 0xF0 等)・`shadow_action` の無い物理 `VK_IME_OFF` が Allow される場合は MS-IME 自身の処理で、composition 中は本 ADR が直した症状(閉じない)がそのまま残りうる。
- GJI、ImmCross 先頭の窓は不変。
- **IMC は VK より先に処理されうる**: `SendInput` は入力キュー経由、`WM_IME_CONTROL` は送信メッセージで、受け手が次にメッセージを取得したときにキュー上の入力より先に処理される。実際の順序は「IMC(0) で閉じる → キューに残った打鍵 → VK_IME_OFF(閉なので何もしない)」になりうる(コード上の順序は VK→IMC)。VK は composition が無いときの本来の経路・IMC が効かない窓の保険として残る。リスク: 直前に注入した romaji がまだ Chrome のキューにあるうちに IMC が処理されると、`か` が `ka`(ASCII)になる新症状。CI `sc-offrca-fix-msime-race`(`race0/10/20`)が、`k`,`a` の KeyUp の 0/10/20ms 後に、待ち・probe を挟まず OFF を出し、OFF 直後のページの文字(`text_post`)に ASCII が無いことで判定する。ただしこれは物理キーを `SendInput` した直後の OFF で、awase 自身が注入した romaji の in-flight(NICOLA の同時打鍵の確定直後)を再現するものではない。実機の追確認が残る。
- 利用者に見える変化: 修正前は「OFF のつもりが半角英数に取り残され、ON キーでかなに戻る」(害は小さい=RCA R4)。修正後は「OFF が確実に効くが未確定文字は消える」。
- **戻す条件**: 実機で「打っている途中の OFF で、VK のみのときより利用者が困る」(確定済みの文字まで消える、`か`→`ka` 化け等)が確認された場合、本補完を外して ADR-208 例外(a)に戻す。

## ADR-208 への訂正

ADR-208 決定4 の例外(a)(MS-IME × 実 Chrome の `VK_IME_OFF`)は「環境の制約」と書かれていたが、原因は **MS-IME の挙動(composition が残る間の VK_IME_OFF は閉じない)× awase が VK だけで OFF していたこと**で、awase 側で直せる。例外(a)は本 ADR で撤回する(ADR-208 側に追記)。CI の実 Chrome ハーネスの `ensure` が毎回 `k,a` を打って composition を残すため、OFF が閉じない側だけが毎回再現して『環境固有』と見えていた。

## 検証

- 純粋関数: `key_sequence_policy.rs::post_vk_followup_only_for_explicit_ms_ime_close`(press 無し・ImmCross を含む chain を含む)。golden: `ime_key_sequence_golden.rs`(MS-IME の OFF・press あり・Standard 以外だけ IMC を含み、GJI・ON・press 無し・Standard は含まない。Windows のみ)。ガード: `architecture_guard.rs::sync_romaji_write_goes_through_a_captured_target`。
- CI(常設、`e2e-ime.yml`): `sc-offrca-fix-msime-awase`(Ctrl+無変換、composition を残す/打鍵後/打鍵なし)、`sc-offrca-fix-msime-race`(打鍵の KeyUp の 0/10/20ms 後に待ち無しで OFF、`text_post` に ASCII が無いこと)、`sc-offrca-fix-gji-awase`(GJI 対照)、`sc-keymatrix-*-chrome-msime` を observe から pass へ昇格。判定は `check_offrca.py --expect-closed` = **made 全試行で実打鍵が ASCII になること**(`typed_closed`)。`--or-then` のある試行は、続く ON でかな入力に戻ること(`then.open`、conv=16 に取り残されない)も要求する。API の読み戻し(`IMC_GETOPENSTATUS`)は修正自身が書く値なので判定に使わない。
- 実機: 未実施(Chrome で語を打っている途中に無変換 → 続く文字が ASCII か、未確定文字が消えるか、`か`→`ka` 化けが出ないか)。
