# ADR-292 Opus 敵対的レビュー round5(最終状態・PR #936マージ後の再検証)

対象:
- ADR: `/home/cuzic/rust-nicola-worktrees/bug233-adr292/docs/adr/292-key-effect-prediction-vs-observation-priority.md`(ブランチ `docs/adr292-key-effect-prediction-vs-observation`、HEAD 4945c0d71)
- BUG: 同 worktree の `docs/known-bugs/BUG-233.md`
- コード: PR #936(`gh pr view 936` → state=MERGED、mergedAt=2026-11-12T14:15:43Z、merge commit 6439bc7d2、head 2d7752d5f)。`gh pr diff 936` で読んだ。

行番号は上記 ADR ファイルの行番号。重大度は 高/中/低。

---

## 1. ADR の記述と PR #936 の実装の食い違い

### 1-1 [中] 決定節の G3 の式に `imm_cross_write_tick_ms` が抜けている
- ADR 432-435行: 「`last_mode_key_tick_ms`と新設の`hook::ime_actuation_send_tick_ms`…の最大値」。
- 実装: `runtime/mozc_shm_shadow.rs` で `ime_actuation_baseline_ms = imm_cross_write_tick_ms().max(ime_actuation_send_tick_ms())`。`state/platform_state.rs::report_mozc_shm` で `last_mode_key_tick_ms().map_or(baseline_ms, |m| m.max(ime_actuation_baseline_ms))`。
- S1(256-257行)の式は 3 項で書かれており、実装はそちらと一致する。決定節だけが 2 項になっている。`last_mode_key_tick_ms == None` のときローリング値へ戻る規則(S2')も決定節に 1 行足すとよい。

### 1-2 [中] round4 で広げた「予測が照合されるまで、どちらの baseline にも保留中の打鍵を入れない」が実装されていない
- ADR 307-311行は、範囲を「予測が照合されるまで」に広げ、「`report_mozc_shm`側の照合も同じ起点で早く済むようにし、U2 を二重に閉じる」と書いている。
- 実装で保持専用の起点を使うのは、保持のタイマーの読み(`mozc_shm_read_for_hold` → `mozc_shm_read_once(false, Some(..))`)だけ。refresh の tick の読み(`ime_refresh.rs:85` `mozc_shm_on_tick` → `mozc_shm_read_once(.., None)`)と、解放後から照合までの通常の読みは、一致する読みにローリング値(`last_operation_tick_ms`)をそのまま使う。
- U2 を閉じているのは `KeyEffectOpenHold::last_held_pred`(予測ごとに 1 回)の 1 つだけ。害は小さい(一致する向きなので、遅れるのは `[key-effect-confirmed]` と `key_effect` の解除だけ)。ただし ADR は「二重に閉じた」と書いているので、実装は 1 つだけで閉じたと訂正するか、残作業にすること。

### 1-3 [中] T3(フォーカス変更時の扱い)は「反映済み」と書かれているが、実装は利用者に見えない
- ADR 337-343行(T3)の選択肢は「捨てる+利用者に知らせる」と「元の窓へ戻ったら流す」の 2 つ。決定節 442行は「T3(フォーカス変更時の無断消失回避)を反映済み」と書いている。
- 実装(`runtime/key_effect_open_hold.rs::key_effect_open_hold_finish` の `Released::Discard`)は `tracing::warn!` を出して捨てるだけ。利用者は awase.log を見ないので、R6(ii) で却下した「無断で文字が消える」(BUG-074 型)から実質的に抜け出せていない。「ログのみで、利用者への通知は未実装」と書き直し、残作業に入れること。
- T3 は「上限に達したときの belief も合わせて決める」とも言っていた(342-343行)。実装は `Cap` で belief を変えずに再生し(`needs_notice()` は偽)、この決定が ADR のどこにも書かれていない(3-3 も参照)。

### 1-4 [低] 新設定数の数え違い
- ADR 455-457行: 「新設タイミング定数2つ(`POLL_MS=20`・`ARM_TTL_MS=700`)は`pending=true`」。
- `tuning.rs` の差分では 3 つ新設され、`KEY_EFFECT_OPEN_HOLD_MAX_MS = 300` も `#[measured_macro::measured(pending = true)]`。300ms の根拠(「0.3 秒程度」の利用者発言 2026-10-19)も ADR に無い。

### 1-5 [低] round の番号の誤り
- 439行「round4 T1反映」→ T1 は round3(58-62行・295-296行で round3 と定義)。
- 458-459行「round4 T6の懸念」→ T6 は round3(412行「round3の補足(T6、小)」)。
- 殻 `runtime/key_effect_open_hold.rs` のコメントは「round3 T1」で正しい。squash commit の本文(2つ目の commit)は「round4 T1」で誤っており、ADR はこちらに引きずられている。

### 1-6 [低] 決定節の P の解放理由が網羅されていない
- 440-442行は「開いた/閉で確定/読めなくなった」だけ。`ReleaseReason` には `PredictionSettled`・`Deadline`・`Cap`・`FocusChanged`・`PanicReset`・`ConfigReload` もある。武装の期限(TTL 700ms、期限を過ぎたら P なしに戻る)と、自動リピートでも保持を始めること(コードレビュー指摘 1)も書かれていない。

### 1-7 [中] S1(打刻の対象)の洗い出しの結果が ADR に記録されていない。漏れもある
- ADR 257-264行は、洗い出しを「検証項目ではなく設計の必須要素」としていた(drift correction の再送・ADR-245・ADR-255 Space 置換の「不要の根拠」)。
- 実装の打刻は `ime::send_ime_mode_key` と `send_ime_mode_key_with_shift_release_prefix` の 2 か所。この 2 つを経由する `ime_controller.rs:115,164`(GjiDirect・MsImeDirect)と `output/mod.rs:545`(ADR-245 のトグル)は含まれる。ImmCross は既存の `open_chain.rs:236` `note_imm_cross_write`。
- 漏れの候補(origin/develop で確認):
  - `runtime/key_pipeline.rs:2936` shift-conv-guard が `VK_DBE_HIRAGANA` を `send_input_safe` で直接注入している。打刻が無い。条件は `effective_open()` が真のときなので、BUG-233 のように belief が偽の開を信じている場面でこそ走りうる。しかも閉じた Mozc をひらがなキーで開きうる。
  - `imm.rs::send_ime_control_raw` を通る、`open_chain` 以外の WM_IME_CONTROL 書き込みが無いか。未確認。
- ADR-255(Space 置換)を「打刻は不要」とする根拠も書かれていない。

### 1-8 [中] defer/replay ファミリーへの登録(S5)が未実施。pre-push の正規表現にも穴
- ADR 362-364行は「`fix-requires-evidence.md`の『defer/replay キューの解放条件』ファミリーに入る」と書いている。
- origin/develop の `.claude/rules/fix-requires-evidence.md` で `key_effect_open_hold` を数えると 0 件(`git show origin/develop:.claude/rules/fix-requires-evidence.md | grep -c key_effect_open_hold`)。
- `.githooks/pre-push` の正規表現(55行)は `runtime/(resume_hold|focus_switch_hold|…)` を列挙しているが `runtime/key_effect_open_hold.rs` が無い。純粋側は、たまたま `state/key_effect_` の接頭辞に一致して拾われている。BUG-116 と同じ型の穴(判断の殻が自動チェックの外にある)。
- S5 が列挙した解放窓口のうち「Engine 非活性化」(363行)は配線されていない。無効アプリは、次のタイマー(20ms 以内)で `read=None` → `Unreadable` として拾われる。それ以外の非活性化(トレイからのオフ等)の扱いは ADR に記載が無い。

### 1-9 [中] PR #936 のタイトル・本文・squash commit が「G3 だけ」のまま
- PR 本文の「残り」は「P案…が必要(別PRで提出予定)」と書いている。merge commit 6439bc7d2 の件名も G3 だけ。実際の差分には P(19 ファイルの大半)が入っている。
- 将来 `git log --grep "ADR-292 P"` や件名で探すと見落とす。agent-handoff.md 3 項(コミットを足したらタイトル・本文を見直す)に反する。マージ済みでもう直せないので、ADR・BUG-233 に「6439bc7d2 が G3 と P の両方を含む」と明記すること。

---

## 2. status 行と、文書全体の古い・不正確な記述

| 場所 | 現在の記述 | 事実 |
| --- | --- | --- |
| ADR 68-73行(status) | 「PR #936として提出(CI確認中)」「developへのマージは未着手」 | 2026-11-12T14:15:43Z にマージ済み(6439bc7d2)。未着手なのはこのドキュメントブランチのマージ |
| ADR 64-65行(summary 末尾) | 「まだ修正方針は…確証取得前のため未確定」 | 17-18行・426行の「収束」・実装済みと矛盾する |
| ADR 96行 | 「round1・round2のOpus敵対的レビューで…訂正された」 | round4 まである |
| ADR 221-231行の表 | 見出しは「(round2反映)」、G3 は「有力」、P は「有力・新」 | どちらも採用・実装済み |
| ADR 341行 | T3 は「本ADRでは未決定、実装PRで決める」 | 実装で「捨てる+warn」に決めた(1-3) |
| ADR 377-422行 | R6 は「決定の前提として未解決」「どの選択肢を採るかは未決定」 | 決定節 470-471行で (iv) を既定として実装済み。「(iv) を既定で実装済み、(i) は原因特定待ち」と書き換える |
| ADR 492行 CI 項目 5 | 「Pを採る場合」 | 採った |
| ADR 498-501行 CI 項目 6 | 先に固定するもの | `decide_use` の G3 の表(`bug233_disagree_baseline_…`)、P の判定(`judge_table`・`hold_baseline_ignores_held_keystrokes`)は済み。「`resolve_open_at` で予測を照合した後に観測が勝つ」テストは PR #936 に無い。済み/未を分けて書く |
| ADR 502行 CI 項目 7 | 「R6 のどれを採るかは 1 を待って判断」 | 既定は (iv) と決まった。残るのは (i) を足すかどうか |
| docs ブランチの `docs/adr/index.md` 298行 | 「CI 確証・developへのマージは未着手」 | コードはマージ済み |
| develop の `docs/adr/index.md` の ADR-291 行 | 「PR #934、未マージ」 | #934・#935 とも develop にマージ済み(`git log origin/develop` で 6b29383d8・97d849bfe) |
| BUG-233.md 5行 | `fix_commits: []` | `["6439bc7d2"]` |
| BUG-233.md 状態行 | 「CI確認中」「developへのマージは未着手」 | マージ済み |
| **BUG-233.md 原因 2** | 「ADR-291 D5への帰属は未確認(D5以前から存在する…性質の可能性が高い)」 | **ADR D1b(25-35行・181-197行)は CI run 43220508025 で D5 への帰属を確定と書いており、結論が逆になっている** |
| BUG-233.md 未解決節 | 「どれを選ぶかが未決定」 | (iv) を既定で実装済み |
| BUG-233.md 全体 | 49 行 | fix-requires-evidence の目安は 30 行。要約すること |

さらに、develop が壊れたリンクを抱えている:
- `git ls-tree origin/develop docs/adr/` に 292 は無く、`docs/known-bugs/` に BUG-233 も無い。ところがマージ済みのコード(`awase-hook-core/src/state/key_effect_open_hold.rs` 23行、`awase-ime-core/src/state/mozc_shm.rs` の `decide_use` の doc、テストの doc)と PR 本文(`blob/develop/docs/...`)はこの 2 ファイルを指している。ドキュメントブランチをマージするまで、これらのリンクは 404 になる。
- ドキュメントブランチは origin/develop より 3 コミット遅れている(6b29383d8・97d849bfe・6439bc7d2)。rebase が要り、`docs/adr/index.md` の 291/292 行の周辺で衝突しやすい。

---

## 3. P の未解決ギャップ(期限に達しても Held/Split のまま)の記述の正確さ

ADR 451-454行の原則(観測の裏付けなしに belief を閉にする入口が無い。`ime-belief-architecture.md` に照らして正しい)は正確。ただし、いつ起きるかと影響の範囲の記述が不正確。

### 3-1 [中] 「settle に届かないまま期限」は、構造上ほぼ起きない。実際に期限に達する条件を書くべき
- 閉の読み(食い違い)の起点は `max(last_mode_key_tick_ms(=予測時刻), 送信打刻)` で、保持は予測より後に始まる。したがって閉の読みは、保持の開始から遅くとも `max(0, 170 − (開始 − 予測)) + 20ms` 以内に `Use` になる。最悪でも約 190ms で、300ms より短い。一致する開の読みも、`hold_baseline` から 170ms で `read_is_after_baseline` の settle の側を通る。
- つまり `Deadline` になるのは、次のどれかに限られる:
  - (1) `Split`(`tsf_open != effective_open`)が続く窓。
  - (2) 保持の間に食い違いの起点が前へ押される場合。awase 自身の送信(drift correction の GjiDirect・ImmCross、ADR-245 のトグル復元、ADR-263 の書き込み)は `ime_actuation_send_tick_ms`/`imm_cross_write_tick_ms` を進める。`UserImeSetIntent`/`ImeApplyRequested`/`ModeKeyPassedThrough` は `last_mode_key_tick_ms` を進める(`ime_model.rs` の reduce 冒頭と `ModeKeyPassedThrough` の分岐)。
- この 2 つを「既知の限界が起きる条件」として列挙すれば、限界が測定・テストできる形になる。(2) は純粋なテスト(`decide_use` の食い違いの起点が保持中に進む表)で固定できる。

### 3-2 [低] 「P導入前と同じ挙動、悪化はしない」は出力についてだけ正しい
- 出力は P 導入前と同じ(予測の開のまま再生するので、閉なら同じリテラルになる)。ただし (a) 保持した打鍵は最大 300ms 遅れて出る。(b) U2 により、同じ予測では掛け直さないので、後続の打鍵は P なしの経路を通る。(c) 保持の間は `TIMER_MOZC_SHM` を止めており、解放時に張り直すので、通常の訂正は解放から最大 500ms 後になる。
- 「出力は悪化しないが、遅延は最大 300ms 増え、通常の訂正の時期も早くならない」と書くのが正確。(1) の Split が続く窓では、モードキーを押すたびに 300ms の遅延と `Deadline` の warn が出る。

### 3-3 [中] Cap による解放も、同じ「belief を変えずに再生」の限界なのに、未解決の一覧に無い
- `ReleaseReason::Cap` は `needs_notice()` が偽で、ログは info だけ。予測の開のまま再生するので、閉なら BUG-233 と同じリテラルになる。
- 保持の 8 件には、保持の中の Down に対応する Up も数える(`on_key` が Up も `items.push` する)。実効は約 4 打鍵。3-1 の最悪約 190ms で 4 打鍵(約 21 打鍵/秒)に達するのは稀だが、Deadline と並べて記述すべき。warn を出さない理由も書くこと。

---

## 4. 残作業の記述の網羅性

書かれているもの:
- R6(i) が未実装である理由(460行)。
- CI 項目 1・3・4・5(464行)。BUG-232 の回帰は 484行、遅延の測定は 495-496行、D1b の位相の直接確認は項目 3。

漏れ・誤り:
- **[中] モードキーが Mozc を開けなかった原因の仮説に、既知の cold の最初のキーの問題が無い。** タイムラインでは、フォーカスの変化(20.193)から 478ms 後の VK_CONVERT(20.671)が、その窓への最初のキーと読める。これは BUG-225/226・ADR-284/285 の「Mozc の cold で最初の 1 キーが TIP に届かない/素通りする」と同じ類型。CI 項目 1(475-477行)と S9 のログ確認(`key_seq`)に、この仮説を明示すべき。
  - この仮説が当たっていれば、R6(i) の「機構」のリスク(389-391行: awase の注入も同じ窓に届かない可能性が高い)は外れる。2 キー目にあたる awase の GjiDirect は届く見込みが高く、(i) を判断する前提が変わる。
- **[中] 実機確認の項目が無い。** status は「実機確認…未着手」の一語だけ。dragonflyg4 がオフラインであることにも触れていない。見るべきログ(`[key-effect-open-hold] 武装`、`保持の解放: reason=… hold_ms=…`、warn の `モードキーの予測(開)を mozc-shm で確認できなかった`)と、確認する場面(Zed での再現、Chrome/Edge の正常系の遅延)を残作業として書くこと。
- **[中] 「developへのマージ」の主語が曖昧。** コード(#936)はマージ済み。未マージなのはこのドキュメントブランチ。2 つを分けて書くこと(2 節を参照)。
- [中] 1-8(fix-requires-evidence・pre-push への登録)、1-7(S1 の残りの洗い出し)、1-3(T3 の利用者への通知)が残作業の一覧に無い。
- [低] U4(R6(i) を実装したら、保持中の「開」を予測するモードキーを消費する規則、369-373行)は (i) と一緒に実装する必要がある。(i) の残作業に紐づけて書くこと。
- [低] 定数の実測(1-4)は 3 つ。CI 項目 5 にも、測る対象として `KEY_EFFECT_OPEN_HOLD_MAX_MS` と `ARM_TTL` の分布を書くこと。

---

## 5. その他の論点(G3×P の相互作用・他の ADR との整合)

### 5-1 [高] P は「Mozc が 170ms より遅れて開く」場面を、正しい出力から誤った出力に変える
- G3 の食い違いの起点は、予測(モードキーの物理押下)の時刻。保持の読みは、その 170ms 後に閉の読みを `Use` にし、`ObservedClosed` にする。すると belief を閉に直し、保持した打鍵を qwerty の素通しとして**再生してしまう**(取り消せない)。
- その後に Mozc が開くと、次の読みで belief は開に戻る。しかし再生済みの打鍵は、qwerty の生の文字として開いた Mozc に入り、NICOLA ではない誤ったかなになる。
- P の導入前は、同じタイミングでもローマ字が belief=開で送られ、Mozc が開いた後なら正しく入っていた。
- `KEY_EFFECT_SETTLE_MS` の「実測最大 131ms」は、どの IME のどの状態(warm/cold)で測ったかを ADR で確認していない。フォーカス直後の cold Mozc は BUG-225/226 の文脈で遅延が知られており、まさに BUG-233 と同じ場面。
- G3 単独では、ローリング値でもモードキー自身のフックの時刻が最小の起点だったので、悪化しない。悪化させるのは P の (iv) の再生。
- CI 項目 4 の 3 つ目(488-489行)は「belief が一時的に閉へ倒れないこと」としか書いていない。「倒れると保持した打鍵が取り消せない形で誤入力になる」ことと、R6(iv) の「閉と確定」の妥当性は「cold Mozc のモードキー→`tsf_open` の遅延の分布」次第であることを、明記すべき。

### 5-2 [中] G3 は予測の場面に限らず、ADR-268 D3 R3'/ADR-291 D5 の意味を窓全体で変える。関連 ADR に追記が無い
- `last_mode_key_tick_ms` は `FocusChanged` まで残る(数分前のモードキーでも Some)。そのため G3 の後は、その窓のどの食い違う読みも、最後のモード系の入力から 170ms 経っていれば即座に `Use` になる。D5 が持っていた「最後の打鍵から settle」の保護は、モード系以外のすべての原因について外れる。
- 意図どおりかもしれないが、影響範囲として ADR に明記すべき。ADR-268 と ADR-291 の本文には「292」への言及が 0 件(`git show origin/develop:docs/adr/{268,291}-*.md | grep -c 292`)。`decide_use` の doc を正とする 2 つの ADR に、「食い違いの起点は ADR-292 G3 で変更」と 1 行足すこと。
- 打刻されない注入(1-7 の shift-conv-guard)が残ると、まさにこの保護が外れた穴になる。

### 5-3 [中] 調査範囲の表の `HwndCacheRestored`(125行)は G3 で直らない
- `HwndCacheRestored` は窓の切替に伴うので、`FocusChanged` で `last_mode_key_tick_ms = None` になり、S2' によってローリング値へ戻る。したがって D1b と同じ遅延が残る。この行に「G3 の対象外(S2' によりローリング値のまま)」と書くこと。

### 5-4 [中] 保持の間の drift correction との相互作用が検討されていない
- 保持の間、belief は予測の開、`desired_open` も開に近い状態にある。この間に drift correction が Mozc へ開の書き込み(GjiDirect/ImmCross)を出すと、次の 2 つが起きうる。
  - (a) `ime_actuation_send_tick_ms` が進み、食い違いの起点が押されて `Deadline` に達する(3-1 の (2))。
  - (b) 事実上の R6(i)(開き直し)が、R6(i) で検討した 4 点(冪等性・ADR-119 の合流点・IntentStore の順序)の外で起きる。
- 保持の間に drift correction が発火しうるかを確認し(未確認)、発火するなら ADR に書くこと。

### 5-5 [低] ほかの保持との順序の記述
- `hold_on_delivery` の順序(窓切替直後 → 戻り待ち → P)は殻の doc にある。ADR-263 の入力言語判定の保持との順序は、ADR にもコードの doc にも無い。

### 5-6 [低] 関連 ADR との矛盾は見つからなかった(確認した点)
- ADR-191 決定3: 保持の読みが 170ms 未満で開を `Use` にしても、`reconcile_key_effect_open` の柵が予測を残す。U2(`last_held_pred`)で整合している。
- ADR-268: belief への入口は `report_mozc_shm` の 1 か所のまま。`architecture_guard::key_effect_open_hold_wiring_adr292_p` が殻に `report_mozc_shm(` を書くことを禁じている。
- ADR-270/285/288: `judge_compose` と `should_check_compose` は変更されていない。
- ADR-291 D5: 一致する読みの高速路は変わらない。食い違いの起点だけが変わる(5-2)。

### 5-7 [低] 打刻は送信に失敗しても行われる
- `note_ime_actuation_send` は `sent != inputs.len()` でも打刻する。起点が後ろへずれるので安全側(Held が長くなるだけ)だが、意図ならコメントに 1 行書くとよい。
