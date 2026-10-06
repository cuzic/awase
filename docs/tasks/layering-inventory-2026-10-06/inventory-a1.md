# 棚卸し A1: hook / platform / journal 領域 (develop 9df983e4)

対象: `crates/awase-windows/src/` の hook.rs, hook_channel.rs, platform.rs, timer.rs, panic_detect.rs, win32.rs, journal.rs, lib.rs, vk.rs, single_thread_cell.rs。読み取りのみ(編集・ビルド・テスト実行なし)。

数え方: 「行」は生の行数(コメント・空行込み)。この領域は doc コメントが極端に厚い(hook.rs は本体 1900 行のうちコードは 1062 行)ので、括弧内に「空行とコメント行を除いたコード行」を併記する。分類の境界は私が引いた行範囲で、区間の合計は各ファイルの本体行数に一致する(検算済み)。
分類記号: P / P* / O / E / F / G / T(依頼どおり)。

---

## 0. 領域の要約

| ファイル | 全体 | 本体 | テスト | cfg(windows) gate の原因 |
|---|---|---|---|---|
| hook.rs | 1900 | 1900 (code 1062) | 0 | lib.rs:51-52 `#[cfg(windows)] pub mod hook` |
| hook_channel.rs | 460 | 235 (152) | 225 (9 test) | モジュールは ungated(lib.rs:31)。186-234 の 4 項目だけ個別に `#[cfg(windows)]`(`win32::post_to_main_thread_quiet` / `crate::WM_KEY_FROM_HOOK` を使うため) |
| platform.rs | 1363 | 1363 (931) | 0 | lib.rs:78-79。`Output`/`SystemTray`/`Win32Timer`/`FocusTracker`(gate 済み型)を所有するため |
| timer.rs | 81 | 81 (56) | 0 | lib.rs:88-89。`SetTimer`/`KillTimer` |
| panic_detect.rs | 131 | 131 (83) | 0 | lib.rs:76-77。Windows API トークンは 0。`crate::win32::post_to_main_thread` と `crate::WM_PANIC_RESET`(gate 済み)を各 1 回呼ぶだけ |
| win32.rs | 594 | 526 (292) | 68 (4 test) | lib.rs:97-98 |
| journal.rs | 2131 | 1682 (1199) | 449 (22 test) | lib.rs:63-64。Windows 依存は 3 点だけ: `win32::SentKeyEvent`(216-230 の `From` 1 つ)と `hook::current_tick_ms()` 2 箇所(1512, 1532) |
| lib.rs | 402 | 402 (260) | 0 | モジュール宣言そのものが gate の源 |
| vk.rs | 1847 | 1038 (682) | 809 (45 test) | ungated(lib.rs:44)。個別 gate は `parse_hotkey`(760-779)と windows 定数突き合わせ(44-47, 200-208) |
| single_thread_cell.rs | 81 | 81 (40) | 0 | ungated(lib.rs:41) |
| **計** | **8990** | **7439** | **1551** | |

領域全体の分類別本体行数(生の行数):

| P | P* | O | E | F | G |
|---|---|---|---|---|---|
| 3083 | 966 | 414 | 499 | 1095 | 1382 |

- 移せる行数(P+P*): 4049(54%)。ただし P の 3083 のうち 2599 行は journal.rs(1600)と vk.rs(999)で、すでに(vk.rs は)ungated か、ungated 化が 3 継ぎ目で済む。
- 残る行数(O+E+G): 2295。G の 1382 のうち 690 は hook.rs の `HOOK_STATE`(23 フィールドの atomic 群と、それを触る 27 個の関数)。
- 分割が要る行数(F): 1095。うち 791 行は platform.rs で、`GjiFsm` 駆動・journal 記録・タイマー指示・`Output` 呼び出しが 1 関数に同居している。

---

## 1. hook.rs (1900 行、全部が本体、テスト 0)

gate: lib.rs:51-52。テストは無い(旧 `alt_impersonation_tests` は `state::alt_impersonation::tests` へ移設済み、hook.rs:1899)。ただし `tests/architecture_guard.rs` が `src/hook.rs` を文字列走査する箇所が 5 件ある(hook_callback 内の tracing 呼び出し数 = 7 固定 4993-5018、`VK_KANA swallow block`、`physical_key_state update block`、`Mutex<` 出現数 = 1、`thumb_down_timestamps` 呼び出し数 = 1)。hook_callback を分割・移動する場合は、このガードの START/END マーカー(`unsafe extern "system" fn hook_callback(` から `pub fn now_timestamp_us` まで)も更新が要る。

### 分類別内訳(本体行数)

| P | P* | O | E | F | G |
|---|---|---|---|---|---|
| 65 | 435 | 246 | 237 | 227 | 690 |

### 関数表(全関数)

| 名前 | 行範囲 | 行数 | 分類 | 一言メモ |
|---|---|---|---|---|
| (定数・import) | 1-28, 177-184 | 36 | G | `LLKHF_*`、診断リング上限 |
| `HookState` struct/impl/static | 29-176 | 148 | G | 23 フィールド(§1.2)。doc は「20 件」だが実数は 23 |
| `classify_key` | 185-208 | 24 | P | `HookConfig`(ungated の state 型)と `scan_to_pos` だけを使う。そのまま移せる |
| `pub use resolve_thumb_key` ほか | 209-217 | 9 | G | `state::alt_impersonation` の再 export |
| `apply_alt_impersonation` | 218-272 | 55 | P* | 判定本体は純粋関数 `decide_alt_impersonation`。ここは `HOOK_STATE.alt_{l,r}_{was_down,impersonating}` と `cached_engine_enabled` の読み書き。seam: atomic 群 → 所有型 `AltImpersonationState` を引数で受ける |
| `classify_ime_relevance` | 273-304 | 32 | P | `vk::ImeKeyKind` だけを使う |
| `tick_hook_alive` / `hook_alive_tick_ms` / `hook_tid_*` ×4 | 305-331 | 27 | G | atomic アクセサ。`tick_hook_alive` は時計を直接読む |
| `is_physical_key_down` / `physical_key_held_ms` | 332-350 | 19 | G | `physical_key_state` / `physical_key_down_at_ms` の読み。`physical_key_held_ms` は `current_tick_ms()` を直接読む |
| `win_key_held` / `alt_key_held` | 351-415 | 65 | P* | 判定は純粋関数 `is_held_fresh`(`state/win_key_guard`)。seam: 時計の注入 + 物理キー表を所有型で受ける |
| `ime_mode_key_injection_blocked_by_modifier` | 416-434 | 19 | P* | 上の 2 つの OR。同じ seam |
| `inject_alt_menu_mask` | 435-454 | 20 | E | 自己注入の Ctrl down+up(`win32::send_input_safe`) |
| `reset_physical_key_state` | 455-495 | 41 | G | 全 VK クリア + 親指ラッチ + Alt 系ラッチ。メインスレッドから呼ばれる(runtime/mod.rs:2363, message_handlers.rs:1053) |
| `clear_hook_latches_for_app_disable` | 496-571 | 76 | G | `SuppressionEdge` で分岐(純粋)+ atomic 書き込み。呼び出し元 runtime/focus_tracking.rs。architecture_guard が本体を文字列走査 |
| `clear_hook_latches_for_watchdog_reinstall` | 572-636 | 65 | G | 上の Leave 分岐と**意図的に重複**(doc に理由。ガードが関数本体を直接走査するため共有化できない) |
| `ctrl_consumed_since_down` | 637-642 | 6 | G | |
| `cached_hook_config` | 643-665 | 23 | G | 5 つの config atomic から `HookConfig` を組む(毎打鍵) |
| `thumb_vk_codes` / `set_thumb_vk_codes` / `thumb_down_timestamps` / `set_*` ×5 / `is_focus_app_disabled` | 666-755 | 90 | G | メインスレッド → フックスレッドの config 受け渡し |
| `is_alt_impersonation_active` | 756-777 | 22 | G | |
| `passthrough_or_swallow_for_impersonation` | 778-797 | 20 | F | 「なりすまし中なら飲み込む(`LRESULT(1)`)、でなければ `CallNextHookEx`」: 判定(1 行)+ E が同居。core 側に `HookDisposition::{Pass,Swallow}` を返す純粋関数を出し、`CallNextHookEx` だけ残す |
| `current_tick_ms` | 798-805 | 8 | O | `GetTickCount64`。この領域だけで 7 箇所(platform.rs ×5, journal.rs ×2)が直接呼ぶ |
| `os_last_input_tick_ms` | 806-827 | 22 | O | `GetLastInputInfo` |
| `os_idle_ms` | 828-861 | 34 | P* | 32bit wrapping の算術(コード 12 行)。seam: `os_last_input_tick_ms()` を引数化 |
| `foreground_window_is_elevated` | 862-936 | 75 | O | `GetForegroundWindow`+`OpenProcess`+`OpenProcessToken`+`GetTokenInformation`。返す型 `bool` |
| `is_secure_desktop_active` | 937-965 | 29 | O | `OpenInputDesktop`。返す型 `bool` |
| `low_level_hooks_timeout_ms` | 966-1000 | 35 | O | レジストリ `RegGetValueW` + `OnceLock`。返す型 `Option<u32>` |
| `OWN_HOOK_HANDLE` / `MY_HOOK_GEN`(thread_local)、`HOOK_GEN`、`is_zombie_hook_thread` | 1001-1075 | 75 | G | 旧フックスレッドの世代判定(opus round1 M2) |
| `CallbackResult` enum | 1076-1084 | 9 | P | `runtime/executor.rs` が戻り値に使う。hook.rs に置く理由は無い |
| `HookGuard` + `Drop` | 1085-1174 | 90 | E | `PostThreadMessageW(WM_QUIT)` + join(`run_with_timeout_in` で 500ms 有界化)+ 孤児スレッドプール |
| `install_hook` | 1175-1267 | 93 | E | スレッド起動、`SetWindowsHookExW`、メッセージポンプ、`Unhook`。TID ハンドシェイク |
| `build_raw_key_event` | 1268-1306 | 39 | P* | 組み立てのみ。seam: `now_timestamp()` を引数化 |
| `NEXT_PRESS_ID` / `assign_press_id` | 1307-1320 | 14 | P* | 判定は `awase::types::is_press_start`(純粋)。seam: 静的カウンタ → 所有 |
| `TEST_INJECTION_MARKER` ほか定数 | 1321-1332 | 12 | G | |
| `send_hook_watchdog_canary` | 1333-1366 | 34 | E | 自己注入 Ctrl down+up(マーカー `HOOK_WATCHDOG_CANARY_MARKER`) |
| `is_test_injection`(2 つの cfg 版) | 1367-1384 | 18 | O | 環境変数を `OnceLock` で 1 回読む。debug ビルドのみ有効 |
| `is_self_injected` | 1385-1391 | 7 | P* | マーカー 3 値の比較。seam: `INJECTED_MARKER`/`TSF_MARKER`/`IME_KANJI_MARKER` が `tsf/output.rs:15-27`(gate 済み)にあるので、定数だけ ungated 側へ |
| `push_hook_ime_mode_diagnostic` / `drain_*` | 1392-1409 | 18 | G | `Mutex<VecDeque>`(HOOK_STATE で唯一の Mutex。ガードが出現数 1 を固定) |
| **`hook_callback`** | **1410-1882** | **473 (code 271)** | **F** | §1.1 に論理ブロック別 |
| `now_timestamp_us` / `now_timestamp` | 1883-1897 | 15 | O | `Instant`(起動時点からの µs) |
| (末尾コメント) | 1898-1900 | 3 | G | |

