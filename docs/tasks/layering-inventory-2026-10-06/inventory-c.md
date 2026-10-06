# 棚卸し C: runtime/ の key_pipeline・mod.rs・transport ほか(develop 9df983e4 時点、読み取り専用)

行数は `wc -l` と自作の行数え(波括弧で関数範囲を取る、コメント・空行を別計上)で数えた。「行数」は doc コメント込みの生行数、「コード行」は空行とコメント行を除いた行数。先頭の粗い数字は使っていない。

## 0. 先に要点

- 領域 7433 行のうち、本体(テスト除く)は 6088 行、テストは 1345 行(transport.rs の 1074 行が大半)。`runtime/` 全体が `lib.rs:82-83` の `#[cfg(windows)] pub mod runtime;` で gate されているので、領域内の `#[cfg(test)]` 計 51 テストは Linux のバイナリに存在しない(`engine_window.rs` は `mod.rs:2-3` で二重に gate)。
- 「今のまま移せる(P)」は小さい(key_pipeline 87 行・mod.rs 205 行・他ファイル少々)。大半は P*(1235 + 809 行)で、**縛りはほぼ全部「PlatformState/ImeStateHub が `state/mod.rs:187-188` で `#[cfg(windows)]`」と「`self.platform.*`(WindowsPlatform)」と「グローバル static(tsf_obs・hook 時計)」の3つ**。HWND/HIMC を直接持つ関数は少なく(mod.rs の `cancel_ime_composition`・`on_window_focus_event` など)、key_pipeline.rs には HWND トークンが 0 個(`ActuationTarget`/`HwndId` の裏に隠れている)。
- F(分割が要る)が大きい: key_pipeline 1298 行・mod.rs 750 行。特に(括弧は関数本体の行数。関数表の行数は doc コメント込みで少し大きい)`kp_restore_kana_from_half_width`(286)、`apply_focus_probe`(274)、`kp_stage_idle_conv_check_inner`(186)、`kp_shift_conv_guard_key_up`(150)、`kp_shadow_actuate`(163)、`apply_config_update`(156)。
- `kp_run_inner` は「1事象を最後まで処理する turn」の入口に**すでになっている**唯一の関数だが、turn の途中で OS へ書く(shadow 段・救済窓の自己再帰)ため、Cmd を返すだけの純関数にするには順序の取り扱いが3か所で要る(§2.3)。
- `with_app` の借用: 1事象あたり同期の借用は 2 回(`lang_check_on_keydown` と `handle_wm_key_from_hook`)、resync 発火時だけ +1。turn の中での同期の再借用は 0 回で、再借用は `spawn_local` の future 内の 9 呼び出し(key_pipeline)+1(mod.rs)だけ。その 10 個のうち 5 個が戻り値の `None` を `let _ =` で黙って捨て、1 個(`key_pipeline.rs:1354`)だけ fail-open(§5)。

## 1. ファイルごとの行数と gate

| ファイル | 全体 | 本体 | テスト | テスト数 | コード行(全体) | コメント行 | gate の原因 |
|---|---|---|---|---|---|---|---|
| runtime/key_pipeline.rs | 3050 | 2949 | 101 (2950-3050) | 8 | 1901 | 1036 (34%) | `lib.rs:82-83`(runtime 全体) |
| runtime/mod.rs | 2579 | 2465 | 114 (68-115 と 2514-2579) | 7 | 1638 | 788 (31%) | 同上 |
| runtime/transport.rs | 1211 | 137 | 1074 (138-1211) | 30 | 956 | 178 | 同上 |
| runtime/focus_tracker.rs | 145 | 103 | 42 (104-145) | 5 | 90 | 41 | 同上 |
| runtime/lang_check.rs | 102 | 102 | 0 | 0 | 80 | 17 | 同上 |
| runtime/outbox.rs | 46 | 46 | 0 | 0 | 22 | 19 | 同上 |
| runtime/ime_actuation.rs | 100 | 100 | 0 | 0 | 41 | 54 | 同上 |
| runtime/ime_coordinator.rs | 29 | 29 | 0 | 0 | 11 | 15 | 同上 |
| runtime/conv_actuation.rs | 29 | 29 | 0 | 0 | 15 | 12 | 同上 |
| runtime/engine_window.rs | 142 | 128 | 14 (129-142) | 1 | 123 | 1 | 同上 + `mod.rs:2-3` の個別 `#[cfg(windows)]`(`windows::` 直接使用) |
| 合計 | 7433 | 6088 | 1345 | 51 | | | |

## 2. runtime/key_pipeline.rs(3050 行、1 つの `impl Runtime` に約 36 関数)

### 2.1 分類別の本体行数(本体 2949 行、doc コメント込みの生行数 / コード行)

| 分類 | 生行数 | コード行 | 割合 |
|---|---|---|---|
| P | 87 | 79 | 2.9% |
| P* | 1235 | 744 | 41.9% |
| O | 58 | 44 | 2.0%(kp_run_inner の診断ブロック H のみ) |
| E | 65 | 32 | 2.2% |
| F | 1298 | 831 | 44.0% |
| G | 119 | 56 | 4.0% |
| 宣言・import・空行(関数外) | 87 | - | 2.9% |

- 移せる行数(P+P*): 1322 行(44.8%)。残る行数(O+E+G): 242 行(8.2%)。分割が要る行数(F): 1298 行(44.0%)。
- F のうち純粋な判断として core に出せる部分は関数ごとにメモ欄に書いた。「F」の大半は、判断の本体が `self.platform_state`(ImeStateHub)を読み書きし、同じ関数の末尾で `spawn_local` や SendInput を呼ぶ形。

### 2.2 関数表(`kp_run_inner` はブロック表を §2.3 に分けた)

