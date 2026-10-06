# W0-a 棚卸し: ImeStateHub / ImeModel / ImeEvent / journal の書き込み点

調査対象は `origin/develop` の ee54b2de。メイン作業ツリー(HEAD efe5248e)は読んでいない。`git show` で `crates/awase-windows/src/state/` と `journal.rs` を取り出して読んだ。ソースの編集・ビルド・テストはしていない。

行番号は origin/develop 上の値で、`git grep` で数えた。件数は自分で数えたものだけを書き、数えていないものは「未確認」と書いた。

## 0. 結論(仮説の検証結果)

仮説「`ImeStateHub` や周辺の状態をメソッドが直接書く・グローバルが書く箇所が多い」は、**成立する**。

1. **`ImeEvent` の variant は 65 ではなく 20 個**(`state/ime_event.rs` の enum 本体を数えた)。そのうち `UserImeToggleIntent` と `UserChangedInputMode` は、本番コードに構築箇所が無い(`git grep` で `ImeModel::reduce` のアームとテストだけ)。実際に動く variant は 18 個。
2. **`ImeModel`(16 フィールド)のうち、reduce だけが書くものは 10 個**。残り 6 個(`observations`・`input_barrier`・`force_guards`・`observe_miss_monitor`・`pending`・`applied`)は、`ImeStateHub` のメソッドが `shadow_model.<field>` を直接書く経路も持つ。`ImeModel` のこれらのフィールドは `pub`。private なのは `desired_open`・`desired_is_placeholder`・`input_mode`・`focus_generation_watermark`・`last_seen_generation`・`current_focus`・`key_effect`・`key_track` だけで、belief 層の保護はこの 8 個に限られる。
3. **`ImeStateHub`(14 フィールド)のうち、reduce を通る状態は `shadow_model` だけ**。他の 13 個は `ImeEvent` を通らない(`belief`、`intent_store`、`mode_key_pass_mark`、`external_change_watch`、`press_ledger`、`generation_alloc`、2 つの `last_*_ms` と `last_external_change_ms` など)。`&mut self` のメソッドは 53 個。そのうち `dispatch_event` を呼ぶだけで済むものは 13 個で、27 個は Event を一切出さない。
4. **journal だけから `ImeModel` を再現することはできない**(第 8 節)。理由は 3 つ。
   - `JournalEntry::ImeEvent` が `tick_ms`・`Instant`・event_log の `seq` を持たない。
   - 直接書き込み(`applied`、`pending`、`input_barrier`、`force_guards`、`observe_miss_monitor`、`intent_store`、`belief`)に対応する記録が無い。
   - `ImeEvent` が `Serialize` のみで `Deserialize` が無い。journal は 2048 件のリングで、古い分は捨てられる。
5. **時刻の入口が 3 つあり、互いに無関係**(第 9 節)。
   - reducer に渡る時刻は `EventTime { seq, monotonic, tick_ms }`。`monotonic` は `HubClock`、`tick_ms` は呼び出し側の引数から来る。
   - hub の読み取りは `clock.now_tick()` と `clock.now_instant()` を直接読む。
   - 一部のメソッドは `std::time::Instant::now()`・`hook::current_tick_ms()`・`win32::foreground_scope()` を呼び出し側が読んで渡す、または hub の中で読む。

## 1. `ImeEvent` 全 variant の表(20 個)

「例外」は `.claude/rules/ime-belief-architecture.md` が「観測でもユーザー意図でもない直接書き込みの正当な例外」として隔離している 4 つ(PanicReset・HwndCacheRestored・ModeKeyPassedThrough・KeyEffectPredicted)。本表の「分類」列は、その 4 つを「例外」とし、`DriftDetected` と `UserChangedInputMode` は reduce が観測でも意図でもなく直接書くので「準例外」とした(私の分類)。

