# ME: 擬似 IME(閉ループのハーネス)の現状と、足りない能力の分析

調査版: origin/develop `f3c2007e`(worktree `/home/cuzic/rust-nicola-wt-mech`)。読み取り専用、cargo 未実行。
行番号は断りがなければ `crates/awase-windows/` からの相対パス。「未確認」は読んだだけで走らせていないもの。

## 結論

- 今のハーネスが**本番の実コードで通す**のは、`ImeStateHub`(`dispatch_event`・`effective_open_at`・`record_explicit_intent`・`apply_key_effect_prediction`・`align_placeholder_desired`・外部変化の監視窓)、`ImeModel::reduce`、`issue_open_warrant`、`check_drift_correction`、`KeyEffectKeymap::predict`、`plan_key_effect_track`、`Engine`(`RefreshState`/`FocusChanged` だけ)。**本番の入口(`runtime/`・`ime_controller.rs`・`output/`)は 1 本も通らない**。
- 写し(複製)は約 160 行(下の 1.2)。うち**本番の純粋な核が既に出ているのに使っていない**ものが 2 系統ある(drift の判断 `decide_drift_plan`、予測入力の `unreadable`/`observable_toggle_guard`/半角英数の追跡)。これは新しい部品を作らず 1 PR で本物に置き換えられる(ME-1)。
- 「撤去/統合で挙動が変わらない」を Linux で言える範囲は、いまは **belief・warrant・予測・drift の検知まで**。**機構の送信(MechanismCommand)・同期 actuation の gate/claim/ledger・TSF の composition/literal・窓/スレッドのモデル**は言えない。
- 最小で検証範囲が最も広がる順(詳細は 4): **ME-1**(写しを既にある核の呼び出しに置換、1 PR)→ **ME-2**(ADR-241 PR-B: ハーネスを crate 内へ移し `dispatch_set_open` を通す。**決定済みの作業の続き**)→ **ME-3**(純粋な判断の「決定表の全列挙 golden」。ADR-208 L0 の方式の一般化。撤去/統合の「差分ゼロ」の主証拠にする)→ ME-4(mode-key-follow・外部変化・placeholder 揃えの 3 つの「読み戻しで belief を揃える機構」を 1 つのハーネスに載せる)→ ME-5(窓/スレッド表)。TSF の composition/literal の模擬(ME-6)は**撤去候補が出てから狭く**。
- TH1e(「N 本の決定レコードの再生で差分ゼロ」)は ADR-241 §5 が既存の再生一式の撤去を決めているため、そのままの形では進めない。代わりは ME-3(生成した入力の全列挙)。complexity-budget は所有者判断で「据え置き」なので、発効は求めない。

## 1. 今のハーネスが通す経路と写し

### 1.1 実コードで通す(`tests/support/harness.rs` 771 行)

| 本番の部品 | ハーネスでの呼び出し | 備考 |
|---|---|---|
| `ImeStateHub`(`state/platform_state.rs`、ungated。`with_clock(HubClock::manual)` で仮想時計) | `harness.rs:230`(start)ほか全体 | テストから届く `pub fn` は 14 個だけ(`tests/architecture_guard.rs:7734` `PLATFORM_STATE_PUB_FNS`)。それ以外は `pub(crate)`(INV-A97-1) |
| `ImeStateHub::dispatch_event` → `ImeModel::reduce`・event_log・journal | `reduce()` | 本物 |
| `effective_open_at`・`record_explicit_intent`・`warrant_context` | `effective_open()`・`user_set_open()` | IntentStore もハブの中で本物 |
| `apply_key_effect_prediction` | `key()` 内 | 本物 |
| `align_placeholder_desired`(`ir_align_placeholder_desired` が呼ぶ核) | `settle()` | 本物 |
| `arm/follow_external_change_in_scope`(ADR-205) | `external_injected_key()`・`prefetch_read()` | 本物。`Setup::external_close_watch` で有効/無効 |
| `issue_open_warrant` | `warrant_for()`(`harness.rs:654`) | 本物 |
| `check_drift_correction` | `settle()`(`harness.rs:698`) | 本物(検知まで) |
| `KeyEffectKeymap::predict`・`plan_key_effect_track` | `key()`(`harness.rs:287`) | 本物。ただし入力 `KeyTrackFacts`/`PredictInput` は決め打ち(1.2 の B) |
| `awase::engine::Engine` | `settle()`・`focus_change()`(`RefreshState`/`FocusChanged`) | `on_input`・タイマーは呼ばない(`harness.rs:26-28` の doc) |

