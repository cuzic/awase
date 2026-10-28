---
id: ADR-284
title: |-
  Mozc の cold 最初の送信で、先頭の文字だけ素通りした部分リテラルを検出して回収する(key_seq の進みの読み直し + Escape + Backspace + 再送)
summary: |-
  ADR-270 の確認(`MozcComposeCheckFsm`)は、入力中の文字ができれば確認済みとして終わる。`ko` の `k` だけを TIP が見ず `o` から入力中になる型(「kおちら」、BUG-225、2026-10-28 実機)は確認済み扱いで見逃されていた。
  共有メモリの `key_seq`(TIP が見たキーダウンの通番)の進みが送った文字数より少なければ部分リテラルと判断する。
  確認の読みは最短 16ms で、2 打目がまだ数えられていないだけのことがある(確認済み 21 回中 2 回、+30ms で揃った)ので、進みが足りないときだけ 30ms 待って読み直し、まだ足りなければ回収する。
  回収は既存の `RawTsfLiteralRecovery`(`escape_composition: true`): Escape で入力中の文字(TIP が見た分)を捨て、足りない文字数だけ Backspace で文書の素通り分を消し、ローマ字全体を再送する。連続 2 回目は既存の打ち切りで消さない。
status: |-
  起草(2026-10-28)。実装済み(PR)。実機は未確認。
related_adr:
  - "ADR-270"
---

# ADR-284: Mozc の部分リテラル(先頭だけ素通り)の回収

## 背景(実測)

2026-10-28 00:33:38 UTC、dragonflyg4(Mozc Readable)。フォーカス移動のあと F2→R で `ko` を送信し、`[mozc-compose] composition confirmed after 15ms (sent 2 chars, key_seq 330→331)`、`settled +30ms ... settled 1`。画面は `kおちら`(BUG-225)。`ti`・`ra` は正常。10-21 の「`dえ`・`kおちら`」「先頭だけ素通りは検出できていない」の実測での確認。

## 決定

1. **検出**: 確認(`phase >= 1`)の時点で `key_seq − 送信前` が送った文字数(`compose_literal_backs`)に満たなければ、`Done` にせず `COMPOSE_SETTLE_MS`(30ms)待つ。待つ間も後続のキーは既存の保留(`pending_tsf`)で止まっているので、進みは他のキーで増えない。
2. **判断**(純粋、`state/mozc_shm.rs::judge_settled`): 読み直した進みが送った文字数以上なら通常の確認。1 以上で足りなければ `PartialLiteral { missing }`。0 は矛盾(入力中の文字があるのにキー 0)なので判断しない。
3. **回収**: `RawTsfLiteralRecovery { backs: missing, escape_composition: true, romaji: 全体 }`。Escape で入力中の文字を捨てるので、Backspace は文書へ出た分(`missing`)だけでよい(入力中の文字の長さはローマ字の長さと一致しない: `o`→`お`)。再送は既存どおり cold の印付けから。
4. **安全側**: 次は回収しない(`Done` のみ): 入力中の文字が消えている(`phase 0`)・IME が閉じた・読めない・宛先の窓が変わった・対象のスレッドが応答しない(`window_thread_responds`、`Literal` と同じ)・連続 2 回目(`probe_io` の打ち切り)。

## 数値の根拠(`COMPOSE_SETTLE_MS` = 30)

新しい定数ではなく、10-21 に観測専用で置いた読み直しの待ち(30ms)を判断に使うだけ。実測: 確認済み 21 回中 2 回が送った文字数より 1 少なく、+30ms 後は揃った(10-21)。BUG-225 の件は +30ms 後も 1 のまま。

## 未確認(実機で測る)

- 欠落が常に先頭の文字か。末尾の欠落では文書の順序が違い、Escape + Backspace で直らない可能性がある。
- Escape が Chrome・Windows Terminal・Electron で入力中以外の副作用(ダイアログを閉じる等)を起こさないか(GJI の部分リテラル回収で既に使っている)。
- 回収の再送が再び部分リテラルになるときは、既存の打ち切り(2 回目は消さない)に任せる。

## 検証

`judge_settled` の単体テスト 3 本。FSM の `settle_step` 4 本は windows-build CI。Opus レビューを通す。
