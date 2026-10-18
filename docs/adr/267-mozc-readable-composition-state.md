---
id: ADR-267
title: |-
  mozc-readable の共有メモリに「組成の段階・候補の種類・未確定文字列の長さ・処理したキーの通番・入力欄のモード指定」を足す(version 3)
status: |-
  提案(2026-10-18、起草。Opus round1〈致命3・重要6・軽微5〉・round2〈致命1・重要4・軽微5〉・round3〈重要1・軽微4〉の指摘を反映した第4版。round4 で収束、軽微3件を反映済み)。publisher・読み手とも未実装
related_adr:
  - "ADR-191"
  - "ADR-265"
---

# ADR-267: mozc-readable の共有メモリに組成の状態を足す(version 3)

## 背景

ADR-265 の共有メモリ(`TipSharedState` version 2、48 バイト)が外に出す値は、IME の開閉・入力モード・`active` だけである。「いま未確定文字列(preedit)があるか」「変換中か」「候補が出ているか」「未確定文字列の長さ」「入力欄(入力スコープ)が入力モードを指定しているか」は分からない。

需要(ユーザー指摘 2026-10-18):

1. **QMK など自作キーボード**: 「組成中・変換中だけ別レイヤーにする」「候補が出ている間だけ Enter/矢印の意味を変える」「OLED に表示する」。
2. **awase**: 
   - 入力欄(パスワード欄など)が入力モードを指定していることを、推測ではなく IME 自身の申告(`scope_forced_mode`)で知る。**awase 側の実害は未確認**(known-bugs に、パスワード欄で Mozc が直接入力に落ちているのに awase が開だと思って送る型の報告は見つからない。BUG-141 の password への言及はテキスト入力欄の種別ごとの挙動の話で別件)。実害が確認できるまで観測のみ。
   - 送ったキーを TIP が見たかの目安(`key_seq`)と、組成の値がどの打鍵まで反映済みかの印(`composition_key_seq`)。
   - 未確定文字列の長さ(参考値)。
   - **BackSpace 数の検算は需要から外す**。round1(R1-C1)で、`flush_raw_tsf_literal_backspaces` / `ze_bs_count` は「IME を素通りしたリテラル」を消す数で preedit の長さとは別の量であり、しかもその経路は `gji_active`(GoogleIMEJaConverter の I/O 観測)が前提で、mozc-readable の窓では走らないと分かった(`warm_send_plan.rs::plan_literal_detect` が `GjiNotActive` で Skip、`tsf/observer.rs::gji_is_active_ime`)。検算の対象は構造的に 0 件になる。mozc 系でリテラル検出を行う必要があるかは未確認で、必要になれば別 ADR とする。

## 決定

### D1. 構造体を version 3 にし、末尾に 32 バイト追記する(80 バイト。詰めて 64 にする案はユーザー判断で見送り)

オフセット 0〜47 は変えない(追記のみ。互換を壊すときだけ `version` を上げる、の取り決めに従う)。version 2 で `reserved` は `active` に使い切っており、version を上げずに済む空きは無い(R1-M4)。

