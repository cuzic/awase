# 棚卸し D1: runtime/message_handlers.rs・runtime/focus_tracking.rs・focus/・observer/

対象: crates/awase-windows/src、develop 9df983e4 時点、読み取りのみ。行数は `wc -l` と関数範囲(doc コメント込み)の実測。
「その他」は use / モジュール doc / 区切りコメント / 空行(分類対象外)。F は関数全体を F に計上(F の中の P 相当部分は分割案に記載)。
分類は読んだ上での判断で、境界が曖昧なものは備考に理由を残した。

## 0. 先に分かったこと(要点)

1. **この領域の大半は gate されている**: `runtime/`・`observer/`・focus/ の 9 ファイルが `#[cfg(windows)]`。ungated(Linux でテストされている)のは focus/ の cache / class_names / current / kinds / thread_scope(純粋部)だけ。**gate されたファイルの #[test] 36 本が Linux バイナリに存在しない**(message_handlers 4 + focus_tracking 8 + classifier 9 + tracker 6 + ime_observer 9)。
2. **HWND の seam は半分できている**: `state::ime_event::HwndId(pub usize)`(168 箇所で使用、`to_hwnd()` は cfg(windows))が既にある。一方 `CurrentFocus.hwnd`・`FocusIdentity.hwnd`・`HwndImeSnapshot.hwnd`・`FocusSnapshot.hwnd_addr` は生の `usize`、`ClassifiedFocus` / `uia::SendableHwnd` / `classify_focus` / `msaa_classify` / `resolve_focus_kind` / `probe_focus_thread` / `learn_imm_capability_on_focus` は `HWND` 型。hwnd をキーにするキャッシュは無く(キーは `(pid, class_name)`、hwnd は値)、置き換え対象は値・引数・比較(§4)。
3. **Win トークン無しなのに gate されている**: `focus/hwnd_cache.rs`(`hook::current_tick_ms` のみ)、`focus/tracker.rs`(`uia::SendableHwnd` 経由)、`observer/gji_observer.rs`(時計 + `tsf_obs()` の atomic)。時計を注入すれば gate を外せる。ungated の `focus/cache.rs` も `Instant::now()` を直読みしている。`ime_observer.rs` は名前は observer だが中身の 8 割は純粋な `classify_*`(P* 264 行)で、O は `poll_and_classify_ime` の 1 呼び出しだけ。
4. **UIA 経路は結果を捨てている**: `focus/uia.rs`(306 行・COM 16 トークン)→ `WM_FOCUS_KIND_UPDATE` → `handle_wm_focus_kind_update` は BUG-12 以来「適用せずログのみ」。bootstrap.rs:1317 で COM ワーカーを起動し、`try_send_uia`(focus_tracking.rs:102,866)が hwnd を送り続けるが結果は誰も使わない。約 400 行。**移すより先に削除を検討する対象**(未確認: 観測目的の利用が残っているか)。
5. **message_handlers.rs は「WM 入口(G)」より「キー経路の判断(F)」が重い**: `deliver_key_event`(116 行)・`handle_wm_timer` の F 2 arm(148 行)・`handle_wm_drain_output_queue`(123 行)で 387 行。残りはほぼ 1 行委譲の G と、bug report 組み立て(O/P*)。core Event に翻訳される入口は実質「キー 2 経路・エンジンタイマー・IME apply 完了・panic reset・コンテキスト無効化」だけ(§3)。
6. **focus_tracking.rs は 63% が F**(615/957 行)。`on_focus_process_changed` 1 関数で 307 行。判断の半分(cache restore/discard の 2 純関数)は既に出ており、残りは「O の facts を作る」「FocusPlan を返す純関数」「適用」の 3 段に割れる(§2 の分割案)。ここが最も危険(§9 D1/D2/D7)。

## 1. ファイル別サマリ

| ファイル | 全体 | 本体 | テスト | P | P* | O | E | F | G | その他 | 移せる(P+P*) | 残る(O+E+G) | 分割要(F) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| runtime/message_handlers.rs | 2032 | 1930 | 102 | 116 | 204 | 228 | 96 | 683 | 502 | 101 | 320 | 826 | 683 |
| runtime/focus_tracking.rs | 1053 | 957 | 96 | 15 | 88 | 0 | 0 | 615 | 202 | 37 | 103 | 202 | 615 |
| focus/cache.rs | 216 | 111 | 105 | 31 | 80 | 0 | 0 | 0 | 0 | 0 | 111 | 0 | 0 |
| focus/class_names.rs | 811 | 374 | 437 | 374 | 0 | 0 | 0 | 0 | 0 | 0 | 374 | 0 | 0 |
| focus/current.rs | 241 | 144 | 97 | 111 | 33 | 0 | 0 | 0 | 0 | 0 | 144 | 0 | 0 |
| focus/kinds.rs | 156 | 73 | 83 | 73 | 0 | 0 | 0 | 0 | 0 | 0 | 73 | 0 | 0 |
| focus/thread_scope.rs | 449 | 290 | 159 | 154 | 0 | 136 | 0 | 0 | 0 | 0 | 154 | 136 | 0 |
| focus/classifier.rs | 766 | 538 | 228 | 67 | 230 | 91 | 74 | 0 | 47 | 29 | 297 | 212 | 0 |
| focus/classify.rs | 245 | 245 | 0 | 48 | 0 | 75 | 0 | 105 | 0 | 17 | 48 | 75 | 105 |
| focus/hwnd_cache.rs | 115 | 115 | 0 | 36 | 79 | 0 | 0 | 0 | 0 | 0 | 115 | 0 | 0 |
| focus/imm_learning.rs | 84 | 84 | 0 | 0 | 0 | 0 | 0 | 77 | 0 | 7 | 0 | 0 | 77 |
| focus/kind_classifier.rs | 82 | 82 | 0 | 0 | 8 | 0 | 0 | 67 | 0 | 7 | 8 | 0 | 67 |
| focus/msaa.rs | 151 | 151 | 0 | 80 | 0 | 0 | 0 | 55 | 0 | 16 | 80 | 0 | 55 |
| focus/probe.rs | 72 | 72 | 0 | 0 | 18 | 50 | 0 | 0 | 0 | 4 | 18 | 50 | 0 |
| focus/tracker.rs | 441 | 351 | 90 | 38 | 311 | 0 | 0 | 0 | 0 | 2 | 349 | 0 | 0 |
| focus/uia.rs | 306 | 306 | 0 | 5 | 15 | 42 | 0 | 131 | 80 | 33 | 20 | 122 | 131 |
| observer/focus_observer.rs | 39 | 39 | 0 | 0 | 0 | 24 | 0 | 0 | 0 | 15 | 0 | 24 | 0 |
| observer/gji_observer.rs | 72 | 72 | 0 | 18 | 54 | 0 | 0 | 0 | 0 | 0 | 72 | 0 | 0 |
| observer/ime_observer.rs | 543 | 318 | 225 | 39 | 225 | 0 | 0 | 38 | 0 | 16 | 264 | 0 | 38 |
| observer/kana_lock.rs | 40 | 40 | 0 | 0 | 0 | 36 | 0 | 0 | 0 | 4 | 0 | 36 | 0 |
| observer/layout_observer.rs | 56 | 56 | 0 | 18 | 0 | 33 | 0 | 0 | 0 | 5 | 18 | 33 | 0 |
| focus/mod.rs + observer/mod.rs | 44 | 44 | 0 | 0 | 0 | 0 | 0 | 0 | 44 | 0 | 0 | 44 | 0 |
| **合計** | 8014 | 6392 | 1622 | 1223 | 1345 | 715 | 170 | 1771 | 875 | 293 | 2568 | 1760 | 1771 |

### 領域グループ別

| グループ | 全体 | 本体 | テスト | P | P* | O | E | F | G | その他 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| runtime/ | 3085 | 2887 | 198 | 131 | 292 | 228 | 96 | 1298 | 704 | 138 |
| focus/ (ungated) | 1873 | 992 | 881 | 743 | 113 | 136 | 0 | 0 | 0 | 0 |
| focus/ (cfg(windows)) | 2262 | 1944 | 318 | 274 | 661 | 258 | 74 | 435 | 127 | 115 |
| observer/ | 750 | 525 | 225 | 75 | 279 | 93 | 0 | 38 | 0 | 40 |
| mod 宣言 | 44 | 44 | 0 | 0 | 0 | 0 | 0 | 0 | 44 | 0 |

本体 6392 行のうち、移せる(P+P*) 2568 行(40%)、残る(O+E+G) 1760 行(28%)、分割が要る(F) 1771 行(28%)、分類対象外 293 行(5%)。

P は今のまま移せる行(1223)、P* は seam を直してから移せる行(1345)。ungated の focus/ 5 ファイル(P 743 + P* 113)は既に Linux でテストされている。

## 2. ファイル別の詳細

### runtime/message_handlers.rs

