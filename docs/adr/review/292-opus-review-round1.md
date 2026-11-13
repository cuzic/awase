# ADR-292 Opus 敵対的レビュー round1

対象: `docs/adr/292-key-effect-prediction-vs-observation-priority.md`(ブランチ `docs/adr292-key-effect-prediction-vs-observation`、HEAD `eccbb0bfb`)
読んだコード: 同ブランチの `ime_model.rs`・`mozc_shm.rs`・`platform_state.rs`・`mozc_shm_shadow.rs`・`hook.rs`・`probe_io.rs`・`class_names.rs`、および BUG-232 修正コミット `c8a4393f9`(ブランチ `docs/adr291-mozc-shm-push-notify-and-engine-recheck`)。

結論を先に書く。

- **D1b の時系列の再構成はおおむね正しい**。ただし「ADR-291 D5 との相互作用」という帰属は誤りか未確認で、遅れの本体は ADR-268 D3 R3' のローリング baseline と 500ms のポーリング周期にある(R1・R2)。
- **D1(170ms の固定の柵)は BUG-233 の結果に関係していない**。最初の送信はモードキーから 318ms 後で、柵はもう過ぎていた(R3)。
- **F2+G1+H1 を組み合わせても `ko` のリテラル化は防げない**。H1 は書かれたとおりでは何もしない(R5)。G1 の「即時」は BUG-232 の逆向きの退行を作る(R4)。
- ADR は**「観測が閉と言ったら何をするか」を決めていない**。belief を閉に直すだけだと、ユーザーには別の形の文字化けが出る(R6)。

---

## 1. 原因分析の検証

### R1(重大): D1b を「ADR-291 D5 との相互作用」とする根拠が無い。遅れは D5 が無くても起きる