| # | variant | reduce が書くフィールド | 発行元(関数) | 分類 |
|---|---|---|---|---|
| 1 | `UserImeToggleIntent { source }` | `key_effect=None`、`key_track.stage=None`、`desired_open` を反転、`desired_is_placeholder=false`、`last_intent` | **本番の発行元なし**(テストのみ) | 通常(死んだ variant) |
| 2 | `UserImeSetIntent { target, source }` | 同上(反転でなく `target` を設定) | `write_sync_key`・`write_physical_key`(`kp_stage_shadow_ime_toggle`、key_pipeline.rs:1025/1033)、`write_set_open_request`(`handle_engine_set_open` 経由、key_pipeline.rs:1423) | 通常(ユーザー意図) |
| 3 | `PanicReset { target }` | `desired_open`、`desired_is_placeholder=false`、`applied=Unknown`。`last_intent` は触らない | `apply_panic_reset`(platform_state.rs:1167、呼び出し元 runtime/mod.rs:2373) | **例外** |
| 4 | `HwndCacheRestored { target }` | `desired_open`、`desired_is_placeholder=false`。`last_intent` は触らない | `apply_hwnd_cache_restore`(platform_state.rs:1254、呼び出し元 focus_tracking.rs:693/735) | **例外** |
| 5 | `ImeApplyRequested { target, generation, ctrl_held }` | `last_seen_generation`、`pending`(新規)、`input_barrier`(`!target && ctrl_held` で CtrlImeChord、`target` かつ chord 中なら解除) | `handle_engine_set_open`(platform_state.rs:636、呼び出し元 key_pipeline.rs:1423) | 通常 |
| 6 | `ImeApplySucceeded { target, generation }` | `pending`(世代一致なら take)、`applied`(Accepted なら `Confirmed`、Superseded なら `Optimistic`) | `record_ime_apply_result` が `from_apply_outcome` で作る(platform_state.rs:1100、呼び出し元 runtime/mod.rs:957 `on_ime_apply_complete`) | 通常 |
| 7 | `ImeApplyFailed { target, generation, error }` | `pending`(世代一致なら take)、`applied`(Accepted の `Failed` 系なら `Confirmed{!target}`) | 同上 | 通常 |
| 8 | `ObserverReported(AnyObservation)` | `observations.per_source`(`record_replayed`、`at=envelope.time.monotonic`)、`observations.drift`(`update_drift`)、`key_effect`(開閉軸の照合) | `write_observer_poll`(ime_refresh.rs:189 `ir_stage_observe`)、`apply_ime_update` の `observer_poll`(ime_refresh.rs:527、runtime/mod.rs:1392)、`follow_external_change`、`write_focus_probe`(key_pipeline.rs:2745 `apply_focus_probe`)、`write_imm_cross_probe`(focus_tracking.rs:843、key_pipeline.rs:2861)、`report_conv_open_inference`(key_pipeline.rs:864 `kp_apply_conv_engine_sync`)、`reset_stale_ime_on_for_imm_broken`・`assume_closed_for_new_thread`(HeuristicDefault、focus_tracking.rs:778/773) | 通常(観測) |
| 9 | `FocusChanged { from, to, profile, focus_epoch }` | `app_policy`、`current_focus`、`last_intent=None`、`key_effect=None`、`key_track=default`、`observations`(`clear_on_focus_change`)、`applied=Unknown`、`focus_generation_watermark`、`force_guards`(`clear_for_focus_change`)、`observe_miss_monitor`(`record_success`)、`input_barrier=FocusTransition` | `on_focus_process_changed`(focus_tracking.rs:639) | 通常(フォーカス) |
| 10 | `FocusHwndUpdated { hwnd }` | `observations.current_fence.hwnd` のみ。`current_focus` は書かない | `notify_focus_hwnd_updated_if_needed`(focus_tracking.rs:523) | 通常 |
| 11 | `InitialFocusFenceEstablished { fence }` | `observations.current_fence` のみ | `sync_initial_focus_fence`(focus_tracking.rs:254) | 起動時初期化 |
| 12 | `InitialAppPolicyEstablished { profile }` | `app_policy` のみ | `sync_initial_app_policy`(focus_tracking.rs:274) | 起動時初期化 |
| 13 | `InitialFocusHwndEstablished { hwnd }` | `current_focus` のみ | `sync_initial_focus_hwnd`(focus_tracking.rs:290) | 起動時初期化 |
| 14 | `ModeKeyPassedThrough { align_desired, demote_applied }` | `last_intent=None`、`align_desired` かつ `derive_any` が Some なら `desired_open`・`desired_is_placeholder=false`、`demote_applied` かつ `applied` が食い違うなら `applied=Unknown` | `pass_through_observed`(platform_state.rs:479)。呼び出し元は `follow_external_change`・`drop_intents_for_mode_key_pass_in_scope`・`align_placeholder_desired`・`align_after_expired_mode_key_pass_in_scope` の 4 つ | **例外** |
| 15 | `KeyEffectPredicted { open, mode, track }` | `key_track`、`key_effect`(開閉・モードのどちらかが Some のとき)、`input_mode`(`mode` が Some)、`last_intent=None`(`open` が Some)、`applied=Unknown`(`open` が `applied` と食い違うとき)。`desired_open` は書かない | `apply_key_effect_prediction`(platform_state.rs:270、呼び出し元 key_pipeline.rs:1800 `kp_predict_key_effect`) | **例外** |
| 16 | `ChordEnded { kind }` | `input_barrier=None` | `on_ctrl_key_up`(platform_state.rs:680、呼び出し元 key_pipeline.rs:257) | 通常 |
| 17 | `DriftDetected { desired, observed, duration_ms }` | `applied=Optimistic(desired)` のみ | `ir_apply_drift_correction`(ime_refresh.rs:972) | 準例外(観測でも意図でもなく、補正の副作用として `applied` を書く) |
| 18 | `InputModeObserved { mode, source, confidence, at }` | Medium 以上かつ fence 外なら `input_mode`・`key_effect`(モード軸の照合)・`key_track.conv=None`。Low は記録のみ | `apply_ime_update` の `new_input_mode`(ObserverPoll、Medium)、`apply_idle_conv_check`(ConvBitsInference、High、key_pipeline.rs:821)、`apply_focus_probe`(ImmCrossProbe、High、key_pipeline.rs:2882)、`ir_stage_observe`(GjiIoInference、Medium、ime_refresh.rs:201) | 通常(観測) |
| 19 | `InputModeApplied { mode, strategy, result, at }` | `result==Applied` のとき `input_mode` | `apply_input_mode_correction`(runtime/mod.rs:1116。呼び出し元は ime_refresh.rs:397、key_pipeline.rs:1078/1128/1517/2042/2063/2441)、`apply_panic_reset`、`apply_hwnd_cache_restore` | 準例外(awase 自身の能動的訂正) |
| 20 | `UserChangedInputMode { mode, at }` | `input_mode` | **本番の発行元なし** | 準例外(死んだ variant) |

注: 同じ `ImeEvent` の `from_apply_outcome` は、`ImeOpenOutcome` の 5 値を `ImeApplySucceeded`/`ImeApplyFailed` へ写す(`Applied`・`AppliedWithoutSendInput`・`AlreadyMatched` が Succeeded)。

## 2. `ImeModel` のフィールド表(16 個)

凡例: B=belief、I=intent、O=observation、G=guard、C=cache、K=diagnostic、X=infrastructure。書き込み経路の R=reduce、M=メソッドが直接書く。「読み手」は、`ime_model.rs`・`platform_state.rs` 以外の本番ファイルでのヒット行を `git grep` で数えた粗い値(正確な読み手の関数数ではなく未確認)。