### 1.2 写し(複製)・バイパス

A〜F は写し。G 以下はバイパス(そもそも通らない)。

- **A. `issue_write`(`harness.rs:662`)+ `warrant_for`(`:654`)**: `ImeStateHub::issue_actuation_order`(`state/platform_state.rs:1398`)の写し。本番は `ActuationOrder::issue` を返し `run_chain` を駆動するが、ハーネスは「`issue_open_warrant` が `Some` なら `PseudoIme::write_open`」の 1 行。機構チェーン(ImmCross→GjiDirect→MsImeDirect)・`decide_gate`・`claim_press_write`・applied の未知化・Failed のフォールスルーを通らない。`MechanismCommand` を作らない。
- **B. `key()` の `PredictInput`(`harness.rs:306-316`)**: 本番 `kp_predict_key_effect`(`runtime/key_pipeline.rs:1921`)との差。`unreadable: false`(`:313`)固定 → **ADR-209 の「読めない窓で変換が開く」予測を、`Setup::imm32_unavailable` でも一度も通らない**。`observable_toggle_guard: false`(`:315`)固定 → ADR-260 のガードを通らない。`track` に `without_conv_while_half_width_alnum`(ADR-244、本番 `key_pipeline.rs:~1985`)が無い。`predict` であって `predict_after_resent`(再送分)ではない。学習済み表(`override_table`、ADR-195)なし。`composing` は `composing_visible` と擬似 IME の stage から作る(本番は TSF の `ime_composition_active_now()`)。`kp_reopen_gji_fsm`(ADR-203)も通らない。
- **C. `settle()` の drift 分岐(`harness.rs:698-729`)**: 本番 `ir_apply_drift_correction`(`runtime/ime_refresh.rs:1025-~1290`、約 260 行)の前半の写し。本番は `ir_observe_drift_facts`(`:943`)→ **ungated な核 `decide_drift_plan(&DriftFacts)`(`awase-ime-core/src/state/drift_plan.rs:386`)** で Idle(EngineDisabled/NotJapaneseIme/NoDrift)・DeferToSettle・Act(SkipWarrantWouldBlock/GiveUp/Confirmed/Send)を決めるが、ハーネスは `decide_drift_plan` を呼ばず(`git grep decide_drift_plan crates/awase-windows/tests` = 0 件)、BUG-163 の「ImmCross で warrant が下りなければ見送る」の 1 条件だけを手で書いている。Blind の打ち切り・Read の収束確認・settle 待ち・conv ラッチは写していない(`harness.rs:21-24` の doc に明記)。
- **D. `user_set_open`(`harness.rs:456`)**: 本番は物理キー → `kp_run_inner`(`key_pipeline.rs:62`)→ `Engine::on_input` → `kp_stage_post_decision`(`:1548`)→ `handle_engine_set_open` → `record_explicit_intent` → `dispatch_ime_set_open`(`runtime/executor.rs:702`)。ハーネスは `UserImeSetIntent` を reduce して `record_explicit_intent` し `issue_write` を呼ぶだけ。使用は閉ループのシナリオで 6 か所。
- **E. `focus_change_to_new_thread_unreadable`(`harness.rs:416`)**: `focus_tracking` の「新スレッドの読めない窓」(`assume_closed_for_new_thread` 相当)を、同じ事象を流して写す。コメントに「本番の関数は `pub(crate)` なので」とある。
- **F. `handle_engine_decision`(`harness.rs:682`)**: Engine の decision が `SetOpen` を出したら panic。ADR-213 P2b/P2c の退行検知(これは有用)。
- **G. 通らない本番(doc に列挙済み)**: ADR-187 の通過マーク(`ModeKeyPassLatch`)→ `ModeKeyPassedThrough`・direct-follow(`follow_direct_read_in_scope` `platform_state.rs:584` は `pub(crate)`)、`ImeApplyRequested`/`applied` の往復、ForceGuard、TSF warmup(`tsf/warmup/*`)、`Engine::on_input`、`PhysicalKeyDisposition::plan_core`(物理キーを IME へ届けるか。`explicit_press_exhaustive` が別に全列挙)、`kp_shadow_actuate`/`kp_stage_shadow_ime_toggle`(`key_pipeline.rs:1381`/`:1044`)、非同期 ImmCross(`runtime/open_chain.rs:567`)。
- **H. 文書の古い記述**: `closed_loop_scenarios.rs:18-22` の表は `ImeStateHub` を「見えない(`#[cfg(windows)]`)」「`ModeKeyPassLatch` は `pub(crate)`」と書くが、`state/mod.rs:15-16` で `platform_state` は ungate 済み(ADR-224 P4)、ラッチ型は `awase-ime-core/src/state/mode_key_pass.rs:57` で `pub`。**ハブ経由の arm(`arm_mode_key_pass_mark_in_scope` `platform_state.rs:357`)が private** という点だけが残る。表は直すべき(ME-1 のついで)。