### 1.1 hook_callback の論理ブロック(1410-1882)

| ブロック | 行範囲 | 行数 | 分類 | 内容 / 分割案 |
|---|---|---|---|---|
| a. 世代ガード | 1410-1429 | 20 | G | `is_zombie_hook_thread()` → `CallNextHookEx` |
| b. 生イベントの取り出し | 1430-1461 | 32 | O | `tick_hook_alive()`、`ncode<0`、`KBDLLHOOKSTRUCT` 読み、カナリア即飲み込み、`is_injected` 導出。フックに残る |
| c. IME モードキー診断 | 1462-1505 | 44 | F | `ImeKeyKind::from_vk` で絞り、`last_ime_mode_hook_ms.swap`、`win32::last_actuation_issue_us()` との差、`tracing::debug!`、診断リングへ push、`post_to_main_thread_quiet(WM_HOOK_IME_MODE_DIAGNOSTIC)`。中身は生イベントの項目(vk/is_down/self_injected/injected/scan)だけなので、生イベント列から導出できる。E(post)だけ残す |
| d. 自己注入の素通し | 1506-1510 | 5 | F | 判定 + `CallNextHookEx` |
| e. 物理キー状態の更新 | 1511-1580 | 70 | P* | `physical_key_state.swap`(`was_down` 取得)、`physical_key_down_at_ms`、`physical_down_vk_by_identity`(BUG-181)。判定は `vk::physical_identity_slot` / `stale_down_vk_on_up`(純粋)。**入力は「生 vk(なりすまし前)」「scan」「拡張ビット」「is_keydown」「!injected」**。seam: 3 配列 → 所有型 `PhysicalKeyTable::apply(&mut self, ev) -> was_down` |
| f. `disable_apps` バイパス | 1581-1592 | 12 | F | `focus_app_disabled`(メインスレッドが書く)を読んで `CallNextHookEx`。**フックの戻り値を決める同期判定** |
| g. VK_KANA / VK_DBE_ROMAN・NOROMAN の飲み込み | 1593-1699 | 107 (code ~50) | F | 判定(`is_injected`・`alt_key_held()`・`cached_swallow_alt_kana_mode_switch`)+ `tracing::info!` ×4 + `inject_alt_menu_mask()`(E)+ `LRESULT(1)`。core 側に `fn hook_swallow_decision(vk, is_injected, alt_held, swallow_cfg) -> Option<{mask: bool}>` を出し、ログと注入は残す。**フックの戻り値を決める同期判定** |
| h. overflow ラッチ中の素通し | 1700-1714 | 15 | F | `HOOK_KEYS.is_overflow_latched()` → `passthrough_or_swallow_for_impersonation`。**同期判定** |
| i. config 取得 + Alt なりすまし | 1715-1747 | 33 | P* | `cached_hook_config()`、`apply_alt_impersonation`(vk を書き換える)、診断ログ 2 件 |
| j. 親指ラッチ | 1748-1808 | 61 | P* | `thumb_latch_identity`/`should_release_thumb_latch`(純粋)を使い、`*_thumb_down_scan` と `*_thumb_down_at_us` を更新。入力は「なりすまし後の vk」「scan」「拡張ビット」「is_keydown」「!injected」と、時刻(T1、`now_timestamp()`) |
| k. Ctrl 消費追跡 | 1809-1828 | 20 | P* | `is_ctrl_variant(vk)`、`is_physical_key_down(L/RCONTROL)`、親指キーは対象外。入力に e の結果(物理 Ctrl 状態)が要る |
| l. 分類 + 修飾スナップショット + イベント組み立て | 1829-1858 | 30 | O 12 + P* 18 | `classify_key`(P)、`thumb_down_timestamps()`、**`read_os_modifiers()`(`GetAsyncKeyState` で Alt と Win を読む = O)**、`LLKHF_ALTDOWN`、なりすまし中は `alt=false`、`build_raw_key_event` |
| m. 世代再判定 + ring へ積む + 起床 | 1859-1882 | 24 | F | `HOOK_KEYS.produce` → `request_engine_wake` → 結果で `LRESULT(1)` か `passthrough_or_swallow...`(overflow 時の判断) |

合計 20+32+44+5+70+12+107+15+33+61+20+30+24 = 473。分類別: G 20, O 44, F 207, P* 202。

### 1.2 HOOK_STATE の全フィールド表(新構想の検討材料)

構想: 「フックを薄くし、生イベントを順序つきでリングに載せ、状態は取り込み側の畳み込みで再計算する」。フィールドは 23 個(doc コメントの「20 件」は古い。`Ordering` 使用数は grep で Relaxed 75 / Release 9 / Acquire 8 / SeqCst 1 / AcqRel 1、doc の「Relaxed 49 ほか」も古い)。

「畳み込みで再計算できるか」の列:
- 可: 生イベント列(vk, scan, 拡張ビット, is_keydown, injected, timestamp)だけから決定できる。
- 条件付き: 下に書く前提(RawKeyEvent の項目追加、リング外イベントの追加、順序の取り決め)が要る。
- 不可: イベント列から出ない(設定・エンジン状態・フック自身の生存・同期ハンドシェイク)。

| # | フィールド | 型 | 書き手 | 読み手 | 畳み込みで再計算 |
|---|---|---|---|---|---|
| 1 | `ime_mode_diagnostics` | `Mutex<VecDeque<Record>>`(上限 64) | フックスレッド(`hook_callback` c ブロック → `push_hook_ime_mode_diagnostic`) | メイン(`message_handlers` が `drain_hook_ime_mode_diagnostics` → journal) | **条件付き**: 内容は生イベントの項目そのもの(vk, is_down, self_injected, injected, scan, 直前との間隔)。ただし **self_injected・カナリア・飲み込んだイベントはリングに載らない**ので、それらを載せる「生記録」用のレコード種を足す必要がある(診断の目的がまさに「自己注入として飲み込まれたのか、フックに届いていないのか」の区別) |
| 2 | `last_ime_mode_hook_ms` | `AtomicU64` | フック(`swap`) | フック(`swap` の戻り値のみ。診断専用) | **可**: 直前の IME モードキーのイベント時刻との差。1 と同じ前提 |
| 3 | `cached_thumb_vks` | `AtomicU32`(上位16bit=左, 下位16bit=右) | メイン(`set_thumb_vk_codes`: app/bootstrap.rs, runtime/mod.rs の設定再読込) | フック(`cached_hook_config` を毎打鍵)、メイン(`thumb_vk_codes`: runtime/mod.rs ×2, app/mod.rs, gji_charset_autodetect.rs) | **不可**: 設定。畳み込みに載せるなら「設定変更イベント」をリングに順序つきで入れる(下記「危険箇所」) |
| 4 | `hook_alive_tick_ms` | `AtomicU64` | フック(`tick_hook_alive`: **全コールバックの先頭**、自己注入・カナリア含む)、app/bootstrap.rs(初期化) | メイン(message_handlers.rs:576 の watchdog、runtime/mod.rs:1945/1970 のカナリア確認) | **不可**: 「フックが呼ばれたか」の観測そのもの(エンジンスレッド停止・フック飢餓の検出が目的)。取り込み側の畳み込みに置くと、検出対象の失敗モードで一緒に止まる。フック側に残す |
| 5 | `hook_tid_init_slot` | `AtomicU32` | フックスレッド(`hook_tid_set`/`fail`)、メイン(`hook_tid_reset`) | メイン(`install_hook` のスピン待ち) | **不可**: 起動ハンドシェイク |
| 6 | `physical_key_state[256]` | `[AtomicBool;256]` | フック(e ブロック: `slot.swap`、`stale_down_vk_on_up` で別 VK の枠を落とす)、メイン(`reset_physical_key_state`, `clear_hook_latches_*` の Ctrl/Shift 6 枠) | フック(`alt_key_held`/`win_key_held` 経由の飲み込み判定、`read_os_modifiers`、k ブロック)、メイン(`observer/focus_observer.rs::read_os_modifiers`, `output/held_modifiers.rs` ×12, `runtime/key_pipeline.rs:150`, `ime.rs`, `output/mod.rs:630`) | **条件付き**: (i) 更新は `was_down` 取得のために**生 vk**(なりすまし前)と**拡張ビット**が要るが、`RawKeyEvent` は拡張ビットを持たず、`vk_code` はなりすまし後。(ii) e ブロックの後にある飲み込み(g)・バイパス(f)・overflow 素通し(h)の対象イベントは**リングに載らないのに状態は更新済み**。リングだけから畳むと f/g/h 分が欠ける。(iii) メイン側の強制クリア(WTS_SESSION_UNLOCK、パニックリセット、`disable_apps` 離脱、フック再インストール)を、リング内の順序つき「リセット印」にする必要がある。(iv) 読み手に「いま」の値が要るもの(`output/held_modifiers.rs` の送信時再同期)と「そのイベント時点」の値が要るもの(`key_pipeline.rs:150`)が混在 |
| 7 | `physical_key_down_at_ms[256]` | `[AtomicU64;256]`(`GetTickCount64` の ms) | フック(e ブロック、自動リピートでは上書きしない)、メイン(同 6) | フック/メイン(`physical_key_held_ms` → `win_key_held`/`alt_key_held` の stale 判定 `WIN_KEY_HELD_STALE_MS`) | **条件付き**: 6 と同じ前提 + 時刻系の変換(`RawKeyEvent::timestamp` は `Instant` 起点の µs で、これは ms の tick。時計を揃える必要)。**`alt_key_held()` はフックスレッド上で飲み込み判定に同期で使う**(g ブロック)ので、この 1 本(Alt の押下時刻)は取り込みを待てずフックに残る |
| 8 | `physical_down_vk_by_identity[512]` | `[AtomicU16;512]` | フック(e ブロック)、メイン(`reset_physical_key_state`) | フック(e ブロックのみ) | **可**(拡張ビット追加が前提): BUG-181/BUG-131(VK_DBE_HIRAGANA は Down=0xF2・Up=0xF0)。畳み込みの一部として 6 と一体 |
| 9 | `left_thumb_down_at_us` | `AtomicU64`(0=非押下) | フック(j ブロックの `mark_down`/`clear`)、メイン(`set_thumb_vk_codes`, 3 つのリセット関数) | フック(l ブロックで RawKeyEvent のスナップショットへ)、メイン(**ライブ読み**: runtime/mod.rs:494 `build_ctx`, message_handlers.rs:706 タイマー経路) | **可**(条件付き): 入力は「なりすまし**後**の vk」「scan+拡張ビット」「is_keydown」「!injected」「時刻」。時刻は現状 T1(`now_timestamp()` をラッチ時点で取る)で `RawKeyEvent::timestamp`(T2)とずれる(types.rs の doc: 減算・比較禁止)。畳み込みでは全部をイベントの時刻 1 本から作ることになり、`left_thumb_consumed`(エンジン側)との時計の取り決めは保たれる。タイマー経路のライブ読み(build_ctx / timer)は、取り込み側が「現在のラッチ」を保持すれば足りる |
| 10 | `right_thumb_down_at_us` | 同上 | 同上 | 同上 | 同 9 |
| 11 | `left_thumb_down_scan` | `AtomicU32`(ラッチ識別子、0=非ラッチ) | フック、メイン(リセット) | フック(j ブロック)のみ | **可**: BUG-132。識別子 = scan | 拡張ビット(0x100)。**メインのリセットとフックの武装が交差して「武装済みなのに scan=0」で固着しうる**(doc に既出)問題は、単一スレッドの畳み込みなら起きない |
| 12 | `right_thumb_down_scan` | 同上 | 同上 | 同上 | 同 11 |
| 13 | `ctrl_consumed_since_down` | `AtomicBool` | フック(k ブロック)、メイン(`clear_hook_latches_*` ×2) | メイン(`runtime/key_pipeline.rs:192` の**ライブ読み**) | **可**(6 の物理 Ctrl 状態と親指 vk 設定が前提)。**畳み込みにするとむしろ改善**: 今は Ctrl+無変換 の判定がエンジン処理時点のライブ値を読むので、リングに溜まっている間に Ctrl↑ が来てフラグが false に戻ると、「Ctrl が他キーで消費済み」の判定が変わりうる(ADR-129 と同型。**未確認**: 実際にこの順序が起きる条件) |
| 14 | `cached_keyboard_model_is_us` | `AtomicBool` | メイン(`set_keyboard_model`) | フック(`cached_hook_config`) | **不可**(設定) |
| 15 | `cached_left_alt_impersonation_enabled` | `AtomicBool` | メイン(`set_alt_impersonation_enabled`) | フック | **不可**(設定) |
| 16 | `cached_right_alt_impersonation_enabled` | `AtomicBool` | 同上 | フック | **不可**(設定) |
| 17 | `cached_engine_enabled` | `AtomicBool` | メイン(`runtime/executor.rs` の `UiEffect::EngineStateChanged`) | フック(`apply_alt_impersonation` の発動条件) | **不可**(エンジン状態)。畳み込みなら取り込み側がエンジン状態を直接持つので不要になるが、「押下時点で確定」という判定の基準時刻が、フック時点 → 処理時点へ後ろにずれる |
| 18 | `focus_app_disabled` | `AtomicBool` | メイン(`runtime/focus_tracking.rs`、書き `Release`) | フック(f ブロック `Relaxed`)、アクセサは `Acquire`(使用箇所 0) | **不可**(フォーカス状態)。**フックの戻り値を決める同期判定**なのでフックに残る |
| 19 | `cached_swallow_alt_kana_mode_switch` | `AtomicBool`(既定 true) | メイン(`set_swallow_alt_kana_mode_switch`) | フック(g ブロック) | **不可**(設定)。同期判定なのでフックに残る |
| 20 | `alt_l_impersonating` | `AtomicBool` | フック(`apply_alt_impersonation`)、メイン(リセット 3 関数) | フック(`is_alt_impersonation_active`: l ブロック、overflow 素通し h)、メイン(**ライブ読み**: runtime/mod.rs:491 `build_ctx`、message_handlers.rs:703 タイマー経路で `modifiers.alt=false` に補正) | **条件付き**: 入力は「生 vk」「拡張ビット」「is_keydown」+ 17 + 14〜16。押下単位でラッチする設計(押している間の設定変更でスタックしない)は畳み込みでも同じに書ける。ただし h(overflow 素通し)がフック側でこの値を同期に読むので、「飲み込むべきか」のためのフック側の写しは残る |
| 21 | `alt_r_impersonating` | 同上 | 同上 | 同上 | 同 20 |
| 22 | `alt_l_was_down` | `AtomicBool` | フック、メイン(リセット) | フック(`apply_alt_impersonation`)のみ | **可**(20 と一体) |
| 23 | `alt_r_was_down` | 同上 | 同上 | 同上 | 同 22 |