| オフセット | 型 | 名前 | 意味 |
| --- | --- | --- | --- |
| 48 | u32 | `phase` | 0=組成なし、1=組成中(読みを入力中)、2=変換中(Space 後など。preedit に注目文節〈`annotation == HIGHLIGHT`〉がある) |
| 52 | u32 | `candidate_category` | 0=候補の窓なし、1=`SUGGESTION`、2=`PREDICTION`、3=`CONVERSION`。この数値は `commands::Category` の値(CONVERSION=0, PREDICTION=1, SUGGESTION=2)とは**別**で、publisher が switch で明示的に写す(実装で判明、R4 後の訂正)。`TipUiHandler` の表示判定〈`IsVisible`〉を通した結果で、UI-less モードでは 0 |
| 56 | u32 | `preedit_utf16_len` | 未確定文字列の UTF-16 の長さ(TIP が文書へ書く長さと同じ単位)。`phase=0` なら 0 |
| 60 | u32 | `preedit_cursor` | 未確定文字列内のカーソル位置。**UTF-32 のコードポイント単位**(Mozc の `output.preedit().cursor()` の単位) |
| 64 | u32 | `key_seq` | TIP が見たキーダウンの通番。`TipKeyeventHandler::OnTestKeyDown` と `OnPreservedKey` の 2 か所で、**関数の入口(中の `OnTest` の早期 return より前)**で +1(`OnKeyDown` では数えない。TSF は `OnTestKeyDown` が eaten=FALSE のキーに `OnKeyDown` を呼ばないため。数えると食べたキーが 2 回になる)。ラップあり。**打鍵ごとに毎回書く**(pulse はしない) |
| 68 | u32 | `scope_forced_mode` | 入力スコープが 1 つのモードに決まるときの「`commands::CompositionMode` の値+1」(0=スコープによる指定なし。取りうる値は 1〜6)。**1(DIRECT)は「IME を閉じる」指定で、`effective_mode` は変わらない**(`StatePair(false, base_state.conversion_mode)`)。`GetOverriddenState` の `states.size() == 1` の分岐で決まる。入力欄がそう指定している、という事実で、打鍵では消えない(R2-I3) |
| 72 | u32 | `server_ready` | 1 のとき、この TIP が Mozc サーバーから応答を受け取れている(warm)。0=まだ応答を受けていない、または直近の呼び出しが失敗した(cold の疑い)。D10。**仮説の段階で、実測で採否を決める** |
| 76 | u32 | `composition_key_seq` | `phase`・`candidate_category`・`preedit_*` の元になった `Output` を**生んだ打鍵**の番号(公開した時点の `key_seq` ではない。R3-I1)。読み手は `(int32_t)(composition_key_seq - 自分が送った打鍵の番号) >= 0` のときだけ組成の項目を採る(ラップを考慮した比較。R3-M3。Python の例も同じ書き方にする) |

- `version` は `3`、サイズは 80 バイト。
- 配置: `update_tick_ms`(u64)がオフセット 40(8 の倍数)、追記の u32×8 が 48〜79、合計 80(8 の倍数)で暗黙の詰め物は入らない(R2-M5)。`offsetof` で 48・64・76 を固定する `static_assert` も足す。今後の追記は 80 から始め、u64 は 8 の倍数のオフセットに置く。参考: Python は `struct.Struct("<10IQ8I")`(= 80)、Rust は `#[repr(C)]` と `const _: () = assert!(size_of::<T>() == 80);`。
- 変更点の一覧(publisher): `static_assert(sizeof(TipSharedState) == 48)` を 80 に、`Snapshot`・`operator==`・`Flush()` の書き込みに新フィールドを全部足す(足し忘れると ADR-265 R2-C2a と同じ「変化したのに書かれない」が起きる)。

### D2. 長さは UTF-16 で出す(R1-I5)

- 組成中の BackSpace は TIP が食べて Mozc の composer が処理するので、アプリの書記素の規則は効かない。長さを出す目的は、TSF/IMM の composition 文字列と突き合わせることで、TIP が文書に書く単位(UTF-16、`ComposePreeditText` → `SetText(..., preedit_text.size())`)が最も直接に使える。
- カーソルだけは Mozc が UTF-32 で持つので、そのまま出す。読み手は、同じブロック内で単位が違うことを仕様書で確かめること。
- コードポイント数・書記素数が要る読み手は少ない。要る需要が出たら追記する。

### D3. 未確定文字列の「中身」は出さない

有無・段階・長さ・カーソル位置だけを出す。パスワード欄の `composing` を 0 に落とすか(旧 Q1)は、**落とさない**。理由: Mozc はパスワードの入力スコープで入力モードを切り替えるため組成が起きにくく実害が小さい(R1-M3、`OnChangeInputScope`)。version 3 で新しく漏れるのは打鍵の時刻(`key_seq`・`update_tick_ms`)だが、同一ユーザーのプロセスは低レベルフックで打鍵そのものを取れるので、ADR-265 R1-M7 の「同一ユーザーは信頼境界の内側」の範囲内とする。

### D4. 候補の「数・選択位置」は足さない