| フィールド | 可視性 | 分類 | 書き手 | 経路 | 読み手(粗い値) | journal から再現 | スライス案 |
|---|---|---|---|---|---|---|---|
| `desired_open` | private | B/I | reduce: 1・2・3・4・14 | R | ime_refresh.rs 他 | 一部(書く event は記録される。`ModeKeyPassedThrough` は `derive_any` の結果に依存し、観測プールと時刻が要る) | belief |
| `desired_is_placeholder` | private | B | reduce: 1・2・3・4・14 | R | platform_state.rs 経由 | 同上 | belief |
| `input_mode` | private | B | reduce: 15・18・19・20 | R | key_pipeline.rs 10、runtime/mod.rs 4、ime_refresh.rs 3、message_handlers.rs 2、focus_tracking.rs 2、executor.rs 1 | 一部(18 の fence 判定が `key_effect` と `at.0` に依存) | belief |
| `last_intent` | **pub** | I | reduce: 1・2・9・14・15 | R | platform_state.rs 7、drift_correction.rs 1、ime_refresh.rs 1、focus_tracking.rs 1 | 一部(`at_ms` は `tick_ms` で、journal に無い) | intent |
| `observations`(`per_source`・`drift`・`current_fence`) | **pub** | O | reduce: 8・9・10・11。M: `apply_panic_reset` の `clear_on_focus_change(cur_fence)`(platform_state.rs:1175 付近) | **Mixed** | platform_state.rs 8、ime_refresh.rs 2、drift_correction.rs 1 | 一部(`at` が `Instant`)。M の分は記録なし | observation |
| `app_policy` | **pub** | G(profile から導く値) | reduce: 9・12 | R | platform_state.rs 4(`focus_settle_ms`、`default_feedback`、`warrant_context`) | Yes(`profile` が payload) | focus |
| `input_barrier` | **pub** | G | reduce: 5・9・16。M: `consume_focus_barrier`・`clear_input_barrier`(runtime/mod.rs:2390)・`try_set_focus_transition_barrier`(runtime/mod.rs:1720) | **Mixed** | platform_state.rs 6、runtime/mod.rs 1 | No(M の分は記録なし。`started_at`・`settle_until` は `Instant`) | guard |
| `force_guards` | **pub** | G | reduce: 9(`clear_for_focus_change`)。M: `reset_detect_state`(`clear`)、`release_panic_reset_guard_on_positive_evidence`(`remove`)、`apply_panic_reset`(`clear`+`add`)、`apply_ime_update`(`remove`) | **Mixed** | platform_state.rs 10、open_warrant.rs 1 | No(M の書き込みに記録なし。`PanicReset` event から add/clear は推測できるが、他は不可) | guard |
| `observe_miss_monitor` | **pub** | K(ほぼ診断。`detect_miss_count` が focus_tracking.rs:860 の `reset_detect_state` の呼び出し条件には使われる) | reduce: 9(`record_success`)。M: `reset_detect_state`、`apply_panic_reset`、`apply_ime_update`(`record_miss`/`record_success`) | **Mixed** | platform_state.rs 6、ime_refresh.rs 3、focus_tracking.rs 1 | No | guard |
| `pending` | **pub** | B(apply の進行中) | reduce: 5・6・7、末尾の期限切れ破棄。M: `confirm_applied`(ime_model.rs:536)、`clear_pending_if_matches`(`record_optimistic` の一部) | **Mixed** | platform_state.rs 5、executor.rs 4 | 一部(5・6・7 は generation が payload。M は記録なし、`ImeOpenApplied` が部分的な手掛かり) | belief |
| `focus_generation_watermark` | private | G | reduce: 9 | R | ime_model.rs の中 | 一部(`last_seen_generation` が要る) | guard |
| `last_seen_generation` | private | G | reduce: 5 | R | ime_model.rs の中 | Yes(5 の payload) | guard |
| `applied` | **pub** | B(awase の直近の書き込みの記録) | reduce: 3・6・7・9・14・15・17。M: `record_optimistic`、`record_confirmed`(= `confirm_applied`) | **Mixed** | platform_state.rs 5、message_handlers.rs 2、executor.rs 2、runtime/mod.rs 1、key_pipeline.rs 1、ime_refresh.rs 1 | 一部(reduce 分は Yes。`Confirmed.at_ms` の `tick_ms` が journal に無い。M 分は記録なし) | belief |
| `current_focus` | private | B | reduce: 9・13 | R | platform_state.rs 11 | Yes | focus |
| `key_effect` | private | B | reduce: 1・2・8・9・15・18 | R | ime_model.rs の中 | 一部(`at_ms=tick_ms` が journal に無い) | belief |
| `key_track` | private | B | reduce: 1・2・9・15・18 | R | key_pipeline.rs 3、platform_state.rs 1 | Yes(15 の payload が `track`) | belief |

注: `ImeModel::current_focus`(reduce: 9・13)と `observations.current_fence.hwnd`(reduce: 9・10・11)は**別の「現在のフォーカス」を持つ**。10 は後者だけを書き、前者は更新しない。

集計(ImeModel 16 フィールド): R のみ 10(`desired_open`・`desired_is_placeholder`・`input_mode`・`last_intent`・`app_policy`・`focus_generation_watermark`・`last_seen_generation`・`current_focus`・`key_effect`・`key_track`)、Mixed(R+M)6(`observations`・`input_barrier`・`force_guards`・`observe_miss_monitor`・`pending`・`applied`)。

## 3. `ImeStateHub` のフィールド表(14 個)

