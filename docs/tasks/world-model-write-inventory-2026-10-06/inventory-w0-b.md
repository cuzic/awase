> **訂正(2026-10-06)**: 本レポートの「`state_dependent_key_warning_dialog` は宣言と初期化だけで一度も使われていない死んだフィールド」は誤検出。`runtime/mod.rs:1609-1611` の `select(...)` で使われている(W-b の実装担当が確認)。

# W0-b 棚卸し: PlatformState の hub 外 / Runtime / Focus / Output / Engine を保持・更新する経路

対象: `origin/develop` @ `ee54b2de`(行番号はこの版の `crates/awase-windows/src/` 基準。`git archive` を scratchpad に展開して読んだ。読み取りのみ)。
担当外(w0-a): `ImeModel`/`ImeBelief`/`ObservationStore`/`IntentStore` の中身と `reduce` の各アームの詳細。ここでは「触れるだけ」。
分類記号は w0-brief どおり(B/I/O/G/C/K/X)に **S=設定(config 由来、起動時と `apply_config_update` で書く)** を足した。書き込み経路は R/M/G/H/Mixed。
- **R\***: `reduce` ではないが「メッセージ → 状態機械 → `Decision`(命令)」の形で入る経路(Engine の `on_input`/`on_timeout`/`on_command`)。Redux 的ではあるが `ImeEvent` ではない。
- **A**: `await` をまたいで `with_app` で書き戻す経路(世代・フェンスで失効判定)。

---

## 0. 要点(先に結論)

1. **Runtime は 40 フィールド**(`runtime/mod.rs:317-455`)。うち reduce(`ImeEvent`)を通って書かれるものは 0。Runtime 自身のフィールドは全てメソッド直接書き込み(M)か Engine の入口(R\*)。`ImeEvent` で動くのは `platform_state.ime`(hub)の中だけ。
2. **hub の外の状態(`FocusStore` 8 + `GateStore` 6 + `KeymapStore` 2 = 16 フィールド)は全て `pub` フィールドで、`ImeEvent` を一切通らない**。`self.platform_state.{focus,gate,keymap}` への参照は本番コードで 84 行、runtime/ の各ファイルから直接書かれる。
3. **「フォーカスが変わった」という 1 つの事実に対し、別々の 4〜5 個の入口が別々の状態を書き換える**(下の §6)。リセット対象は 9 種の型(hub / FocusStore / GateStore / KeymapStore / Output / WindowsPlatform / Runtime / FocusTracker / 全体の static)にまたがり、**書く関数は約 14 個**(§6.2 の表)。ADR-229 C′ 案(「リセット対象を 1 つの struct に集める」)の根拠は十分にある。ただし「リセットされない」ものにも設計上の意味があるものがある(§6.3)。
4. **失効判定(await をまたぐ書き戻しの棄却)に使う世代・フェンスが 8 種類、独立のカウンタとして存在する**(§8)。さらに「フォーカス hwnd」のコピーが 4 か所、「focus epoch」のコピーが 2 か所ある(§8.2)。
5. **Engine は既に Redux 的**(`on_input`/`on_timeout`/`on_command` → `Decision`)で、エンジン内部に壁時計の直接読み取りが 0(grep で確認)。ただし `InputContext` を組む `build_ctx()`(14 か所から呼ばれる)が毎回 OS を 4 つ読む。journal の `KeyInput` は `RawKeyEvent` の全体と `InputContext` を持たないので、エンジンの再生は**できない**。
6. `Runtime.state_dependent_key_warning_dialog` は**宣言と初期化の 2 か所にしか現れず、一度も読み書きされない**(死んだフィールド。`runtime/mod.rs:345`, `:1470`。ADR-217 の掃除対象の候補)。

---

## 1. `Runtime` の全フィールド(40 個、`runtime/mod.rs:317-455`)

読み手の数は `grep` で `self.<field>`/`app.<field>`(本番)の出現行を数えた概数。「journal」は、その書き込みを再生できる記録があるか。

| # | フィールド | 分類 | 書き手(本番) | 経路 | 読み手 | journal | スライス案 |
|---|---|---|---|---|---|---|---|
| 1 | `engine: Engine` | B(エンジン内部。§5) | `on_input`: `key_pipeline.rs:204`, `mod.rs:1428` / `on_timeout`: `message_handlers.rs:717,1842` / `on_command` 11 か所(§5.2) / 設定 setter(§5.3) | R\*+M | 多数 | 一部(§5.4) | engine |
| 2 | `executor: DecisionExecutor` | X(§4.6) | `execute_from_hook/loop`・`drain_deferred`・`enqueue_reinject`・`on_output_guard_timer` | M | 少 | No | shell |
| 3 | `platform: WindowsPlatform` | 混在(§3) | 多数 | M | 多数 | 一部 | 下記 |
| 4 | `layouts: Vec<LayoutEntry>` | S | `reload_layouts` `runtime/mod.rs:1292` | M | 3 | No(設定は再読込で再現) | config |
| 5 | `focus_tracker: runtime::FocusTracker`(sync キー 3 リスト。focus/tracker.rs の同名構造体とは別物) | S | `apply_config_update` `mod.rs:2257-2259` | M | `enrich_ime_relevance` | No | config |
| 6 | `lang_check: LangCheck`(4 カウンタ) | K | `lang_check_on_keydown` `runtime/lang_check.rs:77` | M(`&mut`) | 診断のみ | ログのみ | diag |
| 7 | `platform_state: PlatformState` | 混在(§2) | 多数 | Mixed | 多数 | 一部 | 下記 |
| 8 | `all_keymaps: KeymapTable` | S | `apply_config_update` `mod.rs:2250` | M | `recompute_active_keymaps` | No | config |
| 9 | `post_bypass_rules: Vec<PostBypassEntry>` | S | `apply_config_update` `mod.rs:2255`、起動時 | M | `mod.rs:276`、`message_handlers.rs` | No | config |
| 10 | `ime_coordinator: ImeCoordinator`(2 フィールド) | G/X(保留キー) | `pending_ime_off_rescue`: `set_/take_ime_off_rescue_pending` `mod.rs:879,886`。`deferred_engine_timers`: push `message_handlers.rs:695`、take `:1831` | M | 少 | No | shell |
| 11 | `active_actuation: Option<Actuation>` | G(試行回数・give-up 時刻) | 生成/破棄 `ime_actuation.rs:73,98`、`attempts`/`gave_up_at` 更新 `ime_refresh.rs:836,1023` | M | `ime_refresh.rs` | `ImeActuation`/`ActuationDecision` が結果を記録(試行そのものは No) | guard |
| 12 | `key_effect_keymap: KeymapCache`(GJI の `config1.db` の導出キャッシュ) | C | `get_gji(now_ms)`(読み込み時に `&mut`)`mod.rs:850`, `key_pipeline.rs:1748` | M(read-through) | 2 | No | cache |
| 13 | `last_ime_read_ok: bool` | O(直前の OS 読みの成否) | `ime_refresh.rs:225` | M | 通過マークの読み直し間隔 | No | observation |
| 14 | `key_effect_keymap_native: KeymapCache` | C | `get_native` `mod.rs:851`, `key_pipeline.rs:1750` | M | 2 | No | cache |
| 15 | `state_dependent_key_warning: WarningTracker`(4 フィールド) | K/G(警告の重複抑止) | `detect_gji`/`detect_msime` `mod.rs:1578,1592` | M | 同左 | No | diag |
| 16 | `state_dependent_key_warning_dialog` | **死んだフィールド** | なし(宣言 `:345`・初期化 `:1470` のみ) | - | 0 | - | 削除候補 |
| 17 | `warn_state_dependent_mode_keys: bool` | S | `mod.rs:1521` | M | 1 | No | config |
| 18 | `passthrough_thumb_mode_keys: Vec<VkCode>` | S | `mod.rs:1535`、起動時 `bootstrap.rs:781`、reload `mod.rs:2208` | M | `mod.rs:1584,1598` | No | config |
| 19 | `key_effect_runtime_table: RuntimeTableCache`(学習表) | C | `get_for_keymap(now_ms, keymap)` `key_pipeline.rs:1761`, `mod.rs:860` | M(read-through) | 3 | 学習表は別ファイル | cache |
| 20 | `use_learned_keymap_table: bool` | S | `mod.rs:1509` | M | 3 | No | config |
| 21 | `predict_henkan_open_in_unreadable_windows: bool` | S | `mod.rs:1513` | M | 1 | No | config |
| 22 | `key_role_latch: Option<(ScanCode, Option<ShadowImeAction>)>` | G(Down=Allow・Up=Suppress の非対称防止、BUG-131/132 系) | `enrich_key_role` `mod.rs:717`、`settle_fkey_role_latch` `mod.rs:762` | M | 同左 | **No**(ラッチ判定は journal に出ない) | guard |
| 23 | `muhenkan_dedicated_fn_key_vk: Option<VkCode>` | S | `mod.rs:1656` | M | `recompute_active_keymaps` | No | config |
| 24 | `space_is_thumb_key: bool` | S | `mod.rs:1697` | M | 1 | No | config |
| 25 | `msime_key_assignment_warned: Option<u8>` | K/G(警告のデデュープ) | `mod.rs:1673,1679` | M | 同左 | No | diag |
| 26 | `keyboard_model: KeyboardModel` | S | `mod.rs:1505` | M | 診断 | No | config |
| 27 | `update_check_enabled: bool`(`pub(crate)`) | S | `mod.rs:1517` | M | `message_handlers.rs:1193`(`with_app_ref`) | No | config |
| 28 | `kana_lock_hysteresis: KanaLockHysteresis`(ルート crate の型、3 フィールド) | K/G(警告の通知ヒステリシス) | 観測 `key_pipeline.rs:2525`、リセット `ime_refresh.rs:121`・`mod.rs:1012` | M | 2 | No | diag |
| 29 | `watchdog_kana_edge: Option<KanaLockReading>` | K | `message_handlers.rs:765` | M | ログの重複抑止 | No | diag |
| 30 | `drift_giveup_notified_this_focus: bool` | G(1 フォーカス 1 回) | 立てる `ime_refresh.rs:1057`、**フォーカス変更で下ろす** `ime_refresh.rs:122`・`mod.rs:1013` | M | 1 | No | guard(focus-scoped) |
| 31 | `drift_giveup_started_at: Option<Instant>` | K(診断区間の開始) | 立てる `ime_refresh.rs:1058`、**`take()`** `focus_tracking.rs:616`、リセット `ime_refresh.rs:123`・`mod.rs:1014` | M | 1 | `DriftGiveUpIntervalEnded` が終端のみ記録 | diag(focus-scoped) |
| 32 | `hook_guard: Option<HookGuard>` | X(OS ハンドル) | `set_hook_guard` `mod.rs:1793`、`drop_hook_guard` `:1799`、自己修復 `:2040,2043` | M | 0 | No | shell |
| 33 | `hook_self_heal_enabled: bool` | S | `mod.rs:1805` | M | 1 | No | config |
| 34 | `hook_watchdog_confirmed_attempt_count: u32` | G | `mod.rs:1848`(リセット)・`:2026` | M | 1 | No(`hook_watchdog` は純関数 `state/hook_watchdog.rs::decide` が判断) | guard |
| 35 | `hook_watchdog_next_retry_at_ms: Option<u64>` | G | `mod.rs:1849`・`:2028` | M | 1 | No | guard |
| 36 | `hook_watchdog_reinstall_history_ms: Vec<u64>` | G | `mod.rs:2029,2032` | M | 1 | No | guard |
| 37 | `session_locked: bool` | O(OS セッション状態) | `mod.rs:1811` | M(`WM_WTSSESSION_CHANGE` 由来) | 1 | No | observation |
| 38 | `hook_watchdog_canary_sent_at_ms: Option<u64>` | G | `mod.rs:1940`・`:1960`(`take`) | M | 1 | No | guard |
| 39 | `hook_watchdog_canary_baseline_alive_ms: Option<u64>` | G | `mod.rs:1945` | M | 1 | No | guard |
| 40 | `hook_watchdog_consecutive_alive_ticks: u32` | G | `mod.rs:1826,1839` | M | 1 | No | guard |