### 1.3 乖離のリスクの実例(ADR-224 は「写しが原因の見逃しは実例なし」としていた)

見逃し(閉ループが緑のまま実機で壊れた)は**確認できなかった**(git log の網羅は未確認)。ただし次の 3 件は、撤去/統合の検証に使うと誤る型の実例:

1. `stale_eisu_blocks_engine_after_predicted_open_in_a_new_thread_window`(`closed_loop_scenarios.rs:169`、`#[ignore]`)。本番は mode-key-follow(20ms の直接読み、`runtime/ime_refresh.rs:329`)で救われるが、ハーネスは写さないので**偽の陽性**。mode-key-follow を撤去/統合しても、閉ループは何も言えない(テストは無効化されている)。
2. `non_japanese_ime_blocks_drift_correction_write`(`:544`)。アサートは「DriftCorrection の書き込みが無い」だけで、`decide_drift_plan` の `Idle(NotJapaneseIme)` が止めたのか `issue_open_warrant` の `is_japanese_ime` 棄却(`awase-ime-core/src/state/open_warrant.rs:165`)が止めたのか区別しない。**この 2 重の関門のどちらかを撤去しても通る**(統合の候補になりうるが、閉ループは「どちらも要る/どちらかで足りる」を言えない)。
3. BUG-171(`docs/known-bugs/BUG-171.md:35`): StaleConfirm→ESC は `tsf/warmup/probe_fsm.rs` にあり、ハーネスは TSF warmup を写さないので足せなかった。

## 2. 擬似 IME(`tests/support/pseudo_ime.rs` 521 行)が模す/模さない

### 2.1 模している(根拠は `tools/e2e/ime_key_matrix/grid-tables/*.json` の実測の生データ)

- 状態: `open` × `conv`(0x19/0x10 の 2 値) × `stage`(None/Typing/ConvSpace/ConvHenkan/ConvMuhenkan)。物理キー 13 種(`grid_key_name`)の押下は格子の多数派セルを引く。文字キーは「開なら Typing」。**格子に無い状態からの押下は panic**(推測で埋めない)。
- 格子 3 種(`Grid`): `Atok`(`atok.json`)、`GjiMsime`(`msime.json`、GJI の MS-IME プリセット)、`MsimeNative`(`msime-native.json`)。使い分けは `Setup::with_grid` が擬似 IME の真値と**予測器の同梱表**(`harness.rs:~237` の `KeyEffectKeymap::from_config(Some(1|2))`/`for_msime_native`)を同時に切り替える。
- 使用実績: 既定は ATOK。`GjiMsime`/`MsimeNative` は `msime_grid_hiragana_key_then_typing_and_commit`(`:523`)とランダム探索(`:696`)だけ(`git grep -c "Grid::GjiMsime\|Grid::MsimeNative" closed_loop_scenarios.rs` = 2)。`Setup` の構築子は `imm_cross`/`imm32_unavailable` の 2 つだけで、`TsfNative`/`Plain`/`InputRelay` の窓は作れない(フィールドは `pub` なので手で書けるが、使用 0 件)。

### 2.2 模していない(仮定・欠落)