| フィールド | 分類 | 書き手 | 経路 | journal から再現 | スライス案 |
|---|---|---|---|---|---|
| `belief`(`is_japanese_ime`・`prev_conversion_mode`、`ImeBelief`) | B/O(観測の結果の保持) | `apply_panic_reset`、`apply_ime_update`、`set_is_japanese_ime`、`observe_layout_language`、`set_prev_conversion_mode` | M(Event なし。フィールドは `pub(in crate::state)`) | No | belief(Event 化) |
| `event_log`(`ImeEventLog`、512 件のリング) | X/K | `dispatch_event` の `record_at` だけ | wrapper | No(本番でどこにも読み出されない。第 4 節) | 廃止または journal へ統合 |
| `clock`(`HubClock`) | X | 構築時に `Wall{tick: hook::current_tick_ms}`。本番で書き換えない(`advance_ms` はテストのみ) | なし | 該当なし | shell |
| `journal`(`UnifiedJournal`) | X/K | `dispatch_event` の `record(ImeEvent)`。それ以外に runtime の直接 `record`/`absorb`/`record_key_input`(`ime.journal` への `record`/`absorb` などのアクセスは platform_state.rs の外の本番で 24 行) | Mixed | — | shell(出力) |
| `shadow_model`(`ImeModel`) | B/I/O/G | reduce と第 2 節の M | Mixed | 一部(第 8 節) | 核 |
| `last_user_explicit_off_ms` | I/G | `dispatch_event` の先頭で `UserImeSetIntent` を見て直接書く(platform_state.rs:171-176。reduce の外) | M(event から導出) | 一部(event 順で再現可能だが `tick_ms` が journal に無い) | intent |
| `last_explicit_ime_action_ms` | G | `handle_engine_set_open`(platform_state.rs:669)、`note_explicit_ime_action`(key_pipeline.rs:1260/1999/2062/2172 の 4 呼び出し) | M | No(`handle_engine_set_open` 分は `ImeApplyRequested` から推測可、`note_explicit_ime_action` 分は不可) | guard |
| `intent_store`(`IntentStore`、hwnd ごと、TTL あり) | I | `record`(`write_sync_key`・`write_physical_key`・`record_explicit_intent`)、`remove`(`apply_key_effect_prediction`・`drop_intents_for_mode_key_pass_in_scope`・`follow_external_change`・`apply_panic_reset`)、`invalidate_for_cache_restore`(`apply_hwnd_cache_restore`) | M | No(`UserImeSetIntent` から `record` を推測できるのは sync/physical key 経由だけ。Command 由来は key_pipeline.rs:1432 の `record_explicit_intent` の有無で分かれ、記録なし。`remove` も記録なし) | intent |
| `mode_key_pass_mark`(`ModeKeyPassLatch<ForegroundScope>`) | G | `arm`・`note_awase_write`・`live`・`window_remaining_ms`・`expiry_wait_ms`・`drop_decision`・`align_after_expired`(`live` など問い合わせ名のメソッドも `&mut self`。`ScopedOneShot::peek` が scope 不一致で `armed=None` にする) | M | No(結果の `ModeKeyPassedThrough` だけが記録される。arm や note は記録なし) | guard |
| `external_change_watch`(`ExternalChangeWatch<ForegroundScope>`) | G/O | `arm`・`record_read`・`observe`・`remaining_ms` | M | No | guard |
| `last_external_change_ms` | G/K | `follow_external_change`(platform_state.rs:472)。読み手 ime_refresh.rs:175 | M | No | guard |
| `intent_override_logged`(`Cell<bool>`) | K | **`&self` の `effective_open_at`** の中(`Cell::set`) | 読み取りの中の副作用(診断ログの出し分け) | 該当なし | 診断 |
| `generation_alloc`(`GenerationAllocator`) | X | `allocate_event_generation`(key_pipeline.rs:1418) | M | 一部(使われた世代は `ImeApplyRequested.generation` に載る。チョード抑止で捨てた世代は journal に無い。単調な最大値だけを復元すればよい) | shell |
| `press_ledger`(`PressLedger`) | G | `claim_press_write`・`release_press_write` | M | **Yes**(`JournalEntry::PressWriteClaim{press, open, source, verdict}` を claim と release の両方で記録。ただし `press=None` は記録しない) | guard |

集計(Hub 14 フィールド): reduce を通る状態は 1(`shadow_model`)。`dispatch_event` のラッパーだけが書くものが 1(`event_log`)。Mixed が 2(`shadow_model`・`journal`)。M のみが 9(`belief`・`last_user_explicit_off_ms`・`last_explicit_ime_action_ms`・`intent_store`・`mode_key_pass_mark`・`external_change_watch`・`last_external_change_ms`・`generation_alloc`・`press_ledger`)。書かれないものが 1(`clock`)。`&self` の中で書かれるものが 1(`intent_override_logged`)。

### 3-1. `ImeStateHub` の `&mut self` メソッド 53 個の分類

`#[cfg(test)]` の 2 個(`set_desired_open_for_test`・`clear_last_intent_for_test`)は除く。

**(A) `dispatch_event` を呼ぶだけで、他は書かない: 12 個**
`pass_through_observed`、`write_observer_poll`、`write_focus_probe`、`write_imm_cross_probe`、`report_conv_open_inference`、`write_set_open_request`、`on_ctrl_key_up`、`reset_stale_ime_on_for_imm_broken`、`assume_closed_for_new_thread`、`align_placeholder_desired`、`align_after_expired_mode_key_pass`、`align_after_expired_mode_key_pass_in_scope`。

**(B) Event を出し、さらに直接も書く(Mixed): 13 個**
- `apply_key_effect_prediction`: Event 15 に加え、`intent_store.remove`。
- `apply_hwnd_cache_restore`: Event 4 と 19 に加え、`intent_store.invalidate_for_cache_restore`。
- `write_sync_key`、`write_physical_key`: Event 2 に加え、`intent_store.record`(`record_explicit_intent` 経由)。
- `handle_engine_set_open`: `write_set_open_request`、`on_set_open_requested`(`force_guards.clear`・`observe_miss_monitor.record_success`)、Event 5、`last_explicit_ime_action_ms`。
- `apply_panic_reset`: Event 19 と 3 の 2 つ。その間に `belief` 2 フィールド、`observe_miss_monitor`、`force_guards`(`clear`+`add(PanicReset)`、`generation=event_log.next_seq()`)、`intent_store.remove`、`observations.clear_on_focus_change` を直接書く。
- `apply_ime_update`: Event 8 と 18、`belief` 2 フィールド、`observe_miss_monitor.record_miss`/`record_success`、`force_guards.remove`。
- `follow_external_change`: `external_change_watch`、Event 8、`intent_store.remove`、Event 14、`last_external_change_ms`。
- `drop_intents_for_mode_key_pass_in_scope`、`expire_mode_key_pass_mark`、`invalidate_intents_if_mode_key_pass_live_in_scope`、`invalidate_intents_if_mode_key_pass_live`: `intent_store.remove` と Event 14(後の 3 つは前者へ委譲)。
- `record_ime_apply_result`: `generation` が Some のときは Event 6/7。None のときは `record_confirmed`(直接書き込み)。