**集計(Runtime 40 フィールド)**
- 書き込み経路(40 = 合計): R\*(Engine の `on_*`、+ setter は M)1〔#1〕、M 38〔#2〜#40 のうち #16 を除く。うち #12/#14/#19 は読み込み付きキャッシュ、#3/#7 は §2・§3 に展開〕、書き手なし 1〔#16〕。**R(ImeEvent)は 0**。
- 分類(40 = 合計): S(設定)13〔#4,5,8,9,17,18,20,21,23,24,26,27,33〕、G 10〔#10,11,22,30,34,35,36,38,39,40〕、K/diag 6〔#6,15,25,28,29,31〕、C 3〔#12,14,19〕、O 2〔#13,37〕、X 2〔#2,32〕、B(engine)1〔#1〕、混在 2〔#3,7〕、死 1〔#16〕。
- **フォーカスで触られるのは #11(active_actuation を破棄)・#30・#31・#28(kana_lock_hysteresis)の 4 つ**。#22(`key_role_latch`)は触られない(§6.3)。

### 1.1 `Runtime` の外部入口(状態を書くメッセージの種類)

- `with_app`(排他借用)の本番呼び出し: **74 行**(`with_app`/`with_app_or_repost(_with)`。`lib.rs` の定義内を除く)。`with_app_ref`(読み取りのみ): 6 行。`app/mod.rs` 20、`message_handlers.rs` 13、`key_pipeline.rs` 9、`bootstrap.rs` 8、`open_chain.rs` 7 が上位。
- Win32 メッセージの集約表: `app/mod.rs::dispatch_engine_message`(`WM_TIMER`・`WM_EXECUTE_EFFECTS`・`WM_ASYNC_IME_APPLY_COMPLETE`・`WM_KANA_LOCK_WARNING_CHANGED`・`WM_HOOK_IME_MODE_DIAGNOSTIC`・`WM_PANIC_RESET`・`WM_DUPLICATE_INSTANCE`・`WM_IME_KIND_CHANGED`・`WM_POWERBROADCAST`・`WM_WTSSESSION_CHANGE`・`WM_INPUTLANGCHANGE`・`WM_FOCUS_KIND_UPDATE`・`WM_DUMP_JOURNAL`・`WM_KEY_FROM_HOOK`・`WM_APP`・`WM_RELOAD_CONFIG`・`WM_COMMAND`・`WM_DRAIN_OUTPUT_QUEUE`・`WM_ENGINE_QUIT_REQUEST` の 19 種。`app/mod.rs:440-620`)。
- 物理キーの取り込み口: `app/mod.rs::handle_hook_key_event`(`:624-670`)。**ここで `reduce` の前に 5 つの状態を書く**: `lang_check`(+OS の入力言語を読む)、`panic_detect::RAPID_IME_TIMESTAMPS`(static)、`DUMP_TRIGGER`(static)、`FOCUS_RESYNC.consume_and_close()`(static)、`INPUT_DEFER`(static のキュー)。
- タイマー: `TIMER_*` 定数 10(`lib.rs`)。論理 ID↔OS ID の表は `Win32Timer`(`timer.rs:16`、`to_os`/`to_logical` の 2 つの `HashMap`)。

---

## 2. `PlatformState`(hub 以外)

`PlatformState`(`state/platform_state.rs:1784`)は 4 フィールド: `ime`(hub、w0-a)・`focus: FocusStore`・`gate: GateStore`・`keymap: KeymapStore`。**3 つの Store は全フィールドが `pub`** で、`ImeEvent` を通らない。

### 2.1 `FocusStore`(8 フィールド、`:1629`)

| フィールド | 分類 | 書き手 | 経路 | 読み手 | journal | スライス案 |
|---|---|---|---|---|---|---|
| `app_kind` | B(フォーカス先の種別) | `classify_focus_probe` `focus_tracking.rs:350` | M | 多(`injection_hint` ほか) | `FocusTransition`(`app_kind` を文字列で) | focus |
| `focus_kind` | B | `classify_focus_probe` `focus_tracking.rs:368`、`toggle_app_override` `mod.rs:1330`、`TIMER_POWER_RESUME` `message_handlers.rs:493`(`Undetermined`)、UIA 完了 `WM_FOCUS_KIND_UPDATE`(ハンドラ側) | M | 多 | `FocusTransition` | focus |
| `last_focus_change_ms` | K/O(滞在時間の根拠) | `enter_focus_scope` `focus_tracking.rs:173` | M | キャッシュ保存判定 `:422`、診断、`ime_refresh.rs` 8 か所 | No | focus |
| `last_focus_transition_ms` | K(journal 用) | `record_focus_transition_if_changed` `:59` | M | `:76,:112`(prev_started_ms) | No(`dwell_ms` が間接的に残る) | focus |
| `focus_debounce_ms` | S | `apply_config_update` `mod.rs:2132` | M | `on_window_focus_event` | No | config |
| `ime_poll_interval_ms` | S | `mod.rs:2133` | M | 1 | No | config |
| `focus_epoch: u64` | O/G(失効窓の世代。**フェンスの片方**) | `enter_focus_scope` `focus_tracking.rs:175-176`(`wrapping_add(1)`) | M | `focus_fence()`、`FocusChanged{focus_epoch}` | `ImeEvent::FocusChanged` に値が載る(hub 側) | focus |
| `app_disabled` | B/G(awase 無効アプリか) | `apply_app_disable_transition` `focus_tracking.rs:558` | M | `ime_refresh.rs`、他 | No(`[app-disable]` のログのみ) | focus |

→ 書き込み経路: **M 8 / R 0**。分類: S 2、B 3、K 2、O/G 1。`app_disabled` は同じ事実を `HookState.focus_app_disabled`(atomic、hook スレッドが読む)にも複写する(§8.2)。

### 2.2 `GateStore`(6 フィールド、`:1683`)

| フィールド | 分類 | 書き手 | 経路 | 読み手 | journal | スライス案 |
|---|---|---|---|---|---|---|
| `last_hook_activity_ms` | O(最終キー時刻、typing-idle ガード用) | `message_handlers.rs:153` | M | `ime_refresh.rs:422`・診断・`mod.rs:2098` | No | observation |
| `post_bypass: ScopedOneShot<ForegroundScope, PostBypassArm>` | G(Ctrl+key 直後の 1 回マーク。**`ForegroundScope` が変わると自動失効**) | arm `message_handlers.rs:443`・disarm `:405,:409` | M | `:380,:384` | No | guard |
| `sync_key_gate: SyncKeyGate` | G/X(IME 同期キー直後の保留) | `deactivate` `mod.rs:1372`、`clear` `:2391`、`message_handlers.rs:481` | M | 同左 | No | guard |
| `half_width_alnum: HalfWidthAlnumState` | I+G(左右 Shift 単独タップの半角英数トグルの全状態) | `key_pipeline.rs:1863-2167` の約 12 メソッド(`arm_guard`/`disarm_guard`/`take_guard`/`on_shift_up`/`commit_enter_imc`/`commit_enter_gji`/`set_policy` `mod.rs:1648`) | M | 12 か所 | **No**(Shift-conv 系は journal に出ない) | guard |
| `idle_conv_check_in_flight_since_ms` | G(in-flight 制限、8 秒で自己回復) | 立てる `key_pipeline.rs:500`、**await の後に下ろす** `:552,:559` | **A**(await をまたぐ) | `:489` | No | guard |
| `shadow_key_down_disposition: Vec<(ScanCode, bool)>` | G(物理キー Down の配送結果のラッチ、KeyUp を Down に揃える) | `key_pipeline.rs:291`(`&mut` を `keyup_follows_keydown` へ渡す)、**フォーカス変更で 2 回 `clear`** `focus_tracking.rs:569,:630` | M | 同左 | No | guard(focus-scoped) |

→ 書き込み経路: **M 5 / A 1 / R 0**。分類: G 5、O 1、(I 1 と G が `half_width_alnum` で重なる)。

### 2.3 `KeymapStore`(2 フィールド)