対象は数と選択位置に限る。`candidate_category` は「有無」の判定に必要な最小の情報であり、除外しない(R1-M5)。

### D5. 値の鮮度は `key_seq` で判定する(旧 D5 を置き換え。R1-C3)

- 旧案の「`update_tick_ms` が直近の打鍵より新しいか」は成り立たない。publisher は変化が無ければ書かないので、「処理したが値が変わらなかった」と「まだ処理していない」の区別がつかず、ADR-265 D3 の読み取り規約(`update_tick_ms` による新しさの判定はしない)とも矛盾する。`seq` が進んだかも同じ理由で使えない。
- `key_seq` は「TIP が見たキーダウンの回数の目安」であり、送ったキーの数と完全には一致しない(アプリが `ITfKeystrokeMgr::TestKeyDown` を同じキーで複数回呼ぶと重複して数え、逆に `TestKeyDown` なしで `KeyDown` を直接呼ぶと数えられない。どちらも未確認で、`VKBackBasedDeleter` が送る BackSpace が `key_seq` に入るかも未確認(R4-M3)。CI で Chrome・WezTerm・Notepad に送った打鍵数と `key_seq` の増分を 1 回ずつ記録して確かめる。R3-M1)。IME が閉じていて食べないキーも、`OnTestKeyDown` で数えるので進む(`OnKeyDown` で数えると、IME が閉じた状態では進まず「TIP がキーを受け取っていない」と区別できなくなる。R2-I1)。
- `key_seq` は編集セッションの外で進み、組成の項目は編集セッションの末尾(D7)で書かれるので、`key_seq` だけでは組成の項目が反映済みかは分からない(`k` を送って `key_seq` が進んでも、編集セッションが走る前は `phase=0`)。そこで `composition_key_seq` を一緒に書く。ただし**公開した時点の `key_seq` を読んではいけない**: 非同期の編集セッション(Word の回避策の `TF_ES_ASYNC`)では、打鍵 N の編集セッションが走る前に打鍵 N+1 の `key_seq` が進み、中身は N の `Output` なのに N+1 が付く。`OnEndEdit`・`AsyncSetFocusEditSessionImpl` からの `Update` も、打鍵と無関係に最新の `key_seq` を付けてしまう(R3-I1)。
- 作り方: `TipPrivateContext` に `last_output_key_seq` を持たせ、`last_output` を更新する地点(`UpdatePrivateContext` の `*private_context->mutable_last_output() = output;`、`OnTest` 内の `should_be_sent_to_server && temporal_output.has_consumed()` の書き換え)で、その `Output` を生んだ打鍵の番号を入れる。番号は、編集セッションのオブジェクトを作る `OnOutputReceivedImpl` で、その時点の `key_seq` を保存して運ぶ(運ぶのは打鍵の番号 1 つで、経路は 1 か所)。打鍵に由来しない `Output`(言語バー、`OnCompositionTerminated` の SUBMIT など)は直前の値を引き継ぐ。
  - 番号は `OnOutputReceivedImpl` の**引数**にする。打鍵の経路(`OnKey`、`TurnOnImeAndTryToReconvertFromIme`、`UndoCommint`)だけが番号を渡し、`ReconvertFromApplicationSync`(アプリからの再変換)は「なし」(直前の値を引き継ぐ)を渡す。そうしないと、打鍵 M の編集セッションが `TF_ES_ASYNC` で予約中のときに、再変換の Output に M が付く(R4-M2)。
  - `OnTest` 内の書き換え(`should_be_sent_to_server && temporal_output.has_consumed()`)は、そのコマンドが `TEST_SEND_KEY` か `SEND_KEY` かで分かれる。`TEST_SEND_KEY` ならサーバーが打鍵を処理した結果ではないので、この地点では番号を進めず(直前の値を引き継ぐ)、組成の項目もこの Output からは作らない。`SEND_KEY` なら上の書き方のまま。実装時に確かめる(R4-M1)。
  - `Update` は `last_output_key_seq` をそのまま書く。
