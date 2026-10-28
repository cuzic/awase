---
id: ADR-284
title: |-
  Mozc の cold 最初の送信で、先頭の文字だけ素通りした部分リテラルを検出して回収する(key_seq の進みを期限まで読み直し + Escape + Backspace + 再送)
summary: |-
  ADR-270 の確認(`MozcComposeCheckFsm`)は、入力中の文字ができれば確認済みとして終わる。`ko` の `k` だけを TIP が見ず `o` から入力中になる型(「kおちら」、BUG-225、2026-10-28 実機)は確認済み扱いで見逃されていた。
  共有メモリの `key_seq`(TIP が見たキーダウンの通番)の進みが送った文字数より少なければ部分リテラルの疑いとして、確認で終えず毎 tick 読み直す。揃えば(2 打目の処理が遅いだけ)通常の確認。
  期限(送信から `MOZC_COMPOSE_CHECK_MS`=300ms、全体リテラルと同じ)まで足りず、対象スレッドが応答し、応答確認のあとにもう一度読んでも足りなければ、足りない分が文書へ素通りしたと判断して回収する。
  回収は既存の `RawTsfLiteralRecovery`(`escape_composition: true`): Escape で入力中の文字を捨て、足りない文字数だけ Backspace で文書の素通り分を消し、ローマ字全体を再送する。2 回目以降(フォーカス変更・IME を開くまで)は既存の打ち切りで何も送らない。
status: |-
  起草(2026-10-28)。実装済み(PR)。Opus レビュー round 1(Blocker 0・Major 3・Minor 5)を反映し、round 2 で収束(Blocker 0・Major 0、Minor 5 は記述のみ反映)。実機は未確認。
related_adr:
  - "ADR-270"
---

# ADR-284: Mozc の部分リテラル(先頭だけ素通り)の回収

## 背景(実測)

2026-10-28 00:33:38 UTC、dragonflyg4(Mozc Readable、Windows Terminal 系の窓)。フォーカス移動のあと F2→R で `ko` を送信し、`[mozc-compose] composition confirmed after 15ms (sent 2 chars, key_seq 330→331)`、`settled +30ms ... settled 1`。画面は `kおちら`(BUG-225)。`ti`・`ra` は正常。10-21 の「`dえ`・`kおちら`」「先頭だけ素通りは検出できていない」の実測での確認。

確認が張られる窓は `mozc_compose_check_applies`(`awase-windows-base/src/focus/class_names.rs`)で **Windows Terminal・WezTerm だけ**。物理 F2 でも cold になる(`composition_native_f2_down`、BUG-209)ので、F2 を押すたびの最初の送信が対象になる。

## 決定

