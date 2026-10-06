# W0-c 棚卸し: グローバル / static / thread_local / atomic の書き込み点

対象: `crates/awase-windows/src/` 配下。読み取りのみ(develop `efe5248e` の作業ツリーを読んだ。`origin/develop` は `ee54b2de`、本棚卸しの対象ファイルについて差分は未確認)。
参照: docs/adr/164-global-static-argument-threading-plan.md(2026-10-04 時点「ほぼ実装済み」)、docs/tasks/layering-inventory-2026-10-06/inventory-a1.md(HOOK_STATE の 23 フィールド表)。

分類記号: B=belief, I=intent/設定, O=observation, G=guard(失効窓・ラッチ・世代), C=cache, K=counter/診断, X=infrastructure。
書き込み経路: R=reduce, M=メソッド直書き, G=グローバル/atomic, H=フックスレッド。**本領域のグローバルは R に該当するものが 0 件**(下の集計参照)。

---

## 0. 数え方と全体像

- `rg -n '^\s*(pub(\([a-z: ]+\))?\s+)?static [A-Z_0-9]+\s*:' crates/awase-windows/src` = **58 件**(関数ローカル `static` を含む。うちテスト専用 4 件: `TSF_OBS_TEST_LOCK`、`OUTPUT_GATE_TEST_LOCK`、`focus/classifier.rs:557 COUNTER`、`runtime/transport.rs:1027 CACHE`)。ADR-164 起票時(2026-09-10)は 76 件。
- `thread_local!` ブロック 4 つ・セル 6 個: `hook.rs`(`OWN_HOOK_HANDLE`、`MY_HOOK_GEN`)、`imm.rs:179`(`PROBE_TIMED_OUT`)、`ime_diagnostic.rs:33`(`TSF_PROBE_SNAP`)、`win32.rs:288`(`SENT_INPUT_TRACE`、`SENT_INPUT_STAMP_SOURCE`)。
- **不変・write-once で状態ではないもの(対象外、15 件)**: `IMM_STRATEGY`/`GJI_STRATEGY`/`MS_IME_STRATEGY`(ime_controller.rs:147-149、ZST ディスパッチ)、`IMM_CROSS_DRIVER`/`IMM32_UNAVAILABLE_DRIVER`/`TSF_NATIVE_DRIVER`(state/ime_profile_driver.rs:186-188)、`vk.rs:306 CANDIDATES`、`tsf/output.rs:147 TABLE`(かな表)、`hook.rs:976 CACHE`(レジストリのフックタイムアウト)、`hook.rs:1375 ENABLED`(環境変数)、`hook.rs:1894 BASELINE`(`Instant`)、`tsf/tip_detector.rs:37 GJI_CLSID`(OnceLock)、`tsf/tip_detector.rs:38 PROFILE_DESCRIPTIONS`(RwLock、列挙結果のキャッシュ)、`app/logging.rs:178 LOG_WRITER_STATE`、`hook.rs:1134 HOOK_JOIN_LEAKED_THREADS`。
- ADR-164 との差分: ADR が「集約済み」とした `HOOK_STATE`(struct-of-atomics)・`PROBE_FENCE`・`PROCESS_FLAGS`・`REJECTION_COUNTERS` は現状どおり存在する。**ADR-164 は「裸 static を struct に束ねる」ことが目的で、「状態を reduce に通す」ことは対象外**。束ねられても書き手は依然としてグローバルへの直接 store である(下の分類で全て G または H)。

### 0.1 スレッド一覧(書き手の表記)
- **HK**=フックスレッド(`hook.rs::install_hook` が起動、`hook_callback`)。
- **MT**=メインスレッド(メッセージループ、`with_app`、`WinEventProc` ※`WINEVENT_OUTOFCONTEXT` でメッセージループスレッドで動く、`tsf/win_event_obs.rs:101-103`、設置は `app/bootstrap.rs:1296`)。
- **GM**=`gji-io-monitor` ワーカー(`tsf/gji_monitor.rs:331`、`win32_worker::WorkerThread::spawn`)。
- **WK**=`run_with_timeout`/`offload_unsafe` のワーカー(IMM32/MSAA/UIA、`send_ime_control_raw` など)。
- **CT**=Ctrl+C ハンドラのスレッド(OS 生成)。

---

## 1. 保持者ごとの表

凡例: 「journal」= その書き込みに対応する journal/event_log のレコードがあり、再生で同じ状態になるか。

### 1.1 `HOOK_STATE`(hook.rs:176、23 フィールド。per-field の書き手/読み手/畳み込み可否は inventory-a1.md §1.2 に既出。ここでは分類と集計のみ)

