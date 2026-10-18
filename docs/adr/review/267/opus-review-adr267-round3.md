# ADR-267 Opus 敵対的レビュー round 3

対象: `docs/adr/267-mozc-readable-composition-state.md` 第3版(`a969f13a`)
照合した実コード: `cuzic/mozc-readable` の `readable` ブランチ(round1・2 と同じ取得分、`tip_input_mode_manager.h` と `tip_ui_handler.cc` を追加で確認)。

集計(新規): 致命 0 / 重要 1 / 軽微 4

---

## round2 の指摘への対応の確認

| ID | 判定 | 補足 |
| --- | --- | --- |
| R2-C1 | 妥当(縮小で解消) | 下の「last_change_cause を外した判断について」。 |
| R2-I1 | 解消 | 数える場所は `TipKeyeventHandler::OnTestKeyDown`(`tip_keyevent_handler.cc:473`)の入口にすれば、`OnTest` の中の早期 return(`private_context == nullptr`・`GetKeyboardState` 失敗)より前で数えられる。R3-M1 で場所を固定する文言だけ足すとよい。 |
| R2-I2 | 一部 | `composition_key_seq` の導入は正しい。ただし「公開した時点の `key_seq`」という定義では、組成の値より新しい番号が付くことがある(R3-I1)。 |
| R2-I3 | 解消 | `scope_forced_mode`。実装上の注意は R3-M2。 |
| R2-I4 | 解消 | D8 第 4 項・Q1。 |
| R2-M1〜M5 | 解消 | `TipUiHandler::Update`(`tip_ui_handler.cc:95` が `TipUiHandlerConventional::Update` へ振り分ける 1 か所)。ERROR_ALREADY_EXISTS、Deactivate の列挙、配置もよい。 |

### last_change_cause を外した判断について(team-lead からの確認依頼)

**妥当と考える。** 理由は「費用に見合わない」に加えて、もう 1 つ ADR に書いておくべき根拠がある。

- 打鍵なしの遷移のうち、awase が**既存の観測で見えないもの**は入力スコープの上書きだけである。言語バーのメニュー、候補窓のマウス操作、他のクライアントの `ImmSetOpenStatus` は、どれも最後に TSF の compartment へ書かれる(言語バーは `SwitchInputModeAsync` → `UpdateContext` → `UpdatePrivateContext` → `TipStatus::SetIMEOpen`/`SetInputModeConversion`、`tip_edit_session_impl.cc:413-430`)。このため awase は compartment の観測(と version 2 の `tsf_*`)で既に遷移の瞬間を見ている。原因が分からないだけで、遷移は見えている。
- compartment に出ないのは、入力スコープの上書き(`OnChangeInputScope`/`OnSetFocus` が `mozc_state_` だけを変え、`TipStatus` を呼ばない)だけで、これは `scope_forced_mode` と version 2 の `effective_*` で見える。アプリによる組成の打ち切り(`OnCompositionTerminated`)も `phase` で見える。
- したがって `last_change_cause` が足すのは「原因のラベル」だけで、見えない遷移を見えるようにはしない。しかもそのラベルは、R2-C1 のとおり awase 自身の書き込みを除かないと害になる。外す判断は、ユーザー指摘の動機(打鍵なしの遷移を知る)を損なっていない。

**ADR への提案(軽微)**: D8 第 1 項の理由に、上の 1 点目(「打鍵なしの遷移は、入力スコープを除き compartment に出るので既に見えている。`last_change_cause` が足すのはラベルだけ」)を 1 文足す。費用の議論だけだと、後で「安く作れる方法が見つかった」ときに再提案されやすい。

---

## 重要

### R3-I1. `composition_key_seq` を「公開した時点の `key_seq`」にすると、まだ反映していない打鍵の番号が付く

D1・D5 は `composition_key_seq` を「`phase`・`candidate_category`・`preedit_*` を書いた時点の `key_seq` の値」と定義する。公開は `TipUiHandler::Update` の中(D7)で、`key_seq` は `OnTestKeyDown` で進む。この 2 つは同じ打鍵について順序が保証されない。