HOOK_STATE の外にあるフック関連の静的:

| 名前 | 場所 | 書き手 / 読み手 | 畳み込みで再計算 |
|---|---|---|---|
| `HOOK_GEN`(`AtomicU64`)、`MY_HOOK_GEN`・`OWN_HOOK_HANDLE`(thread_local) | hook.rs:1001-1075 | `install_hook` / フックスレッド | 不可(旧フックスレッドの排除。ring の単一 producer 前提を守る) |
| `NEXT_PRESS_ID` | hook.rs:1309 | フック(`assign_press_id`、`Relaxed`) | 可(取り込み側で「非注入の非リピート KeyDown」に採番すれば足りる。was_down は 6 から出る) |
| `HOOK_KEYS`(ring)、`WAKE_PENDING`、`WAKE_POST_FAILED`、`WAKE_POST_FAILED_LIFETIME_COUNT` | hook_channel.rs:184-199 | フック / メイン | 不可(伝送そのもの) |
| `LAST_ACTUATION_ISSUE_US` | win32.rs:251 | `send_input_safe`(任意スレッド)/ フックの診断ログ | 不可(自分の送信の記録) |

### 1.3 畳み込み構想に対する読み取り結果(要点)

1. 「リングに載るイベント」と「状態が更新されるイベント」が一致しない。`physical_key_state` は e ブロック(`ncode`・自己注入・カナリア・世代ガードの後)で更新され、f(`disable_apps`)・g(KANA/ROMAN 飲み込み)・h(overflow 素通し)で戻るイベントも更新済みだが、これらはリングに載らない。`alt_*`・親指ラッチ・`ctrl_consumed` は逆に g/h の**後ろ**で更新されるので、飲み込まれたイベントでは更新されない。**フィールドごとに「どこまでのイベントを見るか」が違う**。
2. `RawKeyEvent`(`src/types.rs:264-345`)に無いが畳み込みに要るもの: 拡張ビット(`LLKHF_EXTENDED`)、なりすまし前の生 vk、`LLKHF_ALTDOWN` 単独(今は `modifier_snapshot.alt` に混ぜ込み済み)。
3. フックの戻り値(飲み込む/通す)を決める同期判定は取り込み側に移せない: `focus_app_disabled`、`cached_swallow_alt_kana_mode_switch`、`alt_key_held()`(Alt の押下時刻 + stale 判定)、overflow ラッチ、世代ガード、`alt_*_impersonating`(overflow 時)。この 6 項目分の状態はフック側に残る。
4. `read_os_modifiers()` の Alt と Win は `GetAsyncKeyState`(OS 全体の状態、他プロセスの注入も含む)で、Ctrl/Shift だけが物理キー表由来。生イベント列の畳み込みに置き換えると、他ツールが注入した Alt が数えられなくなる(挙動変更)。
5. 逆の利点: 取り込み側(単一スレッド)の畳み込みにすると、`ctrl_consumed_since_down`(key_pipeline.rs:192)と `physical_key_state`(key_pipeline.rs:150)のライブ読みが「イベント時点の値」になり、ADR-129 の事故型(drain replay 中に「いま」の値を読む)を構造的に避けられる。親指ラッチの「メインのリセットとフックの武装が交差して固着」(hook.rs:1764-1768 に doc)も無くなる。
6. すでに畳み込みの手書き模写がある: `vk.rs:1330-1370` の `PhysSim`(テスト用の小さな状態機械)は hook_callback の e ブロックのコピー。`PhysicalKeyTable::apply` を 1 つ作ればフックとテストが共有できる。

---

## 2. hook_channel.rs (460 行)

gate: モジュールは ungated。186-234 の 4 項目(`WAKE_POST_FAILED`、`WAKE_POST_FAILED_LIFETIME_COUNT`、`request_engine_wake`、`wake_post_failed_lifetime_count`、`recover_stuck_wake_if_needed`)だけ個別 `#[cfg(windows)]`。テスト 225 行(9 test。2 スレッド負荷テスト含む)は **Linux で既に走る**。

| 分類 | P | G | E | O | F |
|---|---|---|---|---|---|
| 本体 235 | 184 | 15 | 15 | 7 | 14 |

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| `HookKeyRing`(`produce`/`consume_all`/`consume_one`/`take_dropped_and_clear_latch`/`take_max_occupancy`/`peek_max_occupancy`/`has_pending`/`is_overflow_latched`) | 1-184 | 184 | P | SPSC リング(1024 件)+ overflow ラッチ(dropped 数と latch を 1 語の `AtomicU64` に詰める)。`RawKeyEvent` は core の型。**そのまま新 crate に置ける**(フック側と取り込み側が共有する継ぎ目) |
| `HOOK_KEYS`、`WAKE_PENDING`、`WAKE_POST_FAILED`、`WAKE_POST_FAILED_LIFETIME_COUNT` | 184-199 | 16 | G | グローバル static |
| `request_engine_wake` | 200-214 | 15 | E | `post_to_main_thread_quiet(WM_KEY_FROM_HOOK)` |
| `wake_post_failed_lifetime_count` | 215-221 | 7 | O | |
| `recover_stuck_wake_if_needed` | 222-234 | 13 | F | ring の `has_pending` と 2 フラグの swap + ログ + `request_engine_wake` の再発行。判定(「保留があって WAKE_PENDING が立っている = 固着」)を純粋関数にできる |

Windows 側の依存の種類: `post_to_main_thread_quiet` 1、`crate::WM_KEY_FROM_HOOK` 1、`lifetime_counter`(ungated)1。HWND/HIMC/COM なし。

---

## 3. platform.rs (1363 行、テスト 0)

gate: lib.rs:78-79。`Output`/`SystemTray`/`Win32Timer`/`FocusTracker` を所有する。

### 分類別内訳(本体行数)

| P | P* | O | E | F | G |
|---|---|---|---|---|---|
| 7 | 187 | 53 | 46 | 791 | 279 |

