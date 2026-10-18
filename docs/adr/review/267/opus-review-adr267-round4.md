# ADR-267 Opus 敵対的レビュー round 4

対象: `docs/adr/267-mozc-readable-composition-state.md` 第4版(`6549b4e9`)
照合: `cuzic/mozc-readable` の `readable`(`tip_edit_session.cc`・`tip_keyevent_handler.cc`・`tip_edit_session_impl.cc`、`mutable_last_output` と `OnOutputReceived*` の全呼び出し元を grep)。

**結論: 収束。** 新しい致命・重要は無い。軽微 3 件(うち 2 件は実装時の確認事項)。

---

## round3 の指摘への対応の確認

| ID | 判定 |
| --- | --- |
| R3-I1 | 解消。下の確認で抜けは軽微のみ(R4-M1・M2)。 |
| R3-M1〜M4・D8 の 1 文 | 解消。 |

## R3-I1 の作り方の確認(依頼事項)

`last_output` を書く地点は、実コード全体で次の 2 つだけで、ADR の挙げる 2 地点と一致する。

- `tip_edit_session_impl.cc:402` `UpdatePrivateContext` の `*private_context->mutable_last_output() = output;`
- `tip_keyevent_handler.cc:268` `OnTest` の `*private_context->mutable_last_output() = temporal_output;`

`OnOutputReceived*` を呼ぶ地点は 5 つある。

- `tip_keyevent_handler.cc:462`: 打鍵(`OnKey`)。`VKBackBasedDeleter` の `APPLY_PENDING_STATUS` で、前に保留した `pending_output` を出す場合も含む。
- `tip_edit_session.cc:368/371`: `TurnOnImeAndTryToReconvertFromIme`。打鍵の Output の callback から呼ばれる。
- `tip_edit_session.cc:432`: `UndoCommint`。同じく打鍵の callback から呼ばれる。
- `tip_edit_session.cc:843`: `ReconvertFromApplicationSync`。アプリからの再変換で、打鍵に由来しない。

打鍵の経路では、TSF が同じキーの `OnTestKeyDown` と `OnKeyDown` を続けて呼ぶ。このため `OnOutputReceivedImpl` の時点の `key_seq` は、その打鍵の番号になる。非同期の編集セッションでも、番号は予約の時点で取ってオブジェクトに入るので、R3-I1 の失敗シナリオ(N の Output に N+1 が付く)は防げている。

---

## 軽微

### R4-M1. `OnTest` の書き換え(2 地点目)で番号を進めてよいかは、`temporal_output` の中身次第(要確認)

`OnTest` の `temporal_output` は `KeyEventHandler::ImeProcessKey` のテスト段階の応答である。サーバーが打鍵を実際に処理した結果か、「食べるか」だけを答えた応答(preedit・candidate_window を含まない、または打鍵前の状態)かで、扱いが分かれる。

- 後者の場合、この地点で `last_output_key_seq = M` にすると、打鍵 M を反映していない(または組成の項目が空の)Output に M が付く。そこへ `OnKeyDown` の前に `OnEndEdit` 経由の `Update` が走ると、`phase=0`・`composition_key_seq=M` が公開され、読み手は「M の後で組成が無い」と誤認する。R3-I1 と同じ形の失敗。
- 実装時に、`should_be_sent_to_server` が真のときのコマンドが `TEST_SEND_KEY` か `SEND_KEY` かを確かめる。
  - `TEST_SEND_KEY` なら、この地点では番号を進めない(直前の値を引き継ぐ)。組成の項目もこの Output からは作らない。`SetComposition` は `UpdatePrivateContext` で保存した値から作る。
  - `SEND_KEY`(テスト段階で処理を済ませる)なら、ADR の書き方のままでよい。
- ADR の D5 の「作り方」に、この分岐を 1 行足す。

### R4-M2. 打鍵に由来しない `OnOutputReceivedImpl` の呼び出しでも、その時点の `key_seq` が付く

ADR は「`OnOutputReceivedImpl` で、その時点の `key_seq` を保存して運ぶ」と「打鍵に由来しない Output は直前の値を引き継ぐ」を併記している。だが `OnOutputReceivedImpl` は呼び出し元を区別しない。

- `ReconvertFromApplicationSync`(`tip_edit_session.cc:843`)は、その時点の最新の番号 M を付ける。ほとんどの場合、その Output は M より後の状態なので害は無い(「M 以降の状態」として正しい)。
- 例外は、打鍵 M の編集セッションが `TF_ES_ASYNC` で予約中のまま、アプリが同期の再変換を走らせる場合。M を反映していない状態に M が付く。Word の回避策と再変換が重なるときだけで、狭い。
- 直し方: 番号を `OnOutputReceivedImpl` の引数にする。打鍵の経路(`keyevent_handler.cc:462`、`TurnOnImeAndTryToReconvertFromIme`、`UndoCommint`)だけが渡し、`ReconvertFromApplicationSync` は「なし」(直前の値を引き継ぐ)を渡す。ADR の文言を「`OnOutputReceivedImpl` の打鍵の経路の呼び出し元で番号を渡す」に直せば足りる。

### R4-M3. `VKBackBasedDeleter` が送る BackSpace も `key_seq` に入りうる(要確認)

`deletion_range` を伴う Output では、`VKBackBasedDeleter`(`BeginDeletion`、`tip_edit_session.cc:420-424`)がアプリへ BackSpace を送り、最後に `pending_output` を適用する。この BackSpace が `SendInput` 由来で TIP の `OnTestKeyDown` を通るなら、`key_seq` は awase が送った打鍵より多く進む。読み手の「自分が送った打鍵の番号」(読み始めの `key_seq` + 送った数)とずれる。ずれる向きは、読み手が組成の項目を早めに採る側。

- ずれが出るのは、Mozc が直前の文字を消して置き換える場合(予測の一部など)だけ。
- 段階 1 の CI の「送った打鍵数と `key_seq` の増分を記録」(R3-M1 で足した行)で拾える。仕様書の `key_seq` の注意(重複・取りこぼしの 2 方向)に「IME 自身が送る BackSpace」を 3 つ目として並べる。