- 読み手は `(int32_t)(composition_key_seq - 自分が送った打鍵の番号) >= 0` のときだけ組成の項目を採る(R2-I2、R3-M3)。
- 値は「TIP が編集セッションの中で公開した時点」の状態である。`OnOutputReceivedImpl` は Word の回避策で `TF_ES_ASYNC` に切り替えることがあり、受け取りと公開の間にアプリがロックを与えるまでの遅れが入る。読み手は、このずれを前提にする。

### D6. 書き込みと pulse を分ける(旧 Q2 を閉じる。R1-I6)

- `Flush()` の判定を 2 つに分ける。**書くかどうか**は全項目の比較(`key_seq` を含む)、**pulse するかどうか**は状態の項目(`active`・開閉・モード・`phase`・`candidate_category`・`scope_forced_mode`・`server_ready`)の変化だけ。`preedit_utf16_len`・`preedit_cursor`・`key_seq`・`composition_key_seq` だけの変化では pulse しない。
- 書き込みのコスト(`InterlockedIncrement` 2 回と数個のストア)は、打鍵ごとに既にある mozc_server への IPC 往復(ms 単位)に比べて無視できる。測定は書き込みの遅さではなく、「連続打鍵で pulse の回数が状態の変化回数と一致する」ことの CI 確認だけにする。
- 通知イベントを 2 つに分ける案は、イベント名の増加が identity の置換表と AppContainer の名前空間(ADR-265 で通知イベントの名前空間は未確認)の両方に波及するため採らない。

### D7. 公開位置は編集セッションの末尾にする(R1-I1)

- 現行の `Publish` は `TipInputModeManager` の各 mutator の `PublishOnExit` からだけ呼ばれ、`Output` を受け取らない。引数に足すのではなく、`SetActive` と同じく別の setter(`SetComposition(...)`)を新設する。
- 呼ぶ場所は `TipEditSessionImpl::UpdateContext` / `OnCompositionTerminated` の末尾に当たる **`TipUiHandler::Update` の中**とし、`last_output()` から値を作る。`candidate_category` を決める `FillVisibility`(`IsVisible` 越し)がここでしか計算されないため、ここで書けば判定を二重に書かずに済む。`Update` は `UpdateContext`・`OnCompositionTerminated` に加えて `OnEndEdit`・`AsyncSetFocusEditSessionImpl` からも呼ばれるので、同じスレッドの別 context へのフォーカス移動も一部拾える(R2-M1)。`UpdatePrivateContext` の中では呼ばない。理由: (1) `if (!output.has_status()) return S_FALSE;` の早期 return で、status の無い Output の更新を取りこぼす。(2) `UpdatePreeditAndComposition` より前に呼ばれるので、そこが `E_FAIL` でも `phase` が出てしまう。
- `candidate_category` は `TipUiHandler` の表示判定(`FillVisibility`・`TipUiElementManager::IsVisible`)を通した結果を同じ場所で取る(R1-C2)。

### D8. 入力欄が入力モードを指定していることを申告する(ユーザー指摘 2026-10-18、round2 で縮小)

- ユーザー指摘の動機は「打鍵なしで IME の状態が変わる(パスワード欄など)ことを、awase などが分かるようにしたい」。当初は `override_active`(差があるか)と `last_change_cause`(遷移の原因)を提案したが、round2 で次のとおり成り立たないと分かった。
  - `override_active` を「`effective_*` と `tsf_*` に差がある」で定義すると、パスワード欄で利用者が IME を開いた時点で両方が開になり(`OnReceiveCommand` が両方に `status.activated()` を入れる)、スコープがパスワードのままでも 0 になる。差が出ない上書き(ひらがな指定のスコープにひらがなで入る)もある(R2-I3)。
  - `last_change_cause` は mutator 単位では原因を決められない。`OnReceiveCommand` には打鍵・言語バー・候補窓のマウス操作が全部入り、compartment の `OnChange` は **awase 自身の `ImmSetOpenStatus` でも呼ばれる**ので、awase の actuation が「外部」と申告される。後で belief に入れると、ADR-212 で撤去した補正的 actuation との往復になる(R2-C1)。正しく作るには、原因を入口で決め、編集セッションのオブジェクトで運ぶ設計(非同期のセッションで入口の情報が消えるため)が要り、費用に見合う需要が確認できていない。さらに、打鍵なしの遷移のうち既存の観測(compartment と version 2 の `tsf_*`)で見えないのは入力スコープの上書きだけである(言語バー・候補窓のマウス操作・他クライアントの `ImmSetOpenStatus` は、最後に compartment へ書かれるので、awase は遷移の瞬間を既に見ている)。入力スコープの上書きは `scope_forced_mode` で見えるので、`last_change_cause` が足すのは**原因のラベルだけ**で、見えない遷移を見えるようにはしない(R3 の確認)。**`last_change_cause` は本版では入れない。** 需要(実害)が確認できてから別 ADR(version 4)で扱う。その際の注意として round2 の R2-C1 の表(入口と原因の対応)と、awase 自身の書き込みの除外が要ることを残す。