1. **疑い**: 確認(`phase >= 1`)の時点で `key_seq − 送信前` が 1 以上で、送った文字数(`compose_literal_backs`)に満たなければ、`Done` にせず待つ(`settling`)。待つ間も後続のキーは既存の保留(`pending_tsf`)で止まっているので、進みは他のキーで増えない。進みが 0 の矛盾した読みは待たず従来どおり確認で終える(journal に確認が残る)。
2. **判断**(純粋、`state/mozc_shm.rs::judge_settled`): 進みが送った文字数以上なら通常の確認。1 以上で足りなければ `PartialLiteral { missing }`。0 は矛盾なので判断しない。
3. **待ち**: 毎 tick(10ms)読み直す。揃えば期限を待たず確認(2 打目の処理が遅いだけ)。足りないままなら送信から `MOZC_COMPOSE_CHECK_MS`(300ms)まで待つ。本物の素通りは何 ms 待っても進みが増えず、遅いだけなら待てば揃う非対称性による(Opus M2)。待ちは後続キーの保留が最大 300ms に延びる代償だが、全体リテラルの回収と同じ。**新しい定数は足さない**(10-21 の観測専用の 30ms 読み直しは撤去)。
4. **回収**: `RawTsfLiteralRecovery { backs: missing, escape_composition: true, romaji: 全体 }`。Escape で入力中の文字を捨てるので、Backspace は文書へ出た分(`missing`)だけでよい(入力中の文字の長さはローマ字の長さと一致しない: `o`→`お`)。再送は既存どおり cold の印付けから。`facts.idx` は最後の文字の位置にする: 連続 2 回目の諦める分岐は `romaji_after(romaji, idx)` で後ろの文字を送るが、Mozc は全体を 1 回で送るので後ろの文字は TIP が受け取り済みで、送ると `kおお` になる(Opus M3)。
5. **安全側**: 次は回収しない(`Done` のみ): 入力中の文字が消えている(`phase 0`)・IME が閉じた・読めない・宛先の窓が変わった・対象のスレッドが応答しない(`window_thread_responds`、`Literal` と同じ)・2 回目以降(`probe_io` の打ち切り、何も送らない)。`consecutive` を戻すのはフォーカス変更・IME を開く操作・`mark_literal_session=true` の確認だけで、Mozc の確認は後者を出さない。したがって同じ窓で 1 回回収したあとは、その窓を離れるまで部分リテラルを検出しても直さない(ログのみ。全体リテラルも同じ、既存)。**応答確認のあとにもう一度読み直し**、揃っていたら回収を取り下げる(確認の間に遅れていた 2 打目が処理されると、古い読みで直前の正しい文字を消す。Opus M1)。

## 未確認・限界(実機で測る)

- 待つ間(最大 300ms)に利用者が物理 Backspace を押した場合、保留の対象かは未確認(どちらでも文書の前の文字は消えない見込み)。
- 欠落が常に先頭の文字か。末尾の欠落(`k` だけ入力中)は、端末では `o` がシェルへ届き、Escape + Backspace で直る見込み(Opus の読み)だが実測していない。
- Mozc の候補(サジェスト)表示中は、Escape が入力中の文字を捨てずにサジェストを閉じるだけの可能性がある(共有メモリの `candidate_category`)。その場合は Backspace が入力中の `お` を消し、`k` が残って `kこ`(修正前より悪くはならないが直らない)。カスタムのキーマップで Escape が別の命令なら IME が閉じるおそれもある。
- 端末で Escape を TIP が受け取らずシェルへ届くと、readline では `ESC` + `Backspace` が M-Backspace(直前の**単語ごと**削除)、vim の挿入モードでは通常モードに抜けて再送がコマンドになる。TIP は `o` を見ているので確率は低いが、起きたときの被害は Backspace 単独より大きい。実機の頻度の観察が要る。
- 全体リテラル(`ComposeVerdict::Literal`)の連続 2 回目の諦める分岐にも同じ `romaji_after` の欠陥がある(`wo` → `woお`)。実測が無いので本 ADR では触らない(別件)。
- 部分リテラルの journal 記録は、全体リテラルと `escape_composition=true` と `backs` でしか区別できない。数えるなら `gave_up=false` と組み合わせる(諦めた件も `escape_composition=true` が記録に残る、既存の挙動)。ログの `partial literal` の行でも数えられる。
- `facts.idx` は欠けた位置ではなく、`romaji_after` を空にするため意図的に最後の文字の位置(journal から見ると「最後のキーで疑い」に見える)。

## 後続の案(予防)

原因は cold で TIP がまだキーを受け取る側になっていない間に `k` が届くこと。FSM が既に読んでいる `[mozc-focus]`(version 5)で、TIP のキーボードフォーカス取得が送信より後なら送信を待つ柵にすれば、破壊的な回収は要らない。BUG-225 の件の値を見て成立するかを先に測る(本 ADR の回収は、この予防が効くまでの安全網)。

## 検証

`judge_settled` の単体テスト 3 本(Linux)。FSM の `settle_step` 4 本は `#[cfg(all(test, windows))]` で windows-build CI(実行される)。Opus レビュー round 1 を反映、round 2 で再確認。
