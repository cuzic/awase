# ECRS 調査: crates/awase-windows/src/runtime/

## 調査範囲
worktree `/home/cuzic/rust-nicola-wt-survey`(origin/develop a15081ee)。読み取りのみ、cargo は未実行。

| ファイル | 行数 | 読んだ範囲 |
| --- | --- | --- |
| key_pipeline.rs | 3635 | kp_stage_idle_conv_check_inner, kp_stage_shadow_ime_toggle(全文), kp_shadow_actuate(全文), 1625-1650, observe_after_restore_write, apply_focus_probe(全文)。kana_lock_warn・kp_predict_key_effect・half_width_return 系は未読 |
| message_handlers.rs | 2086 | handle_wm_timer(全文)。drain_output_queue・handle_wm_command・bug report 系は関数サイズと呼び出し元の grep のみ |
| mod.rs | 2657 | 1-75, build_ctx, set_passthrough_thumb_mode_keys, apply_config_update(主要部), drain_runtime_requests。panic_reset・reschedule_ime_refresh・PostBypassEntry は未読 |
| ime_refresh.rs | 1369 | 650-975(read facts、post_focus_change_snapshot、drift 観測)、ir_apply_drift_correction 冒頭 90 行。残りは未読 |
| focus_tracking.rs | 1047 | on_focus_process_changed(全文)、末尾の純粋関数とテスト |
| executor.rs | 885 | dispatch_ime_set_open(全文) |
| open_chain.rs | 793 | grep のみ(関数名の使用件数) |
| conv_actuation.rs / ime_coordinator.rs / outbox.rs / ime_actuation.rs / lang_check.rs | 29/29/46/100/102 | 全文(lang_check は全文は読んでいないが 100 行弱を確認) |
| lang_detect_shadow / indicator_menu_shadow / mozc_shm_shadow / resume_hold / transport / engine_window / focus_tracker | 504/169/184/205/230/142/145 | 冒頭の doc のみ。ADR-263/264/268/271 の現役機能で、E 候補にしなかった |

読めなかった部分: key_pipeline.rs の約 2/3、message_handlers.rs のバグ報告組み立て・drain 部、mod.rs の後半。ここに未発見の重複が残っている可能性が高い。
未使用関数の網羅 grep(runtime/ の全 fn 名を crates と tests で数え、1 件のものを列挙)では、テスト関数以外に 0 件。到達しない関数は見つからなかった。つまり E は「関数の削除」ではなく、間接層・冗長経路・古いコメントの形になる。

## 候補表(効果の大きい順)

