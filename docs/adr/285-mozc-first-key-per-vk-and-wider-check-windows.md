---
id: ADR-285
title: |-
  Mozc の cold の最初の送信を「最初の 1 キー」と「残り」に分け(per-VK)、確認を Chrome 系と空以外のすべての窓に張る
summary: |-
  ADR-270 は Mozc の cold の最初のローマ字を一度にバッチ送信し、入力中の文字ができたかを共有メモリで確かめる。バッチのため、リテラルのときに素通りした文字数が事前に分からず、ADR-284 が `key_seq` の読みと Escape で数える仕組みを足した。GJI/Chrome の per-VK 確認は 1 文字ずつ送るので素通りは常に 1 文字で、Backspace は 1 回に決まる。Mozc も同じ形にする。
  決定: (1) 確認を張る送信は、最初の 1 キーだけ送り、`MozcComposeCheckFsm` が入力中の文字ができた(`phase >= 1`)と確かめてから残りを `TransmitRest` で送る。判断しない場合も残りは必ず送る。(2) リテラルなら Backspace ×1 + 全体の再送。(3) 確認を張る窓を「Windows Terminal・WezTerm だけ」から「Chrome 系と空のクラス以外のすべて」に反転する(BUG-226: LINE で確認が張られず `kおちら`)。ADR-284 の `key_seq` による検出は安全網として残す。
status: |-
  起草・実装(2026-10-28、PR #904)。実機は未確認。
related_adr:
  - "ADR-270"
  - "ADR-284"
---

# ADR-285: Mozc の最初の 1 キーの per-VK 確認と、確認を張る窓の反転

## 背景(実測)

- 2026-10-28 07:41:55 UTC(16:41 JST)、dragonflyg4、LINE(`Qt663QWindowIcon`)、Mozc 系。70.9 秒空いたあとの cold の最初の送信 `ko` が `kおちら` になった。当該の送信のログに `[mozc-compose]` が無く、確認が張られていなかった(`is_mozc_compose_check_window` が Windows Terminal・WezTerm だけ)。BUG-226。
- Backspace の数: GJI/Chrome の per-VK 確認(`probe_fsm.rs`)は 1 文字ずつ送るので、疑いが出たキーだけが素通りし Backspace は 1 回に決まる。Mozc の ADR-270 はバッチ送信で、全体リテラルは「送ったローマ字の文字数」、部分リテラルは ADR-284 の `key_seq` の不足分で数えていた。ADR-270 の記録に、per-VK にしなかった理由は書かれていない(「D4: GJI の仕組みを Mozc へ持ち込まない」が事実上の理由と読める)。
- 窓の限定の根拠: Chrome の TSF 再初期化 ~326ms(`79134f5`)が確認の期限 300ms を越えうる(PR #809 M3)。他の窓は「測っていないから外した」だけで、外す実測の根拠はなかった。

## 決定

1. **最初の 1 キーだけ送る**: 確認を張る送信(`mozc_compose_precheck` が対象を返し、ローマ字が英小文字 2 文字以上、`split_first_key`)は、最初の 1 キーだけを送る。残りは `MozcComposeTarget::rest` に持つ。
2. **確認できたら残りを送る**: `MozcComposeCheckFsm` の結論が `Done` で終わるときは `TransmitRest { romaji: rest }` に置き換える(確認できた・判断しない・読み直しの終わりのいずれも)。残りの送信は確認を張り直さない。待つ間の後続のキーは既存の保留(`pending_tsf`)で止まる。
3. **リテラルなら 1 回だけ消して全体を再送**: `RawTsfLiteralRecovery { backs: 1, romaji: 全体 }`(`full_romaji`)。素通りしたのは最初の 1 キーだけ。連続 2 回目の諦める分岐は、最初のキーより後ろ(`romaji_after(全体, 0)`)だけを送る(既存)。
4. **宛先の窓が変わったら残りは送らない**: 最初の 1 キーは元の窓に入っている。変わった窓へ文字を送らない。
5. **窓の判定を反転**: `is_mozc_compose_check_window` は Chrome 系(`Chrome_WidgetWin_1`・`Chrome_RenderWidgetHostHWND`)と空のクラス以外のすべてで真。Chrome は `confirmed after Nms` の分布を取るまで外したまま。他の窓を除く必要が見つかったら、実測の根拠と一緒に足す。
6. **ADR-284 の `key_seq` による部分リテラルの検出は残す**: 最初の 1 キーを確かめたあとの残りには起きないはずだが、撤去は実機で効かないことを確かめてから(複雑性予算: 追加と撤去の対応は、実機確認後の別 PR)。

新しい定数は足さない(待ちは `MOZC_COMPOSE_CHECK_MS` のまま)。

## 影響・限界(未確認)

- 遅れ: 確認できるまで(正常で 15〜31ms、`[mozc-compose] confirmed after`)残りの送信が遅れる。cold の最初の送信だけ。
- 窓の拡大: 確認を張る窓が増えるので、物理 F2 で warm でも cold にする判断(`physical_f2_marks_cold`、同じ述語)も増える窓で働く。Mozc 系の窓だけ。
- Escape・Backspace の意味がアプリで違う窓(vim の端末など)で、回収の誤爆の害は 1 キー分になった(以前は文字数分)。誤判定自体の頻度は、Chrome 以外の窓で未測定。
- Chrome 系は対象外のまま(Chrome の GJI 経路は別の per-VK 確認が張られている)。Chrome×Mozc で同じ漏れが見つかったら別途。
- ログ: `[mozc-compose] cold=N checking composition after sending "k"`(最初の 1 キー)の行で、送った分が分かる。