### 関数表

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| `WindowsPlatform` struct、`PendingLiteralVk`、`Debug` | 23-56 | 34 | G | |
| `new` | 58-92 | 35 | G | `win32::install_sent_input_stamp_source` で thread_local に採番元を設定(E 的な配線) |
| `drain_journal_entries` | 93-120 | 28 | P* | seam: `win32::drain_sent_input_trace()`(thread_local)を「送信記録の供給元」trait/引数へ |
| `gji_state_label`、`push_journal_entry`、`note_gji_transition` | 122-140 | 19 | P* | `output.gji_state_label()` への依存 |
| `note_tsf_probe_started_from_gji_action` | 141-169 | 29 | F | `output.pending_deferred_len()`・`output.composition.*` を読んで journal を積む(+ `warn!`)。「記録する内容」の組み立ては core 側、読み取りは Output |
| `note_tsf_probe_completed`、`reset_probe_tick_counters`、`note_literal_detect_record`、`flush_pending_literal_vk_as_aborted`、`consume_literal_detect_trace` | 170-296 | 127 | P* | 判定は `journal_policy::*`(純粋)と `GiveUpTracker`(純粋)。seam: `hook::current_tick_ms()` ×4 を時計注入、`output.ime_mode_focus_gen.get()`、`output.composition.consecutive_count()` を引数化 |
| `take_giveup_evidence` | 297-302 | 6 | P | |
| Output 委譲 11 個(`set_conv_mode_authority`、`mark_composition_cold_focus_change`、`is_composition_warm_in_tsf`、`on_composition_cancel`、`update_injection_mode`、`notify_focus_changed`、`confirm_tsf`、`bypass_tsf`、`on_focus_change_tsf`、`on_tsf_warmup_timeout`、`try_hold_key`、`has_pending_tsf_work`) | 304-390 | 87 | G | 薄い委譲(3〜12 行)。新構想では消える層 |
| `advance_tsf_probe` | 392-465 | 74 | F | `output.step_probe()`(結果 = 次の timer 指示・GJI 応答・learned_tsf・完了 cold_seq)→ journal 記録の要否判定(純粋)→ `focus.learn_injection_mode_tsf`(状態書き込み + 永続化)→ `update_injection_mode`/`mark_composition_cold` → `apply_timer_command`。core 側に `ProbeTickOutcome → Vec<Cmd>` を出す |
| `dispatch_gji_response` / `_from` | 467-578 | 112 | F | `GjiAction` の解釈(`StartProbe`/`CancelProbe`/`DiscardPending`)+ タイマー設定(`self.timer.set/kill`)+ `output.gji_store_probe_id`/`cancel_probe` + 再帰(Unicode は即 `WarmupComplete`)。`GjiFsm` は ungated の `tsf::gji_fsm` なので、`Response → Vec<Cmd>` の写像は core に出せる |
| `composition_native_f2_down` | 579-603 | 25 | F | `output.is_tsf_mode()`/`is_composition_warm()` を読んで分岐(BUG-31/BUG-173)。スナップショットを受ける純粋関数にできる |
| `gji_on_focus_change` | 604-695 | 92 | F | FSM 駆動 + journal + **`spawn_local` で IMC 読み(`get_ime_conversion_mode_fenced_async`)を非同期に走らせ、戻りで `with_app` を再取得して `ime_mode_focus_gen` 世代で失効判定**。await をまたぐ失効の典型(§危険箇所) |
| `gji_on_ime_on`、`gji_sync_from_belief`、`dispatch_gji_event`、`gji_on_ime_off`、`gji_on_timer_long_idle`、`gji_on_composition_reset`、`gji_on_native_f2_consumed`、`gji_on_start_composition`、`gji_on_end_composition`、`drain_pending_composition_events` | 696-840 | 145 | F | 各 7〜29 行。`tsf::observer::gji_idle_ms()`/`take_pending_start_composition()` 等のグローバル atomic(O)を読んで `GjiEvent` を作り、FSM → journal → dispatch。**観測(idle_ms, pending フラグ)をイベントとして core に渡し、FSM 駆動と journal 化は core** |
| `flush_raw_tsf_literal_recovery` | 841-875 | 35 | F | `output.flush_raw_tsf_literal_recovery()` + journal の要否(純粋)+ `drain_output_post_send_effects` |
| `drain_output_post_send_effects` | 877-925 | 49 | F | `take_pending_drain_before_send_flush` → journal、`drain_pending_gji_key_responses` → dispatch、`take_composition_reset` → reset、timer 起動。BUG-28 の後処理の集約点 |
| `apply_timer_command` | 927-933 | 7 | E | `Win32Timer` への指示のみ |
| `PlatformRuntime::send_keys` | 941-975 | 35 | F | `needs_belief_sync_on`(純粋)で ADR-203 の突合 → `sync_gji` → `request_unicode_observation` → `output.send_keys`(E)→ `drain_output_post_send_effects`。**Mutex を取る `probe_or_recovery_in_flight()` を条件の最後に回している**(フックの応答時間のため) |
| `reinject_key` | 976-979 | 4 | E | `event.reinject()` → `SendInput` |
| `set_timer`/`kill_timer`/`post_ime_refresh`/`update_tray`/`show_balloon`/`set_tray_layout_name` | 981-1031 | 40 | E | |
| `set_ime_open` | 991-1010 | 20 | F | プロファイル判定(`can_use_imm32_cross_process`)→ `spawn_local` で IMM 書き込み(fire-and-forget、戻り値は「dispatch 成功」) |
| `GjiSyncSink::sync_gji` | 1034-1073 | 40 | F | `GjiFsmSync` → `GjiEvent` の写像。`injection_mode` を settle 時点で読む(ADR-089 §2.4)。スナップショットを受ければ純粋 |
| `TsfComposition` impl 4 個 | 1074-1094 | 21 | O | `Output` への問い合わせ |
| `on_reinject_key` | 1095-1117 | 23 | F | 確定キー KeyDown の cold 化判定(`is_composition_warm`)+ mark_cold + FSM reset |
| `on_ime_applied_inner` | 1118-1187 | 70 | F | `ActuationReceipt`、`ime_mode_fsm.borrow_mut().on_set_open_applied`、`ms_ime_gate_give_up`/`confirm_gate_deadline_override_ms`/`bump_shift_conv_guard_gen` の直書き、`mark_composition_cold`、`receipt.settle`。**Output 内部の状態を直接書き換える**belief 周辺の判断 |
| `build_ime_control_view` | 1188-1219 | 32 | O | `focus` + `tsf_obs()`(グローバル)から `ImeControlView<'_>`(借用)を作る。返す型 `ImeControlView`(借用)。所有スナップショットにすれば P* になる |
| `apply_ime_open_with_view` | 1220-1240 | 21 | G | `ImeController::apply` への薄い委譲 |
| `set_ime_open_ordered` | 1241-1279 | 39 | F | `log_shadow_warrant` → **`order.into_actuation()` が `None` なら書かない(授権判定)** → `PlatformRuntime::set_ime_open`。ADR-090 A-2。actuation 合流点の一つ(fix-requires-evidence の「IME actuation 合流点」行) |
| `is_engine_processing` | 1280-1291 | 12 | P* | `timer.is_active(TIMER_PENDING/SPECULATIVE)`。seam: タイマー表の写し |
| フォーカス委譲 12 個 | 1292-1363 | 72 | G | `focus.*` への薄い委譲 |

### 領域サマリ(platform.rs)

- 移せる(P+P*): 194 行 / 残る(O+E+G): 378 行 / 分割が要る(F): 791 行。
- Windows 側の依存の種類と件数(grep): `self.output.` 52、`self.focus.` 12、`self.timer.` 8、`self.tray.` 3、`tsf::observer::` グローバル 13、`spawn_local` 5(うち 2 が実コード、IMC 読みと IMM 書き込み)、`crate::hook::current_tick_ms()` 5、`with_app` 実呼び出し 1(683 行、`spawn_local` の中)、`event.reinject()`(`unsafe`)1。HWND・HIMC・COM の直接使用は無い(HWND は `usize` で `focus.update` へ渡すだけ)。
- 同期 Mutex を取る呼び出しの存在: `probe_or_recovery_in_flight()`(`send_keys` 内)。

---

## 4. timer.rs (81 行、テスト 0)

gate: lib.rs:88-89(`SetTimer`/`KillTimer`)。

| 分類 | P* | E | G |
|---|---|---|---|
| 本体 81 | 34 | 28 | 19 |

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| `Win32Timer` struct + `new` + `Default` | 14-28, 77-81 | 20 | G | 2 つの `HashMap`(論理 ID ↔ OS ID) |
| `set` | 30-44 | 15 | E | `SetTimer(None,0,ms,None)` の戻り(OS が割り当てる ID)で表を更新し、古い OS タイマーを `KillTimer`。表の更新が `SetTimer` の戻り値に依存するので、「Cmd を受けて Win32 を叩き、結果(OS ID)を Event で返す」形(`TimerArmed{logical, os_id}`)にする継ぎ目 |
| `kill` | 46-55 | 10 | E | |
| `resolve` / `is_active` / `current_os_id` | 57-75 | 19 | P* | 表の問い合わせ。seam: 表を core の `TimerTable` に移し、`set`/`kill` は `os_id` を返す executor に |

Windows 側の依存: `SetTimer`/`KillTimer` 各 2 箇所。呼び出し元: `platform.rs`(8)、`runtime/*`。

---

## 5. panic_detect.rs (131 行、テスト 0)

gate: lib.rs:76-77。Windows API トークンは 0。理由は `win32::post_to_main_thread(WM_PANIC_RESET)` 1 回のみ。

| 分類 | P | P* | G |
|---|---|---|---|
| 本体 131 | 70 | 35 | 26 |

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| `RAPID_IME_TIMESTAMPS`、`PANIC_TRIGGER_COMBOS`(`SingleThreadCell`)、`set_panic_trigger_combos` | 1-19, 125-131 | 26 | G | グローバル static。`bootstrap.rs:1162`、`app/mod.rs:829` が設定 |
| `PanicTriggerCombo`、`RapidPressTracker`(`push`/`clear`) | 21-90 | 70 | P | OFF→ON→OFF が 2000ms 以内、純粋な 3 要素リング。**テストが 0 件**(Linux に移せばすぐ書ける) |
| `record_ime_keydown` | 92-104 | 13 | P* | seam: `tracker.push` が true のとき `post_to_main_thread(WM_PANIC_RESET)` を呼ぶ → 戻り値 `bool`(または Cmd)にして呼び出し側で post |
| `get_panic_trigger_direction` | 106-124 | 19 | P* | seam: 静的 `PANIC_TRIGGER_COMBOS` → 引数 |

注意: ファイル冒頭の doc(「フックコールバックから直接呼べる」「フックコールバックはメインスレッドで実行される」)は古い。実際の呼び出し元は `app/mod.rs:630-636` の `handle_hook_key_event`(リングを取り込む側、メインスレッド)のみで、hook.rs からの参照は無い(grep 済み)。

---

## 6. win32.rs (594 行: 本体 526 + テスト 68)

gate: lib.rs:97-98。テスト 4 個は `windows::Win32::UI::Input::KeyboardAndMouse::INPUT` を組み立てるので **Windows 専用のまま**(Linux へ移すなら、判定関数の引数を `INPUT` から `(vk, flags)` に変えたあとテストも書き直し)。