| 関数 | 行範囲(doc込) | 行数 | うちコード行 | 分類 | メモ(seam/分割案) |
|---|---|---|---|---|---|
| `process_key_event` | 24-27 | 4 | 3 | G | 入口(process_key_event) |
| `replay_ime_off_rescue_event` | 29-33 | 5 | 3 | G | 入口(救済再処理) |
| `kp_latch_keyup_to_keydown_disposition` | 267-300 | 34 | 30 | P* | KeyUp配送ラッチ。&mut gate.shadow_key_down_disposition(PlatformState)のみ |
| `kp_stage_focus_probe` | 302-350 | 49 | 35 | F | フォーカス直後probe: barrier消費+ticket作成(P*)とspawn_local+with_app完了(E/G)が同居 |
| `kp_stage_idle_conv_check` | 352-370 | 19 | 3 | G | idle_conv_check_innerへの薄いラッパ |
| `kp_trigger_focus_resync` | 372-393 | 22 | 3 | G | resync入口(app/mod.rs:649から) |
| `kp_stage_idle_conv_check_inner` | 406-591 | 186 | 117 | F | guard判定(純)+in-flight管理(状態)+fence捕獲(グローバルstatic)+spawn_local読み取り(O)+with_app完了 |
| `close_focus_resync_gate_if_current` | 593-619 | 27 | 15 | F | FOCUS_RESYNC(static)+timer.kill+OUTPUT_GATE+post_drain |
| `apply_idle_conv_check` | 621-836 | 216 | 122 | P* | spawn時fence再照合(a-d)+ConvModeMgr.observe+classify_conv_transition(純)+journal+dispatch。seam: conv_mutation::current/probe_actuation_fence::current/hook::current_tick_ms/platform.output.conv_mode |
| `kp_apply_conv_engine_sync` | 838-891 | 54 | 32 | P* | EngineSyncをdispatch。schedule_ime_refresh(E)をCmd化すれば移せる |
| `kp_stage_shadow_ime_toggle` | 893-1178 | 286 | 159 | P* | shadow toggle判断の本体。seam: tsf_obs()/時計/PlatformState。内部でkp_shadow_actuate(E含む)とkp_restore_kana(F)を同期呼び出し |
| `kp_shadow_noop_write` | 1180-1218 | 39 | 30 | P* | no-op書込判定。plan()は純、tsf_obs/profileはsnapshot化 |
| `kp_shadow_actuate` | 1220-1400 | 181 | 137 | F | claim_press_write/issue order(状態)+timer.kill+build_ime_control_view+ImeController::apply(sync)/run_open_chain_async(async,capture target,with_app) |
| `kp_stage_post_decision` | 1402-1543 | 142 | 81 | P* | SetOpen後処理(belief/intent/eisu)。timer.kill/schedule_ime_refresh/kp_reset(E)をCmd化 |
| `kp_reset_to_hiragana_romaji_capsoff` | 1545-1609 | 65 | 32 | E | CapsLock読み書き+spawn_local(capture→conv read→set_ime_conv_for_target)。mask計算2行のみ純 |
| `kp_arm_external_change_watch` | 1611-1620 | 10 | 7 | P* | 外部変化監視の武装 |
| `kp_stage_mode_key_follow` | 1622-1660 | 39 | 30 | P* | モードキー通過マーク武装 |
| `kp_stage_key_effect_track` | 1662-1705 | 44 | 34 | P* | キー効果予測の入口判定 |
| `kp_reopen_gji_fsm` | 1707-1728 | 22 | 15 | P* | GjiFsm再開。platform.output.f2_warmup_owned/GjiSyncSink(platform) |
| `kp_predict_key_effect` | 1730-1809 | 80 | 62 | P* | 予測。KeymapCache.get_gji/get_native/get_for_keymapが#[cfg(windows)]でconfig1.db/レジストリ/学習表fsを同期I/O(2s間引き)。snapshot化が必要 |
| `kp_stage_shift_conv_guard` | 1811-1875 | 65 | 17 | P* | Shift単独タップ候補管理 |
| `kp_shift_conv_guard_key_down` | 1877-1943 | 67 | 35 | P* | arm_tap/arm_guard(HalfWidthAlnumState) |
| `kp_shift_conv_guard_key_up` | 1945-2094 | 150 | 100 | F | 判断(on_shift_up純)+EnterViaImcWrite: actuate_conv_mode(E)/confirm_gate/spawn_local診断読み取り/Gji: send_gji_half_width_alnum_toggle(E) |
| `kp_send_gji_restore_exit` | 2096-2149 | 54 | 30 | F | GJI exit SendInput(E)+失敗時のrearm/belief継続可否の判断 |
| `kp_restore_kana_from_half_width` | 2151-2448 | 298 | 150 | F | 復元: 判断(was_toggle/IME種別/modifier/effective_open)+send_input_safe(E)+4回160msリトライspawn_local(with_app×最大9) |
| `kp_stage_execute` | 2450-2508 | 59 | 35 | G | executor.execute_from_hook配線+dispatch_outcomes+post WM |
| `kp_stage_kana_lock_warn` | 2510-2547 | 38 | 29 | F | OS kana lock読取(O)+hysteresis(純,awase::engine)+post WM |
| `decision_contains_romaji_send` | 2550-2562 | 13 | 13 | P |  |
| `actions_contain_romaji` | 2564-2578 | 15 | 10 | P |  |
| `streak_side` | 2580-2586 | 7 | 7 | P |  |
| `any` | 2598-2600 | 3 | 3 | P |  |
| `primary_reason` | 2602-2604 | 3 | 3 | P |  |
| `compute_focus_probe_grace` | 2607-2625 | 19 | 18 | P | tuning定数のみ参照 |
| `build_ime_on_suffix` | 2627-2648 | 22 | 22 | P | ログ文字列生成 |
| `release_detect_state_guard_if` | 2651-2663 | 13 | 5 | P* | reset_detect_state呼び出し |
| `apply_focus_probe` | 2667-2947 | 281 | 196 | F | FocusProbe適用: plan_focus_probe(純)+write_focus_probe+同期get_ime_conversion_mode_raw_timeout(10)(O,BUG-34系)+spawn_local ImmCross読み取り+with_app |

(`kp_run_inner` 本体は 35-265 の 231 行を §2.3 の表で分類。表の P*/F などの合計に `kp_run_inner` の分は含まれていないので、§2.1 の集計に足してある。)

### 2.3 `kp_run_inner`(key_pipeline.rs:35-265、231 行、コード 163 行)のブロック分類

| ブロック | 行 | 行数 | コード | 分類 | 内容 |
|---|---|---|---|---|---|
| sig | 35-38 | 4 | 3 | G | `#[expect(clippy::cognitive_complexity)]` と `too_many_lines`、シグネチャ |
| A | 39-44 | 6 | 4 | P* | `lang_check_apply` / `enrich_ime_relevance` / `enrich_key_role` / `enrich_thumb_key_role`(いずれも `&mut self` で belief とラッチを読み書き。`enrich_key_role` は `tsf_obs()`、`KeymapCache`〈fs I/O〉を引く) |
| B | 45-55 | 11 | 8 | P* | `platform.try_hold_key(event)`(TsfGate PendingWarmup 中は保留)→ 早期 return `Consumed` |
| C | 56-87 | 32 | 20 | F | Phase A 救済窓の解決: `take_ime_off_rescue_pending`(timer.kill)→ Ctrl↑ なら破棄、そうでなければ `self.kp_run_inner(pending, true)` の**自己再帰** → PassThrough なら `executor.enqueue_reinject` + `post_to_main_thread(WM_EXECUTE_EFFECTS)` |
| D | 88-89 | 2 | 2 | F | `kp_stage_focus_probe`(spawn_local)・`kp_stage_idle_conv_check`(spawn_local)。同期で呼ぶが結果は後から with_app で届く |
| E | 90-104 | 15 | 12 | P* | `pre_ctx` を組んで `engine.matches_ime_set_open` → `engine_owns_open_key`。**shadow 段より前の belief** で組む(ADR-208 D1、コメントが明示) |
| F | 105-107 | 3 | 2 | P* | `kp_stage_shadow_ime_toggle`(belief 書き込み+actuation を含む)→ `shadow_toggled`、`settle_fkey_role_latch` |
| G | 108-126 | 19 | 13 | P* | `ctx` を再度組む(shadow 段が belief を動かした後。pre_ctx との差が意味を持つ) |
| H | 127-184 | 58 | 44 | O(診断) | `GetAsyncKeyState`・`hook::is_physical_key_down`×2・`INPUT_DEFER.pending_len_nonblocking`・`OUTPUT_GATE.is_active` を読んでログに出すだけ。判断に使われない(`gas_ctrl`/`phys_ctrl`/`pending_drain`/`gate_active` はログと警告のみ)。core 側へは出さずに削る/hook の capture 時点 snapshot に寄せる対象 |
| I | 185-202 | 18 | 13 | P* | Phase B: `!skip_rescue_defer && KeyDown && ctrl && hook::ctrl_consumed_since_down() && engine.matches_ime_off(&ctx,..)` → `set_ime_off_rescue_pending`(timer.set)→ 早期 return `Consumed`。**shadow 段(F)の後**に置かれている |
| J | 203-207 | 5 | 3 | P | `engine.on_input(event, &ctx)` → `Decision`(コアのみ) |
| K | 208-222 | 15 | 9 | P* | `PhysicalKeyDisposition::plan(event, profile, shadow_toggled, active_ime_kind)`+ KeyUp ラッチ。`shadow_toggled` は F の出力 |
| M | 223-246 | 24 | 15 | P* | `journal.record_key_input`(`DecisionKind::from_decision` ほか) |
| N | 247-248 | 2 | 1 | P* | `kp_stage_post_decision` |
| O | 249-259 | 11 | 8 | P* | Ctrl 系 KeyUp で `ime.on_ctrl_key_up`(`hook::current_tick_ms` の時計) |
| P | 260-265 | 6 | 6 | G | `kp_stage_execute`(executor)+ `platform.drain_journal_entries` → journal.absorb |

