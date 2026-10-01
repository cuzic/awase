---
id: ADR-213
title: |-
  shadow toggle の OFF→ON を明示的な actuation にし、そのうえで ActivationSync 起源の SetOpen を撤去する(ADR-212 P2 の再開)
summary: |-
  ADR-212 P2(ActivationSync の撤去)は、Imm32Unavailable で物理の半角/全角を OS へ届けない(`[imm32-off] key suppress`)ため、shadow toggle が belief を ON にしたあとの
  Engine 活性化に伴う `SetOpen(true, ActivationSync)` が唯一の実 ON 書き込みになっており、全面停止すると sc-hz/sc-kanji が退行した。CI スパイク(2026-10-01、`spike/adr212-p2-shadow-on-explicit`)で、
  案B(shadow toggle の OFF→ON を明示 `ImeController::apply(true)`+ActivationSync の SetOpen を止める)と案C(案B+Engine 活性遷移で `GjiEvent::Reopen`)は退行が消えた。
  本 ADR は、(1) shadow toggle の ON/OFF 書き込みを1本の helper にまとめ OFF→ON を【許可】の明示 actuation にする(専用 `DecisionSite::ShadowToggleOn`、ImmCross 窓は Targeted+ROMAN 補完・post 完了通知、`applied` 降格、抑制窓)、
  (2) 同じ打鍵の二重書き込みを strip で防ぐ(`apply` の already-matched 省略は GjiDirect のみ)、(3) `check_active_transition` 由来の ActivationSync だけを止め明示操作の SetOpen は残す、(4) ActivationSync が `on_ime_applied` で担っていた副作用の棚卸し、
  (5) 起動前から存在する窓で `ka` がリテラルになる挙動を P2b の revert 条件にする、(6) P2a/P2b/P2b'/P2c の段階を決める。Opus round1(2026-10-01)の指摘を反映。ADR-212 決定5 を更新し、ADR-191 の「EngineDecision」節は P2c で改訂する。
status: |-
  採用(2026-10-01、Opus round1 反映済み)。実装状況: P2a は PR #408(スパイク `spike/adr213-p2ab` の CI で退行なし・I2 Unwarranted が 0 件に、`docs/experiments.md` エントリ 30 参照)。P2b は実装済み(CI 検証は P2b の PR 参照)。P2c(`SetOpenOrigin`・`ImeEvent::EngineActivationSync`・`handle_engine_activation_sync`・shadow 同一目標 strip の撤去)は feat/adr213-p2c-remove-activation-sync で実装(2026-10-01)。C2 は `handle_conv_engine_on_sync` として副作用のみ残し、C3(`strip_ime_set_open_if_settling`)は残した(settle 中の明示操作が belief 未更新で実送信される非対称を避けるため)。P2b' は未実装。実機・起動前の窓の `ka`・StaleConfirm 件数は未検証。
related_adr:
  - "ADR-212"
  - "ADR-191"
  - "ADR-203"
  - "ADR-205"
  - "ADR-199"
---

# ADR-213: shadow toggle の OFF→ON を明示的な actuation にし、ActivationSync を撤去する

## 背景

ADR-212 P2 は、Engine の active/inactive 遷移が自動で発行する `SetOpen(origin: ActivationSync)` を撤去する段。2026-10-01 の CI で次が分かり、保留になった(ADR-212 決定5)。

- **全面停止は退行する**: Engine で ActivationSync の `SetOpen` を出さないと `sc-hz-*`・`sc-kanji-*` が2回押しても反転せず、cold 起動にも退行した。
- **gate による縮小は無効**: `handle_engine_activation_sync` 先頭の棄却は pending/抑制窓/記録を省くだけで、decision の effect は executor へ流れ、同じ打鍵の `GJI direct: send 0x0016`・`outcome=Applied` が出ていた。
- **原因**: Imm32Unavailable では物理の半角/全角(0x16 等)を OS へ届けず、shadow toggle が belief を ON にする。shadow toggle は ON→OFF を書く(`runtime/key_pipeline.rs` の shadow-toggle 経路。ImmCross 先頭の窓は async の `run_open_chain_async`、他は sync の `ImeController::apply`)が OFF→ON は書かない。
  OFF→ON の実書き込みは、Engine が活性化したことに伴う ActivationSync の `SetOpen(true)` だけだった。つまり ActivationSync は、この経路ではユーザーのキーへの応答(ADR-212 決定2【許可】)を担っている。

