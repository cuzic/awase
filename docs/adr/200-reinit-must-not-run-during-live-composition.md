---
id: ADR-200
title: |-
  chrome-reinit(VK_IME_OFF→ON)は SuspectedLiteral の証拠が2回そろったときだけ送る(StaleConfirm では reinit しない)
summary: |-
  Chrome+GJI で、StaleConfirm(否定的証拠なしの誤検出)が2連続すると give-up が reinit を送り、入力中の未確定文字が全部消える(BUG-168、CI で2件)。
  awase なしの対照で VK_IME_OFF→ON は24/24全消失(CI の GJI)。決定: (1) reinit は「最後の CompositionConfirmed 以降に SuspectedLiteral が累計2回以上」あったときだけ(連続でなくてよい)。give-up 自体(再送の打ち切り)は従来どおり consecutive で行う。
  (2) StaleConfirm の romaji 再送(BUG-075 の重複)・猶予20msは変えない(引き金は猶予不足ではなく deferred 一括送出後の GJI 停止)。(3) 単体テストと CI の A/B、対照ハーネスの修正。
  未決: Escape 経路(per-VK idx≥1 の ESC)の破壊性、reinit の他の呼び出し元、実機での reinit 破壊性。
status: |-
  草案(2026-09-26、opus-adversarial-consult round1〜3 で収束、実装前)。未決事項はリスク節。
related_adr:
  - "ADR-079"
  - "ADR-100"
  - "ADR-153"
  - "ADR-156"
---

# ADR-200: reinit は否定的証拠が2回そろったときだけ

## 背景と事実(2026-09-26 の CI 実測、Chrome + GJI、windows-latest)

