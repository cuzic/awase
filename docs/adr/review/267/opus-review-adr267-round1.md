# ADR-267 Opus 敵対的レビュー round 1

対象: `docs/adr/267-mozc-readable-composition-state.md`(`0406319c`)
照合した実コード: `cuzic/mozc-readable` の `readable` ブランチ(2026-10-18 に `gh api .../contents/...?ref=readable` で取得)、rust-nicola の `0406319c` のツリー。

集計: 致命 3 / 重要 6 / 軽微 5

---

## 致命

### R1-C1. 需要 1(awase の BackSpace 数の検算)は、前提の両方が成り立たない

**(a) 数えている対象が違う。** ADR が挙げる箇所の BackSpace は、**IME を通らずに出たリテラル**(TSF context が cold のまま送られて素通りしたローマ字)を消すためのもので、未確定文字列(preedit)の外にある文字を数えている。

- `crates/awase-windows/src/tsf/output.rs:163` `flush_raw_tsf_literal_backspaces` は `RAW_TSF_LITERAL.backs` の個数ぶん BS を送るだけ。composition 側は `escape_composition` で `VK_ESCAPE` を送り、**文字数に関係なく**破棄している(同ファイル 150〜156 行のコメント)。
- `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs:40-53`: 「composition が実際に何文字分だったかは分からないため…composition 側の推測を ESC に置き換え、BS はここに残る『先頭 literal プレフィックス』の削除のみを担う」。`ze_bs_count`(= `chars.len()`、送ったローマ字の文字数)は「全部リテラルだった」ときに消す数(`word_level_recovery_params`、同 117〜128 行)。
- つまり BS の数は「送った文字数 − IME が受け取った文字数」であって、`preedit_chars` とは別の量。preedit の長さが分かっても、確定済みテキストに混ざったリテラルの数は出てこない。preedit の長さが効くのは部分リテラル経路の composition 側だけで、そこは既に ESC で長さ非依存になっている。

**(b) mozc-readable を使っているときに、その経路は動かない。** リテラル検出・回収は GJI の converter プロセスの I/O 観測を前提にしている。

- `crates/awase-windows-core/src/state/warm_send_plan.rs:119-131` `plan_literal_detect` は `!facts.gji_active` で `Skip(GjiNotActive)`。
- `gji_active` は `crates/awase-windows/src/tsf/observer.rs:564` `gji_is_active_ime()` = `gji_monitor_ok && tsf_active_kind == 1`。監視対象は `crates/awase-windows/src/tsf/gji_monitor.rs:27-30` の `GoogleIMEJaConverter` / `GoogleJapaneseInputConverter` のみ。
- `crates/awase-windows/src/output/vk_send.rs:510-512` のコメントも「raw literal の記録自体が常に `gji_active` でゲートされている」と明言。
- mozc-readable(OSS Mozc 由来、converter は別名)を使っている窓では `gji_active=false` で、`ze_bs_count` も `RAW_TSF_LITERAL.backs` も 0 のまま。逆に GJI を使っている窓には共有メモリが無い。**検算の対象(推定値と共有メモリの値が両方ある打鍵)が構造的に 0 件**で、段階 2・3 の「一致率の実測」は 1 件も集まらない。

**直し方の案**: awase 側の需要を「BS 数の検算」から「mozc-readable の窓で、送ったキーを TIP が受け取ったかの直接の証拠」に書き直す。そのために必要なのは preedit の長さではなく、**TIP が処理したキーの通番**(R1-C3)。GJI の I/O 観測(`gji_write_bytes` 等)が担っている「composition 確証シグナル」を mozc 系で置き換えられるかを問うのが筋で、そうなら別 ADR(リテラル検出を mozc 系にも広げる)になる。そもそも Mozc で cold-start のリテラル化が起きるか自体が未確認なので、需要 1 は「未確認」として段階から外すのが妥当。

### R1-C2. `candidates_visible` の定義(`has_candidate_window`)は、需要 2 が想定する「変換候補が出ている」と一致しない