| ID | 種別 | 場所 | 内容 | 根拠 | 規模 | 安全網 | 再発ファミリー | リスク |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| R1 | C | message_handlers.rs:726-743 / mod.rs:501-526 | `handle_wm_timer` の engine タイマー分岐が `Runtime::build_ctx` を手書きで複製している | 下記詳述1。差分なし(read_os_modifiers → alt なりすまし補正 → thumb_down_timestamps → build_input_context、引数の並びも同じ) | -17 行 + 将来の乖離リスク除去 | 中 | 物理キー押下ラッチ(ADR-129/155) | guard の件数 1→0 を直す必要あり |
| R2 | C | key_pipeline.rs:546, 771 / ime_refresh.rs:658 | 「`half_width_alnum.is_guard_pending() \|\| is_toggle_active()`」が 3 箇所に同一の式で存在する。core の述語 1 つにする | `grep -rn "is_guard_pending()" crates` = 定義除き 3 件(上の 3 箇所、式は同一)。ime_refresh.rs:659 だけ doc に同じ意味と書いてある | -6 行、判断の窓口 1 つ | 強(純粋述語を足せば Linux で単体テスト可) | 半角英数トグル(ADR-245/BUG-193) | 3 箇所とも「conv 観測を凍結する」意味。意味が違う箇所は無かった |
| R3 | R | focus_tracking.rs:922-955, 725-756 | キャッシュ復元・破棄・「閉と仮定」の判断(`should_restore_tsf_cache_on`、`should_discard_imm_broken_cache`、cache-miss 分岐の 4 値の選択)を core の `state/` へ移す | 前者 2 つは純粋で入力は `HwndImeSnapshot`(core の `focus/hwnd_cache`)。`runtime/` は `#[cfg(windows)]` なのでテスト 8 本が Linux のテストバイナリに無い(CLAUDE.md の記述どおり)。`grep -rn "fn should_restore_tsf_cache_on\|fn should_discard_imm_broken_cache" crates` = focus_tracking.rs のみ | 約 -60 行(殻側)、+同量が core | 中(テストは Windows CI でのみ実行)→移せば強 | focus 遷移 / IME belief | `EXPLICIT_OFF_CACHE_SUPPRESS_MS` が focus_tracking.rs のローカル const。tuning.rs へ移すなら measured マクロが必須(tuning-constants.md) |
| R4 | C | key_pipeline.rs:1208-1222, 1259-1273, 1635-1655 | 「トグル ON 中なら `kp_restore_kana_from_half_width`、そうでなければ `apply_input_mode_correction`」が 3 回。違いは strategy 名と記録ラベルだけ | 3 箇所の差分: (a) `UserTurnOnEisuReset`+`TurnOnEisuReset` (b) `UserImeOnEisuReset`+`ImeOnEisuReset` (c) `PostSetOpenEisuReset`+tracing 文言。分岐の形は同じ | -25 行 | 中(shadow toggle の golden / closed_loop が覆うか未確認) | IME belief + 半角英数 | 3 箇所の順序(belief 更新後に呼ぶ)を保つこと。ログの文言は CI の grep が見ている可能性があるので未確認 |
| R5 | C | mod.rs:1586-1604, 2243-2258 / app/bootstrap.rs:1258-1266 | Engine への親指設定(space/muhenkan/henkan/enter/shift_faces)の適用が bootstrap と `apply_config_update` で二重。`from_legacy_bools` は 6 箇所 | `grep -rn from_legacy_bools crates` = windows 側 6 件(bootstrap 2、mod.rs 4)。architecture_guard.rs:6298-6370 がこの二重(起動時と reload の両方で setter を 1 回ずつ呼ぶ)を文字列で固定している | -40〜-60 行(`Engine::apply_thumb_config(left, right, &general)` 1 本にまとめる場合) | 中(guard が固定。reload の挙動の golden は未確認) | なし(config 適用)。ただし BUG-103 型(起動時と reload の乖離)の再発源 | guard の期待を書き換える。core(`awase`)側へ置けるが VK を渡すだけなので ADR-019 に反しない |
| R6 | C | key_pipeline.rs:3451-3490, focus_tracking.rs:814-846, key_pipeline.rs:3175-3200 | ImmCrossProbe の spawn(`read_ime_state_full_async` → floor 判定 → `admit_epoch_in_app` → 書き込み)が 3 箇所 | 差分: (1) focus_tracking は `write_imm_cross_probe` を直に呼び input_mode を見ない (2) apply_focus_probe は `record_imm_cross_probe_observation(..., false)` (3) resume は floor を自分で立て R1 は floor 判定を通さない。architecture_guard.rs:9050-9100 が「floor 判定は 2 箇所にある」を固定 | -30 行 | 中 | IME belief | (1) は意図的に input_mode を書かない可能性(未確認)。統一するなら挙動変更になる。**3 つ目は floor を通さないのが仕様**なので共通化するのは (1)(2) の 2 つまで |
| R7 | C | runtime/executor.rs:775-820, key_pipeline.rs:1440-1500 | ImmCross の async 実行(capture → `conv_after_open` → `OutputActiveGuard` → `run_open_chain_async` → `post_async_ime_apply_complete`)が executor と shadow toggle に 2 つ | 差分: shadow は OFF で `Untargeted`(capture 無し)、await 後に focus_gen の再確認、reason が `ShadowToggle`、generation 無し。`run_open_chain_async(` の呼び出しは architecture_guard.rs:3125 が 2 と固定 | -40 行を狙えるが、差分が多く効果は小さい | 中 | actuation 合流点(ADR-119)。fix-requires-evidence の表で「呼び出し元は構造上必要」と 2026-09-23 に棚卸し済み | **推奨しない**。棚卸し文書が同じ結論を出している。(capture+conv_after_open+guard 部分だけを関数にする案は未確認) |
| R8 | C | runtime/*.rs の 9 箇所 | `for entry in X.platform.drain_journal_entries() { …journal.absorb(entry) }` が 9 箇所 | `grep -rn "drain_journal_entries()" crates/awase-windows/src` = 定義除き runtime/ に 9 件(key_pipeline 2, message_handlers 5, focus_tracking 1, ime_refresh 1) | -20 行 | 中 | なし(ログ転送) | 呼び出し順のみ。副作用は同じ |
| R9 | S/C | key_pipeline.rs:143, 2404 | 打鍵イベントの snapshot から `build_input_context` する 7 引数呼び出しが 2 箇所(`kp_run_inner` と `engine_owns_open_key`) | 引数は両方 `event.modifier_snapshot`、`event.*_thumb_down_snapshot` と belief 4 つ。同一 | -10 行 | 中(architecture_guard.rs:253 が呼び出し元を走査) | 物理キー押下ラッチ(ADR-129) | guard の `thumb_down_timestamps` 件数は変わらない |
| R10 | S | `Runtime` 全体 | `profile.is_effectively_tsf_native(self.platform.focus.class_name())` が runtime/ に 8 箇所 | 8 箇所の位置: lang_detect_shadow:236, executor:715, focus_tracking:776, ime_refresh:810/915, mod.rs:1177, key_pipeline:562/1337/3373(うち 1 つは別の変数 `classified.class_name` を使うので除く)。focus_tracking.rs:770 のコメントが「同一引数での重複計算」を指摘している | -15 行 | 強(型検査で足りる。意味は変わらない) | focus / conv | `focus_tracking.rs:635` は分類直後で `self.platform.focus` がまだ更新前かもしれないため `classified.class_name` を使っている。ここは置換対象から外す |
| R11 | E | runtime/conv_actuation.rs(29 行) | `Runtime::actuate_conv_mode` は `Output::actuate_conv_mode` への 1 行 delegate。呼び出し元は `key_pipeline.rs:2258` の 1 件 | `grep -rn "actuate_conv_mode" crates tests` で `self.actuate_conv_mode(` = key_pipeline.rs:2258 の 1 件のみ。モジュール doc 自身が「呼び出し元を触らずに済ませるため」と書く | -29 行(ファイル削除) | 中 | conv mode / ADR-084 INV-1 | architecture_guard.rs:6641 の DECISION3_FILES から外す必要。INV-1 は関数名 `Runtime::actuate_conv_mode` を指している(ADR-084)ので ADR 側の文言も直す |
| R12 | E | runtime/ime_coordinator.rs(29 行) | `ImeCoordinator` は 2 フィールドだけの箱。doc は「`on_ime_apply_complete` を集約する」と書くが実体は無い | `grep -rn "ime_coordinator\|ImeCoordinator" crates tests` = ref 数 9。フィールド参照は message_handlers:722/1885 と mod.rs:908/915 の 4 つ。architecture_guard.rs:6648, 6670 が名前を固定している | -25 行 | 中 | 無し(2 フィールドの置き場) | guard の `NO_INSTRUMENT_EXCEPTIONS` を直す。名前を残したい意図(ADR の語彙)があるなら見送り。**価値は小さい** |
| R13 | E | runtime/outbox.rs(46 行) + mod.rs:2359-2385 + output/mod.rs:181-184,317-323 + vk_send.rs:322 | `RuntimeRequest` は variant が `StartTsfProbe` の 1 つだけ。push 側のコメント自身が「belt-and-suspenders」と書く(`send_keys` が `pending_tsf_timer()` で既にタイマーを起動する) | `grep -rn "RuntimeRequest\|RuntimeOutbox" crates` で push は vk_send.rs:322 の 1 件のみ。drain は `pending_tsf_timer()` を 1 回呼ぶだけ(platform.rs:961 と同じ関数) | -90 行 | 無(Chrome cold の実機 e2e のみ) | warmup / cold-start | **未確認**: send_keys 側の起動が全経路で必ず通るか。通らない経路があれば outbox が唯一の起動点。確かめるまで消さない |
| R14 | S | ime_refresh.rs:926-933, 760-790 | `ir_evaluate_drift` は `self.platform_state.ime.evaluate_drift(now)` への 1 行 wrapper(呼び出し 1 件)。`ir_log_poll_diff` の `miss_before` 引数は `let _ = miss_before;` で捨てている | `grep -rn ir_evaluate_drift crates tests` = 定義 + 呼び出し 1 + コメント 1 | -12 行 | 強(型検査) | IME belief(読むだけ) | なし |
| R15 | E(コメント) | ime_refresh.rs:810-880, 886, 1056, 1246 / mod.rs:1170-1172 | 撤去済み関数(`apply_force_on_for_imm_broken` 等)の経緯を語るコメントが残り、`ir_post_focus_change_snapshot` の 50 行はほぼ全部「やらないと決めたこと」の記録 | `grep -rn "apply_force_on_for_imm_broken\|force_on_and_correct_romaji\|reassert_explicit_physical_key" runtime/` = 10 件、「撤去済み」= 11 件 | -70 行(コメントのみ) | 強(コード不変) | focus / IME belief | 「なぜ盲目的なガードを足さないか」(BUG-34 追補4 の 3 案却下)は docs/experiments.md に同内容があるか未確認。無ければ移してから消す(experiment-logging.md) |
| R16 | S | message_handlers.rs:481-775 | `handle_wm_timer` が 294 行。「`kill(X)` → 1 ハンドラ呼び出し」の一発タイマー分岐が 7 つ(POWER_RESUME, TSF_GATE, RESUME_HOLD, MOZC_SHM, LANG_HOLD, GJI_LONG_IDLE, FOCUS_RESYNC, CANARY_CHECK) | 目視。TIMER_HOOK_WATCHDOG 分岐だけで約 90 行 | 表引きにしても -20 行程度、読みやすさの改善が主 | 中 | 無し | `kill` を先に呼ぶ順序が各分岐の契約(「冒頭で自ら kill する」とコメントが明記)。順序を保つこと |
| R17 | S | key_pipeline.rs:553-585 | `should_run_idle_conv_check` が bool だけ返し、スキップ理由のログのために呼び出し側が条件を再導出している(KeyDown・TsfNative・idle・explicit_age) | 同じ条件が `awase::engine::should_run_idle_conv_check` の中と、この debug ログの `if` に二重にある | -10 行 | 強(純粋関数、`awase` 側に単体テストがある。理由 enum を返せば同じテストで固定できる) | conv mode / 半角英数 | 条件が将来ずれるとログだけ嘘になる程度(挙動は変わらない) |
| R18 | R | key_pipeline.rs:610-640 | idle conv check の in-flight ゲート(`since` と `IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS` の比較、再武装の判断)を pure な plan にして core へ | 20 行の判断が `resync_generation.is_none()` と時刻の組合せで、spawn 前に殻が直書きしている | 約 -15 行(殻)、+同量(core) | 無(この判断のテストは見つからなかった) | conv mode / BUG-34 / BUG-77 | resync は共有フラグを使わない契約(BUG-77)。移す前にテストを足すこと |

## 上位 5 件の詳述

### R1 handle_wm_timer の build_ctx 複製(C、いちばん確実)
- 何を: `message_handlers.rs:726-743` の「`read_os_modifiers` → `is_alt_impersonation_active()` なら `modifiers.alt = false` → `thumb_down_timestamps()` → `build_input_context(7 引数)`」を `app.build_ctx()` に置き換える。
- 差分(同一の根拠): `Runtime::build_ctx`(mod.rs:501-526)は同じ 4 手順で、引数は `effective_open(), input_mode(), belief.is_japanese_ime(), ime_composition_active_now(), &modifiers, left_thumb_down, right_thumb_down`。timer 側も同じ並び。コメントまで同じ趣旨(「build_ctx の同様の補正参照」と timer 側が参照している)。
- 先に要る/直すもの: `architecture_guard.rs:278-300`(`thumb_down_timestamps_live_query_is_limited_to_designated_call_sites`)の `("src/runtime/message_handlers.rs", 1)` を外す。ガード自身が「ADR-155 が指摘した既知の未解消の穴」と書いている。`build_input_context_callers_do_not_drop_thumb_down_state`(architecture_guard.rs:253)の走査対象から message_handlers.rs を外してもよい。
- 安全な理由: 同一のコードの置換。`thumb_context_guard.rs` が `build_input_context` の挙動を固定している。ADR-129 の「ライブクエリ呼び出しを減らす」方向でもある。

### R2 conv 観測の凍結述語(C)
- 何を: `HalfWidthAlnum` に `fn freezes_conv_observation(&self) -> bool { self.is_guard_pending() || self.is_toggle_active() }` を足し、3 箇所(key_pipeline.rs:546, 771、ime_refresh.rs:658)を置換する。
- 差分: 3 箇所とも式が同一で、コメントも「Shift conv 安全網のブリップ中、または左 Shift 単独タップによる半角英数持続トグル中」で同じ。1 つだけ片方の述語になる将来の修正(例: ADR-245 PR 2)で、3 箇所のうち 1 つを直し忘れるのが再発パターン。
- 先に要るテスト: `state/half_width_alnum.rs` に 4 通り(guard/toggle の組み合わせ)の単体テスト。core なので Linux で走る。
- 安全な理由: 式を移すだけ。fix-requires-evidence の「半角英数トグル」行に当たるが、新規テストを足す前提なので (a) を満たす。

### R3 フォーカス時キャッシュ判断を core へ(R)
- 何を: `should_restore_tsf_cache_on`・`should_discard_imm_broken_cache` と、cache-miss 分岐の選択(`last_off_ms>0 && elapsed<SUPPRESS` / 自プロセスの窓 / `applied`(assume closed) / `reset_stale`)を、`FocusCachePlan` のような enum を返す純粋関数にして `awase-windows-core/src/state/` へ移す。殻は `Facts` を集めて enum を `match` するだけにする。
- なぜ: 現在 8 本のテストは `runtime/` 配下なので Linux では存在しない(CLAUDE.md 記載の制約)。core へ移せば `cargo test` で走る。`thread_scope.rs::should_assume_closed`/`closed_assumption` も純粋で、依存する型が core にあるか未確認(`focus/thread_scope.rs` は awase-windows 側)。
- 先に要るテスト: 現存 8 本をそのまま移植。cache-miss の 4 値の選択を表で足す(現状無)。
- リスク: `EXPLICIT_OFF_CACHE_SUPPRESS_MS` は tuning.rs 外のローカル const。tuning.rs へ移すなら `#[measured]` と実測の根拠が要る。まず core 側のローカル const に据え置き、値は変えない。

### R5 Engine 親指設定の二重(C)
- 何を: bootstrap(app/bootstrap.rs:1230-1290)と `apply_config_update`(mod.rs:2208-2260)が、space/muhenkan/henkan/enter/shift_faces の同じ setter 群を同順で呼ぶ。`Engine::apply_thumb_config(left: VkCode, right: VkCode, &GeneralConfig)` を `awase` 側へ作って両方から呼ぶ。`from_legacy_bools` の 6 箇所のうち 4 箇所(`set_passthrough_thumb_mode_keys` 含む)も同時に整理できる。
- 差分: bootstrap は `special_keys` の組み立て、`set_backspace_vk` と `apply_general_config` を持つ。reload 側は `set_alt_impersonation_enabled`・`set_space_is_thumb_key`・fn key の解決・`set_passthrough_thumb_mode_keys` を持つ。つまり**完全な複製ではなく共通部分は約 35 行**。ここだけを統合する。
- 先に要るテスト: architecture_guard.rs:6298-6370 が「両方から setter を 1 回ずつ呼ぶ」を固定している。統合後は「両方が `apply_thumb_config` を 1 回呼ぶ」に書き換える。reload の golden は未確認。
- リスク: BUG-103(起動時と reload の乖離)の再発源そのものなので、統合は再発の抑止になる。ただし `VK_SPACE` などの VK 定数は ADR-019 により `awase` 側に置けない。呼び出し側(殻)で `Option<VkCode>` に解決してから渡すこと。

### R4 トグル中なら復元、でなければ書き戻し(C)
- 何を: `fn kp_eisu_reset_or_delegate_to_toggle_off(&mut self, new_mode, strategy, tick_ms, step_ok, step_delegated)` を 1 つ作り、3 箇所(1208, 1259, 1635)を置換。
- 差分: 分岐の構造と呼び出し先(`kp_restore_kana_from_half_width(false, ExitOwnership::FromToggle)` / `apply_input_mode_correction(new_mode, strategy, tick_ms)`)は同一。違いは `InputModeApplyStrategy` の variant と、記録のラベル(`ShadowToggleStep::*` か tracing 文言)の 2 点。コメントの理由も「E 節の理由は上と同じ」と互いを参照している。
- 先に要るテスト: shadow toggle の closed_loop シナリオが `TurnOnDelegatedToToggleOff` / `ImeOnDelegatedToToggleOff` を覆うかは未確認。足りなければ journal replay を 1 本足す。
- リスク: `Delegated...` の journal 記録(`kp_note_shadow_toggle`)が 3 つ目だけ tracing で、journal ではない。統一すると journal の件数が変わる。**3 つ目はログ文言を残すか、2 つだけ統合する**。

## 範囲外の注意
- 「呼び出し元の整理」(R7)は fix-requires-evidence.md の 2026-09-23 棚卸しで負と結論済み。候補表には載せたが推奨しない。
- 規模の見積りは行数の目分量で、cargo は動かしていない。