- **ATOK 固定の仮定**: 入力中の段階の遷移(Space→変換中 等)は生データに無く、`next_stage`(`pseudo_ime.rs`)の仮定。BUG-205 はこの仮定(Esc→Typing)と予測器の食い違いだった(修正済み)。
- **`write_open`(`pseudo_ime.rs`、awase からの書き込み)が格子を引かない**: `VK_IME_OFF`(0x1A)を `SendVk` で送る GjiDirect の書き込みは、本来は格子の `ime-off` セル(`press(0x1A)`)で動くはずだが、`write_open` は「開閉だけ変える」単純化。ImmCross の `IMC_SETOPENSTATUS` と GjiDirect の VK 送信を区別しない。ME-2 の sink で解消できる(4)。
- **`is_japanese_ime` 固定**: 既定 true(`harness.rs:~218`)、`set_japanese_ime` で切替(使用 1 本)。本番は ADR-223 でキー入力時にレイアウト言語から更新。
- **読み戻し遅延(Q3)**: `set_readback_lag_ms`(状態変更から N ms の間、`read_state()` が変更前を返す)。使用 3 本。遅れの値を変えても通る分岐は同じ、と `pseudo_ime.rs` 自身が書いている。**非同期の完了(applied の往復)・SendMessageTimeout の時間切れ・IMM probe の二峰性(Q-imm-probe-bimodal)は無い**。
- **MS-IME の composition 中 OFF 無視(Q6)**: `set_off_ignored_while_composing` は**書き込み経路のみ**で、キー押下(格子)には効かない。**閉ループのシナリオからの呼び出しは 0 件**(`git grep set_off_ignored_while_composing crates/awase-windows/tests/closed_loop_scenarios.rs` = 0 件、`pseudo_ime.rs` の単体テスト 3 本だけ)。
- **外部クローズ**: `external_set_open`(awase の見ていない経路。入力中は捨てる)と `external_injected_key`(他プロセスの注入キー。belief は動かさず ADR-205 の監視窓を開く)。**TSF 窓で「読めない」こと**は `Imm32Unavailable` プロファイル(観測が belief へ届かない)で代用。
- **窓・スレッド**: IME は 1 つだけ(共有)。`focus_change()` は擬似 IME を共有のまま(実際はスレッドごと)。`focus_change_to_new_thread_unreadable` が擬似 IME を置き換える特例(E)。HWND→スレッド→IME の表は無い。
- **TSF の composition/literal**: `composing` は `stage != None` から作る真偽だけ。**romaji を送って literal になる/合成になる、の模擬は無い**(出力層を通らないので)。
- **格子の測定条件**: `msime.json` は `ime_key_matrix_spike`(素の EDIT、IMM32)で学習。ADR-209 の実測では**同じ MS-IME プリセットでも実 Chrome(TSF)は変換で開き、EDIT は開かない**。つまり格子は「IMM32 の窓」の真値で、TSF 窓の真値は別。窓の種類で真値が変わる次元が無い(`docs/adr/209-*.md` 背景)。

### 2.3 QUIRKS の modeled 状況(`pseudo_ime.rs` の `QUIRKS`)

| ID | 内容 | modeled | シナリオでの使用 |
|---|---|---|---|
| Q3 | モードキー通過後の IMM 再読 21〜62ms の遅れ | `set_readback_lag_ms` | 3 本 |
| Q4 | 実 Chrome×GJI の外部クローズが観測されない | `Setup::with_external_close_watch`(観測側は本物の `ExternalChangeWatch`) | 2 本(ADR-205 の有無で結果が分かれる) |
| Q6 | MS-IME の composition 中 OFF 無視 | `set_off_ignored_while_composing`(書き込みのみ) | **0 本**(単体 3 本のみ) |
| Q-ext-off-chrome-gji | Q4 の修正前の実測 | なし | — |
| Q-key-latency-gji | 物理キーから open 観測まで 250〜400ms、最大 2.3 秒 | なし | — |
| Q-imm-probe-bimodal | IMM probe の 50ms 境界の二峰性 | なし | — |

modeled なのは 3/6。実害に近いのは Q-key-latency(Q3 の 62ms では収まらない遅れ。BUG-162/163 系の「明示意図あり×古い読み」を作るのに要る、と `set_readback_lag_ms` の doc が書く)。

## 3. 足りない能力(撤去/統合の「挙動が変わらない」を Linux で言うために)

各項目: 必要性(どのクラスの機構の検証に要るか)/ 規模(PR 数・主要ファイル。行数見積りはしない)/ 既存部品の再利用 / 写しのリスク。