| フィールド | 分類 | 書き手(スレッド) | 経路 | journal | スライス案 |
|---|---|---|---|---|---|
| `ime_mode_diagnostics`(Mutex<VecDeque>、上限 64) | K | HK(`push_hook_ime_mode_diagnostic` hook.rs:1394) | H | 一部(MT が `drain_*` で journal へ移す) | diag(journal 側) |
| `last_ime_mode_hook_ms` | K | HK(`swap`) | H | No | diag |
| `cached_thumb_vks` | C(設定ミラー) | MT(`set_thumb_vk_codes` hook.rs:687) | G | No(設定変更は journal に載らない=未確認でなく、`set_*` 内に journal 呼び出しなし) | config |
| `hook_alive_tick_ms` | K(生存信号。O 的) | HK(`tick_hook_alive` hook.rs:305、全コールバック先頭)、MT(bootstrap 初期化) | H | No | infra(畳み込み不可。検出対象の故障と同じ経路に載せられない) |
| `hook_tid_init_slot` | X | HK/MT(起動ハンドシェイク) | G | No | infra |
| `physical_key_state[256]` | O | HK(hook_callback e ブロック hook.rs:1538 付近)、MT(`reset_physical_key_state` hook.rs:455、`clear_hook_latches_*` の Ctrl/Shift 枠) | H+G(Mixed) | 一部(`KeyInput` に `modifier_snapshot` は載るが、表そのものの再生は No) | observation(物理キー表) |
| `physical_key_down_at_ms[256]` | O | 同上 | Mixed | No | observation |
| `physical_down_vk_by_identity[512]` | O | 同上 | Mixed | No | observation |
| `left/right_thumb_down_at_us` | G(ラッチ。O 的) | HK(j ブロック)、MT(`set_thumb_vk_codes` と 3 つのリセット関数) | Mixed | 一部(`RawKeyEvent` のスナップショットとして journal に載る) | guard |
| `left/right_thumb_down_scan` | G | 同上 | Mixed | No | guard |
| `ctrl_consumed_since_down` | G | HK(k ブロック)、MT(`clear_hook_latches_*`) | Mixed | No | guard(MT がライブ読み: runtime/key_pipeline.rs:192) |
| `cached_keyboard_model_is_us`/`cached_left_alt_impersonation_enabled`/`cached_right_alt_impersonation_enabled`/`cached_swallow_alt_kana_mode_switch` | C(設定ミラー) | MT(`set_keyboard_model`/`set_alt_impersonation_enabled`/`set_swallow_alt_kana_mode_switch`、bootstrap.rs:709-712) | G | No | config |
| `cached_engine_enabled` | **B/I のミラー**(engine スライスの写し) | MT(`set_engine_enabled` hook.rs:730、runtime/executor.rs `UiEffect::EngineStateChanged`) | G | 一部(EngineStateChanged は effect。値の journal 記録は未確認) | engine(の読み取りミラー) |
| `focus_app_disabled` | **B のミラー**(focus スライスの写し) | MT(`set_focus_app_disabled` hook.rs:738、runtime/focus_tracking.rs) | G | No | focus(の読み取りミラー) |
| `alt_l/r_impersonating`・`alt_l/r_was_down`(4) | G | HK(`apply_alt_impersonation` hook.rs:218)、MT(リセット 3 関数) | Mixed | No | guard |

HOOK_STATE の集計(23): 分類は O=3、G=9(thumb 時刻 2・thumb scan 2・ctrl_consumed 1・alt 系 4)、C=5(`cached_thumb_vks`+設定ミラー 4)、世界モデルのミラー=2(`cached_engine_enabled`、`focus_app_disabled`)、K=3(`ime_mode_diagnostics`、`last_ime_mode_hook_ms`、`hook_alive_tick_ms`)、X=1(`hook_tid_init_slot`)。
書き込み経路(書き手の表から数えた): HK のみ=`ime_mode_diagnostics`、`last_ime_mode_hook_ms`(2。`hook_alive_tick_ms` は bootstrap の初期化でも MT が書くので Mixed)、MT のみ=設定ミラー 5(`cached_thumb_vks`、keyboard_model、alt 有効 2、swallow)+`cached_engine_enabled`+`focus_app_disabled`(7)、Mixed(HK と MT)=残り 14(`hook_alive_tick_ms`、`hook_tid_init_slot`、物理キー表 3、親指 4、ctrl_consumed、alt 系 4)。2+7+14=23。**R=0**。

**書き込み関数の数**: `HOOK_STATE.` の出現は hook.rs 内のみ(hook.rs 外からの直接アクセスは grep で 0。外部は `hook::*` アクセサ経由)。リセット系 3 関数(`reset_physical_key_state` hook.rs:455、`clear_hook_latches_for_app_disable` hook.rs:496、`clear_hook_latches_for_watchdog_reinstall` hook.rs:572)が**意図的な重複**を持つ(architecture_guard が関数本体を文字列走査するため共有化不可、doc に明記)。

### 1.2 フックスレッド → メインの伝送(`hook_channel.rs`)

| 保持者 | 分類 | 書き手 | 経路 | 読み手 | journal | スライス案 |
|---|---|---|---|---|---|---|
| `HOOK_KEYS`(SPSC リング CAP=1024、`head`/`tail`/`overflow_state`/`max_occupancy`) | X(伝送)。ただしリング内容は I(生の物理入力イベント) | HK(`produce`)、MT(`consume_all`、`take_dropped_and_clear_latch`) | H | MT の `handle_hook_key_event`(app/mod.rs:625) | **Yes**(`KeyInput` として journal に載る) | **Event の入口そのもの**(Redux の dispatch 口) |
| `HOOK_KEYS.overflow_state` の latch bit | G | HK(`produce` が満杯で立てる)、MT(`take_dropped_and_clear_latch` で解除) | Mixed | HK(`is_overflow_latched`、hook.rs h ブロック) | dropped 数は journal に出る(`take_dropped_and_clear_latch` 戻り値)。latch 自体は未確認 | guard |
| `WAKE_PENDING`(AtomicBool) | X | HK/MT(`request_engine_wake` の重複 post 抑止、`recover_stuck_wake_if_needed` が MT 側で解除) | Mixed | HK/MT | No | infra |
| `WAKE_POST_FAILED`(AtomicBool)、`WAKE_POST_FAILED_LIFETIME_COUNT` | K | HK(post 失敗時) | H | MT(watchdog、不具合報告) | No | diag |

### 1.3 `TSF_OBS`(tsf/observer.rs:455、`TsfObservations`、フィールド 22)

アクセス制御: `pub(in crate::tsf)`、tsf 外は `tsf_obs()`(observer.rs:487)・名前付き関数経由。**書き手は GM(gji_monitor.rs)と MT の WinEventProc(win_event_obs.rs)の 2 系統のみ**で、読み手は §下に列挙。

