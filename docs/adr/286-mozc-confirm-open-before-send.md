---
id: ADR-286
title: |-
  Mozc Readable で、belief が予測で ON なのに共有メモリが閉じているときは、送る前に開くのを待ち、開かなければ F2 を 1 回再送する
summary: |-
  BUG-227。F2 の予測(KeyEffectPrediction)だけで ON と信じてローマ字を送ったが、Mozc の共有メモリは閉じたままで `kakino` が素通りした。ADR-270 の確認は送信直前に effective_open=true を要求するため、この場面では張られない。決定: (1) 対象は Mozc Readable が読め、共有メモリ effective_open=false かつ belief ON(予測由来)のときだけ。(2) 打鍵を保持し、共有メモリが true になるのを短く待つ(上限は CI で測って決める。実機の 1 例は F2 から 43ms)。(3) 開かなければ、ひらがなキーを 1 回だけ再送してもう一度待つ。(4) それでも開かなければ belief を OFF に戻して保持した打鍵を素通りさせる。
status: |-
  起草(2026-11-04)。未実装。待ち時間の上限は CI で複数の窓を測ってから決める。
related_adr:
  - "ADR-270"
  - "ADR-285"
---

# ADR-286: Mozc の送信前に ON を確認する

## 背景(実測)

BUG-227 のとおり。2 回目の F2 では 43ms で共有メモリが `tsf_open=true` になった。1 回目は 1.4 秒後も `false` のままで、待つだけでは救えない。

## 決定

1. 対象: Mozc Readable が読め、共有メモリ `effective_open=false`、belief が ON(`KeyEffectPrediction` 由来)の窓。ほかの窓・IME は従来どおり。
2. 保持して待つ: 打鍵を保持し、共有メモリが `true` になるまで待つ。上限 `MOZC_OPEN_CONFIRM_MS` は **CI で窓の種類と idle の長さを変えて測ってから決める**(暫定の目安は 50ms。tuning-constants の実測義務)。
3. 開かなければ再送: ひらがなキーを 1 回だけ再送し、もう一度同じ上限まで待つ(Mozc ではひらがなキーは ON にするだけで、再送しても害がない)。
4. それでも開かなければ、belief を OFF に戻し、保持した打鍵をそのまま通す。

## 影響・限界(未確認)

- 遅れ: 食い違いのときだけ、上限分の遅れ。
- 1 回目の F2 が効かなかった理由は未特定。再送で救えるかは実機で確かめる。