- Mozc は打鍵中にサジェスト(`category == SUGGESTION`)と予測(`PREDICTION`)の窓を出す。`output.has_candidate_window()` はこれらでも真になる。`src/win32/tip/tip_ui_handler_conventional.cc:119-131` `FillVisibility` は category で分岐し、SUGGESTION は suggest window、CONVERSION/PREDICTION は candidate window として扱う。
- 実際に表示されるかは、さらに `TipUiElementManager::IsVisible(...)`(同 107〜112 行)に依存する。UI-less モード(アプリが候補を自前で描く。ゲーム、一部の TSF ネイティブアプリ)では Mozc の窓は出ない。D1 の「表示対象」はこのどちらも考慮していない。
- 結果: QMK の「候補が出ている間だけ Enter/矢印の意味を変える」は、**Space を押す前の普通の打鍵中に**サジェストで発火する。サジェスト中の Enter は「読みのまま確定」で、変換候補中の Enter(選択中の候補を確定)とは意味が違う。段階 1 の CI 期待値「`ka`→かな 1 文字、Space→`candidates_visible=1`」も、`ka` の時点でサジェストが出ていれば既に 1 で、Space の前後を区別できない。
- **直し方**: `candidates_visible` の 0/1 ではなく `candidate_category`(0=無し、1=SUGGESTION、2=PREDICTION、3=CONVERSION。値は `commands::Category` の値に揃え、`static_assert` で固定)を出す。「実際に見えているか」を出すなら、`FillVisibility` と同じく `IsVisible` を通した結果を `TipUiHandler::Update` の中で書く(R1-I1)。

### R1-C3. D5 の鮮度判定は成り立たず、しかも ADR-265 の読み取り規約と矛盾する

- D5 は「`update_tick_ms` が直近の打鍵より新しいかを確かめて古い値を使わない」とする。しかし publisher は**変化が無ければ書かない**(`tip_state_publisher.cc` `Flush()`: `if (published_ && last_ == pending_) return false;`)。打鍵を処理したが値が変わらなかった(例: 直接入力中のキー、カーソルが既に末尾での →、組成外の Enter)と、まだ処理していないの区別がつかない。前者で読み手は永久に「古い」と判断する。
- ADR-265 D3 の読み取りの規約は「`update_tick_ms` による新しさの判定はしない(publisher は変化がなければ書かないので、生存の印にならない)」と明記している。D5 はこれと正面から矛盾する。
- `seq` が進んだかも同じ理由で使えない(変化しなければ進まない)。また高速打鍵中は、seq が進んでいても「直前の 1 打鍵ぶんだけ遅れた値」かどうかが分からない。
- **直し方**: TIP が処理したキーダウンの通番 `key_seq`(u32、`TipKeyeventHandler::OnKeyDown` で eaten の有無に関わらず +1、ラップあり)を足し、これだけは毎回書く(pulse はしない、R1-I6)。awase は「自分が送ったキーの数」と突き合わせて、TIP が追いついたかを判定できる。これが R1-C1 の「キーを受け取ったかの直接の証拠」にもなる。キーの中身は出さないので D3 の方針とも合う(打鍵の時刻は漏れる、R1-M3)。

---

## 重要

### R1-I1. Publish の位置についての記述が実コードと違い、そのまま置くと取りこぼす

ADR 背景の「現行の `Publish()` もこの処理(`UpdatePrivateContext`)の近くから呼ばれており、同じ場所で値を渡せる」は不正確。

