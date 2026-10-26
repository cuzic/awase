# 05 設計の論点の一覧(結論は出さない)

各論点に、擬似窓・擬似 IME で違いが出る列の案を 1 行つける。語彙は `crates/awase-windows/tests/support/harness.rs` のもの(`key(vk)`・`observe`/`observe_value`・`focus_change`/`focus_to`・`user_set_open`・`external_set_open`・`advance_ms`・`block_writes`・`activation()`)。VK は 0xF2=ひらがな、0x1A=VK_IME_OFF、0xF3/0xF4=半角/全角、0xF0=英数。
「ハーネス拡張が要る」は `01-map.md` §8 の「写していない」ものに当たるとき。論点の根拠は `01-map.md`(地図)・`02-ledger.md`(台帳)・`03-constraints.md`(条件)。

| # | 論点 | 選択肢の例(結論なし) | 擬似窓・擬似 IME で違いが出る列 |
|---|---|---|---|
| Q1 | **ON と OFF の根拠の対称性**。OFF キーは意図(`desired_open`・IntentStore・`persistent_explicit_off` を書く)、ひらがなキー等の ON は予測(`key_effect` だけ)。方向ではなく経路で分かれる(A1)。 | (a)すべてのモードキーを同じ経路にする(意図か予測のどちらかへ統一)。(b)今の 2 経路を残し、窓切替・失効の扱いだけそろえる。(c)意図と予測を 1 つの「最後の明示操作」に統合する | `user_set_open(false)` → `key(0xF2)` → `advance_ms(500)` → `focus_change()` → `activation()`。現状は `Inactive(ImeOff)`(PR #854 のブランチ `origin/fix/focus-desired-fallback-off` の `bug_223_explicit_off_then_f2_prediction_is_lost_on_focus_change` が現状を固定。develop には無い)。窓切替を挟まない同じ列は Active。 |
| Q2 | **`desired_open` の意味**: 利用者の意図か、awase の推定か(X2 の 8 つの読み手)。窓切替で持ち越すか。 | (a)意図専用にし、観測ゼロのときの既定は別の値にする。(b)窓ごとに持つ。(c)窓切替で捨て、`Unknown` にする | `focus_to` で窓 A(読める)→窓 B(読めない・観測なし)へ。A で `user_set_open(false)` → B へ → `activation()`。持ち越しの有無で B の Active/Inactive が変わる。**B の「真の IME」が ON の場合と OFF の場合の両方**を打つ(擬似 IME はスレッドごと)。 |
| Q3 | **意図と観測の優先順位と寿命**: 最後の明示操作が勝つ期間(今: `last_intent` は窓切替まで、IntentStore は ON 10 秒/OFF 30 秒、`key_effect` は観測が来るまで)。観測が上書きする条件。時間で失効するか。 | (a)明示操作は常に強い(失効しない)。(b)観測が一致して N 回(または High)続いたら取り下げる(ADR-279 の案)。(c)時間で失効する(今の IntentStore)。(d)窓切替で必ず失効する | `user_set_open(false)` → `external_set_open(true)`(実 IME だけ ON に戻る)→ `observe_value(ObserverPoll/ImmCrossProbe, true)` を `advance_ms(500)` ごとに 0〜60 秒。Engine が Active に戻るまでの時間。ADR-279 の (a2) 構成の模擬。 |
| Q4 | **窓ごとの状態の持ち方**: 実 IME の状態は窓・スレッドごとか、グローバルか(GJI・MS-IME・Mozc で違う。擬似 IME は `switch_thread` でスレッドごと、初めてのスレッドは閉×ひらがな、ADR-191 §3.2)。belief を hwnd(またはスレッド)ごとに持つか。 | (a)今のグローバル + IntentStore(hwnd)。(b)belief ごと hwnd/スレッドキーにする。(c)観測だけ hwnd キー | 窓 A・B を別スレッドにし、A: `user_set_open(false)`、B: 既定、`focus_to(B)` → `focus_to(A)` → `focus_to(B)` の各時点の `activation()` と擬似 IME の真の開閉の食い違い件数。 |
| Q5 | **観測の寿命と窓切替での扱い**: 窓切替で全消し(今)か、hwnd ごとに保持し、同じ hwnd に戻ったら使うか。 | (a)全消し(今)。(b)hwnd ごとに保持(寿命つき)。(c)保持するが信頼度を下げる | A で `observe(ImmCrossProbe)`(ON)→ `focus_to(B)` → `focus_to(A)`(同じ hwnd)→ 観測なしで `activation()`。**ハーネス拡張が要る**場合: キャッシュ復元は写されていない(§8)。 |
| Q6 | **不確実なときの既定**: 観測も意図も予測も無いとき、ON に倒すか OFF に倒すか。今は混在(A5: Imm32Unavailable のキャッシュ無し = ON、測定済み条件の新スレッド = OFF、`desired_open` 初期値 = ON、TsfNative は前窓の値)。コスト: 誤 ON は生の英字が NICOLA の仮名になる、誤 OFF は NICOLA の打鍵が IME のローマ字入力として処理され違う仮名になる(どちらも誤入力。BUG-223 は後者、BUG-110 は前者)。 | (a)常に ON(利用者が日本語入力を主とする前提)。(b)常に OFF。(c)直前の窓の状態を引き継ぐ。(d)lang-detect の判定(ADR-263/280)に任せる | 観測ゼロの窓 × {直前の belief ON・OFF、初回、新スレッド}で、`activation()` と擬似 IME の真の開閉を比べ、食い違い(誤 ON・誤 OFF)の件数を数える。Engine の `on_input`(打鍵の出力)は写されていないので、打鍵そのものの誤りは数えられない(拡張が要る)。 |
| Q7 | **単独の観測を信じるか**: 単独の Medium(ObserverPoll)・High(ImmCrossProbe)の偽 OFF(BUG-219・223(a))。観測の種別・窓の種類ごとの信用。 | (a)読めない窓の IMM 読みは belief に入れない。(b)直前の予測・意図と矛盾する単独の観測は N 回確認する。(c)今のまま(Medium 合意・High 単独)で TTL・fence を調整する | `key(0xF2)`(ON を予測)→ `observe_value(ObserverPoll, false)` を 1 回 → `activation()`。窓が読める/読めないの両方、`advance_ms` で 170ms 前後・3 秒前後。 |
| Q8 | **明示意図があるとき poll を止めるか**(C-c2)。止めると意図が固定される。 | (a)止める(今)。(b)意図があっても低頻度で読む。(c)読めない窓だけ止める | `user_set_open(false)` → `external_set_open(true)` → `advance_ms` 10 秒 → `prefetch_read` の有無で belief が追随するか(ADR-279 段階 0 の構成の模擬)。poll の起動条件そのものは写されていない(拡張が要る)。 |
| Q9 | **「ひらがな」の定義**(方針 C-a1)。`input_mode` が `Unknown`・`ObservedKana`・`ObservedEisu`・`Startup`・半角英数トグル中のときの扱い。 | (a)今の `is_romaji_capable`(Observed/AssumedRomaji だけ)。(b)`Unknown` と `Startup` は ON 側に倒す。(c)かな入力は Engine ON(NICOLA がかな入力を出力する構成)を許す | `InputModeApplied`/`key(0xF0)`/`key(0xF2)` の全組み合わせ × open{true,false} の `activation()` の全列挙(`pseudo_ime_exhaustive.rs` と同じ流儀)。 |
| Q10 | **半角英数トグルの状態を belief の軸にどう入れるか**(X8: `input_mode` と hub の `toggle_held`・戻り待ちの二重。入りが手動・lang-detect・窓切替で 3 経路)。 | (a)今の二重。(b)`input_mode` の 1 軸に統合。(c)トグルを belief の外(Engine の別の入口)にする | 窓 A で左 Shift 単独タップ(入る)→ `focus_to(B)` → `focus_to(A)` → `key(0xF2)`/英数/打鍵の各順。BUG-186・193・220・222 の列。**ハーネス拡張が要る**(トグル・戻り待ちは写されていない)。 |
| Q11 | **窓切替のキャッシュ復元を残すか**(ON だけ復元 / 双方向 / 廃止)。BUG-128・218 は復元が本物の OFF を上書きした事故、BUG-223 は復元されない(または持ち越しで偽の OFF)事故。 | (a)今(窓種ごとに別)。(b)廃止し、観測と予測だけにする。(c)復元は `Low` の観測としてだけ入れる(`HwndCacheRestored` を意図にしない) | `user_set_open(false)`(窓 A)→ 3.6 秒後に別プロセスの窓 B(BUG-128)、A で OFF → 窓 B(古い ON のキャッシュあり)(BUG-218)。**ハーネス拡張が要る**(`hwnd_cache` を写す)。 |
| Q12 | **Engine 活性の入口を 1 か所にそろえるか**(X7)。`app_disabled`・lang-detect の保持・戻り待ちの保持・`FocusTransition` の settle が `compute_state` の外にある。 | (a)今のまま。(b)`ActivationState` の理由に追加する。(c)保持は Engine の外のまま、理由を journal に統一して残す | lang-detect の保持中に `focus_to`、保持中に `user_set_open`。保持は写されていない(拡張が要る)。 |
| Q13 | **予測の寿命**: 観測が来ない窓で、予測に寿命を持たせるか(今: 消えるのは観測との照合・意図・窓切替。BUG-223: 窓切替で消えて `DesiredFallback` の false に戻る)。 | (a)今。(b)窓切替では消さない(同じ hwnd に戻る場合)。(c)予測を `desired_open` に書く(意図扱い)。(d)時間で消す(根拠となる実測が要る) | `key(0xF2)` → `advance_ms(60_000)` → `activation()`(寿命の有無)、`key(0xF2)` → `focus_change()` → `activation()`。 |
| Q14 | **belief に合わせて IME を書き直す補正を再検討するか**。今は ADR-212 P6 で「明示 OFF の再試行だけ」。force-on・reassert・ActivationSync は撤去済み(取り下げ、`02-ledger.md` §C)。 | (a)書かない(今)。(b)観測ゼロの窓に限って絞って復元する(ADR-262 が例外として復元した前例)。(c)利用者の操作にだけ応答する | `external_set_open(true)` → `block_writes(true/false)` → `advance_ms` で、書き込み命令(`WriteCommand`)の件数と実 IME の最終状態。実機の過去ログでは補正の書き込み 169 件が全て OFF 方向だった(`drift_correction.rs:138`)。 |
| Q15 | **ON と OFF で寿命を変える根拠があるか**(IntentStore の ON 10 秒/OFF 30 秒。どちらも「未実測・暫定値」)。 | (a)今。(b)同じ値にする。(c)失効させない | `user_set_open(v)` → `advance_ms(10_000〜30_000)` の間に窓往復(`focus_to`)して `activation()`。**TTL の値の根拠となる実測は、この調査では得ていない**。 |
| Q16 | **`user_enabled` 以外の「利用者が awase を使わない」指定の扱い**(C-a2 の例外): `disable_apps`・`FocusKind::NonText`・日本語でない IME。 | (a)今(個別に分散)。(b)「Engine を使わない理由」を 1 つの enum にする | 窓モデルに `disabled` を足し、窓切替の前後で `activation()` が必ず `Inactive(理由)`になること。 |

## 所有者に確認が要る前提(設計の入口)

1. 「ひらがな」の範囲(Q9)と、かな入力・`Unknown` の扱い。
2. 方針の例外に、`disable_apps`・`NonText`・日本語でない IME を含めるか(Q16)。
3. 不確実なときに倒す側(Q6)。誤 ON と誤 OFF のどちらのコストを重く見るか。
4. 利用者の明示 OFF(IME OFF キー)を、どのくらい強く・長く尊重するか(Q3・Q8)。「IME ON のとき必ず Engine ON」と、「利用者が OFF キーを押した直後は OFF」の優先。
5. 窓切替をまたぐ状態を持つか(Q2・Q4・Q5・Q11)。

## この調査でできていないこと

- 実機ログ・journal は読んでいない(BUG-223 の 5 区間の直前に明示 OFF があったかは、BUG-223 自身が未確認としている)。
- `open_warrant.rs`(授権)が `desired_open` をどう使うかは読んでいない(X2)。
- `key_pipeline.rs` の Engine の前の入口(lang-detect の保持・半角英数の戻り待ち・settle)の詳細は精査していない(X7)。
- TTL・fence・窓の値の実測は新たに取っていない(台帳の値は元の文書の記述)。