1. 行数: 全体 2032 / 本体 1930 / テスト 102。gate: lib.rs:82-83 `#[cfg(windows)] pub mod runtime;` の配下。テスト(#[test] 4本: encode/decode 2本・drain 1本・apply_wparam 1本)は Linux バイナリに存在しない
2. 内訳(本体行): P 116 / P* 204 / O 228 / E 96 / F 683 / G 502 / 分類対象外 101
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| DRAIN_PENDING / DRAIN_RERUN_PENDING (static AtomicBool x2) | 34-36 | 3 | G | WM_DRAIN_OUTPUT_QUEUE 再入ガードのグローバル。core に移すなら Runtime 内フィールド化が先 |
| recover_pending_drain_request | 37-45 | 9 | G | static 2本 + post_to_main_thread(WM_DRAIN_OUTPUT_QUEUE) |
| finish_drain | 47-50 | 4 | G | static + 上の関数 |
| enum KeyOrigin | 52-63 | 12 | P* | seam: `Hook(PumpContext)` の PumpContext が runtime::engine_window(cfg(windows)) の型。Main/Nested を core の enum にすれば P |
| enum KeyDelivery | 65-69 | 5 | P | Consumed/Reinjected |
| notify_if_solo_off_triggered | 75-88 | 14 | G | engine.take_solo_off_notification() → tray.show_balloon。core Event: なし(engine の出力フラグを読む) |
| handle_wm_kana_lock_warning_changed | 90-97 | 8 | G | WM_KANA_LOCK_WARNING_CHANGED → tray 同期のみ。core Event: なし |
| handle_wm_hook_ime_mode_diagnostic | 99-107 | 9 | G | WM_HOOK_IME_MODE_DIAGNOSTIC → journal.record(HookImeModeDiagnostic)。core Event: なし(診断) |
| begin_key_batch | 109-127 | 19 | G | take_needs_engine_resync()(グローバル) → engine.on_command(EngineCommand::FocusChanged)。with_app 借用 0(呼び出し側の &mut Runtime) |
| post_effects_if_reinjected | 129-138 | 10 | G | Reinjected なら PostMessage(WM_EXECUTE_EFFECTS)。判定は 1 行、実体は E |
| deliver_key_event | 140-255 | 116 | F | F: 「keymap latch → Nested → NonText → keymap 照合 → post_bypass → engine」の優先順位判断(純粋)が hook::current_tick_ms / executor.enqueue_reinject / Runtime::process_key_event と混在。分割案: core に `route_key(origin, focus_kind, keymap_latch, ...) -> Route{Consume,Reinject,Keymap,PostBypass,ToEngine}`、Windows 側は Route を実行するだけ。core Event: RawKeyEvent → Runtime::process_key_event / replay_ime_off_rescue_event。architecture_guard の複数本がこの関数の本文順序をテキストで固定 |
| consume_keymap_match | 257-308 | 52 | F | F: `active_keymaps.find_match`(純粋) + composition 判定(tsf::observer の atomic 読み) + cancel_composition(E) + send_keymap_target(SendInput, E)。core: find_match+latch+「cancel が要るか」判定、Windows: cancel/SendInput |
| cancel_composition | 310-321 | 12 | E | cancel_ime_composition()(Win32) + platform.on_composition_cancel |
| cancel_composition_and_arm_post_bypass_on_ctrl | 323-368 | 46 | F | F: 条件(Hook::Main && keydown && ctrl && !passthrough)は純粋、composition 読み(O)と cancel(E)が混在 |
| consume_post_bypass | 370-426 | 57 | F | F: 判定は既に純関数 classify_post_bypass_key。残りは foreground_scope()(Win32) と enqueue_reinject(E)。seam: ForegroundScope を引数で受ける |
| arm_post_bypass_if_matches | 428-455 | 28 | P* | seam: `win32::foreground_scope()`(HWND/PID 取得)を引数化。process_name/class_name は &str で渡せる |
| handle_wm_key_from_hook | 457-468 | 12 | G | WM_KEY_FROM_HOOK → core: RawKeyEvent(キー入口) |
| handle_wm_timer(ヘッダ+timer.resolve) | 470-479 | 10 | G | WM_TIMER → timer.resolve(wparam) で論理 ID 化 |
|   arm TIMER_IME_REFRESH | 480-488 | 9 | G | core: IME refresh 要求(process_deferred_keys + spawn_ime_refresh) |
|   arm TIMER_POWER_RESUME | 489-495 | 7 | G | core: ContextChange::InputLanguageChanged + focus_kind=Undetermined の直接書き込み(1行) |
|   arm TIMER_OUTPUT_GUARD | 496-501 | 6 | G | executor.on_output_guard_timer → dispatch_outcomes |
|   arm TIMER_TSF_PROBE | 502-517 | 16 | G | diagnostic_snapshot を thread_local(ime_diagnostic::set_tsf_probe_snap) へ渡す回避策(with_app_ref の BorrowError 回避)を含む。core Event: TSF probe tick |
|   arm TIMER_TSF_GATE | 518-528 | 11 | G | platform.on_tsf_warmup_timeout → INPUT_DEFER.replay_later |
|   arm TIMER_IME_OFF_RESCUE | 529-548 | 20 | G | core: 保留キーの再投入(KeyOrigin::ImeOffRescueReplay) |
|   arm TIMER_GJI_LONG_IDLE | 549-555 | 7 | G | platform.gji_on_timer_long_idle |
|   arm TIMER_FOCUS_RESYNC | 556-574 | 19 | G | FOCUS_RESYNC / OUTPUT_GATE(static)。判定は既に純関数 should_post_drain |
|   arm TIMER_HOOK_WATCHDOG | 575-659 | 85 | F | F: `stale_ms>5000` / `os_idle<5000` の閾値判断がインライン(hook::hook_alive_tick_ms/os_idle_ms の読み取りと混在)。分割案: core `classify_watchdog_tick(stale_ms, os_idle_ms) -> Alive\|Idle\|Starved`。自己修復の実判断は既に state::hook_watchdog::decide(純粋) |
|   arm TIMER_HOOK_WATCHDOG_CANARY_CHECK | 660-666 | 7 | G | confirm_hook_watchdog_canary |
|   arm 汎用エンジンタイマー | 667-729 | 63 | F | F: OUTPUT_GATE/FOCUS_RESYNC active なら deferred_engine_timers へ延期(判断・static 依存) + read_os_modifiers(O) + build_input_context + engine.on_timeout。core Event: Engine::on_timeout(timer_id, &ctx)。ADR-156 の defer/replay 窓口の片側(drain 側は handle_wm_drain_output_queue) |
|   arm None → DispatchMessageW | 730-737 | 8 | E | 未知タイマーを OS へ素通し(win32-async 用) |
| sample_watchdog_kana_lock_edge | 739-766 | 28 | O | read_kana_lock + foreground_class_name を読み、前回値と比べてログ。belief 不触 |
| handle_wm_execute_effects | 768-777 | 10 | G | WM_EXECUTE_EFFECTS → executor.drain_deferred → dispatch_outcomes |
| encode_outcome | 787-808 | 22 | P | ImeOpenOutcome→isize(wire)。core 型のみ。wire 形式は Windows 通信路固有だが純粋 |
| decode_outcome | 810-826 | 17 | P | 未知値は UnsafeToToggle に倒す |
| post_async_ime_apply_complete | 828-870 | 43 | E | PostMessage(WM_ASYNC_IME_APPLY_COMPLETE)。ApplyGeneration::to_wire(NonZeroU64) 経由 |
| encode_apply_wparam | 872-893 | 22 | P | wparam の bit 配置(open / reason 2bit / generation) |
| decode_reason | 895-902 | 8 | P |  |
| handle_wm_async_ime_apply_complete | 904-917 | 14 | G | core Event: Runtime::on_ime_apply_complete(open, outcome, generation, reason)(= ImmCross 非同期 apply の完了) |
| handle_wm_panic_reset | 919-922 | 4 | G | core Event: ImeEvent::PanicReset(apply_panic_reset) |
| sync_ime_toggle_auto_detect | 924-945 | 22 | F | F: レジストリ読み(O: read_toggle_assignment_from_registry) + `to_combos`(純粋) + engine.set_ime_toggle_auto_keys。分割: Windows が割り当てスナップショットを返し、core が combos 化して engine へ |
| sync_ime_kind_from_observation | 947-1001 | 55 | F | F: tsf_obs() atomic 読み(O) + belief(applied_open) 読み + output.set_active_ime_kind(E) + GJI FSM 通知 + check_and_warn(ポップアップ, E)。分割案: core `plan_ime_kind_sync(kind, detected, applied_open) -> {gji_on_ime_on, check_mode_keys, msime_warn}` |
| handle_wm_ime_kind_changed | 1003-1008 | 6 | G | WM_IME_KIND_CHANGED → 上の関数 |
| handle_wm_duplicate_instance | 1010-1016 | 7 | G | tray balloon のみ。core Event: なし |
| handle_wm_powerbroadcast | 1018-1031 | 14 | G | PBT 定数判定(純粋 1 行) + timer kill/set |
| handle_wts_session_change | 1033-1065 | 33 | G | core: ContextChange::FocusChanged + set_session_locked + hook::reset_physical_key_state + keymap_latch.release_all |
| handle_wm_inputlangchange | 1067-1073 | 7 | G | core: ContextChange::InputLanguageChanged + refresh_ime_state_cache |
| handle_wm_focus_kind_update | 1075-1114 | 40 | G | **結果を破棄する no-op**(BUG-12)。HWND 比較(GetGUIThreadInfo)してログのみ。core Event: なし。architecture_guard が belief 非書き込みを固定 |
| handle_wm_hotkey_toggle / _focus_override | 1116-1124 | 9 | G | core: EngineCommand 相当(toggle_engine / toggle_app_override) |
| handle_wm_app_tray | 1126-1156 | 31 | G | with_app_ref x1 でメニュー表示用データを集めて tray へ。sent message 経由(WndProc 同期配送) |
| handle_wm_reload_config | 1158-1162 | 5 | G | reload_config() |
| handle_wm_command(ヘッダ + ime_target 取得 + match 前半) | 1164-1196 | 33 | G | TrayCommand → Settings/Toggle/Exit/SelectLayout/AutoStart 等。関数全体で with_app 6(+ with_app_ref 1) |
|   arm TrayCommand::BugReport | 1197-1260 | 64 | F | F: journal へ ClockAnchor/DumpTriggered を積む + dump + 診断収集 + プロセス起動が 1 arm に同居。with_app 2。分割: core `prepare_bug_report_journal(&mut Journal)`、Windows は dump/launch |
|   arm CapsLock | 1261-1263 | 3 | E | ime::toggle_caps_lock |
|   arm ResetState | 1264-1284 | 21 | E | caps off + set_ime_mode_for_target(HWND 指定の IMM 書き込み) + force_engine_on。core Event: force_engine_on |
|   arm KanaLockHelp / ClearImmCache | 1285-1307 | 23 | G | platform.clear_imm_capability_cache(BUG-108) |
| current_bug_report_ime_kind | 1309-1322 | 14 | O | tsf_obs() atomic → BugReportImeKind |
| current_bug_report_diagnostics | 1324-1402 | 79 | O | &Runtime から belief/focus/gate を読み、Win32(keyboard_layout_info, process_resource_snapshot)・global(send_health, probe_actuation_fence, hook_channel)を集める BugReportDiagnostics 構築 |
| build_bug_report_keymap_learn_summary | 1404-1420 | 17 | O | ファイル読み(table_file_path) |
| GjiCustomKeymapFields + build_bug_report_gji_keymap_summary | 1422-1549 | 128 | P* | seam: `read_config1_db()`(ファイル I/O) と `app.muhenkan_dedicated_fn_key_configured()` を引数化。残り(is_effective 判定・extract_*・分類)は awase_gji_config の純粋処理 |
| build_bug_report_msime_key_assignment_summary | 1551-1586 | 36 | P* | seam: レジストリ生 DWORD(read_raw_key_assignment_dwords)と space_is_thumb_key() を引数化 |
| build_bug_report_legacy_msime_keymap_summary | 1588-1602 | 15 | O | レジストリ 2 読み |
| ime_toggle_kind_str | 1604-1611 | 8 | P |  |
| gji_composition_mode_str | 1613-1622 | 10 | P |  |
| parsed_key_combo_label | 1624-1641 | 18 | P |  |
| ProcessResourceSnapshot + filetime_to_100ns_units | 1643-1655 | 13 | O | FILETIME |
| process_resource_snapshot | 1657-1718 | 62 | O | GetProcessTimes/GetProcessMemoryInfo/GetGuiResources → ProcessResourceSnapshot |
| bug_report_keyboard_model | 1720-1725 | 6 | P |  |
| write_bug_report_diagnostics | 1727-1735 | 9 | E | temp ファイル書き込み(hook::current_tick_ms 依存) |
| handle_wm_drain_output_queue | 1737-1859 | 123 | F | F/G: with_app 5 回借用(flush_raw_tsf_literal_recovery / take_all+begin_key_batch+enrich / replay(deliver_key_event) / deferred timers replay / drain_runtime_requests)。再入ガード(DRAIN_PENDING/RERUN)・失敗時の INPUT_DEFER 戻し・os_id 照合(別文字のタイマーを早期発火させない)が 1 関数に同居。core Event: 遅延 RawKeyEvent の再投入 + Engine::on_timeout(replay)。ADR-156 の drain 側窓口 |
| handle_taskbar_created | 1948-1952 | 5 | G | tray.recreate |
| handle_wm_dump_journal | 1954-2014 | 61 | G | journal bookkeeping + dump_to_file(E) + balloon。診断のみ |

4. 移せる(P+P*) 320 / 残る(O+E+G) 826 / 分割要(F) 683

### runtime/focus_tracking.rs

1. 行数: 全体 1053 / 本体 957 / テスト 96。gate: runtime/ 配下 = lib.rs:82-83 の `#[cfg(windows)] pub mod runtime`。#[test] 8本(should_restore_tsf_cache_on / should_discard_imm_broken_cache 系)は Linux バイナリに存在しない
2. 内訳(本体行): P 15 / P* 88 / F 615 / G 202 / 分類対象外 37
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| struct ClassifiedFocus | 16-24 | 9 | P* | seam: `hwnd: HWND` → HwndId |
| focus_identity_snapshot | 28-38 | 11 | P* | 既に所有型 FocusIdentity を返す。seam: `&self`(Runtime)借用 → 2 つの substate 引数 |
| record_focus_transition_if_changed | 40-71 | 32 | P* | seam: hook::current_tick_ms()(時計注入)・journal 書き込み(state 側) |
| apply_focus_probe_result | 73-107 | 35 | G | FocusSnapshot → classify → advance → journal → injection_mode → on_focus_process_changed。core Event: ImeEvent::FocusChanged(下流) |
| establish_initial_focus_scope | 109-154 | 46 | G | bootstrap 入口。read_focus_snapshot(O)を直接呼ぶ。core Event: InitialFocusFence/InitialAppPolicy/InitialFocusHwnd Established(下 3 関数)。ADR-102 決定3-b: belief を書かない(architecture_guard が本体内の dispatch_event 文字列を禁止) |
| enter_focus_scope | 156-190 | 35 | F | F: focus_epoch++ / last_focus_change_ms(state) + platform.notify_focus_changed()(E) + recompute_active_keymaps。core 側にエポック進行+keymap フィルタ、Windows 側は notify だけ残す |
| presync_applied_open_on | 192-220 | 29 | F | F: belief(record_confirmed) 書き込み + tsf_obs().active_ime_kind()(O 読み) + GjiFsm 通知(platform.gji_on_ime_on)。BUG-18/BUG-163 の再発ファミリー(IME belief) |
| sync_initial_focus_fence | 222-258 | 37 | G | core Event: ImeEvent::InitialFocusFenceEstablished{fence} |
| sync_initial_app_policy | 260-278 | 19 | G | core Event: ImeEvent::InitialAppPolicyEstablished{profile}(BUG-114) |
| sync_initial_focus_hwnd | 280-294 | 15 | G | core Event: ImeEvent::InitialFocusHwndEstablished{hwnd: HwndId(classified.hwnd.0 as usize)} |
| classify_focus_probe | 296-387 | 92 | F | F: O(get_process_name / learn_imm_capability_on_focus=ImmGetDefaultIMEWnd / resolve_focus_kind=classify_focus を別スレッド実行)と state 書き込み(app_kind/focus_kind)・cache_insert が交互。分割案: Windows が `ProbeFacts{process_name, imm_ime_wnd_null, classify_result}` を返し、core が app_kind/focus_kind 更新・cache 登録・学習判定を行う |
| advance_focus_tracking | 389-485 | 97 | F | F: should_save(MIN_FOCUS_DURATION_MS)・from_explicit_off_intent(last_intent の source 判定)は純粋、update_focus_info(→get_process_name/root_hwnd_of の Win32)・hook ラッチ操作が混在。core: `should_save_ime_cache`, `from_explicit_off_intent` |
| notify_focus_hwnd_updated_if_needed | 487-526 | 40 | G | core Event: ImeEvent::FocusHwndUpdated{hwnd}。判定(is_bootstrap\|\|process_changed\|\|same)は純粋だが dispatch_event 文字列が guard テストに載るため関数分離されている |
| apply_app_disable_transition | 528-582 | 55 | F | F: `edge(was,is)` は純粋、hook::set_focus_app_disabled / clear_hook_latches_for_app_disable(hook static)・latch 解放が混在。core Event: ContextChange::FocusChanged(Enter のみ、bootstrap では抑止) |
| on_focus_process_changed | 584-890 | 307 | F | **最大の F(307 行)**。core Event: ImeEvent::FocusChanged / HwndCacheRestored / ObserverReported(HeuristicDefault) / 非同期 ImmCrossProbe(High)。分割案: Windows が thread_scope probe・SPI thread-local・tsf_obs().table_ime_kind() を `FocusFacts` にまとめ、core の純関数 `plan_focus_process_changed(facts) -> FocusPlan{restore: Restore/Discard/Skip, reset: AssumeClosed/ResetStale/Skip, presync, spawn_imm_probe, arm_resync, send_uia}` を実行側が適用。判断部分(cache restore の 2 分岐・EXPLICIT_OFF_CACHE_SUPPRESS_MS・awase 自身の pid 除外)は既に 2 本の純関数(should_restore_tsf_cache_on / should_discard_imm_broken_cache)に半分出ている |
| detect_and_update_focus | 892-901 | 10 | G | O(read_focus_snapshot) + apply |
| impl From<&FocusIdentity> for FocusEndpoint | 904-918 | 15 | P |  |
| should_restore_tsf_cache_on | 920-935 | 16 | P* | seam: 引数 HwndImeSnapshot が focus::hwnd_cache(cfg(windows)) の型。hwnd_cache は Win トークン無し(時計のみ)なので gate を外せば P |
| should_discard_imm_broken_cache | 937-956 | 20 | P* | 同上 |

4. 移せる(P+P*) 103 / 残る(O+E+G) 202 / 分割要(F) 615

### focus/cache.rs

1. 行数: 全体 216 / 本体 111 / テスト 105。ungated(focus/mod.rs「純粋サブモジュール」)。Linux でテスト済み(#[test] 6)
2. 内訳(本体行): P 31 / P* 80 / 分類対象外 0
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| DetectionSource + ttl_secs | 1-31 | 31 | P |  |
| FocusCache(new/get/insert) | 32-111 | 80 | P* | seam: `Instant::now()`/`elapsed()` を直接読む → now を引数/Clock trait 注入 |

4. 移せる(P+P*) 111 / 残る(O+E+G) 0 / 分割要(F) 0

### focus/class_names.rs

1. 行数: 全体 811 / 本体 374 / テスト 437。ungated。Linux でテスト済み(#[test] 23)
2. 内訳(本体行): P 374 / 分類対象外 0
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| 全関数(AppImeProfile 判定・is_tsf_native_window・is_own_ui_window・detect_app_kind 等) | 1-374 | 374 | P | Win トークン 0(`HWND` は文字列 "Chrome_RenderWidgetHostHWND" のみ)。そのまま core へ |

4. 移せる(P+P*) 374 / 残る(O+E+G) 0 / 分割要(F) 0

### focus/current.rs

1. 行数: 全体 241 / 本体 144 / テスト 97。ungated。Linux でテスト済み(#[test] 3)
2. 内訳(本体行): P 111 / P* 33 / 分類対象外 0
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| CurrentFocus 型・unfocused・update | 1-50 | 50 | P |  |
| CurrentFocus::update_with_process_name | 51-83 | 33 | P* | seam: 内部に `#[cfg(windows)]` / `#[cfg(not(windows))]` の分岐(get_process_name(pid) と root_hwnd_of(hwnd) の Win32 呼び出し)。process_name/root_hwnd を引数で受ければ cfg 分岐が消える |
| is_focused / FocusIdentity / changed_axes / any | 84-144 | 61 | P | hwnd は usize(HwndId ではない) |

4. 移せる(P+P*) 144 / 残る(O+E+G) 0 / 分割要(F) 0

### focus/kinds.rs

1. 行数: 全体 156 / 本体 73 / テスト 83。ungated。#[test] 10
2. 内訳(本体行): P 73 / 分類対象外 0
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| AppKind / FocusKind(+AtomicU8 load/store) | 1-73 | 73 | P |  |

4. 移せる(P+P*) 73 / 残る(O+E+G) 0 / 分割要(F) 0

### focus/thread_scope.rs

1. 行数: 全体 449 / 本体 290 / テスト 159。ungated 部分(1-313)と `#[cfg(windows)] mod win`(314-449)が同居。`SeenThreads` は `cfg(any(windows,test))`。Linux でテスト済み(#[test] 9)
2. 内訳(本体行): P 154 / O 136 / 分類対象外 0
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| ThreadScope / ClosedAssumption / should_assume_closed / closed_assumption / classify_thread_scope | 1-96 | 96 | P | const fn の純粋判定(BUG-163/ADR-212 P2) |
| ThreadIdentity / SeenThreads(LRU 履歴) | 209-266 | 58 | P | `cfg(any(windows,test))` で Windows ビルド時のみ本体にコンパイル |
| mod win: probe_focus_thread / read_thread_local_input_settings | 314-449 | 136 | O | GetWindowThreadProcessId/GetGUIThreadInfo/GetThreadTimes/OpenThread/SystemParametersInfoW。返す型 FocusThreadProbe{pid,tid,created_after_awase_ms,scope}・Option<bool>。引数に HWND と &mut SeenThreads(O に state が混ざる: seen の記録は core 側へ) |

4. 移せる(P+P*) 154 / 残る(O+E+G) 136 / 分割要(F) 0

### focus/classifier.rs

1. 行数: 全体 766 / 本体 538 / テスト 228。gate: focus/mod.rs `#[cfg(windows)] pub mod classifier`。Win32 トークンは `unsafe` 3 と `crate::ime::get_foreground_window_class` のみ(std::fs/toml/RwLock が主体)。#[test] 9(ImmCapabilityStore)は Linux 非実行
2. 内訳(本体行): P 67 / P* 230 / O 91 / E 74 / G 47 / 分類対象外 29
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| INPUT_RELAY_APPS(static OnceLock<RwLock>) + input_relay_apps_cell/snapshot | 14-44 | 31 | G | CLAUDE.md が明示する唯一の例外 static。read_ime_state_fast(self 無し)用 |
| enum ImmCapability / InjectionHint | 46-75 | 30 | P |  |
| matches_override_entry | 77-88 | 12 | P |  |
| input_site_fallback_matches | 90-109 | 20 | P* | seam: `crate::ime::get_foreground_window_class()` を引数化 |
| struct ForceOverrides | 111-122 | 12 | P |  |
| ForceOverrides::new | 124-139 | 16 | G | 副作用: プロセスグローバル INPUT_RELAY_APPS を書く |
| check_app_override | 141-166 | 26 | P* | seam: `classify::get_process_name(process_id)`(OpenProcess)を呼ぶ → process_name を引数で |
| is_app_disabled / input_relay_apps | 168-180 | 13 | P |  |
| injection_hint | 182-207 | 26 | P* | seam: get_process_name + unsafe な InputSite フォールバック(前景クラス取得) |
| ImmCapabilityStore(new/get/learn/clear/record_null_probe/clear_pending_unavailable) | 209-324 | 116 | P* | BUG-56 のデバウンス(2 回連続)は純粋ロジック。seam: learn/clear が即 `self.save()`(std::fs 書き込み) → 永続化 port |
| ImmCapabilityStore::load | 325-389 | 65 | O | cache.toml 読み(std::fs+toml)。Win32 ではないが I/O |
| ImmCapabilityStore::save | 391-412 | 22 | E | cache.toml 書き込み |
| len(test専用) / count_imm_capability_entries | 413-422 | 10 | P* |  |
| save_section | 424-464 | 41 | E | cache.toml の他セクション保持の atomic 書き込み |
| InjectionModeStore(new/has_tsf/learn_tsf) | 466-497 | 32 | P* | seam: base_dir ファイル永続化 |
| InjectionModeStore::load | 499-524 | 26 | O |  |
| InjectionModeStore::save | 526-536 | 11 | E |  |

4. 移せる(P+P*) 297 / 残る(O+E+G) 212 / 分割要(F) 0

### focus/classify.rs

1. 行数: 全体 245 / 本体 245 / テスト 0。gate: `#[cfg(windows)] pub mod classify`。#[test] 0
2. 内訳(本体行): P 48 / O 75 / F 105 / 分類対象外 17
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| consts + ClassifyResult / ClassifyReason(+Display) | 12-59 | 48 | P |  |
| classify_focus(hwnd) | 62-166 | 105 | F | F: GetWindowLongW(EXSTYLE/STYLE)・GetClassNameW・ImmContextGuard(O)の直後に判断表(WS_EX_NOIME / ES_READONLY / 既知テキスト・非テキストクラス名リスト / XamlExplorerHostIslandWindow)。分割: Windows が `{ex_style, style, class_name}` を返し、core `classify_by_style_and_class(...) -> Option<ClassifyResult>`(MSAA が必要なら None)。class_names.rs と別に類似のクラス名表を持つ点も統合候補 |
| get_class_name_string | 168-181 | 14 | O | GetClassNameW → String |
| get_window_process_id | 183-191 | 9 | O | GetWindowThreadProcessId → u32 |
| root_hwnd_of | 193-207 | 15 | O | GetAncestor(GA_ROOT) → usize |
| get_process_name | 209-245 | 37 | O | OpenProcess/QueryFullProcessImageNameW → String(失敗時空) |

4. 移せる(P+P*) 48 / 残る(O+E+G) 75 / 分割要(F) 105

### focus/hwnd_cache.rs

1. 行数: 全体 115 / 本体 115 / テスト 0。gate: `#[cfg(windows)] pub mod hwnd_cache`。**Win32 トークン無し**(`crate::hook::current_tick_ms` と tuning のみ)。#[test] 0 — 回帰は focus_tracking.rs の 8 本が間接的に保持
2. 内訳(本体行): P 36 / P* 79 / 分類対象外 0
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| HwndImeSnapshot 型 | 1-36 | 36 | P | hwnd は usize(HwndId 化の余地) |
| HwndImeCache(new/save/restore) | 37-115 | 79 | P* | seam: `hook::current_tick_ms()` を 2 箇所で直接読む → 時計注入。キーは (pid, class_name)、hwnd は snapshot の値として保持(BUG-128/ADR-165 の hwnd 一致判定の材料) |

4. 移せる(P+P*) 115 / 残る(O+E+G) 0 / 分割要(F) 0

### focus/imm_learning.rs

1. 行数: 全体 84 / 本体 84 / テスト 0。gate: cfg(windows)
2. 内訳(本体行): F 77 / 分類対象外 7
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| learn_imm_capability_on_focus | 8-84 | 77 | F | F: 学習ポリシー(Win32 以外は対象外・process_name 空は諦める・既学習スキップ)の判断が `crate::imm::get_ime_wnd(hwnd)`(ImmGetDefaultIMEWnd)と同居。process_name 取得の評価タイミングを呼び出し元(focus_tracking の process_name 横取り)が前提にしている。分割: O `ime_wnd_is_null: bool` を返し、core が record_null_probe / clear_pending を選ぶ |

4. 移せる(P+P*) 0 / 残る(O+E+G) 0 / 分割要(F) 77

### focus/kind_classifier.rs

1. 行数: 全体 82 / 本体 82 / テスト 0。gate: cfg(windows)
2. 内訳(本体行): P* 8 / F 67 / 分類対象外 7
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| FocusKindResolution | 7-14 | 8 | P* |  |
| resolve_focus_kind | 16-82 | 67 | F | F: 優先順位判断(config override → cache → engine 処理中はスキップ → classify)と `run_with_timeout(300ms, classify_focus(HWND))`(Win32 別スレッド)が同居。doc は「純粋関数(副作用なし)」だが実際は違う(古い記述)。分割: core が override/cache/engine_busy で早期決定、残ったときだけ Windows に classify を依頼 |

4. 移せる(P+P*) 8 / 残る(O+E+G) 0 / 分割要(F) 67

### focus/msaa.rs

1. 行数: 全体 151 / 本体 151 / テスト 0。gate: cfg(windows)
2. 内訳(本体行): P 80 / F 55 / 分類対象外 16
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| enum MsaaRole + from_u32 / is_text_input / is_non_text | 17-96 | 80 | P | ロール表(Win トークン無し) |
| msaa_classify(hwnd) | 97-151 | 55 | F | F(小): AccessibleObjectFromWindow/get_accRole(COM, O)の結果を MsaaRole で TextInput/NonText に写す。O は `Option<u32> role_id` を返すだけにする |

4. 移せる(P+P*) 80 / 残る(O+E+G) 0 / 分割要(F) 55

### focus/probe.rs

1. 行数: 全体 72 / 本体 72 / テスト 0。gate: cfg(windows)
2. 内訳(本体行): P* 18 / O 50 / 分類対象外 4
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| FocusSnapshot | 3-20 | 18 | P* | seam: `hwnd_addr: usize` を持ち `hwnd() -> HWND` 提供。HwndId 返しにすれば P(`unsafe impl Send` も不要になる) |
| run_focus_probe_async | 22-45 | 24 | O | offload + GetGUIThreadInfo(150ms) → FocusSnapshot{hwnd_addr,process_id,class_name} |
| read_focus_snapshot | 47-72 | 26 | O | 同上の同期版(run_with_timeout 300ms)。2 関数がほぼ同一本体(重複) |

4. 移せる(P+P*) 18 / 残る(O+E+G) 50 / 分割要(F) 0

### focus/tracker.rs

1. 行数: 全体 441 / 本体 351 / テスト 90。gate: `#[cfg(windows)] pub(crate) mod tracker`。Win32 トークン無し(`focus/uia::SendableHwnd` と classifier の Win 依存を経由して gate される)。#[test] 6 は Linux 非実行
2. 内訳(本体行): P 38 / P* 311 / 分類対象外 2
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| struct FocusTracker | 1-35 | 35 | P* | seam: `uia_sender: Option<Sender<SendableHwnd>>` が Win HWND ラッパー型。他フィールドは P/P* 型の集約 |
| new/クエリ/injection_hint(_for) | 36-119 | 84 | P* | seam: ForceOverrides::injection_hint 内の get_process_name |
| update / update_with_process_name | 120-176 | 57 | P* | seam: CurrentFocus::update_with_process_name の Win32(process_name/root_hwnd) |
| should_log_demotion / apply_learned_imm_capability | 178-215 | 38 | P | 純関数(BUG-111) |
| cache_get/override_check/is_app_disabled/cache_insert/save_ime_state/restore_ime_state/imm_* 委譲 | 217-336 | 120 | P* | seam: override_check → get_process_name、save_ime_state → hook 時計(hwnd_cache)、imm_* は永続化 port |
| set_uia_sender / try_send_uia | 337-351 | 15 | P* | seam: SendableHwnd。UIA 結果が破棄されるため(BUG-12)機能ごと削除候補 |

4. 移せる(P+P*) 349 / 残る(O+E+G) 0 / 分割要(F) 0

### focus/uia.rs

1. 行数: 全体 306 / 本体 306 / テスト 0。gate: cfg(windows)。COM 16 トークン。**この経路の結果は handle_wm_focus_kind_update で破棄される(BUG-12)**
2. 内訳(本体行): P 5 / P* 15 / O 42 / F 131 / G 80 / 分類対象外 33
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| SendableHwnd | 28-42 | 15 | P* | seam: HWND ラッパー(unsafe impl Send) |
| UiaClassifyResult | 43-47 | 5 | P |  |
| resolve_app_kind | 49-70 | 22 | F | F(小): CurrentFrameworkId(COM)→ "Win32"\|"WinForm" / "DirectUI"\|"XAML"\|"WPF" の写像 |
| check_value_pattern | 72-95 | 24 | O | ValuePattern.IsReadOnly を読み TextInput/NonText |
| check_text_pattern | 97-114 | 18 | O |  |
| check_control_type | 116-158 | 43 | F | F: ControlType(COM)を読んだ直後に 18 種の NonText リスト |
| uia_classify_focus | 160-225 | 66 | F | F: プローブ順(Value → Text → ControlType)の決定木。分割: O が UiaFacts を返し core が判定 |
| spawn_uia_worker | 227-306 | 80 | G | 専用スレッド + COM 初期化 + PostMessage(WM_FOCUS_KIND_UPDATE)。wparam に FocusKind/AppKind を bit pack |

4. 移せる(P+P*) 20 / 残る(O+E+G) 122 / 分割要(F) 131

### observer/focus_observer.rs

1. 行数: 全体 39 / 本体 39 / テスト 0。gate: lib.rs:72-73 `#[cfg(windows)] pub mod observer`(observer/ 全体)
2. 内訳(本体行): O 24 / 分類対象外 15
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| read_os_modifiers | 16-39 | 24 | O | 返す型: awase::engine::ModifierState{ctrl,alt,shift,win}(core 型)。Ctrl/Shift は hook::is_physical_key_down(static)、Alt/Win は GetAsyncKeyState |

4. 移せる(P+P*) 0 / 残る(O+E+G) 24 / 分割要(F) 0

### observer/gji_observer.rs

1. 行数: 全体 72 / 本体 72 / テスト 0。gate: observer/ 全体(Win32 トークン無し: hook::current_tick_ms と tsf::observer の atomic のみ)
2. 内訳(本体行): P 18 / P* 54 / 分類対象外 0
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| struct GjiBlacklistObservation | 1-18 | 18 | P | observer_poll_value: Option<bool> / input_mode_correction: Option<InputModeState> |
| observe_gji_after_focus | 19-72 | 54 | P* | seam: 時計(hook::current_tick_ms)と `tsf::observer::tsf_obs()`(gji_last_io_ms/gji_attach_ms の atomic)を引数 {now, last_io, attach_ms} にすれば純粋。実体は判断(O ではない) |

4. 移せる(P+P*) 72 / 残る(O+E+G) 0 / 分割要(F) 0

### observer/ime_observer.rs

1. 行数: 全体 543 / 本体 318 / テスト 225。gate: observer/ 全体。#[test] 9 は Linux 非実行(これらは純粋なので移せば Linux で回る)
2. 内訳(本体行): P 39 / P* 225 / F 38 / 分類対象外 16
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| ImeObs / ImeUpdate / PollOutcome | 15-53 | 39 | P | ImeUpdate は純粋な更新命令(core へ) |
| impl ImeSnapshot { classify_poll_outcome / input_mode_from_romaji_flag / input_mode_from_conversion } | 54-157 | 104 | P* | seam: レシーバ `crate::ime::ImeSnapshot`(ime.rs=cfg(windows) の型・中身は Option<bool>/Option<u32>/String の所有データ)を core へ移す |
| classify_ime_snapshot | 159-248 | 90 | P* | 同上。BUG-106/BUG-158 の判断本体 |
| poll_and_classify_ime | 249-286 | 38 | F | F(薄い): read_ime_state_full_with_timeout(O, 300ms) + hook::current_tick_ms + is_own_ui_window 判定 + classify。O と P* の接着だけなので分割は機械的 |
| classify_fetched_snapshot | 513-543 | 31 | P* | async drain 後の同期ラッパー。seam: ImeSnapshot |

4. 移せる(P+P*) 264 / 残る(O+E+G) 0 / 分割要(F) 38

### observer/kana_lock.rs

1. 行数: 全体 40 / 本体 40 / テスト 0。gate: observer/ 全体
2. 内訳(本体行): O 36 / 分類対象外 4
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| read_kana_lock | 5-20 | 16 | O | 返す型: KanaLockReading{On,Off}(core: awase::engine::kana_input_warn)。GetKeyState(VK_KANA) |
| foreground_class_name | 21-40 | 20 | O | String(診断ログ用)。GetForegroundWindow→GetClassNameW |

4. 移せる(P+P*) 0 / 残る(O+E+G) 36 / 分割要(F) 0

### observer/layout_observer.rs

1. 行数: 全体 56 / 本体 56 / テスト 0。gate: observer/ 全体
2. 内訳(本体行): P 18 / O 33 / 分類対象外 5
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| struct ThreadLanguage | 6-23 | 18 | P | {japanese: Option<bool>, tid, lang_id} |
| read_thread_language(Option<HwndId>) | 24-56 | 33 | O | GetWindowThreadProcessId + GetKeyboardLayout。入力は既に HwndId(to_hwnd 経由)。判定 classify_layout_language は state/ の純関数 |

4. 移せる(P+P*) 18 / 残る(O+E+G) 33 / 分割要(F) 0

### focus/mod.rs + observer/mod.rs

1. 行数: 全体 44 / 本体 44 / テスト 0。mod 宣言のみ(focus/mod.rs 33 + observer/mod.rs 11)
2. 内訳(本体行): G 44 / 分類対象外 0
3. 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|:-:|---|
| mod 宣言 | 1-44 | 44 | G |  |

4. 移せる(P+P*) 0 / 残る(O+E+G) 44 / 分割要(F) 0


## 3. message_handlers.rs: WM ごとの入口(G)と core Event への翻訳

dispatcher は `app/mod.rs` の `run_message_loop` の `match msg.message`(app/mod.rs:505-607)。`app: &mut Runtime` を受け取るハンドラ系関数は 32 箇所(grep)で、**借用は app/mod.rs 側の `with_app` 1 回**。ハンドラ自身が `with_app` を借りるのは下表の 3 本だけ(合計 13 呼び出し)。

| WM / 入口 | ハンドラ | core Event / 翻訳先 | ハンドラ内で借りる with_app |
|---|---|---|---:|
| WM_KEY_FROM_HOOK | handle_wm_key_from_hook | RawKeyEvent → `Runtime::process_key_event`(deliver_key_event 経由) | 0 |
| WM_TIMER(エンジン用) | handle_wm_timer 汎用 arm | `Engine::on_timeout(timer_id, &ctx)` | 0 |
| WM_TIMER(IME_OFF_RESCUE) | 同 arm | 保留 RawKeyEvent の再投入(KeyOrigin::ImeOffRescueReplay) | 0 |
| WM_TIMER(POWER_RESUME) | 同 arm | ContextChange::InputLanguageChanged | 0 |
| WM_TIMER(その他 8 種) | 同 arm | Event 化されず platform/executor メソッド直呼び(IME refresh・output guard・TSF probe・TSF gate・GJI long idle・focus resync・hook watchdog・watchdog canary check) | 0 |
| WM_DRAIN_OUTPUT_QUEUE | handle_wm_drain_output_queue | 遅延 RawKeyEvent の再投入 + 遅延 `Engine::on_timeout` | **5** |
| WM_EXECUTE_EFFECTS | handle_wm_execute_effects | executor.drain_deferred → dispatch_outcomes(Cmd の実行) | 0 |
| WM_ASYNC_IME_APPLY_COMPLETE | handle_wm_async_ime_apply_complete | `Runtime::on_ime_apply_complete(open, outcome, generation, reason)`(ImeEvent の apply 完了) | 0 |
| WM_PANIC_RESET | handle_wm_panic_reset | ImeEvent::PanicReset(apply_panic_reset) | 0 |
| WM_IME_KIND_CHANGED | handle_wm_ime_kind_changed → sync_ime_kind_from_observation | IME 種別同期(Event 化されていない。platform.output / GJI FSM / engine へ直接) | 0 |
| WM_INPUTLANGCHANGE | handle_wm_inputlangchange | ContextChange::InputLanguageChanged | 0 |
| WM_WTSSESSION_CHANGE | handle_wts_session_change | ContextChange::FocusChanged(ロック時) + latch 解放 | 0 |
| WM_POWERBROADCAST | handle_wm_powerbroadcast | なし(timer 設定のみ) | 0 |
| WM_FOCUS_KIND_UPDATE | handle_wm_focus_kind_update | **なし(破棄、BUG-12)** | 0 |
| WM_HOTKEY(2 種) | handle_wm_hotkey_toggle / _focus_override | toggle_engine / toggle_app_override | 0 |
| WM_APP(トレイ) | handle_wm_app_tray | なし(UI)。sent message 経由のため自前借用 | ref 1 |
| WM_COMMAND(トレイ) | handle_wm_command | toggle_engine / switch_layout / force_engine_on 等(UI コマンド) | 6 + ref 1 |
| WM_RELOAD_CONFIG | handle_wm_reload_config | reload_config()(app/) | 0 |
| WM_KANA_LOCK_WARNING_CHANGED / WM_HOOK_IME_MODE_DIAGNOSTIC / WM_DUPLICATE_INSTANCE / TaskbarCreated / WM_DUMP_JOURNAL | 各 handle_* | なし(UI・診断) | 0 |

観察: 「core Event に翻訳される入口」は実質 **キー(2 経路)・タイマー(エンジン)・IME apply 完了・panic reset・コンテキスト無効化 3 種** だけで、残りは platform/executor メソッドの直呼びか UI。新 crate 側に `Event` として切り出す価値があるのは前者。

## 4. focus/・runtime の HWND をキーにしているもの(seam: HwndId への置き換え)

`state::ime_event::HwndId(pub usize)` が既にあり、`HwndId` は 168 箇所で使われている(state/・journal・ime_controller 等)。この領域は「同じ値を 3 通り(HWND / 生 usize / HwndId)で持つ」状態:

| 持ち方 | 場所 | 備考 |
|---|---|---|
| `HWND` 型 | `ClassifiedFocus.hwnd`、`classify_focus`、`msaa_classify`、`resolve_focus_kind`、`learn_imm_capability_on_focus`、`probe_focus_thread`、`FocusSnapshot::hwnd()`、`uia::SendableHwnd`、`handle_wm_focus_kind_update`、`handle_wm_app_tray` | 13 関数/型、9 ファイル。大半は O または F(Win32 を呼ぶ)なので Windows 側に残り、境界でだけ HwndId を受け取ればよい |
| 生 `usize` | `CurrentFocus.hwnd/root_hwnd`、`FocusIdentity.hwnd`、`HwndImeSnapshot.hwnd`(+ `HwndImeCache::save(old_hwnd)`)、`FocusSnapshot.hwnd_addr`、`should_restore_tsf_cache_on(new_hwnd)`、`root_hwnd_of(usize)` | ここが P/P* 側の seam。HwndId に統一すれば型で「別物の usize」との取り違えが防げる |
| `HwndId` | `ImeEvent::{FocusChanged,FocusHwndUpdated,Initial*}`、`journal::FocusEndpoint.hwnd`、`layout_observer::read_thread_language(Option<HwndId>)` | 既に core 側(state/)の語彙 |
| 相互変換の cast | focus_tracking.rs:288,461,632,682 / classify.rs:205 / kind_classifier.rs:62,65 / probe.rs:18,39,67 / message_handlers.rs:1079 | `.0 as usize` / `HWND(x as *mut _)` が 11 箇所に散在。`HwndId::to_hwnd` / `From<HWND>`(ime_event.rs:36-)に寄せられる |

**キャッシュのキー**: `HwndImeCache` のキーは `(pid, class_name)`、hwnd は値(BUG-128/ADR-165 の「同じインスタンスか」判定の材料)。`FocusCache`(cache.rs)のキーも `(pid, class_name)`。hwnd をキーにしているキャッシュは**無い**。したがって「WindowId に置き換える」対象はキーではなく**値・引数・比較**で、置き換え自体は型の機械的な付け替えで済む(ただし OS による hwnd 値の再利用(ABA)を世代で守る仕組みは無い。下の危険箇所 D9)。

## 5. observer/ が返す snapshot 型の一覧

| observer 関数 | 返す型 | 型の所在 | 備考 |
|---|---|---|---|
| `focus_observer::read_os_modifiers` | `awase::engine::ModifierState{ctrl,alt,shift,win}` | core | Ctrl/Shift は `hook::is_physical_key_down`(hook の static)、Alt/Win は `GetAsyncKeyState` |
| `gji_observer::observe_gji_after_focus` | `GjiBlacklistObservation{observer_poll_value, input_mode_correction}` | observer 内(P) | 実体は判断。入力に時計と `tsf_obs()` を直接読む(O ではなく P*) |
| `ime_observer::poll_and_classify_ime` | `ImeUpdate{is_japanese_ime, observer_poll, increment_miss_count, clear_force_on_panic_reset, new_input_mode, new_prev_conversion_mode}` | observer 内(P) | 内部で `ime::ImeSnapshot` を読む(下) |
| (`ime::read_ime_state_full*`) | `ime::ImeSnapshot{is_japanese_ime, ime_on, is_romaji, conversion_mode, is_tsf_native, focused_class, probe_timed_out}` | ime.rs:467(cfg(windows)) | 実質の O の返り値。全フィールド Option/所有型 → そのまま core に置ける |
| `kana_lock::read_kana_lock` | `awase::engine::kana_input_warn::KanaLockReading` | core | |
| `kana_lock::foreground_class_name` | `String` | std | 診断ログ用 |
| `layout_observer::read_thread_language` | `ThreadLanguage{japanese: Option<bool>, tid, lang_id}` | observer 内(P) | 入力は `Option<HwndId>` |
| (focus/) `probe::read_focus_snapshot` | `FocusSnapshot{hwnd_addr, process_id, class_name}` | focus/probe.rs | |
| (focus/) `thread_scope::win::probe_focus_thread` | `FocusThreadProbe{pid, tid, created_after_awase_ms, scope}` | thread_scope.rs | `&mut SeenThreads` を引数に取る(O が state を更新) |
| (focus/) `classify::classify_focus` / `msaa_classify` / `uia_classify_focus` | `ClassifyResult{kind, reason}` / `UiaClassifyResult{focus_kind, app_kind}` | focus/ | O+判断が混在(F) |

型の方向: **返り値の型はほぼ全て既に OS 非依存**(Option<bool>/u32/String/core enum)。observer を「facts を返す dumb な読み取り」にするのに新しい型はほとんど要らず、`ImeSnapshot` を core 側に移し、F の `classify_focus` / `msaa_classify` / `uia_classify_focus` / `resolve_focus_kind` から判断を剥がして `…Facts` を返させるのが主な作業。

## 6. Windows 側依存の種類と件数

「出現行数」は本体(テスト・コメント行を除く)で `grep` した行数で、関数数ではない。

| 依存 | message_handlers | focus_tracking | focus/ 合計 | observer/ 合計 | 補足 |
|---|---:|---:|---:|---:|---|
| `windows::` クレート直参照 | 12 | 1 | classify 6・msaa 4・uia 3・thread_scope(win) ほか | focus_observer 1・kana_lock 1・layout 2 | |
| `HWND` 型 | 3 | 2 | classify 5・msaa 2・uia 2・kind_classifier 3・imm_learning 2・probe 3・thread_scope(win) 3 | 0 | §4 |
| `with_app`(Runtime 借用) | 14 | 1(`spawn_local` 内) | 0 | 0 | layer_boundary_guard が focus/・observer/ での with_app を禁止 |
| `crate::hook::*`(hook の static/atomic) | 17 | 7 | hwnd_cache 2 | focus_observer 4・gji 1・ime_observer 1 | `current_tick_ms` が最多(時計 seam) |
| `crate::win32::*` | 5 | 0 | classify 1・kind_classifier 1・probe 3・uia 1 | 0 | `run_with_timeout` / `get_gui_thread_info_with_timeout` / `post_to_main_thread` |
| `tsf::observer::*`(`tsf_obs()` の atomic 読み) | 10 | 4 | 0 | gji_observer 2 | |
| グローバル static(OUTPUT_GATE / FOCUS_RESYNC / INPUT_DEFER / DRAIN_* / INPUT_RELAY_APPS) | 15 | 1 | classifier 1 | 0 | |
| `thread_local` 相当 | 1(`set_tsf_probe_snap`) | 0 | 0 | 0 | `with_app_ref` の BorrowError 回避 |
| `unsafe` | 23 | 6 | classify 10・uia 13・msaa 5・probe 5・imm_learning 3・classifier 3 | focus_observer 2・kana_lock 5・layout 3・ime_observer 2 | |
| COM(UIA / MSAA) | 0 | 0 | uia 16・msaa 3 | 0 | |
| SetTimer(`platform.timer.*`) | 9 | 0 | 0 | 0 | |
| PostMessage(`post_to_main_thread*`) | 6 | 0 | uia 1 | 0 | |
| tray / UI | 29 | 0 | 0 | 0 | トレイ・balloon・about |
| IMM(`crate::imm::` / `crate::ime::`) | imm 2・ime 5 | ime 1 | classify 1・imm_learning 1・classifier ime 1 | ime_observer 3・kana_lock 1 | |
| 時計(`current_tick_ms` / `Instant::now`) | 6 | 5 | hwnd_cache 2・cache.rs 1(`Instant`) | gji 1・ime_observer 1 | |
| std::fs / toml 永続化 | 0(bug report 診断の temp 書き込みと config1.db 読みのみ) | 0 | classifier(cache.toml: ImmCapabilityStore・InjectionModeStore・save_section) | 0 | Win32 ではないが I/O port が要る |

## 7. 領域の seam 一覧

| seam | 現れるファイル数 | 現れる関数(確認したもの) | 備考 |
|---|---:|---|---|
| **時計の注入**(`hook::current_tick_ms` / `Instant::now`) | 6 | HwndImeCache::save/restore、FocusCache::get/insert、observe_gji_after_focus、poll_and_classify_ime、record_focus_transition_if_changed、enter_focus_scope、advance_focus_tracking、notify_focus_hwnd_updated_if_needed、deliver_key_event、write_bug_report_diagnostics ほか | 最も安い seam。`tick_ms: u64` 引数化で足りる(`state::TickMs` が既にある) |
| **HWND / 生 usize → HwndId** | 12 | §4 の 13 関数/型 + 生 usize 6 箇所 | 型付けのみ。Windows 側 O/F は境界で変換 |
| **O を引数(facts)に出す**(判断の中に Win32 読み取りがある P*) | 4 | check_app_override・injection_hint・input_site_fallback_matches(classifier)、CurrentFocus::update_with_process_name(+tracker の update 2 本)、arm_post_bypass_if_matches | `process_name: &str` / `fg_class: &str` / `ForegroundScope` を引数に |
| **F の分割(O が facts を返し core が判断)** | 8 ファイル | classify_focus、msaa_classify、uia(resolve_app_kind/check_control_type/uia_classify_focus)、resolve_focus_kind、learn_imm_capability_on_focus、classify_focus_probe、advance_focus_tracking、on_focus_process_changed、deliver_key_event(+consume_*)、handle_wm_timer の 2 arm、sync_ime_*(2)、drain、bug report arm | F 計 1771 行。うち on_focus_process_changed(307)・timer 汎用/watchdog(148)・drain(123)・deliver_key_event(116)・advance_focus_tracking(97)・classify_focus(105)・classify_focus_probe(92) が上位 |
| **`&mut Runtime` / `with_app` 借用** | 2(message_handlers と focus_tracking) | message_handlers: ハンドラ系 32 箇所 + 自前借用 13 呼び出し(3 関数)。focus_tracking: `impl Runtime` の `&self`/`&mut self` 18 | 新 crate の Runtime 型を core に移すかどうかが前提。移さないなら P* は「引数の substate 借用」で足りる |
| **永続化 port**(std::fs/toml) | 1(classifier) | ImmCapabilityStore.learn/clear/new、InjectionModeStore.learn_tsf、save_section | Win32 ではない。trait `CachePersist` 注入か、core 側に残して `base_dir` だけ Windows から渡す |
| **グローバル static** | 3 | DRAIN_*(message_handlers)、INPUT_RELAY_APPS(classifier、CLAUDE.md が明示する唯一の例外)、OUTPUT_GATE/FOCUS_RESYNC/INPUT_DEFER(timer・drain arm) | ADR-164 のフェーズ計画に従う対象 |
| **`tsf_obs()` atomic 読み** | 3 | sync_ime_kind_from_observation、presync_applied_open_on、observe_gji_after_focus、current_bug_report_ime_kind、on_focus_process_changed(table_ime_kind) | snapshot 構造体 `TsfObsSnapshot{active_ime_kind, detected, gji_last_io_ms, gji_attach_ms, ...}` を O が返す |
| **cfg(windows) の本体内分岐** | 1 | CurrentFocus::update_with_process_name | seam を引数化すれば消える |

## 8. 新 crate への移動候補の順序案(依存が少ない順)

1. **ungated の focus/ 5 ファイル**(class_names / kinds / thread_scope 純粋部 / current / cache): 計 743 P + 113 P*。Win トークン 0(current の cfg 分岐と cache の `Instant` だけ要 seam)。import の付け替えだけで移る。テスト 881 行が既に Linux で動いている。
2. **core 型だけの純関数**(seam 不要): message_handlers の `encode_outcome` / `decode_outcome` / `encode_apply_wparam` / `decode_reason` / `ime_toggle_kind_str` / `gji_composition_mode_str` / `parsed_key_combo_label` / `bug_report_keyboard_model`(約 100 行 + テスト約 70 行)、`msaa.rs::MsaaRole` 表(80 行)、`classify.rs` の `ClassifyResult/Reason`(48 行)、focus_tracking の `From<&FocusIdentity> for FocusEndpoint`。
3. **hwnd_cache.rs + 2 本の cache 判定**(`should_restore_tsf_cache_on` / `should_discard_imm_broken_cache`): 時計を注入するだけで gate を外せる(Win トークン無し)。focus_tracking.rs の #[test] 8 本が Linux で動くようになる。BUG-128/ADR-165 の回帰が Linux で守られる。
4. **ime_observer.rs の `classify_*` 群**(P* 225 行 + 型 39 行 + テスト 225 行): `ime::ImeSnapshot` を core に移すだけ(所有型のみ)。BUG-106/BUG-158 の判断本体。+ `gji_observer::observe_gji_after_focus`(時計と atomic を引数化)。
5. **tracker.rs / classifier.rs**: `FocusTracker`(311 P*)、`ForceOverrides`、`ImmCapabilityStore`(BUG-56 のデバウンス)。前提: UIA 経路の削除(`SendableHwnd` を除く)、`get_process_name` の引数化、永続化 port。
6. **F の分割(facts を返す O を作る)**: classify_focus → msaa_classify → uia(削除しないなら) → resolve_focus_kind → learn_imm_capability_on_focus。いずれも「O が facts、core が判断」で機械的。
7. **message_handlers の P* と bug report 組み立て**: gji/msime の summary 2 本(P* 約 165 行)、`arm_post_bypass_if_matches`、deliver_key_event の `route_key` 抽出。
8. **最後: focus_tracking の F 群**(`classify_focus_probe` → `advance_focus_tracking` → `on_focus_process_changed`): 危険箇所 D1/D2/D7 に直結し、guard テスト 29 本のテキスト検査も書き換えが要る。journal replay(`docs/journal-replay-guide.md`)で先に回帰網を張ってから。
9. **移さない**: Win32 を読む O(`thread_scope::win`・`classify` の 4 関数・`probe`・`kana_lock`・`focus_observer`・`layout_observer`・`process_resource_snapshot`)、E(`cancel_composition`・`post_async_ime_apply_complete`・write_bug_report_diagnostics・DispatchMessageW)、WM 入口(G)。

**移すより削除を先に検討する対象**: UIA 経路(`focus/uia.rs` 306 行 + `handle_wm_focus_kind_update` 40 行 + `try_send_uia`/`set_uia_sender` の委譲 + bootstrap.rs:1317 のワーカー起動 + `WM_FOCUS_KIND_UPDATE`)。結果は BUG-12 以来使われていない。約 400 行、COM ワーカースレッド 1 本。

## 9. 危険箇所(分けると隙間ができる所)

| # | 箇所 | 何が壊れうるか | 関連 |
|---|---|---|---|
| D1 | `on_focus_process_changed` 内の `spawn_local(async { read_ime_state_full_async().await; with_app(admit_epoch_in_app(ticket, ...)) })`(focus_tracking.rs:819-853) | **await をまたぐ失効**。フォーカスが変わった後に完了した probe が High 観測を書くと「Engine OFF カスケード」(コメント記載)。いまは spawn 時の `focus_fence()`(epoch+hwnd)を `ImmLikeTicket` に入れ、完了側で照合している。判断を plan 化して実行側に渡す分割をすると、ticket の発行を plan 側(core)、照合を apply 側(core)に置かないと、Windows 側の async が古い fence を持ち越す | ADR-106 決定3、BUG-102、state/probe_admission.rs |
| D2 | bootstrap の順序不変条件(`establish_initial_focus_scope` → `advance_focus_tracking` → `enter_focus_scope` → `sync_initial_focus_fence` → `sync_initial_app_policy` → `sync_initial_focus_hwnd`) | 順序を崩すと fence の epoch/hwnd が食い違い(BUG-102)、最初の IME 観測前に belief を書く(ADR-102 決定3-b)、`app_policy` が既定 Standard のまま(BUG-114)。architecture_guard が**関数名のテキスト位置**で固定(`establish_initial_focus_scope_*` 4 本、`initial_*_event_only_touches_*` 3 本、`focus_hwnd_updated_dispatch_is_skipped_during_bootstrap`、`app_disable_invalidate_engine_context_is_skipped_during_bootstrap`)。移動で関数名・ファイル名が変わるとガードが黙って空振りするので、同じ PR で意味ベース(journal replay)に置き換える必要がある | ADR-102/106/134、BUG-102/114 |
| D3 | `deliver_key_event` の分岐順序(keymap latch → Nested → NonText → keymap 照合 → post_bypass → engine) | latch を先に見ないと Down/Up 非対称(KeyUp 回収と自動リピート抑制の両方を先頭に置いている)。`ImeOffRescueReplay` は NonText 早期 return の対象外(コードレビュー指摘3)。`route_key` を core に出すなら、latch の set(`consume_keymap_match`)と release(KeyUp 側)を**同じ純関数内で対称に**保つこと。物理キー押下ラッチ family と同型 | ADR-114 決定2、BUG-131/132、architecture_guard `deliver_key_event_*` 2 本 |
| D4 | defer/replay の 2 窓口: `handle_wm_timer` 汎用 arm(gate active なら `deferred_engine_timers` へ)と `handle_wm_drain_output_queue`(os_id 照合つき replay) | 片側だけ移す/変えると「drain で chord パートナーが処理される前に PendingChar が Idle に遷移」(K+右親指=の が き になる)や「新規タイマーを早期発火して文字順が狂う(というのは → とはいうの)」が再発。gate 種別追加の配線漏れ(ADR-123→128)と同型 | ADR-156、BUG-77、fix-requires-evidence の defer/replay 行 |
| D5 | `handle_wm_drain_output_queue` の 5 回の `with_app` 借用と再入ガード(`DRAIN_PENDING`/`DRAIN_RERUN_PENDING`) | 借用の間にモーダルポンプ(`PumpContext::Nested`)や hook が割り込める。1 借用にまとめると効果の順序が変わる。`with_app` が None を返したとき `take_all()` 済みキューを INPUT_DEFER に戻す処理(Opus 指摘で追加)を落とすとキー消失。static を Runtime 内に移すときはこの再入契約ごと移す | ADR-156、WM_DRAIN の再入(2026-08-26 修正) |
| D6 | engine スレッド上の同期 Win32 読み取りを core が直接呼ぶ形にすると BUG-34 型の打鍵消失 | 今は `read_focus_snapshot`(`run_with_timeout` 300ms)・`resolve_focus_kind`(同 300ms)・`poll_and_classify_ime`(300ms)が O 側にタイムアウトを閉じ込めている。core が observer trait を同期で呼ぶ設計にするなら、**タイムアウト隔離は Windows の trait 実装側に置く**(core に漏らさない)。さらに `current_bug_report_diagnostics` / `process_resource_snapshot` / レジストリ読みは WM_COMMAND のメッセージループスレッドで同期実行(タイムアウト隔離なし) | BUG-34、CLAUDE.md「Concurrency model」`run_with_timeout` |
| D7 | `ImeKindId`/IME 種別が推測値(INV-45): `sync_ime_kind_from_observation` は `active_ime_kind()` が未検出時に `MicrosoftIme` を安全デフォルトとして返すため `detected` を一緒に見ている(message_handlers.rs:974,997)。`on_focus_process_changed` は `tsf_obs().table_ime_kind()`(Option)から `closed_assumption` を作る(focus_tracking.rs:739-741)。`closed_assumption` は測定済み条件(SPI thread-local + 測定済み IME)に限定 | snapshot に分けるとき `kind` だけを運んで `detected` を落とすと、GJI ユーザーの起動時に MS-IME 用処理(警告ポップアップ)が誤発動する。facts には `Option<ImeKindId>`(= 未検出が表現できる型)を使う | ADR-089 INV-45、ADR-212 P2、BUG-163 |
| D8 | belief 書き込み点の指定関数と件数ガード | `apply_hwnd_cache_restore` / `assume_closed_for_new_thread` / `reset_stale_ime_on_for_imm_broken` / `record_confirmed`(presync)は state/ 側に残るので dylint は維持されるが、`architecture_guard` の「構築箇所数をファイル名で固定」テスト(focus_tracking.rs を数える箇所が少なくとも 3 つ: 3626・3687・3848 行付近)は移動で数え直しが要る | ime-belief-architecture.md、dylint ime_event_guard |
| D9 | hwnd 値の再利用(ABA)と `(pid, class)` キーの粒度 | HwndImeCache は `(pid,class)` キー + hwnd 値一致で TsfNative の復元を許可(BUG-128)。hwnd を HwndId に型付けしても OS が値を再利用する問題は解決しない(世代は無い)。BUG-11/12(`(pid,class)` キャッシュと UIA 結果の粒度不一致)が UIA 適用を止めている根拠なので、UIA を残す場合は hwnd 粒度のキャッシュ設計が先 | ADR-165、BUG-11/12/128 |
| D10 | `consume_post_bypass`/`arm_post_bypass_if_matches` の `foreground_scope()` の取得時点 | 武装時(Ctrl+J)と消費時(次のキー)の 2 点で O を呼び、前景が変わっていたら失効させる設計。scope を RawKeyEvent に載せて hook 時点で取る形に「改善」すると、消費時点の判定が消えて失効の意味が変わる | ADR-103 決定3 |
| D11 | `handle_wm_timer` 内 TSF_PROBE arm の `set_tsf_probe_snap` thread_local | `with_app_ref` が排他借用中に BorrowError になる回避策。借用構造(`&mut Runtime`)を変える場合に最初に壊れる前提。core に移すなら snapshot を引数で渡す形にして thread_local を消せる | 該当コメント(message_handlers.rs:503-509) |

## 10. 未確認の点

- `hook::current_tick_ms` / `hook::is_physical_key_down` / `hook::os_idle_ms` / `tsf::observer::tsf_obs()` の実体が純粋な atomic 読みか Win32 呼び出しかは**未確認**(コメントと呼び出し形から atomic と判断。時計の seam 扱いにしたのはこの前提)。
- `ime::ImeSnapshot` を `ime.rs` 以外(key_pipeline 等)がどう使っているかは**未確認**。core に移す場合の影響範囲は未調査。
- UIA 経路の削除可否(観測目的の利用が残っていないか、BUG-12 の「UIA FocusChanged 購読」案が将来計画に生きているか)は**未確認**。architecture_guard の `uia_async_focus_kind_handler_does_not_write_belief` が固定していることのみ確認。
- guard テスト 29 本は「`message_handlers`/`focus_tracking`/`src/focus/`/`observer/` の文字列を本文に含む関数」を機械抽出した候補で、すべてが移動で壊れるとは限らない(確認は件名レベル)。
- `docs/layer-boundaries.md`(419 行)は全文を読んでいない。参照したのは `layer_boundary_guard.rs` の「observer/・focus/ で with_app を呼ばない」規則のみ。
- 本体行数は「関数範囲(doc コメント込み)」で数えた近似で、F の中の純粋部分を P に再配分すると P+P* は増える(F を完全に分割した場合の上限は約 40% + 28% × 純粋部分の割合)。純粋部分の割合は**未計測**。
- テスト行(1622 行)は T として合計のみ。Linux に移した後に回せるかは、**関数の移動先に依存**(例: ime_observer の 9 本・focus_tracking の 8 本は移せば回る。classifier の 9 本は永続化 port が要るので tempdir 前提のまま std::fs で回せる見込み)。