| フィールド | 分類 | 書き手(スレッド・関数) | 経路 | 読み手 | journal | スライス案 |
|---|---|---|---|---|---|---|
| `gji_last_io_ms` | **O** | GM(gji_monitor.rs:426-432 付近) | G | `gji_idle_ms`(→ probe の long-idle 判定、`ObservedState.gji_last_io_ms`)、runtime/ime_refresh | **No**(`ObservedState` は `from_snapshot` でその場スナップショット。journal.rs/journal_policy.rs に `tsf_obs`/`gji_last_io` の参照 0 件) | observation |
| `gji_attach_ms` | O | GM | G | `gji_io_is_attach_artifact`(BUG-176) | No | observation |
| `gji_monitor_ok` | O | GM(432/457/469 行)、test(chrome_probe.rs:92) | G | `active_ime_kind()`(tsf_active_kind 未取得時の派生) | No | observation |
| `gji_write_bytes` | O | GM | G | `LiteralDetector`(composition 確認の閾値判定 = 判定ロジックで使用) | No | observation |
| `gji_last_write_ms` | O | GM、MT(win_event_obs.rs:165/188 は**読み取り**) | G | literal_detect_fsm の StaleConfirm 判定 | No | observation |
| `gji_write_ops`/`gji_read_ops`/`gji_other_ops`/`gji_other_bytes` | K(診断専用、doc で「判定には使わない」と明記) | GM | G | journal 診断のみ | 一部 | diag |
| `gji_candidate_visible` | O | MT(WinEventProc SHOW=true/HIDE=false、win_event_obs.rs:157) | G | literal_detect_fsm(veto)、`ObservedState` | No | observation |
| `candidate_was_seen`(ラッチ) | **G**(観測由来のラッチ) | **MT の WinEventProc が立て(win_event_obs.rs:158)、MT の 3 経路が下ろす**(`platform.rs:1144`、`ime_controller.rs:268`、`runtime/focus_tracking.rs:627`) | G | `ObservedState.candidate_was_seen`(`state/ime_decision_view.rs:87`)→ `gji_direct_already_matches`(state/ime_actuation_decision.rs:227)=**送信を省略してよいかの判定入力** | **No**(`ActuationDecisionRecord` は `candidate_was_seen` を持つが決定時の入力コピーのみで、ラッチ自体の遷移は記録されない=未確認でなくコード上 `state/actuation_decision_record.rs:614` は false 既定) | guard(世界モデルの一部) |
| `literal_session_confirmed_gen`(世代) | **G**(失効窓。Generation 比較で自動失効) | MT(`output/probe_io.rs:657` mark、`platform.rs:824` と probe 系が reset) | G | `output/mod.rs:982` → warmup コルーチン(literal-detect のスキップ判定) | No | guard(warmup スライス) |
| `pending_start_composition`/`pending_end_composition` | X(WinEventProc → platform の**メールボックス**) | MT WinEventProc(win_event_obs.rs:160/184) | G | MT `platform.rs:833/836` が `swap(false)` で drain し `GjiFsm` へ `StartComposition`/`EndComposition` を dispatch | No | **Event 化の第一候補**(後述) |
| `ime_composition_active` | **O** → `InputContext.composing` 経由でエンジンの判断入力 | MT WinEventProc(IME_SHOW/HIDE、win_event_obs.rs:203/211) | G | key_pipeline.rs:98/122/1776(`ime_composition_active_now()`)、output/mod.rs:654、open_chain.rs:299 | No | observation |
| `tsf_active_kind`(u8)、`ms_ime_native_identified`、`ime_product_name`(RwLock<Option<String>>) | **B 的(IME 種別の同定)**。観測だが、`table_ime_kind()` が打鍵時予測表の選択を決める(key_pipeline.rs:973 ほか)ため、belief の更新を間接的に動かす | GM(`TipIdentityDebounce` で 2 tick デバウンス後に `set_tsf_active_kind`/`set_ms_ime_native_identified`、gji_monitor.rs:359-446)、`set_ime_product_name`(書き手の呼び出し元は未精査) | G | key_pipeline.rs(215/1113/1198/1495/1949/2185)、focus_tracking.rs(211/739)、focus/classifier.rs | **No**(IME 同定は journal から再生できない) | **B/observation の境界**。同定結果は `ImeKindId`/`TipIdentity` という純粋型があり、reduce に載せる素地はある |
| `focus_namechange`/`gji_candidate_show`/`ime_show_seq`/`ime_change_seq`(`ChangeCounter`、`notify`/`baseline`/`has_changed`) | K(世代つきカウンタ。`has_changed(baseline)` で「基準以後に発火したか」を判定=G 的に使われる) | MT WinEventProc | G | `ObservedState.ime_show_seq`/`ime_change_seq`、probe の確認判定 | No | observation |

TSF_OBS の集計(22 フィールド。`ChangeCounter` 4 を 4 と数える): O=7(`gji_last_io_ms`、`gji_attach_ms`、`gji_monitor_ok`、`gji_write_bytes`、`gji_last_write_ms`、`gji_candidate_visible`、`ime_composition_active`)、B 的(IME 同定)=3(`tsf_active_kind`、`ms_ime_native_identified`、`ime_product_name`)、G=2(`candidate_was_seen`、`literal_session_confirmed_gen`)、X=2(`pending_*`)、K=8(診断 4 + ChangeCounter 4)。合計 22。書き込み経路は**全て G**(R=0)。書き手は MT(WinEventProc 等)と GM のみ=HK は書かない。

**「世界モデルの一部なのにグローバルが持っている」ものの代表**: `candidate_was_seen`(送信省略の判定に使う観測ラッチ。ラッチの遷移が journal に無い)、`tsf_active_kind`/`ms_ime_native_identified`(IME 同定、belief の更新経路を変える)、`ime_composition_active`(エンジン判断の入力)、`literal_session_confirmed_gen`(warmup の失効窓)。

### 1.4 出力ゲートと入力保留(`OUTPUT_GATE` / `OutputActiveGuard` / `INPUT_DEFER`)