集計: P* 124 / F 34 / O 58 / G 10 / P 5(合計 231)。

**列にならない依存(純関数化の邪魔になるもの)**

1. `pre_ctx`(E)は shadow 段より前、`ctx`(G)は shadow 段より後に同じ入力から組む。shadow 段が `write_sync_key`/`write_physical_key`/`on_ime_toggled`/`apply_input_mode_correction`/`eisu` 救済で belief を書くので、2 つの値は違いうる。core の `turn()` に移すときも「belief snapshot を 2 回取る」順序をそのまま保たないと ADR-208 D1 の衝突解消(`engine_owns_open_key`)が壊れる。
2. `shadow_toggled`(F の戻り値)が K の `plan()` の入力で、かつ `plan` の結果が `kp_stage_execute` の `physical` に流れる。shadow 段が OS へ書く(`kp_shadow_actuate`)戻り値と物理キー配送が 1 つの bool で結ばれている。
3. shadow 段の同期経路(`kp_shadow_actuate` の else 側)は `ImeController::apply(order,&view)` を**turn の途中で実行**し、その結果で `on_ime_apply_complete` → `record_ime_apply_result`(`applied` 更新)と `release_press_write`(押下台帳の予約解放)まで書く。後段 `kp_stage_post_decision` が呼ぶ `handle_engine_set_open` の `claim_press_write` はこの台帳に依存する。core が Cmd を後で返す形にしても、**台帳の claim だけは Cmd 発行時に core が持っている必要がある**(現在も `claim_press_write` は order 発行の直前)。
4. I(Phase B の保留)は F(shadow 段)の**後**。保留して早期 return した時点で、belief 書き込み・`claim_press_write` による予約・ラッチ更新・ImmCross の async actuation はすでに走っている。50ms 後の replay(`skip_rescue_defer=true`)は同じ `RawKeyEvent` で shadow 段を**もう一度**通る。再実行が無害なのは `claim_press_write`(`PressSource::Shadow`、`kp_shadow_actuate` 冒頭)と `settle_fkey_role_latch`/`enrich_key_role` の「Upで消さず上書きのみ」設計に依存している(`mod.rs:359-364` の doc、`key_pipeline.rs:1245-1247` のコメントが「Ctrl 救済の 50ms 保留の再処理」を明記)。turn を純関数にするなら「defer は副作用の後」を仕様として固定し、再入で idempotent なこと自体をテストにする必要がある。I を shadow 段の前へ移すと `matches_ime_off(&ctx,..)` が見る ctx(belief)が変わる(`matches_ime_off` が ctx の何を読むかは未確認)。
5. B(`try_hold_key`)は C(救済窓の解決)より前で、保留した事象は救済 pending を触らない。これを崩すと、TsfGate 保留中の event に対して救済窓が先に解決される。
6. C は `self.kp_run_inner(pending_event, true)` の**自己再帰**(1段)。再帰先は B・C を再度通る(`skip_rescue_defer=true` で I だけ無効)。再帰の戻り値は PassThrough のときだけ reinject に使い、それ以外は捨てる。driver(`deliver_key_event`)側で「保留中の救済 event があれば先に `turn(pending, skip=true)` → 次に `turn(current)`」とすれば再帰は消せる。ただし順序は 「B(hold)→ C(救済)→ 以降」なので、hold された事象では救済 pending は解決されない点を driver が守る必要がある。
7. B と I は「早期 return `Consumed`」の2か所。D(`kp_stage_focus_probe` の `consume_focus_barrier`)は one-shot でバリアを消費するため、I で保留されて replay された事象では 2 回目は消費済み。これは今の動作通り。

**「1事象を最後まで処理する turn」の入口にできる関数**

- 入口として使える: `Runtime::process_key_event`(`key_pipeline.rs:25`)と `replay_ime_off_rescue_event`(同 31)。どちらも `kp_run_inner(event, skip)` の薄い殻で、唯一の呼び出し元は `message_handlers.rs:241-243`(`deliver_key_event`)。
- 実質の turn は `kp_run_inner` 本体(A〜P)。外側から見た入力は `RawKeyEvent`(capture 時点の `modifier_snapshot`/`left/right_thumb_down_snapshot`/`was_down`/`press_id` を持つ。H を除けば live な OS 読みは不要)、出力は `CallbackResult`(PassThrough/Consumed)と、`executor` に積まれた effects・WM_EXECUTE_EFFECTS の post・timer 操作。
- 案: core 側に `fn key_turn(&mut self, snap: KeyTurnInput) -> KeyTurnOutput { callback, cmds: Vec<Cmd>, physical }` を置き、入力 snapshot に `tsf_obs`(active_ime_kind・table_ime_kind・composition_active・gji_candidate_visible)・時計・`AppImeProfile`・class_name・TsfGate の保留要否・`ctrl_consumed_since_down`・`KeymapSnapshot`(§5 参照)を載せる。A・B・E〜G・I〜O は snapshot だけで動く。F の中の `kp_restore_kana_from_half_width`・`kp_shadow_actuate` は Cmd を返すだけにする。C は driver に出す。

**with_app をいつ・何回借りるか(runtime/mod.rs と app/mod.rs の借用点)**

| 借用点 | どこ | 頻度 |
|---|---|---|
| `lang_check_on_keydown` | `app/mod.rs:627`(`handle_hook_key_event` の先頭、`with_app` 1 回) | 1 事象 1 回(KeyDown 以外・修飾付きは中で即 return) |
| `handle_wm_key_from_hook` → `deliver_key_event` → `process_key_event` → `kp_run_inner` | `app/mod.rs:665`(`with_app` 1 回。再入で `None` なら `INPUT_DEFER.replay_later`) | 1 事象 1 回。**turn 全体がこの 1 回の借用の中**。`kp_run_inner` の自己再帰も追加借用なし |
| `kp_trigger_focus_resync` + `schedule_focus_resync_deadline` | `app/mod.rs:647-652`(`with_app` 1 回) | resync 対象の最初のキーだけ。OUTPUT_GATE active または `defer_for_resync` のとき、event は `INPUT_DEFER.defer_during_output` へ退避され、turn 自体は後続の drain 再生(`message_handlers.rs:1797`)で別借用 |
| 再借用(async future 内) | key_pipeline 内 9 呼び出し(331, 542, 1354, 1599, 2333, 2360, 2402, 2419, 2852)+ mod.rs 1068 | すべて `spawn_local` の future の中。turn の同期部分からの再借用は 0 |

### 2.4 テスト(101 行・8 テスト)

`FocusProbeOpenStatus::classify` の 5 分岐(`state/observation_store`、ungated)と `decision_contains_romaji_send`(awase の型のみ)の 3 テスト。Linux に移した後はそのまま回せる(`decision_contains_romaji_send`・`actions_contain_romaji` を `state/` 側の ungated モジュールに移す前提。関数は 13+15 行の純関数で、seam 無し)。`classify` 側は既に ungated モジュール内なので、テストだけを移せば足りる。