**(C) Event を一切出さない直接書き込み: 27 個**
- **`shadow_model`(= `ImeModel`)を直接書く: 10 個**
  - `record_optimistic`、`record_confirmed`、`clear_pending_if_matches`: `applied`・`pending`。
  - `consume_focus_barrier`、`clear_input_barrier`、`try_set_focus_transition_barrier`: `input_barrier`。
  - `reset_detect_state`、`on_ime_toggled`、`on_set_open_requested`: `observe_miss_monitor`・`force_guards`。
  - `release_panic_reset_guard_on_positive_evidence`: `force_guards`。
- **`belief` を書く: 3 個**: `set_is_japanese_ime`、`observe_layout_language`、`set_prev_conversion_mode`。
- **`intent_store` を書く: 1 個**: `record_explicit_intent`(`kp_stage_post_decision` が key_pipeline.rs:1432 から呼ぶ)。
- **時刻のフィールドを書く: 1 個**: `note_explicit_ime_action`。
- **`press_ledger` を書く: 2 個**: `claim_press_write`、`release_press_write`(journal の `PressWriteClaim` だけは記録する)。
- **ラッチと監視窓: 9 個**: `arm_mode_key_pass_mark`、`mode_key_pass_expiry_wait_ms`、`note_awase_write_for_mode_key_pass`、`note_awase_write_for_mode_key_pass_in_scope`、`mode_key_pass_mark_live_in_scope`、`mode_key_pass_window_remaining_ms`、`mode_key_pass_mark_live`、`arm_external_change_watch`、`external_change_watch_remaining_ms`。
- **世代の採番: 1 個**: `allocate_event_generation`。

残り 1 個は `dispatch_event` 自身(先頭で `last_user_explicit_off_ms` を書く)。合計 12+13+27+1=53。

## 4. 周辺ストアの書き込み点

### `ObservationStore`(observation_store.rs)

- `record*`(`record`・`record_belief`・`record_replayed`)は本番では `ImeModel::reduce` の `ObserverReported` アームだけが呼ぶ(`git grep` で確認)。
- `update_drift`・`establish_initial_fence`・`update_focus_window`・`clear_on_focus_change` も reduce の中。
- 例外は 1 か所: `ImeStateHub::apply_panic_reset` が `observations.clear_on_focus_change(cur_fence)` を reduce の外で直接呼ぶ。
- 観測の `at: Instant` は、プローブが読んだ時刻ではなく `envelope.time.monotonic`(= dispatch 時の `HubClock::now_instant()`)。`await` をまたぐ非同期プローブでは完了時刻が入る。受理判定(`AcceptedObservation`)は dispatch の前に shell 側(`probe_admission.rs`)で済ませる。

### `IntentStore`(intent_store.rs)

- `ImeEvent` を一切通らない。書き手は第 3 節の表のとおり(`record`・`remove`・`invalidate_for_cache_restore`)。
- 期限は `TickMs` を引数で受ける。`effective_open_at` の読み取りで `TickMs` が必要。
- reduce の `FocusChanged` はこの store を消さない。hwnd をキーにした TTL だけで失効する。
- `IntentStore::clear` は hub からは呼ばれていない(`git grep` で確認)。

### `ForceGuardSet` / `ObserveMissMonitor`(force_guard.rs)

- 本番で追加されるのは `ForceOnReason::PanicReset`(`expires_at=None`)だけ。`ProfilePolicy` を add するのはテストと `open_warrant.rs` のテストのみ。
- `purge_expired` は本番から呼ばれない。`expires_at` が常に None なので影響はない。
- `ForceGuard.generation` は `event_log.next_seq()` から取る。読み手は未確認(`generation` を読む本番コードは見つからなかった)。

### `ImeEventLog`(ime_event_log.rs)

- 書き手は `dispatch_event` だけ。512 件のリングで、**本番の読み手がいない**(`recent`・`iter` は `platform_state.rs` のテストだけ)。ダンプもされない。
- ただし `next_seq()` は 2 か所で値として使われる: `InputBarrier::FocusTransition.started_seq`(`try_set_focus_transition_barrier`)と `ForceGuard.generation`(`apply_panic_reset`)。
- つまり event_log の実質は「`seq` を採番するカウンタ」。`UnifiedJournal` は別の `next_seq`(`Arc<AtomicU64>`)を持ち、全 `JournalEntry` で共有する。同じ event でも 2 つの `seq` 空間が並ぶ。

### `UnifiedJournal`(journal.rs)

- `JournalEntry` は 22 種(enum 本体を数えた)。4 つのレーン(State・Timing・Actuation・KeyInput)。State と Timing は各 2048 件。
- `dispatch_event` は、reduce の**後**に `JournalEntry::ImeEvent { event }` を記録する(platform_state.rs:199)。記録される値は `event` だけ。
- `JournalEnvelope { seq, elapsed_ms, entry }` の `seq` と `elapsed_ms` は journal 自身の `quanta::Clock` で付く。`ImeEventEnvelope.time` とは別物。
- `JournalEntry` は `Serialize` のみ。`ImeEvent` も `Serialize` のみで、`Deserialize` は付いていない。

## 5. 集計(領域全体)

### 書き込み経路の別

| 領域 | R のみ | M(直接)のみ | Mixed | ラッパー/なし |
|---|---|---|---|---|
| `ImeModel` 16 フィールド | 10 | 0 | 6 | 0 |
| `ImeStateHub` 14 フィールド | 0 | 9 | 2 | event_log(wrapper)1、clock(なし)1、intent_override_logged(`&self` 内)1 |
| `ImeStateHub` の `&mut self` メソッド 53 個 | — | 27 | 13 | A: 12、`dispatch_event` 自身 1 |

G(グローバル/static/atomic)と H(フックスレッド)が、この領域のフィールドを書く箇所は見つからなかった。`probe_admission.rs` の棄却カウンタはこの領域の外(未調査)。

### 意味の分類の別(フィールド数。ImeModel 16 と Hub 14 の合計 30、`shadow_model` は ImeModel 側で数えるため Hub からは除く)