| 保持者 | 分類 | 書き手 | 経路 | 読み手 | journal | スライス案 |
|---|---|---|---|---|---|---|
| `OUTPUT_GATE.active`(AtomicBool) | **G**(再入防止ガード。**保持状態が world の一部**) | MT: `OutputActiveGuard::begin()`(depth 0→1 で true、tsf/probe_bridge.rs:113)と `Drop`(1→0 で false + `post_drain_output_queue`)。`begin()` の呼び出しは `output/vk_send.rs:296/811`、`output/tsf_warmup_coord.rs:159`(GJI probe 中ガードを RefCell に保持)、`output/mod.rs`(OutputSession)。**HK からは書かない/読まない**(hook.rs に `OUTPUT_GATE` 参照 0)。`INPUT_DEFER` の doc に「フックから呼ぶ」とあるのは古い記述(実際の呼び出し元は app/mod.rs:654=MT) | G | app/mod.rs:645(取り込み口で defer 判定)、key_pipeline.rs:140/613、message_handlers.rs:569/689 | **No**(ガードの開閉は journal に出ない。ただし defer された打鍵は `KeyInput` として既に記録済みのため、再生ではキューを再現せず入力順が違う可能性=未確認) | guard |
| `OUTPUT_GATE.depth`(AtomicU32) | G(参照カウント) | MT(`begin`/`Drop`) | G | `OutputActiveGuard` 内のみ | No | guard |
| `OUTPUT_GATE.last_vk_output_ms` | K(時刻の記録) | MT(`output/mod.rs:679` の `mark_vk_output`) | G | MT `runtime/ime_refresh.rs:424` | No | diag/guard |
| `INPUT_DEFER`(Mutex<VecDeque<RawKeyEvent>>、cap 1024)+`overflow_count` | **G+X**(保留中の入力イベント列。順序が意味を持つ=world の一部) | MT: app/mod.rs:654/662/666、message_handlers.rs:523-526/1763/1810、runtime/mod.rs:1235、`take_all`(message_handlers.rs:1763) | G(Mutex 経由) | MT のみ(`pending_len_nonblocking` は key_pipeline.rs:139、probe_bridge.rs:131) | **No**(`RawKeyEvent` は到着時に journal 済みだが、defer→replay の順序と drain タイミングは記録されない) | **intent/engine スライスの「保留入力キュー」**。Mutex は MT 単独使用なので不要に見えるが、`OutputActiveGuard::drop` が WK から呼ばれうる可能性は未確認 |
| `FOCUS_RESYNC`(`armed`/`gate_active`/`generation`/`armed_at_ms`) | **G**(focus 遷移後の resync ゲート。generation で失効) | MT: `arm`(runtime/focus_tracking.rs:888)、`consume_and_close`(app/mod.rs:647)、`open_if_current`(key_pipeline.rs:609、message_handlers.rs:563)、`disarm` | G | app/mod.rs:644、message_handlers.rs:689(`is_gate_active`) | No | guard(focus スライス) |
| `DRAIN_PENDING`/`DRAIN_RERUN_PENDING`(message_handlers.rs:34-35) | X(drain 再入抑止) | MT | G | MT | No | infra |

集計(§1.4、フィールド 11): G=7(`OUTPUT_GATE.active`/`depth`、`INPUT_DEFER` 本体、`FOCUS_RESYNC` の 4 つのうち実質 1 保持者として数えず個別に 4 とすると G=10)、K=1、X=2。**R=0**。

### 1.5 世代・フェンス・カウンタ(失効判定に使われるもの)

| 保持者 | 分類 | 書き手(スレッド) | 経路 | 読み手 | journal | 備考 |
|---|---|---|---|---|---|---|
| `CONV_MUTATION_SEQ`(conv_mutation.rs:33) | **G**(失効トークン) | `bump()`: `win32::send_input_safe`(win32.rs:352、conv を変えうる VK)と `imm::send_ime_control_raw`(imm.rs:229、IMC_SETCONVERSIONMODE)。**スレッド不問(MT と WK)** | G | `current()` を `spawn` 時に取り、`apply` 時に再比較: runtime/key_pipeline.rs:516/700 | No | await をまたぐ失効判定(危険箇所 D1) |
| `PROBE_FENCE.fence_value`(probe_actuation_fence.rs:152) | **G**(同上) | `bump()`: win32.rs:360(`ime_actuation_marker_kind` が Some の送信)、imm.rs:240(probe 系以外の cmd)。MT と WK | G | `current()`: ime.rs:793/797(issue 直前の再比較、MT/WK 両方)、output/probe_io.rs:245/259、platform.rs:657/672、key_pipeline.rs:521/726 | No | ADR-140 Step1 決定B。**3 点の独立チェックポイントで比較**(spawn・issue main/worker・apply) |
| `PROBE_FENCE.abandoned_*`/`spawned_*`(LifetimeCounter 4) | K | MT(key_pipeline.rs:525/540)、WK | G | 不具合報告スナップショット | 診断のみ | |
| `SEND_HEALTH`(`last_elapsed_ms`/`consecutive_slow`/`last_slow_at_ms`) | **G**(サーキットブレーカ。**actuation 可否を決める状態**) | `send_health::record`(imm.rs:290)= `send_ime_control_raw` から**MT と WK 両方** | G | `blocking_allowed`: key_pipeline.rs:2792、focus_tracking.rs:882(`arm` の可否) | 不具合報告スナップショットのみ(message_handlers.rs:1340-1342)。判定入力としての再生は No | **ブレーカ状態が world の一部**で、record(IMM 呼び出しの実測 ms)は時刻を `current_tick_ms()` で直接読む |
| `LAST_ACTUATION_ISSUE_US`(win32.rs:251) | K(診断) | `send_input_safe`(任意スレッド) | G | HK(hook.rs:1483、診断ログ) | No | 唯一「MT/WK が書き HK が読む」診断値 |
| `NEXT_PRESS_ID`(hook.rs:1309)、`HOOK_GEN`(hook.rs:1066)、`OWN_HOOK_HANDLE`/`MY_HOOK_GEN`(thread_local) | `NEXT_PRESS_ID`=K/ID 採番(I 的)、`HOOK_GEN`・`MY_HOOK_GEN`=G(旧フックスレッド排除)、`OWN_HOOK_HANDLE`=X | HK/MT(`install_hook`) | G/H | HK | `press_id` は `RawKeyEvent` 経由で journal に載る=Yes | |
| `REJECTION_COUNTERS`(state/probe_admission.rs:71) | K | MT/WK(probe 棄却時) | G | drain して journal/診断へ | 診断のみ | |
| `ACTUATION_DECISION_RECORD_SKIPPED`(runtime/open_chain.rs:72) | K | MT | G | MT | 診断のみ | |