**ME-C1 出力層の記録と照合(送ったキー列)**
- 必要性: 機構の戦略・チェーン・gate・ledger・`fallback_write`・`apply_mechanism`・ImmCross の同期/非同期、`kp_shadow_actuate` と `dispatch_ime_set_open` の 2 入口の重複、drift correction の送信。つまり「何をどの順で送るか」が変わらないことを言う全ての機構。
- 到達できる粒度: **`MechanismCommand` のバリアント列まで**(`awase-ime-core/src/state/ime_actuation_decision.rs:132`: `SetOpenCrossProcessSync`/`...AsyncUntargeted`/`SetOpenThenConvForTarget`/`SendVk(vk)`)。`SendInput` の lparam・scan・ext フラグや `with_app` の再入は ADR-163 の TH1e が既に対象外としており、Linux では言えない(実機要)。
- 規模: ADR-241 PR-B(下の ME-2)。本番の核 `state/sync_actuation.rs` は PR-A で ungated(`CommandSink` `:48`、`apply_sync` `:216`、`dispatch_set_open` `:301`、`CoreSyncWriter` `:103`、単体テスト 13 本は FakeSink)。**ハーネスからの配線だけが残っている**。
- 再利用: `CommandSink` に `PseudoIme` を差す(`SendVk(vk)`→`press(vk)`、`SetOpenCrossProcessSync(b)`→`write_open(b)`)。
- 写しのリスク: 減る(A・D が消える)。残る写しは ADR-241 決定3 の表(押下 ID の付与、バッチの始めの applied、完了の返し)。非同期(`run_open_chain_async`・`fallback_write`)は段階 4 まで通らない。
- 限界: ProbeIo 側(romaji 送信・warmup の VK 送信)は別。`ProbeIo` の seam と `FakeProbeIo`(`output/probe_io.rs:24`/`:753`)は既にあるが、`output` モジュールが `#[cfg(windows)]` なので**テストは windows-build CI でしか走らない**(`lib.rs` の mod 一覧。`probe_io.rs` 20 本・`tsf_warmup_coord.rs` 15 本)。

**ME-C2 TSF 窓の composition/literal の模擬**
- 必要性: warmup・cold-start・LiteralDetect・probe・GjiFsm の warm/cold に関わる機構(再発ファミリー「warmup / cold-start」)。「F2 後 N ms で最初の文字が literal になるか」の等価性。
- 規模: 大。(1)擬似 TSF 窓(送られた VK/Unicode を受け、IME の warm 状態に応じて literal/composed/dropped を返し、composition を持つ)、(2)`output` の Win32 を剥がして `ProbeIo` を通す側を Linux から呼べる場所へ(`output/probe_io.rs` 1418 行・`tsf_warmup_coord.rs` 780 行は gated)、(3)時間依存(Chrome 再初期化 ~326ms 等、`tuning.rs` の実測定数)を仮想時計で駆動。少なくとも 2〜3 PR。
- 再利用: `tsf/warmup/*`・`tsf/gji_fsm.rs`(53 本)・`literal_detect_fsm`(14 本)・`probe_fsm`(19 本)は ungated モジュール(`tsf/mod.rs` の `cfg_attr(not(windows), allow(dead_code))`)で、**FSM 単体のテストは Linux で既に走る**。足りないのは「FSM を IME の応答で閉じる」こと。`warm_send_plan.rs`(`awase-windows-plans`)も純粋。
- 写しのリスク: **最大**。実機の応答(literal になる条件・遅延)はすべて仮定になり、等価性は「模擬に対して」しか言えない。実測(`tuning.rs`・BUG-002)を取り込める範囲に限らないと、模擬の誤りが撤去の誤判定になる。
- 判断: 先に作らない。warmup の撤去/統合候補が mech-b-tsf から出て、かつ FSM 単体テストで足りないと分かった後に、その候補に必要な応答だけを作る(4 の ME-6)。

**ME-C3 窓・フォーカス・スレッドのモデル**
- 必要性: focus 遷移の機構(`HwndCacheRestored`・`assume_closed_for_new_thread`・`reset_stale_ime_on_for_imm_broken`・フォーカス classify/resolve の優先順位・hwnd キャッシュ・「新スレッドは閉」)。BUG-204 のフォーカス移動の件がこれ。
- 規模: 1〜2 PR。ハーネスに `Desktop{windows: HwndId→{profile, thread, readable, tsf_native}, threads: Thread→PseudoIme}` を足し、`focus_change(to)` が `FocusChanged` と `focus_classify_plan`/`focus_resolve_plan`(純粋、`awase-windows-plans`)の結果を流す。
- 再利用: `focus/{classifier,cache,tracker}`(ungated)、plans の `*_plan`(Linux で単体テスト済み)、`PseudoIme::from_grid` をスレッドごとに複製。E の特例を一般化。
- 写しのリスク: 中。事実の収集(`runtime/focus_tracking.rs` 1037 行、gated)は写しになる。判断は plans を呼べば本物。

**ME-C4 仮想時計**
- 既にある(`HubClock::manual`、`advance_ms`)。足りないのは(1)Engine のタイマー(`on_timeout`、`TIMER_PENDING`)を駆動する側(B5 の `key_input_replay_tests.rs`〈613 行〉は自前のタイマーループを持つ。ハーネスは持たない)、(2)実機の遅延分布(ADR-241 決定6 が「作らない」と決定済み)。IME 機構の等価性には(1)は不要なので**足さない**。