| フィールド | 分類 | 書き手 | 経路 | 読み手 | journal | スライス案 |
|---|---|---|---|---|---|---|
| `active_keymaps` | 派生(`all_keymaps` を現在のプロセス名で絞った結果) | `recompute_active_keymaps` `mod.rs:2272`(←`enter_focus_scope`・`apply_config_update`) | M | `message_handlers.rs:271`、`mod.rs:1660,2286` | No(`[keymap] active rules updated` のログ) | derived |
| `keymap_latch: KeymapLatch` | G(`[[keymap]]` の KeyUp 回収・リピート抑制) | `message_handlers.rs:273`(latch)・`:1057`・`focus_tracking.rs:565`・`mod.rs:2055,2366`(`release_all` 3 回) | M | `:167,:179` | No | guard |

### 2.4 hub の中だが本タスクの担当(通過マーク・外部変化の監視窓・ForceGuardSet)

| 保持者 | 分類 | 書き手 | 経路 | 備考 |
|---|---|---|---|---|
| `ImeStateHub.mode_key_pass_mark: ModeKeyPassLatch<ForegroundScope>`(`platform_state.rs:86`) | G(ADR-187、無変換/変換を IME へ素通しした直後の一回マーク。寿命は純粋な `state/mode_key_pass.rs`) | arm `executor.rs:645`→`arm_mode_key_pass_mark` `:306`、`note_awase_write` `:323`(`applied` 更新時)、`drop_decision` `:392`(`invalidate_intents_if_mode_key_pass_live`/`expire_mode_key_pass_mark`)、`align_after_expired` `:542` | **Mixed**: メソッドがラッチを直接書き、副作用だけ `ImeEvent::ModeKeyPassedThrough` と `intent_store.remove`(直接)で適用 | **8 つのメソッドが OS(`crate::win32::foreground_scope()`)を中で読む**(`:307,:317,:337,:347,:356,:378,:412,:421`)。`_in_scope` 版(`:327,:334,:359,:385,:536`)は ADR-229 F-D3 の核と殻の分割の実例 |
| `ImeStateHub.external_change_watch: ExternalChangeWatch<ForegroundScope>`(`:91`) | G(ADR-205、外部注入キー直後の監視窓。基準値 `baseline` と直近の読み `last_read`) | arm `:416`(←`key_pipeline.rs:1618`、`ime_refresh.rs:303`)、`observe`/`record_read` `:457,:463`(`follow_external_change`、←`ime_refresh.rs:272`) | Mixed(同上。追随時に `write_observer_poll`→`ObserverReported`、`intent_store.remove`、`ModeKeyPassedThrough`) | scope は `foreground_scope()` を `arm`/`remaining`/`follow` の 3 メソッドが中で読む(`:418,:432,:455`)。**`_in_scope` 版は無い**(ModeKeyPass とは非対称) |
| `ImeStateHub.last_external_change_ms`(`:96`) | O(柵) | `follow_external_change` `:472` | M | `ime_refresh.rs:~330` の `observe_gji_after_focus` の第 1 引数 |
| `ImeModel.force_guards: ForceGuardSet`(`ime_model.rs:220`) | G(`PanicReset` のみ。**`ProfilePolicy` は本番の `add` が 0**) | 本番 5 か所: `reset_detect_state` `platform_state.rs:1117`(`clear`)、`release_panic_reset_guard_on_positive_evidence` `:1132`(`remove`)、`apply_panic_reset` `:1159-1160`(`clear`+`add`)、`apply_ime_update` `:1222`(`remove`)、**`reduce_focus_changed` `ime_model.rs:1028`(`clear_for_focus_change`。reduce 内はこの 1 つだけ)** | **M 4 + R 1** | 追加は `apply_panic_reset` 1 か所のみ。`PanicReset` ガードの追加は `ImeEvent::PanicReset` の reduce(`ime_model.rs:764`)の**外**(ガードの `generation` に `event_log.next_seq()` を使うので、reduce の前に積む) |

---

## 3. `WindowsPlatform`(13 フィールド、`platform.rs:24`)と focus 周辺

| フィールド | 分類 | 書き手 | 経路 | journal | スライス案 |
|---|---|---|---|---|---|
| `output: Output` | §4 | | | | |
| `tray: SystemTray`(7 フィールド) | X/K | `set_*`(トレイ表示)、`set_kana_lock_warned` | M | No | shell |
| `timer: Win32Timer` | X | `set`/`kill`/`resolve` | M(+OS) | `TimerFired` は発火側のみ | shell |
| `focus: FocusTracker`(`focus/tracker.rs`、8 フィールド、§3.1) | | | | | |
| `stamper: JournalStamper` | X | clone を `win32` へ登録 `platform.rs:74` | G | - | shell |
| `pending_journal_entries: Vec<JournalEnvelope>` | X(journal への橋) | `push_journal_entry`、`drain_journal_entries` | M | これ自体が journal の前段 | shell |
| `active_tsf_probe_started_ms: Option<(u64,u64)>` | K | TSF probe 開始/終了 | M | `TsfProbeStarted/Completed` | warmup |
| `probe_tick_index: u32` | K | `advance_tsf_probe` | M | `GjiFsmTransition`(`TsfProbeTick(#n)`) | warmup |
| `suppressed_probe_ticks: u32` | K | 同上 | M | 同上 | warmup |
| `suppressed_literal_confirms: u16` | K | リテラル検出 | M | `LiteralDetect` | warmup |
| `pending_literal_vk: Option<PendingLiteralVk>` | O/G(リテラル検出の途中状態) | `consume_literal_detect_trace` | M | `LiteralDetect` | warmup |
| `giveup_tracker: GiveUpTracker`・`pending_giveup: Option<GiveUpEvidence>` | G(ADR-227、give-up の判定は純粋、証拠を runtime へ渡す 1 件) | `advance_tsf_probe`、**取り出し `take_giveup_evidence`**(`message_handlers.rs:514`)→`ir_follow_after_literal_giveup` | M | `GiveUpFollow`(結果のみ) | warmup |

### 3.1 `focus::FocusTracker`(`focus/tracker.rs:22`、8 フィールド)

| フィールド | 分類 | 書き手 | 経路 | journal | スライス案 |
|---|---|---|---|---|---|
| `current: CurrentFocus`(6 フィールド: `hwnd`/`root_hwnd`/`pid`/`class_name`/`app_profile`/`process_name`) | B(フォーカス先の同一性) | `update_with_process_name` ←`advance_focus_tracking`(`runtime/focus_tracking.rs`)、**`ir_stage_focus` が 500ms ごとの tick でも同じウィンドウに対して毎回呼ぶ**(BUG-111 の doc) | M | `FocusTransition`(`hwnd`/`pid`/`class`/`process`。`app_profile` は `profile` 文字列) | focus |
| `cache: FocusCache`(`(pid, class) → FocusKind`) | C | `cache_insert` `focus_tracking.rs:372`(`Automatic`)・`mod.rs:1338`(`UserOverride`)、`cache_reset` `mod.rs:2160` | M | No | cache |
| `overrides: ForceOverrides`(config の `app_overrides` の写し。`INPUT_RELAY_APPS` static にも複製、`classifier.rs:125`) | S | `reset_overrides` `mod.rs:2157` | M(+G: static へ複製) | No | config |
| `uia_sender: Option<Sender<SendableHwnd>>` | X | `set_uia_sender` `bootstrap.rs:1319` | M | - | shell |
| `imm_learning: ImmCapabilityStore`(`cache.toml` の `[imm_capability]`、`pending_unavailable`) | C(学習結果) | `learn`(←`platform.rs:1337`←`runtime/mod.rs:632`)、`record_null_probe`・`clear_pending_unavailable`(`focus/imm_learning.rs:80,82`)、`clear`(`message_handlers.rs:1290`) | M(**書き込みごとに `cache.toml` へファイル保存**、`classifier.rs:391` の `save`) | No(`[imm-learning]` のログのみ) | cache |
| `injection_mode_store: InjectionModeStore`(`[injection_mode]`) | C | `learn_tsf` `platform.rs:448`(`advance_tsf_probe` の `learned_tsf`) | M(+ファイル保存 `classifier.rs:526`) | No | cache |
| `hwnd_ime_cache: HwndImeCache`(`(pid, class) → HwndImeSnapshot`、`MIN_FOCUS_DURATION_MS` 未満は保存しない) | C | `save_ime_state` `focus_tracking.rs:445`(**フォーカスを離れる時**)、復元は `restore_ime_state`(読み)→hub の `apply_hwnd_cache_restore`→`ImeEvent::HwndCacheRestored` | M(保存)+R(復元) | `ImeEvent::HwndCacheRestored{target}`(復元側のみ。保存側 No) | cache |
| `seen_threads: SeenThreads` | O/K(新規スレッド判定の記録) | `probe_focus_thread` `focus_tracking.rs:594`(`&mut` を渡す) | M | `[thread-scope]` のログのみ | observation |

→ 書き込み経路: **M 8 / R 1(HwndCacheRestored のみ。復元側)**。分類: C 4、B 1、S 1、X 1、O 1。

---

## 4. `Output` 系(出力・warmup・defer)

### 4.1 `Output`(15 フィールド、`output/mod.rs:61`)

「`&self`」で書くもの(内部可変)が **11 フィールド**あり、借用検査が書き込みを保護していない。

