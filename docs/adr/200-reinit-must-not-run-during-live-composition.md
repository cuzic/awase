---
id: ADR-200
title: |-
  chrome-reinit(VK_IME_OFF→ON)は生きた composition があるとき送らない(StaleConfirm 起因の give-up では reinit しない)
summary: |-
  Chrome+GJI で、StaleConfirm(否定的証拠なしの誤検出)が2連続すると give-up が reinit を送り、入力中の未確定文字を全部破棄する(BUG-168、
  awase なしの対照実験で VK_IME_OFF→ON は12/12・OFF単独12/12・F2 6/12 が全消失、何も送らない対照は0/12と実証)。
  決定: (1) give-up→reinit は最新の verdict が SuspectedLiteral(候補窓不可視＝生きた composition が無い)のときだけ許す。StaleConfirm 起因では reinit しない。
  (2) StaleConfirm の猶予20ms・romaji再送(BUG-075の重複)は本ADRでは変えない(実機分布の計測が先)。(3) 回帰テストと、reinit 前提を監視する CI 対照実験。
status: |-
  草案(2026-09-26、opus-adversarial-consult 待ち)。
related_adr:
  - "ADR-079"
  - "ADR-100"
  - "ADR-153"
  - "ADR-156"
---

# ADR-200: reinit は生きた composition があるとき送らない

## 背景と事実(2026-09-26 の CI 実測、Chrome + GJI、windows-latest)

- 実 Chrome(アドレスバー `chromebar`・ページ内 textarea `chromepage`)で NICOLA 打鍵を 2〜30ms 間隔で注入、読み戻しは UI Automation。
  awase あり約4,700試行で失敗8件(0.2〜0.7%)。連続打鍵の定常状態は約2,000試行で失敗ゼロ(BUG-165 の修正は効いている)。
- 8件の内訳: reinit による全消失1、StaleConfirm 再送による文字重複1、起動直後の最初の試行4(awase なしでも発生)、読み戻しが早すぎた疑い1、awase 主スレッド約7秒停止1。
- **reinit の破壊性(対照実験、awase なし・raw ローマ字20ms・入力中に送信、`ts-raw-*-reinit-*`)**: `VK_IME_OFF→VK_IME_ON` 12/12試行、`VK_IME_OFF` 単独 12/12、`VK_DBE_HIRAGANA` 6/12 で未確定文字が全消失(アドレスバー/ページとも)。何も送らない対照は 0/12。
  `probe_io.rs` のコメントと BUG-36 は「未確定 preedit は commit される」前提だが、Chrome+GJI では commit されず破棄される。
- **StaleConfirm の実測**: `since_vk_sent_ms` の分布(CI、約1,430判定)は CompositionConfirmed p50=16/p90=32/p99=63ms、StaleConfirm 11件(0.8%)。猶予 `EPOCH_FENCE_GRACE_MS`=`GJI_SAMPLE_INTERVAL_MS`×2=20ms は正常系の p90 より短い。

## 原因連鎖

1. 候補窓が可視のまま GJI の I/O サンプルが送信時刻に追いつかず、`visible_fencing_verdict` が StaleConfirm を返す(誤検出。GJI は実際には受理している。BUG-075 と同じ現象)。
2. StaleConfirm は「着弾しなかった」という否定的証拠を持たない(BUG-075)。それなのに `probe_io.rs` の `RawTsfLiteralRecovery` 処理は SuspectedLiteral と同じ `consecutive` カウンタに数える。
3. 2連続で give-up となり `schedule_chrome_gji_reinit` が VK_IME_OFF→ON を送る(BUG-33 の前提「2連続 literal = GJI が本当に OFF」は SuspectedLiteral では成り立つが、StaleConfirm では成り立たない)。
4. reinit が生きた composition(35文字の preedit)を破棄する。実例: run 36222195067 の cold=9・10。

## 決定

**決定1: reinit は「生きた composition が無い」証拠があるときだけ送る。**
`RawTsfLiteralRecovery` の give-up 分岐(`probe_io.rs`)で、最新の verdict が `SuspectedLiteral`(deadline まで候補窓・I/O ともゼロ＝composition が無い)のときだけ `schedule_chrome_gji_reinit` する。
`StaleConfirm`(候補窓は可視)では reinit せず、`consecutive` をリセットして cleanup のみで終える。判定は `facts.verdict`(`LiteralDetectFacts`)で行い、新しい状態は持たない。

**決定2: 猶予20ms・StaleConfirm 時の romaji 再送(BUG-075 の文字重複)は本 ADR では変えない。**
猶予の延長は `tuning-constants.md` により実機の実測が要る。CI の分布(p99=63ms)は windows-latest のもので、実機では違う可能性が高い。
BUG-075 は診断フィールド(`grace_hold_ms` 等)で実機データを集める段階のまま。本 ADR は「重複」より重い「全消失」だけを、新しい機構なしで止める。

**決定3: 回帰テストと前提の監視。**
(a) `probe_io.rs` の FakeIo テストに「StaleConfirm 起因の give-up では `schedule_chrome_gji_reinit` が呼ばれない」「SuspectedLiteral 起因では呼ばれる(BUG-33 の回復を保つ)」を足す。
(b) CI 対照 `ts-raw-{chromebar,chromepage}-gji-20ms-reinit-{off_on,off,f2,none}` を残す。Chrome/GJI が将来 reinit で preedit を保持するようになったら、決定1の制限を緩められる。

## 却下した案

- **reinit の撤去**: BUG-33(GJI が本当に direct-input のまま literal 化し続ける)の回復手段を失う。SuspectedLiteral 起因は残す。
- **reinit 前に Enter / Escape で composition を確定・破棄**: アドレスバーで Enter は検索を実行し、ページ内で Enter は改行になる。Escape は破棄で結果が同じ。
- **猶予を p99(63ms)以上に延長**: 実機分布が無い。遅延を入れる副作用の評価も未了(決定2)。
- **StaleConfirm を CompositionConfirmed に倒す**: ADR-079 が守っている epoch fencing(前世代の候補窓を現世代の証拠にしない)を壊す。

## リスク・未決

- reinit を抑止すると、StaleConfirm の裏で GJI が本当に OFF だった場合の回復が1回遅れる(次の SuspectedLiteral で回復)。
- 他の reinit 呼び出し元(`Output::send_f22_f21_reinit`、Unicode モードの long-cold)は preedit が空の前提で、本 ADR では未監査。
- 起動直後の最初の試行の失敗(awase なしでも発生)と主スレッド約7秒停止は、本 ADR の対象外(別件、原因未特定)。
