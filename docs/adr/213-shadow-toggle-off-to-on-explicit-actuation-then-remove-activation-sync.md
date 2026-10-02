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
  採用(2026-10-01、Opus round1 反映済み)。実装状況: **P2a(PR #408)・P2b(PR #411)は develop にマージ済み**(2026-10-01、`sc-*` で期待表は develop と同一・I2 Unwarranted が全構成で 0・BUG-179 の `sc-p2-initial-chrome-msime` が 5/5、BUG-180〈PR #410〉の修正と併せて `i4` 超過は消えた。`docs/experiments.md` エントリ 30 参照)。P2c(`SetOpenOrigin`・`ImeEvent::EngineActivationSync`・`handle_engine_activation_sync`・shadow 同一目標 strip の撤去)はマージ済み(PR #412)。**P2d-1(C2 の縮小)はマージ済み**(PR #413、2026-10-01。`handle_conv_engine_on_sync` を削除し、`EngineSync::SetOpen` 分岐はログ1行と `ImeStateHub::release_panic_reset_guard_on_positive_evidence`〈`ForceOnReason::PanicReset` のみ除去〉だけ。`should_release_panic_guard` 純粋関数・`architecture_guard`・単体テストで固定。ジャーナルリプレイは `ImeStateHub` が host から見えないため未追加。実機・`ts-*` は未検証)。**P2d-2(C3 の撤去)は実装済み(PR #414)**: `strip_ime_set_open_if_settling`・`handle_engine_set_open` の settle フィルタ・`focus_transition_was_pending` のスナップショットと、strip した SetOpen の `schedule_settle_retry` 呼び出しを撤去(drift correction の settle 延期用の `schedule_settle_retry` は P6 まで残す)。判定の純粋関数 `settle_disposition` は SetOpen を落とす分岐自体が無くなったので作らず、`architecture_guard::settle_does_not_drop_explicit_set_open` で固定。P2b' は未実装(B3 は未検証)。実機・起動前の窓の `ka`・StaleConfirm 件数は未検証。
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
5. **C2・C3 は P2c では残し、P2d で撤去する**(2026-10-01、所有者判断。根拠は Opus による C2/C3 の根本原因調査)。
   - **共通の根本原因**(推測を含む): (1) GjiFsm の同期・composition cold・`ImeModeFsm`・`applied`・pending・refresh の停止・抑制窓が、すべて `SetOpen` の effect → `on_ime_apply_complete` → `on_ime_applied` に束ねられている(「awase が書いた」事実に束縛)。書き込みを止める(C3 の strip)と副作用が起きず retry を足し、書かずに同期したい(C2)と副作用だけを手で再現する(書かない pending・抑制窓・kill)ガードを足した。(2) Engine は決定と同時に遷移を確定し(`prev_activation`)、Platform が後から effect を拒否できるため、拒否のたびに「自然には再発行されない」状態が生まれる。(3) 「フォーカス直後は観測が当てにならない」問題を、読み側のフェンスでなく書き込み側で止めている。
   - **C2**(`kp_apply_conv_engine_sync` → `handle_conv_engine_on_sync`、idle-conv-check の `RomajiRecovered`): IME へは書かない。4つの副作用のうち、守っているのは **`on_set_open_requested` の中の `force_guards.clear()`(TsfNative でトレイの「状態をリセット」が立てる PanicReset ガードを外す地点)だけ**。TsfNative では観測の成功による解除が来ない。残り3つは書き込みと対の副作用の模倣で、C2 は書かないので何も守らず、害がある: `TIMER_IME_REFRESH` の kill は他の目的の refresh(BUG-51 の20ms再確認・ADR-205 の読み直し・BUG-158・`schedule_settle_retry`)を消し(TsfNative では explicit_intent 確定後に timer が恒久停止するため戻らないことがある)、`ImeApplyRequested{target:true}` は完了が来ない孤立 pending を最大8秒残して executor の世代付けと journal を嘘にし、Ctrl chord barrier を解除しうる(推測)、`last_explicit_ime_action_ms` は次の idle-conv-check を `EXPLICIT_IME_SUPPRESS_MS`(1500ms)止めるだけ(RomajiRecovered の再発火は前段の `InputModeObserved` で belief が更新されるので抑制が無くても起きない。推測、リプレイで確認)。
   - **C3**(`strip_ime_set_open_if_settling`・`schedule_settle_retry`・`handle_engine_set_open` の settle フィルタ): P2b 後に SetOpen を出すのは明示操作(ToggleEngine・EngineOn/Off コンボ・`ime_set_open_effects` の Ctrl+変換・ADR-206 の無変換/変換の単独タップ)だけ。2026-07-05 の動機(Alt+Tab 中間窓で Engine の**自動**遷移が未確定 belief から書く)と、2026-07-08 の動機(`eab554e3`、HwndCache 復元の ActivationSync が strip され GjiFsm が OffCold に固着して「このせっけい」の先頭が欠落)は、どちらも ActivationSync 起因で消えた(OffCold は ADR-203(i) の `send_keys` 直前の level reconcile が拾う)。残る strip は settle 中(`caps()` で ImmCross 100ms・TsfNative 200ms・Imm32Unavailable 500ms)に押された明示操作を**黙って捨てる**だけで、ADR-206 の単独タップは Consume 済みなので IME にも awase にも届かない二重の空振りになる。retry(refresh)は意図を記録していないので何も再発行しない。
     **ただし残る懸念(推測、実測が要る)**: Imm32Unavailable の settle 500ms は Chrome の TSF 文脈の再初期化(BUG-002、実測 約326ms)と重なり、strip を外しても settle 直後の書き込みが Chrome×GJI に受け付けられない可能性がある。P2a 以降、物理の半角/全角(shadow toggle)は settle 中でも書いているので、同じ条件は既にキーボードで起きているはずだが実測が無い。
   - **P2d の段階**(P2c では残す。P2c で消す・書き換えたものは下記):
     - **P2d-1(C2)**: `handle_conv_engine_on_sync` を削除し、`EngineSync::SetOpen` の分岐はログ1行と PanicReset ガードの解除だけにする(`ImeStateHub` に `release_panic_reset_guard_on_positive_evidence` のような意図の名前を持つメソッドを置き、`force_guards` を直接触らない)。TIMER の kill・世代・`ImeApplyRequested`・抑制窓は書かない。PanicReset ガードの期限切れ(`purge_expired` が本番から呼ばれていない)の根本対応は P6(tuning 定数の実測規約がかかる)。回帰テスト: `should_release_panic_guard(EngineSync)` の純粋関数を全 variant で固定、`architecture_guard`(`kp_apply_conv_engine_sync` に `ImeApplyRequested`・`allocate_event_generation`・`timer.kill(TIMER_IME_REFRESH)` が無い)、ジャーナルリプレイ(TsfNative の Eisu→Romaji 列で孤立 pending が無く RomajiRecovered が再発火しない)。revert 条件: TsfNative で RomajiRecovered の直後に conv 読みが連続して belief の入力モードが振動する場合(抑制窓だけ戻す)。
     - **P2d-2(C3)**: **先にスパイクで実測する**。CI の chromeprobe に、窓の切り替え直後 t=50/150/300/450ms に Ctrl+変換と無変換の単独タップ(と、比較として物理の半角/全角)を押すケースを足し、(a) develop(strip あり)で押下が無視される率、(b) strip を外したスパイクで `VK_IME_ON` が Chrome×GJI/MS-IME に受け付けられたか(`Process(229)` と次の文字が composition になるか)を測る。(i) 受け付けられるなら、strip と `handle_engine_set_open` の settle フィルタを**明示操作についてまとめて**外す(belief と実書き込みの非対称を作らないため)。(ii) 受け付けられないなら、意図(`write_set_open_request`・IntentStore)は即記録し、実書き込みだけを settle 明けの既存の refresh で1回行う**単一スロット**(`deferred_explicit_actuation`、最新が勝つ、FocusChanged で破棄)にする。新しいキューは作らない(ADR-156: 解放の窓口は settle 明けの refresh の1つ、破棄の窓口は FocusChanged と上書きの2つ。`fix-requires-evidence.md` の defer/replay の行に追記する)。判定は純粋関数 `settle_disposition(origin, settling) -> {Execute, Defer, Strip}` に切り出し、明示操作が Strip にならないことを全数で固定する。revert 条件: 窓の切り替え直後の明示操作で IME が逆向きに切り替わる、または別の窓の IME が変わる(宛先の誤り。experiment-logging に従いアプリ × IME × 押下時刻を本文に)。
     - **P2d-2 の実測結果**(2026-10-01、スパイク `spike/adr213-p2d2-settle-explicit`、CI run 36846631891、Chrome 前面化→ helper 窓へ focus を外して戻す→Tms 後に押下、各5回): **Ctrl+変換(keys.ime_on の既定コンボ)を awase が focus を検知した約20ms後(目標 t50、実押下 約192ms)に押すと、strip あり(現状)は5/5 が `[focus-settle] SetOpen effect stripped` で落ちて IME が開かず(次の k,a が `ka`)、strip なし(`AWASE_SPIKE_NO_SETTLE_STRIP=1`)は Chrome×GJI(ATOK プリセット)・Chrome×MS-IME とも5/5 が受け付けられ、次の文字がかなになった**(strip が落とした SetOpen は Chrome が受け付けられる)。t150(実押下 約292ms、focus 検知の約120ms後)以降は strip が働かず(`effect stripped` 0件)両方 PASS。つまり実効の settle(barrier が残る窓)は focus 検知後の短い間(<約120ms)で、500ms ではない。→ **(i) 受け付けられる場合**に該当: strip と `handle_engine_set_open` の settle フィルタを明示操作についてまとめて外す方針を採る(P2d-2 本実装)。限界: n=5、物理の半角/全角(hz)は t50 の実押下が focus 検知より前(約111ms)で測れず、t150 以降は両方 PASS。無変換の単独タップは CI の ATOK/MS-IME 構成で開閉に割り当てが無く全 T で FAIL(測定不能、別途構成が要る)。Alt+Tab の中間窓(2026-07-05 の状況)に明示操作を押す場合は未測定。
     - **P2d-2 の実装(2026-10-01)**: 実測が(i)だったので、(a)`executor.rs::strip_ime_set_open_if_settling`(関数・単体テスト5本)、(b)`key_pipeline.rs::kp_run_inner` の strip 呼び出しと settle の `schedule_settle_retry`、`focus_transition_was_pending` のスナップショットと `kp_stage_post_decision` の引数、(c)`runtime/mod.rs::execute_decision` の strip 結果の再試行、`executor.execute_from_loop` の戻り値(3要素→2要素)、(d)`platform_state.rs::handle_engine_set_open` の settle フィルタと引数を撤去した。残したもの: `ime_refresh.rs` の drift correction の settle 延期用 `schedule_settle_retry`(drift (a) は P6 まで)、`is_focus_transition_settling` と `consume_focus_barrier` 等の barrier 機構(観測の保護・drift の判断に使う、SetOpen 以外の目的)。`settle_disposition` は作らない(落とす分岐が無い)。決定論の回帰: `platform_state.rs` の `handle_engine_set_open_applies_even_while_focus_transition_settling`(settle 中でも belief を書いて適用、Windows ターゲット限定の `state/` 内なので CI の windows-build で走る)と `architecture_guard::settle_does_not_drop_explicit_set_open`(Linux)。executor/`kp_run_inner` は `#[cfg(windows)]` で Linux のハーネスからは見えないので、settle 中の明示操作が実送信まで届くことの golden/closed_loop 化はできない(`sc-settle-explicit-*` の CI シナリオ新設は未実施)。
     - **P2d-2 で strip を外したことで起きうる退行**(未測定): Alt+Tab の中間窓(2026-07-05 の状況)に明示操作(Ctrl+変換・EngineOn/Off コンボ・無変換/変換の単独タップ)を押すと、strip なしでは中間窓の宛先に書く。現在は SetOpen を出すのが明示操作だけなので、ユーザーが押した意図は中間窓でも有効だが、窓の切り替えの最中に押すと最終着地先でなく中間窓へ書かれうる。**revert 条件**: 窓の切り替え直後(focus 検知後 約20〜500ms)の明示操作で IME が逆向きに切り替わる、または別の窓の IME が変わる(宛先の誤り)。観測したら experiment-logging に従い、アプリ × IME(GJI/MS-IME、ON/OFF)× 押下時刻(focus 検知からの ms)を revert コミット本文と `docs/experiments.md` に書く。実測済みの範囲は Chrome×GJI・Chrome×MS-IME の focus 検知約20ms後・各5回のみ。
     - **P2d-3**: 片付け(ADR-212 決定5 の C3 の注記の更新=済。残りは P2d-1 後の確認のみ)。
   - P2c で消す・書き換えたもの: `lints/ime_event_guard/src/lib.rs` の許可リスト、`golden_scenarios.rs` シナリオ16/16b(削除)、`platform_state.rs` の ActivationSync テスト群(filter 系3本を削除、不変条件2本を `handle_conv_engine_on_sync` 向けに書き換え)、`tests/support/{harness,invariants}.rs`(`WriteOrigin::EngineActivationSync` を撤去し、Engine decision に SetOpen が出たら panic する退行ガードに変更)。
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
   | P2c | `SetOpenOrigin`(enum ごと)・`ImeEvent::EngineActivationSync`・`handle_engine_activation_sync`・shadow 同一目標 strip の撤去。C2 は副作用だけ残し(`handle_conv_engine_on_sync`)、C3 は残す(決定5)。テスト・lint・guard の更新 | コンパイル、`-D warnings`、`architecture_guard`、golden、dylint(CI) |
   | P2d-1 | C2 の撤去(PanicReset ガードの解除だけ残す。決定5) | 純粋関数テスト、`architecture_guard`、ジャーナルリプレイ、`ts-*` |
   | P2d-2 | C3 の撤去(実測の結果(i)=外す。実装済み、`settle_disposition` は作らず `architecture_guard` と `handle_engine_set_open` のテストで固定) | `architecture_guard::settle_does_not_drop_explicit_set_open`、`sc-settle-explicit-*`(新設は未実施) |
   | P2d-3 | 片付け(strip・settle フィルタの削除または Defer への置き換え、ADR の更新) | コンパイル、`architecture_guard`、golden |
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

## 所有者決定(2026-10-01、起動直後の最初の内部状態と v2 の扱い)

- **原則**: IME が ON なら NICOLA ON、IME が OFF なら NICOLA OFF。モードずれは許容するが、できるだけ少なくする。
- **決定6(M7)の確定**: IME の状態が読めない窓(実 Chrome など Imm32Unavailable)で、awase 起動直後は**観測が得られるまで NICOLA は OFF(生キーを通す)**。実 IME が ON と分かってから NICOLA ON にする。belief の既定値(placeholder の ON)で Engine を active にしない。代償: IME が実際に ON でも、観測が得られるまで(先同期は最大約0.5秒、読めない窓はもっと長い)最初の文字が生のキーになる。実装は未着手。MS-IME×実 Chrome の ON 起動の切り分け実験(`sc-startup-msime-chrome-on-{noawase,precheck,gated,norefocus2}` 各8回、2026-10-01 23:22 UTC)は**全 32 回 PASS**(打鍵直前の実 IME は全回 open、awase なしの対照も 8/8 かな)で、14:52 UTC ごろに出た約半数の `ka` は**再現せず環境依存と判断**した。再現できない現象への修正は見送り、再発時に切り分け構成で `real_ime_open_before_type=false` の回を捕まえてから、`[msime-ready]` が閉と確認したら強制送信せず belief を正す修正とあわせて決める。
- **v2 のブロッカーにしない**: MS-IME×実 Chrome の起動直後の最初の文字(約半数でローマ字 `ka`、develop でも 6 回中 3 回)は、実験で環境側の寄与を切り分けたうえで既知の制限として記録する。awase 側の欠陥(IME が閉と確認したのに強制送信、観測を belief に反映しない)だけを修正する。
- **v1 へは backport しない**(v2 リリースで v1 は保守終了、重大バグのみ backport の既存方針)。
- **実機確認**(Windows 11、MS-IME 本体、起動前からある窓、WT×GJI の A/B)は、ADR-208 の L1〜L3 をマージし終えてからまとめて行う。

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