| フィールド | 分類 | 内部可変 | 書き手 | journal | スライス案 |
|---|---|---|---|---|---|
| `injector: KeyInjector`(`kana_table`/`symbol_to_vk`) | X | - | 構築時のみ | - | shell |
| `composition: CompositionState`(`WarmEpoch` 3 Cell + `ColdContext` 4 Cell) | B(warm/cold の信念) | **Cell×7** | `mark_composition_cold`(`ColdReason` 付き)、`on_focus_changed`、`update_last_send_ms`、`set_last_unicode_transmit_ms`、`record_cold`/`increment_consecutive_count`/`reset_consecutive_count` | `GjiFsmTransition`/`LiteralDetect` が一部 | warmup |
| `warmup_coord: TsfWarmupCoordinator`(9 フィールド、§4.2) | G/B | RefCell×5+Cell×4 | §4.2 | 一部 | warmup |
| `tsf_gate: TsfGate`(`HoldingGate<TsfGateMachine, RawKeyEvent>`、保留キーの列を持つ) | G | なし(`&mut self`) | `on_focus_change_tsf`(PendingWarmup 中は再入しない)、`confirm_tsf`、`bypass_tsf`、`on_tsf_warmup_timeout`、`try_hold_key` | No | guard(focus-scoped) |
| `injection_mode: InjectionMode` | B(派生。focus と app_kind から**push で複写**) | なし | `update_injection_mode` ←`apply_focus_probe_result`(`focus_tracking.rs:97`)・`establish_initial_focus_scope`(`:153`)・**`on_window_focus_event`(`mod.rs:1733`)の 3 か所** | No | derived |
| `conv_mode: ConvModeMgr`(`last: Cell<Option<ConvModeRecord>>`) | O | Cell | `observe`(`kp_stage_idle_conv_check` 経由、フェンスで単調ガード) | `ConvClassifyCall` | observation |
| `ime_mode_fsm: RefCell<ImeModeFsm>`(state/confirmed/`last_vk_send_ms`) | B | RefCell | `on_focus_changed`・`update_ime_mode_*`・VK 送信時 | No | warmup |
| `ime_mode_focus_gen: Cell<u32>` | G(**フォーカス世代の 2 つ目**。await をまたぐ読み取りの失効に使う) | Cell | `on_ime_mode_focus_changed`(`+1`)←`gji_on_focus_change` | No | guard |
| `ms_ime_gate_give_up: Cell<bool>` | G(BUG-13 の give-up ラッチ) | Cell | `probe_io.rs:188`(立てる)、`output/mod.rs:367`・`platform.rs:1160`・`conv_actuation.rs:158`(下ろす) | No | guard |
| `confirm_gate_deadline_override_ms: Cell<u64>` | G(shift-conv-guard 中の期限延長) | Cell | `kp_shift_conv_guard_*`、**フォーカス変更で `0`** `output/mod.rs:~373` | No | guard |
| `shift_conv_guard_gen: Cell<u32>` | G(上の所有権世代) | Cell | `bump_shift_conv_guard_gen`(新 hold・早期 return・**フォーカス変更**・`SetOpen(true)` の 4 か所) | No | guard |
| `observe_unicode_literal: AtomicBool` | G | Atomic | `request_unicode_observation`、`swap(false)` | No | warmup |
| `conv_mutation_allowed: Cell<bool>` | S | Cell | `set_conv_mutation_allowed` ←`set_conv_mode_authority` | No | config |
| `runtime_outbox: RefCell<RuntimeOutbox>` | X(Output→Runtime の遅延リクエスト) | RefCell | 積む(vk_send 等)・`take_pending_requests` | No | shell |
| `pending_drain_before_send_flush: Cell<usize>` | K | Cell | `drain_pending_deferred_before_send_*`・`take_pending_drain_before_send_flush` | `DeferredRecoveryFlush`(変換後) | warmup |

### 4.2 `TsfWarmupCoordinator`(9 フィールド、`output/tsf_warmup_coord.rs:36`)

| フィールド | 分類 | 書き手 | 備考 |
|---|---|---|---|
| `tsf_warmup: RefCell<Box<dyn ImeWarmupStrategy>>`(GjiFsm / MsImeStrategy) | B(GJI の warm/cold 状態機械) | `set_active_ime_kind`(戦略の**取り替え**。取り替えると `GjiFsm::new()` で状態が初期化される `:107-113`)、`gji_on_event`(`GjiEvent`) | `GjiFsmTransition` に遷移が記録される(R 相当。`GjiFsm` は `TimedStateMachine`) |
| `pending_tsf: RefCell<Option<Box<dyn TickableFsm>>>`(進行中の probe。`ProbeCoroState` を内包) | G/B | `install_pending_tsf` `:216`・`take_pending_tsf` `:237`・`:260,:267` | probe は `TIMER_TSF_PROBE` ごとに tick。**フォーカス変更では明示的に捨てない**(§6.3、`probe_io.rs:164,242` が `ime_mode_focus_gen` で個別に失効) |
| `current_gji_probe_id: Cell<Option<ProbeId>>` | G | `GjiAction::StartProbe` 受信時 | 世代(§8) |
| `gji_probe_guard: RefCell<Option<OutputActiveGuard>>` | X(RAII。`OUTPUT_GATE` を立てる) | probe 開始/終了 | RAII の `Drop` が `OUTPUT_GATE` を書く(G) |
| `stage: Cell<StageRecord>` | K | probe 段ごと | No |
| `pending_gji_composition_reset: Cell<bool>` | G | `ProbeIo::mark_cold_raw_tsf` が立て、`advance_tsf_probe` が消費 | 橋渡しフラグ |
| `pending_gji_key_responses: RefCell<Vec<GjiResponse>>` | X(橋渡しバッファ) | `send_romaji_*` が積む・取り出し | |
| `pending_deferred: RefCell<Vec<DeferredVk>>` | G(probe 進行中に届いた後続 VK の**唯一のキュー**、ADR-156) | defer 側 `defer_respecting_gate`(`output/vk_send.rs:47`)、drain 側 `flush_pending_deferred_vks`(`:83-90`、`drain_pending_deferred_before_send_if_queue_only`)。**2 窓口** | `DeferredRecoveryFlush` |
| `next_deferred_order_token: Cell<u64>` | K | defer のたびに+1 | |

`ProbeCoroState`(`tsf/warmup/probe_coro_state.rs:8`、4 フィールド: `coro: StepCoro`・`pending_transmit_done`・`pending_vk_sent`・`cold_seq: Generation`)は `pending_tsf` の内側にあり、Cmd(`ProbeAction`)を yield・Event(`ProbeTickInput`)を受ける**標準形の核**(ADR-229 F-D4)。この領域で唯一「イベント → 状態機械 → 命令」が閉じている。

### 4.3 キューとゲートの global

| static | フィールド | 書き手 | スレッド |
|---|---|---|---|
| `INPUT_DEFER`(`input_defer.rs:23`) | `queue: Mutex<VecDeque<RawKeyEvent>>`(上限 1024)、`overflow_count` | `defer_during_output`(`app/mod.rs:654`)、`replay_later`(`app/mod.rs:662,666`・`message_handlers.rs:526,1810`・`mod.rs:1235`)、`take_all` | **メインスレッド**(`handle_hook_key_event` は `with_app` と同じスレッドで走る。フックスレッドは `HOOK_KEYS` へ積むだけ) |
| `OUTPUT_GATE`(`tsf/probe_bridge.rs:65`) | `active: AtomicBool`・`depth: AtomicU32`・`last_vk_output_ms: AtomicU64` | `OutputActiveGuard`(RAII)、VK 出力時 | メイン+ワーカー |
| `FOCUS_RESYNC`(`focus_resync.rs:127`) | `armed`・`gate_active`・`generation`・`armed_at_ms`(4 atomic) | `arm`(**`on_focus_process_changed` の末尾** `focus_tracking.rs:892`)、`consume_and_close`(`app/mod.rs:647`)、`open_if_current` | メイン |

### 4.4 `DecisionExecutor`(5 フィールド、`runtime/executor.rs:105`)

`queue: VecDeque<Effect>`(Engine の命令 FIFO)・`passthrough_queue: PassthroughQueue`・`guard_held: Option<RawKeyEvent>`(`OUTPUT_GUARD` で park した 1 件。不変条件 `guard_held.is_some() ⟺ TIMER_OUTPUT_GUARD 登録済み`)・`applied_snapshot: AppliedImeState`(**hub の `ImeModel.applied` の複写**。decision サイクル開始時に pre-fetch、バッチ内で `update_intra_batch_applied` が更新 `:901`)・`belief_input_mode: InputModeState`(同じく複写)。全て M。journal: `ActuationDecision`/`SentInput` が結果を記録(キュー自体は No)。

---

## 5. Engine(ルートの `awase` クレート)

### 5.1 保持する状態

- `Engine`(`src/engine/engine.rs:52`、7 フィールド): `adapter: FsmAdapter`(`NicolaFsm` を包む)・`special_keys`・`ime_toggle_auto`・`lifecycle: KeyLifecycle`・`prev_activation`・`solo_off_notify`・`phase1_held`。
- `NicolaFsm`(`src/engine/nicola_fsm.rs:140-272`): 公開フィールドで約 17(`layout`・`state: EngineState`・`threshold_us`・`enabled`・`ngram_model`・`timing_margin_percent`・`min_overlap_margin_percent`・`confirm_mode`・`speculative_delay_us`・`last_key_timestamp`・`output_history`・`phys: PhysicalKeyState`・`left_thumb_consumed`・`right_thumb_consumed`・…)+ private で約 10(`thumb_shift_faces_enabled`・`solo_counter`・`engine_off_extra_solo_counter`・`engine_off_extra_key_suppressed`・`engine_off_solo_repeat_vk`・`engine_off_requested`・`space_thumb_vk`・`text_key_space`・`muhenkan_vk`・`mode_key_muhenkan` ほか)。**数えきれていない部分あり(未確認)**。

### 5.2 状態遷移の入口(Redux 的な部分)

| 入口 | メッセージ | 本番の呼び出し箇所 | 出力 |
|---|---|---|---|
| `Engine::on_input(event, &ctx)` | `RawKeyEvent` | `runtime/key_pipeline.rs:204`、`runtime/mod.rs:1428`(**2 か所**) | `Decision`(Effects、`Consume`/`PassThrough`) |
| `Engine::on_timeout(timer_id, &ctx)` | OS タイマー ID | `message_handlers.rs:717`、`:1842`(**2 か所**) | `Decision` |
| `Engine::on_command(cmd, &ctx)` | `EngineCommand`(`ToggleEngine`・`InvalidateContext`・`SwapLayout`・`ReloadKeys`・`UpdateFsmParams`・`SetNgramModel`・`RefreshState`・`FocusChanged`・`ForceEngineOn` の 9 種) | `ime_refresh.rs:410,1116`、`message_handlers.rs:124`、`mod.rs:1004,1023,1032,1310,1415,2122,2149,2300`(**11 か所**) | `Decision` |

→ **入口 15 か所、メッセージ型 3(`RawKeyEvent`/タイマー ID/`EngineCommand`)**。`Decision` は命令の列(Cmd 相当)で、`Runtime::execute_decision`(`mod.rs:893`)が `DecisionExecutor` に渡して実行する。**時刻はイベントに載っている**(`RawKeyEvent.timestamp`)。`src/engine/*.rs`(テスト以外)に `Instant::now`/`SystemTime`/`current_tick`/`GetTickCount` は 0(grep で確認)。