| 分類 | P | P* | O | E | F | G |
|---|---|---|---|---|---|---|
| 本体 526 | 52 | 102 | 108 | 128 | 63 | 73 |

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| import・re-export(`run_with_timeout`、`LeakedThreadPool` 等) | 1-27 | 27 | G | |
| `HwndExt::non_null` | 28-42 | 15 | O | HWND の null チェック(Windows 側の補助) |
| `ForegroundScope`(`INVALID`、`is_valid`) | 44-61 | 18 | P | `{pid:u32, hwnd:isize}` の値型 |
| `foreground_scope` | 62-80 | 19 | O | `GetForegroundWindow` + `get_window_process_id`。返す型 `ForegroundScope` |
| `post_to_main_thread` / `_with` / `_quiet` / `_inner` | 82-160 | 79 | E | `PostMessageW`。宛先は `runtime::engine_window::engine_hwnd()`(グローバル)。`WM_QUIT` を `WM_ENGINE_QUIT_REQUEST` に差し替える |
| `input_may_mutate_conv` | 162-178 | 17 | P* | `vk::vk_may_mutate_conv`(純粋)。seam: `&INPUT`(ユニオン読み)→ `(vk, is_unicode)` |
| `ime_actuation_marker_kind` | 180-220 | 41 | P* | `dwExtraInfo` と vk から `kanji_marker`/`tsf_marker_warmup` を判定。同じ seam |
| `actuation_vks` | 221-243 | 23 | P* | 同じ seam |
| `LAST_ACTUATION_ISSUE_US`、`last_actuation_issue_us` | 245-257 | 13 | G | `AtomicU64` |
| `SentKeyEvent`、`SentInputBatch` | 259-284 | 26 | P | プレーンデータ。`journal.rs` がこれを参照して gate されている |
| `SENT_INPUT_TRACE`、`SENT_INPUT_STAMP_SOURCE`(thread_local)、`install_sent_input_stamp_source`、`drain_sent_input_trace` | 285-316 | 32 | G | thread_local。**書き込みはワーカースレッド分も含むが drain はメインスレッドだけ**(doc に既出: ワーカーから呼ばれた分は上限で古い順に捨てられる) |
| `sent_key_events` | 317-335 | 19 | P* | `INPUT` → `SentKeyEvent`。seam 同上 |
| `send_input_safe` | 336-398 | 63 | F | `SendInput`(E)の前後に、`conv_mutation::bump()`(BUG-34)、`probe_actuation_fence::bump()`(ADR-140、`SendInput` より前で完了が必須要件)、`LAST_ACTUATION_ISSUE_US`、`shadow_send_trace::record_send_input`(ADR-159)、`tracing::debug!`、journal 採番、`SENT_INPUT_TRACE` への記録。**このクレート全 `SendInput` の唯一の合流点**(BUG-34 横展開 Step0-a)。core 側に「送信バッチ → 付随記録(bump 要否・マーカー種別・記録)」の判定を出し、`SendInput` と時刻採取だけ残す |
| `to_wide` | 399-406 | 8 | O | UTF-16 変換(Win32 の定型) |
| `spawn_command_with_null_stdio` | 407-427 | 21 | E | 子プロセス起動(BUG-79/BUG-134) |
| `show_error_dialog` | 428-454 | 27 | E | `MessageBoxW` |
| `GuiThreadResult` | 455-462 | 8 | P | 値型(`Option<HWND>` を含むので厳密には P*。HWND → `WindowId` で移る) |
| `get_gui_thread_info_with_timeout` | 463-526 | 64 | O | `run_with_timeout` + `GetGUIThreadInfo`。返す型 `GuiThreadResult` |
| テスト | 527-594 | 68 | T | 4 test、Linux 不可 |

Windows 側の依存の種類と件数: `INPUT` ユニオン読み(unsafe)6 関数、`SendInput` 1、`PostMessageW` 2、`GetForegroundWindow` 3、`GetGUIThreadInfo`/`GetWindowThreadProcessId` 1、`MessageBoxW` 1、thread_local 3、`run_with_timeout` 1。

---

## 7. journal.rs (2131 行: 本体 1682 + テスト 449)

gate: lib.rs:63-64。**Windows 依存は 3 点だけ**(`crate::win32::SentKeyEvent`(216-230 の `From` と、テスト 2062)、`crate::hook::current_tick_ms()` 2 箇所(1512 `dump_to_file`、1532 `dump_to_file_for_report`))。`focus::current`・`tsf::literal_facts`・`state::*`・`journal_policy` はすべて ungated。テスト 22 件は上の 3 点を直せば Linux で回る(`quanta::Mock` で時計を注入済み)。

| 分類 | P | P* |
|---|---|---|
| 本体 1682 | 1600 | 82 |

| 名前 / 区間 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| import・定数・`DumpError` | 1-37 | 37 | P | |
| `KeyEventSummary`(`from_raw`)、`DriftGiveUpDiagnosticRecord`、`ImeVkDiagnostic`、`HookImeModeDiagnosticRecord`、`DecisionKind`、`PhysicalDispositionSummary`、`DeferredRecoveryOutcomeSummary`、`SentKeyEventSummary` | 39-215 | 177 | P | |
| `From<win32::SentKeyEvent>` | 216-231 | 16 | P* | seam: `SentKeyEvent` を ungated 側へ |
| `JournalEntry`(28 variant)+ サイズ固定(264 バイト) | 233-510 | 278 | P | |
| `JournalEnvelope`・`FocusEndpoint`・`OldestElapsedByLane`・`EvictedByLane`・`LaneCapacities`・`JournalLane`(`push` の evict/順序挿入)・`JournalLanes`・`lane_kind` | 512-693 | 182 | P | 4 レーン(State/Timing/Actuation/KeyInput) |
| 変換補助(`decision_kind_shape` 等) | 695-782 | 88 | P | |
| `JournalEntry::emit_tracing`(全 variant 網羅、`_ =>` 禁止) | 784-1205 | 422 | P | `tracing` は OS 非依存。`architecture_guard` が `?`/`%`/ワイルドカードを機械的に禁止 |
| `UnifiedJournal`、`JournalStamper`(`reserve`/`stamp`)、`record`、`absorb`、`route_to_lane`、`record_key_input`(自動リピート畳み込み)、`len`、`to_json`、`oldest_elapsed_ms_by_lane`、`evicted_by_lane` | 1207-1508 | 302 | P | `quanta::Clock` 注入済み |
| `dump_to_file`、`dump_to_file_for_report` | 1510-1576 | 67 | P* | seam: `hook::current_tick_ms()` を引数化。`std::fs`/`temp_dir` は OS 非依存 |
| `entries_by_seq`、`Default`、`DumpTriggerTracker` | 1578-1681 | 104 | P | |
| テスト | 1683-2131 | 449 | T | 22 test |

### journal は入力の記録として足りているか(レコード種別ごと)

結論: **足りない**。journal の `KeyInput` は「エンジン処理後」の要約で、フック層の生入力の再現には使えない。

| レコード | 何を持っているか | 持っていないもの(生入力の再生に要る) |
|---|---|---|
| `KeyInput`(`KeyEventSummary`+ `state_before`/`state_after`(String)+ `decision`(種別と effect 数)+ `physical`(Allow/Suppress+理由)+ `repeat_count`/`last_*`) | vk, scan, is_down, injected, timestamp_us, key_class, alt/ctrl/shift(win は無し) | **拡張ビット、`extra_info`、`was_down`(畳み込みの判定にだけ使い、保存しない)、`press_id`、親指スナップショット 2 つ、`physical_pos`、`ime_relevance`(vk から導出可)、`LLKHF_ALTDOWN` の分離、なりすまし前の vk**。自動リピートは `repeat_count` と最初・最後の時刻に畳まれ、各リピートの間隔は失われる。`decision` は effect の中身ではなく個数のみ。**リングに載らなかったイベント(f/g/h の飲み込み・バイパス、自己注入、カナリア)は一切記録されない**。構築点は `runtime/key_pipeline.rs:228` の 1 箇所(ガードで固定) |
| `HookImeModeDiagnostic` | vk, is_down, self_injected, injected, scan, 直前の IME モードキーとの間隔(ms) | 拡張ビット、`extra_info`。IME モードキー(0x15/0x16/0x17/0x19/0x1A/0xF0-0xF6 付近)限定。リング 64 件、メインが `WM_HOOK_IME_MODE_DIAGNOSTIC` で吸い上げるまでに溢れると古い順に捨てる |
| `SentInput` | `send_input_safe` 1 回ごとに vk, scan, up, unicode, ch, marker、`accepted`、発行時刻 | awase が**送った**側の記録として足りている(入力側ではない) |
| `ImeEvent` / `ImeOpenApplied` / `PressWriteClaim` / `ActuationDecision` / `ImeActuation` | belief reducer への入力、actuation の決定点記録 | 判断の入出力としては構造化済み(ADR-163)。キー入力そのものではない |
| `TimerFired` | timer_id, state_before/after | |
| `ConvClassifyCall` | `classify_conv_transition` の引数と戻り値 | リプレイの入力源として既に使われている(`tests/journal_replay.rs`) |
| `ClockAnchor` | `elapsed_ms` / OS tick / hook µs の対応 | |

新構想で「生イベント列から状態を再計算する」を journal で回帰テストにする場合は、リングに載る生レコード(vk, scan, 拡張ビット, flags, injected, 自己注入・カナリア・飲み込み印, timestamp, リセット印, 設定変更印)を、別レーンとして `KeyInput` とは別に残す必要がある。

---

## 8. lib.rs (402 行、テスト 0)

| 分類 | P | P* | E | G |
|---|---|---|---|---|
| 本体 402 | 25 | 52 | 45 | 280 |

| 名前 / 区間 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| クレート属性・モジュール宣言・再 export | 1-122 | 122 | G | gate の源。ungated は 14 モジュール、`#[cfg(windows)]` は約 24 |
| `ProcessFlags`(`main_thread_id`/`quit_requested`/`elevated`)+ アクセサ 6 | 123-173 | 51 | G | Ctrl+C ハンドラ(別スレッド)から読むため atomic。`SeqCst` と `Relaxed` の非対称は意図的(doc) |
| `RawTsfLiteralPending`(`set_pending`/`take_pending`)、`RAW_TSF_LITERAL` | 174-225 | 52 | P* | `backs`/`romaji`/`escape_composition` のペイロード。seam: 静的 → 所有フィールド(`Mutex<String>` を使うのはこの領域で 2 箇所目) |
| `RUNTIME`、`with_app`、`with_app_ref`、`with_app_or_repost`、`with_app_or_repost_with` | 227-275 | 49 | G | 再入時は `None` + `tracing::warn!`。再入で消えるメッセージは `post_to_main_thread` で再投函 |
| `TIMER_*` 定数 10 個 | 276-300 | 25 | P | 純粋 usize。core の `TimerId` に写せる |
| `WM_*` 定数、`FOCUS_KIND_UPDATE_NO_APP_KIND` | 301-357 | 57 | G | `WM_APP + n`(gate 済み)。欠番の注記あり |
| `RawKeyEventExt::reinject` | 358-402 | 45 | E | `SendInput` |

Windows 側の依存: `WM_APP`、`windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, ...}`、`with_app` 13 箇所(定義 + 参照)、`RUNTIME`(`SingleThreadCell`)。

---

## 9. vk.rs (1847 行: 本体 1038 + テスト 809)

ungated(lib.rs:44)。P 999 行、P* 39 行(`windows` 定数との突き合わせ 4 箇所と `parse_hotkey`)。テスト 45 件は **Linux で既に走る**。関数表は省略(P のみ)。