### 1.6 プロセス全体フラグ・ハンドル類

| 保持者 | 分類 | 書き手 | 経路 | 読み手 | journal | 備考 |
|---|---|---|---|---|---|---|
| `PROCESS_FLAGS.main_thread_id`/`quit_requested`/`elevated`(lib.rs:149) | X(+`elevated`は O) | MT(起動時)、CT(`request_quit`) | G | 全スレッド | No | CT がクロススレッドで書くので atomic 必須。reduce 対象外が妥当 |
| `ENGINE_HWND`/`MODAL_DEPTH`/`NEEDS_ENGINE_RESYNC`(runtime/engine_window.rs:14-16) | X(+`NEEDS_ENGINE_RESYNC`は G) | MT(`ModalPumpGuard::enter`/`Drop`、`engine_wnd_proc`) | G | MT、WK(`win32::post_to_main_thread` 経由で `engine_hwnd()`) | No | `engine_wnd_proc`(WNDPROC 固定署名)とモーダルポンプの再入から使う |
| `MENU_TARGET_HWND`(tray.rs:35) | C(捕捉したフォーカス HWND) | MT(tray.rs:591) | G | MT(`handle_wm_command`) | No | ADR-164 round2 で対象外確定 |
| `LAST_FOCUS_HWND`(app/bootstrap.rs:857、関数ローカル) | G(連続 FOCUS の重複排除) | MT(`win_event_proc`) | G | 同関数 | No | |
| `TASKBAR_CREATED_MSG`(app/mod.rs:55) | X | MT(起動時 1 回) | G | MT | No | |
| `LAST_BALLOON_WARNINGS`(app/mod.rs:89、Mutex<Vec<String>>) | G(同一バルーンの再表示抑止) | MT | G | MT | No | |
| `DUMP_TRIGGER`(app/mod.rs:43、SingleThreadCell) | G/K(journal ダンプ起動のコード列追跡) | MT(`handle_hook_key_event` app/mod.rs:637) | G | MT | No | |
| `RAPID_IME_TIMESTAMPS`(`RapidPressTracker`)、`PANIC_TRIGGER_COMBOS`(panic_detect.rs:12/19、SingleThreadCell) | G(パニック検出のスライディング窓)/I(設定) | MT(`record_ime_keydown` app/mod.rs:636、`set_panic_trigger_combos`) | G | MT | No | `record_ime_keydown` が窓成立で `post_to_main_thread(WM_PANIC_RESET)`=PanicReset イベント発行側。**ここは既に「内部状態の更新→自己 post」で擬似的に Event 化されている**(受け側が reduce へ `PanicReset` を投入するかは W0-a/b 領域) |
| `INPUT_RELAY_APPS`(focus/classifier.rs:28、OnceLock<RwLock<Vec<String>>>) | I(設定の複製=C) | MT: `ForceOverrides::new`(classifier.rs:133、設定再読込のたびに上書き) | G(RwLock) | MT・WK(`ime.rs:867 read_ime_state_fast` が `self` なしで読む) | No | CLAUDE.md が認める唯一の RwLock 例外。**`FocusTracker::input_relay_apps()` と二重に持つ**(同期は `ForceOverrides::new` に依存) |
| `RAW_TSF_LITERAL`(lib.rs:225、`backs`/`romaji`(Mutex<String>)/`escape_composition`) | **G+X(リカバリペイロードの受け渡し箱)** | MT: `Output::record_raw_tsf_literal`(output/mod.rs:1215)が書き、`take`/`flush_*`(output/mod.rs:1237、tsf/output.rs:163)が読む | G | 同上(MT のみ) | `journal.rs:180` に「破棄した `RAW_TSF_LITERAL` の中身」のレコードあり(破棄時のみ)。書き込み自体は No | **MT 内の関数間受け渡しにグローバルを使っている**=引数/戻り値(Cmd)で渡せる。ADR-164 は「既に原則を満たす」として据え置き |
| `RUNTIME`(lib.rs:229、`SingleThreadCell<Runtime>`) | X(コンテナ。中身は別領域) | MT(`with_app`)のみ | G(RefCell の `try_borrow_mut`) | `with_app(`/`with_app_ref(` の出現 77 行(src 全体) | — | 再入時は `None` を返し `with_app_or_repost` で自スレッドへ再 post(lib.rs:252)=**再入で Event が失われないための補償機構**。WinEventProc/WK 完了コールバックが `&mut Runtime` を取れないことが、TSF_OBS 等を atomic にしている根本原因 |

### 1.7 thread_local(6 セル)

| セル | 分類 | 書き手/読み手 | 備考 |
|---|---|---|---|
| `OWN_HOOK_HANDLE`、`MY_HOOK_GEN`(hook.rs:1019/1027) | X/G | HK のみ | フックスレッド専有 |
| `PROBE_TIMED_OUT`(imm.rs:182) | K(同一スレッド内の戻り値代わり) | 呼び出しスレッド(MT か WK) | `read_ime_state_full` 開始で reset→`ime_on` が読めなければ `ImeSnapshot::probe_timed_out` へ載せる=**戻り値で運べるものを thread_local で運んでいる**(難度低) |
| `TSF_PROBE_SNAP`(ime_diagnostic.rs:34) | X(`RUNTIME` 排他借用中に診断スナップショットを渡す回避策) | MT(`set_tsf_probe_snap`/`clear_*`) | `with_app_ref` が BorrowError で None を返すための回避(コメントに明記)。引数渡しで消せる(難度低、呼び出し木は handle_wm_timer→advance_tsf_probe→log_composition_probe) |
| `SENT_INPUT_TRACE`、`SENT_INPUT_STAMP_SOURCE`(win32.rs:296/301) | K(送信記録。journal の `SentInput` 源) | `send_input_safe` を呼んだスレッド(MT ぶんだけ `drain_journal_entries` が journal へ移す。WK ぶんは上限 512 で捨てられる) | **送信(Cmd の実行結果)の記録で、journal に載る唯一の送信側記録**。WK からの送信は記録が失われる(設計上の穴、BUG としては未確認) |