**ME-C5 MS-IME 本体の格子・窓種別の次元**
- 必要性: MS-IME 系の機構(MsImeDirect、`predict_henkan_open_in_unreadable_windows`=ADR-209、Q6)の撤去/統合。
- 規模: 1 PR(格子を窓種別 {IMM32, TSF} でキー付けし、TSF 側のセルを CI 実測〈`sc-adr209-chrome-msime*`〉で埋める。実機要)+ 1 PR(Q6 をキー押下にも効かせ、シナリオから呼ぶ)。**測定が無いセルは panic のまま**(推測で埋めない方針を保つ)。
- 再利用: `Grid` の `file()` に窓種別を足す、`tools/e2e/ime_key_matrix/grid-tables/`。
- リスク: 測定の無い窓種別の格子を作らない限り写しは増えない。測定コストが主。

**ME-C6 journal 再生との接続(TH1e)**
- 現状: 決定点の再生は `awase-ime-core/src/state/actuation_decision_record.rs` の `mod tests`(`replay_record`)と凍結コーパス 1 件(`crates/awase-ime-core/tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`、37 レコード)。ほかに `tests/journals/{conv_classify(3件),drift_correction(1),ime_apply(1),key_input(2),read_strategy(1)}`。**TH1e(実際の撤去+差分ゼロ再生)は未達**で、ADR-241 §5 は既存の再生一式の撤去(約 730 行のテスト・Deserialize 一式・コーパス 1440 行)を決定済み(PR-C 未実施)。所有者は 2026-10-06 に complexity-budget を据え置きと判断(記憶メモ)。
- 判断: TH1e をこの形で進めない(再提案しない)。**前提が変わった点**: PR-A で核が `apply_sync` に 1 本化され `decide_attempt` の重複が消えたので、「差分ゼロ」の対象は記録ではなく**生成した入力の全列挙**で取れる(ME-3)。記録の入力(段階 5)は ADR-241 が「方向のみ」。
- ついでに: `complexity-budget.md` の「発効条件」節は `tests/journals/actuation_decision/bug-131-...json`(旧パス)と TH1e を前提に書かれており、コーパスは `awase-ime-core/tests/` へ移っている。文書の追随のみ(挙動は無関係)。

**ME-C7 純粋な判断の決定表の全列挙(新しい部品ではなく既存の方式の拡張)**
- 必要性: 撤去/統合が**純粋な判断(Facts→Plan)の分岐・関門の削減**であるクラス(例: drift の 2 重の `is_japanese_ime` 関門、`decide_gate` と `decide_chain`、`plan_set_open`、`warm_send_plan`、`relay_plan`、`deferred_gate_plan`、focus の `*_plan`)。実機要の機構(送信の副作用・タイミング)には効かない。
- 先例: `tests/explicit_press_exhaustive.rs`(120,960 状態 × 12 キーを全列挙し、反例のクラス別件数を `tests/golden/explicit_press_counterexamples.txt` に固定。`UPDATE_GOLDEN=1` で再生成)。撤去の前後で golden が不変なら「差分ゼロ」。
- 規模: 判断 1 系統につき 1 PR(Facts の値域を有限に切る + golden)。足場(golden 比較・再生成フラグ)は再利用。
- リスク: 値域の切り方が恣意的(未列挙の組が残る)。写しは増えない(本物の関数を呼ぶだけ)。変更したくない挙動の固定(特性テスト)なので、意図した変更では golden が変わる(それは正しい)。

## 4. 整備案(最小で最大の検証範囲が広がる順)

**ME-1(1 PR、新規の部品なし): 写し B・C・H を本物/実入力に置換**
- `settle()` の drift 分岐を `DriftFacts` を作って `decide_drift_plan` に通す(`ir_observe_drift_facts` の事実のうち、`engine_enabled`・`japanese_ime`・`drift`・`settling`・`active`・`default_policy`・`imm_cross`・`warrant_would_block` はハーネスが持つ値で作れる。`converged`/`fresh_evidence_after_giveup` は `ObservationStore::read_back` が要るので最初は偽固定で、写しとして列挙)。`PredictInput` の `unreadable`・`observable_toggle_guard`・half-width-alnum の `track` を `Setup` の項目にする(`kp_observable_toggle_guard` が純粋か未確認。純粋でなければ `Setup` の値で与える)。`closed_loop_scenarios.rs:18-22` の古い表を直す。
- 検証できる撤去/統合の例: (1)drift の `is_japanese_ime` の 2 重の関門(`decide_drift_plan::Idle(NotJapaneseIme)` と warrant の `ctx.is_japanese_ime` 棄却)。いまはアサートが区別しない(1.3-2)。(2)ADR-209 の `predict_henkan_open_in_unreadable_windows` 予測と ADR-260 の `observable_toggle_guard`(予測を出さない軸)の相互作用。
- 注意: 置換して結果が変わったら ADR-224 の規則どおり「写しのずれの実例」として記録して止める(これ自体が成果)。