その後のスパイク(`spike/adr212-p2-shadow-on-explicit`、マージしない)で、次の2案を CI の `sc-*` で試した。

- **案B**: shadow toggle の OFF→ON で明示 `ImeController::apply(true)` を書き、Engine は ActivationSync の `SetOpen` を出さない。
- **案C**: 案B+Engine の活性遷移(`enabled == true`)で `GjiFsmSync::Reopen(ReopenSource::EngineActivated)` を GjiFsm に送る(ActivationSync の書き込み結果が `on_ime_applied` 経由で担っていた GjiFsm の同期の代替。BUG-170 型の OffCold 固着の予防)。

sc-hz/kanji/dbe/shift の退行は B・C とも消えた(書き込み全停止の案Aは退行した)。`charthumb` の FAIL は注入ハーネスのずれ(WM_TIMER 約64ms刻み)で、案とは無関係だった。

## 決定

(Opus round1〈2026-10-01〉の指摘 B1〜B4・M1〜M9 を反映。スパイクの案B/C をそのまま本実装にはしない。)

1. **shadow toggle の ON/OFF 書き込みを1本の helper `kp_shadow_actuate(open, tick_ms)` にまとめ、OFF→ON を【許可】の明示 actuation として書く**(ADR-212 決定2)。`run_open_chain_async`・`ImeController::apply`・`issue_actuation_order` の呼び出し件数(`architecture_guard`)を増やさない。
   - `DecisionSite::ShadowToggleOn` を足し、open に応じて On/Off を選ぶ(ON と OFF を journal で数え分ける。決定1で書く ON は `dispatch_effect` を通らず `[set-open] origin=` ログに出ないので、ON の数は `caller=ShadowToggleOn` で数える)。
   - **async(ImmCross が先頭の窓)の ON は ON→OFF の写しにしない**(M2・M3)。ON は executor の ActivationSync と同じ `ImmCrossOp::Targeted`+`decide_dispatch_conv_after_open`(ROMAN 補完と宛先 hwnd の捕獲を保つ)を使う。完了通知は `let _ = with_app(..)` でなく `post_async_ime_apply_complete(open, outcome, None, OpenApplyReason::ShadowToggle)`(再入で黙って消えない。ON→OFF も揃える)。`focus_gen` が一致しない完了は捨てる(m1)。書き込み前の `record_confirmed(true)` を行うかは実装で ON→OFF と揃え、drift correction (a) との競合を journal で確認する。
   - **`applied` の降格**(M1): 分岐の先頭で `applied.applied_open() == Some(!new_val)`(記録が直前の belief と食い違う)なら `Unknown` に降格してから書く。そうしないと GJI で `AlreadyMatched` により、物理キーが Suppress されたまま誰も IME を開けない(BUG-156 型)。`None` の強制は BUG-113 を招くので、決定2の strip とセットにする。
   - **抑制窓**(M4): この書き込み(ON・既存の OFF とも)で `note_explicit_ime_action(tick_ms)` を呼ぶ。`handle_engine_activation_sync` を消すと、この責務(idle-conv-check が遷移途中の conv を拾わない)が抜けるため。
   - 決定1の書き込みは `Decision` の effect を経由しないので C3 の `strip_ime_set_open_if_settling` に落とされない(構造的に通らない)。settle 中でも書く(ユーザーのキーへの応答)。