- 代わりに **`scope_forced_mode`** を出す(実装: `GetOverriddenState` は `static` で `StatePair` を返すだけで、`states.size() == 1` かどうかとモードは外から取れない。戻り値を拡張するか、`input_scope_` から別に計算する関数を足す。`input_scope_` を更新する `OnSetFocus`・`OnChangeInputScope` はどちらも `PublishOnExit` を持つので、公開は `SetComposition` ではなく既存の `Publish` 側に足す。R3-M2)。入力スコープが 1 つのモードに決まる(`GetOverriddenState` の `states.size() == 1`)ときの「モード+1」で、0 は指定なし。入力欄がそう指定しているという事実で、打鍵では消えない。「いま差がある」は、読み手が version 2 の `effective_*` と `tsf_*` から導く(新しい項目は要らない)。
- 入力スコープの上書きは compartment に書かれない(`OnChangeInputScope` は `kUpdateUI` を返すだけ)ので、awase が IMM/TSF から読む開閉は上書き後の値と食い違う。これが version 2 でも食い違いから推測できる根拠である。
- awase の drift correction はこれを**見ていない**はず(IMM/TSF の観測と belief の食い違いで再送するため)。したがって「drift correction が推測で拾っていた類い」とは書かない。段階 3 の別 ADR は「drift correction の置き換え」ではなく、「入力欄が指定しているモードを belief の入力に加えるか」として議論する。最初は journal に記録するだけで、belief は動かさない。

### D9. 読み手の規約(R1-I2、R1-I4)

- **map サイズ**: 長寿命のプロセスは旧 DLL のまま 48 バイトのセクションを作り続けるので、新旧が同時に存在する。読み手は「サイズ 0(セクション全体)で map する」か、「48 で map して `version` を読み、`version >= 3` のときだけ 80 で map し直す」。80 固定で map すると、version 2 のブロックの開閉・モードまで読めなくなる。
- **書き手の `ERROR_ALREADY_EXISTS`**: 同名のセクションが旧サイズ(48)で残っていると、`MapViewOfFile(..., 80)` が失敗し、`tried_ = true` でそのスレッドは二度と書かない。この場合は **`version=2` として 48 だけ書く**(`Get()` は `version` を無条件に書くので、そのままだと 48 バイトのセクションに `version=3` が載り、D9 第 1 項の読み手が 80 で map し直して失敗する)。`ThreadSlot` に「書ける大きさ」を持たせ、48 のときは `Flush()` も 48 以降に触れない(R2-M2)。
- **`active == 0` のときは他の項目を無視する**ことを必須として仕様書に書く。`Deactivate` は組成の項目を触らないので、IME を切り替えた後も `phase` が残りうる。あわせて `Deactivate` で `phase`・`candidate_category`・`preedit_utf16_len`・`preedit_cursor`・`scope_forced_mode`・`composition_key_seq` を 0 に落とす。`key_seq`(通番)は残す(R2-M4)。
- **context とスレッドのずれ**: 組成の状態は `ITfContext` ごと(`TipPrivateContext::last_output`)だが、共有メモリはスレッドごとに 1 スロット(`thread_local ThreadSlot`)。同じスレッドの別 context へフォーカスが移っても、キーを押すまで更新されないことがある。段階 1 の CI で何が出るかを確かめ、仕様書に書く。