| 分類 | 数 | 内訳 |
|---|---|---|
| B(belief) | 8 | `desired_open`・`desired_is_placeholder`・`input_mode`・`pending`・`applied`・`current_focus`・`key_effect`・`key_track`(+ Hub の `belief` を B/O として別枠で 1) |
| I(intent) | 2 | `last_intent`、`intent_store`(+ `last_user_explicit_off_ms` を I/G として別枠で 1) |
| O(observation) | 1 | `observations` |
| G(guard) | 7 | `app_policy`・`input_barrier`・`force_guards`・`focus_generation_watermark`・`last_seen_generation`(ImeModel)、`mode_key_pass_mark`・`external_change_watch`・`press_ledger`・`last_explicit_ime_action_ms`(Hub) |
| K(診断) | 2 | `observe_miss_monitor`、`intent_override_logged` |
| X(infrastructure) | 4 | `event_log`・`clock`・`journal`・`generation_alloc` |

(分類が複合のものがあるため、行の合計は 30 と一致しない。)

## 6. reduce を通らない書き込み(M)の一覧と、Event 化の難度

| 書き込み | Event に直せるか | 難度 | 理由 |
|---|---|---|---|
| `record_optimistic`・`record_confirmed`(`applied`・`pending`) | `ImeApplyRequested`/`Succeeded` と似た `AppliedRecorded{open, kind, at}` を足せる | 低〜中 | `ImeModel::confirm_applied` が純粋部として既に切り出されている。`note_awase_write_for_mode_key_pass`(OS の `foreground_scope()` を読む)が同じメソッドの中にある。呼び出し元 4 か所は `effective_open()` を読んで書く(第 10 節) |
| `consume_focus_barrier`・`clear_input_barrier`・`try_set_focus_transition_barrier` | `InputBarrierConsumed`・`FocusBarrierArmed` | 低 | `ImeModel` の中だけで閉じる。`started_at` は呼び出し側が渡す `Instant`、`started_seq` は `event_log.next_seq()` |
| `reset_detect_state`・`on_ime_toggled`・`on_set_open_requested`・`release_panic_reset_guard_on_positive_evidence`(`force_guards`・`observe_miss_monitor`) | `DetectStateReset`・`PanicResetGuardReleased` | 低 | `ImeModel` の中だけ |
| `apply_ime_update` の miss 計数・guard 解除・`belief` | `ObserverMissed`・`PanicGuardCleared`・`LanguageObserved` | 低〜中 | 時刻は `clock.now_instant()` の読み取り(`HubClock` なので注入できる) |
| `apply_panic_reset` の直接書き込み(5 種) | `PanicReset` の reduce 側へ寄せる | 中 | 2 つの Event の間にある書き込みで順序が意味を持つ。`force_guards.add` の `generation` は `event_log.next_seq()`(reduce の中では取れない) |
| `belief`(`set_is_japanese_ime`・`observe_layout_language`・`set_prev_conversion_mode`) | `LayoutLanguageObserved`・`ConvModeObserved` | 低 | 呼び出し元(key_pipeline.rs、lang_check.rs、focus_tracking.rs)がすでに値を持っている |
| `intent_store.record`・`remove`・`invalidate_for_cache_restore` | `IntentRecorded{hwnd,...}`・`IntentRemoved` | 中 | 現在の `ImeModel::current_focus` を読んで hwnd を決める(状態が 2 つにまたがる)。`TickMs` が必要 |
| `last_explicit_ime_action_ms`・`last_external_change_ms` | `ExplicitActionNoted{at}` | 低 | 値を保存するだけ。key_pipeline.rs:425/664 が `await` をまたいで値の一致を比べる(第 10 節) |
| `mode_key_pass_mark`・`external_change_watch` | 状態機械を `ImeModel` の外の別スライスにして Event を通す | 高 | `ForegroundScope` を呼び出しごとに OS から読む。`live`/`window_remaining_ms` など「問い合わせ」が `peek` で状態を変える |
| `press_ledger` | 既に `PressWriteClaim` を journal に記録している。reduce に通す必要は薄い | 低 | — |
| `generation_alloc` | 採番はカウンタなので shell に置くのが自然 | 低 | — |
| `intent_override_logged`(`&self` の中の `Cell`) | 診断の出し分けなので Event 化は不要。読み取りから外す | 低 | ログの重複抑止だけ |

## 7. 例外イベント(reduce に入るが、観測でもユーザー意図でもない)

| Event | 書くフィールド | 備考 |
|---|---|---|
| `PanicReset { target }` | `desired_open`、`desired_is_placeholder`、`applied=Unknown` | `last_intent` は触らない。`apply_panic_reset` 専用。前後に直接書き込みが 5 種ある(第 3-1 節 B) |
| `HwndCacheRestored { target }` | `desired_open`、`desired_is_placeholder` | `last_intent` は触らない。`apply_hwnd_cache_restore` 専用。同じ関数が `intent_store.invalidate_for_cache_restore` を直接呼ぶ |
| `ModeKeyPassedThrough { align_desired, demote_applied }` | `last_intent=None`、条件付きで `desired_open`・`desired_is_placeholder`・`applied=Unknown` | 発行元 4 経路。`derive_any(envelope.time.monotonic)` を reduce の中で読む(観測プールと時刻に依存) |
| `KeyEffectPredicted { open, mode, track }` | `key_track`、`key_effect`、`input_mode`、`last_intent=None`、`applied=Unknown` | `desired_open` は書かない。同じメソッドが `intent_store.remove` も呼ぶ |
| `DriftDetected` | `applied=Optimistic(desired)` | 直後に `ir_apply_drift_correction` が `record_optimistic(desired)` を重ねて呼ぶ経路がある(ime_refresh.rs:993)。reduce と直接書き込みが同じ値を二重に書く |
| `InputModeApplied { strategy }` | `input_mode` | strategy は 7 種(`ImmBrokenCorrection`・`PanicReset`・`CacheRestore`・`PostSetOpenEisuReset`・`UserImeOnEisuReset`・`UserTurnOnEisuReset`・`UserHalfWidthAlnumToggle`) |
| `UserChangedInputMode` | `input_mode` | 本番の発行元なし |