- 現行の `Publish` は `TipInputModeManager` の各 mutator の `PublishOnExit`(`tip_input_mode_manager.cc:168,191,207,236,250,273`、本体 315〜318 行 `PublishState()`)からだけ呼ばれ、`Output` を受け取らない。`Publish(effective_open, effective_mode, tsf_open, tsf_mode)` の引数に preedit を足す形にはできない(`TipInputModeManager` は `Output` を持っていない)。`SetActive` と同じく別の setter(例 `SetComposition(...)`)が要る。
- `UpdatePrivateContext`(`tip_edit_session_impl.cc:396`)は `if (!output.has_status()) return S_FALSE;`(403 行)で早期に返る。その後ろに置くと、status を持たない Output で preedit の更新を取りこぼす。置くなら `*private_context->mutable_last_output() = output;` の直後。
- `DoEditSessionInComposition`(同 465〜475 行付近)は `UpdatePrivateContext` を **`UpdatePreeditAndComposition` より前に**呼ぶ。ここで公開すると、文書へ `SetText` する前で、`UpdatePreeditAndComposition` が `E_FAIL`(`CreateComposition` 失敗、`InsertTextAtSelection` 失敗)でも「composing=1」が出る。
- 公開場所は `TipEditSessionImpl::UpdateContext` / `OnCompositionTerminated` の末尾(`UpdateUI` の直前か `TipUiHandler::Update` の中)にし、`last_output()` から値を作るのが正確。こうすると R1-C2 の `IsVisible` も同じ場所で取れる。
- D5 の「TIP が `Output` を受け取った時点」も不正確。公開は編集セッションの中で、`OnOutputReceivedImpl`(`tip_edit_session.cc:489-`)は Word の回避策で `TF_ES_ASYNC` に切り替えることがある(510〜517 行)。受け取りと公開の間に、アプリがロックを与えるまでの遅れが入る。

### R1-I2. version 3 の読み手が version 2 のブロックを 64 バイトでマップすると失敗する(逆方向の互換)

D1 は「version 2 の読み手が 48 バイトで map しても読める」だけを書いているが、問題は逆方向。

- MSI を上げても、既に `mozc_tip64.dll` を読み込んでいる長寿命のプロセス(explorer、Chrome、常駐アプリ)は再起動まで旧 DLL のまま、48 バイトのセクションを作り続ける。新旧のブロックが同時に存在する。
- `MapViewOfFile(..., 64)` はセクションサイズ(48)を超えるので失敗する。読み手が 64 固定で map すると、その窓では version 2 のフィールド(開閉・モード)まで読めなくなり、ADR-265 段階 1 の観測が退行する。AppContainer 経路(`NtMapViewOfSection` の `ViewSize`)も同じ。
- 規約として仕様書に書くべきこと: 「サイズ 0(セクション全体)で map するか 48 で map して `version` を読み、`version >= 3` のときだけ 64 で map し直して 48 以降を読む」。`examples/read_ime_state.py` の `assert STRUCT.size == 48`(19〜20 行)と `MapViewOfFile(..., STRUCT.size)`(67 行)を version 3 対応で書き換えるときにこの罠を踏みやすい。
- publisher 側の変更点も ADR に列挙が無い: `static_assert(sizeof(TipSharedState) == 48)`(`tip_state_publisher.h:73`)を 64 に、`Snapshot` と `operator==`(`tip_state_publisher.cc:156-167`)と `Flush()` の書き込み(185〜201 行)に新フィールドを足す。`Snapshot` に入れ忘れると、ADR-265 R2-C2a と同じく「変化したのに書かれない」が起きる。
- 端の場合: `CreateFileMappingW` は同名のセクションが残っていると**既存のサイズ(48)のまま**返す(`ERROR_ALREADY_EXISTS`)。続く `MapViewOfFile(..., 64)` が失敗し、`tried_ = true` なのでそのスレッドでは二度と書かない(`Get()`、204〜236 行)。同じ TID の古いセクションを誰かが開いたままにしていると起きる。`ERROR_ALREADY_EXISTS` のときの扱い(48 だけ書く/諦めてログ)を決めておく。

### R1-I3. Q3(変換前/変換中の区別)は後回しにできない

- Mozc の BackSpace は、変換中(Space 後)は文字を消さず**変換を取り消して読みに戻す**。変換中の preedit は変換後の表記(「漢字」2 文字)で、読み(「かんじ」3 文字)と長さが違う。`preedit_chars` を BS の回数として使うと、変換中は 1 回目で読みに戻り、残りの回数で読みを途中まで消す。
- QMK の Enter の意味も、組成中(読みを確定)と変換中(候補を確定)で違う(R1-C2)。
- 取れる材料は実コードにある: preedit のセグメントの `annotation == HIGHLIGHT` は変換中の注目文節にだけ付く(`tip_edit_session_impl.cc` 310〜320 行付近、`tip_ui_handler_conventional.cc:79-88` `GetTargetPos`)。`candidate_window.category == CONVERSION` も同じ判定に使える。
- **直し方**: `composing` を 0/1 ではなく `phase`(0=無し、1=組成中、2=変換中)にし、`candidate_category`(R1-C2)と合わせて version 3 で一度に出す。後から足すと version 4 か「version 3 の途中追記」になり、読み手の分岐が増える。

