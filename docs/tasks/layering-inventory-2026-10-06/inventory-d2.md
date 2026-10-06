# 棚卸し D2: app/ ・ tray ・ autostart ・ output/ ・ src 直下のその他

対象: develop 9df983e4。担当は `crates/awase-windows/src` の `app/`(3 ファイル)、`output/`(10 ファイル)、`tray.rs`、`autostart.rs`、および src 直下で他担当に含まれないもの 16 ファイル(下記)。読み取りのみ。数字は自分で数えた(`/tmp/.../scratchpad/agg.py` で範囲指定から集計)。

## 0. 先に知らせたいこと

- **`crate::APP` はもう存在しない。** グローバルは `lib.rs:229` の `pub static RUNTIME: SingleThreadCell<Runtime>` で、入口は `with_app`(`&mut Runtime`、再入なら `None`)/`with_app_ref`/`with_app_or_repost`/`with_app_or_repost_with`(`lib.rs:238-275`)。`docs/layer-boundaries.md:56` の B-1 は旧名 `crate::APP` のまま。以下「APP に触る箇所」は `RUNTIME`/`with_app*` の箇所。
- **行数の定義**: 行数はコメント・空行込み。関数の直前の doc/属性コメントはその関数の行に含めた。関数に属さない import・`impl` 見出し・空行は「decl」としてファイルの主分類に足した(各表の注に書く)。`conv_actuation.rs` の冒頭 113 行は ADR 経緯の doc で、分類不能なので `D` とした。
- 分類は関数の宣言範囲ごと。巨大関数(`run_all`、`dispatch_probe_actions` 等)は論理ブロックでも見た。

## 1. 領域全体の集計

| 区分 | 総行数 | 本体(テスト除く) | テスト | P | P* | O | E | F | G | D |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| app/ (3 ファイル) | 2,724 | 2,437 | 287 | 258 | 468 | 60 | 554 | 328 | 769 | 0 |
| output/ (10 ファイル) | 5,476 | 4,044 | 1,432 | 865 | 1,129 | 14 | 335 | 1,432 | 156 | 113 |
| tray.rs + autostart.rs | 1,324 | 1,324 | 0 | 183 | 37 | 51 | 641 | 332 | 80 | 0 |
| src 直下その他 (14 ファイル) | 8,388 | 4,822 | 3,566 | 4,186 | 10 | 361 | 105 | 133 | 27 | 0 |
| **合計** | **17,912** | **12,627** | **5,285** | **5,492** | **1,644** | **486** | **1,635** | **2,225** | **1,032** | **113** |

(「src 直下その他」は bug_report, config_diagnostics, config_key_resolution_tests, focus_resync, gji_charset_autodetect, journal_policy, keymap, lifetime_counter, main, msime_key_assignment, msime_legacy_keymap, scancode_map, scanmap, tuning。)

- 移せる(P+P*): **7,136 行**(本体の 56%)。ただし内訳は偏る: P の 5,492 行のうち **4,186 行は src 直下の既に ungated で Linux テスト済みのファイル**。移しても Linux で回るテストは増えない(構造上の整理にしかならない)。**移すと Linux で初めて回るテストが増えるのは gated 側の P/P***(app/ 258+468、output/ 865+1,129、tray 183+37)だけで、約 2,940 行。
- 残る(O+E+G): **3,153 行**。
- 分割が要る(F): **2,225 行**。うち output/ が 1,432 行(64%)。`vk_send.rs`(623)・`output/mod.rs`(576)・`probe_io.rs`(129)・`conv_actuation.rs`(73)が中心。
- gated ファイルのテスト: app/ 287 行、output/ 1,432 行(合計 1,719 行、約 71 テスト)は `lib.rs:47-101` の `#[cfg(windows)]` のため Linux のバイナリに存在しない。

### ゲート(どの宣言が原因か)