フック関連で重要な純粋関数(すべて `const fn`、hook.rs から呼ばれる): `ImeKeyKind::from_vk`(259)、`thumb_latch_identity`(485、scan | 拡張ビット 0x100)、`should_release_thumb_latch`(503)、`physical_identity_slot`(514、scan を 0..512 に写す、scan=0 と 0xFF 超は None)、`stale_down_vk_on_up`(529)、`is_passthrough`(539)、`classify_modifier`(587)、`is_ctrl_variant`(613)。

P* の 39 行: `windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT,MOD_CONTROL,MOD_SHIFT}` を使う `parse_hotkey`(760-779)は定数 3 個なのでローカル定数に置換すれば P。突き合わせ(44-47, 200-208)は Windows ターゲットのビルド時検査で、残す。

---

## 10. single_thread_cell.rs (81 行、テスト 0)

P(81 行)。ungated。`RefCell<Option<T>>` + `unsafe impl Sync`(「アクセスはメインスレッドだけ」という前提を型は強制しない)。`RUNTIME`、`PANIC_TRIGGER_COMBOS`、`RAPID_IME_TIMESTAMPS`、`DUMP_TRIGGER` などが使う。借用中は `None` を返す(UB なし)ので、再入は握りつぶされる(`with_app` の `must_use` はこの対策)。新 crate に移すなら `unsafe impl Sync` の安全性根拠(単一スレッド保証)を呼び出し側の契約として残す必要がある。

---

## 11. 領域全体の集計

### 11.1 分類別の本体行数

| ファイル | P | P* | O | E | F | G | 本体計 |
|---|---|---|---|---|---|---|---|
| hook.rs | 65 | 435 | 246 | 237 | 227 | 690 | 1900 |
| hook_channel.rs | 184 | 0 | 7 | 15 | 14 | 15 | 235 |
| platform.rs | 7 | 187 | 53 | 46 | 791 | 279 | 1363 |
| timer.rs | 0 | 34 | 0 | 28 | 0 | 19 | 81 |
| panic_detect.rs | 70 | 35 | 0 | 0 | 0 | 26 | 131 |
| win32.rs | 52 | 102 | 108 | 128 | 63 | 73 | 526 |
| journal.rs | 1600 | 82 | 0 | 0 | 0 | 0 | 1682 |
| lib.rs | 25 | 52 | 0 | 45 | 0 | 280 | 402 |
| vk.rs | 999 | 39 | 0 | 0 | 0 | 0 | 1038 |
| single_thread_cell.rs | 81 | 0 | 0 | 0 | 0 | 0 | 81 |
| **計** | **3083** | **966** | **414** | **499** | **1095** | **1382** | **7439** |
| テスト(T) | | | | | | | 1551 |

テスト内訳: hook_channel 225(9)、win32 68(4、Windows 専用のまま)、journal 449(22)、vk 809(45)。このうち 1483 行(hook_channel + journal + vk)は journal の 3 点を直せば Linux で走る(hook_channel と vk は既に走る)。

### 11.2 seam 一覧(この領域で共通して必要になる継ぎ目)

| seam の種類 | 現れるファイル数 / 関数数 | 該当 |
|---|---|---|
| グローバル静的の atomic 群(`HOOK_STATE` ほか)→ 所有型 | 3 ファイル(hook.rs, win32.rs, lib.rs)/ hook.rs の 27 関数 + win32 2 + lib 1 | `apply_alt_impersonation`、`win_key_held`、`alt_key_held`、`physical_key_*`、各 `set_*`、`reset_*`、`clear_hook_latches_*` |
| 時計の直接読み → 注入 | 4 ファイル / 約 14 箇所 | `hook::current_tick_ms()`(platform.rs ×5、journal.rs ×2、hook.rs ×3)、`now_timestamp()`(hook.rs の `build_raw_key_event`、`hook_callback`、win32 の `send_input_safe` ×2) |
| `INPUT` ユニオン → `(vk, flags, marker)` | 1 ファイル / 4 関数 | win32.rs の `input_may_mutate_conv`、`ime_actuation_marker_kind`、`actuation_vks`、`sent_key_events` |
| `win32::SentKeyEvent`(プレーンデータ)を ungated 側へ | 2 ファイル / 1 impl + テスト | journal.rs の gate 解除に必須 |
| マーカー定数(`INJECTED_MARKER` ほか 3 値、`tsf/output.rs:15-27`)を ungated 側へ | 2 ファイル / 3 関数 | hook.rs の `is_self_injected`、win32.rs の `ime_actuation_marker_kind` |
| `with_app` / `spawn_local` + 世代で失効 | 1 ファイル / 1 関数(実呼び出し) | platform.rs `gji_on_focus_change`(683 行) |
| `Output` / `FocusTracker` への直接アクセス | 1 ファイル / 約 50 関数 | platform.rs(`self.output.` 52、`self.focus.` 12) |
| `tsf::observer::*` グローバル atomic(O)の読み | 1 ファイル / 12 関数(13 呼び出し) | platform.rs の `gji_on_*` 群 |
| 借用スナップショット(`ImeControlView<'_>`)→ 所有 | 1 ファイル / 2 関数 | platform.rs `build_ime_control_view`、`apply_ime_open_with_view` |
| `post_to_main_thread` を戻り値(Cmd)へ | 3 ファイル / 3 箇所 | panic_detect.rs `record_ime_keydown`、hook_channel.rs `request_engine_wake`、hook.rs の診断 post |
| HWND 型を引数に取る | 1 ファイル / 2 型 | win32.rs の `HwndExt`・`GuiThreadResult`(HWND → `WindowId`)。platform.rs は HWND を `usize` で受けるので継ぎ目なし |
| thread_local | 3 ファイル / 4 個 | hook.rs ×2(`OWN_HOOK_HANDLE`、`MY_HOOK_GEN`)、win32.rs ×2(`SENT_INPUT_TRACE`、`SENT_INPUT_STAMP_SOURCE`) |
| `crate::hook::*` / `crate::win32::*` への逆向き依存 | journal.rs(`hook` 2、`win32` 1)、platform.rs(`hook` 5、`win32` 3)、win32.rs(`hook` 2)、hook.rs → `journal`(`HookImeModeDiagnosticRecord`)、hook.rs → `win32`(4 関数)、hook.rs → `observer::focus_observer`(`read_os_modifiers`)と `win32` ⇄ `hook` の相互参照 | 循環の源。`hook ⇄ win32`(`now_timestamp_us` と `post_to_main_thread_quiet`/`last_actuation_issue_us`)は時計を別モジュールに出せば切れる |

### 11.3 新 crate への移動候補の順序案(依存の少ない順)

1. **vk.rs**(P 999)と **single_thread_cell.rs**(P 81): すでに ungated で Linux でテスト済み。`parse_hotkey` のローカル定数化だけ。
2. **hook_channel.rs の `HookKeyRing`**(P 184 + テスト 225): そのまま。フック側と取り込み側の共有型として最初に切り出すのが自然。
3. **hook.rs の純粋部**: `classify_key`、`classify_ime_relevance`、`CallbackResult`(P 65)。依存は ungated の `vk`/`scanmap`/`state` だけ。
4. **panic_detect.rs の `RapidPressTracker`**(P 70)+ `record_ime_keydown` を bool 返しに(P* 32): 現在テスト 0 件なので、移すと同時にテストが書ける。
5. **win32.rs の `SentKeyEvent`/`SentInputBatch`/`ForegroundScope`**(P 52)を先に出し、これで **journal.rs の gate 解除**(P 1600 + P* 82 + テスト 449): 継ぎ目は 3 点(`SentKeyEvent`、`current_tick_ms` ×2)だけで、最も行数あたりの効果が大きい。
6. **マーカー定数と時計**(`INJECTED_MARKER` 等、`current_tick_ms`/`now_timestamp_us` を trait にして注入): 4 と 5 の前提にもなる。
7. **win32.rs の判定関数**(`input_may_mutate_conv`、`ime_actuation_marker_kind`、`actuation_vks`、`sent_key_events`: P* 102): `INPUT` → `(vk, flags, marker)` に変える。`send_input_safe`(F 63)は「判定 → 付随記録」を core に出して分割。
8. **hook.rs の畳み込み部**(e/j/k ブロック、`win_key_held`/`alt_key_held`、`apply_alt_impersonation`、`build_raw_key_event`、`assign_press_id` = P* 約 400): `PhysicalKeyTable` / `ThumbLatch` / `AltImpersonation` の所有型に切り出し、フック側は atomic の写しだけ持つ。§1.3 の前提(拡張ビット等の `RawKeyEvent` 追加、リング外イベントの扱い)を先に決める。**最後**に回す理由は、ここで挙動が変わる(§1.3 の 4)から。
9. **platform.rs**(F 791): 最大の塊。`ProbeTickOutcome → Vec<Cmd>`、`GjiResponse → Vec<Cmd>`、`gji_on_*` を観測 → イベントに、の順に分割。`Output`(領域外)の分割が前提なので、他領域の棚卸し結果に従う。
10. **timer.rs**(P* 34 + E 28): `TimerTable` の core 化は 9 の `apply_timer_command` と一緒に。
11. **lib.rs**: `TIMER_*`(P 25)は `TimerId` に写して core へ。`RawTsfLiteralPending`(P* 52)は所有フィールド化。残りは G/E でWindows 側。

### 11.4 危険箇所(分けると隙間ができる・await をまたぐ失効・不変条件)