### R1-I4. 組成の状態は ITfContext ごとだが、共有メモリはスレッドごとに 1 つ

- `last_output` は `TipPrivateContext`(ITfContext ごと、`tip_edit_session_impl.cc:401`)に入る。共有メモリのスロットは `thread_local ThreadSlot`(`tip_state_publisher.cc:246-249`)で 1 スレッド 1 つ。
- 同じスレッドに複数の context がある窓(ダイアログ内の複数の入力欄、ブラウザの複数フレーム)で、A で組成中に B へフォーカスが移ると、A の組成が確定されない限り共有メモリには `composing=1` が残る。B 側で Output が来るまで(キーを押すまで)更新されない。`OnSetFocus`(`tip_text_service.cc:720`)→`OnDocumentMgrChanged`→`OnSetFocusAsync` が Output を流すかは未確認。
- `Deactivate`(`tip_text_service.cc:448-456`)は `SetActive(false)` だけで組成のフィールドを触らない。version 2 の読み手と違い、QMK のように `composing` だけを見る読み手は、IME を切り替えた後も `composing=1` を見続ける。仕様書に「`active==0` のときは他のフィールドを無視する」を必須として書くか、Deactivate で 0 に落とす。
- 段階 1 の CI に「A で組成中 → 同じ窓の B へフォーカス移動 → `composing==0` か、少なくとも何が出るか」と「組成中に Win+Space で別 IME → 何が出るか」を足す。

### R1-I5. D2 の根拠が対象とずれている。UTF-16 長は TIP がすでに持っている

- D2 の「BackSpace はアプリが 1 文字として扱う単位で消す…アプリごとに 1 回か 2 回かが違う」は**確定済みテキスト**の話。組成中の BS は TIP が食べて Mozc の composer が処理する(アプリの書記素の規則は効かない)。preedit_chars が描くのは組成中のテキストなので、根拠が噛み合っていない。
- TIP が文書に書く長さは UTF-16(`tip_edit_session_impl.cc:285-288` `ComposePreeditText` → `SetText(..., preedit_text.size())`)。TSF/IMM の composition 文字列と突き合わせる読み手(awase は UTF-16 で扱う)には UTF-16 長の方が直接使える。カーソルは UTF-32(370 行のコメント「`output.preedit().cursor()` is in the unit of UTF-32」)なので、ADR の `preedit_cursor` = コードポイント数は正しい。
- 「コードポイント数 ≥ UTF-16 長 / 2」は常に真で、読み手の役に立たない。Mozc の候補には IVS(例: 葛󠄀 = 2 コードポイント・UTF-16 で 3)が出るので、コードポイント・UTF-16・書記素の 3 つが全部食い違う場合がある。
- **直し方**: 長さは UTF-16 で出す(`preedit_utf16_len`)。コードポイント数が要る読み手は少ないので、要るなら両方。どちらにするかを決めた理由を「組成中の BS は Mozc が処理する」に置き換える。

### R1-I6. D6 の対策 (a) は実測を待たずに決められる。待つと段階 1 が止まる

- 書き込みのコストは `InterlockedIncrement` 2 回と数個のストアで、打鍵ごとに既に起きている mozc_server への IPC 往復(ms 単位)と比べて無視できる。D6 が挙げる「キー→反映の遅さ」「ホストアプリの入力遅延」の測定は、書き込みについては結果が見えている。問題は pulse による読み手の起床だけ。
- (b) の「イベントを 2 つに分ける」は、イベント名を 1 つ増やし、identity の置換表(ADR-265 D4-3)と AppContainer の名前空間(ADR-265 で通知イベントの名前空間は未確認)の両方に波及する。
- pulse(`SetEvent`→`ResetEvent`、`tip_state_publisher.cc` `Pulse()`)は取りこぼす前提(ヘッダのコメント 37〜41 行)なので、preedit の長さを必要とする読み手(awase)はどのみち判断の時点で読みに行く。pulse で知りたいのは `phase`/`candidate_category`/開閉/モード/`active` の変化だけ。
- **直し方**: (a) で確定し、Q2 を閉じる。`Flush()` を「書くかどうか」(全フィールド比較)と「pulse するかどうか」(状態グループの比較)の 2 つの判定に分ける。測定は「連続打鍵で pulse の回数が状態の変化回数と一致する」ことの CI 確認だけでよい。

