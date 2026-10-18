# ADR-267 Opus 敵対的レビュー round 2

対象: `docs/adr/267-mozc-readable-composition-state.md` 第2版(`f4352cbd`)
照合した実コード: `cuzic/mozc-readable` の `readable` ブランチ(2026-10-18 に取得)。`tip_input_mode_manager.cc`・`tip_keyevent_handler.cc`・`tip_text_service.cc`・`tip_edit_session.cc`・`tip_edit_session_impl.cc`・`tip_ui_handler_conventional.cc`・`tip_state_publisher.{h,cc}`。

集計(新規): 致命 1 / 重要 4 / 軽微 5

---

## round1 の指摘への対応の確認

| ID | 判定 | 補足 |
| --- | --- | --- |
| R1-C1 | 解消 | 背景の需要 2 から BS の検算を外し、理由と根拠の関数名がある。 |
| R1-C2 | 解消 | `candidate_category`(IsVisible 越し)。公開位置の細部は R2-M1。 |
| R1-C3 | 一部 | `key_seq` の導入は妥当。ただし数える場所(`OnKeyDown`)では「eaten の有無に関わらず」にならず(R2-I1)、組成の項目がどの打鍵まで反映済みかが分からない(R2-I2)。 |
| R1-I1 | 解消 | D7。 |
| R1-I2 | 一部 | 読み手の規約は良い。書き手の `ERROR_ALREADY_EXISTS` 時の振る舞いが不足(R2-M2)。 |
| R1-I3 | 解消 | `phase`。予測の選択中の扱いは R2-M3。 |
| R1-I4 | 一部 | Deactivate で 0 に落とす項目の列挙が不足(R2-M4)。 |
| R1-I5 | 解消 | |
| R1-I6 | 解消 | |
| R1-M1〜M5 | 解消 | |

---

## 致命

### R2-C1. `last_change_cause` は、提案の作り方だと原因を系統的に取り違える。とりわけ awase 自身の書き込みを「外部」と申告し、後で belief に入れると drift correction との往復を作る

ADR の列挙(0=不明、1=打鍵、2=アプリの指示、3=フォーカス移動、4=外部〈Win+Space など〉)を、実コードの状態を変える入口と突き合わせると、1 つの mutator が複数の入口から呼ばれ、原因を mutator 側では決められない。

| 状態を変える mutator(`tip_input_mode_manager.cc`) | 実際の入口 | ADR の原因 |
| --- | --- | --- |
| `OnReceiveCommand`(168 行付近) | 打鍵(`OnKey`→`OnOutputReceived*`→`UpdateContext`)、**言語バーのメニュー**(`tip_text_service.cc:918,944` `SwitchInputModeAsync` → `AsyncSwitchInputModeEditSessionImpl` → `UpdateContext`)、**候補窓のマウス操作**(`OnRendererCallbackAsync`)、`OnCompositionTerminated` の SUBMIT(アプリによる組成の打ち切り)、`AsyncSessionCommandEditSessionImpl` | 全部「1=打鍵」になる |
| `OnChangeOpenClose` / `OnChangeConversionMode` | compartment の `OnChange`(`tip_text_service.cc:894-905`)。**どのクライアントの書き込みでも呼ばれる**: IMM 経由の `ImmSetOpenStatus`/`ImmSetConversionStatus`(**awase の actuation そのもの**)、他アプリ、そして **Mozc 自身**が `UpdatePrivateContext` で書く `TipStatus::SetIMEOpen`/`SetInputModeConversion`(`tip_edit_session_impl.cc:413-430`)の echo | 「4=外部」 |
| `OnSetFocus` | フォーカス移動。ただし移動先の入力スコープの上書き(`GetOverriddenState`)も同じ関数で行う | 3 か 2 か決まらない |
| `OnChangeInputScope` | `OnEndEditImpl`(`tip_edit_session_impl.cc:505-516`)。同じ context 内で選択位置が別のスコープの範囲へ移ったとき | 2 |

具体的な失敗シナリオ:

1. **awase 自身の書き込みが「外部」になる。** awase が IME を開く(`ImmSetOpenStatus` → TSF の compartment)→ Mozc の `OnChange` → `OnChangeOpenClose` で開閉が変わる → `last_change_cause=4`。段階 3 で D8 を belief に入れ「打鍵由来ではない変化」として扱うと、awase は自分の actuation を外部からの変化として観測し、それを根拠に belief を動かす。ADR-212 で段階撤去した「補正的な actuation」と drift correction の往復(`.claude/rules/ime-belief-architecture.md` の背景の事故類型)と同じ形になる。D8 が「最初は journal のみ」としているのは正しいが、journal を見て昇格を判断する人が、この取り違えを知らずに一致率を読むと誤る。
2. **原因が数 ms で上書きされる。** Mozc 自身の書き込みの echo は、値が変わらないので `OnChangeOpenClose` が `kDoNothing` を返す(`prev_open != new_open_close_mode` の判定)。ここで原因を「値が変わったときだけ」記録すれば上書きは起きない。ただし ADR は「原因を記録する条件」を書いていない。入口で無条件に記録すると、打鍵の直後に「4」で上書きされる。
3. **「Win+Space」は compartment の変化ではない。** Win+Space は TIP のプロファイル切り替えで、Mozc 側では `Deactivate`/`ActivateEx`(`active` の 0/1)として現れる。開閉・モードの compartment は変わらないので、原因 4 の例として誤り。原因 4 に実際に入るのは、IMM/TSF 経由で compartment に書くクライアント(awase、他のリマッパー、アプリの `ImmSetOpenStatus`)。
4. **非同期の編集セッションで入口の情報が消える。** 入口で `thread_local` の「いまの原因」を立てる実装は素直に見えるが、`OnSwitchInputModeAsync`・`OnSetFocusAsync`・Word の回避策で `TF_ES_ASYNC` になった打鍵(`tip_edit_session.cc:510-517`)は、入口を抜けた後で編集セッションが走る。原因は**編集セッションのオブジェクトに持たせて運ぶ**必要がある。

**直し方の案**:

- 原因は mutator ではなく**入口**で決め、編集セッションのオブジェクトに入れて運ぶ。列挙を実際の入口に合わせる: 1=打鍵(`OnTestKeyDown`/`OnKeyDown`/`OnPreservedKey`)、2=入力スコープ(`OnEndEditImpl`・`OnSetFocus` でスコープの上書きが変わったとき。`OnSetFocus` ではスコープの結果が変わったら 2、それ以外は 3)、3=フォーカス移動、4=compartment への書き込み(**awase 自身を含む**と仕様書に明記)、5=言語バー・候補窓などのマウス操作、6=アプリによる組成の打ち切り(`OnCompositionTerminated`)。
- 原因は「状態の項目が実際に変わったときだけ」書く(echo で上書きしない)。
- awase 側の扱いを ADR に書く: 原因 4 は「awase 以外」を意味しない。awase は自分の actuation の時刻(または `key_seq`、R2-I2 の印)と突き合わせて、自分の書き込みの echo を除いてから数える。

---

## 重要

### R2-I1. `key_seq` を `OnKeyDown` で数えると「eaten の有無に関わらず」にならない。数えるのは `OnTestKeyDown`

- TSF は `ITfKeyEventSink::OnTestKeyDown` で `*eaten=FALSE` が返ったキーについて、`OnKeyDown` を呼ばない。mozc-readable では `OnTestKey`(`tip_keyevent_handler.cc` 150〜270 行付近)が `*eaten = result.should_be_eaten` を返し、IME が閉じているとき・直接入力のとき・`VKBackBasedDeleter` が `SEND_KEY_TO_APPLICATION` を返したとき(196 行付近)などは食べない。これらのキーは `OnKeyDown` に来ないので、D1 の定義どおりに実装すると `key_seq` は進まない。
- 失敗シナリオ: IME を閉じた状態で awase がキーを 3 つ送る。TIP は 3 つとも `OnTestKeyDown` で見ているが、`key_seq` は 0 のまま。読み手は「TIP がキーを受け取っていない」(cold・TIP 未接続)と区別できない。R1-C3 が求めた「受け取ったかの直接の証拠」が、IME が閉じているときにだけ逆の意味になる。
- `OnPreservedKey`(`tip_text_service.cc:822-849`)は `OnTestKeyDown` を通らずに `TipKeyeventHandler::OnKeyDown` を直接呼ぶ。数える場所を `TipKeyeventHandler::OnTestKeyDown` と `OnPreservedKey` の 2 か所にし、`OnKeyDown` では数えない(数えると食べたキーが 2 回数えられる)。
- アプリが `ITfKeystrokeMgr::TestKeyDown` を同じキーで複数回呼ぶ場合は重複して数えられる(未確認)。仕様書に「`key_seq` は TIP が見たキーダウンの回数の目安であり、送ったキーの数と完全に一致するとは限らない」と書き、CI で「IME を閉じた状態のキーでも進む」を確かめる項目を足す(段階 1 の `key_seq` の行は「値が変わらない打鍵でも進む」だけで、閉の場合が無い)。