- 実 Chrome(`chromebar`=アドレスバー・`chromepage`=ページ内 textarea)へ NICOLA 打鍵を 2〜30ms 間隔で注入、読み戻しは UI Automation。awase あり 4,200試行(取得済み artifact)で失敗8件(0.19%)。人間の打鍵の10倍以上(2ms)の条件が中心。
- 8件の内訳: **本件(StaleConfirm→give-up→reinit で消失)2件**(run 36222195067 ページ 35文字、run 36225188542 アドレスバー 23文字)、StaleConfirm 1回の再送による文字重複1(BUG-075 系)、起動直後の IME モード不整合2、awase 主スレッド約7秒停止1(2試行)、読み戻しが GJI 処理完了前だった疑い1。
- **reinit の破壊性(対照実験、awase なし・raw 20ms・打鍵終了300ms後に送信、`ts-raw-*-reinit-*`)**: `VK_IME_OFF→VK_IME_ON` は各フォーム12試行、計24/24で未確定文字が全消失。何も送らない対照は0/24。
  `VK_IME_OFF` 単独は、有効な試行(各 run の #0)4/4で全消失。#1 以降は IME が OFF のまま打鍵してローマ字化するので無効(ハーネスが試行間で IME を ON に戻さない)。
  `VK_DBE_HIRAGANA`(F2)は未確定文字を壊さない(偶数試行 PASS)。奇数試行の失敗は F2 で IME が OFF に切り替わったためのローマ字化。
- **BUG-036(実機、2026-07-23)は同じ連鎖で「commit されて literal が残った(tみや)」と観測**。本 ADR の CI 対照とは食い違う。差の候補: GJI の版、reinit 時に preedit が短い(1文字)か長い(23〜35文字)か、ハーネスは打鍵終了後300ms待機・awase はバックログ中に OFF/ON を連続送信、scan 値の違い。未解明。
- 実利用への結びつき: 2ms は人間の打鍵の10倍以上。CI の打鍵試行で StaleConfirm が出たのは2〜10ms間隔だけで、30ms 以上の試行では0件(ready 段階の1文字確認では出る)。BUG-036 は通常速度の実機で連鎖が起きた記録。

## 原因連鎖

1. 直前の語で deferred VK を一括送出(38〜63 VK)すると GJI の I/O が止まる(実例: 223ms 以上)。この間に次の語を per-VK 確認し、`visible_fencing_verdict` が StaleConfirm を返す。猶予を p99 まで延ばしても両方 Stale になるので、引き金は猶予不足ではない。
   (`since_vk_sent_ms` は GetTickCount の15.6ms刻みでメインスレッドの消費時刻を測るので、猶予を決める根拠には使わない。)
2. StaleConfirm は否定的証拠を持たない(BUG-075)。それなのに `probe_io.rs` の `RawTsfLiteralRecovery` 処理は SuspectedLiteral と同じ `consecutive` に数える(`mark_cold_raw_tsf` が無条件に増やす)。
3. 2連続で give-up となり `schedule_chrome_gji_reinit` が VK_IME_OFF→ON を送る。BUG-033 の前提「2連続 literal = GJI が本当に OFF」は SuspectedLiteral では成り立つが、StaleConfirm では成り立たない。
4. reinit が生きた preedit を破棄する(対照実験)。

## 決定

**決定1: reinit は SuspectedLiteral が累計2回そろったときだけ送る。**
`consecutive`(再送を打ち切るための連続失敗カウンタ)は従来どおり StaleConfirm も数える(これを外すと Stale 再送が自走して無限に続き、重複文字が増え続ける)。
それとは別に**否定的証拠カウンタ**を `ColdContext`(`tsf/probe.rs` の `raw_tsf_literal_consecutive_count` の隣)に持つ。
- 増やすのは `RawTsfLiteralRecovery` 分岐の中で `facts.verdict == SuspectedLiteral` のときだけ(`mark_composition_cold` 側には入れない。どの verdict でも増えてしまう)。
- リセットは `consecutive` と同じ3か所(`CompositionConfirmed` の dispatch=per-VK 途中の confirm を含む、FocusChange/SetOpenTrue、`on_focus_changed`)。
- **評価順序**: 同じ回収の中で、今回の verdict が SuspectedLiteral なら**先にカウンタを増やし、増やした後の値**で give-up 時の reinit の可否を判定する(判定を先にすると S,S でも2回目の判定時点でカウンタが1になり、BUG-033 の回復が消える)。
- give-up 時にカウンタ(増加後)が2未満なら reinit を予約せず cleanup のみで終える。S,S は従来どおり reinit(BUG-033 の回復を保つ)。S,U,S は累計2回で reinit する。S,U / U,S / U,U は reinit しない。最新 verdict だけでは判定しない(U→S で証拠1回のまま reinit が走るのを避ける)。
- **単一スロット保護を保つ**: reinit しない give-up でも、先行する reinit が Scheduled または Polling のあいだは、現行の `schedule_pending_gji_reinit` と同じく `set_raw_literal` を呼ばず cleanup を抑止する(`SuppressedExistingScheduled`/`SuppressedExistingPoll` の判定を共用する。「reinit を予約するか」の引数を足す形が安全)。抑止しないと、先行 give-up の backspace 数・escape が上書きされる(BUG-074 系、`probe_io.rs` の Angle A #1 テストと同じ回帰)。

**決定2: StaleConfirm 時の romaji 再送と猶予20msは本 ADR では変えない。**
再送で文字が重複する問題は BUG-075(suffix 再送は6ラウンドの対話設計で致命的欠陥が見つかり revert 済み。「着弾したかの事後推測」は証拠なしの仮定になる)。未送信分だけを再送する案は同じ罠なので採らない。
決定1のあと、Stale 起因の give-up では romaji が捨てられ、送り済みの子音が残る(今回の事例を再生すると、`k` が2つとも composition に入れば「っくてとせ」型、ローマ字のまま出れば「kkてとせ」型になると推定)。35文字の消失より小さいが残る誤りとして BUG-168 に記録する。
猶予20msは延長しない(引き金が猶予不足ではない、`tuning-constants.md` の実測義務)。

**決定3: 検証。**
(a) `probe_io.rs` の FakeIo テスト: S,S→reinit予約あり / S,U,S→あり(累計) / U,U→なし / S,U→なし / U,S→なし / Confirmed(per-VK 途中の相乗り confirm を含む)でカウンタがリセット / 先行 reinit が Scheduled・Polling のとき reinit しない give-up がスロットを上書きしない / reinit しない give-up では romaji を再送しない(決定2で許容した残りの誤りを固定)。S,S のテストは「2回目の回収の直前のフィクスチャでカウンタ=1」の形で書く。既存の give-up テスト(`probe_io.rs` の `…consecutive_gives_up_with_cold_mark` など、`consecutive: 1` と SuspectedLiteral の facts だけで2回目を表すもの)は、否定的証拠カウンタ=1(先行 S あり)のフィクスチャに更新する。「予約しない側の Suppressed」用のテストを別に足す。Output 側の分岐(既存 pending があれば予約しなくても Suppressed を返し、無ければ pending を作らない)のテストは `schedule_pending_gji_reinit_does_not_overwrite_scheduled_phase` の隣に置く。`output` は `#[cfg(windows)]` なので、Linux では `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` で確かめ、実行は windows-build CI。
(b) CI A/B: `ts-chromebar-gji-2ms` と `ts-chromepage-gji-2ms` を修正の前後で各 N run(発生率は約1/100試行なので数百試行必要)。決定的に再現させるため、ハーネスに「60 VK 一括送出の直後に OFF/ON」を足す。
(c) 対照ハーネスの修正: 試行ごとに `turn_ime_on`、awase と同じ間隔・scan で OFF/ON を送る、`--reinit-after=esc` と `burst+off_on` を足す。修正後の対照で reinit の破壊性を再確認する。

## 却下・保留した案

- **reinit の撤去**: BUG-033(GJI が本当に direct-input のまま literal 化し続ける)の回復手段を失う。
- **reinit 前に Enter で確定**: アドレスバーで Enter は検索を実行し、ページ内で Enter は改行。
- **Escape**: 本ADRの範囲外だが**既存コードが既に ESC を送っている**(`per_vk_recovery_params` の idx≥1 は Stale/Suspected とも `escape_composition=true`)。2ms 打鍵のように前の語の未確定が残る状況で ESC が全体を取り消す可能性があり、`--reinit-after=esc` の対照(決定3c)で確かめてから別の決定にする。
- **StaleConfirm を CompositionConfirmed に倒す**: ADR-079 の epoch fencing を壊す。
- **StaleConfirm を consecutive に数えない**: Stale 再送が無限に続く。
- **猶予の延長**: 上記のとおり引き金ではなく、実機の分布も無い。

## リスク・未決

- **回復の低下(R2-2)**: 候補窓が残ったまま GJI が本当に OFF の場合(ADR-079・`93bb36a7` の「kれでできる」型)、候補窓が可視の per-VK 確認は Confirmed か Stale しか返さない(SuspectedLiteral は不可能)。旧コードは U,U で reinit して IME を ON に戻せたが、決定1では否定的証拠カウンタが増えず、`consecutive` も戻らないので、次の語も idx=0 で即 give-up して romaji を捨て続け、候補窓が消えて SuspectedLiteral が2回出るまで語が失われる。旧 U,U の reinit がこのケースで実際に役立っていたかは、実機の不具合報告 journal で `gave_up=true` かつ StaleConfirm の後に回復した例を探して確かめる。判別の候補(事後推測ではなく awase 自身が持つ状態): 「直前 N ms 以内に自分で deferred を一括送出したか」(今回の2件に共通)、「give-up の後に GJI の write が再開したか」。決定ではなく未決事項。
- **S,S でも生きた preedit を壊す余地(R2-4)**: 「候補窓は見えないが preedit は長い」状態(GJI のバックログが300msを超えサジェスト窓が HIDE した場合など)で idx=0 の SuspectedLiteral が2回出れば、同じ全消失が起きる。コーパスの SuspectedLiteral による give-up 7件はすべて ready 段階で、試行中は0件(頻度は低いが、起きない理由の証明は無い)。ready 段階の reinit の直後に ready が失敗した例が3件ある(「起動直後」の失敗の一部は awase 自身の reinit の疑い)。
- **idx≥1 の失敗に打ち切りが無い(既存)**: per-VK の途中の confirm が `consecutive` を0に戻すので、idx≥1 の失敗(ESC＋全体の再送)は毎回0から始まり、現行コードの時点で打ち切りが無い。本 ADR の範囲外。

- reinit の破壊性は CI の GJI での観測。実機で `off_on` 対照を1回回すまで一般化しない(BUG-036 の食い違い)。
- 他の reinit 呼び出し元(`Output::send_f22_f21_reinit`、Unicode モードの long-cold)は preedit が空の前提だが未監査。`CHROME_GJI_REINIT_CONFIRM_MS` のレート制限と `Suppressed*` 分岐(cleanup も romaji も捨てる)との相互作用も未整理。
- 上流: 2ms の構成では毎語が cold path(`vk-send` warm=true が0件、gji-fsm が OffCold のまま)で、一括送出と Stale が起きやすい。`chrome-per-vk-confirmed` のあとセッションが warm にならない理由は別途調査する(直れば Stale に至る経路の大半が消える可能性)。
- 起動直後の IME モード不整合(全角英数など)と、awase 主スレッド約7秒停止は本 ADR の対象外。