dylint と `architecture_guard.rs` が固定しているのは、このうち 4 つ(PanicReset・HwndCacheRestored・ModeKeyPassedThrough・KeyEffectPredicted)の構築場所。`DriftDetected` と `InputModeApplied` は設計上の例外として隔離されていない。

## 8. journal / event_log から `ImeModel`・`ImeBelief` を再現できるか(再生の完全性)

結論: **できない**。経路は 2 つあり、どちらも欠けている。

### 8-1. `event_log`(`ImeEventLog`)

- 持っているのは `ImeEventEnvelope { time: {seq, monotonic, tick_ms}, event }` で、**時刻は完全**。
- ただし 512 件のリングで、**ダンプされず、`Serialize` もない**(`ImeEventEnvelope` は `Debug, Clone` のみ)。再生に使える形では外に出ない。

### 8-2. `journal`(`UnifiedJournal`)

`JournalEntry::ImeEvent { event }` から `reduce` を再実行するには、次が足りない。

1. **`tick_ms`**: reduce が `record_intent` の `at_ms`、`applied.Confirmed.at_ms`、`key_effect.at_ms` の判定に使う。journal には入らない。journal の `elapsed_ms`(quanta、ms 精度、journal 開始からの経過)と `ClockAnchor{tick_ms, hook_us}` から近似はできる。ただし `ClockAnchor` を記録するのは bootstrap(bootstrap.rs:734)と message_handlers.rs の 2 か所(1206・1984)のダンプ時だけ。
2. **`Instant`(`monotonic`)**: observation の `at`、`drift.started_at`、barrier の `started_at`・`settle_until`、`pending.timeout_at`、`derive_any` の鮮度判定に使う。`elapsed_ms` で近似はできるが、`HubClock`(`Instant::now()`)と journal の `quanta::Clock` は別の時計。
3. **event_log の `seq`**: `started_seq` と `ForceGuard.generation` に使われる。journal の `seq` とは別空間。
4. **型**: `ImeEvent` は `Serialize` のみ。`AnyObservation` のフィールドは private で、`restored_from_journal` という構築関数だけがある。`FocusFence`・`ApplyGeneration`・`KeyTrack`・`InputModeState`(awase コア)に `Deserialize` が付いているかは未確認。既存の `journal_replay.rs` は `ImeEvent` の 3 variant(`ImeApplyRequested`・`ImeApplySucceeded`・`FocusChanged`)を手書きの専用型で再生しているだけ。
5. **リングの打ち切り**: State レーンは 2048 件で、`FocusTransition`・`ImeOpenApplied`・`ClockAnchor` と共有される。古い Event は捨てられ、初期状態が分からなくなる。`DumpTriggered` に捨てた件数だけが記録される。

### 8-3. reduce の外で書かれ、記録の無い状態

| 状態 | 記録 | 再現できるか |
|---|---|---|
| `applied`・`pending` への `record_optimistic`/`record_confirmed` | 記録なし(`on_ime_apply_complete` が `ImeOpenApplied{open,outcome,reason}` を記録するが、`generation`・時刻が無い。`focus_tracking.rs:205`・`ime_refresh.rs:618`・`key_pipeline.rs:1290`・`runtime/mod.rs:1409` の 4 経路は journal に何も残さない) | No |
| `input_barrier` の M 書き込み 3 種 | 記録なし | No |
| `force_guards`・`observe_miss_monitor` の M 書き込み | 記録なし(`PanicReset` event から `add`+`clear` は推測可) | 一部 |
| `observations` の `clear_on_focus_change`(`apply_panic_reset`) | 記録なし(`PanicReset` event から推測可) | 一部 |
| `belief.is_japanese_ime`・`belief.prev_conversion_mode` | 記録なし(`ConvClassifyCall` が `conv` を記録するが、`prev_conversion_mode` の書き込みとは別) | No |
| `intent_store` | 記録なし | No(上記第 3 節) |
| `last_explicit_ime_action_ms` の `note_explicit_ime_action` 分 | 記録なし | No |
| `mode_key_pass_mark`・`external_change_watch` | 記録なし(`ModeKeyPassedThrough` が結果として残るだけ) | No |

### 8-4. 再現できるもの

- `desired_open`・`desired_is_placeholder`・`app_policy`・`current_focus`・`key_track`・`last_seen_generation`: 書く Event の payload に必要な値が入っている。ただし `desired_open` は `ModeKeyPassedThrough` の `derive_any` に依存するため、観測の時刻が要る。
- `press_ledger`: `PressWriteClaim` の記録で再現できる(`press=None` を除く)。

## 9. 時刻の扱い

| 時刻 | 持ち主 | 読む場所 | 使い道 |
|---|---|---|---|
| `EventTime.seq` | `ImeEventLog.next_seq`(Event ごとに +1) | `dispatch_event` が `record_at` で採番 | barrier の `started_seq` と `ForceGuard.generation`(順序判断には使われていない) |
| `EventTime.monotonic` | `HubClock::now_instant()`(本番は `Instant::now()`) | `dispatch_event` が毎回読む | pending の期限切れ、barrier の `settle_until`、観測の `at`、drift の起点、`derive_any` の鮮度 |
| `EventTime.tick_ms` | **呼び出し側が `dispatch_event(event, tick_ms)` の引数で渡す**。呼び出し元によって `hook::current_tick_ms()` を読んだ値、`TickMs(ts)`(`on_ime_apply_complete` の `ts`)、event の payload の `at` などが混ざる | reduce が `envelope.time.tick_ms` で読む | `last_intent.at_ms`、`applied.Confirmed.at_ms`、`key_effect.at_ms`、`reconcile_key_effect_*` の fence 判定 |
| `HubClock.now_tick()` | `HubClock` | `effective_open()`(platform_state.rs の `self.clock.now_tick()`)だけ | `intent_store.resolve_effective_open` の TTL 判定 |
| `HubClock.now_instant()` | `HubClock` | `dispatch_event`、`effective_open_at`、`apply_ime_update` の `record_miss` | 上記 |
| 引数 `now: Instant` | 呼び出し側が `Instant::now()` を読む | `align_placeholder_desired`(ime_refresh.rs:696)、`try_set_focus_transition_barrier`(runtime/mod.rs:1720)、`check_drift_correction`、`warrant_context`、`issue_actuation_order` | hub は `HubClock` を通さずに時刻を受ける |
| 引数 `now_ms: u64` / `tick_ms: TickMs` | 呼び出し側が `hook::current_tick_ms()` を読む | モードキー通過・外部変化の各メソッド、`effective_open_at`、`note_explicit_ime_action` | TTL・窓の判定 |
| `ImeModel::effective_open()`(ime_model.rs:435) | `Instant::now()` を直接読む | `ImeModel` 内の 1 か所(コメントに「この 1 箇所」と明記)と `apply_engine_request_and_completion`(ime_model.rs:504、テスト用オラクルだが `pub fn` で本番コードの中にある) | 読み取り |
| `win32::foreground_scope()` | OS | `mode_key_pass_*`・`external_change_watch`・`follow_external_change` など platform_state.rs の約 10 か所 | latch の scope 一致 |