### R2-I2. `key_seq` と組成の項目は別の時点に書かれるので、「組成の値がどの打鍵まで反映済みか」が分からない

- `key_seq` は打鍵を受けた時点(`OnTestKeyDown`、編集セッションの外)で進み、`phase`/`preedit_*`/`candidate_category` は編集セッションの末尾(D7)で書かれる。非同期の編集セッション(Word の回避策など)では、この間にアプリがロックを与えるまでの遅れが入る(D5 第 3 項も認めている)。
- 失敗シナリオ: awase が `k` を送り、`key_seq` が N に進んだのを見て「TIP が追いついた」と判断し、`phase` を読む。その時点では `k` の編集セッションはまだ走っておらず、`phase=0` のまま。awase は「`k` を送ったのに組成が始まっていない」と誤認する。D5 の「TIP が追いついたかを判定する」は、`key_seq` だけでは組成の項目について成り立たない。
- **直し方**: `SetComposition` の時点の `key_seq` の値を `composition_key_seq` として一緒に書く(reserved の 76 を使える)。読み手は「`composition_key_seq >= 自分が送った打鍵の番号`」のときだけ組成の項目を採る。80 バイトの配置は変わらない。

### R2-I3. `override_active` の定義(「`effective_*` が Mozc セッション本来の値から上書きされている」)は、実コードの状態の持ち方と合わず、打鍵 1 つで消える

- 実コードでは、入力スコープの上書きは `GetOverriddenState(tsf_state_, input_scopes)` の結果を `mozc_state_`(= 公開の `effective_*`)に入れることで表される(`tip_input_mode_manager.cc` `OnSetFocus`・`OnChangeInputScope`)。上書きの元は TSF の compartment 側(`tsf_state_` = 公開の `tsf_*`)で、「Mozc セッション本来の値」ではない。ADR の文言は向きが逆。
- **開閉の上書きはサーバーの応答で消える。** `OnReceiveCommand` は `tsf_state_.open_close` と `mozc_state_.open_close` の**両方**に `status.activated()` を入れる。パスワード欄(`IS_PASSWORD` → `DIRECT` → `StatePair(false, ...)`)で利用者が半角/全角を押して IME を開くと、両方が開になり、`effective_*` と `tsf_*` の差は消える。入力スコープはパスワードのまま。「差がある」で `override_active` を定義すると、ここで 0 になる。
- **モードの上書きは残る。** モードは `tsf_state_.conversion_mode = comeback_mode`、`mozc_state_.conversion_mode = mode` と別の値が入る。開閉とモードで振る舞いが違う。
- **差が出ない上書きがある。** `IS_HIRAGANA` のスコープに、もともとひらがなで入ると、上書きは効いているが差は 0。
- 加えて、上書きは compartment に書かれない(`OnChangeInputScope` は `kUpdateUI` を返すだけで `TipStatus::SetIMEOpen` を呼ばない)。awase が IMM/TSF から読む開閉は「開」のまま。これは D8 が言う「version 2 でも食い違いから推測できる」の根拠でもあり、正しい。
- **直し方**: 2 つの事実を分けて定義する。(a) `scope_forced_mode`: 入力スコープが 1 つのモードに決まるとき、そのモード+1(0=スコープによる指定なし)。`GetOverriddenState` の `states.size() == 1` の分岐で決まる。これは「入力欄がそう指定している」という事実で、打鍵では消えない。(b)「いま差がある」は version 2 の `effective_*` と `tsf_*` から読み手が導く(新しい項目は要らない)。`override_active` という名前は (a) と (b) のどちらにも読めるのでやめる。

### R2-I4. D8 の背景の「drift correction が推測で拾っていた類い」は、ADR-212 以降の現状と合っているか確認が要る