| ファイル群 | gate の原因 |
|---|---|
| app/* | `lib.rs:100-101` `#[cfg(windows)] pub(crate) mod app;` |
| output/* | `lib.rs:74-75` `#[cfg(windows)] pub mod output;` |
| tray.rs | `lib.rs:90-91`。autostart.rs | `lib.rs:47-48` |
| bug_report, config_diagnostics, focus_resync, journal_policy, keymap(`cfg_attr(not(windows), allow(dead_code))`)、lifetime_counter, scanmap, tuning | gate なし(Linux でテストが回る) |
| config_key_resolution_tests | `lib.rs:26-27` `#[cfg(test)]` のみ(Linux で回る) |
| gji_charset_autodetect / msime_key_assignment / msime_legacy_keymap / scancode_map | 上部の純粋部は ungated、レジストリ・ファイル I/O の `mod windows_impl`/`mod registry` だけ `#[cfg(windows)]` |
| main.rs | `#[cfg(windows)]` の関数と `#![windows_subsystem]`(`startup_error_hint` は純粋だが不要に gated) |

## 2. APP(RUNTIME)に触る箇所の全洗い出し

領域内の `with_app*` / `RUNTIME` の行(コメント行除く)は **約 41 行、5 ファイル**。

| ファイル | 箇所 | 種別 |
|---|---:|---|
| `app/mod.rs` | 25 | `report`(150)、`init_ngram_validated`(464)、`check_keyboard_layout_on_change`(486)、`dispatch_engine_message` の 16 アーム(513-618)、`handle_hook_key_event` の 3 回(627, 648, 665)、`run_message_loop` 冒頭(676)、`reload_config` の 2 回(851, 886) |
| `app/bootstrap.rs` | 8 + 2 | `with_app_ref`(680)、`with_app`(770 config 反映、796、872 `win_event_proc`、1311、1319、1324、1340)、`RUNTIME.set`(740)、`RUNTIME.clear`(802) |
| `tray.rs` | 3 | `handle_autostart_toggle` のバルーン 3 分岐(1008, 1020, 1028)。`tray_wnd_proc` は `message_handlers::handle_wm_app_tray`/`handle_wm_command` に委譲(これらの借用は領域外) |
| `output/probe_io.rs` | 2 | `start_ms_ime_ready_poll` の `spawn_local` 内(261, 300) |
| `output/conv_actuation.rs` | 1 | `actuate_conv_mode` の `spawn_local` 内(177) |

**`dispatch_engine_message`(`app/mod.rs:498-623`)の 16 アームの再入時の扱い**:
- `let _ = with_app(...)`(再入なら黙って捨てる): WM_TIMER、WM_EXECUTE_EFFECTS、WM_DUPLICATE_INSTANCE、WM_IME_KIND_CHANGED、WM_POWERBROADCAST、WM_WTSSESSION_CHANGE、WM_INPUTLANGCHANGE、WM_HOTKEY×2、WM_DUMP_JOURNAL、TaskbarCreated = **11 アーム**。
- `with_app_or_repost*`(再入なら自スレッドに再 post): WM_ASYNC_IME_APPLY_COMPLETE、WM_KANA_LOCK_WARNING_CHANGED、WM_HOOK_IME_MODE_DIAGNOSTIC、WM_PANIC_RESET、WM_FOCUS_KIND_UPDATE = **5 アーム**。
- 直接借用しない: WM_KEY_FROM_HOOK(`handle_hook_key_event` へ)、WM_APP、WM_RELOAD_CONFIG、WM_COMMAND、WM_DRAIN_OUTPUT_QUEUE、WM_ENGINE_QUIT_REQUEST(ハンドラ内で借用、未確認)。

**「借用は 1 ターンの入口で 1 回」構想の入口候補**(1 つのイベントを 1 回の借用で処理できるか):

| 入口 | 場所 | 現状の借用回数 / 注意 |
|---|---|---|
| ★ 物理キー 1 打 | `app/mod.rs:625-668` `handle_hook_key_event` | 1 打につき最大 **3 回**(`lang_check_on_keydown` が `event` を書き換え → `kp_trigger_focus_resync` → `handle_wm_key_from_hook`)。借用の外で `OUTPUT_GATE.is_active()`、`INPUT_DEFER.pending_len_nonblocking()`、`FOCUS_RESYNC.is_armed()`、`DUMP_TRIGGER`、`panic_detect` の静的も読み書きする。最有力かつ最も危険 |
| WM_* 16 アーム | `app/mod.rs:498-623` | 各アーム 1 回。再入時の挙動が 11/5 で非対称(§6 危険 2) |
| フォーカス | `app/bootstrap.rs:842-882` `win_event_proc` | 1 回。関数内 static `LAST_FOCUS_HWND` で重複 HWND を落とす判断が借用の外にある |
| 設定再読込 | `app/mod.rs:770-893` `reload_config` | 2 回(`apply_config_update` と `reload_layouts`)、間に FS 走査。さらに `panic_detect::set_panic_trigger_combos`、`hook::*` の静的を借用の外で書く |
| トレイ | `tray.rs:986-1038` | 結果バルーンのたびに最大 1 回、3 分岐に分散 |
| 起動 | `app/bootstrap.rs:1008-1343` `run_all` | 8 + 2 回(起動順序があるので 1 回化は対象外が妥当) |

RUNTIME 以外の「借用の外のグローバル」: `FOCUS_RESYNC`(`focus_resync.rs:127`)、`INPUT_DEFER`、`OUTPUT_GATE`、`HOOK_KEYS`/`WAKE_PENDING`、`DUMP_TRIGGER`(`app/mod.rs:43`)、`RAPID_IME_TIMESTAMPS`、`TASKBAR_CREATED_MSG`、`LAST_BALLOON_WARNINGS`(`Mutex<Vec<String>>`)、`MENU_TARGET_HWND`、`RAW_TSF_LITERAL`(`lib.rs:225`)、`hook::set_*`(bootstrap 709-712)。

## 3. ファイル別

### 3.1 app/mod.rs(総 964 / 本体 894 / テスト 70・3 テスト) — gate: `app` モジュール宣言
分類(行): P 97 / P* 185 / E 70 / F 226 / G 316 / O 0。decl 130 行は G に加算。

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| `StartupDiagnostics::{new,warn,note,warn_config}` | 92-122 | 25 | P | 文字列収集。`tracing` のみ |
| `StartupDiagnostics::report` | 124-156 | 33 | F | 「前回と同じ警告ならバルーン省略」の判断が `static LAST_BALLOON_WARNINGS` と `with_app(show_tray_balloon)`(150)に直結。判断(`unchanged`)を core、バルーン発行を Cmd に |
| `HotKeyGuard::drop` | 163-169 | 7 | E | `UnregisterHotKey` |
| `set_taskbar_created_msg` / WTS FFI | 57-59 / 72-73 | 5 | G / E | `AtomicU32` と `wtsapi32` 宣言 |
| `run` | 180-182 | 3 | G | |
| `load_config` | 197-211 | 15 | P* | Win32 なし。ambient `std::env::args`/`current_exe`/FS。seam: 設定ソースを注入 |
| `cli_arg_config_path`/`find_config_path`/`is_dev_build`/`exe_dir`/`ensure_default_config_exists`/`ensure_default_layouts_exist`/`resolve_relative` | 217-305 | 59 | P* | 同上(OS 非依存、そのまま core へ置ける。ambient env/FS) |
| `read_bug_report_attachments` | 311-351 | 41 | P* | FS 読み。同上 |
| `parse_key_combos` | 354-370 | 17 | P | `crate::vk::parse_key_combo`(ungated)+ diag |
| `init_ime_sync_keys` | 382-422 | 41 | P | BUG-140 の重複除外。純粋 |
| `build_panic_trigger_combos` | 425-450 | 26 | P* | 戻り値型 `panic_detect::PanicTriggerCombo` が gated(lib.rs:77)。seam: 型を ungated へ |
| `init_ngram_validated` | 453-468 | 16 | G | ファイル読み→`with_app(set_ngram_model)` |
| `check_keyboard_layout_on_change` | 471-493 | 23 | F | `ime::keyboard_layout_info`(O)→LANGID 判定→警告文→`with_app(show_tray_balloon)`。**`bootstrap::check_keyboard_layout`(562-582)と判定・文言が重複**。判定+文言を P、取得を O、バルーンを E に |
| `dispatch_engine_message` | 498-623 | 126 | G | WM→`message_handlers::handle_wm_*`。アーム内に素のロジックが 2 つ: WM_KEY_FROM_HOOK(579-593、ring 消費・dropped 検出・`mark_needs_engine_resync`)、WM_ENGINE_QUIT_REQUEST(607-614、モーダルポンプ判定) |
| `handle_hook_key_event` | 625-668 | 44 | F | defer 判断(`OUTPUT_GATE.is_active() \|\| defer_for_resync`、`pending_len_nonblocking`)+ panic/dump 記録 + 3 回の借用。§2・§6 参照。判断を core(「このキーは defer / replay_later / 処理」を返す純関数)、gate 読み取りを入力 snapshot に |
| `run_message_loop` | 670-701 | 32 | G | 冒頭で `sync_ime_kind_from_observation`(676) |
| `launch_settings`/`launch_bug_report`/`launch_settings_with_args` | 706-767 | 60 | E | プロセス spawn。`launch_bug_report` の引数組み立て 20 行は P だが小 |
| `reload_config` | 770-893 | 124 | F | 設定→実行時パラメータ(`SpecialKeyCombos` 等)の導出は純粋(約 60 行)、その後に `with_app(apply_config_update)`、MS-IME 時の `tsf_obs()` 参照(859-867)、`reload_layouts`。**`run_all`(bootstrap 1125-1183)と導出が重複**。純関数 `derive_runtime_params(config, thumb_vks) -> (Params, warnings)` に切り出せる |

- 移せる(P+P*): 282 / 残る(O+E+G): 386 / 分割(F): 226。
- Windows 依存の種類(件数、コメント行除く): `with_app*` 26、`unsafe` 21、`crate::hook::` 4(`current_tick_ms`、`resolve_thumb_key`、`thumb_vk_codes`)、`tsf::observer::` 2(`tsf_obs()` 含む)、`OUTPUT_GATE` 1、HWND/WPARAM/LPARAM(`dispatch_engine_message` の引数)、`win32::` 2、WTS FFI、`SingleThreadCell` static 1、`Mutex` static 1。

### 3.2 app/bootstrap.rs(総 1,450 / 本体 1,344 / テスト 106・2 テスト) — gate: `app`
分類(行): P 161 / P* 283 / O 60 / E 285 / F 102 / G 453。decl 91 行は G に加算。

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| `show_no_layouts_dialog` | 39-49 | 11 | E | `MessageBoxW`(`show_error_dialog`)+ 設定起動 |
| `warn_layout_fallback` | 68-92 | 25 | F | 「フォールバック発生か」(純粋 8 行)+ モーダル。判定を P、ダイアログを E |
| `thumb_shift_faces_enabled_for` | 99-102 | 4 | P | `VkCodeExt::classify_modifier` |
| `log_path`/`init_logging` | 115-120 / 122-168 | 6 / 47 | O / E | `AttachConsole`、tracing 初期化 |
| `handle_auto_start` | 187-207 | 21 | F | `autostart::migrate_from_schtasks`(E)と `is_registered`(O)+ 警告判断。小 |
| `init_engine_validated` | 214-298 | 85 | G | `hook::resolve_thumb_key`、レイアウト走査、`show_error_dialog`、`NicolaFsm::new`。配線。判断は `select_default_layout` と `warn_layout_fallback` に出ている |
| `select_default_layout` | 301-308 | 8 | P* | `runtime::LayoutEntry` が gated。seam: 型を ungated へ |
| `enumerate_process_exe_names` | 322-357 | 36 | O | Toolhelp32 → `Vec<String>` |
| `scan_running_processes` | 359-372 | 14 | P | 名前リスト照合 |
| `detect_conflicting_software` | 375-391 | 17 | P* | 定数リスト + O 呼び出し。seam: プロセス名リストを注入 |
| `RELAY_OR_REMAP_CANDIDATES` | 431-517 | 87 | P | データ(+前の doc 38 行) |
| `detect_relay_or_remap_software` / `list_all_running_process_names` / `check_conflicting_software` | 519-559 | 3 / 6 / 9 | P* | O への依存のみ |
| `is_relay_or_remap_software_process` | 531-535 | 5 | P | hook watchdog(issue #165)の 3 秒周期から呼ばれる |
| `check_keyboard_layout` | 562-582 | 21 | F | `app/mod.rs:471` と同じ判定の重複 |
| `init_tray` / `install_hooks_and_hotkeys_validated` | 585-595 / 598-619 | 11 / 22 | E | |
| `HotKeyGuard::register_*` / `WtsGuard::drop` / `register_session_notification` | 627-687 | 16+15+7+9 | E | `RegisterHotKey` ×2、`WTSRegisterSessionNotification` |
| `initialize_app` | 691-792 | 102 | G | `PlatformState::new`、`hook::set_*` を 4 つ(709-712)、`RUNTIME.set(Runtime::new(...))`、`with_app` で `set_*` を 10 回(771-788)。**`RUNTIME` の唯一の構築点** |
| `initialize_ime_cache`/`cleanup` | 795-797 / 800-804 | 3 / 5 | G | `RUNTIME.clear()` は 802 |
| `WinEventHookGuard::drop`/`install_focus_hook` | 812-839 | 7+18 | E | `SetWinEventHook` |
| `win_event_proc` | 842-882 | 41 | G | フォーカス WinEvent。→ `on_window_focus_event(HwndId, Instant::now())`(880)。HWND 重複排除は関数内 `static LAST_FOCUS_HWND`(857)。時計を直接読む(`Instant::now`) |
| `install_ctrl_handler` | 885-901 | 17 | E | `SetConsoleCtrlHandler` |
| `LayoutEntry::scan_all` | 907-997 | 91 | P* | `read_dir` + yab パース + lint + `StartupDiagnostics`。OS 非依存。seam: FS 注入。テスト 2 本はここ周辺 |
| `run_all` | 1008-1343 | 336 | G | 論理ブロック: 1008-1030 args・panic hook(E 23)、1031-1049 `--exit-after` スレッド(E 19)、1050-1103 mutex 多重起動(E 54)、1104-1124 診断・昇格・設定読込(G 21)、1125-1173 キー解析(P* 49)、**1174-1261 Engine 構成**(P* 88、thumb VK から `set_space_thumb_config` 等を設定)、1262-1343 keymaps・`initialize_app`・フック・ワーカー・メッセージループ・`cleanup`(G 82) |

- 移せる(P+P*): 444 / 残る(O+E+G): 798 / 分割(F): 102。
- Windows 依存: `unsafe` 14、`with_app*` 9(+`RUNTIME.set/clear`)、`crate::win32::` 6、`crate::hook::` 多数(`resolve_thumb_key`、`set_thumb_vk_codes`、`set_keyboard_model`、`set_alt_impersonation_enabled`、`set_swallow_alt_kana_mode_switch`、`install_hook`、`tick_hook_alive`、`current_tick_ms`、`now_timestamp_us`)、`tsf::observer::`(`install_observation_hooks`、`start_monitor_thread`)、Toolhelp32、Named Mutex(`CreateMutexW`、`FindWindowW`、`PostMessageW`)、`RegisterHotKey`、WTS、`SetWinEventHook`、`SetConsoleCtrlHandler`、`AttachConsole`、`panic::set_hook`、`Instant::now`。

### 3.3 app/logging.rs(総 310 / 本体 199 / テスト 111・3 テスト) — gate: `app`
全 199 行 E。`std::fs` と tracing の書き込み先(20MB で 1 世代ローテート、`OnceLock<Arc<Mutex<..>>>` static)。**Win32 なし**で OS 非依存の I/O。残す対象は I/O だが、移すなら infra 側。テスト 111 行は std のみなので移せば Linux で回る(`unique_temp_path` を使う FS テスト)。

### 3.4 output/mod.rs(総 1,749 / 本体 1,375 / テスト 374・27 テスト) — gate: `output`
分類(行): P 519 / P* 250 / F 576 / G 30。decl 152 行は P に加算。

`Output`(61-193、doc 込み 133 行)は **state と injector の混在構造体**: 状態 = `composition`、`warmup_coord`、`tsf_gate`、`conv_mode`、`ime_mode_fsm`、`ime_mode_focus_gen`、`ms_ime_gate_give_up`、`confirm_gate_deadline_override_ms`、`shift_conv_guard_gen`、`observe_unicode_literal`、`conv_mutation_allowed`、`runtime_outbox`、`pending_drain_before_send_flush`。injector = `KeyInjector`。結果として全体を F の宣言と数えた。

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| `Output`(構造体) | 61-193 | 133 | F | 上記。**状態部は core の `OutputState` へ、`KeyInjector` だけ Windows 側に** |
| `TimerCommand`/`fmt_ms`/`RawRecoveryOutcome`/`WarmthContext`/`WarmupOutcome` | 36-56, 201-245 | 約 45 | P | |
| `StepProbeResult` | 222-237 | 16 | P* | 型 `tsf::gji_fsm::GjiAction` は ungated だが `tsf::literal_facts` との組。実質 P* |
| `Output::new` | 260-278 | 19 | G | `KeyInjector::new`(KanaTable/symbol 表構築)含む |
| 状態アクセサ・委譲(`gji_*`、`update_ime_mode_*`、`mark_composition_cold`、`on_focus_changed`、`on_focus_change_tsf`、`confirm_tsf` 等 約 40 関数、3-9 行) | 284-604 | 約 150 | P | `warmup_coord`/`composition`/`tsf_gate` への薄い委譲 |
| `begin/end_probe_guard`、`mark_send`、`mark_vk_output`、`take_pending_requests` | 456-465, 665-680, 284 | 約 15 | P* | `OutputActiveGuard`/`OUTPUT_GATE` 静的、`hook::current_tick_ms`。seam: ゲートを Cmd/Event に、時計を `TickMs` で渡す |
| `on_ime_mode_focus_changed` | 361-380 | 20 | P* | 時計を直接読む。**3 つの世代/ラッチ(`ime_mode_focus_gen`、`ms_ime_gate_give_up`、`confirm_gate_deadline_override_ms`+`shift_conv_guard_gen`)を 1 関数で更新**(BUG-49 追補2)。分離しない |
| `bump/extend/clear_confirm_gate_*` | 390-421 | 17 | P | 世代照合つきの純粋ロジック(BUG-49 pass-5) |
| `send_gji_half_width_alnum_toggle` | 619-662 | 44 | F | `hook::ime_mode_key_injection_blocked_by_modifier`(O)+ `ime_open` 引数での抑止判断 + `tsf::observer` 読み(ログのみ)+ `unsafe send_ime_mode_key_with_shift_release_prefix`(E)。判断(action→vk、skip 条件 3)を P、送信を E。戻り値 `false`=未送信のとき belief を進めない(INV-D) |
| `send_keys` | 691-811 | 121 | F | `KeyAction` を `OutputSession` 経由でモード別に送る。内部に判断が 2 つ: Unicode モードの「GJI 書き込み観測を張るか」(741-763: `observe_unicode_literal` swap、`injection_mode`、`ime_mode_fsm` 状態、`gji_write_bytes()` → `install_pending_tsf`)と、未平坦化 `Sequence` の防御展開(773-798)。**計画 `plan_send(actions, mode, state) -> Vec<SendCmd>` に** |
| `assess_warmth` | 815-826 | 12 | P | warm/cold/expired を返す。純粋(`tuning::COMPOSITION_TIMEOUT_MS`) |
| `defer_if_probe_in_flight` ほか defer 系 6 関数 | 838-963 | 70 | P* | 「probe/recovery 実行中か」判断 + キュー上限(2048)。ただし `raw_recovery_owns_deferred` が静的 `RAW_TSF_LITERAL`(`lib.rs:225`、Mutex+atomic)を読む。seam: それを `Output`(`OutputState`)のフィールドに |
| `step_probe` | 970-1045 | 76 | F | `TsfEnvSnapshot` の組み立て(`gji_is_active_ime`、`gji_candidate_visible_now`、`literal_session_confirmed_gen_snapshot`: O)+ `machine.tick(env)`(P)+ `dispatch_probe_actions`(E 経由)+ 段末分岐。**env を呼び出し側から受ける**形にすれば `tick`+dispatch は P* |
| `raw_recovery_owns_deferred` | 1057-1065 | 9 | P* | 上記静的 |
| `finish_probe_stage` | 1071-1102 | 32 | F | (a) deferred 解放(INV-F: raw recovery 側に所有があれば触れない)→(c) ゲート解放 →(b) GjiEvent。**順序が仕様**。実送信(`flush_pending_deferred_vks`)を含む |
| `install_pending_tsf`、`pending_tsf_timer`、`cancel_probe` ほか | 1108-1166 | 約 35 | P | |
| `CompositionOutput` impl | 1169-1199 | 31 | G/P | `send_romaji` は `injection_mode` で E 関数へ分岐(10 行 G) |
| `record_raw_tsf_literal` | 1215-1230 | 16 | P* | `RAW_TSF_LITERAL` 書き込み |
| `flush_raw_tsf_literal_romaji`/`send_romaji_dispatching_on_gate`/`flush_raw_tsf_literal_recovery`/`flush_stale_deferred_vks_after_recovery`/`flush_pending_deferred_vks` | 1237-1371 | 14/11/6/9/34 | F | 「取り出し→gate 状態でマーカー選択(`gate_is_bypass`)→`send_deferred_vks`」。**回復の順序(backspace → romaji 再送 → deferred flush)が BUG-36/38 の根本**。計画(順序つき `Vec<Cmd>`)にして core、送信を E |

- 移せる(P+P*): 769 / 残る(O+E+G): 30 / 分割(F): 576。
- Windows 依存: `hook::current_tick_ms` 3 + `crate::hook::` 計 4、`tsf::observer::` 7、`RAW_TSF_LITERAL` 7、`OUTPUT_GATE/OutputActiveGuard` 1、`unsafe` 2、`ime::` 1。**HWND は 0**(output/ は HWND を扱わない)。

### 3.5 output/vk_send.rs(総 866 / 本体 782 / テスト 84・3 テスト) — gate: `output`
分類(行): P 24 / P* 13 / E 89 / F 623 / G 33。decl 49 行は E に加算。

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| `DeferGate` / `deferred_origin` | 22-38 | 17 | P | |
| `defer_respecting_gate` | 47-53 | 7 | P* | |
| `drain_pending_deferred_before_send_if_queue_only` | 83-100 | 18 | F | 「queue-only か」判断 + `flush_pending_deferred_vks`(E)。**gate は `Enforced` のみ**(`Exempt` で呼ぶと順序反転、ADR-128) |
| `TsfSendPipeline::transmit` | 114-162 | 49 | F | eager/unicode か VK run かの選択 + `send_input_safe` |
| `send_romaji_batched_gated` | 201-345 | 145 | F | **最大の F**。GjiEvent KeyInput(P)→ログ用 `gji_last_write_ms`(O)→drain→`assess_warmth`→defer gate→cold なら `increment_cold_start_count`、`get_foreground_window_class`(unsafe、同期 O、ログ用)、`OutputActiveGuard::begin`(**OUTPUT_GATE 静的**)、`ChromeProbe` 生成・`install_pending_tsf`、`runtime_outbox.push(StartTsfProbe)`、`spawn_local`(診断の IMC 読み)→warm なら `ms_ime_gate_defer`→即送信。計画 enum `{Defer, ColdProbe{..}, MsImeWait, Immediate(vk_runs)}` に |
| `send_romaji_as_tsf_gated` | 378-476 | 99 | F | 同型(TSF 側)。`ColdWarmupSequence::run_start`、`GjiWarmupCoro::new`。2 関数の骨格重複が大きい |
| `ms_ime_gate_defer` | 516-554 | 39 | F | 判断(`needs_f2_probe`、defer、give-up latch、`is_native_ready`)+ `start_ms_ime_ready_poll`(spawn)+ `MsImeReadyCoro` 設置 |
| `send_romaji_as_tsf_warm` | 556-641 | 86 | F | `in_post_unicode_pending`(`gji_last_io_ms` 読み)、診断 `spawn_local`、`transmit`、`LiteralDetectFsm` 設置判断(`gji_is_active_ime`、`gji_last_io_ms`、`tsf_gate` 状態) |
| `send_char_as_tsf` / `send_char_as_vk` | 648-702 / 714-762 | 55 / 49 | F | `resolve_char`(P)→romaji 経路へ合流/VK 直送/Unicode。到達不能とコメントされた分岐を含む |
| `send_romaji_batched*`/`send_romaji_as_tsf*`(薄い入口) | 178-199, 368-376 | 12 | G | |
| `send_unicode_char`/`send_romaji_batch_immediate`/`send_romaji_as_unicode`/`send_vk_runs`/`send_deferred_probe_vks_from`/`send_vk_pair`/`push_unicode_char_inputs` | 各 3 行 | 21 | E | `KeyInjector` への委譲 |

- 移せる(P+P*): 37 / 残る(O+E+G): 122 / 分割(F): 623。
- Windows 依存: `hook::current_tick_ms` 8、`tsf::observer::` 6、`imm::`(cmode_has 等、診断ログ)5、`ime::` 3、`spawn_local` 2、`OUTPUT_GATE/OutputActiveGuard` 2、`RAW_TSF_LITERAL`(tuning 定数名のみ)、`unsafe` 2、`send_input_safe` 1、`INPUT` 型 1。

### 3.6 output/probe_io.rs(総 1,370 / 本体 683 / テスト 687・20 テスト) — gate: `output`
分類(行): P 135 / P* 352 / E 67 / F 129。decl 30 行は P に加算。テストは `FakeProbeIo` を使う。

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| `trait ProbeIo` | 23-57 | 35 | P | **既に dumb executor の port(10 メソッド)**。分割の見本 |
| `impl ProbeIo for Output` | 59-125 | 67 | E | `note_stage_injection` の状態更新を伴う(小さな F) |
| `refresh_ime_mode_if_focus_matches`、`ms_ime_ready_poll_check_deadline` | 159-199 | 30 | P* | 世代照合/期限。時計 `hook::current_tick_ms`。seam: `TickMs` を引数に |
| `start_ms_ime_ready_poll` | 241-329 | 89 | F | **await をまたぐ**: `gen`(`ime_mode_focus_gen`)を spawn 前に捕獲 → `get_ime_conversion_mode_fenced_async`(O)→ fence を 3 点で照合 →`with_app` 内で gen 照合・`is_native_ready`・期限判定 →`Ready/Pending/Expired/Stale`。期限切れで `ms_ime_gate_give_up` を立てる。**Event(`ImcRead{gen, fence, conv}`)を受ける reducer と、読み取りを依頼する Cmd に分ける**と対応が素直 |
| `plan_skipped_record`、`StageEnd`/`DispatchResult`、`MsImePollStatus`、`fmt_conv` | 128-143, 332-382 | 約 65 | P | |
| `dispatch_probe_actions` | 384-682 | 299 | P* | `ProbeIo` を介する本体はほぼ純粋(`TransmitTarget::{Tsf,Chrome}` 分岐、literal 判定記録、give-up)。**Win 依存は 3 種のみ**: `tsf::observer::{gji_idle_ms, gji_write_bytes, mark_literal_session_confirmed}`(3)、時計(1)、診断 `spawn_local`(IMC 読み、ログのみ、441-456)。seam: observer 値を入力 snapshot で、診断は executor 側へ |

- 移せる(P+P*): 487 / 残る(O+E+G): 67 / 分割(F): 129。
- Windows 依存: `with_app` 2(`spawn_local` 内、B-1 の許容)、`spawn_local` 2、`hook::current_tick_ms` 3、`tsf::observer::` 3、`imm::` 2、`ime::` 2(`get_ime_conversion_mode_*_async`)、`probe_actuation_fence::current()`(静的 fence)。
- テスト 687 行(20 本、`FakeProbeIo`)は現在 Linux では **存在すらしない**。`dispatch_probe_actions` を seam 2 つ解消して移せば、一番大きなテスト獲得になる。ただし `tsf/warmup/*`・`tsf::probe`・`tsf::literal_facts` の移動が先(これらは gated、領域外)。

### 3.7 output/tsf_warmup_coord.rs(総 711 / 本体 424 / テスト 287・13 テスト) — gate: `output`
全 424 行 P*。**Win API トークンなし**。保持物: `RefCell<Box<dyn ImeWarmupStrategy>>`、`RefCell<Option<Box<dyn TickableFsm>>>`、`gji_probe_guard: RefCell<Option<OutputActiveGuard>>`、stage、composition-reset 橋渡し、key-response バッファ、`pending_deferred`(上限 `DEFERRED_QUEUE_CAP = 2048`、順序トークン)。
seam: (1) `OutputActiveGuard`(`tsf::probe_bridge` の `OUTPUT_GATE` 静的。RAII で probe 寿命にぶら下がる)、(2) `TimerCommand` の `crate::TIMER_TSF_PROBE`(usize 定数)、(3) 型 `tsf::observer::ActiveImeKind`・`warmup_strategy`・`TickableFsm`・`gji_fsm`・`probe_fsm` が gated(gji_fsm/literal_facts のみ ungated)。
テスト 13 本は `Output` なしで書かれていて(grep で `Output::new`/`with_app`/windows:: 参照 0)、seam を解けば最も安く Linux へ移せる。
- 移せる 424 / 残る 0 / F 0。

### 3.8 output/key_injector.rs(総 290、テストなし) — gate: `output`
分類: P 118 / P* 45 / E 127。decl 72 行は P に加算。
- P: `resolve_char`(79-87)、`split_vk_runs`(222-236、同一 VK 連続でラン分割)、`format_vk_run`(249-259)、`new`(66-71)。
- P*: `VkMarker::make_input`/`make_key_input`/`push_unicode_char_inputs`(37-52, 156-185)。`INPUT` 構造体を作る純粋処理。seam: 中立な `KeyEvent{vk,scan,flags,marker,unicode}` にして `INPUT` への変換を E に。
- E: `send_key`/`send_ctrl_chord`/`send_unicode_char`/`send_romaji_per_key`/`send_romaji_as_unicode`(8、表引き→フォールバックの小判断あり)/`send_vk_pair`/`send_vk_run_batch`/`send_romaji_batch_immediate`/`send_vk_runs`/`send_deferred_probe_vks_from`。`send_input_safe` 5。
- Windows 依存: `win32::send_input_safe` 5、`INPUT/KEYBDINPUT/VIRTUAL_KEY` 型、`tsf::output::make_*_input`、`tsf::observer::gji_idle_ms`(ログ)1。

### 3.9 output/held_modifiers.rs(148、テストなし) — gate: `output`
P 36 / P* 15 / O 14 / E 52 / F 31。`HeldModifiers::read`(37-46、O: `hook::is_physical_key_down` ×6 を `PHYSICAL_KEY_STATE` 静的から)、`push_release`(52-62、P*: INPUT 構築)、`push_restore`(68-94、**F**: 物理状態を読み直して「まだ押下中の修飾だけ復元」を判断し INPUT を作る)、`send_keymap_target`(113-148、E、`INJECTED_MARKER`)。`hook::` 12 行(静的読み)、`unsafe` 4。判断(release/restore の組)を core、物理状態は入力 snapshot で。

### 3.10 output/conv_actuation.rs(186、テストなし) — gate: `output`
冒頭 113 行は ADR-084/086/089 の経緯 doc(D)。`actuate_conv_mode`(141-185、45 行、F): ① `conv_mutation_allowed` 却下(P)、② `ime_mode_fsm.unconfirm`(INV-2、同期、P)、③ give-up latch 解除(P)、④ `spawn_local` で `ActuationTarget::capture(focus_gen).await`(O)→ `set_ime_conv_for_target`(E)。`with_app` を完了時の世代照合クロージャで 1 回使い、`None`(再入)なら `focus_gen.wrapping_add(1)` で書き込み中止。①-③ を core の reducer、④ を Cmd に。

### 3.11 output/sender.rs(93) / resolve.rs(33) / types.rs(30)
- `sender.rs`: `InjectionSender`(trait)+ Unicode/Vk/Tsf の 3 sender + `OutputSession`(`OutputActiveGuard::begin`)。全 93 行 G(モード→`Output` のメソッドへのディスパッチ)。
- `resolve.rs`: `special_key_to_vk`(const 表)と `CharResolution`。全 33 行 P。
- `types.rs`: `From<(InjectionHint, AppKind)> for InjectionMode`。全 30 行 P*。seam: `InjectionHint`(`focus::classifier`、gated)を `state::injection_mode` 側へ(`InjectionMode` 自体は既に ungated へ移設済み)。

### 3.12 tray.rs(総 1,114、テストなし) — gate: `lib.rs:90`
分類: P 183 / P* 37 / O 19 / E 463 / F 332 / G 80。decl 66 行は P に加算。

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| `MENU_TARGET_HWND`/`menu_target_hwnd` | 26-44 | 19 | G | `AtomicIsize` 静的(右クリック直前のフォーカス窓) |
| `IDM_*` 定数・`TrayCommand` | 46-91 | 46 | P | |
| `append_menu_*` | 93-122 | 30 | E | |
| `SystemTray`(構造体+impl) | 140-379 | 約 215 | E | `new`(76: `RegisterClassW`/`CreateWindowExW`/`Shell_NotifyIconW`)ほか。`set_layout_names`/`current_layout_name`/`kana_lock_warned` の 3 行は P |
| `create_keyboard_icon` | 418-526 | 109 | E | GDI(`CreateDIBSection` 等)。ピクセル配置の計算(約 40 行)は P だが GDI と一体 |
| `set_tooltip` | 529-555 | 27 | F | ツールチップ文言の決定(P、約 13 行)+ `szTip` への書き込み(E) |
| `handle_tray_message` | 561-722 | 162 | F | **メニュー内容の判断**(kana_lock_warned・elevated・更新表示 `update_state::display` で項目の有無を決める)+ 読み取り 4 種(`get_gui_thread_info_with_timeout(150ms)`(同期 O、BUG-34 型のブロック、上限 150ms)、`ime::is_caps_lock_on`、`autostart::is_registered`(レジストリ)、`update_state::load`(FS))+ `TrackPopupMenu`(モーダル、`ModalPumpGuard`)。引数 7 個は `handle_wm_app_tray` が `Runtime` から集めて渡す。「メニュー項目リスト(`Vec<MenuItem>`)を返す P」と「表示する E」に |
| `handle_tray_command` | 726-747 | 22 | P* | `WPARAM` → `TrayCommand`。seam: `usize` を引数に |
| `show_kana_lock_help_dialog` / `show_about_dialog` | 750-776 / 862-925 | 27 / 64 | E / F | about は `update_state::display` → 文言・URL の決定(P、約 35 行)+ `MessageBoxW` |
| `is_elevated` | 783-791 | 9 | O | `IsUserAnAdmin` |
| `restart_as_admin`/`restart_self`/`open_url`/`open_homepage` | 797-955 | 31/20/23/3 | E | `ShellExecuteW`、`exit(0)` |
| `open_update_page` | 957-967 | 11 | F | state → URL の決定(P)+ `open_url`(E) |
| `approx_duration` | 969-980 | 12 | P | |
| `handle_autostart_toggle` | 986-1038 | 53 | F | 登録状態→トグル(O/E: `autostart::*`)→設定保存(`save_auto_start_config`)→結果 3 分岐のバルーン(`with_app` 3 回)。判断(成功/警告あり/保存失敗の文言選択)を P に |
| `save_auto_start_config` | 1046-1052 | 7 | P* | `find_config_path`(FS)+ `AppConfig::save_auto_start`(core) |
| `tray_wnd_proc` | 1076-1114 | 39 | G | **WM_APP/WM_COMMAND の実際の到達点**(`SendMessage` 同期配送。2026-07-27 の実機事故の経緯が doc に残る)。`message_handlers::handle_wm_app_tray`/`handle_wm_command` に委譲 |

- 移せる(P+P*): 220 / 残る(O+E+G): 562 / 分割(F): 332。
- Windows 依存: `unsafe` 26、`to_wide` 11、`win32::` 13、`with_app` 3、HWND/HMENU/HICON/WPARAM/LPARAM/NOTIFYICONDATAW、GDI、Shell、`ModalPumpGuard`(`runtime::engine_window`)、`ime::is_caps_lock_on` 1。

### 3.13 autostart.rs(210、テストなし) — gate: `lib.rs:47`
O 32 / E 178。レジストリ(`HKCU\...\Run`、`Software\awase\SchtasksMigrated`)の読み書きと `schtasks.exe` の spawn のみ。`is_registered`(112-126)と移行マーカー読み(177-191)が O、`register_path`/`unregister`/`migrate_from_schtasks`/マーカー書き込みが E。判断ロジックなし。全体が Windows 側に残る。

### 3.14 src 直下の ungated/ほぼ ungated ファイル(Linux でテスト済み)

| ファイル | 総/本体/テスト(テスト数) | 分類(行) | メモ |
|---|---|---|---|
| `bug_report.rs` | 2,794 / 1,496 / 1,298 (49) | P 1,496 | 不具合報告の payload 組み立て・gzip+base64・予算内切り詰め・RFC3339 変換。`crate::state::*` のみ依存。Win トークンは文字列中の 2 つだけ |
| `config_diagnostics.rs` | 279 / 176 / 103 (6) | P 176 | ADR-201 の「以前は無視されていた設定が有効になった」判定 |
| `config_key_resolution_tests.rs` | 609 / 0 / 609 (5) | T | `lib.rs:26` の `cfg(test)` のみ。Linux で回る |
| `focus_resync.rs` | 211 / 128 / 83 (7) | P 126 / P* 2 | 4 つの atomic による armed/gate/generation。`open_if_current` は世代照合+`compare_exchange`。**型は純粋、`pub static FOCUS_RESYNC`(127)だけが seam**(`app/mod.rs` の 3 箇所と runtime が読む) |
| `gji_charset_autodetect.rs` | 594 / 397 / 197 (10、うち 2 本は gated 内) | P 298 / P* 8 / O 47 / F 44 | 純粋: `classify_*`(BUG-115 の優先順位判定、1-248)、`lookup_from_config1_db_read`(367-394)。O: `config1_db_path`/`read_config1_db`/`config1_db_stamp`/`read_key_effect_keymap`。P*: `is_configured_thumb_key`(`hook::thumb_vk_codes` の静的)。F: `bundled_preset_for_adjudication`(FS 読み+プリセット分類、`TipIdentity` が gated) |
| `journal_policy.rs` | 618 / 268 / 350 (19) | P 268 | レーン分類・容量・`order_violation`(output が呼ぶ)・`coalesce_key_input` |
| `keymap.rs` | 705 / 345 / 360 (24) | P 345 | `[[keymap]]` のコンパイルとマッチ。`warn_on_engine_hotkey_collision` は tracing のみ |
| `lifetime_counter.rs` | 34 / 34 / 0 | P 34 | `AtomicU64` |
| `main.rs` | 67 | P 33 / E 7 / G 27 | `startup_error_hint`(33、文字列分類)は純粋だが `cfg(windows)`。`main`/`show_startup_error` は G/E |
| `msime_key_assignment.rs` | 564 / 408 / 156 (14) | P 183 / O 92 / E 67 / F 66 | 純粋(1-160): `MsImeToggleAssignment::to_combos`、`MsImeKeyAssignment::conflict_warning`。O: レジストリ `read_dword` 以下。E: `spawn_yes_dialog`(別スレッド `MessageBoxW`)、`open_ime_settings`(`ShellExecuteW`)。F: `check_and_warn`(`&mut Runtime`、レジストリ読み→判断→重複警告ラッチ→ダイアログ、179-203)、`read_key_effect_keymap_native_with_reassignment_bits`(293-316、O 読み+bits 計算 P) |
| `msime_legacy_keymap.rs` | 599 / 461 / 138 (11) | P 275 / O 163 / F 23 | 純粋: Shift-JIS テキストのパース(先頭 100 行は doc)。O: `read_raw_value` 以下(`RegGetValueW`)。F: `read_legacy_toggle_assignment`(O 読み+`from_table`) |
| `scancode_map.rs` | 544 / 315 / 229 (19) | P 225 / O 59 / E 31 | 純粋: Scancode Map の parse/build/merge。`mod registry`(`HKLM`)の read=O、write/delete=E。`awase-settings` が使う `pub` |
| `scanmap.rs` | 299 / 256 / 43 (3) | P 256 | scancode⇔`PhysicalPos`(JIS/US 表) |
| `tuning.rs` | 471 / 471 / 0 | P 471 | 定数(`#[measured(...)]` 36 件)。「HWND」は定数名/コメントのみ |

## 4. seam 一覧(領域共通)

件数は領域内の関数数/ファイル数。

| seam の種類 | 現れるファイル・関数 | 直す方針 |
|---|---|---:|
| **グローバル `RUNTIME`/`with_app*`** | 5 ファイル・約 41 行(app/mod 25、bootstrap 10、tray 3、probe_io 2、conv_actuation 1) | 入口を Event に、Cmd 実行側が結果を Event で返す。§2 |
| **`RUNTIME` 以外の借用外グローバル(`RAW_TSF_LITERAL`)** | `output/mod.rs` の 4 関数(`raw_recovery_owns_deferred`、`record_raw_tsf_literal`、`flush_raw_tsf_literal_romaji`、+ vk_send の定数名) | `Output`/`OutputState` のフィールドへ。**コメント上は「Ctrl+C ハンドラ別スレッドから参照」(`lib.rs:222-224`)だが、`install_ctrl_handler`(bootstrap 886-894)は `request_quit`/`post_to_main_thread` しか呼ばず、領域内の読み書きは全て main スレッドに見える(未確認: 他領域の読み手)** |
| **`OUTPUT_GATE`/`OutputActiveGuard`(RAII がタスク寿命にぶら下がる)** | `tsf_warmup_coord.rs`(3)、`sender.rs`(4)、`vk_send.rs`(2: `ChromeProbe` に move)、`output/mod.rs`(`mark_vk_output`)、`app/mod.rs`(`OUTPUT_GATE.is_active()`) | ゲート取得/解放を Cmd(`GateAcquire`/`GateRelease`)にし、probe の寿命と対にする。対を落とすと入力が defer されたまま固まる(ADR-156 系) |
| **時計(`hook::current_tick_ms`)** | 5 ファイル 16 行(vk_send 8、output/mod 3、probe_io 3、bootstrap 1、app/mod 1)+ `Instant::now`(bootstrap 880) | `TickMs` を引数/Event に載せる |
| **`tsf::observer::` の named API(gji_*)** | 7 ファイル 21 行(output/mod 7、vk_send 6、probe_io 3、bootstrap 2、app/mod 2、tsf_warmup_coord 2、key_injector 1) | 打鍵ごとの観測 snapshot を入力に |
| **`hook::*` の静的(物理キー状態・親指 VK・修飾)** | `held_modifiers`(12)、app/mod(4)、bootstrap(10 程度)、gji_charset(1)、output/mod(4) | 入力 snapshot(物理修飾 3 bit、thumb VK 2 つ)を渡す |
| **`spawn_local` + await をまたぐ staleness** | `probe_io::start_ms_ime_ready_poll`、`conv_actuation::actuate_conv_mode`(各 1)、`vk_send` の診断(2、ログのみ) | 世代 `gen`/fence を Event に載せる reducer(§6 危険 4) |
| **gated な型を P* の引数・戻り値に取る** | `types.rs`(`InjectionHint`)、`build_panic_trigger_combos`(`PanicTriggerCombo`)、`select_default_layout`/`scan_all`(`LayoutEntry`)、`StepProbeResult`、`tsf_warmup_coord`/`probe_io`/`output/mod`(`tsf::*` 型多数) | 型を ungated へ(`tsf/` 側の移設が先決、領域外) |
| **ambient env/FS(Win32 なし)** | app/mod 9 関数(115 行)、bootstrap `scan_all`(91)、tray `save_auto_start_config`(7)、gji `config1_db_*`(約 30)、logging(199) | FS 抽象を注入、または OS 非依存のまま core/infra に置く |
| **レジストリ I/O** | autostart、msime_key_assignment、msime_legacy_keymap、scancode_map の 4 ファイル(O/E 計約 290 行) | Windows 側に残す。読んだ値は snapshot で渡す |
| **`INPUT`(SendInput 構造体)の構築** | key_injector(P* 45)、held_modifiers(15) | 中立な `KeyEvent` へ。`INPUT` 変換は E |
| **HWND/HMENU/HICON/WPARAM** | tray(全体)、app/mod(`dispatch_engine_message` の引数)、bootstrap(`WtsGuard`、`win_event_proc`)。**output/ には HWND がない** | `HwndId`/`usize` newtype |
| **FFI(shell32/wtsapi32)** | tray(`IsUserAnAdmin`)、app/mod(WTS) | Windows 側に残す |

## 5. 新 crate への移動候補の順序案

依存が少ない順。

1. **段 0(構造整理のみ、新規 Linux テスト増なし)**: `bug_report`、`config_diagnostics`、`journal_policy`、`keymap`、`scanmap`、`tuning`、`lifetime_counter`、`focus_resync`(型部)、`scancode_map` と `msime_*`/`gji_charset_autodetect` の純粋部。合計約 4,200 行 + 既存テスト 3,000 行超は**すでに Linux で回っている**。
2. **段 1(seam ほぼ不要、Linux でテストが増え始める)**: `output/resolve.rs`、`output/types.rs`(`InjectionHint` を ungated へ)、`app/mod.rs` の `StartupDiagnostics`/`parse_key_combos`/`init_ime_sync_keys`/`build_panic_trigger_combos`(型を ungated へ)、ambient 設定読み 9 関数、`bootstrap` の `thumb_shift_faces_enabled_for`/`scan_running_processes`/`RELAY_OR_REMAP_CANDIDATES`/`is_relay_or_remap_software_process`/`LayoutEntry::scan_all`(+既存テスト 2)、`tray` の `TrayCommand`/`handle_tray_command`/`approx_duration`/メニューと文言の決定(F から切り出し)。app/ のテスト 70+106 行が Linux で回り始める。
3. **段 2(`tsf/` 側の移設が前提、領域外)**: `output/tsf_warmup_coord.rs`(P* 424、テスト 13 本)→ `output/probe_io.rs` の `ProbeIo` と `dispatch_probe_actions`(seam: observer 3 値・時計・診断)。ここで約 970 行のテスト(287 + 687)が初めて Linux で回る。前提: `tsf::probe`/`ime_mode_fsm`/`tsf_gate`/`warmup/*`/`probe_bridge` の ungated 化(inv-c か d1 の担当と思われるが未確認)。
4. **段 3(Output の分割)**: `Output` 構造体の状態部を `OutputState` へ(`RAW_TSF_LITERAL` と `OutputActiveGuard` を取り込む)。`vk_send.rs` の F(623 行)を計画関数に分け、`send_keys`、`step_probe`、`finish_probe_stage`、`flush_*` を「順序つき `Vec<Cmd>` を返す + 結果 Event を受ける」形にする。`probe_io::start_ms_ime_ready_poll` と `conv_actuation` を Event 駆動に。output/ の F 約 1,430 行がここ。
5. **段 4(入口)**: `handle_hook_key_event`、`reload_config`/`run_all` の Engine 構成(P* 88)を `build_engine`/`derive_runtime_params` の純関数に。`dispatch_engine_message` は G のまま残すが、再入ポリシー(drop/repost)を表にする。

Windows 側に残る: `key_injector`(E)、`held_modifiers::send_keymap_target`、tray の E(GDI アイコン、`Shell_NotifyIconW`、メニュー表示)、`autostart`、レジストリ読み、`logging`、bootstrap の E ブロック(mutex、panic hook、フック・ホットキー・WTS・WinEvent 登録、`install_ctrl_handler`)、`tray_wnd_proc`、`dispatch_engine_message`。

## 6. 危険箇所

1. **`handle_hook_key_event`(`app/mod.rs:625-668`)— ADR-156 の「defer/replay キューの窓口」問題**。defer 判断が `RUNTIME` の外で読む 3 つのグローバル(`OUTPUT_GATE.is_active()`、`INPUT_DEFER.pending_len_nonblocking()`、`FOCUS_RESYNC`)に依存し、借用に失敗した場合(`with_app` が `None`、665)は `replay_later` に回す。判断だけ core に出して gate の読みを借用の外に残すと、判断から借用までの間に gate が変わる隙間が残る。**判断に必要な値は同じ瞬間に集めた 1 つの snapshot で渡す**。同じファミリーの反対側窓口は `output/vk_send.rs:83` の drain-before-send と `message_handlers::handle_wm_drain_output_queue`(領域外)。
2. **`dispatch_engine_message` の再入ポリシー非対称**。11 アームが再入で黙って捨て、5 アームだけ再 post。WM_TIMER が捨てられたとき、one-shot タイマーの再発火は保証されない(**未確認**: `handle_wm_timer` が借用前にタイマーを kill/再設定しているか)。Event 化するとこの非対称が見えなくなるので、アームごとの扱いを表にしてから進める。
3. **出力の順序が仕様の箇所(BUG-36/38、INV-F、ADR-103/123/128)**: `flush_raw_tsf_literal_recovery`(backspace → romaji 再送 → deferred flush)、`finish_probe_stage`(deferred 解放 → ゲート解放 → GjiEvent)、`drain_pending_deferred_before_send_if_queue_only`(`Enforced` のみ)。分割しても 1 ターン内の `Vec<Cmd>` の順序を保証し、`raw_recovery_owns_deferred`(グローバルを読む)を計画関数の入力 snapshot にすること。`architecture_guard.rs::raw_recovery_owns_deferred_call_sites_are_accounted_for` が呼び出し件数を固定しているので、移すとテストが落ちる(意図どおり)。
4. **await をまたぐ失効(INV-45 の隣)**: `probe_io::start_ms_ime_ready_poll`(BUG-13、ADR-140 Step1b)は `gen` を spawn 前に捕獲し、fence を 3 点(issue 前 2、read 直後 1)で比較し、`with_app` 内でもう一度 gen 照合する。abandon 分岐の世代照合欠落は過去に実際のバグ(doc に記録)。`conv_actuation` は `ActuationTarget::capture` 後に完了時 gen を照合し、再入時は `wrapping_add(1)` で書き込み中止(BUG-59 追補、ADR-086 INV-14)。Event 化では **`gen`/fence を Event に載せ、reducer 側で比較**する。「発行時点の IME 種別・戦略を await 後も有効とみなす」設計は INV-45(`ImeKindId` は推測値)と衝突する。`needs_f2_probe()` は `set_active_ime_kind` が `WM_IME_KIND_CHANGED` で切り替えるキャッシュなので、await 後に使う場合は再評価する。
5. **`OutputActiveGuard` の RAII 寿命**(`vk_send.rs:296`、`tsf_warmup_coord.rs:159`)。ゲートは `ChromeProbe` に move され probe 完了まで保持される。core の probe マシンが `Drop` でゲートを返す設計のまま Cmd 化すると、probe を破棄する経路(`cancel_probe`、上書き `install_pending_tsf` の warn 経路)で対を落とし、入力が defer されたまま戻らない危険がある。`mark_vk_output` のコメント(678-680)は「`with_app` は `execute_one` からの再入で使えない → グローバル atomic に書く」とあり、**グローバルが再入回避の裏口になっている**。
6. **`tray_wnd_proc` が WM_APP/WM_COMMAND の唯一の到達点**(`tray.rs:1054-1075`)。`SendMessage` 同期配送なので `GetMessageW` の戻りでは観測できない。メッセージループ側へ寄せると右クリックメニューが出なくなる(2026-07-27 の実機事故)。また `handle_tray_message` はモーダル(`TrackPopupMenu`、`ModalPumpGuard`)中に入れ子のメッセージポンプが回り、`get_gui_thread_info_with_timeout(150ms)` が主スレッドを最大 150ms 止める(BUG-34 型。上限あり)。メニューの内容は **モーダルに入る前に snapshot で確定**すること。
7. **重複による乖離**: (a) `check_keyboard_layout_on_change`(`app/mod.rs:471`)と `check_keyboard_layout`(`bootstrap.rs:562`)は同じ判定・ほぼ同じ文言。(b) `reload_config`(796-849)と `run_all`(1135-1183)の設定→パラメータ導出は重複。BUG-140 の修正を `init_ime_sync_keys` に集約した経緯があるが、`reload_config` だけは `hook::resolve_thumb_key` を再解決(815-821)し、`run_all` は初回解決値を渡す。片方だけの修正漏れが再発しやすい。**未確認**: `runtime/mod.rs:2107 apply_config_update` が Engine の `set_space_thumb_config` 等を再設定するかどうか(領域外)。
8. **`Output::on_ime_mode_focus_changed`(361-380)は 3 つの世代/ラッチを 1 関数で更新**(`ime_mode_focus_gen`、`ms_ime_gate_give_up`、`confirm_gate_deadline_override_ms`+`shift_conv_guard_gen`、BUG-49 追補2 pass-5)。分割して別々のハンドラにすると、フォーカス変更中に旧 hold の retry task が override を復活させる不具合が戻る。
9. **起動時の既定戦略の食い違いの可能性**: `TsfWarmupCoordinator::new` は `GjiFsm`(`tsf_warmup_coord.rs:64`)を既定にするが、`run_message_loop` のコメント(`app/mod.rs:671-675`)は「未検出なら MicrosoftIme 安全デフォルト」と書く。**未確認**(`sync_ime_kind_from_observation` は領域外)。`ms_ime_gate_defer` は `!needs_f2_probe()` で発動するため、起動直後のどちらが既定かで cold 経路が変わる。
10. **`win_event_proc`(`bootstrap.rs:842-882`)の関数内 static `LAST_FOCUS_HWND`**。Chrome/UWP の子オブジェクト由来の連続イベントを落とす重複排除がここにあり、belief には現れない。core に移すときは「同一 HWND なら捨てる」を明示的な入力フィルタとして移す(TsfGate の PendingWarmup 巻き戻しで held queue が捨てられる、という理由がコメントに残る)。

## 7. 未確認の点

- `tsf::probe`/`tsf::ime_mode_fsm`/`tsf_gate`/`tsf::warmup::*`/`tsf::probe_bridge` は gated(`tsf/mod.rs:24-50`、確認済み)。これらを含む型の移設が段 2・3 の前提。担当領域の分析は別エージェント(未確認)。
- `Output` の P/P* 判定は、フィールド型(`CompositionState`、`ImeModeFsm`、`TsfGate`)が Win32 に触れないことを前提にしている。`tsf/` 内部は読んでいない(brief によれば Win トークンなし)。
- WM_TIMER の再入時 drop の実害(危険 2)。
- `RAW_TSF_LITERAL` が本当にスレッド間共有か(危険、seam 表)。
- `apply_config_update`・`handle_wm_app_tray` の内部(領域外)。
- テスト数・行数: 範囲指定は `fns.py`(インデント一致で関数末尾を決める簡易判定)と目視。gji_charset_autodetect の gated 内テスト 45 行は手で補正した。
- `msime_key_assignment.rs`/`msime_legacy_keymap.rs` のテスト 14+11 本が Linux で実行されるかは未確認(`windows_impl` の外にあるものだけ)。
- 分類は行数ではなく論理で付けたので、F の中の「P に切り出せる行数」(例: `handle_tray_message` のメニュー判断、`reload_config` の導出 約 60 行)は上の F 行数に含めたまま。