1. **フックの同期判定は分けられない(§1.3 の 3)**。`LowLevelHooksTimeout`(既定 5000ms、hook.rs:31-36 の doc)内に戻らないとフックが外される。f/g/h(`focus_app_disabled`、KANA/ROMAN 飲み込み、overflow 素通し)は同期で戻り値を決めるので、状態の写しがフック側に残る。取り込み側の畳み込みと二重管理になるので、**写しと畳み込み結果の食い違い**を検出する仕組み(リセット印・世代)が要る。関連: BUG-08/BUG-61/BUG-62(KANA の飲み込み)、BUG-78(`disable_apps` 離脱の Ctrl/Shift スタック)、issue #165(フック飢餓)。
2. **キー KeyUp の消失で状態がスタックする**(BUG-48 Win キー、BUG-62 Alt、BUG-78 mstsc、2026-07-09 の右 Shift)。現在の対策(`reset_physical_key_state`、`clear_hook_latches_for_app_disable` Leave、`clear_hook_latches_for_watchdog_reinstall`、stale 判定 `WIN_KEY_HELD_STALE_MS`)は、いずれも**メインスレッドから atomic を直接書く**ことで成り立つ。畳み込みにするなら、これらをリング内の順序つき「リセット印」にしないと、リセットとフックの更新が交差する(今は atomic の順序で緩く解決している)。さらに overflow で落ちたイベントは畳み込みに届かないので、**overflow 後の resync**(ラッチ解除時に物理状態の再読込)が必須になる。現状 `take_dropped_and_clear_latch` はカウントしか返さない。
3. **overflow ラッチ中は `apply_alt_impersonation` を通らない**(h ブロックは i ブロックより前で戻る)。overflow 中に新しく Alt を押すと `alt_*_impersonating` が更新されず、`passthrough_or_swallow_for_impersonation` は古い値で判断して実 Alt を OS に通しうる(Alt 単独タップのメニュー起動の恐れ)。**未確認**: 実際に overflow が起こる頻度と、この順序で問題になるか。ADR-102/hook_channel の指摘2で overflow の設計は議論済みだが、この組み合わせの記載は見つけていない。
4. **`hook_callback` 内のログ呼び出し数 = 7 を `tests/architecture_guard.rs:4993` が固定**している(hook_channel.rs の不変条件「フックコールバック上でログを出さない」が実際は 7 件、うち IME モードキーごとの `tracing::debug!` は常時発火)。`tracing` の同期ファイル I/O でフックが詰まる経路が M2/M6(世代ガード)の存在理由。薄いフックにする際は、`tracing` 呼び出しを 0 に近づけるのが利点になる。
5. **世代ガード(zombie 判定)は hook_callback の 5 箇所**(冒頭、IME 診断 push 直前、物理状態更新直前、親指ラッチ直前、`produce` 直前)に分散している。`tracing` の同期 I/O が各所で詰まりうるため(hook.rs:1056-1065 の doc)。薄いフックにして書き込み箇所が減れば、判定も 2 箇所(冒頭と `produce` 直前)に減らせるが、**単一 producer 前提の SPSC**(hook_channel.rs の `unsafe impl Sync`)なので、`produce` 直前の再判定は残す。
6. **時計が 3 系統ある**: `GetTickCount64`(ms、`physical_key_down_at_ms` と診断)、`Instant`(µs、`RawKeyEvent::timestamp` と親指スナップショット)、`quanta::Clock`(journal)。畳み込みで `physical_key_held_ms` を再現するには ms と µs を揃える。`RawKeyEvent::timestamp`(T2)と親指の T1 が数 µs ずれる(types.rs の doc: 比較禁止)ので、畳み込みでは T1 を廃して 1 本にし、エンジン側の `left_thumb_consumed` との整合を確認する(`tests/thumb_context_guard.rs`、ADR-129)。
7. **`gji_on_focus_change`(platform.rs:604-695)は await をまたぐ失効**: `spawn_local` の中で IMC を非同期に読み、戻りで `with_app` を取り直して `ime_mode_focus_gen` の一致で捨てる。`probe_actuation_fence` との二重の失効管理(`FencedProbeOutcome::Abandoned` と checkpoint3)。core 側に出す場合、**世代トークンを Cmd に含め、Event に返させる**形にしないと、await 中のフォーカス変更が隙間になる(BUG-59、ADR-140 Step1b)。
8. **`send_input_safe` の副作用の順序**: `probe_actuation_fence::bump()` は `SendInput` より前に完了する(ADR-140 決定B の必須要件)。F 分割で「判定は core、`SendInput` は executor」にしても、bump を core の判定の側に置くと、`SendInput` との順序が Cmd の返し方次第になる。この関数は BUG-34 横展開 Step0-a の「全 `SendInput` の唯一の合流点」でもある。
9. **actuation 合流点の台帳との接触**: platform.rs の `set_ime_open_ordered`(`order.into_actuation()` による授権)と `apply_ime_open_with_view` は `fix-requires-evidence.md` の「IME actuation 合流点」行と `lints/actuation_call_guard/src/lib.rs::RESTRICTED_CALLS`、`architecture_guard` の出現数ガードの対象。移動・分割すると、ガードの許可リストの更新が要る(complexity-budget.md は未発効)。
10. **`ctrl_consumed_since_down` と `physical_key_state` のライブ読み**(key_pipeline.rs:150, 192)は ADR-129 型の窓を持つ(§1.2 の 13)。畳み込みでイベント時点の値にすると**挙動が変わる**ので、`tests/thumb_context_guard.rs` と ime_key_sequence_golden の期待値を確認してから。
11. **ガードテストの文字列走査**: `architecture_guard.rs` が `src/hook.rs`(5 件)、`src/platform.rs`(複数)、`src/win32.rs`(出現数 1)、`src/journal.rs`(1)を直接読む。関数の移動・改名は、本体の挙動が同じでもテストを落とす。移動 PR にはガードの更新を同梱する。

### 11.5 未確認の点

- `Ordering` の使用数は grep の件数(コメント内の言及を含みうる)。doc の「Relaxed 49 ほか」が古いことは確認したが、正確な数は未確認。
- §1.2 の 13、§11.4 の 10 の「リング滞留中に Ctrl↑ が来て判定が変わる」順序が実際に起こるか(エンジンスレッドの処理遅延がどれほどか、`INPUT_DEFER` の drain 経路で滞留するキーに対してどう読まれるか)は未確認。コード上は窓がある、までを確認。
- §11.4 の 3(overflow 中の Alt なりすまし)の実害は未確認。
- `read_os_modifiers` のコメントは「`GetKeyState`」「`GetAsyncKeyState`」が混在(runtime/mod.rs:483 は `GetKeyState` と書き、実装は `GetAsyncKeyState`)。実装を読んだのは `observer/focus_observer.rs:15-42` のみで、`GetAsyncKeyState` を LL フック内で呼んだときの「いま処理中のキーの反映状態」は未確認。
- `HookConfig`(`state/mod.rs` 側の型)、`state::alt_impersonation`、`state::win_key_guard`、`state::app_suppression` は ungated と判断したが、各ファイル自体は読んでいない(lib.rs の `pub mod state` が ungated であることだけで判断)。
- `FocusTracker`・`Output`・`ImeController` の内部は領域外のため未読。platform.rs の F の分割案は、それらの分類結果に従う。
- `hook.rs::CallbackResult` を `runtime/executor.rs` が使っていることは確認したが、`hook_callback` 自体は `CallbackResult` を使っていない(戻り値は直接 `LRESULT`)。かつて hook_callback が返していた名残とみられるが、履歴は未確認。

---

# 追補 A: win32.rs / hook.rs の `win32::` 呼び出しを S / B / A で再分類(ポート化の検討)

依頼: 依存性注入で Linux からテストできるようにする前提で、各関数を (S)即時同期 / (B)ブロックしうる(時間制限つき)/ (A)完了待ち・後で結果が来る、に分け、OS 非依存側から見たポート(trait メソッド、引数と戻り値は所有型のみ)と、タイムアウトを値で表せるかを評価する。
確認した事実(`crates/win32-async/src/thread_timeout.rs:89-150`): `run_with_timeout` は**呼び出し元スレッドを `recv_timeout` で同期的に止める**。ワーカーを 1 本 spawn し、タイムアウトなら `None` を返してワーカーを孤児プールへ(`LEAKED_THREADS` 上限 8)、プール満杯なら**待たずに即 `None`**、ワーカーが結果なしで終われば `None`。つまり名前に反して(B)で、(A)ではない。(A)は `offload`/`offload_timeout`/`race_with_timeout`(`win32-async` の async 版)と `PostMessageW`、`SetTimer`。

## A.1 win32.rs の関数(全項目)

| 関数 | 種別 | 止める最大時間 / 完了の形 | 戻り値(今) | ポート案(1 行、所有型のみ) |
|---|---|---|---|---|
| `HwndExt::non_null` | S | 0 | `Option<HWND>` | ポート不要(Windows 側の内部ヘルパー。`WindowId(isize)` への変換だけ) |
| `foreground_scope` | S | 0(`GetForegroundWindow` + `GetWindowThreadProcessId`) | `ForegroundScope{pid,hwnd:isize}`(失敗は `INVALID`) | `fn foreground_scope(&self) -> ForegroundScope` — すでに所有型。**失敗が値(`INVALID`)で表れているので、Linux の偽実装で任意値を返せる** |
| `post_to_main_thread` / `_with` / `_quiet` / `_inner` | S(投函は即時)/ 結果は A | 0。処理は後でメインループが行う | `bool`(投函成否。エンジン HWND 未作成の間は `false`) | `fn post(&self, msg: CoreMsg, wparam: usize, lparam: isize) -> bool`(`CoreMsg` は core 側の enum。`u32` の `WM_*` 番号は Windows 側で写す)。失敗は `bool` で既に値。**フック用の `_quiet` はログ無し版なので、同じポートでフック用/通常用を分ける必要は無く、ログ有無は実装側の都合** |
| `input_may_mutate_conv` / `ime_actuation_marker_kind` / `actuation_vks` / `sent_key_events` | S(純粋な判定) | 0 | `bool` / `Option<&str>` / `Vec<u16>` / `Vec<SentKeyEvent>` | **ポートではなく純粋関数**: `fn classify_send(batch: &[SentKey]) -> SendFacts`(`SentKey{vk,scan,up,unicode,marker}`)。`INPUT` ユニオンの読みだけ Windows 側 |
| `send_input_safe` | S(`SendInput` は即時)+ 付随記録 | 0(OS のキューへ積むだけ。受理数が戻る) | `u32`(受理件数) | `fn send_input(&self, batch: &[SentKey]) -> u32`。付随の `conv_mutation::bump`・`probe_actuation_fence::bump`・記録は `classify_send` の結果を受けて core が決める(`bump` は `SendInput` より前、の順序は「ポート呼び出しの前に core が先にフェンスを進める」で表現できる)。Linux の偽実装は送信を記録して `accepted` を返す(`accepted < len` を注入して部分受理を試験できる) |
| `last_actuation_issue_us` | S | 0 | `u64`(0 = 未発行) | core 側が自分で持つ値(最後の `send_input` の時刻)にして、ポート不要にする |
| `drain_sent_input_trace` / `install_sent_input_stamp_source` | S(thread_local の読み書き) | 0 | `Vec<SentInputBatch>` / `()` | ポート不要(`send_input` の戻りか、core が送信時に自分で記録する) |
| `to_wide` | S | 0 | `Vec<u16>` | ポート不要 |
| `spawn_command_with_null_stdio` | S(構築のみ。実際の spawn は呼び出し元) | 0 | `std::process::Command` | `fn spawn_detached(&self, path: String, args: Vec<String>) -> Result<(), SpawnError>`(core が起動する場面があるなら) |
| `show_error_dialog` | **B**(`MessageBoxW` はユーザーが閉じるまで戻らない) | **無制限**(時間制限なし。呼び出しスレッドを止める) | `()` | `fn show_error(&self, title: String, message: String)`。起動失敗の場面でしか使わないので core 側のポートにしない案が妥当 |
| `get_gui_thread_info_with_timeout(timeout)` | **B** | **最大 `timeout`**(呼び出し側の指定: tuning 30ms、tray 150ms、ime.rs 150ms / 200ms、ime_diagnostic 100ms)。呼び出しスレッドを同期で止める | `GuiThreadResult{focused_hwnd:Option<HWND>, thread_id:u32}`(**タイムアウトは値で表れていない**: 超過時は `GetForegroundWindow()` にフォールバックし `thread_id=0`、`run_with_timeout` が `None` を返した情報は捨てられる) | `fn gui_thread_info(&self, timeout: Duration) -> GuiInfo`、`GuiInfo{ focused: Option<WindowId>, thread_id: u32, timed_out: bool }`。**今の型では「タイムアウトしたのか、本当に thread_id が 0 なのか」を区別できない**ので、`timed_out`(または `Result<GuiInfo, Timeout>`)を足す。戻り型は所有型のみ(`WindowId` は `isize` の newtype)。Linux の偽実装は `timed_out: true` を注入でき、呼び出し側の「150ms だけ待つ」分岐を試験できる |
| `run_with_timeout` / `run_with_timeout_in`(re-export) | **B** | **最大 `timeout` + ワーカー起動コスト**(`recv_timeout`)。プール満杯なら 0 で即 `None` | `Option<T>`(`None` = タイムアウト・プール満杯・ワーカー異常終了の 3 つを区別しない) | 汎用の `run_with_timeout` をポートにしない。**個々の API ごとにポートの戻り値を `Result<T, Timeout>` とし、`Timeout` を `TimedOut` / `PoolFull` / `WorkerFailed` に分ける**(今は 3 つとも `None`)。ログ(`error!`/`warn!`)で区別しているだけなので、値にして core が見えるようにすると、「プール満杯 = Win32 API が恒久的に詰まっている」を core が判断材料にできる |
| `LeakedThreadPool` | G(孤児管理の static) | — | — | 実装側に隠す(ポートの外) |