---

## 2. 集計

### 2.1 書き込み経路別(保持者=フィールド単位、不変・write-once・テスト専用を除く)

| 保持者群 | フィールド数 | R | M | G | H | Mixed(HK+MT) |
|---|---|---|---|---|---|---|
| HOOK_STATE | 23 | 0 | 0 | 7(MT のみ) | 2(HK のみ) | 14 |
| HOOK_KEYS/WAKE_* | 6(`head`/`tail`/`overflow_state`/`max_occupancy` は 1 保持者として束ねず 4+WAKE 3 = 7 が正確) | 0 | 0 | 2 | 3 | 2 |
| TSF_OBS | 22 | 0 | 0 | 22 | 0 | 0(MT と GM の別スレッドだが各フィールドの書き手は 1 系統) |
| OUTPUT_GATE+INPUT_DEFER+FOCUS_RESYNC+DRAIN | 11+2 | 0 | 0 | 13 | 0 | 0 |
| 失効トークン・フェンス・カウンタ(§1.5) | 約 16 | 0 | 0 | 約 16 | 0 | 0(ただし MT と WK が同じ値を書く) |
| プロセスフラグ・ハンドル類(§1.6) | 約 20 | 0 | 0 | 約 20 | 0 | 0 |

(「約」は §1.5/§1.6 でフィールド数を厳密に数えていないため。確定値は HOOK_STATE=23、TSF_OBS=22、OUTPUT_GATE=3、FOCUS_RESYNC=4、INPUT_DEFER=2 のみ。)

**結論: 本領域の全グローバルについて、書き込みが `ImeModel::reduce`(または他の reduce)を通るものは 0 件**。全て「その場で atomic/Mutex/RefCell へ store」で、書き込みに対応する Event variant は存在しない。ただし書き込みの**結果**(例: `tsf_active_kind` が変わった)を後から MT が読んで `ImeEvent` に変換して reduce へ入れる経路は存在し得る(`ObservedState::from_snapshot` → 判断関数 → 結果が belief へ)。その経路は W0-a/b の担当。

### 2.2 意味分類別(確定できる範囲)

| 分類 | HOOK_STATE | TSF_OBS | その他(§1.2〜1.6) |
|---|---|---|---|
| B/ミラー | 2 | 3(IME 同定) | 0 |
| O | 3 | 7 | `elevated` ほか少数 |
| G | 9 | 2 | 約 12(`OUTPUT_GATE` 2、`INPUT_DEFER` 1、`FOCUS_RESYNC` 1、`CONV_MUTATION_SEQ`、`PROBE_FENCE.fence_value`、`SEND_HEALTH`、`HOOK_GEN`、`LAST_FOCUS_HWND`、`LAST_BALLOON_WARNINGS`、`RAPID_IME_TIMESTAMPS`) |
| C | 5 | 0 | `MENU_TARGET_HWND`、`INPUT_RELAY_APPS`、`PANIC_TRIGGER_COMBOS` |
| K | 3 | 8 | 約 10(各種カウンタ、診断) |
| X | 1 | 2 | 約 8(`HOOK_KEYS`、`WAKE_PENDING`、`PROCESS_FLAGS`、`ENGINE_*`、`DRAIN_*`、`RUNTIME`) |
| I(設定の複製) | 0 | 0 | `RAW_TSF_LITERAL` は受け渡し箱のため X |

---

## 3. reduce を通らない書き込み(M・G・H)を「Event に直せるか」で分類

### 3.1 難度 低(同一スレッド内の受け渡しで、引数・戻り値・Cmd に直せる)
| 対象 | 理由 |
|---|---|
| `RAW_TSF_LITERAL`(MT 内 `record_raw_tsf_literal`→`flush_*`) | 書き手と読み手が同じ `Output` の 2 メソッドで、間に `WM_DRAIN_OUTPUT_QUEUE` を挟むだけ。`Cmd`(RawTsfRecovery{backs, romaji, escape})として返せばグローバル不要 |
| `PROBE_TIMED_OUT`(thread_local) | 戻り値の追加で足りる |
| `TSF_PROBE_SNAP`(thread_local) | 呼び出し木を辿って引数で渡せる |
| `pending_start_composition`/`pending_end_composition` | WinEventProc(MT)が set し、platform(MT)が drain して `GjiFsm` に dispatch するだけの**メールボックス**。`post_to_main_thread(WM_*)` で Event として運べば atomic を 2 つ消せる(ただし WinEventProc 内では `with_app` が取れないことがある=再入。post なら失われない) |
| `candidate_was_seen` の 3 箇所の reset | 全て MT。リセット契機が `ImeEvent` に既にあるか、新設すればよい |
| `INPUT_RELAY_APPS`(二重保持) | `ForceOverrides::new` の副作用での複製。`read_ime_state_fast` の `self` なし制約が原因で、呼び出し木を変えれば消せるが ADR-164 が「変更対象外」と決定済み |