### D10. cold/warm の状態を `server_ready` で出す(ユーザー指摘 2026-10-18。**根拠は無く、観測用の仮置き**)

- 当初の背景は、mozc-readable の CI で「新しい runner の最初のキーが落ちる」現象を mozc_server の起動・接続待ち(cold)と見たこと。**この根拠は成り立たない**。原因は検証スクリプト側で、コンソールの python/PowerShell を並行して起動すると、その窓(Windows Terminal)が前面を奪い、キーが別の窓に行っていた。公式の無改造 Mozc でも同じ現象で、TIP や mozc_server の状態とは関係が無かった(メモリ `reference_windows_ci_harness_pitfalls_2026_10_18`)。
- したがって「Mozc 系の窓に cold/warm がある」という証拠は、今のところ無い。awase の cold/warm(GJI の F2 待ち、Chrome の TSF 文脈の再初期化 ~326ms〈BUG-002〉)は、ホストアプリ側の TSF 文脈の話であり、TIP が持つ状態ではない。
- それでも `server_ready`(この TIP がサーバーから応答を受け取れているか)は、事実として取れるなら安い(追記の位置は `reserved`〈72〉、更新は応答を受けた地点と呼び出しが失敗した地点だけ)。「0=不明/なし、1=応答を受け取れている」と定義して**観測用に出す**。
- **採否の条件**: 段階 1 の CI で、`server_ready` が 0 の間にキーを送ったときの結果が 1 の後と違う、という差が出たときだけ採る。差が出なければ項目を外して `reserved` に戻す(使われない項目を残さない。ADR-158 の複雑性予算)。外すときは version を上げずに済む(0 に戻すだけ)。
- 読み手の使い方: 最初は journal への記録のみ。判断には使わない。
- warm up を指示する経路(awase → TIP)は本 ADR の範囲外。cold/warm の存在が Mozc 系で実測できてから、別の ADR にする。

### D11. 素通りしたキーの数を `key_seq` の差で測れるか確かめる(ユーザー指摘 2026-10-18。**仮説。測定が先で、採否は未定**)

背景は D1 の「BackSpace 数の検算は需要から外す」と同じ問題の別の角度である。`flush_raw_tsf_literal_backspaces` の BackSpace 数(`RAW_TSF_LITERAL.backs`)は、IME を素通りして文書に出た文字の数で、`preedit_utf16_len`(IME の中の長さ)とは別の量である。素通りしたキーは TIP が見ていないので、TIP が「正しい数」を直接書くことはできない。

代わりに、awase が送ったキーの数 N と、同じ区間で `key_seq` が進んだ数 M の差 N−M を「TIP が見なかったキーの数」の候補にする。cold で最初の数打鍵が落ちる型の回収を、固定のヒューリスティックでなく測った値で決められる可能性がある。

- 成立しない見込みの点(測定で確かめる): `key_seq` は重複や漏れがある(D5。`TestKeyDown` の複数回呼び出し、`KeyDown` の直呼び)。素通りしたキーが全部リテラルとして文書に残るとは限らず、落ちるだけのキーもある(N−M は文書に残った数の上限にすぎない)。cold で TIP がまだ動いていないスレッドには、そもそも書き手がいない。
- 効く範囲は mozc-readable の窓だけ。GJI 配布版・MS-IME では使えない。
- 測定(段階 1 の CI に足す): Chrome・WezTerm・Notepad に既知の打鍵列を、新しい runner の最初のキーから送り、(a) N−M、(b) 文書に実際に残ったリテラルの文字数、(c) 現行ヒューリスティックの BackSpace 数、を 1 打鍵列ごとに記録する。(a)=(b) が全打鍵列で成り立たなければこの D11 は却下し、却下した代案の節に移す。
- 採る場合も、awase の BackSpace 回数の決定への接続は別の ADR とする(この ADR は観測のみ)。

## 却下した代案