## A.2 hook.rs が呼ぶ `win32::` と Windows API

| 呼び出し(場所) | 種別 | 止める最大時間 / 完了の形 | ポート案 |
|---|---|---|---|
| `win32::send_input_safe`(`inject_alt_menu_mask`:452、`send_hook_watchdog_canary`:1364、**いずれもフックコールバックの中または watchdog**) | S | 0 | `send_input`(上と同じ)。**フックスレッドから呼ぶ場合は「フック内で呼べる(ロック・ログ・ブロッキング無し)」という制約を trait の doc 契約にする**。偽実装は記録のみ |
| `win32::post_to_main_thread_quiet`(1504、hook_channel.rs の `request_engine_wake` も) | S(投函)/ A(処理) | 0 | `post`(`bool`)。失敗時にログを出さず `WAKE_POST_FAILED` を立てる設計(フック内でロックを取らない)が要るので、**ポートの戻り値 `bool` をそのまま呼び出し側が見る**形が適合 |
| `win32::last_actuation_issue_us`(1483) | S | 0 | core 自身が保持(上記) |
| `win32::run_with_timeout_in(&HOOK_JOIN_LEAKED_THREADS, 500ms, join)`(`HookGuard::drop`:1157) | **B** | **メインスレッドを最大 500ms**(WM_TIMER ハンドラ内の再インストール経路。専用プール 4) | `fn stop_hook(&self, guard: HookHandle, wait: Duration) -> StopOutcome`、`StopOutcome::{Joined, Leaked, PostFailed}`。今は `join` の成否を `tracing::error!` に出すだけで呼び出し側へ値で返さない。値にすれば「リークして続行」を core が journal に残せる |
| `PostThreadMessageW(WM_QUIT)`(1143) | S / A | 0 / フックスレッドが後で終了 | `HookHandle` の内部 |
| `SetWindowsHookExW`+`GetMessageW` ループ(`install_hook`) | **A**(別スレッドでメッセージポンプ。`hook_tid_init_slot` のスピン待ちで**メインスレッドを `SetWindowsHookExW` の完了まで待たせる**: 上限なし、`std::hint::spin_loop`) | **無制限のスピン**(フックスレッド起動が失敗すると `u32::MAX` で抜けるが、スレッドが `SetWindowsHookExW` の中で詰まると戻らない。**未確認**) | `fn install_hook(&self) -> Result<HookHandle, InstallError>`(Windows 側の executor。core から呼ぶ場面はブートストラップと watchdog の再インストールのみ) |
| `current_tick_ms`(`GetTickCount64`)/ `now_timestamp`(`Instant`) | S | 0 | `trait Clock { fn tick_ms(&self) -> u64; fn now_us(&self) -> u64 }`。**Linux の偽実装は手動で進める時計**(`quanta::Mock` と同型) |
| `os_last_input_tick_ms` / `os_idle_ms` | S | 0 | `fn os_idle_ms(&self, now_tick_ms: u64) -> Option<u64>`(失敗は `None` で既に値) |
| `foreground_window_is_elevated` | **B 的**(`OpenProcess` + `OpenProcessToken` + `GetTokenInformation` は通常即時だが、他プロセスのトークン取得で待たされうる。時間制限は**付いていない**) | 未確認(タイムアウト無し) | `fn foreground_is_elevated(&self) -> bool`(失敗は `false` に倒す設計が doc にある) |
| `is_secure_desktop_active` | S | 0 | `fn secure_desktop_active(&self) -> bool` |
| `low_level_hooks_timeout_ms` | S(1 回だけ読み `OnceLock`) | 0 | `fn low_level_hooks_timeout_ms(&self) -> Option<u32>`(診断のみ) |
| `observer::focus_observer::read_os_modifiers`(l ブロック、`GetAsyncKeyState` ×3 + 物理表) | S | 0 | `fn os_modifiers(&self) -> ModifierState`。物理 Ctrl/Shift は畳み込み側、Alt/Win は OS 状態 |

## A.3 platform.rs / panic_detect.rs など領域内のその他の呼び出し(参考)

| 呼び出し | 種別 | 備考 |
|---|---|---|
| `Win32Timer::set` / `kill`(`SetTimer`/`KillTimer`) | S(設定は即時)/ 結果は A(`WM_TIMER` が後で届く) | ポート: `fn set_timer(&mut self, id: TimerId, d: Duration)` と `fn kill_timer`。OS が割り当てる ID は戻り値(`TimerArmed`)でなくポートの内部に隠せる(論理 ID だけを core が持つなら)。偽実装は仮想時計で `advance(d)` したときに `TimerFired(id)` を返す |
| `spawn_local(async { get_ime_conversion_mode_fenced_async(50ms) ... })`(platform.rs:658) | **A**(async + `race_with_timeout` 系) | `async fn read_conv_mode(&self, timeout_ms: u32) -> FencedProbeOutcome`。**async trait にするか、「Cmd を出し Event で結果を返す」にするかが論点**(§A.4) |
| `spawn_local(async { set_ime_open_cross_process_async(open) })`(platform.rs:1005) | A(fire-and-forget、結果は捨てる) | Cmd のみ。結果は後続の `WM_ASYNC_IME_APPLY_COMPLETE` で Event として返る(lib.rs:340) |

## A.4 評価

**1. タイムアウトを値(`Result<T, Timeout>`)で表せるか: 表せる。ただし今の型は表していない。**
- `run_with_timeout` は `Option<T>` で、`None` が 3 つの原因(超過 / プール満杯 / ワーカー異常終了)を区別しない。原因はログ(`tracing`)にだけ出る。
- `get_gui_thread_info_with_timeout` は超過時にフォールバック値(`GetForegroundWindow`、`thread_id=0`)を返し、**超過したことが戻り値から消える**。呼び出し側(ime.rs:41、tray.rs:588、ime_diagnostic.rs:156)は `thread_id == 0` からしか推測できない。
- 値にする方針: ポートごとに `Result<T, WaitError>`、`WaitError::{TimedOut, PoolFull, WorkerFailed}`。または `T` に `timed_out: bool` を載せる(フォールバック値を使い続けたい呼び出し元向け)。

**2. Linux の偽実装でタイムアウトを注入できるか: 値で表せば可能、今のままでは不可能。**
- 今の `run_with_timeout` は実スレッドと実時間(`recv_timeout`)を使うので、Linux でテストするとその間実際に待つか、実スレッドが孤児になる。決定的に再現できない。
- 偽実装(`FakeWin32`)の側で「n 回目の `gui_thread_info` は `TimedOut` を返す」「プール満杯を返す」を設定できれば、呼び出し側の分岐(ime.rs の `focused_hwnd.is_none()` 経路、150ms と 200ms の違い、30ms タイムアウトの tuning.rs 参照箇所)を実時間なしで試験できる。**時間を引数に取らせて `Duration` は偽実装が無視する**形にする。
- **実スレッドの孤児管理(`LeakedThreadPool`、GC、満杯判定)自体のテストは別問題**: これは `win32-async` の内部ロジックで、`run_with_timeout_in` の単体テストとして既存。ポートの外側。

**3. 同期 / 非同期で対応が違うか: 違う。次の 3 通りに分ける。**
- (S): そのままの同期メソッド。偽実装は即値を返す。この領域の大半(`send_input`、`post`、`Clock`、`foreground_scope`、`os_idle_ms` ほか)。
- (B): 同期メソッドのまま、引数に `timeout: Duration`、戻り値に `Result<T, WaitError>`。**呼び出しスレッドを最大 `timeout` 止めるという契約**を doc に書く。呼び出し元が 150ms や 500ms 止まることを core が把握できる(今は ime.rs、tray.rs、`HookGuard::drop` がそれぞれ暗黙に止めている)。偽実装は仮想時計を進めず値を返すだけ。
- (A): 2 通りの選択肢がある。(a) `async fn`(trait で `async fn` を使い、偽実装は即 `ready`)。現状の `spawn_local` + `with_app` 再取得 + 世代照合と同じ形で、await をまたぐ失効(platform.rs:604-695 の `ime_mode_focus_gen`)を core が自分で書く必要がある。(b) 「Cmd を出し、結果は Event として後で返る」(`WM_ASYNC_IME_APPLY_COMPLETE` が既にこの形)。**(b) のほうが失効判定(世代トークンを Cmd と Event の両方に載せる)を core の純粋関数にでき、Linux でテストしやすい**。フォーカス・IMC 読み・IMM 書き込みなど await を含む経路は (b) に統一する案を推す。(a) は `race_with_timeout` を使う箇所(`ime::get_ime_conversion_mode_fenced_async` など)で、領域 A1 の外。

**4. 注意(この追補で見つかった点)**
- `get_gui_thread_info_with_timeout` は `unsafe fn` だが、本体は `run_with_timeout` のクロージャ内にだけ unsafe があり、関数自体の unsafe に実質的な意味は無い(呼び出し元 ime_diagnostic.rs:153 などが SAFETY コメントで受けている)。ポート化で `unsafe` を外せる。
- `HookGuard::drop` は `WM_TIMER` ハンドラの中(メインスレッド)で最大 500ms 止まる(hook.rs:1103-1130 の doc)。(B) の「呼び出しスレッドを止める時間」がフック再インストール経路にも存在することを、ポート化したときに見落とさないこと。
- `foreground_window_is_elevated` は時間制限が付いていない(未確認: `OpenProcessToken` が長く詰まる事例は見つけていない)。メインスレッド(`runtime/mod.rs` の watchdog 経路)から呼ばれるので、(B) に格上げするかは実測してから。
- `install_hook` のスピン待ち(`hook_tid_poll`)は上限が無い。フックスレッドが `SetWindowsHookExW` の中で戻らないと、呼び出し元(起動時とフック再インストール)が止まる。**未確認**。