### 3.2 難度 中(別スレッド書き手が居るが、順序と時刻を Event に載せれば reduce に入る)
| 対象 | 理由 |
|---|---|
| `TSF_OBS` の GM 書き手 7 フィールド(`gji_last_io_ms` ほか) | GM は 10ms ごとのサンプリング値。reduce に載せるなら「GJI I/O 観測 Event」を MT へ post して時刻(`current_tick_ms()`)を Event に載せる。**書き込み頻度が高い**(毎 tick)ので、変化時のみ Event にする必要がある。現状は MT が「必要な時にスナップショットを読む」pull 型で、読む時刻と値の対応が journal に残らない |
| `tsf_active_kind`/`ms_ime_native_identified`/`ime_product_name` | デバウンス済みの確定時のみ書く(頻度は低い)。`TipIdentity` という純粋型があり、`IdentityChanged(TipIdentity)` の Event として MT へ運べる。ただし**読み手(key_pipeline.rs 6 箇所、focus_tracking.rs 2 箇所)が `tsf_obs().active_ime_kind()` を直読み**しているため、読み取りも `ImeModel`/`PlatformState` のスライスに移す必要がある(変更箇所が広い) |
| `ime_composition_active`/`gji_candidate_visible`/`candidate_was_seen`/`ChangeCounter` 4 つ(WinEventProc) | WinEventProc は MT で動くが `with_app` に入れない場合がある。「WinEvent 観測 Event」を post する方式なら Event 化できる。**順序保証**: `OUTPUT_GATE` が active の間に来た WinEvent は、post なら他の post 済み Message との相対順序が OS キュー順で決まる(現状の atomic 直書きは「その場の値」で順序情報を持たない) |
| `literal_session_confirmed_gen` | MT のみ。世代は既に `Generation` 型。`LiteralSessionConfirmed(cold_seq)` Event に直せる。難度は warmup 領域の他の書き込みとの整合次第(中) |
| `OUTPUT_GATE.active/depth` と `OutputActiveGuard` | MT のみだが **RAII ガードで await をまたいで保持される**(`vk_send.rs:296` は await 前に取得して ChromeProbe に move)。Event 化すると「ガード取得/解放」Event の対で、`Drop` が Event を発行する形になる。**`Drop` からの Event 発行が失われると `active` が固着する**(BUG-65 追補2 の depth ラップアラウンド事故の教訓)。ガード状態が belief ではないため、reduce に載せるメリットは小さい(journal に開閉が残る程度) |
| `FOCUS_RESYNC` | MT のみ・generation で失効。`FocusResyncArmed`/`Closed`/`Opened(gen)` Event に直せる。ゲートが開いたかどうかは `OUTPUT_GATE.is_active()` と合わせて「defer するか」を決めるので、**2 つのゲートの合成判定**が取り込み口(app/mod.rs:644-645)にある |
| `INPUT_DEFER` | MT のみ。reduce の外の「副作用待ちキュー」。`RawKeyEvent` は到着時に journal 済みで、defer→replay は Cmd の遅延実行にあたる。**キューの順序を journal に残す**なら `Deferred{reason}`/`Replayed` Event が要る |

### 3.3 難度 高(フックスレッドが書く、または reduce の同期判定に使われる)
| 対象 | 理由 |
|---|---|
| `HOOK_STATE` の HK 書き込み 20 フィールド(O 3、G 9、K 3 ほか) | HK は OS のフックタイムアウト(`low_level_hooks_timeout_ms`、既定 300ms 前後)内に戻り値を返す必要があり、`&mut` の reduce 状態を持てない。**HK が書く値は「HK 自身の戻り値(飲み込む/通す)を決める同期判定」に使われる**(inventory-a1.md §1.3 項目 3: `focus_app_disabled`、`cached_swallow_alt_kana_mode_switch`、`alt_key_held()`、overflow ラッチ、世代ガード、`alt_*_impersonating`)。それ以外(`physical_key_state` の MT 読み、`ctrl_consumed_since_down`)は SPSC リングに載せる生イベントの拡張(拡張ビット・なりすまし前 vk)で畳み込める(難度 中〜高) |
| `HOOK_KEYS` リング自体 | これが既に「HK→MT の Event の運び口」。reduce の入口としてはこのまま使える。難度は「リングに載らないイベント(自己注入・カナリア・飲み込み)を載せるか」 |
| `cached_engine_enabled`/`focus_app_disabled`(MT→HK の方向の書き込み) | **逆向き(MT が書き HK が読む)**。HK は MT の reduce 状態を直読みできないので、「状態の写しを atomic で渡す」以外に手が無い。reduce 化するなら「reduce 後の状態の写しを書く**射影**」として扱う(状態の書き込みではなく出力の一種)。**この方向の atomic は、reduce の結果を shell が HK に公開する Cmd と見なせる** |
| `SEND_HEALTH`/`CONV_MUTATION_SEQ`/`PROBE_FENCE.fence_value` | WK(IMM 呼び出しワーカー)が書く。**書き込みが `SendMessageTimeoutW` の直前・直後で行われ、その時刻が意味を持つ**(`bump` は syscall の前に完了する必要がある=ADR-140 決定B の必須要件。Event を post すると順序が崩れる)。Event 化は不適(失効トークンは書き込みと OS 呼び出しの同期点が要)。reduce の外に残し、**値を spawn 時の Event/Cmd に載せて比較する**形(現状どおり)が妥当 |
| `LAST_ACTUATION_ISSUE_US` | MT/WK が書き HK が読む診断値。HK 側の読み取りは診断ログ専用なので Event 化の価値なし |

---

## 4. 例外イベント(reduce に入るが「ユーザー意図でも観測でもない直接書き込み」)

本領域(グローバル)は reduce の外なので、該当する reduce 内の例外イベント(`PanicReset`・`HwndCacheRestored`・`ModeKeyPassedThrough`・`KeyEffectPredicted` など)は **W0-a/b の担当**で、ここでは**グローバルから reduce へ橋を渡している箇所**だけを挙げる(いずれも該当 Event の存在は未確認。私は `state/ime_model.rs` の Event 定義を精読していない):

| 橋 | グローバル側 | reduce 側(想定、未確認) |
|---|---|---|
| `panic_detect::record_ime_keydown` が窓成立で `post_to_main_thread(WM_PANIC_RESET)` | `RAPID_IME_TIMESTAMPS`(G) | `PanicReset`(W0-a/b で確認) |
| `ObservedState::from_snapshot(tsf_obs())` | `TSF_OBS` | 判断関数の入力(`ImeControlView.observed`)。reduce には直接入らない |
| `candidate_was_seen`(`gji_direct_already_matches`) | `TSF_OBS.candidate_was_seen` | actuation の「送信省略」判定。belief は更新しないが **actuation の有無=awase の外向き挙動を決める** |

---

## 5. 危険箇所