**ME-2(PR-B、ADR-241 の続き。所有者承認済み 2026-10-07、未着手)**
- ハーネス・擬似 IME・シナリオを crate 内の `#[cfg(test)]` へ移し(`pub(crate)` を呼べる: `handle_engine_set_open` `platform_state.rs:921`・`on_ctrl_key_up` `:989`・`claim_press_write` `:267`・`follow_direct_read_in_scope` `:584`)、`press(RawKeyEvent)` で `plan_core` → `Engine::on_input` → `on_engine_set_open_request` → `dispatch_set_open`(sink = `PseudoIme`)を通す。`user_set_open` と `issue_write` の明示分岐と `handle_engine_decision` を消す(写し A・D・F の削除)。
- ADR-241 決定4 の合否基準(単体では落ちず打鍵からの再生でだけ落ちる変異が 1 つ以上)は**そのまま有効**。無ければ配線は取りやめ、核と単体テストだけ残る。この ME は基準の再検討を求めない。
- 検証できる撤去/統合の例: (1)`kp_shadow_actuate` と `dispatch_ime_set_open` の 2 入口が共有する「claim → applied の未知化 → imm_first」の骨格(段階 2 で核へ。`explicit_press.rs` の合成モデル 2056 行のうち `attempt_write`/`explicit_press_delivery_after` を核の呼び出しに置換できる可能性、ADR-241 は 0〜約 265 行・未確認とする)。(2)`apply_mechanism` 内の `decide_attempt` の再計算(PR-A で既に消えたが、`fallback_write` 側の 1 行は非同期経路のまま)。
- 副産物: `SendVk(vk)`→`press(vk)`(格子)で 2.2 の `write_open` の不整合が解消。`pub` 14 個(`PLATFORM_STATE_PUB_FNS`)を `pub(crate)` に戻せる(ADR-241 質問 3)。`ci.yml` の `--test closed_loop_scenarios`・`ci_test_coverage_guard`・`architecture_guard` の `harness.rs` 走査の付け替えが要る(未確認)。

**ME-3(判断 1 系統ごとに 1 PR): 決定表の全列挙 golden を、撤去/統合の候補の「前」に置く**
- 撤去/統合の PR は「候補の前に golden を足す PR → 候補の PR(golden 不変)」の 2 段。この順序は ADR-241 決定4 の「対照を先に固定」と同じ。
- 対象は mech-a〜d の候補が出た系統に絞る。既存で足場がある順: `decide_gate`/`decide_chain`/`decide_attempt`(`ime_actuation_decision.rs`、ME-2 の単体と重なる)、`decide_drift_plan`、`plan_set_open`、`warm_send_plan`、`relay_plan`、`deferred_gate_plan`。
- 検証できる撤去/統合の例: `InputRelay` の gate の 3 関数の独立した再検出(`decide_gate` を `run_open_chain_async`/`fallback_write`/`imm_cross_write` が再呼びするもの、ADR-180 INV-45)の整理は ME-2 が先(再入の挙動は列挙では言えない)。drift の `DriftStep` の枝(`Confirmed`/`GiveUp`/`SkipWarrantWouldBlock`)の統合は ME-3 で言える。

**ME-4(1〜2 PR、ME-2 の後): 「読み戻しで belief を揃える機構」を 1 つのハーネスに載せる**
- 通過マーク → `ModeKeyPassedThrough` → direct-follow(ADR-187/256、`platform_state.rs:584`、単体テスト 20 本以上が in-crate で既にある)、外部変化の監視窓(ADR-205、ハーネス済み)、`align_placeholder_desired`(BUG-163、ハーネス済み)、`ObserverPoll` の同期。ME-2 で `pub(crate)` に届くようになれば、`stale_eisu_...`(`:169`)の `#[ignore]` を外せる(偽の陽性の解消)。
- 検証できる撤去/統合の例: これらが同じ belief を揃える機構として重複していないか(mech-c の候補になりうる。**重複しているという主張はここではしない**。これら 3〜4 機構が持つ「実 IME を読んで belief を直す」入口を、同じシナリオで ON/OFF して差を見られるようにする、というだけ)。Q3 の読み戻し遅延と組み合わせて「明示意図あり×古い読み」(BUG-162/163)も書ける。