- D8 は awase の用途を「belief の打鍵由来ではない変化として扱える(drift correction が推測で拾っていた類い)」とする。だが入力スコープの上書きは compartment に出ない(R2-I3)ので、awase の drift correction(IMM/TSF の観測と belief の食い違いで再送する)は、これを**見ていない**はず。見ていないものを「推測で拾っていた」と書くと、段階 3 の別 ADR で「drift correction の置き換え」という誤った枠組みで議論が始まる。
- 実際に awase が困っているのが「パスワード欄で Mozc が直接入力に落ちているのに、awase は開だと思ってかなを送る」型なのかどうか、known-bugs に該当する報告があるかを確かめ、無ければ D8 の awase 側の需要は「未確認」と書く(R1-C1 と同じ扱い)。

---

## 軽微

### R2-M1. D7 の「`UpdateUI` の直前、または `TipUiHandler::Update` の中」は後者に決める

- `IsVisible` を通した `candidate_category` は `FillVisibility`(`tip_ui_handler_conventional.cc:97-140`)の中でしか計算されていない。`UpdateUI` の直前で書くなら、同じ判定をもう 1 回書くことになる。
- `TipUiHandler::Update` は `UpdateContext`・`OnCompositionTerminated` に加えて `OnEndEdit`(`update_ui`)と `AsyncSetFocusEditSessionImpl`(`kUpdateUI` のとき)からも呼ばれる。ここで書けば、同じスレッドの別 context へのフォーカス移動(D9 第 4 項)の一部も自然に拾える。
- `tip_ui_handler.cc` は conventional にだけ振り分けている(この版の `src/win32/tip` に immersive は無い)ので、1 か所で足りる。

### R2-M2. `ERROR_ALREADY_EXISTS` で 48 しか書けないときは、`version` を 2 にして書く

D9 第 2 項は「48 だけ書く」とする。だが `Get()`(`tip_state_publisher.cc:204-236`)は `state_->version = kTipSharedStateVersion` を無条件に書くので、そのままだと 48 バイトのセクションに `version=3` が載り、D9 第 1 項の読み手(48 で map → version 3 なので 80 で map し直す)が失敗する。`ThreadSlot` に「書ける大きさ」を持たせ、48 のときは `version=2` を書き、`Flush()` も 48 以降に触れないことを ADR に書く。

### R2-M3. 予測の候補を選んでいるときの `phase`

`Tab`(または `↓`)で予測の候補を選ぶと、`candidate_category=PREDICTION` で preedit が選んだ候補に置き換わる。このとき注目文節(`HIGHLIGHT`)が付くかで `phase` が 1 か 2 かが決まる。BS の動き(取り消しか削除か)も変換中と同じかどうかが分からない。段階 1 の CI の `phase` の行に「予測の候補を選んだとき」を 1 つ足す。

### R2-M4. Deactivate で 0 に落とす項目の列挙が欠けている

D9 第 3 項は `phase`・`candidate_category`・`preedit_utf16_len` だけを挙げる。`preedit_cursor`・(R2-I2 を採るなら)`composition_key_seq`・`override_active`(または `scope_forced_mode`)も 0 にする。`key_seq` と `last_change_cause` は残してよい(前者は通番、後者は Deactivate の原因として「不明」を書くか、`active` で読めるので触らない、のどちらかを決める)。

### R2-M5. 80 バイトの配置は問題ない。ただし仕様書に書き方を固定する

- `update_tick_ms`(u64)はオフセット 40 で 8 の倍数。新しい u32 × 8 は 48〜79 で、合計 80 も 8 の倍数なので、C++ の `TipSharedState` に暗黙の詰め物は入らず、`static_assert(sizeof(TipSharedState) == 80)` が成り立つ。`offsetof` で 48・64・76 を固定する `static_assert` も足すとよい(並べ替えの事故を拾える)。
- reserved(76)を外すと `sizeof` は 76 ではなく(u64 の境界のため)80 のままで、末尾の 4 バイトが初期化されない詰め物になる。reserved を明示しておくのは正しい。R2-I2 で `composition_key_seq` に使うなら、今後の追記は 80 から始まり、u64 を足すときは 8 の倍数のオフセットに置くことを仕様書に書く。
- 例の Python は `struct.Struct("<10IQ8I")`(= 80)。`<` は詰め物を入れないので、u64 の位置が 40 である限り C++ の配置と一致する。Rust の読み手は `#[repr(C)]` で同じ並びにし、`const _: () = assert!(size_of::<T>() == 80);` で固定する。