- **非同期の編集セッション**: Word の回避策(`tip_edit_session.cc:510-517`)で打鍵の編集セッションが `TF_ES_ASYNC` になると、打鍵 N の `OnKeyDown` は編集セッションを予約して戻る。アプリがロックを与える前に次の `WM_KEYDOWN` が処理されると、打鍵 N+1 の `OnTestKeyDown` で `key_seq` が N+1 になる。その後に打鍵 N の編集セッションが走って公開すると、`composition_key_seq = N+1` が書かれる。中身は打鍵 N の `Output` なのに、読み手は「N+1 まで反映済み」と判断して古い `phase`/`preedit_utf16_len` を採る。
- **打鍵と無関係な `Update`**: D7 が利点として挙げた `OnEndEdit`(`tip_edit_session_impl.cc:590-600` 付近、`update_ui`)と `AsyncSetFocusEditSessionImpl`(`tip_edit_session.cc:157-160`)からの `Update` も同じ問題を起こす。打鍵 N の `OnTestKeyDown` の後、編集セッションの前に、アプリ自身の編集で `OnEndEdit` が走ると、その時点の `last_output()`(打鍵 N−1 の結果)を `composition_key_seq = N` で公開する。同期の経路でも、アプリが `OnTestKeyDown` と `OnKeyDown` の間に文書を編集する場合に起きうる。
- **直し方**: 番号を公開の時点で読まず、**`last_output` を更新した時点で、その `Output` を生んだ打鍵の番号を記録**する。具体的には、`TipPrivateContext` に `last_output_key_seq` を持たせる。`UpdatePrivateContext` の `*private_context->mutable_last_output() = output;`(`tip_edit_session_impl.cc:401`)で、編集セッションのオブジェクトが運んできた番号を入れる。`SyncEditSessionImpl` を作る `OnOutputReceivedImpl` で、その時点の `key_seq` を保存しておく。`OnTestKey` の中で `last_output` を書き換える箇所(`tip_keyevent_handler.cc:265` 付近、`should_be_sent_to_server && temporal_output.has_consumed()`)も、その打鍵の番号で記録する。`Update` は `last_output_key_seq` をそのまま `composition_key_seq` として書く。打鍵に由来しない `Output`(言語バー・`OnCompositionTerminated` の SUBMIT など)は、直前の値を引き継ぐ(進めない)。
- これは R2-C1 で問題になった「入口の情報を編集セッションで運ぶ」と同じ形の作業だが、運ぶのは打鍵の番号 1 つだけで、運ぶ経路も `OnOutputReceivedImpl` の 1 か所に限られる。
- CI の `key_seq` の行(段階 1)に、「連続打鍵中に `composition_key_seq` が、その時点の `preedit_utf16_len` が実際に反映している打鍵の番号を超えない」ことを足す(Word の回避策が効くアプリか、`TF_ES_ASYNC` を強制する CI 用のスイッチで確かめる)。

---

## 軽微

### R3-M1. `key_seq` を数える場所を「関数の入口」と書く

D1 は「`TipKeyeventHandler::OnTestKeyDown` と `OnPreservedKey` の 2 か所で +1」とする。前者は `OnTest(...)` を呼ぶだけの薄い関数なので入口で数えればよいが、実装者が中の `OnTest` の `eaten` を決めた後に置くと、`private_context == nullptr`(管理外の context)や `GetKeyboardState` の失敗で早期 return するキーが数えられない。仕様書に「早期 return より前、関数の入口で数える」と書く。

あわせて、アプリが `ITfKeystrokeMgr::KeyDown` を `TestKeyDown` なしで直接呼ぶ場合は `OnTestKeyDown` を通らず数えられない(未確認)。D5 第 2 項の「`TestKeyDown` を複数回呼ぶと重複する」と並べて、逆向きのずれとして書いておく。CI(Chrome・WezTerm・Notepad)で、送った打鍵数と `key_seq` の増分を 1 回ずつ記録しておくと、どちらのずれがあるかが分かる。

### R3-M2. `scope_forced_mode` の値と実装の注意

- `GetOverriddenState` は `static` で `StatePair` を返すだけ(`tip_input_mode_manager.h:61-62`)なので、「`states.size() == 1` だったか」と「どのモードか」は外から取れない。戻り値を拡張するか、`input_scope_` から別に計算する関数を足す必要がある。`input_scope_` は `OnSetFocus` と `OnChangeInputScope` だけが更新し、どちらも `PublishOnExit` を持つので、公開の経路はそのまま使える(`SetComposition` ではなく `Publish` 側に足す)。
- `DIRECT` の上書きは「モードを変える」のではなく「IME を閉じる」(`StatePair(false, base_state.conversion_mode)`、モードは元のまま)。`scope_forced_mode = DIRECT + 1 = 1` を読み手が「直接入力モード」と読むと、`effective_mode` と食い違って見える。仕様書に「1(DIRECT)は閉を意味し、`effective_mode` は変わらない」と書く。
- 値は `commands::CompositionMode`(DIRECT=0〜HALF_KATAKANA=5)。ADR-265 D4-4 の `static_assert`(0〜5 の固定)を `scope_forced_mode` にも適用する、と書く。

### R3-M3. `key_seq` の大小比較はラップを考慮した書き方を仕様書で固定する

D5 第 3 項の「`composition_key_seq >= 自分が送った打鍵の番号`」は、u32 がラップすると逆転する。現実的に 2^32 打鍵には届かないが、awase は自分の打鍵の番号を「読み始めた時点の `key_seq` + 送った数」で作るので、起点が大きいと境界をまたぐ。比較は `(int32_t)(a - b) >= 0` の形と仕様書に書き、例の Python も同じ書き方にする。

### R3-M4. reserved(72)を将来の追記に使うときの扱いを決めておく

`reserved` が 72 に入り、`composition_key_seq` が 76 にある。将来 `last_change_cause` を足すとき(D8)、72 を使えば version を上げずに済むように見える。だが version 3 の読み手は 72 を無視するので、意味を持たせるなら「0 は不明」と決めておけば version を上げずに追記できる。そうしないなら version 4 にする。どちらにするかを D1 に 1 行書いておくと、「互換を壊すときだけ version を上げる」の取り決めとの関係が後で議論にならない。