### 5.3 Decision を返さない書き込み(`set_*`、状態の取り込み口が別)

`Engine` の `&mut` setter は本番で **19 呼び出し**(`bootstrap.rs:1185-1259` の起動時 9 回 + `mod.rs` の `apply_config_update`/`set_thumb_role_open_actions`(`:824,826,2121`)/`set_muhenkan_solo_tap_dedicated_fn_key`(`:1655`)/`set_space_thumb_config`(`:2179`)/`set_thumb_key_solo_tap_config`(`:2192`)/`set_enter_thumb_config`(`:2217`))。**`set_thumb_role_open_actions` だけは打鍵ごとに(`enrich_thumb_role`、無変換/変換の非リピート KeyDown)呼ばれる**。それ以外は設定の反映。つまり**エンジンへの入口は `on_*`(R\*)と `set_*`(M)の 2 種類**があり、後者は `EngineCommand` に載っていない。

### 5.4 Decision を経由しない出力(副チャネル)

- `take_solo_off_notification()`(`message_handlers.rs:81`): `solo_off_notify` の 1 ショットフラグを `mem::take`。`Decision` に載っていない。
- `take_engine_off_requested()`(`on_timeout` 内)・`take_ime_open_requested()`(`apply_ime_open_request`): エンジン内部の 1 ショットチャネル。ADR-229 F-D3 の「副チャネルは戻り値に載せる」の対象。
- `retro_eval_stats()`(`bug_report.rs` が読む累積カウンタ)。

### 5.5 journal から再現できるか

**一部のみ(エンジンの再生はできない)**。`JournalEntry::KeyInput` は `KeyEventSummary`(vk/scan/is_down/injected/timestamp_us/key_class/alt/ctrl/shift)+ `state_before`/`state_after`(文字列ラベル)+ `decision`(種別)だけで、`RawKeyEvent` の `ime_relevance`・`was_down`・`modifier_snapshot` の全体、`InputContext`(`ime_on`/`input_mode`/`composing`/親指の押下時刻)は持たない。`TimerFired` は `timer_id` と状態ラベルのみ。ADR-229 の「所有者の判断 4」が名指す前提(`KeyInput` が生入力を持たない)と同じ事実。

---

## 6. フォーカス遷移でリセットされる状態(C′ 案の根拠)

### 6.1 フォーカスに関する入口は 4〜5 つあり、書く関数は約 14 個

| 入口 | 契機 | 呼び出しの経路 |
|---|---|---|
| **(a) WinEvent** `EVENT_OBJECT_FOCUS` | OS(同一 hwnd の連続は `LAST_FOCUS_HWND` static で捨てる) | `bootstrap.rs::win_event_proc:842` → `with_app` → `Runtime::on_window_focus_event`(`mod.rs:1713`) |
| **(b) デバウンス後のポーリング** | `TIMER_IME_REFRESH`(約 50ms 後、定常は 500ms ごとに毎回 `ir_stage_focus` が走る) | `spawn_ime_refresh` → `ir_execute` → **`ir_stage_focus`**(`ime_refresh.rs:~94`)→ `apply_focus_probe_result`(`focus_tracking.rs:77`)→ プロセスが変わったときだけ `on_focus_process_changed`(`:587`)。**起動時だけ別入口** `establish_initial_focus_scope`(`:109`) |
| **(c) 同一プロセス内の hwnd 変化** | (b) の中 | `notify_focus_hwnd_updated_if_needed`(`focus_tracking.rs:512`)→ `ImeEvent::FocusHwndUpdated`(hub の fence の hwnd だけ追従) |
| **(d) モーダルポンプ出入り** | `take_needs_engine_resync()`(`engine_window.rs`) | `begin_key_batch`(`message_handlers.rs:119`)→ `EngineCommand::FocusChanged` を Engine だけに送る |
| (e) 電源復帰 | `TIMER_POWER_RESUME` | `message_handlers.rs:488-494` → `focus_kind=Undetermined` + `invalidate_engine_context` |

### 6.2 `on_focus_process_changed`(プロセスが変わったとき)が触る状態の全一覧

実行順。フィールドに加え、それを書く関数名・経路・「どの型か」を示す。**型の数: hub 内部 / FocusStore / GateStore / KeymapStore / Output / WindowsPlatform / Runtime / FocusTracker / 全体の static の 9 種**。

| # | 順 | 状態 | どの型 | 書く関数(`focus_tracking.rs` の行 / 先) | 経路 |
|---|---|---|---|---|---|
| 1 | `advance_focus_tracking`(`on_focus_process_changed` の**前**、`apply_focus_probe_result` 内) | `hwnd_ime_cache`(離れる側の ime_on・input_mode・`from_explicit_off_intent` を保存。滞在 `MIN_FOCUS_DURATION_MS` 未満は保存しない) | FocusTracker | `save_ime_state` `:445` | M |
| 2 | 同上 | `FocusTracker.current`(hwnd/pid/class/process/`app_profile`) | FocusTracker | `update_focus_info_with_process_name` | M |
| 3 | 同上 | `FocusStore.app_disabled`・`HookState.focus_app_disabled`(atomic)・フック側のラッチ(`alt_*`・`thumb_*`・`ctrl_consumed_since_down` ほか 9 atomic)・`keymap_latch`・`shadow_key_down_disposition`(1 回目 `clear`) | FocusStore+hook static+KeymapStore+GateStore | `apply_app_disable_transition` `:547-585` | M+G(**エッジ時のみ**) |
| 4 | 同上 | `ImeBelief.prev_conversion_mode = None` | hub(belief) | `set_prev_conversion_mode(None)` `:480` | **M**(reduce の外。w0-a 領域) |
| 5 | `classify_focus_probe`(`apply_focus_probe_result` の前半) | `FocusStore.app_kind`・`focus_kind`、`FocusCache`、`ImmCapabilityStore.pending_unavailable`/`learn` | FocusStore+FocusTracker | `:350,:368,:372`、`imm_learning::learn_imm_capability_on_focus` | M |
| 6 | `record_focus_transition_if_changed` | `last_focus_transition_ms` + journal `FocusTransition` | FocusStore+journal | `:59-75` | M |
| 7 | `apply_focus_probe_result` | `Output.injection_mode`(push の複写) | Output | `update_injection_mode` | M |
| 8 | `on_focus_process_changed` 冒頭 | `seen_threads`(`&mut`)、`TSF_OBS.candidate_was_seen`、`shadow_key_down_disposition`(2 回目 `clear`)、`drift_giveup_started_at.take()`(+journal `DriftGiveUpIntervalEnded`) | FocusTracker+static+GateStore+Runtime | `:594,:628,:630,:616` | M+G |
| 9 | `enter_focus_scope` | `last_focus_change_ms`、`focus_epoch++`、`Output.composition`(`on_focus_changed`: `last_unicode_transmit_ms=0`、`cold_ctx.record_cold(FocusChange)`、`reset_consecutive_count`)、`keymap.active_keymaps` | FocusStore+Output+KeymapStore | `:173,:175,:179,:182`(`recompute_active_keymaps`) | M |
| 10 | `dispatch_event(FocusChanged)` | hub: `app_policy`・`current_focus`・`last_intent`・`key_effect`・`key_track`・`observations`(`clear_on_focus_change`)・`applied=Unknown`・`focus_generation_watermark`・`force_guards`・`observe_miss_monitor`・`input_barrier` | hub | `reduce_focus_changed` `ime_model.rs:989-1041` | **R**(これだけ) |
| 11 | 続く分岐 | hub: キャッシュ復元/破棄・`assume_closed_for_new_thread`・`reset_stale_ime_on_for_imm_broken`(`HwndCacheRestored`/`ObserverReported`) | hub | `apply_hwnd_cache_restore`・同 | R(w0-a) |
| 12 | 同 | `applied`(Confirmed に先同期)+ `GjiFsm` へ `ImeOn` | hub+Output.warmup_coord | `presync_applied_open_on` `:204` | M(+`record_confirmed`) |
| 13 | 同 | `ImmCrossProbe`(非同期で hub へ書き戻し) | hub | `spawn_local`、ticket=`FocusFence` | **A** |
| 14 | 同 | `force_guards.clear`・`observe_miss_monitor.record_success` | hub | `reset_detect_state`(`platform_state.rs:1115`、呼び元は `on_focus_process_changed` 末尾近く) | M(`reduce_focus_changed` の `clear_for_focus_change` と**重複**して 2 回消す) |
| 15 | 同 | UIA 送信 | FocusTracker | `try_send_uia` | M(OS) |
| 16 | 末尾 | `FOCUS_RESYNC.arm`(static) | static | `:892` | G |

その後、**同じフォーカス変更を `ir_stage_focus` が後処理する**(`ime_refresh.rs`):

