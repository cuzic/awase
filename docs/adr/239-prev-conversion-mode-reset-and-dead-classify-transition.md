---
id: ADR-239
title: |-
  分類で `prev_conversion_mode` を読む `input_mode_from_conversion`(`classify_transition`)の撤去 — 約 6 か月、refresh の経路で一度も結果を返していない
summary: |-
  `runtime/focus_tracking.rs::advance_focus_tracking` は IME 読み取りのたびに(フォーカスが変わらなくても)走り、末尾で `prev_conversion_mode` を `None` に戻す。元は ed4e17b6(2026-04-03)で WinEvent のフォーカス通知時だけだったが、翌日の 2f1527aa(ADR-028 debounce-first)がフォーカス処理を `refresh_ime_state_cache` に移したときに毎回の refresh に入った。結果、`classify_ime_snapshot` の `input_mode_from_conversion`(`ConvMode::classify_transition`)は **refresh の tick の中の分類では prev が常に None** で、手元の CI の全経路で `IME input method changed: conv=…` が 906 ログ中 0 件。
  (b) が結果を出せるのは 2 通りだけで、どちらもコードが他の場所で採らないと決めた形(場合 E: 閉じた IME の conv=0 を英数とみなす=BUG-57、場合 K: ROMAN ビットなしの conv を ObservedKana とみなす)。案 A(リセットをフォーカス変更時だけにする)は手元のログの再生で実際に誤分類する(GJI ATOK × Edit で ObservedKana 約 30 件、`sc-hz-msime-native` で閉じた IME の ObservedEisu 2 件)。
  決定(案): B = 分類で prev を読む側(`input_mode_from_conversion`・`classify_transition`・分類の引数・`ImePollState.prev_conv`)だけを撤去する。**`prev_conversion_mode` の書き込みとリセットは残す**(読み手は予測の入力 `conv_raw`〈`key_pipeline.rs:1774`〉で、挙動に効く)。撤去は「ログで一度も起きていない残りの場合(ImmCrossProbe・フォーカス読み失敗の tick)の挙動を、安全な向きに変える」撤去であって、挙動不変ではない。
status: |-
  設計は収束(2026-10-06、Opus round2 で「収束」。round1: Blocker 2・Must 6・Should 5・Nit 5、round2: Should 4・Nit 5 を反映)。実装済み・CI 確認済み(run 37568952498)。
related_adr:
  - "ADR-028"
  - "ADR-238"
  - "ADR-186"
---

# ADR-239: 分類で `prev_conversion_mode` を読む `classify_transition` の撤去

