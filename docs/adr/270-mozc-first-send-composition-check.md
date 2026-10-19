---
id: ADR-270
title: |-
  Mozc で、cold の最初のローマ字を送った直後に、入力中の文字ができたかを共有メモリで確かめ、リテラルなら消して送り直す
status: |-
  実装中(2026-10-19)。純粋な判断(`state/mozc_shm.rs`)・読み取り(`observer/mozc_shm.rs`)・FSM(`tsf/warmup/mozc_compose_check_fsm.rs`)・配線(`output/`)を PR にした。実機は未確認(BUG-209)
related_adr:
  - "ADR-265"
  - "ADR-267"
  - "ADR-268"
  - "ADR-269"
---

# ADR-270: Mozc の cold の最初のローマ字の確認と回収

## 背景

Mozc Readable(実機 dragonflyg4、Windows Terminal)で、窓の切り替えやひらがなキーの直後の**最初の 1 打鍵だけがリテラル**(`wo` `ke` などのアルファベット)になり、2 打鍵目から正常に変換される症状がある(利用者の報告「けいり → ke いり」「をもと → wo もと」)。

**実測(2026-10-19、`RUST_LOG=debug` の awase.log)**: 「をもと」の最初の `wo`(09:23:04)は、`[tsf-send] warm=false ... prepend_f2_warmup=true`(`reason=NativeF2Consumed`、cold)→ `[gji-coro] GjiProbe 完了 (16ms gji_idle=69478156ms settled=false)`(Mozc は GJI の I/O が動かないので、待たず準備の確認もしない)→ `[tsf-transmit] romaji="wo" → vk-run (eager=false)`(`W`・`O` のキー送信)。続く「もと」は入力中の文字が普通にできた(最初のキーのあとで Mozc が目覚めた)。cold の処理は 13 分で 13 回走り、リテラルは確認できたもので 1〜2 回。

この症状は、[BUG-208](../known-bugs/BUG-208.md) の修正(cold の最初の打鍵を、かな直接〈unicode〉からローマ字のキー送信に戻した)が**露出させた**。修正前は最初の 1 打鍵が unicode で、リテラルにならなかった(代わりに、以後のほぼ全てが unicode で送られる不具合だった)。

**旧版の ADR-270(PR #689)の訂正**: 旧版は、約 7 秒 `phase=0` が続いた記録(UTC 06:47)を「Mozc が入力中の文字を保持しない」と読んで、Mozc 側の原因を想定していた。実際は、awase が GJI 用の応答待ち(`PendingGjiConfirm`)を Mozc でも続け、かなを直接送っていたこと(BUG-208)が原因だった。

## 決定

### D1. 判定: cold の最初のローマ字の送信から 0.3 秒たっても入力中の文字ができない、かつ Mozc サーバーが応答できる状態

- 対象: cold の stage の最初の送信で、**ローマ字を VK で送った**とき(unicode 送信は組成を作らない)。送信の直前に、共有メモリ(version 3 以降、80 バイト)で次をすべて確かめる: 読める、TIP が有効、IME が開いている、入力中の文字を作るモード(ひらがな・全角カタカナ・半角カタカナ)、まだ入力中の文字が無い(`phase == 0`)、入力スコープが指定していない(パスワード欄など)、送るのが英小文字だけのローマ字(`should_check_compose`)。
- 判定(`judge_compose`、10ms ごとに送った先のスレッドを読む): `phase >= 1` なら正常(`Confirmed`)。期限に達しても `phase == 0` で、`server_ready == 1` なら**リテラル**(`Literal`)。読めなくなった・TIP が無効になった・IME が閉じた・サーバーが応答できないときは**判断しない**(`Skip`)。
- **サーバーが応答できないときは消さない**: 最初のキーが Mozc に食われて何も出ていない(握りつぶされた)場合に、Backspace が直前の正しい文字を消すおそれがある。サーバーが応答できる状態で入力中の文字ができないなら、キーは Mozc に食われずアプリへリテラルで届いたと判断できる。

### D2. 待つ時間は 0.3 秒(`tuning::MOZC_COMPOSE_CHECK_MS`、pending)

利用者の許容(「0.3 秒程度なら待たせてもよい」)。実測(`shm3_rec` の共有メモリの記録と awase のログの突き合わせ、入力中の文字が空から始まった打鍵の塊 19 件): 最初の打鍵から入力中の文字ができるまでは 0.02〜0.06 秒、実機のリテラル(「wo もと」)は 0.34 秒。サンプルが小さく 1 台・1 アプリなので pending。確認ログ `[mozc-compose] ... composition confirmed after Nms` の分布で見直す。

### D3. 実装: FSM(`TickableFsm`)で、既存のリテラル回収を再利用する

- `MozcComposeCheckFsm`(`tsf/warmup/mozc_compose_check_fsm.rs`)は、cold の stage の終わりに `pending_tsf` へ張る(`Output::step_probe` の段末、deferred の解放より前)。張られている間、後続のキーは既存の保留(`defer_vks_if_probe_or_recovery_in_flight`)で待たされ、確認の終わりで解放される。
- リテラルなら `ProbeAction::RawTsfLiteralRecovery { backs = 送ったローマ字の文字数, romaji, escape_composition: false }` を出す。**消す文字数を数える機構は作らない**: 確認の対象を最初の 1 回の送信だけにして、その間は後続のキーを保留するので、文書に出ている誤りは送ったローマ字だけ。回収は既存の経路(Backspace と再送、cold の印付け、連続失敗の打ち切り〈2 回目は消すだけで再送しない〉、宛先が変わったら回収を捨てる BUG-194)をそのまま使う。
- 正常なら `CompositionConfirmed`(連続失敗の数え直し)。

### D4. やらないこと

- GJI の仕組み(I/O の監視、F2 の待ち、`LiteralDetect`)を Mozc へ持ち込まない。Mozc の共有メモリの `phase` は、推測ではなく Mozc 自身の事実で、GJI の間接的な推測より直接。
- warm の送信(cold でない)は確認しない(頻度が高く、`phase != 0` のことが多い)。cold の最初の 1 回だけ。

## 限界・未確認

- **部分的なリテラル**(`ke` の `k` がリテラルで `e` が組成を作る、GJI の「bあ」型)は、入力中の文字ができているので検出できない。全体がリテラルになった場合(実機の `wo`)だけを拾う。
- `server_ready` の意味は mozc-readable 側で「仮説の段階」(ADR-267 D10)。0 のまま 1 にならない環境では、リテラルでも消さない(安全側)。
- 実機で効くか、再送が成功するかは未確認。別の cold の最初の打鍵(warm の送信での失敗)の有無も未確認。
- Windows Terminal(`Windows.UI.Input.InputSite.WindowClass`)以外のアプリ(Chrome・WezTerm)は未測定。

## 却下した案

- **cold の最初の打鍵だけ、かな直接(unicode)に戻す**: リテラルは出ないが、最初の 1 文字が変換の対象から外れる(「を」+「もと」)。
- **GJI と同じ cold の待ち(固定時間)**: 実測なしの定数を増やす(規約で禁止)。Mozc は共有メモリで準備の事実を取れる。
- **旧版の「検知を awase.log に出すだけ」(段階 1)**: 原因が分かり、回収に必要な情報(文字数・安全条件)も決まったので、検知と回収を同時に入れる。
