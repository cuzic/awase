# ADR-245 追補 2(改訂 r2)Opus レビュー round 2

対象: `/home/cuzic/rust-nicola-wt-adr245`、`feat/adr245-pr2-pr3-rebased`、`c6a6ec3c`(`git log -3 --oneline` で確認)、
`docs/adr/245-half-width-alnum-restore-on-return.md` 102〜174 行。
追加で読んだコード: `key_pipeline.rs` 2729-2844(復元タスク)・3246-3318(FocusProbe の ImmCrossProbe)、`runtime/focus_tracking.rs` 806-833(フォーカス時の ImmCrossProbe)、
`runtime/mod.rs::focus_fence`(537-542)、`state/probe_admission.rs::ImmLikeTicket`(97-)、`probe_rejection_stats.rs::admit_epoch_in_app`(101-140)、
`state/platform_state.rs::write_imm_cross_probe`(1997-2010)、`ime.rs::read_ime_state_full_async`(641-645)、`win32-async/src/offload.rs`(82-92、59 行で 1 呼び出し 1 スレッド)、
`awase-windows-core/src/state/observation_store.rs`(287-303、451-474)、`tests/architecture_guard.rs` 640-700。

件数: Blocker 1 / Must 4 / Should 4 / Nit 4

round 1 の対応の確認: B-2・M-1・M-3・M-5・M-6・S-1〜S-6・N-1〜N-5 は、方針として反映されている。M-2 は typed2 の実測で否定されたので撤回する(却下表への追加も妥当)。
B-1 は事実の節に 27/35 が明記され、解放条件の付け替えで前提がなくなった。ただし付け替え先の R1 の置き場所には新しい問題がある(B-1')。M-4 は契機の表に入ったが、行の間に矛盾がある(S-1)。

---

## Blocker

### B-1' R1 を「復元完了」の分岐に置くと、観測は書き込みの 160ms 後になり、期限 100ms より必ず遅い
- 根拠: 追補 2 決定 2(117 行)は、R1 を「`Written` の NATIVE 確認で復元完了を判断した直後(`key_pipeline.rs` の『復元完了』の分岐)」に置く。
  復元タスクの流れは次のとおり。`set_ime_conv_for_target` の結果が `Written` でも何もしない(`key_pipeline.rs:2785-2786`)→ `win32_async::sleep_ms(RETRY_INTERVAL_MS)`(160ms、`:2734,2802`)→ `get_ime_conv_for_target`(`:2810`)→ NATIVE なら「復元完了」(`:2812-2827`)。
  つまり「復元完了」の分岐は、最短でも書き込みの約 160ms 後になる。
- 失敗シナリオ: 決定 6 の導出(書き込み 6.0ms + 読み 0.5ms、典型 ~7ms)は、書き込みの直後に読むことを前提にしている。書いてあるとおりに実装すると、R2 は毎回 `Deadline`(100ms)で解放されて最初の打鍵は `w` のままになり、R1 の観測はその 60ms 以上後に着く。検証節の「strict の MS-IME 構成で `Deadline` 0 件」も必ず落ちる。
- 提案: R1 は 1 回目(`attempt == 0`)の `Written` の直後、`sleep_ms` の前に 1 回だけ読むと明記する。2 回目以降のリトライの `Written` では読まない(読み直しは 1 回で、R2 の完了通知も 1 回にする)。
  「NATIVE 未確認のまま終了」(`:2831-2843`)の分岐は R1 とは無関係になる(R1 は既に済んでいる)。そう書いておけば、team-lead の問い 1 の「4 回のリトライの途中で観測が走る/走らない」も 1 通りに決まる。

---

## Must

### M-1 R1 より前に始まった ImmCrossProbe の読みが R1 より後に着くと、同じ枠を上書きして belief を閉・英数へ戻す
- 根拠: 観測ストアは観測元ごとに 1 枠で、後から着いたものが勝つ(`PerSourceObservations::set`、`observation_store.rs:287-303`。`record_any` も時刻を比べない、`:451-474`)。
  `offload` は 1 回の呼び出しごとに別スレッドを立てる(`win32-async/src/offload.rs:59`)ので、読みの完了の順序は開始の順序と一致しない。
  同じ枠に書くものが R1 のほかに 2 つある。(i) フォーカス時の ImmCrossProbe(`focus_tracking.rs:806-833`、A に戻った時点で起動)、(ii) 最初の打鍵の `kp_stage_focus_probe`(`key_pipeline.rs:110`、半角英数の段 `:112` の**前**)が起動する ImmCrossProbe(`:3255-3317`)。(ii) は読みの結果を `classify_fetched_snapshot` にも渡すので、conv 0x10 を読めば `InputModeObserved{ObservedEisu, High}` も出す。
- 失敗シナリオ: (ii) の読みが書き込みの前に始まり、応答の遅い窓で数十 ms かかる。その間に R1 が書き込み後の「開・0x19」を記録し、R2 が解放される。その後に (ii) が「閉・0x10」を High で上書きする。drain より先に着けば、再生した打鍵も素通しの `w` になる。後に着いても、2 打鍵目以降は NICOLA が止まる(R1 単独の効果が消える)。
  逆の順序(8/35 の「たまたま書き込みの後に着いた」)が実測で起きているので、着く順序が入れ替わるのは仮定ではない。
- 提案: 読みの開始時刻(`read_started`、A-M2 の規律で既に持っている)が「直近の復元の書き込みの起案時刻」より前の ImmCrossProbe は、admission で棄却する。Resume で `note_explicit_ime_action` を打っているので(`key_pipeline.rs:2591`)、その時刻を比べればよい。
  これは belief の reducer に新しい規則を足すことになるので、置き場所(`admit_epoch_in_app` の呼び出し側か、`ImmLikeTicket` に下限の時刻を持たせるか)を決定 2 に書く。少なくとも「(i)(ii) の読みが R1 より後に着く」ケースを単体テスト(ストアの reduce)と、検証節の未確認の項目に加える。

### M-2 Resume の打鍵が `INPUT_DEFER` の drain の中で保持を始めると、同じ batch の後続の打鍵は取り込み口を通らずに追い越す
- 根拠: 決定 3(123 行)は、後続の打鍵を足す場所を取り込み口(`handle_hook_key_event`)にした。しかし Resume の打鍵自体が `INPUT_DEFER` を経由して届くことがある。OUTPUT_GATE 中の `defer_during_output`、`has_pending_drain` での `replay_later`、`with_app` が `None` のとき(`app/mod.rs:711-733`)、TsfGate の再生、ime-off-rescue の再生がそうである。
  その間に来た後続の打鍵も、同じ `INPUT_DEFER` に積まれる。drain は `take_all` で取り出した列を `deliver_key_event(DeferredReplay)` で順に処理するので(`message_handlers.rs:1791,1816-1833`)、列の途中で R2 が保持を始めても、残りの打鍵は取り込み口の判定を通らず、そのまま Engine と reinject へ進む。
- 失敗シナリオ: 戻った直後に OUTPUT_GATE が立っている(awase の出力の途中)と、`w`,`k` が両方 defer される。drain で `w` が Resume → 保持され、`k` は素通しで先に出る。後から `w` が再生されて `kw` の順になる。
- 提案: 「保持中なら足す」判定を、取り込み口に加えて `deliver_key_event` の先頭(origin を問わない)か drain のループにも置く。R2 自身の再生は、保持が終わったあと(保持していない状態)でしか drain に来ないので、二重に保持されることはない。
  もう一つの方法は、drain の途中で保持が始まったら、残りの列をまるごと保持へ移すこと。どちらにするかを決定 3 に書き、architecture_guard の (i) を「保持の判定が取り込み口と deliver_key_event(または drain)の両方にある」に広げる。

### M-3 R2 を必ず解放させる「完了の通知」が、復元タスクの途中で終わる経路を拾えていない
- 根拠: 決定 3 の解放 (a) は「R1 の観測が記録された、または復元が失敗/中断/世代不一致で終わった」。しかし復元タスクと R1 には、通知が出ない終わり方が少なくとも 5 つある。
  - `ActuationTarget::capture` が `None` で `return`(`key_pipeline.rs:2736-2739`)
  - `still_owner` が偽で `return`(`:2770-2776`)
  - 1 回目が `Failed`/`Aborted` のまま 160ms のスリープに入る(`:2787-2802`。ここで終わりではないので「失敗で終わった」とは言えない)
  - R1 の読みで `snap.ime_on` が `None`(FocusProbe の形では何も記録せずに抜ける、`:3270`)
  - `admit_epoch_in_app` での棄却(`probe_rejection_stats.rs:117-138`)と、`with_app` の再入による `None`(`:3271` の `let _ =`)
- 失敗シナリオ: どの経路でも打鍵は失われず `Deadline` で出るが、そのたびに 100ms 遅れる。そのうえ CI の「Deadline 0 件」が、原因の分からない形で揺れる。
- 提案: Resume の段で R2 を予約したとき、復元タスクへ「完了の札」(drop 時に通知する RAII の値)を渡す。R1 が記録したとき・棄却したとき・読めなかったとき、およびタスクのすべての早期 return で、札が 1 回だけ通知されるようにする。
  `with_app` が `None` のときは、`post_to_main_thread_with` で再送する(`with_app_or_repost_with` と同じ流儀)。解放理由の `RestoreEnded` を `CaptureFailed`/`NotOwner`/`FirstWriteFailed`/`ReadNone`/`Rejected` に分けてログに出し、CI のログ断片検査で数える。

### M-4 Resume の判断の時点で、追跡しているフォーカスが A に追いついている保証が無い(fence・プロファイル・`is_japanese_ime` が B のまま採られうる)
- 根拠: 戻り待ちの照合はライブの `win32::foreground_scope()` で行う(決定 2、`half_width_return_entry_for_foreground`)。一方 R1 の ticket は `self.focus_fence()` で、中身は追跡側の `focus_epoch` と `platform.focus.current.hwnd`(`runtime/mod.rs:537-542`)。
  決定 8 の条件 `current_app_profile()`/`belief.is_japanese_ime()` も追跡側の値である。A に戻った最初の打鍵が、フォーカス追跡の tick(`ir_stage_observe`)より先に来ると、両者は食い違う。
- 失敗シナリオ: (1) fence が B のまま採られ、後のフォーカスの tick で epoch が進むと、R1 は棄却される(M-3 の通知が無ければ 100ms 待つ)。
  (2) tick がまだ来ていない間に R1 が着くと、admission は B の fence で通り、`write_imm_cross_probe` は A の読みを **B の hwnd**(`accepted.hwnd()`、`platform_state.rs:2004-2008`)として記録する。その後の `clear_on_focus_change` で消えるので実害は小さいが、宛先を誤る。
  (3) 決定 8 の判定が B のプロファイル(B が TsfNative なら R1 も R2 も無し、B が Standard で A が TsfNative なら観測が着かない予約)で行われる。
- 提案: R2 の予約(および R1 の予約)の前提に、`lang_detect_focus_in_sync()`(ADR-263 がフォーカス追跡と前面窓の一致を見るために既に持っている、`lang_detect_shadow.rs:260`)と同じ条件を入れる。一致しなければ R1 も R2 も予約せず(PR 2 のままの挙動)、その旨をログに出す。決定 8 の判定はこの前提の後で行うと書けば、team-lead の問い 5「判断の時点で確定できるか」は「一致したときだけ確定でき、一致しなければ予約しない」と答えられる。

---

## Should

### S-1 契機の表の行 4(窓の変化)と行 6(保持の外の KeyUp)が食い違っている。キーの同一性は scan で書く
- 根拠: 行 6 は「保持の外で Down が OS に届いたキーの KeyUp は保持に足さず、通常どおり処理」とする。それなら、そうした KeyUp は保持に入らない。行 4 の「Down が保持の外にある KeyUp は再生する」は、存在しない打鍵を扱っていることになる。
  実装では「この KeyUp の Down は保持の中にあるか」を取り込み口で判定する必要がある。`VK_DBE_*` は Down と Up で vk が変わり、左右の Alt は raw の scan が同じで拡張ビットだけが違う(fix-requires-evidence の物理キー押下ラッチの行。`vk::thumb_latch_identity`)。
- 提案: 行 6 を正とし(保持の外の KeyUp は保持に入れない)、行 4 は「保持の中の打鍵はすべて捨てる。保持の中にある Down に対応する Up も、後から来たら保持がなくても捨てる/通す」のどちらかに書き直す。後から来た Up を通しても、OS から見れば Down の無い Up で無害なので、通すほうが簡単。
  同一性は `thumb_latch_identity(scan, extended)` で取り、vk では判定しないと書く。リピート(`was_down`)の Down は、同じ同一性の Down が保持の中にあれば保持に足し、無ければ(B で押されたままのキー)行 6 と同じく通す、と 1 行足す。

### S-2 ImmCrossProbe の記録の括り出しは、既存の 2 か所のどちらを共有するかと、ガードの件数を書く
- 根拠: ImmCrossProbe の記録は 2 か所にあり、中身が違う。`focus_tracking.rs:806-833` は開閉だけを書き、`key_pipeline.rs:3263-3317` は開閉に加えて `classify_fetched_snapshot` → `InputModeObserved` も出す。
  追補 2 の「フォーカス時の ImmCrossProbe がたまたま書き込みの後に着いた」の 8 件は、前者の経路である。R1 には input_mode の更新が要るので、後者を共有するのが正しい。
  `input_mode_observed_construction_sites_are_accounted_for` は `ImeEvent::InputModeObserved {` をファイルごとに数えている(`architecture_guard.rs:680-700`、`key_pipeline.rs` = 2)。メソッドを別のファイルへ移すと件数が変わる。
- 提案: 括り出すのは `key_pipeline.rs` の側だけで、同じファイルに置く(件数 2 は変わらない)と明記する。`focus_tracking.rs` の側は変えない(挙動が違うので統合しない)。
  括り出しの前後で、`write_imm_cross_probe` → `effective_open()` を読む → `classify_fetched_snapshot` の順序が変わらないことを、レビュー点として残す。

### S-3 通常の Exit(`FromToggle`)に R1 が要らない理由を 1 行書く
- 根拠: 決定 1 は R1 を `FromResume` だけに限る。MS-IME 本体の半角英数トグル中の A は閉と読まれる(決定 5 の行 3 の注記)。それなら、同じ窓での左 Shift による Exit(`FromToggle`)でも、belief が閉のまま残ってもおかしくない。
  実際にはそうならない理由は、離脱が無いので観測がクリアされず、トグルに入る前の open の観測が残っているから、という推測になる。
- 提案: 既存の CI(トグルの入出)の結果か、`[effective-open-flip]` のログで、`FromToggle` では belief が開のまま保たれることを 1 行の根拠として書く。根拠が無ければ、`FromToggle` も同じ構成で typed2 を見る構成を observe で足す。

### S-4 R1 の記録は ImmCrossProbe/High のままでよいが、ログと journal では起点を区別する
- 根拠: 新しい観測元を作らないのは規律どおりで妥当(ime-belief-architecture の Observe → classify → reduce を外れない。`InputModeObserved` は実際の読みに基づくので、`InputModeApplied` にすべき偽装にも当たらない)。
  ただし、journal の `ObserverReported` だけを見ると、FocusProbe 起点の読みと R1(書き込み後の読み直し)を区別できない。M-1 の調査では、両者を分けて読めることが前提になる。
- 提案: ログのタグを `[ImmCrossProbe/resume]` に分ける。journal の値は変えなくてよい(variant を足さない)。

---

## Nit

### N-1 決定 6 の 100ms の性格を書き分ける
- 「典型 ~7ms」は測定に基づく(書き込みの `Written` が n=35、読みの `elapsed_us` 177〜529)。しかし 100ms は「来ないときに体感の遅れをどこで打ち切るか」の判断で、測定から導いた値ではない。tuning-constants の要件に合わせて、「測定=典型 7ms、上限 100ms は判断(根拠: 来ないときは素通しで PR 2 と同じ)」と分けて書く。B-1' を直さないと典型 7ms 自体が成り立たない点にも注意。

### N-2 ADR-263 の保持との排他は成り立つ。理由を書いておく
- MS-IME 本体の Resume は `effective_open` が偽なので、`lang_detect_may_hold` の `lang_detect_engine_active()` が偽になる(`lang_detect_shadow.rs:248-261`)。窓切替の後の最初の打鍵で始まるエピソードは「保持しない」で始まり、そのエピソードの打鍵は取り込み口で `Pass` される(`src/lang_detect.rs:750-760`)ので、R2 の判定に届く。
  R2 の再生は drain から `deliver_key_event` に入り、`lang_detect_on_key` を二度通らない。これを決定 3 に 1 行書いておく(team-lead の問い 3)。将来 `lang_detect_may_hold` の条件が変わると排他が崩れるので、architecture_guard の (i) の順序の固定が保険になる。

### N-3 R1 は追跡側ではなくライブのフォーカス窓を読む
- `read_ime_state_full` はライブのフォーカス窓を読み、書き込みは `ActuationTarget` で捕獲した hwnd に行う。fence(epoch + 追跡側の hwnd)が変わらなくても、捕獲後にフォーカスだけが動いた場合(復元タスクの `TargetMoved` と同じ型、`key_pipeline.rs:2790-2799`)は別の窓を読みうる。FocusProbe と同じ既知の性質なので、未確認の節に 1 行足す程度でよい。

### N-4 「開いた後に MS-IME が誤って英数へ戻す」ケース
- 復元のリトライは、MS-IME が後から誤って切り替える(最大 478ms)ことに備えている(`key_pipeline.rs:2730-2733`)。R1 は 1 回しか読まないので、その誤切替の後は belief が古いまま残る。従来からある経路(idle-conv-check)で拾える範囲なので、範囲外として 1 行書いておく。