| # | 箇所 | 内容(根拠) |
|---|---|---|
| D1 | **await をまたぐ失効トークン**(`CONV_MUTATION_SEQ`、`PROBE_FENCE`、`TSF_OBS` 世代) | `runtime/key_pipeline.rs:516-540`(spawn 時に `conv_mutation::current()`/`probe_actuation_fence::current()` を取る)→ await → `:700/:726`(apply 時に再比較して破棄)。比較が 3 点(spawn・issue main/worker・apply)に分散(ime.rs:793/797、output/probe_io.rs:245/259、key_pipeline.rs:726)。**4 点目の比較点を足し忘れる**と ADR-140 決定D の窓が開く。reduce の外にあるため、journal を再生しても「その時点の fence 値」は復元できない |
| D2 | `candidate_was_seen`: 立てるのは WinEventProc、下ろすのは 3 経路(platform.rs:1144、ime_controller.rs:268、focus_tracking.rs:627)。**下ろし漏れ/下ろしすぎ**が `gji_direct_already_matches` の「送信省略」判定(state/ime_actuation_decision.rs:227)を直接変える | ラッチの遷移が journal に無いので、再現報告で「その時 `candidate_was_seen` は何だったか」が分からない(ただし `ActuationDecisionRecord.candidate_was_seen` は決定時の入力値として残る場合がある=未確認) |
| D3 | **OUTPUT_GATE と FOCUS_RESYNC の 2 ゲート合成**(app/mod.rs:644-645、message_handlers.rs:689): 取り込み口で `OUTPUT_GATE.is_active() \|\| defer_for_resync` の OR を取り、同じ式が message_handlers.rs:689 にもある。fix-requires-evidence.md の「defer/replay キューの解放条件」ファミリー(ADR-156、ADR-123→ADR-128)に該当 | 新しい解放条件を足す際は defer 側 3 窓口(app/mod.rs:654/662/666)と drain 側(message_handlers.rs:1763/1810)の**全て**に配線が要る |
| D4 | `OUTPUT_GATE.depth` の `AtomicU32` ラップアラウンド(BUG-65 追補2 で実害確認済み): `fetch_sub` の 0 アンダーフローで `active` が恒久 false。`real` フラグで暫定対処 | reduce 化せず atomic のまま RAII を維持する限り再発余地は残る(本棚卸しで再発の兆候は未確認) |
| D5 | **HK が書く値の MT 側ライブ読み**(inventory-a1.md §1.3 項目 5): `ctrl_consumed_since_down`(key_pipeline.rs:192)、`physical_key_state`(key_pipeline.rs:150)、`alt_*_impersonating`(runtime/mod.rs:491、message_handlers.rs:703)。ADR-129 の事故型(drain replay 中に「いま」の値を読む) | リングに溜まっている間に値が変わる。journal には「読んだ値」が `modifier_snapshot` として残る分は再現できるが、`ctrl_consumed` は未確認 |
| D6 | `INPUT_RELAY_APPS` と `FocusTracker::input_relay_apps()` の二重保持(ADR-180 決定1 の InputRelay ゲートが依存): 同期は `ForceOverrides::new` の副作用のみ | 設定再読込で `ForceOverrides` が作り直されない経路があれば不一致(未確認) |
| D7 | `WinEventProc`(MT)が `with_app` に入れない再入: `with_app` は `try_borrow_mut` で None を返し warn のみ(lib.rs:235-245)。TSF_OBS を atomic にしているのはこのため。**Event 化(post)に切り替えると、モーダルポンプ中(ModalPumpGuard)や `OUTPUT_GATE` active 中の配送順序が「atomic の即時反映」から「メッセージ順序」に変わる** | `NEEDS_ENGINE_RESYNC`(engine_window.rs:16)が既にこの補償(モーダル中に落ちた状態の再同期要求)を担っている。ここを触る場合は ADR-105 を確認 |
| D8 | `SENT_INPUT_TRACE`(thread_local)は WK からの `send_input_safe` ぶんが journal に移されず捨てられる(win32.rs:296 の doc に明記) | WK から SendInput が呼ばれる経路が現存するか未確認(確認すれば「journal に載らない送信」が実在するか分かる) |
| D9 | `hook_alive_tick_ms` は「フックが呼ばれたか」の観測そのもの(エンジンスレッド停止・フック飢餓の検出目的、issue #165)。**reduce の中に置くと検出対象の故障と同じ経路で止まる** | 永久に reduce の外(infra)に置く根拠 |
| D10 | ADR-164 が解消対象とした「裸 static の個数」と、本棚卸しの「reduce を通らない書き込み」は**別軸**。HOOK_STATE を struct に束ねても書き込みは直書きのまま(`HOOK_STATE.x.store(...)` 113 箇所の出現、hook.rs 内のみ) | 「グローバルが少ない」=「reduce に通っている」ではない |

---

## 6. 未確認の点

- `origin/develop`(`ee54b2de`)との差分: 作業ツリー(`efe5248e`)を読んだ。対象ファイルの差分は未確認。
- `set_ime_product_name` の書き手の呼び出し元は未精査(gji_monitor.rs 周辺と推測)。
- `ObservedState`/`ImeControlView` が `ActuationDecisionRecord` にどこまで記録されるか(`candidate_was_seen`、`gji_last_io_ms`)は、レコード定義を精読していない。「決定時の入力」として載る可能性があり、その場合 journal からの「入力値の再現」は部分的に Yes になる。
- `INPUT_DEFER` を `OutputActiveGuard::drop` 経由で WK から触る経路が存在するか(Mutex の必要性)。`tsf/probe_bridge.rs:131` の `pending_len_nonblocking()` は `Drop` から呼ばれるが、`Drop` が MT 以外で走るかは未確認。
- `ImeKindId`/`TipIdentity` を `ImeEvent` として reduce に載せる素地(Event variant の既存有無)は `state/ime_model.rs` を未精査。
- `ime_profile_driver.rs`・`ime_controller.rs` の ZST static は定義のみ確認、不変であることは ADR-164 の分類に依拠。
- HOOK_STATE の journal 再生可否: `KeyInput` に載るフィールド(`modifier_snapshot`、`was_down`、`press_id`、親指スナップショット)は確認したが、再生で `physical_key_state` が復元されるかは `journal_replay.rs` を読んでいない。