---

## 軽微

### R1-M1. 例のパスが違う

段階 4 の `examples/read_ime_state.py` は、実際は `readable/docs/examples/read_ime_state.py`。

### R1-M2. 段階 1 の CI 期待値が Mozc の既定と合わない可能性

- `k` の時点の preedit は Mozc の既定で全角の「ｋ」(1 コードポイント・UTF-16 で 1)なので値は合うが、`preedit_chars` の単位を UTF-16 に変える場合も含め、期待値は「1」で同じ。問題は `candidates_visible`(R1-C2)。`ka` の時点でサジェストが出ていれば Space の前に 1 になる。期待値を `candidate_category` で書く(`ka`→0 または SUGGESTION、Space→CONVERSION)。
- Enter→0 は、変換中の Enter で確定した直後に Mozc が次のサジェスト(確定後の予測)を出す設定だと 0 にならないことがある。設定を CI で固定する。

### R1-M3. D3/Q1: パスワード欄の心配は小さく、漏れるのは打鍵の時刻

- Mozc はパスワードの入力スコープで入力モードを切り替える(`tip_edit_session_impl.cc:505-516` `OnChangeInputScope`)。組成が起きにくいので、Q1 の「0 に落とす」は実害が小さい(どのモードに落ちるかは `TipInputModeManager::OnChangeInputScope` で確認)。
- version 3 で新しく漏れるのは、打鍵ごとに進む `update_tick_ms`(と R1-C3 の `key_seq`)による**打鍵の時刻**。打鍵の間隔は既知のサイドチャネル。ただし同一ユーザーのプロセスは低レベルフックで打鍵そのものを取れるので、ADR-265 R1-M7 の「同一ユーザーは信頼境界の内側」の範囲内。Q1 はこの理由を書いて閉じてよい。

### R1-M4. version を上げずに済ませる代案は無い(確認済み)

- v2 で `reserved` は `active` に使い切っており(`tip_state_publisher.h:57`、70 行)、空きフィールドは無い。追記には version を上げる必要がある(上げないと読み手が 48 以降の有無を判定できない)。D1 の version 3 は妥当。
- 別のマッピング名(例 `Local\mozc-readable-comp-<tid>`)に分ける案は、R1-I2 の互換問題が消え、更新頻度の違う値を分けられる利点がある。ただし AppContainer 経由の読み取り(ADR-265 の `NtOpenSection`)と identity の置換表が 2 倍になる。同じマッピング + R1-I2 の読み手規約の方が安い。却下の理由として ADR に 1 行残しておくとよい。

### R1-M5. D4 の「候補の数を足さない」は R1-C2 の category と矛盾しない

`candidate_count` を足さない判断はそのままでよい。ただし R1-C2 の `candidate_category` は「有無」の判定に必要な最小の情報で、D4 の「表示順・ページ送りと絡む」懸念には当たらない。D4 の文面で category まで除外していると読まれないよう、対象を「数・選択位置」に限ると書く。

---

## まとめ(この版で直すべき順)

1. 需要 1 の書き直し(R1-C1)。BS 数の検算は対象外と明記し、awase の需要は「TIP がキーを受け取ったか」(`key_seq`、R1-C3)に置き換えるか、未確認として段階から外す。
2. フィールドを `phase` / `candidate_category` / `preedit_utf16_len` / `preedit_cursor` / `key_seq` に組み直す(R1-C2・I3・I5・C3)。
3. 公開位置を `UpdateContext`/`OnCompositionTerminated` の末尾に(R1-I1)、context とスレッドのずれと Deactivate の扱い(R1-I4)、読み手の map の規約(R1-I2)を仕様に書く。
4. Q2 は (a) で閉じる(R1-I6)。Q1 は理由を書いて閉じる(R1-M3)。Q3 は本 ADR に取り込む(R1-I3)。