整理:
- Event に載る時刻は `tick_ms` と、Event の payload に入った `at: TickMs`(`InputModeObserved`・`InputModeApplied`・`UserChangedInputMode`)。`Instant` と `seq` は Event に載らず、`dispatch_event` が付ける。
- `at: TickMs` を持つ 3 variant は、`tick_ms` と同じ値を二重に持つ(`InputModeObserved` の `reconcile_key_effect_mode` は `at.0` を使い、`envelope.time.tick_ms` ではない)。
- hub の読み取りが `clock` を直接読む。`effective_open()` が `&self` なのに `clock.now_tick()`・`clock.now_instant()` を読み、さらに `intent_override_logged` に書く。
- 時計を差し替えられるのは `HubClock` を通る箇所だけ。呼び出し側が `Instant::now()`・`current_tick_ms()` を読んで渡す経路は、`HubClock::Manual` に差し替えても追随しない。

## 10. 危険箇所と未確認の点

### 危険箇所

1. **`applied` を `effective_open()`(belief)で書く経路がある**(ADR-098 決定1-b の「belief を actuation の記録として書く」誤用と同型)。`runtime/mod.rs:1409`(`process_deferred_keys`)は `effective_open()` の値をそのまま `record_confirmed` している。コメントに「本番到達不能」とある。`ime_refresh.rs:618` も `effective_open()` を `Confirmed` として書く(TsfNative は除外)。
2. **`await` をまたぐ失効が、フィールド値の一致比較で手書きされている**。`key_pipeline.rs:425`(spawn 時の `last_explicit_ime_action_ms_raw`)と `:664`(apply 時に比較)。`ApplyGeneration` や `PressId` のような世代ではなく、`TickMs` の値そのものを世代に使っている。`note_explicit_ime_action` は同じ ms に 2 回呼ばれると変化を検出できない(同一 tick 内の複数書き込みは未確認)。
3. **「問い合わせ」が状態を変える**。`mode_key_pass_mark_live`・`mode_key_pass_window_remaining_ms`・`mode_key_pass_expiry_wait_ms`・`external_change_watch_remaining_ms` は `&mut self` で、`ScopedOneShot::peek` が scope 不一致のとき `armed=None` にする。呼び出し時の OS の前面ウィンドウ(`foreground_scope()`)に結果が依存する。
4. **`apply_panic_reset` は 2 つの Event の間に 5 種の直接書き込みを挟む**。Event 順序と直接書き込みの順序が意味を持つが、journal には Event だけが残る。
5. **`ImeModel` の `pub` フィールドが `model()` 経由で読まれる**。`model()` は `&ImeModel` を返すので外から書けないが、`ImeStateHub` のメソッドは `shadow_model.<field>` を直接書ける。`ime-belief-architecture.md` の「`reduce()` が唯一の書き込み点」は、`desired_open`/`input_mode` など private の 8 フィールドにだけ当てはまる。
6. **`ImeModel.current_focus` と `observations.current_fence.hwnd` が別々に更新される**(`FocusHwndUpdated` は後者だけ)。`intent_store` の hwnd と `AcceptedObservation` の照合がそれぞれ違う方を読む。
7. **`DriftDetected` が `applied=Optimistic` を書いた後、`ir_apply_drift_correction` が `set_ime_open_ordered` の戻り値によって `record_optimistic` を重ねて呼ぶ**(ime_refresh.rs:972-993)。書き込みを拒否された(`Unwarranted`)場合でも、reduce 側の `Optimistic(desired)` は残る。ADR-090 A-2 のコメントが「送っていないのに送った体で記録する」欠陥と警告しているのと同じ構造(reduce 側の分は未検証)。
8. **ADR-119/180 との関係**: `record_confirmed(false)` を「actuation の前に」書く経路が `key_pipeline.rs:1290` と `open_chain.rs:477` にある(ADR-098 決定5/6-a)。`ImeStateHub` の書き込みが `await` の前に済み、完了時の reduce(`ImeApplySucceeded`)と二重になる。
9. **`event_log` が本番で読まれない**。512 件のリングへ毎回 clone して積む(`dispatch_event` は event を 2 回 clone し(journal 用・reduce 用)、元の値を event_log へ move する)。

### 未確認の点

- `ImeEvent` が含む型(`FocusFence`・`ApplyGeneration`・`KeyTrack`・`InputModeState`・`TickMs`)に `Deserialize` が付いているか。
- `ForceGuard.generation` を読む本番コード(`git grep` では見つからなかった)。
- `probe_admission.rs` の棄却カウンタ(static/atomic)は G 分類だが、この領域の外なので調べていない。
- `FocusStore`・`GateStore`・`KeymapStore`(`platform_state.rs` の後半)は調べていない(担当外)。
- 読み手の数は、ファイル別の行ヒット数で出した粗い値。関数単位の読み手数は未確認。
- `note_explicit_ime_action` を同じ ms に複数回呼ぶと、`await` またぎの一致判定が見逃すかどうか。
- `dispatch_event` の journal 記録が reduce の後であることによる、reduce 中に panic した場合の記録の欠落(未確認)。