## 3. runtime/mod.rs(2579 行、`impl Runtime` に約 85 関数 + 型定義)

### 3.1 分類別の本体行数(本体 2465 行 = 全体 2579 − テスト 114)

| 分類 | 生行数 | コード行 | 割合 |
|---|---|---|---|
| P | 205 | 127 | 8.3% |
| P* | 809 | 521 | 32.8% |
| E | 144 | 99 | 5.8% |
| F | 750 | 521 | 30.4% |
| G | 213 | 157 | 8.6% |
| 宣言・import・空行(関数外) | 344 | - | 14.0%(うち `Runtime` 構造体 317-458 の 142 行・40 フィールド) |

移せる行数(P+P*): 1014 行(41.1%)。残る行数(O+E+G): 357 行(14.5%)。分割が要る行数(F): 750 行(30.4%)。

`Runtime` 構造体 40 フィールドの内訳(未分類の宣言行): Windows 側に残る: `executor`・`platform`・`hook_guard`(+ hook watchdog 8 フィールド)・`focus_tracker`(P だが Windows 型を含まない)。core に寄せうる: `engine`・`layouts`・`lang_check`・`platform_state`(gated)・`all_keymaps`・`post_bypass_rules`・`ime_coordinator`・`active_actuation`・`key_effect_*` 3 キャッシュ(Win の fs/レジストリ読み取り closure を内包)・`key_role_latch`・`kana_lock_hysteresis` ほか。

### 3.2 関数表

| 関数 | 行範囲(doc込) | 行数 | うちコード行 | 分類 | メモ(seam/分割案) |
|---|---|---|---|---|---|
| `thumb_forced_open_actions` | 37-50 | 14 | 11 | P |  |
| `migrate_legacy_solo_tap_actions` | 52-66 | 15 | 12 | P |  |
| `resolve_dedicated_fn_key` | 117-136 | 20 | 13 | P |  |
| `build_input_context` | 138-166 | 29 | 20 | P | const fn |
| `new` | 184-186 | 3 | 3 | P |  |
| `names` | 188-190 | 3 | 3 | P |  |
| `into_vec` | 192-194 | 3 | 3 | P |  |
| `as_slice` | 196-198 | 3 | 3 | P |  |
| `resolve_index` | 202-233 | 32 | 9 | P |  |
| `strip_yab_extension` | 236-253 | 18 | 8 | P |  |
| `compile_all` | 269-293 | 25 | 21 | P | config_diagnostics(ungated)のみ |
| `matches` | 295-299 | 5 | 5 | P |  |
| `build_ctx` | 480-504 | 25 | 17 | F | O(read_os_modifiers/GetKeyState/hook::thumb_down/alt_impersonation/composing)とctx組立が同居 |
| `injection_hint` | 506-517 | 12 | 7 | P* | platform.injection_hint |
| `focus_fence` | 519-527 | 9 | 7 | P* | HwndId(isize)newtype既にあり |
| `focus_hwnd` | 529-535 | 7 | 4 | P* |  |
| `issue_actuation_order` | 539-558 | 20 | 11 | P* | Instant::now+hook::current_tick_ms→時計注入 |
| `issue_actuation_order_with_origin` | 560-571 | 12 | 11 | P* | 同上 |
| `can_use_imm32_cross_process` | 573-591 | 19 | 7 | P* | platform.current_app_profile |
| `external_change_watch_applies` | 593-603 | 11 | 7 | P* | tsf_obs global |
| `learn_imm_capability_from_miss` | 605-634 | 30 | 26 | P* | platform.focus cache+learn_imm_capability |
| `enrich_ime_relevance` | 636-644 | 9 | 3 | P* |  |
| `enrich_key_role` | 646-719 | 74 | 46 | P* | 役割ラッチ。tsf_obs()+KeymapCache(fs I/O) |
| `kanji_shadow_action` | 721-746 | 26 | 22 | P* |  |
| `settle_fkey_role_latch` | 748-769 | 22 | 14 | P* |  |
| `enrich_thumb_key_role` | 771-828 | 58 | 36 | P* | engine setter+platform.current_app_profile |
| `derive_key_shadow_action` | 830-872 | 43 | 34 | F | キャッシュ取得(config1.db/レジストリ/学習表のfs I/O)と純関数key_shadow_actionが同居 |
| `take_ime_off_rescue_pending` | 874-880 | 7 | 4 | F | pending取り出し(状態)+timer.kill(E)を一体化 |
| `set_ime_off_rescue_pending` | 882-891 | 10 | 7 | F | pending設定(状態)+timer.set(E) |
| `execute_decision` | 893-901 | 9 | 9 | G | executor.execute_from_loop配線 |
| `on_ime_apply_complete` | 903-968 | 66 | 28 | P* | journal+ime.record_ime_apply_result(純)+platform.post_ime_refresh/on_ime_applied(E) |
| `dispatch_outcomes` | 970-980 | 11 | 10 | P* |  |
| `shadow_ime_control_view` | 982-993 | 12 | 11 | P* | ImeControlView<'_>借用 |
| `toggle_engine` | 995-1017 | 23 | 12 | P* | tray.set_kana_lock_warned(E) |
| `force_engine_on` | 1019-1025 | 7 | 5 | P* |  |
| `invalidate_engine_context` | 1027-1034 | 8 | 7 | P* |  |
| `refresh_ime_state_cache` | 1036-1051 | 16 | 3 | G | run_ime_refreshへ |
| `spawn_ime_refresh` | 1053-1073 | 21 | 11 | G | async observe(run_focus_probe_async/read_ime_state_full_async)→with_app適用 |
| `schedule_ime_refresh` | 1075-1084 | 10 | 6 | E | timer.set |
| `schedule_focus_resync_deadline` | 1086-1095 | 10 | 6 | E | timer.set |
| `schedule_settle_retry` | 1097-1108 | 12 | 5 | P* | focus_settle_ms+schedule |
| `apply_input_mode_correction` | 1110-1131 | 22 | 16 | P* | dispatch_eventのみ |
| `reschedule_ime_refresh` | 1133-1205 | 73 | 45 | P* | refresh間隔の純判断。時計+platform.current_app_profile+schedule(E) |
| `settle_tsf_gate_after_refresh` | 1207-1237 | 31 | 16 | F | TsfGate遷移(platform)+timer.kill+INPUT_DEFER.replay_later(static) |
| `ime_apply_should_defer` | 1239-1268 | 30 | 5 | P* | Instant::now |
| `reload_layouts` | 1270-1300 | 31 | 16 | P* | tray名(E) |
| `switch_layout` | 1302-1319 | 18 | 14 | P* | tray(E) |
| `toggle_app_override` | 1321-1362 | 42 | 33 | P* | focus cache/tray balloon(E) |
| `process_deferred_keys` | 1364-1431 | 68 | 39 | F | unsafe poll_and_classify_ime(O)直呼び+適用。本番到達不能とコメント(sync_key_gate呼び元ゼロ) |
| `new` | 1435-1498 | 64 | 63 | G | Runtime::new |
| `keyboard_model` | 1500-1502 | 3 | 3 | G |  |
| `set_keyboard_model` | 1504-1506 | 3 | 3 | G |  |
| `set_use_learned_keymap_table` | 1508-1510 | 3 | 3 | G |  |
| `set_predict_henkan_open_in_unreadable_windows` | 1512-1514 | 3 | 3 | G |  |
| `set_update_check_enabled` | 1516-1518 | 3 | 3 | G |  |
| `set_warn_state_dependent_mode_keys` | 1520-1522 | 3 | 3 | G |  |
| `set_passthrough_thumb_mode_keys` | 1524-1550 | 27 | 22 | P* | engine.thumb_forced_open_actions |
| `learned_cells_for_warning` | 1552-1567 | 16 | 13 | F | 学習表fs I/O(get_for_keymap) |
| `check_state_dependent_mode_keys` | 1569-1640 | 72 | 70 | F | config1.db/レジストリ読取(O)+detect(純)+modalダイアログ/設定起動(E) |
| `set_half_width_alnum_toggle_policy` | 1642-1649 | 8 | 6 | P* |  |
| `set_muhenkan_dedicated_fn_key_config` | 1651-1667 | 17 | 14 | P* |  |
| `swap_msime_key_assignment_warned` | 1669-1674 | 6 | 3 | G |  |
| `reset_msime_key_assignment_warned` | 1676-1680 | 5 | 3 | G |  |
| `muhenkan_dedicated_fn_key_configured` | 1682-1691 | 10 | 4 | G |  |
| `set_space_is_thumb_key` | 1693-1698 | 6 | 3 | G |  |
| `space_is_thumb_key` | 1700-1705 | 6 | 4 | G |  |
| `tray_hwnd` | 1707-1710 | 4 | 3 | E | HWND公開 |
| `on_window_focus_event` | 1712-1780 | 69 | 47 | F | get_class_name_string/get_window_process_id/detect_app_kind/get_process_name(O)+update_injection_mode(E)+should_reprime(純)+TSF gate timer(E) |
| `start_hook_watchdog` | 1782-1788 | 7 | 6 | E | timer.set |
| `set_hook_guard` | 1790-1794 | 5 | 3 | G |  |
| `drop_hook_guard` | 1796-1800 | 5 | 3 | G |  |
| `set_hook_self_heal_enabled` | 1802-1806 | 5 | 3 | G |  |
| `set_session_locked` | 1808-1812 | 5 | 3 | G |  |
| `note_hook_watchdog_tick_alive` | 1814-1833 | 20 | 9 | P |  |
| `note_hook_watchdog_tick_not_alive` | 1835-1840 | 6 | 3 | P |  |
| `note_hook_watchdog_recovered` | 1842-1850 | 9 | 4 | P |  |
| `evaluate_hook_watchdog` | 1852-1908 | 57 | 42 | F | O(is_elevated/secure_desktop/foreground elevated/process名)+decide(純)+E(canary/reinstall) |
| `send_hook_watchdog_canary` | 1910-1951 | 42 | 22 | F | しきい値判断(純)+hook::send_hook_watchdog_canary+timer.set |
| `confirm_hook_watchdog_canary` | 1953-1987 | 35 | 26 | P* | canary_confirmed_starved(純)→reinstall呼び出し(E) |
| `reinstall_keyboard_hook_for_watchdog` | 1989-2066 | 78 | 30 | F | backoff/履歴(純)+hook drop/install(E)+latch clear(hook static) |
| `set_uia_sender` | 2068-2074 | 7 | 6 | G |  |
| `show_tray_balloon` | 2076-2079 | 4 | 3 | E | tray balloon |
| `diagnostic_snapshot` | 2081-2101 | 21 | 20 | P* | Snapshot生成 |
| `apply_config_update` | 2103-2262 | 160 | 146 | F | engineコマンド(純)+hook::set_*(static)×4+platform.focus reset+keymap再構築+警告 |
| `recompute_active_keymaps` | 2264-2293 | 30 | 19 | P* |  |
| `set_ngram_model` | 2295-2301 | 7 | 6 | P* |  |
| `drain_runtime_requests` | 2303-2327 | 25 | 18 | G | Output→Runtimeのoutbox drain(variant 1個) |
| `panic_reset` | 2329-2400 | 72 | 34 | F | cancel_ime_composition(E)+spawn_local off→on(E)+send_all_modifier_key_ups(E)+hook::reset+apply_panic_reset(純)+decision実行 |
| `send_all_modifier_key_ups` | 2403-2457 | 55 | 40 | E | SendInput(INPUT) |
| `cancel_ime_composition` | 2459-2512 | 54 | 35 | E | GetGUIThreadInfo+ImmNotifyIME(HWND/HIMC) |