2. **同じ打鍵の二重書き込みを作らない**(B1)。`ImeController::apply` の already-matched 省略は **GjiDirect だけ**(`ime_controller.rs:334-354`)で、MS-IME は `VK_IME_ON` と ROMAN 補完を毎回送り、ImmCross は非同期書き込みが2本走る。よって「冪等だから二重でよい」とはしない。P2a で、**shadow toggle が同じ打鍵で書いた目標と同じ `SetOpen(origin=ActivationSync)` を、キーボード経路の decision から取り除く**(`kp_run_inner` の `strip_ime_set_open_if_settling` の隣、条件 `shadow_toggled && target == effective_open`)。
3. **ActivationSync の止め方**(B2): 止める対象は `check_active_transition`(`engine.rs:339-403`)由来の遷移だけ。`transition_activation` は `ToggleEngine`・EngineOn/Off コンボ(`apply_active_transition`)、`apply_engine_on_with_ime_recovery`、`ime_set_open_effects`(IME OFF 中の Ctrl+変換が inactive→active に遷移する)からも呼ばれ、これらは明示操作として `SetOpen` を出し続ける。`transition_activation` に `emit_set_open` を渡す。P2c で `SetOpenOrigin` は ExplicitUserAction の1値になるので、enum ごと消すか残すかをそこで決める。`src/engine/tests.rs:8775-8797`(RefreshState 由来の SetOpen は ActivationSync)は「RefreshState 由来の遷移は `SetOpen` を出さない」に書き換える。
4. **ActivationSync が `on_ime_applied` 経由で担っていた副作用の棚卸し**(B3)。ON 方向だけでなく OFF 方向(active→inactive、言語バー操作・observation・RefreshState)の書き込みも担っていた。残す・消す・理由を次の表で決める。
   | 副作用(`platform.rs:984,1052-1118`) | 方針 |
   |---|---|
   | GjiFsm `ImeOff`(OnComposing の破棄)| **残す**(書き込みなしの BeliefSync `ImeOff` 通知を、Engine の `Inactive(ImeOff)` 遷移で送る。P2b' で判断) |
   | GjiFsm `OnImeOn`/`Reopen`(ON 方向)| ON は OffCold なら最初のローマ字で ADR-203(i)(`needs_belief_sync_on`)が拾う。`OnWarm` が古いまま残るケースだけが未カバー(M8)。**P2b では足さず**、取り残しの目印が出たときだけ P2b' |
   | `ImeModeFsm.on_set_open_applied(false)` | P2b で観測し、取り残しが出たら P2b' |
   | `mark_composition_cold(SetOpenTrue/False)`・`reset_candidate_was_seen` | 同上 |
   | idle-conv-check の抑制窓 | 決定1の `note_explicit_ime_action`(M4) |
   P2b' の `EngineActivated` Reopen は、スパイクの配置(`dispatch_effect` の `EngineStateChanged`)では settle 中の一瞬の活性化でも発火するため(M9)、**発火点を遷移の origin が分かる場所**(キーボード経路 `kp_stage_post_decision`、loop 経路 `execute_decision`)にし、settle 中は送らない。
5. **C2・C3 の整理**(M5・M6): `kp_apply_conv_engine_sync` は `handle_engine_activation_sync` を直接呼ぶため、P2c でこの関数を消すなら C2 が何を残すか(ログだけ・抑制窓だけ)を決める。`lints/ime_event_guard/src/lib.rs`、`golden_scenarios.rs` シナリオ16、`platform_state.rs:2179-2300` のテスト、`tests/support/{harness,invariants}.rs` を、消す・書き換える対象として列挙する。C3 は、ActivationSync が消えると strip が落とすのは ExplicitUserAction だけになり、「settle 中の Ctrl+変換が黙って消える」という【許可】に反する挙動だけが残るので、P2c で strip と `schedule_settle_retry` を撤去する(または残す理由を書く)。
6. **loop 経路・起動直後の期待される挙動**(M7): `ImeModel` の初期値は `desired_open: true`(placeholder)で、起動47ms後の loop 経路の書き込みはこの既定 ON を実 IME に書いて自己成就させていた。P2b の後、**awase の起動前から存在するスレッドの窓(新スレッド=閉の対象外)で IME が実際に閉じていると、Engine は active のままローマ字を送り `ka` がリテラルで出る**。所有者方針(ADR-191、awase は IME に書かない)では「書かない結果」として受け入れる余地があるが、ADR-212 M6(i)(「nonaiyo」)の再現でもあるので、**P2b の revert 条件にする**: 起動前から存在する窓・IME 閉・観測なしで最初の文字がリテラルになる件数を CI/実機で数え、所有者に提示して判断を仰ぐ(受け入れる/belief の既定値を変える/P2b を revert)。
7. **検証**(ADR-212 決定5 を引き継ぐ)。
   - **1打鍵あたりの実送信数**(P2a の不変条件): MS-IME の `VK_IME_ON` と ROMAN の IMC write が1回であること。`[apply-ime]`・`[ime-io] actuation SendInput` の件数を同じ打鍵のログで数える。`outcome=Applied` だけで成功としない。
   - StaleConfirm・ESC での未確定文字消失(BUG-170 型)、`[key-effect-miss]`。合格基準は「ゼロ」でなく「develop と同じ土台で比べて増えない」。
   - gate/棄却を足したときは、自分の skip ログでなく下流の実送信が消えたかを見る。CI は PR の土台と同じコミットで取る。
8. **段階**(1段=1PR、各段は単独で revert できる。キーボード経路〈二重書き込みのリスク〉と loop 経路〈nonaiyo・予測の表面化のリスク〉の境界で分ける)。
   | 段 | 内容 | 検証 |
   |---|---|---|
   | P2a | 決定1・2・(M4 の抑制窓)。ActivationSync はまだ止めないが、shadow 打鍵の同一目標 SetOpen は strip | MS-IME/ImmCross で1打鍵あたり送信1回。sc-hz/kanji/dbe/shift |
   | P2b | loop 経路と shadow 以外のキーボード経路で、`check_active_transition` 由来の ActivationSync の SetOpen を止める(決定3)。**`EngineActivated` は入れない** | 起動直後(決定6)、OnWarm/OnComposing の取り残しの目印、StaleConfirm・`[key-effect-miss]` を develop と比較 |
   | P2b' | (P2b で取り残しが見えた場合だけ)ON/OFF 対称の BeliefSync 通知(決定4) | P2b と同じ土台での A/B |
   | P2c | `SetOpenOrigin::ActivationSync`・`ImeEvent::EngineActivationSync`・`handle_engine_activation_sync`・C2/C3 の整理(決定3・5)。テスト・lint・guard の更新 | コンパイル、`architecture_guard`、golden |
9. **ADR-212 との関係**: ADR-212 決定5 の「P2 は保留」を、本 ADR の段階で再開する旨に更新する。ADR-191 の「EngineDecision」節は、P2c の PR で改訂する(ADR-212 決定6)。

## 実装後の知見(2026-10-01、P2a=PR #408・P2b の CI 結果)

- **P2b の CI**(`sc-*`、develop `59a5072c` と比較): 期待表は同一、I2 Unwarranted は全構成で 0(develop は最大17件)、起動前の窓の GJI(`sc-p2-initial-chrome-gji`)は 5/5 PASS。
- **I2 が P2a 単体で間欠的に増える**(`sc-adr211-chrome-msime-f13`、5回中2回が超過、develop は 5回とも 1): Opus のコードレビューでは**退行ではない**。develop でも各 action の時点で ActivationSync の書き込みは出ており(filtered ログが Unwarranted しか拾わず見えなかった)、IME OFF より前の GJI I/O 観測(鮮度窓3秒)で授権されている。間欠は refresh(約500ms周期)と次の打鍵(約40ms)の競合による(推論)。`eff=false conf=true` は診断用の値で belief の食い違いではない。**P2b で I2=0 になるのは Unwarranted を出す経路ごと止めた副産物**で、原因の本体(明示意図より古い観測を drift correction と授権に使うこと)は残る。→ **P2a と P2b は同時に入れる**。原因の本体は P6 の候補(明示意図より前の観測を除外)。
- **P2b の新しい懸念 `i4_gji_fsm_off_cold_composition`**(`sc-follow-chrome-atok-eisu`・`sc-follow-chrome-msime-hankaku`、再実行 4回中 1回+最初の run): 当初の仮説(絶対 IME OFF キーの書き手が ActivationSync だけだった、Opus B3)は**ログで反証**された(実 IME は閉じており、OFF は shadow toggle の `VK_IME_OFF` で書かれていた)。真因は2つの組み合わせ。
  1. P2b で起動直後の loop 経路の `VK_IME_ON` が無くなり、それを引き金に起動していた GJI 変換プロセスが立ち上がらず、`gji_monitor` が最初の IME ON から最大約3秒つながらない(develop は書き込みの 10〜30ms 後に接続)。その間 literal-detect が `PlanSkippedLiteral` になり、cold probe が1 tickで `OnWarm` に確定する(実機では、ログイン後に一度でも IME を使っていれば小さい。GJI 変換プロセスの再起動後・ログイン直後は同じ窓ができる。推論)。
  2. 候補窓 SHOW の保留 latch が IME OFF・フォーカス変更で捨てられない潜在バグ(develop にも以前からある。BUG-180、PR #410)。probe が早く終わったため、最後の送信の後に来た SHOW が残り、IME OFF の後の文字で古い SHOW が `StartComposition` として配られた。
  対策: (1)BUG-180 の修正(PR #410)、(2)IME ON を書いたとき `gji_monitor` が未接続なら即時に再探索を要求する(新しい定数を足さない。**BUG-180 だけで i4 が消えるなら不要**)。起動時の `VK_IME_ON` を戻すのは ADR-212 の方針に反するので採らない。
- **Opus B3 は今回の i4 の原因ではなく、未検証のまま残る**: IME が awase 以外の手段(言語バー・IME 自身が処理するキー)で閉じ、Engine が観測で deactivate する場合に、ActivationSync の OFF 方向が担っていた GjiFsm `ImeOff` 等が届かない件。P2b' の候補。
- **検証に追加**: P2b 以降の CI では、各構成の run 1 で `attached to GJI process` の時刻が最初の送信より前か、`i4` と `PlanSkippedLiteral` の件数を develop と比べる。CI の複数回比較は、同じ ref への連続 dispatch が concurrency でキャンセルされるため、別ブランチ(`spike/*-repN`)で並列に流す。

## 非目的

- shadow toggle の ON→OFF の挙動変更。
- conv 軸(P7)、BUG-179(CI の MS-IME 構成の TIP 同定)。BUG-179 は別 PR。
- v1(`v1-develop`)への backport の判断。

## 代替案

- **案A: 書き込み全停止**: CI で退行(却下済み、ADR-212 決定5)。
- **gate による縮小**: 実書き込みを止めない(取り下げ済み)。
- **ActivationSync を残す**: ADR-212 の所有者方針(awase は IME に書かない)に反し、ユーザー操作を引き金にしない書き込みが残る。

## リスク

1. shadow toggle OFF→ON を async で書く場合の `with_app` 再入(ON→OFF と同じパターンで回避するが、ImmCross 先の窓で CI/実機確認が要る)。
2. P2b 後に、起動前から存在する窓で `ka` がリテラルで出る(決定6)。
3. ActivationSync の OFF 方向が担っていた GjiFsm `ImeOff` 等の取り残し(決定4)。P2b で目印を見る。
4. 実機(Windows 11、MS-IME 本体)は未確認。CI の MS-IME 構成は TIP 同定が異なる(BUG-179)。