| # | 状態 | 書く関数 | 経路 |
|---|---|---|---|
| 17 | `ir_notify_focus_changed`(`ime_refresh.rs:368`): `active_actuation = None`、半角英数トグルの強制解除(`kp_restore_kana_from_half_width`、`half_width_alnum`・IMC を書く、spawn_local のリトライ)、`input_mode` の補正(`apply_input_mode_correction`)、`Engine` へ `EngineCommand::FocusChanged`(**保留キーの flush**。`phase1_held` と lifecycle も) | `discard_actuation`・同・`execute_decision` | M+R\*+A |
| 18 | `kana_lock_hysteresis = new()`・`drift_giveup_notified_this_focus = false`・`drift_giveup_started_at = None`・`tray.set_kana_lock_warned(false)` | `ir_stage_focus:121-124` | M |
| 19 | `ir_post_focus_change_snapshot`(`ime_refresh.rs:588`、`ir_stage_observe` の末尾): `record_confirmed`(非 TsfNative)、`mark_composition_cold_focus_change`(`WarmEpoch.mark_cold`+`cold_ctx.record_cold(FocusChange)`、**2 回目の cold マーク。#9 と重複**)、`gji_on_focus_change`(`discard_pending_composition_events`〈static〉・`GjiFsm::FocusChange`・`ImeModeFsm.on_focus_changed`・`ime_mode_focus_gen++`・`ms_ime_gate_give_up=false`・`confirm_gate_deadline_override_ms=0`・`shift_conv_guard_gen++`・conv ヒントの `spawn_local`) | 同 | M+A |

**WinEvent 入口(a)** は(b)より先に走る別経路で、`on_window_focus_event`(`mod.rs:1713`)が: `try_set_focus_transition_barrier`(hub の `input_barrier`)、`Output.injection_mode`(**3 つ目の更新点**。他は `focus_tracking.rs:97`・`:153`、本件は `mod.rs:1733`)、条件付きで `mark_composition_cold_focus_change`(#19 と同じ関数)、`TsfGate::on_focus_change`(PendingWarmup に戻す)、`TIMER_TSF_GATE` をセット、`schedule_ime_refresh(debounce)` を行う。

### 6.3 フォーカスで**リセットされない**状態(設計か漏れか)

| 状態 | リセット | 実際に何が守っているか |
|---|---|---|
| `key_role_latch`(Runtime) | されない | 設計(Down と Up の対称性は scan_code が一致する限り保つ)。フォーカスをまたいだ KeyUp が来ない場合は**次の KeyDown が上書き**(`mod.rs:717` の注記)。`shadow_key_down_disposition` が同じ目的で**フォーカスで clear される**のと非対称(要確認: 両者のフォーカス時の扱いが違う理由は未確認) |
| `post_bypass`・`mode_key_pass_mark`・`external_change_watch` | 明示では行わない | **`ForegroundScope` が変わると `peek`/`live`/`remaining` が自動失効**(`ScopedOneShot`)。リセット関数に載せなくても期限切れになる設計 |
| `pending_tsf`(進行中 probe)・`pending_deferred` | **明示では捨てない**(`Output::on_focus_changed` のコメント `output/mod.rs:539-543` が「probe と一緒にドロップされる」と書くが、コードは `composition.on_focus_changed()` だけ) | `probe_io.rs:164,242` が `ime_mode_focus_gen` を照合して個別に失効。**defer 側・drain 側の全窓口で照合しているかは未確認**(fix-requires-evidence の ADR-156 行の観点) |
| `idle_conv_check_in_flight_since_ms` | されない | 8 秒の自己回復(`IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS`) |
| `INPUT_DEFER` | されない | キーは捨てると欠落になる |
| `half_width_alnum` | (17)で**強制解除(復元)** | spawn_local のリトライが `shift_conv_guard_gen` で失効 |
| `conv_mode`(`ConvModeMgr.last`) | されない | `observe()` の fence 単調ガード |
| `hook` の 9 ラッチ | `app_disabled` のエッジ時と watchdog 再インストール時のみ | (3) |

### 6.4 C′ 案への含意

- リセットの対象は **9 種の型にまたがり、フォーカスの入口は 4 つ(5 つ)、同じ状態を 2 度消す箇所が 2 つ**(#10/#14 の `force_guards`、#9/#19 の cold マーク)。
- 重複は 2 つの入口が別々に書くため、**片方だけを変えると食い違う**(fix-requires-evidence.md の「focus 遷移」ファミリーの実例)。
- 「リセット対象を 1 つの struct に集める」は、**状態ごとの寿命が 3 種類ある**ことを区別する必要がある: ①フォーカスで明示的にクリアする(`shadow_key_down_disposition`・`drift_giveup_*`・`kana_lock_hysteresis`・`active_actuation`・`composition`・`ImeModeFsm`・`ms_ime_gate_give_up`・`force_guards`)、②`ForegroundScope` が変わると自動失効(`post_bypass`・`mode_key_pass_mark`・`external_change_watch`)、③フォーカスを意識せず世代で失効(`ime_mode_focus_gen`・`focus_epoch`・`conv_mutation_seq`・`probe_actuation_fence`)。①と②③は同じ struct にまとめても意味が変わる。

---

## 7. 書き込み経路の集計(本タスクの範囲)

| 範囲 | フィールド数 | R(ImeEvent) | R\*(Engine) | M | A | G(static/atomic が書く) | H(フックスレッド) |
|---|---|---|---|---|---|---|---|
| Runtime 40 | 40 | 0 | 1 | 38(うち読み込み付きキャッシュ 3) | 0 | 0 | 0 |
| FocusStore 8 | 8 | 0 | 0 | 8 | 0 | 0 | 0 |
| GateStore 6 | 6 | 0 | 0 | 5 | 1 | 0 | 0 |
| KeymapStore 2 | 2 | 0 | 0 | 2 | 0 | 0 | 0 |
| hub のうち担当 4(通過マーク・監視窓・`last_external_change_ms`・ForceGuardSet) | 4 | 1(ForceGuard の `clear_for_focus_change`) | 0 | 4(+Mixed 2) | 0 | 0 | 0 |
| FocusTracker 8 | 8 | 1(`hwnd_ime_cache` の復元側) | 0 | 7 | 0 | 0 | 0 |
| WindowsPlatform 13(うち journal 橋・probe 記録 9) | 13 | 0 | 0 | 13 | 0 | 0 | 0 |
| Output 15 | 15 | 0 | 0 | 15(うち `&self` の内部可変 11) | 0 | 0 | 0 |
| TsfWarmupCoordinator 9 | 9 | 0(`GjiFsm` は `TimedStateMachine` 経由で別) | 0 | 9 | 0 | 0 | 0 |
| DecisionExecutor 5 | 5 | 0 | 0 | 5 | 0 | 0 | 0 |
| ImeCoordinator 2 | 2 | 0 | 0 | 2 | 0 | 0 | 0 |
| Engine(Engine 7 + NicolaFsm 約 27、数えきれていない) | 約 34 | 0 | 約 34(`on_*` 経由) | setter 経由 | 0 | 0 | 0 |
| 可変な global static / thread_local(宣言 60、うち不変 10 前後を除き約 50) | 約 50 | 0 | 0 | 一部 | 一部 | **約 50(全て)** | HookState 23 + `HOOK_KEYS`/`WAKE_*` |

意味の分類の合計(Runtime+3 Store+hub 担当+FocusTracker+Output+WindowsPlatform の主なもの): G が最多(Runtime 11 + GateStore 5 + hub 4 + Output 7 + warmup 5)、S(設定)が Runtime で 14、C が 7、B が 8、O が 6、K が 12、X が 10。

---

## 8. 失効判定の世代・フェンスと、同じ事実のコピー

### 8.1 await をまたぐ書き戻しの失効判定に使う 8 種のカウンタ

| カウンタ | どこで進む | 何を守るか | 使われる await 経路 |
|---|---|---|---|
| `FocusStore.focus_epoch` | `enter_focus_scope`(プロセス変更のみ) | フォーカス失効(**`FocusFence.epoch`**) | `ImmLikeTicket::admit`(ADR-106)・`ImmCrossProbe`・`idle-conv-check`・`focus-probe` |
| `ObservationStore.current_fence().epoch` | `FocusChanged` の reduce(`focus_epoch` の**写し**) | 観測の鮮度(derive が古い epoch の観測を捨てる) | hub 内 |
| `Output.ime_mode_focus_gen: Cell<u32>` | `gji_on_focus_change` → `on_ime_mode_focus_changed` | **別の**フォーカス世代(ADR-229 F-D5 が名指す `focus_gen` の実体)。生の `u32`、取り違え可能(ADR-229 toolkit が 22 か所と数える) | conv ヒント読み取り・MS-IME ready poll・`open_chain` の 3 関数・`cold_warmup` |
| `shift_conv_guard_gen` | 新 hold・早期 return・**フォーカス変更**・`SetOpen(true)` | `confirm_gate_deadline_override_ms` の所有権 | `kp_restore_kana_from_half_width` の spawn_local リトライ |
| `conv_mutation::CONV_MUTATION_SEQ`(static) | conv を書く側が進める | 自己出力による conv 変化の検出 | idle-conv-check |
| `probe_actuation_fence::PROBE_FENCE`(static) | GJI actuation が進める | 観測と actuation の交錯 | idle-conv-check・conv ヒント・MS-IME ready poll |
| `FOCUS_RESYNC.generation`(static) | `consume_and_close` | resync の in-flight | `kp_trigger_focus_resync` |
| `ApplyGeneration`(hub の `GenerationAllocator`) / `PressId`(`NEXT_PRESS_ID` static) / `cold_seq: Generation` / `HOOK_GEN` | 各所 | IME 反映要求・押下・warmup・フック | hub・executor・warmup |

### 8.2 同じ事実のコピーが複数ある(1 つが書かれても他は追随しない)

| 事実 | コピー(本タスクの範囲) |
|---|---|
| フォーカス hwnd | ① `FocusTracker.current.hwnd` ② `ObservationStore.current_fence().hwnd` ③ `ImeModel.current_focus` ④ `bootstrap.rs::win_event_proc` の `LAST_FOCUS_HWND`(static)。`focus_fence()`(`mod.rs`)は①を読む |
| focus epoch | `FocusStore.focus_epoch` と `ObservationStore.current_fence().epoch`(両者は混ぜない、と `reduce_ime_apply_requested` のコメントが明記) |
| フォーカスの世代(別軸) | `focus_epoch` と `Output.ime_mode_focus_gen` |
| awase 無効アプリか | `FocusStore.app_disabled` と `HookState.focus_app_disabled`(`set_focus_app_disabled` で複写) |
| エンジン有効 | `Engine.adapter(enabled)` と `HookState.cached_engine_enabled` |
| 親指キー VK・キーボードモデル・Alt なりすまし | `Runtime.keyboard_model` と `HookState.cached_*`(`set_*` で複写) |
| app profile | `FocusTracker.current.app_profile`(static 分類+学習降格)、`ImeModel.app_policy`(`FocusChanged`/`InitialAppPolicyEstablished` で導出)、`Output.injection_mode`(3 か所の push) |
| `applied` | `ImeModel.applied`(SSOT)と `DecisionExecutor.applied_snapshot`(バッチ内の複写) |
| input_mode belief | `ImeBelief.input_mode` と `DecisionExecutor.belief_input_mode`(複写) |
| IME 種別 | `TSF_OBS`(tip_detector)と `TsfWarmupCoordinator.tsf_warmup` の戦略(`set_active_ime_kind` で取り替え) |
| `input_relay_apps` | `ForceOverrides.inner`(`FocusTracker.overrides`)と `INPUT_RELAY_APPS`(static、`self` を持たない `read_ime_state_fast` 専用) |

---

## 9. reduce を通らない書き込み(M・G・H)の一覧と「Event に直せるか」の難度

「難度」は **Event(Msg)にして reducer に通す**ときの難しさ。理由は借用・await・フックスレッド・時刻・OS 読み。

### 9.1 低(状態の入れ替えだけ。入力値はすでに揃っている)

| 書き込み | 難度 | 理由 |
|---|---|---|
| 設定の反映(Runtime #4,5,8,9,17,18,20,21,23,24,26,27,33、FocusStore #5,6、`Engine` の `set_*`、`HookState.cached_*`) | 低 | `ConfigApplied(config)` 1 イベント。`apply_config_update` が既に 1 関数に集まっている(`mod.rs:2100-2260`)。ただし設定を**フックスレッドの atomic と Engine の両方に複写する副作用**がある |
| `drift_giveup_*`・`kana_lock_hysteresis`・`watchdog_kana_edge`・`state_dependent_key_warning`・`msime_key_assignment_warned`(診断・通知のデデュープ) | 低 | 入力は時刻と観測の値。ただし**いずれもユーザーの挙動に影響しない**ので Event 化の優先度は低 |
| hook watchdog の 6 フィールド(`hook_watchdog_*`・`session_locked`) | 低〜中 | 判断は既に純粋関数 `state/hook_watchdog.rs::decide` に出ている。入力は `hook_alive_tick_ms`(atomic)・`now_ms` で、**OS を読む**(`hook::hook_alive_tick_ms()`)ので Facts 化が要る |
| `FocusStore` の `app_kind`/`focus_kind`/`last_focus_*`/`focus_epoch`/`app_disabled` | 低〜中 | `FocusProbe → FocusResolved` の 1 イベントにできる(`classify_focus_probe` は OS を読む部分(`detect_app_kind`・`resolve_focus_kind`・`imm_learning`)と純粋部分が混在)。**分けるなら Facts を先に切る必要がある**(借用 `&self.platform` を渡している: `kind_classifier::resolve_focus_kind(&self.platform, …)`) |
| `GateStore.last_hook_activity_ms`・`post_bypass`・`sync_key_gate`・`keymap_latch` | 低 | 入力は物理キー。ただし `post_bypass` は `foreground_scope()` を中で読むので `_in_scope` 化が前提(ModeKeyPass 方式) |
| `Runtime.key_role_latch` | 低 | 遷移は既に純関数 `latch_step`/`settle_fkey_latch`(`state/key_effect_runtime.rs`)。入力は 2 つの値だけ |

### 9.2 中(await をまたぐ・OS を読む・借用が絡む)

| 書き込み | 難度 | 理由 |
|---|---|---|
| `GateStore.idle_conv_check_in_flight_since_ms` | 中 | **A**: spawn → await → `with_app` で下ろす。`with_app` が再入で `None` を返すと下ろし漏れ(BUG-34 横展開、8 秒の自己回復で保険) |
| `GateStore.half_width_alnum`(12 メソッド) | 中〜高 | **A**: IMC を書く spawn_local リトライ、世代で失効。状態機械は `state/half_width_alnum.rs` に分かれているが、`key_pipeline.rs` の約 12 箇所が直接叩く |
| `Output.composition` の warm/cold(Cell×7) | 中 | `&self` の内部可変。書き手が `output/`・`platform.rs`・`tsf/` に散る。時刻を `crate::hook::current_tick_ms()`(グローバル)で読む(`ms_since_last_send`) |
| `TsfWarmupCoordinator.pending_deferred`(2 窓口) | 中 | ADR-156 の「defer と drain の 2 窓口」。`RefCell`+`&self`。解放条件を足すときに片側だけ直す事故が実在(fix-requires-evidence の ADR-156 行) |
| 学習結果(`imm_learning`・`injection_mode_store`) | 中 | **書き込みごとに `cache.toml` へファイル保存**(状態更新と副作用が同じメソッド内)。イベント(`ImmCapabilityLearned`)+ Cmd(`PersistCache`)に分けられる |
| `FocusTracker.hwnd_ime_cache`(保存側) | 中 | 「フォーカスを**離れる**側」の状態を `advance_focus_tracking` で保存。`FocusLeft` イベントを 1 つ足す形になる |
| `Output.ime_mode_focus_gen`・`ms_ime_gate_give_up`・`confirm_gate_deadline_override_ms`・`shift_conv_guard_gen` | 中 | `FocusChanged` の 1 イベントで全て更新できるが、**今は `gji_on_focus_change`(`ime_refresh.rs:622` の経路)から更新**されるため、WinEvent 入口(a)では更新されない(§6.2 の注) |
| `FocusStore.focus_epoch` と `ObservationStore.fence.epoch` の二重管理 | 中 | 1 本化は reduce 側の `FocusChanged{focus_epoch}` に値を持たせる形で既に半分できている(`FocusStore` が採番し、イベントで写す) |

### 9.3 高(フックスレッド・複数スレッド・RAII・時刻と OS の直接読み)

| 書き込み | 難度 | 理由 |
|---|---|---|
| `HookState`(23 フィールド、`hook.rs:~46`) | **高** | **H**: フックスレッドが書く(`WH_KEYBOARD_LL` は `LowLevelHooksTimeout` 内に返さないとフックが外れるので、ロックもメイン待ちも不可)。doc は「20 件」と書くが**実フィールドは 23**(doc の数がずれている)。物理キー状態・親指ラッチ・Alt なりすまし・`physical_down_vk_by_identity`(BUG-181)は Down と Up を同じ関数が扱う非対称が BUG-131/132/181 の根 |
| `HOOK_KEYS`・`WAKE_PENDING` | 高 | SPSC リング、フック→メインの唯一の経路 |
| `OUTPUT_GATE`(RAII `OutputActiveGuard`) | 高 | `Drop` が書く。ADR-229 F-D4 は「`wants_output_gate: bool` を値で返し実体は shell」と決めた |
| `TSF_OBS`(約 14 フィールド、`tsf/observer.rs:78`)と GJI モニタ | 高 | モニタスレッド+WinEvent フックが書く atomic。メインは読むだけ(`tsf_obs()`) |
| `PROCESS_FLAGS`・`RAW_TSF_LITERAL`・`SEND_HEALTH`・`REJECTION_COUNTERS`・`ENGINE_HWND` 等 | 中〜高 | static の atomic/Mutex。全て「世界の事実の複写」ではなく「計数・フラグ」 |
| `Win32Timer` の論理 ID↔OS ID | 高 | OS が所有する資源。Cmd(`SetTimer`)として戻す形になる |

---

## 10. 例外イベント(reduce に入るが「ユーザー意図でも観測でもない」もの)

`ImeModel::reduce`(`state/ime_model.rs:757-990`)のアームのうち、書き手が「ユーザー意図」「観測」のどちらでもないもの。w0-a が主担当のため、本タスクが触れる範囲(フォーカス・通過マーク・外部変化・キャッシュ・パニック)だけ。

| イベント | 誰が dispatch するか | 書くフィールド(`ImeModel`) | 備考 |
|---|---|---|---|
| `FocusChanged{from,to,profile,focus_epoch}` | `on_focus_process_changed`(`focus_tracking.rs:~640`)のみ | `app_policy`・`current_focus`・`last_intent=None`・`key_effect=None`・`key_track=default`・`observations.clear_on_focus_change`・`applied=Unknown`・`focus_generation_watermark`・`force_guards.clear_for_focus_change`・`observe_miss_monitor`・`input_barrier=FocusTransition` | **11 フィールド**を 1 イベントで書く。「リセット対象をまとめる」の先例が hub の中には既にある |
| `FocusHwndUpdated{hwnd}` | `notify_focus_hwnd_updated_if_needed` | `observations.update_focus_window` のみ | fence の hwnd 追従だけ |
| `InitialFocusFenceEstablished{fence}` | `sync_initial_focus_fence`(起動時 1 回) | `observations.establish_initial_fence` のみ | 起動時 |
| `InitialAppPolicyEstablished{profile}` | `sync_initial_app_policy` | `app_policy` のみ | 起動時 |
| `InitialFocusHwndEstablished{hwnd}` | `sync_initial_focus_hwnd` | `current_focus` のみ | 起動時 |
| `HwndCacheRestored{target}` | `apply_hwnd_cache_restore`(`focus_tracking.rs` の cache_hit 分岐、`platform_state.rs:1245`) | `desired_open`・`desired_is_placeholder=false`(`last_intent` は触らない) | 「キャッシュ」由来の信念の書き込み |
| `PanicReset{target}` | `apply_panic_reset`(`runtime/mod.rs::panic_reset`←`WM_PANIC_RESET`) | `desired_open`・`desired_is_placeholder`・`applied=Unknown`(BUG-182) | **ForceGuard の追加は reduce の外**(`apply_panic_reset` が事前に積む)。`IntentStore.remove`・`clear_on_focus_change` も reduce の外 |
| `ModeKeyPassedThrough{align_desired, demote_applied}` | `pass_through_observed`(`platform_state.rs:479`)のみ | `last_intent=None`、`align_desired` なら `desired_open`(←`derive_any`)・`desired_is_placeholder=false`、`demote_applied` なら `applied=Unknown` | 呼び元は 5 経路(`drop_intents_for_mode_key_pass_in_scope`・`follow_external_change`・`align_placeholder_desired`・`align_after_expired_mode_key_pass_in_scope`・`invalidate_intents_if_mode_key_pass_live_in_scope`) |
| `KeyEffectPredicted{open,mode,track}` | `apply_key_effect_prediction`(`platform_state.rs:270`) | `key_track`・`key_effect`・`input_mode`・`last_intent=None`(open を動かすとき)・`applied=Unknown`(`applied` が予測と食い違うとき) | **予測**であり意図でも観測でもない値で `input_mode` を書く。reducer の呼び出しの**外**で `IntentStore.remove` も同時に行う(`:294`) |
| `DriftDetected{desired}` | drift correction | `applied=Optimistic(desired)` | 診断のはずの名前の事象が `applied` を書く |
| `InputModeApplied{mode,strategy,result,at}` | `apply_panic_reset`(`strategy=PanicReset`)、`apply_input_mode_correction`(`ImmBrokenCorrection` ほか) | `input_mode`(`result==Applied` のとき) | 「awase が書いた」ことの記録であり、**観測ではなく書き込みの写し** |

→ **例外イベントは 10 種**。いずれも「reduce の外で `IntentStore`・`ForceGuard`・`observations` を同時に触る」点が共通(`apply_key_effect_prediction`・`apply_panic_reset`・`pass_through_observed` 周辺・`follow_external_change`)。

---

## 11. 危険箇所と未確認の点

### 11.1 危険箇所

1. **フォーカス入口の 4〜5 分裂**(§6.1)。WinEvent 入口(a)は `ime_mode_focus_gen`・`ms_ime_gate_give_up`・`shift_conv_guard_gen` を更新しない(これらは `gji_on_focus_change`、すなわちデバウンス後の(b)でだけ更新される)。(a)から(b)までの約 50ms(`focus_debounce_ms` 既定)の間、旧世代のまま走る spawn_local は失効しない。実害の有無は**未確認**(Chrome のように 1 回の遷移で EVENT_OBJECT_FOCUS が連発する窓で問題になりうる。`LAST_FOCUS_HWND` の同一 hwnd 抑止があるので多くは (b) に集約される)。
2. **同じ状態を 2 度消す箇所**: `force_guards`(#10 の `reduce_focus_changed` と #14 の `reset_detect_state`)・cold マーク(#9 `Output::on_focus_changed` と #19 `mark_composition_cold_focus_change`)。後者は `ColdContext.last_cold_reason` を 2 回上書きし、`consecutive_count` も 2 回リセットする(冪等だが、**片方を直して他方を忘れる**構造)。
3. **`Output` の 11 フィールドが `&self` で書かれる**: 借用検査が書き込みを守らない。`with_app` の再入防止が唯一の直列化(INV-45、ADR-119/180 が `open_chain` の 3 関数に各自 `with_app` を取り直させている理由)。
4. **`GateStore` は全フィールド `pub`**: `half_width_alnum` だけは private で `state/half_width_alnum.rs` のメソッド経由(`architecture_guard.rs` が生フィールド名の出現を固定)。他の 5 つは `runtime/` のどこからでも直接書ける(`architecture_guard` の固定は無い。**未確認**)。
5. **hub の中だが reduce の外で `IntentStore.remove` を呼ぶ 4 経路**(`apply_key_effect_prediction` `platform_state.rs:294`・`drop_intents_for_mode_key_pass_in_scope` `:403`・`follow_external_change` `:469`・`apply_panic_reset` `:1173`)が `dispatch_event` と**並べて**書かれている。イベントだけ再生すると `IntentStore` が復元できない(journal の `ImeEvent` から `IntentStore` は再現できない)。これは w0-a の確認事項。
6. **`ForceOnReason::ProfilePolicy` は本番の `add` が 0**(テストだけ)。`ForceGuardSet` が複数理由を持つ設計は現状 `PanicReset` 1 種にしか使われていない(ADR-217 型の「後方互換の名目だけのコード」候補)。
7. **`state_dependent_key_warning_dialog` は死んだフィールド**(§0-6)。
8. **`dispatch_event` 自体が reduce の外で `last_user_explicit_off_ms` を書く**(`platform_state.rs:165-184`)。reducer の中ではない。w0-a の領域。
9. **`belief.is_japanese_ime` を hub が直接代入**: `observe_layout_language`(`:1401`、`lang_check_apply` から)・`apply_ime_update`・`apply_panic_reset`。本タスクの `handle_hook_key_event → lang_check_on_keydown`(OS の入力言語を読む)→ `lang_check_apply` の経路はこれに連なる(reduce を通らない)。ime-belief-architecture.md の「belief は reduce が唯一の書き込み点」とは別に、`is_japanese_ime`/`prev_conversion_mode` は `ImeBelief` の公開フィールドへの直接書き込みである点は w0-a が確認すること。
10. **キャッシュの read-through が `&mut self` で OS のファイルを読む**: `KeymapCache::get`・`RuntimeTableCache::get_for_keymap` は `now_ms` と stamp・loader を**引数で受ける**ので FCIS 適合(注入済み)だが、呼び出し元(`key_pipeline.rs:1748-1761`)が `crate::gji_charset_autodetect::*`(ファイル)を直接渡している。
11. **学習結果の保存が状態更新メソッドの中でファイル I/O**(`ImmCapabilityStore::learn`→`save`、`InjectionModeStore::learn_tsf`→`save`)。再生すると `cache.toml` を上書きする。

### 11.2 未確認の点(「たぶん」を書かずに列挙)

- `NicolaFsm` の private フィールドの正確な数(約 10 としたが全数を数えていない)。
- WinEvent 入口(a)と(b)の間に旧世代の spawn_local が走るかどうか(§11.1-1)の実害。
- `pending_tsf`・`pending_deferred` の defer 側/drain 側の**全窓口**が `ime_mode_focus_gen` を照合しているか(§6.3)。
- `GateStore` の `pub` フィールドを `runtime/` 以外から書く経路がないことを固定する `architecture_guard` テストの有無。
- 可変な global static のうち、`Mutex`/`OnceLock` で不変か可変かの内訳(§7 の「約 50」は grep の宣言数から不変 10 前後を引いた概数)。
- `SystemTray`・`Win32Timer` を journal で再現できるか(OS が所有する状態なので再現の対象外と判断したが、検証していない)。
- journal の再現性は `JournalEntry` 22 種(`KeyInput`・`TimerFired`・`ImeEvent`・`ConvClassifyCall`・`ImeActuation`・`ActuationDecision`・`SentInput`・`DriftGiveUpDiagnostic`・`HookImeModeDiagnostic`・`DriftGiveUpIntervalEnded`・`GiveUpFollow`・`ImeOpenApplied`・`PressWriteClaim`・`FocusTransition`・`GjiFsmTransition`・`TsfProbeStarted`・`TsfProbeCompleted`・`LiteralDetect`・`DeferredRecoveryFlush`・`GjiReinitRetryCompleted`・`ClockAnchor`・`DumpTriggered`)の定義から判断した。実際に replay して状態が一致するかを確かめたのは `ConvClassifyCall`(`journal-replay-guide.md` が MVP として純関数 `classify_conv_transition` のみを再生すると明記)と `ActuationDecision` だけで、他は**再生の実績がない**。

---

## 12. 仮説の検証(ime-belief-architecture.md の「近道が繰り返し発生する理由」)

> 「世界」全体が 1 つの reduce された状態ではなく、`ImeStateHub` や周辺の状態をメソッドが直接書く・グローバルが書く箇所が多い、という仮説。

**本タスクの範囲では、仮説は支持される。**

- reduce(`ImeEvent`)を通るのは hub の `ImeModel` と、そこへ流れる経路だけ。**本タスクの 約 196 の状態(Runtime 40 + Store 16 + hub 担当 4 + FocusTracker 8 + WindowsPlatform 13 + Output 15 + Warmup 9 + Executor 5 + Coordinator 2 + Engine 約 34 + static 約 50)のうち、`ImeEvent` で書かれるのは 2 つだけ**(`force_guards` の `clear_for_focus_change`、`hwnd_ime_cache` の復元の読み出し側)。
- Redux 的な入口を持つのは Engine(`on_input`/`on_timeout`/`on_command` → `Decision`)と warmup の `StepCoro`/`GjiFsm`(`timed_fsm`)の 2 系統で、**`ImeEvent` とは別の 3 つ目・4 つ目の reducer**になっている。つまり現状は「reducer が 1 つ+それ以外」ではなく「reducer が 4 系統(`ImeModel::reduce`・Engine・`GjiFsm`・`ProbeCoroState`)あり、互いの状態を複写し合っている」。
- フォーカスのような**横断的な事実**が 9 種の型・5 つの入口に分かれているのが、最大の隙間(§6)。
- 一方で、各スライスの内側は小さく閉じているものが多く(`ScopedOneShot`・`ModeKeyPassLatch`・`ExternalChangeWatch`・`latch_step`・`hook_watchdog::decide`・`half_width_alnum`・`HubClock`)、**純粋な判断を `state/` に出し、副作用の適用だけ `ImeStateHub`/`Runtime` に残す**形は既に確立している。スライス化の足場はある。

### 12.1 スライス案(本タスクの範囲)

| スライス | 入れるもの | 現在の書き手の散らばり |
|---|---|---|
| focus | `FocusStore`・`FocusTracker.current`・`seen_threads`・`app_disabled`・WinEvent の `LAST_FOCUS_HWND` | 4〜5 入口 |
| engine | `Engine`+`NicolaFsm`+`DecisionExecutor.queue/guard_held`・`ImeCoordinator` | 既に R\*。setter(`set_*`)を `EngineCommand` に畳めば閉じる |
| guard(寿命別に 3 つ) | ①focus で消える: `shadow_key_down_disposition`・`drift_giveup_*`・`active_actuation`・`ms_ime_gate_give_up`・`confirm_gate_deadline_override_ms`…、②scope で失効: `post_bypass`・`mode_key_pass_mark`・`external_change_watch`、③世代で失効: `idle_conv_check_in_flight_since_ms`・`FOCUS_RESYNC` | §6.4 |
| warmup | `Output.composition`・`tsf_warmup`・`pending_tsf`・`pending_deferred`・`ime_mode_fsm`・`WindowsPlatform` の probe 記録 | `Output` の `&self` 内部可変 11 |
| cache | `FocusCache`・`HwndImeCache`・`ImmCapabilityStore`・`InjectionModeStore`・`KeymapCache`×2・`RuntimeTableCache` | 保存がメソッド内ファイル I/O |
| config | Runtime の S 14 + `FocusStore` の S 2 + `Engine` setter + `HookState.cached_*` | `apply_config_update` 1 関数に集まっている |
| hook(shell 側に残す) | `HookState`(23)・`HOOK_KEYS`・`WAKE_*`・`TSF_OBS` | 高難度(§9.3) |
| diag | `lang_check`・`kana_lock_hysteresis`・`watchdog_kana_edge`・`state_dependent_key_warning`・`msime_key_assignment_warned` | 優先度低 |