**ME-5(1〜2 PR): 窓・スレッド表(ME-C3)**
- focus 遷移系の機構が候補になったら。`Setup` に TsfNative/Plain/InputRelay の構築子を足す(いまは 0 件)のは ME-2 と同時にできる小さな追加。

**ME-6(条件付き。撤去候補が出てから): TSF の最小の応答の模擬(ME-C2)**
- 候補を先に mech-b-tsf で決める。FSM 単体で足りる候補(`probe_fsm`・`literal_detect_fsm` の分岐削除)にはこの模擬は要らない。要るのは「F2 の重複送信を 1 つ減らす」のような、IME の応答との閉ループで初めて意味を持つ変更だけ。そのとき作るのは、測定済みの 1〜2 の窓(Chrome×GJI の cold/warm、実測 ~326ms 等)に限った模擬。

**ME-7(小、独立): 擬似 IME の欠けたクセ**
- Q6 をキー押下にも効かせシナリオから使う(MS-IME 本体の機構を触るときに)。Q-key-latency(250〜2300ms)を `set_readback_lag_ms` の値の例として入れるだけでは新しい検知にならない(`pseudo_ime.rs` 自身の注記、PR #477 を閉じた経緯)。**「明示意図がある状態で古い読みが来る」シナリオと同時にだけ足す**。

## 5. 既に見送った/決定済みで、再提案しないもの

- ADR-159 段階 2(送信内容のログ出力、`shadow_send_trace`)の復活。TOML 宣言ランナー・Rhai・マクロ。報告 journal の自動 fixture 化・oracle 共有・遅延較正(ADR-225/226)。
- 案 A の ungate を先にやること(ADR-224。P4 で ungate 済みで、いま残るのは「`pub(crate)` と `pub` の境界」であり ME-2 の置き場所の問題)。`platform_state.rs`・`sync_actuation.rs` の core 移動(ADR-275 で見送り)。
- 遅延の確率モデル(ADR-241 決定6、ADR-226 候補 D)。
- TH1e をそのままの形で(記録の N 本)進めること。ME-3 の根拠が変わった点は ME-C6 に書いた(PR-A で核が 1 本化され、入力を生成できるようになった)。
- ME-1・ME-2 は新しい提案ではなく既存の ADR-224 P5/ADR-241 段階 1 の続き。ME-3 は ADR-208 L0 の方式の他の判断への適用で、新しい基盤は作らない。

## 6. 確認に使ったコマンド(版 `f3c2007e`)

```sh
# 写し・バイパスの根拠
git grep -n "on_input\|decide_drift_plan\|ModeKeyPass\|ForceGuard\|MechanismCommand\|plan_core\|dispatch_set_open" crates/awase-windows/tests/support crates/awase-windows/tests/closed_loop_scenarios.rs
#  -> doc コメント行のみ(harness.rs:26-28, closed_loop_scenarios.rs:19-22)。呼び出し 0 件
grep -n "unreadable: false\|observable_toggle_guard: false" crates/awase-windows/tests/support/harness.rs   # :313, :315
grep -n "set_off_ignored_while_composing" crates/awase-windows/tests/closed_loop_scenarios.rs               # 0 件
# 本番の核が既に ungated
grep -n "pub fn decide_drift_plan" crates/awase-ime-core/src/state/drift_plan.rs                            # :386
grep -n "pub(crate) fn dispatch_set_open\|pub(crate) fn apply_sync\|trait CommandSink" crates/awase-windows/src/state/sync_actuation.rs   # :301, :216, :48
# ハブの pub 14 個
sed -n 7734,7749p crates/awase-windows/tests/architecture_guard.rs
# ADR-241 の実施状況(PR-A のみ)
git log --oneline --grep="ADR-241" | head      # 2a178d5b (#552) 段階 1 PR-A
sed -n 123p docs/tasks/fcis-layering-tasks-2026-10-06.md   # 「ハーネスへの配線(決定3)と既存の再生一式の撤去は PR-B・PR-C」
# 凍結コーパスの所在
find crates -path '*journals/actuation_decision*'          # crates/awase-ime-core/tests/journals/actuation_decision/bug-131-*.json
```

未確認: (1)`kp_observable_toggle_guard` が純粋に切り出せるか(ME-1 の範囲を決める)。(2)ME-2 での `architecture_guard` の `harness.rs` 走査の付け替えの本数。(3)写しが原因の見逃しが git 履歴にあるか(網羅していない)。(4)ME-3 の各判断の Facts の値域の大きさ(列挙が現実的か)は判断ごとに数える必要がある。