(型の補足: `NonEmptyLayouts`/`LayoutEntry`/`PostBypassEntry` は P。`RuntimeDiagnosticSnapshot`(468-477)は P。`impl Debug for Runtime`(3 行)は宣言扱い。)

### 3.3 テスト(114 行・7 テスト)

- `adr192_tests`(68-115、2 テスト): `thumb_forced_open_actions` の分類。純粋関数(`awase::engine::SpecialKeyCombos` と `crate::vk` 定数のみ)で、そのまま Linux で回る。
- `layout_entry_tests`(2514-2579、5 テスト): `LayoutEntry::resolve_index`・`strip_yab_extension`。純粋。関数ごと `mod.rs` に居るため、Linux では**テスト自体がバイナリに存在しない**(`resolve_index` の BUG-104 と PR #131 の回帰テストを含む)。

## 4. 小さいファイル

### 4.1 runtime/transport.rs(本体 137 行、テスト 1074 行・30 テスト)

| 関数/型 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| `PhysicalKeyDisposition::suppress_reason` | 11-36 | 26 | P | 入力は `RawKeyEvent`・`AppImeProfile`(ungated)・`crate::vk`(ungated)のみ。gate されているのは置き場所だけ |
| `PassthroughQueue`(struct・`new`・`check_keyup_symmetry`・`check_output_guard_defer`) | 37-116 | 80 | P | `HashSet<VkCode>` のみ |
| `PhysicalKeyDisposition::plan` | 117-135 | 19 | P* | 本体は `state/physical_disposition.rs::plan_core`(ungated、248 行)。ここは `ActiveImeKind`(gated)→`ImeKindId`(`tsf/observer.rs:720` の `From`)の殻。seam: `ImeKindId` を直接渡せば消える |
| import・空行 | 1-10, 136-137 | 11 | - | |

- 移せる行数: 125 / 残る: 0 / F: 0。
- **テスト**(1074 行、30 テスト、うち `run_plan_matrix` 160 行の全数決定表):`state/physical_disposition.rs` には単体テストが 0 件(同ファイル冒頭の doc が「`plan_tests` は `#[cfg(windows)]` 配下のため Linux では存在しない。一致テストは `explicit_press.rs` 側が担う」と明記)。つまり**純粋な `plan_core` の全数決定表(ADR-166)が Linux では走らない**。テストは `plan(...)`(殻)を呼んでいるため、`plan_core`(`ImeKindId` 引数)を直接呼ぶ形に書き換えて `state/physical_disposition.rs` に移せば Linux で回るはず(`ActiveImeKind` の gate だけが障害で、他は ungated な型。**未確認**: 実際に移してコンパイルはしていない)。テスト中のコメントに「Linux では走らず windows-build で初めて失敗した実例」の記述がある。

### 4.2 runtime/focus_tracker.rs(本体 103、テスト 42・5 テスト)

`FocusTracker`(sync キー 3 リスト、`enrich_ime_relevance` 約 30 行、`decide_imm_capability` 約 15 行): すべて P*。`ImmCapability` が `focus/classifier.rs`(`focus/mod.rs` で `#[cfg(windows)]`)に居るのが唯一の seam(enum の移設)。それ以外は `RawKeyEvent`・`VkCode` だけ。移せる 103 / 残る 0 / F 0。テスト 5 本は純粋で、seam を直せば Linux で回る。

### 4.3 runtime/lang_check.rs(102 行、テスト 0)

| 関数 | 行数 | 分類 | メモ |
|---|---|---|---|
| `LangCheck::observe_keydown` | 約 36 | F | `read_thread_language(hwnd)`(O、`observer::layout_observer`)を直接呼ぶ。カウンタ(P)とログの抑止(P)は core に出せる。O の結果を引数にすれば P |
| `Runtime::lang_check_on_keydown` | 約 27 | F | `focus_hwnd()`(P*)+ O 呼び出し。ADR-223 D1: 結果を `event.ime_relevance.layout_japanese` に載せて運ぶ |
| `Runtime::lang_check_apply` | 約 21 | P* | `observe_layout_language`(belief)+ `schedule_ime_refresh(20)`(E) |

移せる(P*)約 21 + `LangCheck` の P 部分(約 20)/ 残る 0 / F 約 60。

### 4.4 その他

| ファイル | 行数 | 分類 | メモ |
|---|---|---|---|
| runtime/outbox.rs | 46 | P | `RuntimeRequest`(variant は `StartTsfProbe` の 1 個)と `RuntimeOutbox`(`Vec`)。Output→Runtime の手作りの Cmd キュー。drain 側は `mod.rs::drain_runtime_requests`(G) |
| runtime/ime_actuation.rs | 100 | P(`Actuation` 構造体、`Instant` を持つ)+ P*(`actuation_for`/`discard_actuation`、`&mut Runtime` のフィールド操作。`Instant::now()` の時計注入が seam) | |
| runtime/ime_coordinator.rs | 29 | P | `Option<RawKeyEvent>` と `Vec<(usize,usize)>`(OS timer id を持つが型は単なる整数) |
| runtime/conv_actuation.rs | 29 | G | `platform.output.actuate_conv_mode` への 1 行 delegate(ADR-084 INV-1 の関数名維持用) |
| runtime/engine_window.rs | 142 | G(全体) | message-only window。`windows::`・HWND・`PostQuitMessage`・static atomic 3 個。`ModalPumpGuard` の atomic カウンタ 3 つは純粋な論理だが同じファイルに居るので Windows 側に残る。テスト 1 本(atomic のみ) |

## 5. Windows 側の依存の種類(コード行のみ、コメント除外)

| 種類 | key_pipeline | mod.rs | transport | 他(5ファイル) |
|---|---|---|---|---|
| `with_app(` 呼び出し | 9(すべて `spawn_local` future 内。+ 文字列1) | 1 | 0 | 0 |
| `win32_async::spawn_local` | 7 | 2 | 0 | 0 |
| `win32_async::offload` / `sleep_ms` | 1 / 2 | 0 / 0 | 0 | 0 |
| `hook::` の関数(時計 `current_tick_ms` を除く) | 6 種(is_physical_key_down×2, now_timestamp_us, ime_mode_key_injection_blocked_by_modifier, ctrl_consumed_since_down, CallbackResult) | 約 15 種(thumb_vk_codes, resolve_thumb_key, set_thumb_vk_codes, set_swallow_alt_kana_mode_switch, set_alt_impersonation_enabled, send_hook_watchdog_canary, reset_physical_key_state, is_secure_desktop_active, is_alt_impersonation_active, install_hook, foreground_window_is_elevated, clear_hook_latches_for_watchdog_reinstall, hook_alive_tick_ms, thumb_down_timestamps) | 0 | 0 |
| `hook::current_tick_ms`(時計) | 20 | 8 | 0 | 0 |
| `Instant::now` | 0 | 2 | 0 | 1(ime_actuation) |
| `tsf::observer::`(`tsf_obs()` 9 + `ime_composition_active_now` ほか) | 21 | 6 | 1(`ActiveImeKind`) | 0 |
| `crate::ime::`(IMM/COM ラッパ) | 20(`ActuationTarget`×3・`get_ime_conversion_mode_*`×3・`get_ime_conv_for_target`×2・`set_ime_conv_for_target`×2・`is_caps_lock_on`/`toggle_caps_lock`・`read_ime_state_*_async`×2・`ConvAfterOpen`・`FastImeProbeResult`・`ActuationOutcome`×4) | 4 | 0 | 0 |
| `crate::imm::`(`IME_CMODE_*` 定数のみ。純粋な定数) | 10 | 1 | 0 | 0 |
| `crate::win32::`(`send_input_safe`・`post_to_main_thread`) | 2 | 1 | 0 | 0 |
| `post_to_main_thread` | 5 | 0 | 0 | 0 |
| `timer.set` / `timer.kill` | 0 / 3 | 6 / 3 | 0 | 0 |
| `TIMER_*` 定数参照 | 4 | 9 | 0 | 0 |
| `unsafe` ブロック | 8(`GetAsyncKeyState`×1・`read_kana_lock`・`foreground_class_name`・caps lock×2・`get_ime_conversion_mode_raw_timeout`×2 ほか) | 11 | 0 | 7(engine_window) |
| `GetAsyncKeyState` | 3(H ブロックのログ用) | 0 | 0 | 0 |
| グローバル static: `INPUT_DEFER`/`OUTPUT_GATE`/`FOCUS_RESYNC`/`conv_mutation`/`probe_actuation_fence`/`send_health` | 1/2/2/2/7/1 | 1/0/2/0/0/0 | 0 | 0 |
| HWND/HIMC の直接保持 | 0(`ActuationTarget`/`FocusFence` の裏) | 約 20 トークン(`HWND` 1・`hwnd` 19。`on_window_focus_event`・`tray_hwnd`・`focus_fence`・`cancel_ime_composition` に集中) | 0 | engine_window 18 |
| COM / UIA / MSAA / TSF の直接呼び出し | 0 | 0 | 0 | 0 |
| `self.platform.`(WindowsPlatform) | 34 | 54 | 0 | 0 |
| `self.platform_state.`(ImeStateHub 等、`state/mod.rs:187-188` で gated) | 78 | 41 | 0 | 1 |
| `self.executor`(DecisionExecutor) | 2 | 1 | 0 | 0 |
| `self.engine`(awase core、seam ではない) | 8 | 22 | 0 | 0 |

注意点: `KeymapCache::get_gji`/`get_native`/`RuntimeTableCache::get_for_keymap` は `state/` の `#[cfg(windows)]` メソッドで、**打鍵ごと**(2 秒に 1 回だけ版を確認)に `config1.db` の stamp・レジストリ・学習表のファイルを同期で読む(`key_effect_predictor.rs:571-588`、`key_effect_runtime.rs:817`)。呼び元は `kp_predict_key_effect`・`derive_key_shadow_action`・`learned_cells_for_warning`・`check_state_dependent_mode_keys`。「純粋」に見える判断の中に fs/レジストリ I/O が隠れている。`KeymapCache::get` 自体は stamp/load を closure で受ける設計(core に出せる)。

## 6. 領域全体の集計

### 6.1 分類別の行数(本体のみ、生行数)

| ファイル | P | P* | O | E | F | G | 宣言ほか | 本体計 |
|---|---|---|---|---|---|---|---|---|
| key_pipeline.rs | 87 | 1235 | 58 | 65 | 1298 | 119 | 87 | 2949 |
| mod.rs | 205 | 809 | 0 | 144 | 750 | 213 | 344 | 2465 |
| transport.rs | 106 | 20 | 0 | 0 | 0 | 0 | 11 | 137 |
| focus_tracker.rs | 0 | 103 | 0 | 0 | 0 | 0 | 0 | 103 |
| lang_check.rs | 約 20 | 約 21 | 0 | 0 | 約 60 | 0 | 約 1 | 102 |
| outbox.rs | 46 | 0 | 0 | 0 | 0 | 0 | 0 | 46 |
| ime_actuation.rs | 約 50 | 約 50 | 0 | 0 | 0 | 0 | 0 | 100 |
| ime_coordinator.rs | 29 | 0 | 0 | 0 | 0 | 0 | 0 | 29 |
| conv_actuation.rs | 0 | 0 | 0 | 0 | 0 | 29 | 0 | 29 |
| engine_window.rs | 0 | 0 | 0 | 0 | 0 | 128 | 0 | 128 |
| 合計 | 約 543 | 約 2238 | 58 | 209 | 約 2108 | 489 | 約 443 | 6088 |
| テスト(T) | | | | | | | | 1345(51 テスト) |

(lang_check・ime_actuation の「約」は関数境界を手で割り振った概数。他は関数範囲の機械集計。)

- 移せる行数(P+P*): 約 2781 行(45.7%)。残る行数(O+E+G): 756 行(12.4%)。分割が要る行数(F): 約 2108 行(34.6%)。宣言ほか約 443 行(7.3%)。
- コード行ベースでは P+P*(key_pipeline 823 + mod.rs 648 ほか)の比率は少し下がる。key_pipeline はコメントが 34% なので、生行数だと P*/F の「重さ」を過大に見せる。

### 6.2 seam 一覧(領域で共通して必要になる継ぎ目)

数字は「その関数本体にトークンが現れる関数の数」(コメントは除外していない関数範囲の正規表現検索、`kp_run_inner` を 1 関数として数えた。key_pipeline / mod.rs の順)。

| seam | 関数数 | 直す方向 |
|---|---|---|
| PlatformState/ImeStateHub(`self.platform_state`)が `#[cfg(windows)]`(`state/mod.rs:187-188`) | 17 / 19(うち P* 8 / 12) | `ImeStateHub` と `PlatformState` を ungated 化(中身は `ImeModel`/`ImeBelief`/journal などで、`state/platform_state.rs` 内の Win 依存を切り出す必要あり。この領域の外) |
| `self.platform.*`(focus/timer/output/tray) | 16 / 30 | `Runtime` が `WindowsPlatform` を直接持つ形をやめ、timer・tray・injection mode の書き込みは Cmd にし、focus/profile の読み取りは snapshot(`AppImeProfile`+class_name+process_name+pid)にする |
| 時計(`hook::current_tick_ms`・`Instant::now`) | 12 / 8 | `state::TickMs` を注入(`hub_clock` が既にある)。`kp_*` は event の capture 時刻を使えば `Instant::now` を消せる |
| `tsf::observer` グローバル(`tsf_obs()`・`ime_composition_active_now` ほか) | 10 / 5 | turn 入力に `TsfSnapshot`(active_ime_kind・table_ime_kind・ms_ime_native_identified・composition_active・gji_candidate_visible)を載せる。`ObservedState::from_snapshot` が既にある |
| async 再入(`spawn_local`/`with_app`/`offload`) | 7 / 2 | Cmd(`ReadConv`・`ProbeFocus`・`ApplyImeOpen` など)+ 結果を Event で返す形に。fence/ticket 照合は core 側の Event 受理関数に移す |
| 他のグローバル(`hook::*` の設定 static・`INPUT_DEFER`・`OUTPUT_GATE`・`FOCUS_RESYNC`・`conv_mutation`・`probe_actuation_fence`・`send_health`) | 6 / 10 | `apply_config_update` は設定 Cmd に、fence 値は Event に載せる |
| `crate::ime::` ラッパ(IMM/COM) | 7 / 2 | executor 側に残す。返り値は `ActuationOutcome`/`FastImeProbeResult`(plain 値)で既に分離しやすい |
| timer/post/schedule(E 呼び出しとして混入) | 8 / 11 | `Cmd::SetTimer/KillTimer/Post` |
| `ActuationTarget`/HWND | 4 / 5 | `WindowId` newtype(`HwndId(isize)` が既にある。`ActuationTarget` は capture の async 結果で、executor に閉じ込める) |
| `self.executor`(DecisionExecutor) | 2 / 1 | executor = Windows 側に残る部分そのもの |
| `unsafe` | 5 / 4 | E/O 側に閉じる |
| `KeymapCache`/`RuntimeTableCache` の `#[cfg(windows)]` 取得口 | 4 / 3 関数 | `get(now, stamp, load)` の closure を呼び出し側で渡す形は既に core 対応済み。Windows 側が stamp/load を定期に読んで snapshot として turn 入力に載せる |
| ImeController(`ImeController::apply`・`imm_cross_is_first_applicable`・`build_ime_control_view`) | `kp_shadow_actuate` 1 | `ime_controller.rs`(別担当)。借用 `ImeControlView<'_>` を所有 snapshot に |

### 6.3 新 crate への移動候補の順序案(依存の少ないものから)

1. 既に ungated な `state/` へ置き直すだけ(seam 無し、P): `decision_contains_romaji_send`・`actions_contain_romaji`(key_pipeline.rs:2550-2578)、`compute_focus_probe_grace`・`build_ime_on_suffix`・`FocusProbeGraceFlags`、`PassthroughQueue`・`suppress_reason`(transport.rs)、`thumb_forced_open_actions`・`migrate_legacy_solo_tap_actions`・`resolve_dedicated_fn_key`・`build_input_context`・`LayoutEntry`/`NonEmptyLayouts`/`PostBypassEntry`/`strip_yab_extension`(mod.rs)、`RuntimeRequest`/`RuntimeOutbox`、`ImeCoordinator`、`Actuation`、hook watchdog の P(`note_hook_watchdog_tick_alive` ほか 3 関数)。合計約 540 行 + 付随テスト 約 190 行(adr192・layout_entry・focus_tracker・key_pipeline のテスト)が Linux で回り始める。
2. `plan_tests`(transport.rs 1074 行・30 テスト)を `plan_core` に向けて `state/physical_disposition.rs` へ移す(§4.1)。これだけで Linux で回る純粋テストが約 1074 行増える。
3. `FocusTracker`(`ImmCapability` の enum 移設のみが seam)、`LangCheck` のカウンタ。
4. `state/platform_state.rs` の ungated 化が前提の P*: `kp_latch_keyup_to_keydown_disposition`・`kp_apply_conv_engine_sync`・`kp_stage_mode_key_follow`・`kp_stage_key_effect_track`・`kp_arm_external_change_watch`・`kp_stage_shift_conv_guard` 系・`apply_input_mode_correction`・`dispatch_outcomes`・`issue_actuation_order*`・`focus_fence`・`settle_fkey_role_latch`・`enrich_ime_relevance`。いずれも `self.platform_state` と小さな時計だけで動く。
5. 時計・`tsf_obs` を snapshot 引数にした P*: `kp_stage_shadow_ime_toggle`・`kp_shadow_noop_write`・`kp_stage_post_decision`・`kp_predict_key_effect`(KeymapSnapshot が前提)・`enrich_key_role`・`kanji_shadow_action`・`enrich_thumb_key_role`・`apply_idle_conv_check`(fence 値を Event に載せる)・`reschedule_ime_refresh`・`on_ime_apply_complete`。
6. F の分割: 判断と E を分けやすい順に `apply_focus_probe`(`plan_focus_probe` が既に純関数で、残りは同期 conv 読み取りと ImmCross probe の spawn)→ `kp_stage_idle_conv_check_inner`(+`close_focus_resync_gate_if_current`)→ `kp_shift_conv_guard_key_up`/`kp_send_gji_restore_exit` → `on_window_focus_event`・`evaluate_hook_watchdog`・`apply_config_update` → `kp_shadow_actuate` → `kp_restore_kana_from_half_width`(リトライ ループを含み最後)。
7. `kp_run_inner` の turn 化は上記の後。先に C(救済窓の自己再帰)を driver へ出す。

### 6.4 危険箇所(分けると隙間ができる、await をまたぐ失効など)

| # | 場所 | 内容 | 関連 |
|---|---|---|---|
| 1 | `kp_run_inner` I → shadow 段 → replay | 保留して return した時点で F(shadow 段)の belief 書き込み・press 台帳の予約・actuation が実行済みで、replay で同じ event が shadow 段を再通過する。二重送信の防止は `claim_press_write`(`PressSource::Shadow`)と「Upで消さず上書きのみ」のラッチに依存。turn を分割するとき、F と I の相対順序と、再入での idempotency を仕様に昇格させないと BUG-113 型(二重 actuation)が再発しうる | ADR-208 D1/L1、BUG-113、BUG-131/132(`mod.rs:359-364`) |
| 2 | `kp_shadow_actuate` の async 分岐(ImmCross) | order は `spawn_local` の**外**で起案(future 内は `with_app` 再入で `ImeStateHub` に届かない、ADR-090 §4.2)。完了後の焦点世代チェック(`key_pipeline.rs:1354`)は `with_app` が `None` のとき**一致扱い**(fail-open)で、他の async 完了(1599, 2333, 2360 は fail-closed)と方針が揃っていない。完了は `with_app` を握らず WM(`post_async_ime_apply_complete`)で戻す設計 | PR #408 M-3、ADR-090 |
| 3 | `kp_stage_idle_conv_check_inner` ⇄ `apply_idle_conv_check` | spawn 時に 4 種の値(`conv_mutation_seq`・`probe_actuation_fence`・`explicit_action_ms`・focus fence)を捕獲し、完了時に再照合する。core/Windows に割ると「どこで照合するか」が 2 か所になりうる。in-flight フラグは `with_app` の再入で解放されないと STALE_MS まで止まる(自己回復あり)。resync 経路は世代 gate(`FOCUS_RESYNC`)を belief 適用の**前**に閉じる契約で、順序を変えると遅れて届いた結果が belief に入る | BUG-34 横展開、BUG-77、ADR-140、ADR-106 |
| 4 | `apply_focus_probe` | `with_app` の中(完了ハンドラ内)で `get_ime_conversion_mode_raw_timeout(10)` を**同期**で呼ぶ(`SendMessageTimeoutW`、BUG-34 と同族。`send_health::blocking_allowed` の breaker でのみ守る)。dumb executor 化するときに executor 側へ出すと「ConvModeMgr 観測」と「belief 更新なし」の境界が動く(コメントが「belief は更新しない」を明示) | BUG-34、ADR-106 |
| 5 | `kp_restore_kana_from_half_width` のリトライループ | `spawn_local` で 4 回 × 160ms、各回 `with_app`(`extend_confirm_gate_override`/`clear_confirm_gate_override`/focus_gen 読み)。`owner_gen`・`focus_gen` を spawn 前に捕獲。future の中から `Runtime` を直接読み書きするので、Cmd + Event に割ると世代(`shift_conv_guard_gen`)の照合点が分裂する。`with_app` 戻り値の `None` は `let _ =` で捨てられ、override が期限まで残る(2402, 2419) | BUG-15 追補9、BUG-25/49/58、ADR-084/086 INV-14 |
| 6 | `kp_restore_kana_from_half_width` の IME 種別分岐 | `ActiveImeKind::MicrosoftIme` を条件に scan 付き VK_DBE_HIRAGANA を注入し、`else` を GJI と仮定(`debug_assert`)。しかし同ファイル `kp_predict_key_effect`(1743-1746)自身が「`MicrosoftIme` は『GJI 以外』の意味で ATOK・Japanist・未知 TIP・IMM32 HKL も含む」と書いている。`ImeKindId` は推測値で非対称な選択に使わない(INV-45)に該当しうる(同型: `kp_shift_conv_guard_key_up` の `uses_imc_conv_write`)。**実際に ATOK 等で誤注入になるかは未確認** | INV-45(ADR-089)、BUG-15 追補7 |
| 7 | `kp_stage_shadow_ime_toggle` ⇄ `kp_restore_kana_from_half_width` | shadow 段(P* として core に出す候補)が内部から F の `kp_restore_kana_from_half_width(false)` を同期で呼ぶ(半角英数トグル ON 中の IME ON 経路 3 か所: 1076, 1126, post_decision 1511)。core に出すときはこの 3 か所を Cmd に変える必要があり、`kp_send_gji_restore_exit` の「失敗時に belief 補正を続けるか」の戻り値(bool)が呼び出し元の文脈依存(Opus 2巡目の指摘がコメントに残る) | BUG-25 追補、ADR-107 |
| 8 | `process_deferred_keys`(mod.rs:1370-1431) | コメントが「呼び出し元ゼロで本番到達不能」と書く F(unsafe `poll_and_classify_ime` 直呼び + `record_confirmed(effective_open)`)。移す前に実在の呼び出し元を確認して削除の候補にできる(**未確認**: grep で呼び出し元を探していない) | ADR-098 決定5 |
| 9 | `build_ctx`(mod.rs:481)と `kp_run_inner` の `ctx` 構築 | `build_ctx` は live で `read_os_modifiers`/`hook::thumb_down_timestamps`/`is_alt_impersonation_active` を読み、`kp_run_inner` は capture 時点の snapshot(ADR-129)を使う。2 つの `InputContext` の組み方が別系統で残っている(タイマー・コマンド経路は live)。core 化の際に snapshot 版と live 版を `OsInputSnapshot` で統一できるが、drain replay の「replay している今」の値を読まない規律(ADR-129)を崩さない | ADR-129 |
| 10 | `reinstall_keyboard_hook_for_watchdog`・`apply_config_update` の `hook::set_*` | グローバル static への設定書き込み。Cmd 化すると「設定が反映済みの前提で後続が動く」順序(`resolve_thumb_key` → `set_thumb_vk_codes` → `thumb_vk_codes()` で `KeymapTable::new`)が崩れうる | issue #165 自己修復、ADR-114 決定8、BUG-103 |

### 6.5 未確認の点

- `engine.matches_ime_off(&ctx, &event)`(`src/engine/engine.rs:880`)は `match_special_keys` が `ImeOff` を返すかだけを見る副作用なしの判定。`match_special_keys` が ctx のどのフィールドを読むかは読んでいない(I を shadow 段の前へ移せるかの判断に必要)。なお `matches_ime_set_open` の `ImeToggle` の向きは `!ctx.ime_on` で決まる(確認済み)。
- `transport.rs` の `plan_tests` を `plan_core` 直呼びに書き換えるだけで Linux の型が揃うか(実際にコンパイルしていない)。
- `process_deferred_keys` の呼び出し元の有無(コメントは「ゼロ」と主張)。
- `kp_restore_kana_from_half_width` の MicrosoftIme 分岐が ATOK・Japanist で実害を出すか(実機・ログは見ていない)。
- 1 事象あたりの `with_app` 借用回数のうち、drain 再生経路(`message_handlers.rs:1797`)が事象ごとに借用するか、drain ループ全体で 1 回かは見ていない。
- `state/platform_state.rs`(ImeStateHub)を ungated 化したときの実際の Win 依存の数(別担当の範囲)。
- 行数の「P*」「F」の仕分けは各関数を私が読んで付けた判断で、F のうち純粋に core へ出せる割合は関数ごとのメモ以上には数えていない。