- ADR-292 のブランチは `e2f438871`(develop、PR #933)から切られている。BUG-232 の修正 `c8a4393f9` は含まれていない(`git merge-base --is-ancestor c8a4393f9 HEAD` → 偽)。このブランチの `mozc_shm.rs:454-476` の `decide_use` は D5 前の形で、読みが belief と食い違っても `update_tick_ms > baseline_ms` の高速路を使う。ADR 本文が「D5 は belief と食い違う読みに無条件に settle を要求する」と書いて引用しているコードは、このブランチには存在しない。
- BUG-233 では Mozc の開閉の状態は変わっていない(閉のまま)。D5 が効くかどうかは「閉のままの Mozc の TIP が、awase の注入したローマ字や Backspace で `key_seq` などを進め、`update_tick_ms` を書き直すか」で決まる。
  - 書き直さない場合: `update_tick_ms` は 20.634 以前のまま。D5 の有無にかかわらず高速路は通らず、`now - baseline >= 170` だけが条件になる。つまり **D1b は ADR-268 D3 R3' そのものの性質**で、D5 とは関係しない。
  - 書き直す場合: D5 の前なら、21.134 付近の読みは高速路で `Use` になり、訂正はもっと早かった。この場合だけ「D5 が遅れを延ばした」と言える。
- **dragonflyg4 で 20:24 JST に動いていた awase.exe に `c8a4393f9` が入っていたか**が、ADR に書かれていない。コミットは 2026-11-12 02:47 UTC で、報告は 11:24 UTC なので、どちらもありうる。
- **要求**: awase.log の起動行からビルドのコミットを確かめて本文に書くこと。D5 が入っていなかったなら、D1b の節の「BUG-232 でついこの前導入した D5 が…」(ADR 150-155 行)と、G1 の動機(189 行)を書き直す。D5 が入っていたなら、閉の Mozc が `key_seq` を進めるかを `tickchurn` と同じ方法で IME OFF の状態で測る(CI の確証計画に足す。R9)。

### R2(中): 23.142 の説明は合っているが、ADR の根拠は弱い。本当の根拠はポーリングの位相

- `baseline_ms = hook::last_operation_tick_ms()`(`mozc_shm_shadow.rs:157`、`hook.rs:348`)は `max(hook_alive_tick_ms, imm_cross_write_tick_ms)`。`hook_alive_tick_ms` は**自己注入キー・KeyUp・ウォッチドッグのカナリアを含む全てのフックコールバック**で進む(`hook.rs:60-66` の doc)。ADR の「ユーザーが送った物理キー」という書き方は狭すぎる。awase 自身が注入したローマ字の ↓↑ と Backspace の ↑ も baseline を押し出している。
- ADR の「22.617+170ms 以降の次の tick が 23.134 近辺」という説明では、22.79〜23.29 のどの値でも「一致」になり、区別する力が無い。強い根拠は次のとおり。`TIMER_MOZC_SHM` は読むたびに 500ms で張り直される(`mozc_shm_ensure_timer`)ので、読みの時刻は 20.634(フォーカス直後に閉と読んだ時刻)+ 500k ≒ 21.134・21.634・22.134・22.634・23.134 になる。それぞれの直前の操作は次のとおり。
  - 21.134: 直前の `ko`/`re` の送信から 170ms 未満 → Held
  - 21.634: `,` の送信(21.526 と KeyUp)から 108ms 未満 → Held
  - 22.134・22.634: Backspace の最中 → Held
  - 23.134: 最後の Backspace から 170ms 以上 → Use。実測の 23.142 と 8ms 差で合う。
- **要求**: この位相による説明に差し替えるか、journal または `[mozc-shm]` の診断(`mozc_shm_shadow = true` なら読むたびに記録される)で、各読みが Held だったことを直接確かめて書く。refresh の tick(`ir_execute` → `mozc_shm_on_tick`)がこの窓で止まっていたか(Zed は TSF ネイティブ扱いか)も確かめる。refresh の tick が回っていたなら読みの時刻は上と違う。

### R3(重大): D1 の柵は BUG-233 の結果に関係していない

- 柵は `pred.at_ms`(20.671)から 170ms、つまり 20.841 で終わる(`ime_model.rs:815`)。最初のローマ字の送信は 20.989 で、柵はもう終わっていた。20.841〜20.989 の間に Medium 以上の観測が `reconcile_key_effect_open` に一度でも届いていれば、予測は消えて belief は閉に戻っていた。届かなかった理由は柵ではない。(a) mozc-shm の読みが 21.134 まで無かったこと(500ms の周期)と、(b) あっても `decide_use` が Held を返したこと(R3' のローリング baseline)だ。
- したがって「D1(確定)」と「F2(有力)」は、BUG-233 については効果がゼロ。ADR は 182 行で F2 を「G1 と対で必要」と書いているが、実際には **G1/G2 だけで D1b の遅れは変わり、F2 は何も足さない**。F2 が効くのは、モードキーから 170ms 以内に High の観測が `Use` で届く場合だけだ。そこで柵を外すと、ADR-191 決定3 が防いでいる「IME が処理する前の読み」を通すことになり、危険の方が大きい(R4 と同じ構図)。
- **要求**: D1 を「BUG-233 の原因」から「構造上の懸念(BUG-233 では効いていない)」へ格下げする。F2 は候補から外すか、効果が無いことを明記する。

### R4(重大): `resolve_open_at` の優先順位(予測が derive_any より先)自体は正しい設計で、F1 は不可

- 予測が観測より先にあるのは、柵の中で観測プールに入った古い High の読み(`report_mozc_shm` は Use のときだけ記録するが、IMM などの他の源は柵の中でも記録される。`ime_model.rs:796-799` の doc)が `derive_any` で予測を覆すのを防ぐためだ。照合が済めば `key_effect.open = None` になり、次の解決から観測が勝つ。フェンスは「確度を問わない」のではなく「時刻で古さを決める」もので、mozc-shm の High でも古さの問題は同じ(BUG-232 がまさにそれ)。
- F1 は却下と明記すべきだ。

---

## 2. 修正候補の評価

### R5(致命的): H1 は書かれたとおりでは何もしない。動いても回収が誤り

1. `judge_compose`(`mozc_shm.rs:735-741`)は、読みが `!b.effective_open` なら即座に `Skip(ImeNotActive)` を返す。H1 で `should_check_compose` の前提だけを外しても、送信後の最初の tick で閉と読まれて確認は終わり、Literal にはならない。H1 は `judge_compose` の変更も要る。
2. 仮に Literal と判定できても、回収は「Backspace × n + 同じローマ字の再送」(`set_raw_literal`、`probe_io.rs:443-464`)になる。IME は閉のままなので、再送はまたリテラルになる。2 回目は `consecutive` で諦め、残りを素通しで送る(`probe_io.rs:450-463`)。結果は `kore,` が出て消えてまた出るだけで、BUG-233 は直らない。
3. 確認が張られるのは cold の probe 経路(`dispatch_probe_actions` の `ProbeAction::Transmit`、`probe_io.rs:507`)だけだ。BUG-233 の `ko` がこの経路で送られたか(`cold=` の付いたログ・warmup の段)を ADR は確かめていない。warm の経路で送られていたなら、H1 は呼ばれもしない。`re`・`,` は 2 番目以降の送信なので、どちらにしても対象外。
- **要求**: H1 を有力から外す。本当に要るのは R7 の「送信前の照合」で、回収ではなく、送る前に止めることだ。

### R6(致命的): 「観測が閉と言ったとき何をするか」が決まっていない。belief を直すだけでは利用者の結果が変わらない

- F2・G1・G2 が成功した場合、belief は閉に戻り、Engine は非活性になり、NICOLA の打鍵はそのまま素通しになる。利用者は IME を開けるつもりで変換キーを押したので、出るのは「qwerty の生の文字」になる。`kore,` が別の文字列に変わるだけで、リテラル化は防げない。利用者の要望(「確実な状態観測に基づいてアクションしてほしい」)と所有者の方針(memory `feedback_user_intent_over_belief_2026_10_23`: ずれが直らない窓では、利用者がやりたいことを belief より優先する)からすると、正しい行動は次のどれかで、ADR はそれを選ぶ必要がある。
  - (i) 変換キーを「開きたい」という意図として扱う。観測で閉と確定したら、awase が開く書き込みをやり直す(Zed は `Imm32Unavailable` に降格済みなので ImmCross 以外の機構。どれが Zed+Mozc で効くかは要測定)。開いたと確かめてから、保留した打鍵を送る。
  - (ii) 開いたと確かめるまで打鍵を保留する(R7)。確かめられないまま期限が来たら、(i) に進むか、belief を閉に直して保留分を捨てるかを決める。
  - (iii) 何もしない(belief を閉に戻すだけ)。この場合は「BUG-233 は直さない、訂正を早くするだけ」と明記する。
- 加えて、**変換キーがなぜ効かなかったのか**(ADR の範囲外になっている)が分からないと、(i) の機構を選べない。`apply_key_effect_prediction` の doc(`platform_state.rs:565`)によれば生キーは素通りしているので、Mozc が受け取らなかったか、受け取って開かなかったかのどちらかだ。v5 ブロックの `key_seq` が 20.671 の前後で進んだかで区別できる(`mozc_shm_shadow` の診断の記録か、ADR-284 の `[mozc-compose]` の行)。Zed(GPUI)が VK_CONVERT を TIP に渡していないなら、「フォーカス直後の cold」ではなく**アプリ固有で毎回起きる**ことになる。そうなら、CI の確証計画の 1(cold を作る)は的外れになる。

### R4'(重大): G1 の「即時に Use」は BUG-232 を逆向きに再現する

- 予測から belief が開いている直後に、閉と言う読みが来たとする。この読みは「Mozc がまだ変換キーを処理していない」場合と「処理したが開かなかった」場合を区別できない。これが ADR-191 決定3 の柵の理由そのものだ。予測由来の食い違いを即時に `Use` すると、Mozc がもうすぐ開くのに belief が閉に倒れる。Engine は非活性になり、生の打鍵が開いた Mozc に入り、qwerty のローマ字として変換される(BUG-232 は「閉なのに開と読む」、こちらは「開くのに閉と読む」で、向きが逆なだけの同型)。
- G1 の「mozc-shm High なら短い settle で Use」の部分だけが成り立つ。ただしその場合、効いているのは G1 の区別ではなく「baseline の取り方」(G2)だ。G1 のシグネチャ変更(ExplicitIntent か予測かを渡す)は不要になる。

### R7(推奨案): baseline を「最後のモード系の入力」に変える(G2 の一般化)と、送信前の照合

**G3(G2 の代案)**: `decide_use` に渡す baseline のうち、食い違う読みの settle の起点を、`hook_alive_tick_ms`(全キー)ではなく、既にある `ImeModel::last_mode_key_tick_ms`(`ime_model.rs:325-329`、`since_last_mode_key_ms`)と `imm_cross_write_tick_ms` の大きい方にする。
- `last_mode_key_tick_ms` は `UserImeSetIntent`・`ImeApplyRequested`・`KeyEffectPredicted`・`ModeKeyPassedThrough` で進み、`FocusChanged` で消える(`ime_model.rs:893-898`)。ローマ字・Backspace・KeyUp・カナリアでは進まない。
- BUG-232 の場面(明示 OFF の直後の `M`)では、起点は明示 OFF(`UserImeSetIntent`)のままなので、D5 の保護はそのまま残る。G1 のような意図の種類による場合分けも要らない。
- BUG-233 の場面では、起点は 20.671 になる。21.134 の読みが `Use` になり、訂正は 2471ms から約 460ms に縮む。
- リスクとして確かめること:
  - 開閉を動かしうるのにモード系の事象として記録されない送信があるか。ADR-245 の半角英数トグルの注入、ADR-255 の Space 置換、drift correction の書き込み、`ImeApplyRequested` を経由しない awase 自身の IME キーの送信など。あれば、それを `last_mode_key_tick_ms` に入れるか、baseline に足す。
  - 一致する読みの高速路は今の baseline のままでよい(危険は食い違う読みにしかない)。
  - `FocusChanged` で `None` になったときの扱い(フォーカス直後の最初の食い違う読み)。今の `last_operation_tick_ms` を下限に使うのが安全。
- これで D1b の悪循環(Backspace が訂正を遅らせる)は直接なくなる。

**P(D2 の代案、R5・R6 の本命)**: 送信前の照合。`mozc_compose_precheck` は送信の直前に生のブロックを読んでいる(`probe_io.rs:113`)。BUG-233 では 20.989 で、モードキーから 318ms 後、柵の後の新しい読みだ。belief が予測由来で開(`decided_by == KeyEffectPrediction`、または `key_effect.open == Some(true)`)なのに、この読みが `active && !effective_open && phase==0` を示したら、**ローマ字を送らずに保留**する。同時にこの読みを観測として報告し(柵の後なので照合が通る)、R6 で選んだ行動に渡す。
- H1(送ってから回収する)より安全で、Backspace も要らない。
- 照合は cold の経路だけでなく、予測が残っている間の warm の送信にも要る(`re`・`,`)。予測が照合済みになるまでの送信全部に掛けるのが筋で、予測が照合されれば以降は確認しない。
- 送るたびの読みのコスト(`read_us`)は、予測が未照合の間だけなので小さい。

### R8(中): F3 の評価が逆

- 予測をやめると、モードキーの後、次の読みまで(最大 500ms + 170ms)belief は閉のままになり、その間の打鍵は素通しで開きかけの Mozc に入る。ADR が書く「タイピングの遅延」ではなく「誤入力」になる。成り立つのは、確かめるまで打鍵を保留する場合(P と同じ)だけだ。F3 は単独の案ではなく、P の別の言い方として整理し直すこと。

---

## 3. 検討されていない代案と関連 ADR

- **ADR-291 D2(500ms のポーリングへの依存、保留中)との関係を書いていない**。BUG-233 の遅れの半分以上は周期で決まっている(R2)。モードキーの KeyEffectPredicted の 170ms 後に 1 回だけ mozc-shm を読む「単発の読み」を予約すれば、G3 がなくても約 200ms で照合できる。これも案として挙げるべきだ(タイマーの追加なので、ADR-291 D2 と合わせて判断する)。
- **IntentStore**: `apply_key_effect_prediction` は予測のときに `intent_store.remove` する(`platform_state.rs:599-603`)。`report_mozc_shm` が食い違いのときに呼ぶ `pass_through_observed(true, true)` は `desired_open` を観測に揃える。R6 の (i) で「開きたい意図」として扱うなら、この揃えが意図を消してしまう。順序の設計が要る。
- **ADR-270・285・288**: P は送信前に止めるので、per-VK の確認(ADR-285)の対象と重ならない。H1 を採る場合は `judge_compose` の `ImeNotActive` が ADR-270 の「キーがアプリに届いたか IME が受けたかを言えない」という意図的な判断なので、変えるなら ADR-270 の追記が要る。
- **調査範囲の表**: `HwndCacheRestored` について「次の mozc-shm 観測 tick で即座に上書き」と書いているが、その読みも `decide_use` の Held を通る(R3' の柵がある)。「即座」ではない。

## 4. ADR 本文の事実誤り(小)

- 36 行・159 行: `should_check_compose` は `mozc_shm.rs:716-727` ではなく、このブランチでは `688-699`。
- 135 行: `mozc_shm_shadow.rs:157` は正しい。ただし「全ての物理キー」ではなく「自己注入・KeyUp・カナリアを含む全てのフックコールバック」。
- 146 行: 「23.134 近辺」の導出は R2 の位相の説明に差し替える。
- 189 行: G1 の「呼び出し元 2 箇所」。`decide_use` の本番の呼び出し元は `report_mozc_shm` の 1 箇所で(`grep -rn "decide_use(" crates/`)、ほかはテストだけ。

---

## 5. CI の確証計画

### R9: 足すべき検証項目

1. **変換キーがなぜ効かなかったか**(R6): Zed 相当の窓(GPUI、または `herdr-app` そのもの)で、フォーカス直後でない状態でも VK_CONVERT が Mozc を開くか。`key_seq` が進むか。毎回効かないなら、計画の 1(cold を作る)は別のシナリオに置き換える。
2. **閉の Mozc が注入キーで `update_tick_ms` を進めるか**(R1): `tickchurn` を IME OFF の状態で回す。D5 の帰属を決めるのに要る。
3. **D1b の時系列**: 計画の 2 に、`[mozc-shm]` の診断(`mozc_shm_shadow = true`)を有効にして、各読みの Held/Use と `baseline_ms` を記録することを加える。23.142 の再現は位相で決まるので、「最後のキーから 170ms 以上の静止の後の最初の tick で Use」を確かめれば足りる。
4. **修正後の退行**:
   - BUG-232(明示 OFF → `M` → 古い開の読み)が Held のままであること。G3 では起点が明示 OFF のままであることの単体テスト。
   - **予測が正しい場合**(変換キーで Mozc が 100〜300ms 遅れて開く)に、belief が閉へ一時的に倒れないこと。R4' の逆向きの退行の確認で、G1 の「即時」を採るなら必須。
   - ADR-245 の半角英数トグル・ADR-255 の Space 置換・drift correction の書き込みの直後に食い違う読みが来た場合(G3 の起点から漏れていないか)。
   - P を採るなら、warm の送信で予測が未照合の間だけ読みが増えること(送信の遅れ)の測定。
5. **Linux の単体テストで先に固定できるもの**: `decide_use` の G3 の起点の表と、`resolve_open_at` で予測が照合された後に mozc-shm が勝つことは、`awase-ime-core` の単体テストで書ける。ただし memory の注意どおり `awase-ime-core/tests` は CI で走らないので、`src/` の `#[cfg(test)]` に置く。P の判定(送るか保留か)は純粋関数に切り出して同じく単体テストにする。

---

## まとめ(推奨の組み合わせ)

| 案 | 判定 |
| --- | --- |
| F1 | 却下(R4) |
| F2 | BUG-233 には効果ゼロ、柵を外す危険だけが残る(R3) |
| F3 | 単独では誤入力を増やす。P に吸収(R8) |
| G1 | 「即時」は BUG-232 の逆向きの退行(R4')。短い settle 部分は G3 で足りる |
| G2 → G3 | 採用を推奨。起点を `last_mode_key_tick_ms` と ImmCross の大きい方にする(R7) |
| H1 | 書かれたとおりでは動かず、回収も誤り(R5)。P に置き換え |
| P(新) | 採用を推奨。予測が未照合の間の送信前に読み、閉なら送らず保留(R7) |
| R6 の行動の選択 | 決定の前提。(i) 開き直し、(ii) 保留、(iii) 直さない、のどれかを ADR で決める |