- **別のマッピング名**(例 `Local\mozc-readable-comp-<tid>`)に分ける案: D9 の互換問題は消え、更新頻度の違う値を分けられるが、AppContainer 経由の読み取り(`NtOpenSection`)と identity の置換表が 2 倍になる。同じマッピング + D9 の読み手規約の方が安い。
- **`composing` / `candidates_visible` の 0/1**: 打鍵中のサジェストでも `has_candidate_window` が真になり、Space の前後を区別できない(R1-C2)。変換中の BackSpace は変換の取り消しで、読みの文字数とは別の動きになる(R1-I3)。QMK の Enter の意味も、組成中(読みを確定)と変換中(候補を確定)で違う。

## 実装の段階

1. **mozc-readable**: `readable/docs/shared-memory.md` に version 3 を書く(仕様が正)。D7 の位置に `SetComposition` を配線し、`Snapshot`・`Flush()` を直す。CI で別プロセスから次を確かめる。
   - `phase`: 入力前=0、`k`=1、Space=2、Enter=0。予測の候補を Tab/↓ で選んだとき(`candidate_category=PREDICTION`)に 1 か 2 のどちらになるか、BS の動きが変換中と同じか(R2-M3)。
   - `candidate_category`: `k` の時点で `SUGGESTION`(実測 1)、1 回目の Space 後は 0、2 回目の Space で `CONVERSION`(実測 3)(確定後に予測を出す設定は固定して外す)。
   - `composition_key_seq`: 連続打鍵中に、その時点の `preedit_utf16_len` が実際に反映している打鍵の番号を超えない(`TF_ES_ASYNC` を強制する CI 用のスイッチ、または Word の回避策が効くアプリで確かめる)。
   - `key_seq`: 値が変わらない打鍵でも進む。**IME を閉じた状態のキーでも進む**。`composition_key_seq` が `key_seq` に追いつくまで `phase` を採らない読み方で、`k` の直後に `phase=1` が読めること。
   - 組成中に同じ窓の別 context へ移る/Win+Space で別 IME に切り替える、のときに何が出るか。
   - パスワード欄(入力スコープ)に入ったとき `scope_forced_mode` が立つか。その状態で IME を開いても立ったままか。ひらがな指定のスコープに入ったときはどうか。
   - 連続打鍵で pulse の回数が状態の変化回数と一致する。
   - `key_seq` の差(D11): 上の測定(a)(b)(c)を記録する。
   - `server_ready`(D10): 新しい runner の最初のキーの前は 0 か。0 の間と 1 になってからで、結果に差があるか(差が無ければ外す)。
2. **awase**: ADR-265 段階 1 の Observe が入った後で、journal に `phase`・`candidate_category`・`key_seq`・`composition_key_seq`・`scope_forced_mode`・`server_ready` を記録する(観測のみ)。
3. 記録の結果を見て、belief への反映(D8)や `key_seq`/`composition_key_seq` を使った「TIP が追いついたか」の判定を、別の ADR にする。
4. **QMK 向け**: `readable/docs/examples/read_ime_state.py` を version 3 対応にする(D9 の map 規約に注意。現状の `assert STRUCT.size == 48` と `MapViewOfFile(..., STRUCT.size)` は書き換えが要る)。`phase`・`candidate_category`・`scope_forced_mode` を Raw HID で送る例を足す(スケッチのまま。実機未検証と明記)。

## 未決

- **Q1**: awase 側で、入力欄が指定するモードと belief が食い違って困った実例があるか(known-bugs・journal で探す)。無ければ `scope_forced_mode` は QMK 向けの表示用に留める。
- **Q3**: D11 の N−M が文書に残ったリテラル数と一致するか(段階 1 の測定。一致しなければ D11 を却下する)。

## 確認していないこと

- Mozc 系の窓に cold/warm があるか(D10。根拠なし、観測用)。サーバー呼び出しの失敗を TIP のどの地点で検知できるか。

- `candidate_category` を `IsVisible` 越しに取れるか(`TipUiHandler::Update` の中で取る前提。UI-less モードの扱いを含む)。
- AppContainer 内の窓での読み取り(80 バイトでも名前空間の扱いは変わらない見込みだが、CI で確かめる)。
- 組成中の context 切り替え・IME 切り替え時に、どのフィールドがどう残るか(D9、段階 1 の CI で確かめる)。