(同じ番号 239 の別 ADR〈parallel-agent-instruction-hygiene〉が別セッションの未マージブランチにあるが、本 ADR は #543 で develop に入った方で、別件。番号の重複は、あちらが改番する。)

## 背景(ADR-238 の実装中に見つかった)

ADR-238(BUG-190)の実装で、`[eisu-adopt]` の `prev_conv` が**全件 `None`** なのに気づき、原因を調べた。

### 由来

- **ed4e17b6**(2026-04-03、`Reset prev_conversion_mode on focus change to prevent false ROMAN transition`): wezterm 0x19 → Zoom 0x09 の偽の romaji→kana 遷移を防ぐため、WinEvent のフォーカス通知(`focus_observer::observe`)で prev をリセットした。**フォーカス変更時だけ**。これが `classify_transition`(の前身=`ime_observer` の ROMAN ビット遷移の判定)が実害(偽の ObservedKana)を出した唯一の記録。
- **2f1527aa**(翌日、ADR-028 debounce-first): フォーカス処理を `refresh_ime_state_cache` に移したとき、リセットが毎回の refresh に入った。e1babb40(`prev_conversion_mode = 0` → `None` の型の置き換え)は由来ではない。**(b) はそれから約 6 か月、refresh の経路で死んでいる。**

### 事実(コードとログで裏取り)

1. `Runtime::apply_focus_probe_result`(`runtime/focus_tracking.rs:77`)は、IME の読み取りのたび(`ir_stage_focus`〈`runtime/ime_refresh.rs:85-91`〉→ `detect_and_update_focus`/`apply_focus_probe_result`)に走り、`advance_focus_tracking`(361 行)が、フォーカスが変わっていなくても毎回、末尾で `set_prev_conversion_mode(None)` を実行する(445 行付近)。ただし `classify_focus_probe` が `None` を返す tick(フォーカスの読みが時間切れ、または `process_id == 0`〈ロック画面・デスクトップ等〉、`focus_tracking.rs:272-278`)は `advance_focus_tracking` を呼ばずに `false` を返し(83-85 行)、リセットされない。
2. 同じ tick の中の順序は `ir_stage_focus`(reset)→ `ir_stage_observe`(`capture_poll_state` → 分類)→ `apply_ime_update`(prev を書く、`platform_state.rs:1293`)。**同じ tick の分類にとって prev は常に `None`**。ただし tick と tick の間の prev は「最後の OsPoll の conv」(`Some`)で、**次の 2 つの場合は分類が `Some` の prev を読む**:
   - **ImmCrossProbe**(`runtime/key_pipeline.rs:2860-2890`): フォーカス変更後の最初のキー(`kp_stage_focus_probe`)で非同期に読み、戻ったところで `ime.belief.prev_conversion_mode()` を読む。tick と tick の間なので prev は**同じ窓の直前の OsPoll の conv(`Some`)**。ログで ImmCrossProbe は 619 回走っている。この経路は `new_prev_conversion_mode` を捨て(prev を書かない)、`InputModeObserved` を **High** confidence で dispatch する。
   - フォーカス読み失敗の tick(上記 1)。
   - (`process_deferred_keys`〈`runtime/mod.rs:1389-1402`〉は本番から到達しない。)
3. **`prev_conversion_mode` は分類以外に、予測の入力として挙動に効く**: `runtime/key_pipeline.rs:1774` `conv_raw: self.platform_state.ime.belief.prev_conversion_mode()` → `state/key_effect_predictor.rs:375` `input.track.conv.or_else(|| input.conv_raw.and_then(Conv::from_raw))`。予測表を引くときの現在の conv の種になり、`KeyEffectPredicted` を通して belief(open/mode)を動かす(`ime_model.rs:860` のコメントも、変換モードの追跡は観測へ戻す、とこの使い方を前提にしている)。
4. 裏取り(手元の CI ログ。`awase.log` 906 ファイル、debug の `IME snapshot:` を含むのは 853、行数 76,439。以前の ADR 草稿の「754 run」は数え方が不明で、906 で数え直した): `IME input method changed: conv=…→…`(`input_mode_from_conversion` が結果を返したときだけ出る info ログ。OsPoll・prefetch・ImmCrossProbe のどの経路でも出る)は **0 件**。`[eisu-adopt]` は `branch=a` が 49 件で、`branch=b` は 0 件、`prev_conv` は全件 `None`(OsPoll の経路)。**言えるのは「(b) は手元の CI で一度も結果を返していない」で、「構造上返せない」ではない。**
5. TsfNative の idle-conv-check(`key_pipeline.rs:752`)と focus-conv-check(2810 行)は prev を**書くだけ**で、分類は読まない(TsfNative 窓の OsPoll の読みは `is_tsf_native_window` で `conversion_mode: None`〈`ime.rs:569-577`〉なので、`input_mode_from_conversion` の `curr_conv?` で必ず `None`)。この 2 か所の書き込みを読むのは `conv_raw` だけ。

### (b) が結果を出せる場合(コードから全部数えた)

`classify_ime_snapshot`(`observer/ime_observer.rs:159-199`)で `input_mode_from_conversion` に届く条件: `trust_input_mode`、`!(guard_active && is_romaji.is_none())`、(a) `is_eisu_evidence` が `Some(true)` でない、`input_mode_from_romaji_flag` が `None`(= `is_romaji == None`)、conv が `Some`、prev が `Some`。`read_ime_state_full`(`ime.rs:590-611`)で「conv が Some なのに `is_romaji` が None」になるのは、(i) NATIVE=0(英数の conv)、(ii) NATIVE=1・ROMAN=0 で `detect_kana_for_hwnd` が失敗、の 2 通りだけ(NATIVE=1・ROMAN=1 は必ず `Some(true)` で romaji フラグが先に拾う)。したがって (b) が `Some` を返すのは:

| 場合 | 条件 | (b) の結果 | コードの他の場所の判断 |
| --- | --- | --- | --- |
| E | 英数の conv(0x00/0x08/0x10/0x18)かつ `ime_on == Some(false)`、prev が非英数 | ObservedEisu | `is_eisu_evidence` が BUG-57 として**採らない**と決めた形そのもの |
| K | conv が NATIVE・ROMAN なし(0x09/0x01 等)で直接の読みが失敗、prev に ROMAN、現在が romaji-capable | ObservedKana | OsPoll では `ir_poll_and_learn` の ImmCross 抑制(`ime_refresh.rs:528-536`)、idle では `classify_idle` の `is_roman_reliable=false` が、**ROMAN ビットなしを根拠に ObservedKana にしない**と決めている |

`ObservedRomaji` は (b) から**出せない**。以前の「(b) は (a) と romaji フラグに包含される」は誤り: (b) は romaji フラグが**判断できなかったときだけ**届き、根拠の弱い conv でその隙間を埋めている。前提(`romaji=None conv=Some`)は日常的に起きる(手元の `IME snapshot:` 9398 件: `ime_on=Some(false)` で 0x09 が 7885・0x01 が 1093・0x00 が 65・0x10 が 29、`ime_on=Some(true)` で 0x09 が 200・0x10 が 104・0x00 が 6)ので、**prev が生きた途端に (b) が動く**。ImmCrossProbe は ObservedKana の抑制(`ir_poll_and_learn` の ImmCross 抑制)を持たず High で dispatch するので、場合 K が ImmCrossProbe で起きると OsPoll より強く belief を動かす。

## 決めること

分類で `prev_conversion_mode` を読む `input_mode_from_conversion`/`classify_transition` を、どうするか。

## 案

| 案 | 内容 | 評価 |
| --- | --- | --- |
| A | リセットを**フォーカスが実際に変わったときだけ**にして、(b) を意図どおり生かす | **誤分類する。** 手元の CI ログを順に再生して近似(フォーカス変更らしい行で prev を捨て、conv=None の読みでは上書きしない。`adr239tools/sim.py`、近似)すると、`cal-driftrec-refocus-edit-gji-atok-{1,2,3}` で 0x19 → 0x09(`romaji=None`)の ObservedKana 候補が各 10 件(GJI ATOK × Edit、ローマ字入力中に belief がかなに落ちる)、`sc-hz-msime-native-3` で `ime_on=Some(false)` の ObservedEisu が 2 件(ADR-238 のフィルタは `ime_on=None` しか弾かないので候補になり、次の読みも閉じた conv=0 なら確定する=**BUG-57 の再発が具体的に再現**)。さらに `conv_raw` の寿命が変わる(下記「`conv_raw` の寿命」) |
| **B(推奨)** | **分類で prev を読む側だけを撤去する。書き込みとリセットは残す。** | 下記「B の範囲」 |
| C | そのままにして記録のみ | 「死んだコードを残す」ではなく、**ImmCrossProbe で High confidence の ObservedKana/閉じた IME の ObservedEisu を出しうる経路を残す**ことになる。採らない |
| 別案: (b) の英数遷移だけ `ime_on` を見て生かす | 場合 E は塞げるが場合 K が残る。K を塞ぐと (b) が出せる結果が無くなる=撤去と同じ。採る意味が無い | |
| 別案: リセットだけ直して (b) は撤去 | (b) 撤去後のリセットの意味は `conv_raw` の寿命だけなので、予測の側から別 ADR で判断する。本件に混ぜない | |

## B の範囲

**消す:**
- `observer/ime_observer.rs` の `input_mode_from_conversion`(115-133 行)と、`classify_ime_snapshot` の 2 本目の `or_else`(176-178 行)。
- `src/engine/conv.rs` の `ConvMode::classify_transition`(135-163 行)と、そのテスト(8 件)。ルート crate の `pub` だが、ワークスペース内の呼び出しは `ime_observer.rs:122` だけ(`classify_idle`・`state/conv_classify.rs::classify_conv_transition` は別関数で残す)。
- `classify_ime_snapshot`/`poll_and_classify_ime`/`classify_fetched_snapshot` の `current_prev_conversion_mode` 引数(呼び出し元 4 か所: `ime_refresh.rs`、`key_pipeline.rs:2874`、`runtime/mod.rs:1402`)と `ImePollState.prev_conv`(読むのは `ir_poll_and_learn` だけ)。
- `log_eisu_diagnostics` の `branch=` と `prev_conv=`(撤去後は `classify_ime_snapshot` の中では (a) 以外に ObservedEisu を出す箇所が無いので `branch` は常に a。ほかの ObservedEisu の出どころ〈idle-conv-check の `classify_idle`、予測 `KeyEffectPredicted`、GJI の観測など〉はある)。経路(OsPoll/prefetch/ImmCrossProbe)を出すには引数を 1 つ足すことになる(4 か所の呼び出し元)ので、**B では `branch=`・`prev_conv=` を消すだけにし、経路の項目は足さない**(必要になったら ADR-238 の診断の続きとして別 PR)。

**残す(消すと予測の `conv_raw` が壊れる):**
- `belief.prev_conversion_mode`、`set_prev_conversion_mode`、`ImeUpdate.new_prev_conversion_mode`、`apply_ime_update` の書き込み(`platform_state.rs:1293`)、`apply_panic_reset` のリセット(1213)、`advance_focus_tracking` のリセット(445)、idle-conv-check/focus-conv-check の書き込み(752、2810)。**読み手は `conv_raw`(1774)だけになる**。
- `classify_ime_snapshot` の `trust_input_mode=false` で `new_prev_conversion_mode` を `None` にする処理。理由のコメントを「awase 自身の UI の conv を `conv_raw` に入れない」に書き換える。
- 名前を `last_observed_conv` 等に変えるかは任意(差分が膨らむので別 PR)。コメント(752 の「次回 input_mode_from_conversion が使えるように」、2759 付近、`belief.rs:1`・`:26`、`platform_state.rs:33,37`、`harness.rs:201`)は「予測の `conv_raw` 用の、直近に観測した conv」に揃える。

**純粋関数の境界:** `trust_input_mode` は純粋関数の引数に入れず呼び出し元に残す(false なら関数を呼ばずに `None`。`new_prev_conversion_mode` と ADR-238 のフィルタの扱いも trust に依存するので、外側にある方が今の構造と同じ)。順序は else-if の鎖そのまま(guard が英数の証拠より先、英数の証拠が真なら現在が既に ObservedEisu でも `None` で終わり romaji フラグ・stale 回復へ落ちない)。ADR-238 のフィルタ(`filter_eisu_adoption`)は純粋関数の結果に対して今と同じ位置で掛ける(関数の中に入れない)。テストは (i)〜(iv) に (v) guard が英数の証拠より先、(vi) 英数の証拠が真で current が既に ObservedEisu なら `None`、`romaji=Some(false)`・conv=0x09・current=ObservedEisu → ObservedKana の境界、`ime_on` 3 値 × `is_romaji` 3 値 × conv 5 値 × current 5 値 × guard 2 値の表テスト(元の else-if の鎖を独立に書き下した期待値との突き合わせ)を足した。

**挙動:** 撤去前後で `classify_ime_snapshot` の結果が違うのは「`input_mode_from_conversion` が `Some` を返していた入力」だけ。それは場合 E・K に限られ、refresh の経路では「フォーカス読み失敗の tick」のとき、ImmCrossProbe では毎回ありうる。**ログでは一度も起きていないが、構造上は起きうる**ので、「挙動不変」ではなく「**起きていない残りの場合の挙動を、安全な向きに変える**」と書く。撤去後、場合 E・K は 3 本目の `or_else`(ObservedEisu の stale 回復)へ落ちる: E は `current` が ObservedEisu 以外なら回復が不発で `None`(belief 維持)、K は `current` が romaji-capable なら不発で `None`(`current == ObservedEisu` のときは撤去前も (b) が `None`〈`current.is_romaji_capable() == self.romaji` で弾く〉なので回復が走り、前後で同じ)。ADR-238 のフィルタは、撤去前に E が作っていた候補(`ime_on=Some(false)` の ObservedEisu)が作られなくなる以外は同じ。ADR-238 の `filter_eisu_adoption` の `current_mode != ObservedEisu` 条件は `classify_ime_snapshot` からは到達しない枝になる(残してよい、害は無い)。

## `conv_raw` の寿命(別件、本件では変えない)

B の後、`advance_focus_tracking` のリセットが効くのは `conv_raw` だけになる。現状: 打鍵中(`ImeReadStrategy::SkipTyping`)は `ir_stage_focus` がリセットするが `ir_poll_and_learn` を呼ばないので、**打鍵が続く間は `conv_raw` が `None`**(予測は `track.conv` か既定値 C19/C10 で引く)。読みの conv が None(時間切れ、TsfNative)、または awase 自身の UI の読み(`trust_input_mode=false`)の後も `None`。これが意図どおりかは ADR-195/199/211 側(予測)の判断で、B の PR ではリセットに触らない(触ると予測の挙動が変わる)。将来 A を再検討するときは「(b) を生かす」ではなく「`conv_raw` を打鍵中も保つか」の問題になる。

## 回帰テスト(fix-requires-evidence、IME belief ファミリー)

- **判定を `state/` の純粋関数へ移す:** `new_input_mode` を決める枝(guard/(a)/romaji フラグ/ObservedEisu の stale 回復)を、cfg の無い `state/`(例: `state/snapshot_input_mode.rs`。引数は `ime_on`・`is_romaji`・`conv`・`current`・`guard_active`)の純粋関数へ移し、`classify_ime_snapshot` はそれを呼ぶだけにする(`observer/` は `#[cfg(windows)]` で Linux のテストが存在しないため)。Linux で走るテスト: (i) 場合 E: `ime_on=Some(false)`・`romaji=None`・conv=0・current=ObservedRomaji → `None`(BUG-57 が「prev が None だから」でなく構造で守られる)、(ii) 場合 K: conv=0x09・`romaji=None`・current=ObservedRomaji → `None`、(iii) stale 回復: conv=0x09・`romaji=None`・current=ObservedEisu → AssumedRomaji、(iv) `guard_active && romaji=None` → `None`。
- **`tests/architecture_guard.rs` のソース走査:** (a) 本番コードに `classify_transition(`/`input_mode_from_conversion` が無い、(b) `.prev_conversion_mode()` を読む本番コードは全ファイルで `runtime/key_pipeline.rs` の `conv_raw:` の 1 件だけ(`belief.rs` の定義と `platform_state.rs` の代入は数えない)、(c) `observer/ime_observer.rs` に `current_prev_conversion_mode`/`prev_conv` が無い(prev を**書く**側の `new_prev_conversion_mode` は残す)。分類に prev を戻す変更を、A を知らずに足すことを防ぐ(`classification_does_not_read_prev_conversion_mode`)。
- **journal リプレイは使えない:** `classify_ime_snapshot` の入力列(`ImeSnapshot`)は journal に記録されていない(記録されるのは `classify_conv_transition` の呼び出し、`journal.rs:301`)。閉ループのハーネス(`tests/support/harness.rs`)も `classify_ime_snapshot` を通らない(`last_conv_raw` を自前で持つ)。

## 実装前の測定

追加の診断ログは要らない: (1) `IME input method changed: conv=` は全経路で出る info ログで 0 件、(2) (b) が出せる結果は場合 E・K の 2 通りと列挙した、(3) その前提(`romaji=None conv=Some`)の出現数をログで数えた。ImmCrossProbe での prev の値はログに出ていないので「ImmCrossProbe で prev が Some だった回数」は数えられない(事実 2 はコードからの結論)。

## 実装 PR で回す CI(撤去で挙動が変わりうるのは ImmCrossProbe〈Standard、フォーカス変更後の最初のキー〉と場合 E・K)

MS-IME: `msime-native`・`msime-native-henkan`・`sc-dbe-msime-native`・`sc-hz-msime-native`(A の再生で場合 E が出た構成)・`sc-shift-msime-native`・`msime-stale-table`。GJI: `sc-dbe-gji-atok`・`sc-dbe-gji-msime`・`cal-driftrec-refocus-edit-gji-atok`(A の再生で場合 K が出た構成、再フォーカスで ImmCrossProbe も通る)。ImmCross/外部アプリ: `tsx-ext-qt-qlineedit-{msime,gji}-20ms`・`tsx-ext-wf-textbox-msime-20ms`・`tsx-ext-wx-field-msime-20ms`。合格基準: (1) 各構成の PASS/FAIL が、撤去前の develop の同じ構成(同じ回数、例えば各 3 回)と同じ。(2) `[eisu-adopt]` の件数と `decision` の分布が同程度(`branch`/`prev_conv` の項目は消えるので、比べるのは `decision` と `ime_on`・`conv`)。(3) **変わりうる経路を実際に通ったこと**: 各構成で `[ImmCrossProbe] child-hwnd` が 1 件以上あること(通っていなければ、その構成は B の確認になっていない。特に `cal-driftrec-refocus-edit-gji-atok` と `tsx-ext-qt-qlineedit-*`)。撤去したログ `IME input method changed: conv=` が 0 のままであることは、撤去で行そのものが消えるので基準にならない。

## 実装後の CI(2026-10-07、run 37568952498、13 構成 33 run、撤去前は run 37486784708・37561079357)

- (1) **各構成の PASS/FAIL は撤去前と同じ:** `msime-native`・`msime-native-henkan`・`sc-hz-msime-native` は全 PASS、`sc-dbe-msime-native`・`msime-stale-table`・`sc-dbe-gji-msime`・`sc-dbe-gji-atok`・`sc-shift-msime-native` は各 3/3 OK、`tsx-ext-qt-qlineedit-{msime,gji}-20ms`・`tsx-ext-wf-textbox-msime-20ms`・`tsx-ext-wx-field-msime-20ms` は PASS。`cal-driftrec-refocus-edit-gji-atok` は `rc=1`・`verdict=UNDETERMINED`・`[drift-skip]` 9 回/run・試行 #0 のみ回復、で**撤去前と完全に同じ**(`typed_blind` のため元から判定不能)。
- (2) `[eisu-adopt]` の分布は同じ: `sc-dbe-msime-native` は「候補 6・確定 6」(撤去前も 6/6)、他の構成は 0 件。
- (3) **変わりうる経路(ImmCrossProbe)を通った構成:** `cal-driftrec-refocus-edit-gji-atok`・`msime-stale-table`・`sc-dbe-gji-atok`・`sc-dbe-gji-msime` の各 3/3 run で `[ImmCrossProbe] child-hwnd` が出た。`msime-native*`・`sc-dbe-msime-native`・`sc-hz-msime-native`・`sc-shift-msime-native`・`tsx-ext-*` は ImmCrossProbe を通らない(自前 Edit の MS-IME 構成、または ICP の対象外の窓)ので、B の ImmCrossProbe 経路の確認にはならず、OsPoll の分類の等価性(純粋関数の表テスト)の確認になる。**ImmCrossProbe 経路で場合 E・K が起きる入力は CI の構成には無く、手元のログでも一度も起きていない**(撤去は「起きていない残りの場合を安全な向きに変える」撤去、という位置づけのまま)。

## ADR-238・BUG-190・コード内コメントの訂正(実装 PR で行う)

- ADR-238 の「(b) の `classify_transition` が前回 0x19・今回 0 という孤立した 1 回の形で ObservedEisu を返す。(a) だけを弾いても 5 件とも同じ結果」は誤り(prev が None なので (b) は返していない。5 件は (a) が作った。`[eisu-adopt]` 49 件すべて `branch=a`)。「(a) と (b) の両方」「(b) の件もこのテストで固定」「(b) が `Some(ObservedEisu)` を返し直すので素通し」の記述も、(b) の撤去と合わせて直す。BUG-190 の「作る箇所は 2 つ」も。
- `ime_observer.rs` の stale 回復のコメント(「classify_transition が None を返し」)、テスト `isolated_conv_zero_is_a_candidate_then_confirmed` の doc(「`classify_transition` の英数遷移が拾う形」。このテストは `ime_on=Some(true)` なので (a) を通っている)。

## 状態

起草中。Opus round1(Blocker 2・Must 6・Should 5・Nit 5)反映済み、round2 待ち。
