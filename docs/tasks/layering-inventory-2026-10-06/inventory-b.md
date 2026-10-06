# 棚卸し B: state/platform_state.rs / ime_decision_view.rs / (state/ の cfg(windows) 一式) / runtime/{open_chain,executor,ime_refresh}.rs

対象: develop 9df983e4。読み取り専用。行数は `wc -l` と範囲指定で自分で数えた(行範囲は doc コメント込み、空行は含まない集計)。ビルド・テストは実行していない。
表記: P / P* / O / E / F / G / T は共通指示どおり。「未確認」は読んでいない、または実行しないと分からないもの。

---------------------------------------------------------------------------
## 0. 結論(最重要の成果物: platform_state.rs は本当に Windows に依存しているか)

**ほぼ依存していない。** `state/platform_state.rs` の本体 1813 行に、Win32 API の呼び出し・HWND・HIMC・COM・UIA・MSAA・TSF・SendInput・SetTimer・thread_local・`with_app`・`unsafe` は **0 件**(grep と全関数の通読で確認)。`windows::` のトークンも 0。`HwndId` は `usize` の newtype(ungated の `state/ime_event.rs`)。

ファイル全体を `#[cfg(windows)]` にしている**実際の原因は、次の 6 個の継ぎ目だけ**(いずれも「Win32 を呼ぶ」ためではなく、「型や関数がたまたま gated なモジュールにある」ため):

| # | 継ぎ目 | platform_state.rs での出現 | 実体 | ungate に必要な作業 |
|---|---|---|---|---|
| S1 | `crate::win32::ForegroundScope`(型)と `crate::win32::foreground_scope()`(関数) | 計 19 件 = 型 8(L86, L92, L329, L337, L363, L389, L540, L1691)+ 関数呼び出し 11(L308, L317, L324, L348, L356, L379, L418, L432, L456, L532, L562) | 型は `win32.rs:46-62` の `{pid:u32, hwnd:isize}` で**純粋**。関数だけが `GetForegroundWindow` + `GetWindowThreadProcessId`(`focus::classify::get_window_process_id`) を呼ぶ(O) | 型を ungated へ移す(win32.rs は re-export)。`foreground_scope()` は `ImeStateHub` に関数ポインタで注入(`HubClock::wall(fn)` と同じ流儀)。本体側は既に `_in_scope(scope)` 版の二重化があり(L327, L334, L359, L385, L536)、判断は `ModeKeyPassLatch<S>`/`ExternalChangeWatch<S>`/`ScopedOneShot<S,_>`(全部 ungated で `S` ジェネリック)に入っているので、型を差し替えるだけ |
| S2 | `crate::hook::current_tick_ms` | 1 件(L137、`HubClock::wall(crate::hook::current_tick_ms)`) | `GetTickCount64`(`hook.rs:801`) | `ImeStateHub::new()` が tick 関数を引数に取る。`PlatformState::new()` の呼び出しは `app/bootstrap.rs:706` の 1 箇所+テスト |
| S3 | `crate::journal::{JournalEntry, UnifiedJournal}` | import 1 + 使用 3 メソッド(`dispatch_event` L195、`claim_press_write` L231、`release_press_write` L249)+ フィールド `journal`(L40, L138) | `journal.rs`(2131 行)は丸ごと gated だが、`windows::`/HWND は 0。Win 依存は **(a) `crate::win32::SentKeyEvent`(`journal.rs:216-217`、`win32.rs:262` の純粋な 5 フィールド構造体)、(b) `dump_to_file`/`dump_to_file_for_report`(`journal.rs:1511-1570` 付近、`hook::current_tick_ms` + `std::fs::write` + `env::temp_dir`)** の 2 つだけ。`quanta` は ungated 依存 | `SentKeyEvent` を移し、dump 2 関数は tick を引数化するか Windows 側拡張へ(約 60 行)。journal.rs は別担当の領域の可能性が高い(未確認)が、platform_state.rs の ungate のブロッカー |
| S4 | `crate::observer::ime_observer::ImeUpdate`(+ `ImeObs`) | 1 件(`apply_ime_update`, L1191) | 純粋な 6 フィールドのレコード(`observer/ime_observer.rs:22-45`)。ファイルが gated(中に `unsafe poll_and_classify_ime` がある) | 構造体 2 つを `state/` へ移す(`classify_fetched_snapshot` は `crate::ime::ImeSnapshot`〈ime.rs、中身は純粋な Option 群〉を取るので、そちらも別 seam) |
| S5 | `crate::focus::hwnd_cache::HwndImeSnapshot` | 1 件(`apply_hwnd_cache_restore`, L1247) | `focus/hwnd_cache.rs` は `HashMap` + `tuning::HWND_CACHE_MAX_AGE_MS` だけ(`windows::` 0)。focus/mod.rs で個別に gated | ファイルの gate を外すだけ |
| S6 | `state/mod.rs` の gate 3 組 | `platform_state`+`PlatformState` re-export(L187-190)、`ime_decision_view`(L192-195)、`ime_event_log`(L202-203) | `ime_event_log.rs` は gate の理由が実質なし(使う側が platform_state だけ) | `#[cfg(windows)]` を外す(+dead_code 抑制) |

加えて**ungate してもテストから呼べない障害が 1 つ**: S7 可視性。`ImeStateHub`/`ImePollState`/`FocusStore`/`GateStore`/`KeymapStore` は `pub(crate)`、`PlatformState.ime/focus/gate/keymap` フィールドも `pub(crate)`。`tests/` の統合テスト(= `tests/support/harness.rs`)は外部クレート扱いなので、`pub(crate) fn` 75 個 + 非公開 `fn` 11 個(うち `note_awase_write_for_mode_key_pass` 等)の公開範囲を決める必要がある(別 crate 化するなら全部 `pub` になる)。`ImeModel` の `desired_open`/`input_mode` は private のまま(モジュール境界は変わらないので belief 規約は保たれる)。

ついでに見つけた**時計の二重化**(S8): `ImeStateHub` は `clock: HubClock`(実時計/手動時計)を持ち `effective_open`・`dispatch_event` はそれを読むが、周辺が時計を直接読んでいる — `runtime/ime_refresh.rs` に `hook::current_tick_ms` 13 件 + `Instant::now` 2 件、`runtime/executor.rs` に 3+1 件(`issue_self_actuation_order` L99-100 と `update_intra_batch_applied` L920)、`runtime/mod.rs:561-567`(`issue_actuation_order_with_origin`、他担当)。core に出す関数がこれらを呼ぶ限り、仮想時計では動かない(ハーネスが仮想時間を使っているのに写しが必要だった一因、`state/hub_clock.rs` の doc 冒頭に同趣旨)。

### 0.1 harness.rs が「手で写している配線」と ungate 後の対応

harness.rs 冒頭 doc(L13-26)の列挙は 7 項目(+ 本体で写している 1 つ)。ungate 後にどう本物の呼び出しになるか:

| # | harness の写し(harness.rs) | 写し元 | ungate 後に呼ぶ本物 | 追加で要る seam |
|---|---|---|---|---|
| 1 | `apply_key_effect_prediction`(L598-612) | platform_state.rs:266-297 | `ImeStateHub::apply_key_effect_prediction(prediction, tick)`。内部で `dispatch_event`(= `event_log.record_at` + reduce + journal)を通るので、ハーネスの `reduce()`(L547-557、`EventTime` を手組み)は不要になる | S2(手動時計で構築)、S3、S7 |
| 2 | `effective_open`(L562-567) | platform_state.rs:837-891 | `hub.effective_open_at(TickMs)`(`hub.clock.now_instant()` を読む)。ハーネスの `base + now_ms` と同じ仮想時計になる(`HubClock::manual`) | S2、S7。S8 は不要(hub 内で完結) |
| 3 | `warrant_for`(L615-627)+ `issue_write`(L632-648) | platform_state.rs:986-1043 | `hub.warrant_context(now, now_ms)`・`hub.issue_actuation_order(open, origin, now, now_ms)`。**現状の写しは `issue_open_warrant` を直接呼び、本物は `ActuationOrder::issue(open, target, &ctx, origin)`(INV-47)を呼ぶ**——ungate すると `ActuationOrder` と `would_have_blocked()` を検査対象にできる | S7 |
| 4 | `user_set_open`(L394-408)の `reduce(UserImeSetIntent)` + `intents.record`、`observe_value`(L326-373)の `Observed::<…>` 手組み + `InputModeObserved` | platform_state.rs:623-671(`handle_engine_set_open`)、1417-1608(`write_*`・`report_conv_open_inference`)、1183-1240(`apply_ime_update`) | `hub.handle_engine_set_open`(chord フィルタ・`ImeApplyRequested`・`last_explicit_ime_action_ms` まで含む)、`hub.write_physical_key`/`write_observer_poll`/`write_imm_cross_probe`/`report_conv_open_inference`/`apply_ime_update`。ハーネス doc が「写していない」とする `ImeApplyRequested`/`applied` の往復・ForceGuard もここで本物になる | S4(`apply_ime_update` を使う場合)、S7 |
| 5 | `key()` の `PredictInput` 組み立て(L286-318) | `runtime/key_pipeline.rs::kp_stage_key_effect_track`/`kp_predict_key_effect` | **この領域外**(key_pipeline は別担当)。hub 側で要るのは `hub.model().key_track()`・`hub.effective_open()`・`hub.model().input_mode()` だけで、これらは platform_state.rs 内(P)。写しを消すには key_pipeline 側の純粋核の切り出しが必要(未確認) | key_pipeline 側 |
| 6 | `settle()` 内の drift 検知まで(L688-706) | runtime/ime_refresh.rs:728-797 | **ここは seam では済まない(F)**。`check_drift_correction` は既に ungated の `drift_correction.rs`。残りの「engine 有効 && 日本語 IME → check → settle 待ち → `actuation_for` → 授権が下りるか(ImmCross のみ)」と、ハーネスが「写していない」Blind/Read の再送打ち切り・settle 待ち(ime_refresh.rs:801-948)は `impl Runtime` の中。§3.3 の分割案を参照 | F の分割(§3.3) |
| 7 | `external_injected_key`(L426-436)・`prefetch_read`(L440-475) | platform_state.rs:415-474 | `hub.arm_external_change_watch(now_ms)`・`hub.follow_external_change(read, now_ms, tick, accepted)`。ハーネスは `ExternalChangeWatch<u32>`(スコープ型を u32 に差し替え)を直接使っているので、本物にするには scope の注入(S1)が前提 | S1、S7 |
| 8(本体で写す) | `settle()` 内の `ir_align_placeholder_desired`(L673-684) | ime_refresh.rs:683-708 + platform_state.rs:500-518 | `hub.align_placeholder_desired(now, tick)`(既に hub の中、P)。ime_refresh 側のガード(`engine.is_user_enabled() && belief.is_japanese_ime()`)は 2 つの bool の AND なので、呼び出し側のテストコードに残しても差は小さい | S7 |

つまり **#1,2,3,4,7,8 は platform_state.rs の ungate(S1〜S7)だけで本物になる**。#5 は別領域、#6 は ime_refresh.rs の F 分割が必要で ungate では解決しない。

---------------------------------------------------------------------------
## 1. ファイルごとの内訳

### 1.1 `state/platform_state.rs`

1. 行数: 全体 3313 / 本体 1813(L1-1813。うち L1610-1623 は `#[cfg(test)] impl ImeStateHub`〈テスト用 setter 2 個〉14 行) / テスト 1500(L1814-3313、`#[test]` 48 本)。gate 原因: `state/mod.rs:187-190`(`#[cfg(windows)] pub mod platform_state; pub use platform_state::PlatformState`)。ファイル内に `#[cfg(windows)]` は 0 件(コメント中の言及 2 件のみ)。
2. 分類別本体行数: P 1208(imports L1-18・空行を含む)/ P* 591 / O 0 / E 0 / F 0 / G 0 / テスト用ヘルパ 14(T 扱い)。
3. 関数表(全項目、行範囲は doc 込み):

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| struct `ImeStateHub` | 24-113 | 90 | P* | S1(`ModeKeyPassLatch<ForegroundScope>`・`ExternalChangeWatch<ForegroundScope>`)、S3(`journal: UnifiedJournal`) |
| struct `ImePollState` | 115-129 | 15 | P | |
| `new` | 131-151 | 21 | P* | S2(`HubClock::wall(hook::current_tick_ms)`) |
| `dispatch_event` | 153-198 | 46 | P* | S3(`journal.record`)。belief への唯一の入口(`reduce`)なので最優先で core に置く |
| `claim_press_write` | 200-239 | 40 | P* | S3(journal)。ADR-208 L1 |
| `release_press_write` | 241-257 | 17 | P* | S3 |
| `explicit_intent` | 259-264 | 6 | P | |
| `apply_key_effect_prediction` | 266-303 | 38 | P | harness #1。dylint `ime_event_guard` の designated 関数 |
| `arm_mode_key_pass_mark` | 305-309 | 5 | P* | S1(関数) |
| `mode_key_pass_expiry_wait_ms` | 311-320 | 10 | P* | S1 |
| `note_awase_write_for_mode_key_pass` | 322-325 | 4 | P* | S1 |
| `note_awase_write_for_mode_key_pass_in_scope` | 327-332 | 6 | P* | S1(型のみ) |
| `mode_key_pass_mark_live_in_scope` | 334-341 | 8 | P* | S1(型のみ) |
| `mode_key_pass_window_remaining_ms` | 343-351 | 9 | P* | S1 |
| `mode_key_pass_mark_live` | 353-357 | 5 | P* | S1 |
| `invalidate_intents_if_mode_key_pass_live_in_scope` | 359-366 | 8 | P* | S1(型のみ) |
| `expire_mode_key_pass_mark` | 368-382 | 15 | P* | S1 |
| `drop_intents_for_mode_key_pass_in_scope` | 384-413 | 30 | P* | S1(型のみ)。`PassEffect` の適用 |
| `arm_external_change_watch` | 415-422 | 8 | P* | S1。harness #7 |
| `external_change_baseline` | 424-427 | 4 | P | |
| `external_change_watch_remaining_ms` | 429-436 | 8 | P* | S1 |
| `last_external_change_ms` | 438-441 | 4 | P | |
| `follow_external_change` | 443-474 | 32 | P* | S1。harness #7。`AcceptedObservation` を引数に取る(P) |
| `pass_through_observed` | 476-492 | 17 | P | `ModeKeyPassedThrough` の唯一の dispatch 元(dylint designated) |
| `desired_is_placeholder` | 494-498 | 5 | P | |
| `align_placeholder_desired` | 500-518 | 19 | P | harness #8 |
| `align_after_expired_mode_key_pass` | 520-534 | 15 | P* | S1 |
| `align_after_expired_mode_key_pass_in_scope` | 536-552 | 17 | P* | S1(型のみ) |
| `invalidate_intents_if_mode_key_pass_live` | 554-564 | 11 | P* | S1 |
| `record_optimistic` | 566-592 | 27 | P | architecture_guard が呼び出し数を固定(INV-A97-1)。S1 の関数は `note_awase_write_for_mode_key_pass` 経由で間接依存 |
| `record_confirmed` | 594-602 | 9 | P | 同上 |
| `clear_pending_if_matches` | 604-613 | 10 | P | |
| `is_ctrl_ime_chord_active` / `active_chord_kind` | 615-621 | 7 | P | |
| `handle_engine_set_open` | 623-671 | 49 | P | harness #4 |
| `on_ctrl_key_up` | 673-691 | 19 | P | |
| `consume_focus_barrier` / `clear_input_barrier` | 693-706 | 14 | P | |
| `try_set_focus_transition_barrier` | 708-725 | 18 | P | `Instant` を引数で受ける |
| `note_explicit_ime_action` / `explicit_ime_action_age_ms` / `last_explicit_ime_action_ms_raw` / `persistent_explicit_off_ms` | 727-770 | 41 | P | |
| `effective_open` | 772-835 | 64(コード 2) | P | `self.clock.now_tick()` |
| `effective_open_at` | 837-891 | 55 | P | harness #2。`Cell<bool>` のログ重複排除(`!Sync` 前提) |
| `is_focus_transition_settling` / `detect_miss_count` / `is_force_on_guard_active` / `desired_open` / `input_mode` / `applied_state` | 893-924 | 34 | P | |
| `capture_poll_state` | 926-936 | 11 | P | |
| `focus_settle_ms` / `default_feedback` / `allocate_event_generation` / `correction_for_imm_broken` / `model` | 938-984 | 47 | P | |
| `warrant_context` | 986-1020 | 35 | P | harness #3 |
| `issue_actuation_order` | 1022-1043 | 22 | P | harness #3(ADR-090 A-1) |
| `check_drift_correction` | 1045-1061 | 17 | P | 本体は ungated な `drift_correction.rs` |
| `record_ime_apply_result` | 1063-1109 | 47 | P | `ime_model::apply_result_effective_open`(ungated)を使用 |
| `reset_detect_state` / `on_ime_toggled` / `release_panic_reset_guard_on_positive_evidence` / `on_set_open_requested` | 1111-1139 | 25 | P | |
| `apply_panic_reset` | 1141-1181 | 41 | P | dylint designated |
| `apply_ime_update` | 1183-1240 | 58 | P* | S4(`ImeUpdate`) |
| `apply_hwnd_cache_restore` | 1242-1299 | 58 | P* | S5(`HwndImeSnapshot`)。dylint designated |
| `reset_stale_ime_on_for_imm_broken` | 1301-1361 | 61 | P | |
| `assume_closed_for_new_thread` | 1363-1393 | 31 | P | |
| `set_is_japanese_ime` / `observe_layout_language` / `set_prev_conversion_mode` | 1395-1415 | 21 | P | |
| `write_observer_poll` | 1417-1429 | 13 | P | |
| `write_sync_key` / `record_explicit_intent` / `write_physical_key` / `write_set_open_request` | 1431-1516 | 83 | P | harness #4 |
| `write_focus_probe` / `write_imm_cross_probe` / `report_conv_open_inference` | 1518-1608 | 89 | P | |
| `#[cfg(test)] impl ImeStateHub`(テスト用 setter 2) | 1610-1623 | 14 | T | |
| struct `FocusStore` + `new`/`Default` | 1625-1677 | 49 | P | `AppKind`/`FocusKind` は ungated(`focus/kinds.rs`) |
| struct `GateStore` | 1679-1722 | 44 | P* | S1(`post_bypass: ScopedOneShot<ForegroundScope, PostBypassArm>`) |
| struct `PostBypassArm` + const `IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS` | 1724-1736 | 12 | P | |
| `GateStore::new`/`Default` | 1738-1759 | 20 | P* | S1 の型パラメータのみ |
| struct `KeymapStore` | 1761-1774 | 14 | P | `keymap::KeymapTable` は ungated |
| struct `PlatformState` + `new`/`Default` | 1776-1812 | 37 | P | |

4. 移せる行数(P+P*)1799 / 残る行数(O+E+G)0 / 分割が要る行数(F)0。S1〜S7 を直せば**全量移せる**。
5. Windows 側依存の種類と件数(本体 L1-1813): `win32::ForegroundScope` 型 8、`win32::foreground_scope()` 呼び出し 11、`hook::current_tick_ms` 1、`journal`(`JournalEntry`/`UnifiedJournal`)import 1+使用 3 メソッド、`observer::ime_observer::ImeUpdate` 1、`focus::hwnd_cache::HwndImeSnapshot` 1。HWND/HIMC/COM/UIA/MSAA/TSF/SendInput/SetTimer/thread_local/`with_app`/`unsafe`: 各 0。その他の `crate::` は ungated(`tuning` 8、`state::*`、`keymap::KeymapTable`、`IME_DETECT_MISS_THRESHOLD`)。`tracing::` 16。
   テスト 1500 行: `crate::win32::ForegroundScope` の構造体リテラル 4、`crate::focus::hwnd_cache` 2、`crate::tuning` 10、他は ungated。S1・S5 が直れば Linux でそのまま回せる(現状は `windows-build` CI のみで実行)。

### 1.2 `state/ime_decision_view.rs`

1. 行数: 全体 153 / 本体 153 / テスト 0。gate 原因: `state/mod.rs:192-195`。ファイル内に `#[cfg(windows)]` は 0。
2. 内訳: P 49(`FocusFacts` L19-33、`ControlLog` L96-118、`From<&ImeControlView> for DecisionInputs` L143-153)/ P* 61(`ObservedState` L35-61 と `Default` L63-74 は `crate::tsf::observer::ActiveImeKind` を持つ、`ImeControlView<'a>` L120-141 は `class_name: &'a str` の借用)/ O 19(`ObservedState::from_snapshot` L76-94、`tsf::observer::TsfObservations` と `candidate_was_seen()` グローバルを読む)/ 残り 24 は doc・import。
3. 関数表: 型 4 つ + `from_snapshot` + `Default` + `From`(上記のとおり)。
4. 移せる 110(P+P*)/ 残る 19(O、`from_snapshot` を Windows 側へ)/ F 0。
5. 依存: `tsf::observer::ActiveImeKind`(`tsf/observer.rs:710-715` の 2 バリアント enum、純粋)、`tsf::observer::TsfObservations`/`candidate_was_seen`(O、1 関数)。借用 `FocusFacts.class_name: &'a str` と `ImeControlView<'a>` のライフタイム。
   **これが ADR-180・BUG-34 周辺の「`&ImeControlView<'_>` 借用」の正体**: ビューは `Copy` だが、`FocusFacts<'a>` が `Runtime.platform.focus` の class_name 文字列を借りるため、`with_app` の外へ持ち出せない(`open_chain.rs:424-436` のコメント)。seam は `class_name: &'a str` → `Arc<str>`/小さな所有型(ログ専用フィールドなので意味は変わらない)、`ActiveImeKind` → ungated へ移動(`ImeKindId` に `From` が既にある)。使用箇所: `ImeControlView` 参照は `ime_controller.rs` 16+19、`runtime/transport.rs` 38、`tsf/gji_monitor.rs` 30、`key_pipeline.rs` 11 など(他担当領域)。

### 1.3 `state/ime_event_log.rs`

1. 全体 177 / 本体 113 / テスト 64(5 本)。gate 原因: `state/mod.rs:202-203`。中身は `VecDeque`・`Instant`・`ime_event::*`・`TickMs` だけで Windows 依存なし。
2. 内訳: P 113。
4. 移せる 113 / 残る 0 / F 0。seam: gate を外すのみ(S6)。

### 1.4 `state/mod.rs`(gate 状況の確認)

全体 203 行。`#[cfg(windows)]` は 6 組: L32-35(`ConvModeMgr`・`ConvActuationOutcome`・`ConvModeTarget`・`ConvMutationReason` の re-export)、L154-155(`AppliedImeState` re-export)、L187-190(`platform_state`/`PlatformState`)、L192-195(`ime_decision_view` と re-export)、L202-203(`ime_event_log`)。re-export の gate(L32-35, L154)は dead_code 回避だけで、`conv_mode.rs`(409 行)にも `ime_model.rs` にも Win 依存はない(`windows::`/`win32`/`hook`/`imm` の grep 0)。`#[cfg_attr(not(windows), allow(dead_code))]` は 18 箇所(呼び出し元が gated な ungated 純粋モジュールの印)。
state/ 配下でファイル全体が gated なのは **platform_state.rs / ime_decision_view.rs / ime_event_log.rs の 3 つだけ**。

### 1.5 state/ の他の `#[cfg(windows)]`(インライン、全部列挙)

| ファイル:行 | 項目 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| `ime_event.rs:36-44` | `impl HwndId { to_hwnd() }` | 9 | E 近傍(型変換の殻) | `windows::Win32::Foundation::HWND` を作る。Windows 側の拡張 trait(`HwndIdExt`)へ |
| `ime_event.rs:46-51` | `impl From<HWND> for HwndId` | 6 | 同上 | |
| `key_effect_runtime.rs:377-381` | `table_file_path` | 5 | O | `crate::app::find_config_path()` |
| `key_effect_runtime.rs:385-389` | `last_attempt_file_path` | 5 | O | 同上 |
| `key_effect_runtime.rs:393-404` | `table_file_stamp` | 12 | O | `fs::metadata` |
| `key_effect_runtime.rs:408-432` | `load_and_log` | 25 | O+ログ | 純粋核 `load_runtime_table(path, fingerprint)` は ungated で既存 |
| `key_effect_runtime.rs:440-454` | `current_fingerprint_probe` | 15 | O | `gji_charset_autodetect`/`msime_key_assignment`(レジストリ) |
| `key_effect_runtime.rs:816-833` | `RuntimeTableCache::get_for_keymap` | 18 | P* | `get(now, key, stamp_fn, load_fn)` が既に関数注入型。`table_file_stamp`/`load_and_log` を渡すだけなので、関数ポインタ引数化で移せる |
| `key_effect_predictor.rs:570-577` | `get_gji` | 8 | P* | 同様に `stamp`/`read` クロージャ注入 |
| `key_effect_predictor.rs:580-587` | `get_native` | 8 | P* | 同上 |
| `probe_admission.rs:325-350` | `admit_epoch_in_app<R>(app: &mut Runtime, …)` | 26 | F | `app.focus_fence()`(P*)と、棄却時の `focus::classify::root_hwnd_of`(`GetAncestor`、O)+ `platform.focus.current.root_hwnd` を読んで `record_hwnd_mismatch` を数える計測。分割案: 判定 `ticket.admit(current)` は既に純粋(`Admission`)。`same_root` 計測の入力 2 つ(spawn 側 root、現在 root)を引数化して純関数にし、`&mut Runtime` を取る殻を Windows 側に残す |

---------------------------------------------------------------------------
## 2. runtime/ 3 ファイル

3 ファイルとも `crates/awase-windows/src/runtime/mod.rs` の `#[cfg(windows)]` モジュールツリー配下(lib.rs の `#[cfg(windows)] pub mod runtime;`)。本体の `#![allow(unsafe_code)]` を宣言。

### 2.1 `runtime/ime_refresh.rs`

1. 行数: 全体 1119 / 本体 1119 / テスト 0。gate: `lib.rs` の `#[cfg(windows)] pub mod runtime;`(+ `mod ime_refresh;` は `runtime/mod.rs` で非 pub)。コード行(コメント・空行除く)713。全関数が `impl Runtime` の中(`Runtime` 自体が Windows 型を多数持つ巨大 struct)。
2. 分類別: P 19 / P* 72 / O 0(O は分割後に出る)/ E 0(同)/ F 900 / G 95 / その他(import・doc)10。
3. 関数表:

| 名前 | 行範囲 | 行数 | 分類 | メモ(P* は seam、F は分割案) |
|---|---|---|---|---|
| enum `IoMode<'m>` | 10-22 | 13 | P* | `focus::probe::FocusSnapshot`(`HWND` を返す `hwnd()` を持つ、中身は `usize`+pid+class_name)と `ime::ImeSnapshot`(純粋な Option 群)を持つ。seam: 両型の gate を外す |
| enum `ImeReadStrategy` / struct `FocusInfo` | 24-43 | 19 | P | |
| `run_ime_refresh` / `run_ime_refresh_with_prefetched` | 45-61 | 17 | G | 入口: タイマー(`TIMER_IME_REFRESH`)・prefetch 完了メッセージ → core Event「ImeRefreshTick{focus: Option<FocusProbe>, ime: Option<ImeSnapshot>}」 |
| `ir_execute` | 63-89 | 27 | G | 段階の直列化。`focus.app_disabled` の早期 return は純粋判定 |
| `ir_stage_focus` | 91-133 | 43 | F | `detect_and_update_focus`(unsafe、O)/`apply_focus_probe_result` を呼ぶ→ core 側: 「フォーカス変更後の後始末」(`kana_lock_hysteresis` リセット・`drift_giveup_*` クリア・トレイ表示解除)は純粋な状態更新+E(トレイ)。O の結果(`focus_changed: bool`)を Event で受ける形に |
| `ir_stage_strategy` | 135-144 | 10 | G | 委譲 |
| `ir_stage_observe` | 146-255 | 110(コード 96) | F | 戦略 `match` が core の判断(SkipTyping/Blacklist/OsPoll)。Blacklist 枝の `tsf_obs().active_ime_kind()==GJI`・`observe_gji_after_focus`(O)・`write_observer_poll`・`dispatch_event(InputModeObserved{GjiIoInference})`、OsPoll 枝の `ir_poll_and_learn` と通過マーク追随(`invalidate_intents_if_mode_key_pass_live`→`schedule_ime_refresh`)。分割: O = `{active_ime_kind, gji_io_observation}` の取得、core = 戦略ごとの hub 呼び出しと「次の refresh の予約要求」(E) |
| `ir_follow_external_change` | 257-279 | 23 | F | `external_change_watch_applies()`(= profile==Imm32Unavailable && `tsf_obs().active_ime_kind()==GJI`、`runtime/mod.rs:598`)を読み、`hub.follow_external_change` を呼ぶ。判定は 2 値の AND なので core 側で「入力: profile, active_ime_kind」 |
| `ir_follow_after_literal_giveup` | 281-331 | 51 | F | `giveup_follow_decision`(ungated 純粋)を呼び、`platform.output.ime_mode_focus_gen.get()`(O)・`schedule_ime_refresh`(E)・`journal.record(GiveUpFollow)`。分割: core = `decision` 以降の arm と journal、E = 予約 |
| `ir_stage_notify` | 333-360 | 28 | G | Phase 4〜5 の直列化(`expire_mode_key_pass_mark` の判定は hub 内) |
| `ir_resolve_skip_imm_query` | 362-366 | 5 | P* | `!profile.can_use_imm32_cross_process()`(`Runtime::can_use_imm32_cross_process`)。seam: profile を引数に |
| `ir_notify_focus_changed` | 368-417 | 50 | F | `discard_actuation`(`active_actuation=None`、`Actuation` 構造体 `runtime/ime_actuation.rs:25` は純粋)・`half_width_alnum.is_toggle_active`→`kp_restore_kana_from_half_width`(E)・`correction_for_imm_broken`(hub、P)→`apply_input_mode_correction`・`build_ctx`+`engine.on_command(FocusChanged)`+`execute_decision`(G)。core に出せる条件式(`skip_imm_query && effective_open && !romaji_capable`)が 3 行ある |
| `ir_decide_read_strategy` | 419-470 | 52(コード 36) | F | **最もきれいに P* へ変えられる**: 入力は `last_hook_activity_ms`・`OUTPUT_GATE.last_vk_output_ms`(AtomicU64、O)・`current_tick_ms`・`mode_key_pass_mark_live`・`explicit_intent`・`applied`・`half_width_alnum` の 2 述語・`skip_imm_query`。`fn decide_read_strategy(inputs) -> ImeReadStrategy` にして Atomic と時計は呼び出し側が渡す(S8) |
| `ir_poll_and_learn` | 472-540 | 69 | F | `poll_and_classify_ime`(**unsafe IMM、O**)か `classify_fetched_snapshot`(純粋)で `ImeUpdate` を得て、ImmCross 時の `ObservedKana` 抑制(純粋条件)→`hub.apply_ime_update`→`learn_imm_capability_from_miss`(Runtime)。core: 抑制規則と apply。O: 同期 poll だけ |
| `ir_log_poll_diff` | 542-586 | 45 | P* | 診断ログ専用。時計と hub 読み出し(S8)。移さず捨てる/ログ層へ、のどちらでも可 |
| `ir_post_focus_change_snapshot` | 588-681 | 94(コード 23) | F | `ime_diagnostic::ImeDiagnosticSnapshot::capture`(O)、`is_effectively_tsf_native`(P)、`record_confirmed`(hub)、`platform.mark_composition_cold_focus_change`/`gji_on_focus_change`/`drain_journal_entries`(E/O)。コード 23 行に対して**コメントが 71 行(BUG-34 追補4・ADR-098・ADR-212 P4 の歴史)**。分割時にこのコメントの置き場を決めること |
| `ir_align_placeholder_desired` | 683-708 | 26 | F | ガード 2 つ→`hub.align_placeholder_desired`(P)→`presync_applied_open_on`(`runtime/focus_tracking.rs:204`、E 側)。harness #8 |
| `ir_check_drift_correction` | 710-718 | 9 | P* | `hub.check_drift_correction(now, explicit_intent)` の薄い殻 |
| `ir_apply_drift_correction` | 720-1026 | **307**(コード 172) | F | 本ファイル最大。下の §3.3 参照 |
| `ir_notify_drift_giveup_diagnostic` | 1028-1102 | 75 | F | 閾値判定(`duration_ms >= DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS` && 未通知)は純粋→`show_tray_balloon`(E)+ 状態更新+`journal.record(DriftGiveUpDiagnostic)`(`platform.tray.current_layout_name()` は O) |
| `notify_engine_refresh` / `ir_notify_engine_refresh` | 1104-1119 | 15 | G | `build_ctx`+`engine.on_command(RefreshState)`+`execute_decision`。core Event「EngineRefresh」 |

4. 移せる(P+P*)91 / 残る(G)95 / 分割が要る(F)900(= ファイルの約 80%)。
5. 依存(件数): `hook::current_tick_ms` 13、`Instant::now` 2、`unsafe` 3(`detect_and_update_focus` 1、`poll_and_classify_ime` 1、コメント 1)、`self.platform.*` 13(`focus` 3、`tray` 2、`output` 2、`set_ime_open_ordered`/`build_ime_control_view`/`apply_ime_open_with_view`/`mark_composition_cold_focus_change`/`gji_on_focus_change`/`drain_journal_entries` 各 1)、`self.platform_state.ime` 33、`.gate` 4、`.focus` 2、`self.engine.*` 4、`schedule_ime_refresh`/`schedule_settle_retry`/`reschedule_ime_refresh` 6、`execute_decision` 2、`build_ctx` 3、`tsf::probe_bridge::OUTPUT_GATE`(Atomic)1、`tsf::observer::tsf_obs()` 2、`observer::{ime_observer,gji_observer}` 3、`with_app` 0、`thread_local` 0。

### 2.2 `runtime/open_chain.rs`

1. 行数: 全体 791 / 本体 721(L1-721、コード 403。L1-60 がモジュール doc、L61-70 が import)/ テスト 69(L723-791、`#[test]` 2 本。`record_actuation_decision_skipped` のカウンタ加算と `async_record` の `caller` 転記)。gate: `runtime/mod.rs` の `pub(crate) mod open_chain;`(`runtime` 全体が gated)。
2. 分類別: P 63 / P* 41 / O 0 / E 0 / F 533 / G 0 / doc・import 71(空行 13)。
3. 関数表:

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| static `ACTUATION_DECISION_RECORD_SKIPPED` | 72-73 | 2 | P* | `lifetime_counter`(ungated)のグローバル。seam: カウンタを呼び出し側から注入 |
| enum `ImmCrossOp` | 75-90 | 16 | P* | `Targeted{ target: ime::ActuationTarget(HWND を含む), conv_after_open: ime::ConvAfterOpen, focus_gen }`。seam: `ActuationTarget` を不透明トークン(`TargetToken`)に、`ConvAfterOpen` は ungated ミラー `conv_after_open::ConvAfterOpenId` が既存(`conv_after_open_id` L162 がその変換) |
| `ImmCrossOp::verified_target` | 92-99 | 8 | P | |
| struct `AsyncChainWriter` | 101-107 | 7 | P* | `ImmCrossOp` を保持(HWND 経由で `!Send`、`#[allow(clippy::future_not_send)]`) |
| `AsyncMechanismWriter::is_applicable` | 109-120 | 12 | P | |
| `AsyncMechanismWriter::write` | 122-149 | 28 | F | 機構ごとに `imm_cross_write`(await)か `fallback_write`(同期)へ振り分け。`AsyncMechanismWriter` trait 自体は ungated(`state/actuation_chain.rs`)で、走査規則(`run_chain_async`)は既に core 側 |
| `record_attempt` | 151-160 | 10 | P | |
| `conv_after_open_id` | 162-169 | 8 | P* | Win 型→ungated ミラー変換。ConvAfterOpen の gate を外せば不要 |
| `all_chain_record` / `async_record` | 171-204 | 33 | P | |
| `record_actuation_decision_skipped` | 206-213 | 8 | P* | グローバルカウンタ |
| `imm_cross_write` | 215-410 | 196(コード 120) | F | **下に詳述**(§3.1) |
| `fallback_write` | 412-555 | 144(コード 66) | F | §3.1 |
| `run_open_chain_async` | 557-721 | 165(コード 106) | F | §3.1 |

4. 移せる(P+P*)104 / 残る(G)0 / F 533。
5. 依存(件数): `crate::with_app` **7 箇所**(L245 view 読み、L319 focus_gen 再読、L467 view 再構築、L598 view 読み、L632/L670/L707 journal 記録)、`unsafe` 2(`read_ime_state_fast`)、`ime::set_ime_open_then_conv_for_target(...).await`/`ime::set_ime_open_cross_process_async(...).await`(IMM32 の `SendMessageTimeout`、E)、`ime::ActuationTarget`(HWND)、`tsf::observer::tsf_obs()` 1(ログ用 O)、`ime_controller::{apply_mechanism, mechanism_is_applicable, log_shadow_warrant}` 3(別担当)、`tracing::` 14、thread_local 0。

### 2.3 `runtime/executor.rs`

1. 行数: 全体 958 / 本体 923(L1-923、コード 583)/ テスト 34(L925-958、2 本、`AppliedImeState` の純粋アクセサのみ。`state/ime_model.rs` 側へ移せば Linux で回る)。gate: `runtime` 全体。
2. 分類別: P 91 / P* 192 / O 0 / E 36 / F 463 / G 93 / doc・import 27(空行 21)。
3. 関数表:

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| struct `ImeApplyCompletion` + alias `ImeApplyPair` | 28-42 | 14 | P | |
| struct `BatchResult` | 44-55 | 12 | P* | `callback: hook::CallbackResult`(`hook.rs:1079` の 2 値 enum、純粋)。seam: enum を ungated へ |
| `impl ImeStateHub { issue_self_actuation_order }` | 57-103 | 47(コード 14) | P* | 本来 platform_state.rs に置く内容。`Instant::now()`+`hook::current_tick_ms()` を直読み(S8)。`hub.clock` を使う形へ |
| struct `DecisionExecutor` / `Debug` / `new` | 105-143 | 37 | P | キューと guard 保持の純粋な状態 |
| `execute_from_hook` | 145-163 | 19 | G | hook 入口。`ime.model().applied` をスナップショット |
| `execute_from_loop` | 165-206 | 42 | G | WM ループ入口 |
| `drain_deferred` | 208-279 | 72 | F | reinject の guard 規則(park/`guard_held`/`reinject_guard_passed`)は純粋な状態機械だが、`reinject_wait_remaining(platform…)` と `platform.timer.{kill,set}` が混在。分割: core = `GuardDrain::step(queue, guard_held, wait_remaining) -> {Execute(effect) / Park{event,remaining} / KillTimer / Done}`、E = timer と effect 実行 |
| `on_output_guard_timer` | 281-289 | 9 | G | `TIMER_OUTPUT_GUARD` |
| `has_pending` | 291-294 | 4 | P | |
| `reinject_wait_remaining` | 296-324 | 29 | P* | `platform.has_pending_tsf_work()`・`platform.output_in_flight_ms()` の 2 値を読むだけ。seam: 引数化 |
| `park_in_guard` | 321-334 | 14 | F | `guard_held` 更新(P)+ `platform.timer.set`(E) |
| `enqueue_reinject` | 336-371 | 36(コード 3) | P | コメントに BUG-116 の訂正を含む |
| `execute_relay` | 373-455 | 83 | F | `Decision`×`PhysicalKeyDisposition` の場合分け(core の計画)+ `execute_one`(Timer を即実行する E)。分割: `plan_relay(decision, physical) -> RelayPlan{queue_extend, run_now_timers, callback}` |
| `run_passthrough_pipeline` | 457-532 | 76(コード 46) | P* | `platform.output_in_flight_ms()`・`has_pending_tsf_work()` を引数化。`PassthroughQueue`(`runtime/transport.rs`、`HashSet<VkCode>` のみ、`windows::`/unsafe 0 を確認)は純粋 |
| `execute_one` | 534-556 | 23 | G | |
| `handle_reinject` | 558-593 | 36 | E | `OutputActiveGuard::begin`・`win32_async::spawn_local`・`event.reinject()`(`SendInput`)・`platform.on_reinject_key` |
| `dispatch_effect` | 595-687 | 93(コード 79) | F | `Effect` の match が core の翻訳。`EngineStateChanged` の `conv authority` 切替(`AwaseOwned`/`UserOwned`)と `hook::set_engine_enabled`(グローバル Atomic、E)、`SendKeys` が通すモードキーか(`vk::is_followed_mode_key`、純粋)→`ime.arm_mode_key_pass_mark`+`platform.timer.set(TIMER_IME_REFRESH, 20ms)`(E)。分割: 「モードキー通過」判定と arm は core、timer と `platform_rt.*` は E |
| `dispatch_ime_set_open` | 689-889 | **201**(コード 122) | F | §3.2 |
| `update_intra_batch_applied` | 891-923 | 33 | P* | `hook::current_tick_ms()` 1(S8)。**`ime_model::apply_result_effective_open`(L110-124)と同じ写像(Applied/AppliedWithoutSendInput/AlreadyMatched→open、Failed→!open、残り None)を二重に持っている**ので、そちらを呼べば本体は 6 行に縮む |

4. 移せる(P+P*)283 / 残る(E+G)129 / F 463。
5. 依存(件数): `with_app` 実呼び出し **0**(コメント中の注意のみ。`ime: &mut ImeStateHub` を引数で受け取る設計)、`crate::hook::*` 3(`CallbackResult`、`set_engine_enabled`、`current_tick_ms` ×3)、`Instant::now` 1、`WindowsPlatform` 引数 11 関数、`win32_async::spawn_local` 2、`OutputActiveGuard::begin` 2、`ime::ActuationTarget::capture` 1、`ime_controller::ImeController::imm_cross_is_first_applicable` 1、`unsafe` 2(`event.reinject()` 他)、`tracing::` 16。

---------------------------------------------------------------------------
## 3. F の分割案(領域内の主要 5 本)

### 3.1 open_chain.rs(INV-45・ADR-089 C-4/§9-20・ADR-180・BUG-34 E-prep に直結)

共通の骨格: 「with_app で view を作る → `decide_gate`/`is_input_relay` で NotOwned を判定 → 書く → 結果を `ImeOpenOutcome` に写像 → `AttemptRecord` を作って journal へ」。

- core に出せる(P 化): ① gate 判定 `inputs → is_input_relay`(既に ungated `ime_actuation_decision`)、② `command`(`SetOpenThenConvForTarget`/`…Untargeted`)の組み立て(L261-269)、③ ImmCross 結果の写像(`Written→AppliedWithoutSendInput`、`Aborted→UnsafeToToggle`、`Failed`→再観測を入力に `imm_cross_reobservation_already_matches`〈ungated〉で `AlreadyMatched`/`Failed`、L340-395)、④ `fallback_write` の `shadow_on=None` 上書き(BUG-113 追補)と `mechanism_is_applicable` 判定、⑤ `AttemptRecord`/`ActuationDecisionRecord` の組み立てと journal 追記規則、⑥ `run_chain_async` の走査(既に core)。
- Windows に残す(E/O): `set_ime_open_then_conv_for_target(...).await`/`set_ime_open_cross_process_async(...).await`、`read_ime_state_fast()`(unsafe IMM 読み)、`ActuationTarget`(HWND)、`tsf_obs()` ログ、`apply_mechanism`(別担当の `ime_controller.rs`)。
- **危険(後述 §5 の D1〜D4 を必ず読むこと)**: view を「いつ」作るかの意味論を変えると INV-45・ADR-089 C-4 を壊す。

### 3.2 executor.rs::dispatch_ime_set_open

core: `plan_set_open(inputs: {applied_snapshot, belief_input_mode, open, press, is_tsf_native, view}) -> SetOpenPlan`:
 - `NotOwned{record}`(`decide_gate`、L713-761)、`Duplicate`(`claim_press_write`→`DUPLICATE_OUTCOME`、L767-776)、`Async{order, conv_after_open, applied_snapshot=Optimistic}`、`Sync{order}`。
 - 判定材料 `unknowns_applied`(`engine_press_unknowns_applied`)・`explicit_press_applied_pair`・`decide_dispatch_conv_after_open`・`imm_cross_is_first_applicable(&view)` は既に純粋関数。
 E: `platform.build_ime_control_view(...)`(O)、`OutputActiveGuard::begin`、`ActuationTarget::capture(focus_gen).await`、`spawn_local`、`post_async_ime_apply_complete`、`platform.apply_ime_open_with_view`(sync 側の実行、`ImeController::apply` は別担当の巨大関数)。
 順序の制約: `claim_press_write` は**order 発行の直前**(コメント L762-766)、`release_press_write` は sync で何も送らなかったときのみ(L871-874)、async は解かない(完了が後から来る)。これは `architecture_guard::press_id_is_claimed_and_carried_at_every_order_issuing_entry` が固定している 3 点(claim・`with_press`・`explicit_press_applied_pair`)と対。

### 3.3 ime_refresh.rs::ir_apply_drift_correction(307 行)

`architecture_guard.rs:1823-1906` が、この関数内の `match act_policy { … }` ブロックの文字列マーカーを走査して ADR-080 不変条件 6(GaveUp 時に observations へ書かない)を固定している(コメント L726 にも「ロジックの分割はしない」と明記)。分割するならガードも同時に付け替え。

core(純粋化できる): 入力 = {`engine.is_user_enabled`, `belief.is_japanese_ime`, `now`, `hub`(`check_drift_correction`・`default_feedback`・`model().observations.read_back`)、`ime_apply_should_defer`(= `is_focus_transition_settling`)、`Option<Actuation>`(純粋構造体)、`can_use_imm32_cross_process`、order の `would_have_blocked`}。出力 = `DriftPlan::{Skip, Defer{reason}, SkipUnwarranted, GiveUpNow{record}, ParkGaveUp, Rearm/Discard, Confirmed→Discard, Send{order, record, via: ImmCross|StrategyChain}}`。ハーネス #6 が写せなかった Blind/Read の再送打ち切り・settle 待ちがここに入る。
E(Runtime に残す): `schedule_settle_retry`、`set_ime_open_ordered`(ImmCross)、`build_ime_control_view`+`apply_ime_open_with_view`(Blacklist 経路)、`on_ime_apply_complete`(G)、`discard_actuation`/`advance_epoch` の適用(Runtime の `active_actuation` はそのまま core に移せる)。
注意: `Instant::now()`(L740)は S8 の対象。`journal.record`×3・`dispatch_event(DriftDetected)` は hub のメソッドで P。

---------------------------------------------------------------------------
## 4. 領域全体の集計(行数、doc 込み)

| ファイル | 全体 | 本体 | テスト | P | P* | O | E | F | G | doc/import/空行 |
|---|---|---|---|---|---|---|---|---|---|---|
| state/platform_state.rs | 3313 | 1813 | 1500 | 1208 | 591 | 0 | 0 | 0 | 0 | (P に含む。テスト用ヘルパ 14 は別) |
| state/ime_decision_view.rs | 153 | 153 | 0 | 49 | 61 | 19 | 0 | 0 | 0 | 24 |
| state/ime_event_log.rs | 177 | 113 | 64 | 113 | 0 | 0 | 0 | 0 | 0 | |
| state/mod.rs | 203 | 203 | 0 | モジュール宣言のみ(G 相当) | | | | | | |
| state/ のインライン gated 項目 10 個 | (計 158) | 158 | 0 | 0 | 34(`get_for_keymap`・`get_gji`・`get_native`) | 62(`table_file_path`・`last_attempt_file_path`・`table_file_stamp`・`load_and_log`・`current_fingerprint_probe`) | 15(`HwndId` の HWND 変換、E 近傍) | 26(`admit_epoch_in_app`) | 0 | |
| runtime/open_chain.rs | 791 | 721 | 69 | 63 | 41 | 0 | 0 | 533 | 0 | 71 |
| runtime/executor.rs | 958 | 923 | 34 | 91 | 192 | 0 | 36 | 463 | 93 | 27 |
| runtime/ime_refresh.rs | 1119 | 1119 | 0 | 19 | 72 | 0 | 0 | 900 | 95 | 10 |
| **合計(上 7 行 + インライン)** | 6714 | 5203 | 1667 | 1543 | 991 | 81 | 51 | 1922 | 188 | |

- 移せる(P+P*): 2534 行 / 残る(O+E+G): 320 行 / 分割が要る(F): 1922 行。
- **platform_state.rs 単体で P+P* が 1799 行(領域の移せる量の 71%)かつ F・O・E・G が 0**。この 1 ファイルの ungate が、領域内で最も安く最も効果が大きい。F の 1922 行は全て runtime/ の 3 ファイル(open_chain 533、executor 463、ime_refresh 900、probe_admission 26)で、`impl Runtime`・`with_app`・`&mut WindowsPlatform` に絡んでいる。
- テスト 1667 行のうち platform_state.rs の 1500 行(48 本)は S1・S5 が直れば Linux へ移して回せる。ime_event_log 5 本は gate を外すだけ。open_chain 2 本・executor 2 本は純粋アクセサ/カウンタなのでそのまま Linux で回る(コードが ungate された後)。

## 5. seam 一覧(領域共通、現れるファイル数・関数数)

| seam | 内容 | 現れる場所 |
|---|---|---|
| S1 ForegroundScope(型の移動+`foreground_scope()` の注入) | 純粋な 2 フィールド構造体を ungated へ、関数は hub に関数ポインタ注入 | platform_state.rs 1 ファイル・**18 関数/型**(型 8 + 関数呼び出し 11 件。同一関数に両方ある場合を除くと P* 関数 20 個)。他: `state/mod.rs` の `GateStore` 型パラメータ |
| S2 時計 tick の注入(`hook::current_tick_ms`) | `ImeStateHub::new(tick_fn)` | platform_state.rs 1 関数(`new`)。`PlatformState::new()` の呼び出し元 `app/bootstrap.rs:706` 1 箇所 |
| S3 journal の ungate | `SentKeyEvent` 移動 + dump 2 関数の tick/fs を分離 | platform_state.rs 3 関数+1 フィールド。journal.rs(別担当)の約 60 行 |
| S4 `ImeUpdate`/`ImeObs` の移動 | 純粋レコードを `state/` へ | platform_state.rs 1 関数(`apply_ime_update`)、`runtime/ime_refresh.rs` 1 関数(`ir_poll_and_learn`) |
| S5 `hwnd_cache.rs` の gate 解除 | 純粋ファイル | platform_state.rs 1 関数 |
| S6 `state/mod.rs` の gate 3 組解除 | `platform_state`/`ime_decision_view`/`ime_event_log` | 1 ファイル |
| S7 可視性 `pub(crate)`→`pub`(または crate 分割) | | platform_state.rs の 75+11 関数、struct 5、`PlatformState` の 4 フィールド。ハーネスが使うため必須 |
| S8 時計の直読みを `hub.clock` へ置換 | `current_tick_ms`/`Instant::now` | ime_refresh.rs 15 件(関数 9)、executor.rs 4 件(関数 2)、(`runtime/mod.rs:561-567` は他担当) |
| S9 借用ビュー → 所有スナップショット | `ImeControlView<'a>`・`FocusFacts<'a>` の `class_name: &'a str` を所有型へ。`ActiveImeKind` を ungated へ | ime_decision_view.rs 1 ファイル、利用側は ime_controller.rs・transport.rs・gji_monitor.rs・key_pipeline.rs 等(他担当、`ImeControlView` 参照は領域内で open_chain.rs 3、executor.rs 1) |
| S10 HWND/IMM を含む不透明トークン化(`ActuationTarget`、`FocusSnapshot::hwnd()`) | `ImmCrossOp::Targeted` の HWND | open_chain.rs 3 項目(`ImmCrossOp`・`AsyncChainWriter`・`imm_cross_write`)、ime_refresh.rs の `IoMode` |
| S11 Atomic/グローバル読みの引数化 | `OUTPUT_GATE.last_vk_output_ms`、`tsf_obs()`、`record_actuation_decision_skipped` のカウンタ | ime_refresh.rs 3 関数、open_chain.rs 3 関数 |
| S12 `&mut Runtime` / `&mut WindowsPlatform` を取る殻の分離 | 判断(core)と実行(E)に分ける | F の全関数: ime_refresh.rs 11、open_chain.rs 4、executor.rs 5、probe_admission.rs 1 = 21 関数 |
| S13 Win 型の変換殻(`HwndId::to_hwnd`/`From<HWND>`)の拡張 trait 化 | ime_event.rs の 2 impl | 15 行 |

## 6. 新 crate への移動候補の順序案(移しやすい順)

1. `state/ime_event_log.rs`(gate を外すだけ、0 seam)・`focus/hwnd_cache.rs`(S5、別担当)。
2. `state/platform_state.rs` 本体(S1〜S7)。前提の小さな作業: ① `ForegroundScope` 型を ungated へ(14 行)、② `ImeUpdate`/`ImeObs` を `state/` へ(約 30 行)、③ `journal.rs` の `SentKeyEvent` と dump 2 関数の分離、④ `ImeStateHub::new(tick_fn, scope_fn)`、⑤ 可視性。この 5 つで 1799 行が Linux でテストでき、ハーネス写し #1,2,3,4,7,8 が本物に置き換わる。
3. `state/ime_decision_view.rs`(S9: `ActiveImeKind` 移動+`class_name` の所有化、`from_snapshot` だけ Windows 側へ。残る 19 行は O)。
4. runtime/executor.rs の P/P*: `ImeApplyCompletion`、`BatchResult`(`CallbackResult` を ungated へ)、`DecisionExecutor` の状態、`update_intra_batch_applied`(`apply_result_effective_open` に一本化すれば消える)、`issue_self_actuation_order`(platform_state.rs 側へ戻す)、`reinject_wait_remaining`/`run_passthrough_pipeline`(2 つの O 値を引数化)。
5. open_chain.rs の P/P*(`all_chain_record`・`async_record`・`record_attempt`・`is_applicable`・`ImmCrossOp` を `ConvAfterOpenId`/`TargetToken` へ)。
6. F の分割(順に): `ir_decide_read_strategy`(P* にしやすい 36 行) → `dispatch_ime_set_open` の `plan_set_open` → `execute_relay`/`drain_deferred` の計画/ガード状態機械 → `ir_apply_drift_correction` の `DriftPlan`(guard テスト付け替えが必要) → open_chain.rs の 3 関数(INV-45・BUG-34 の制約が最も強い、最後)。
7. インライン gated 項目: `get_for_keymap`/`get_gji`/`get_native`(関数ポインタ注入で P* 化)、`admit_epoch_in_app`(`same_root` 計測を引数化)。

## 7. 危険箇所(分けると隙間ができる所)

- **D1(await をまたぐ失効 / INV-45 / ADR-089 §9-20、C-4)**: `run_open_chain_async` は ImmCross を await した後、`fallback_write` が**機構ごとに view を作り直して `is_applicable` を完了時点の観測で評価**する(open_chain.rs モジュール doc L35-59)。chain を起案時の `caps(p,k).chain` に固定すると、await 中にフォーカスが動いたとき「完了時点で適用可能な機構が chain にない」取りこぼしが生まれる(KanjiToggle が選ばれる等)。`ImeKindId` は推測値(INV-45)で、非対称なゲートに使えない。core/executor 分割では「各ステップの入力スナップショットは**そのステップ実行時点**に取る」ことを Event/Cmd の契約にすること。`WriteMechanism::ALL` を渡す理由を失うとここを壊す。
- **D2(ADR-119/ADR-180、issue #136)**: `imm_cross_write`・`run_open_chain_async`・`fallback_write` が**それぞれ独立に** InputRelay gate を再検出する(3 重)。共有ヘルパーに束ねると、`fallback_write` が既に `with_app` の中で動いているため再入で `try_borrow_mut` が失敗し fail-open でゲートが恒久的に無効化される(ADR-180 決定1、open_chain.rs L239-244、L463-466 のコメント)。core 側に `decide_gate(inputs)` の純関数を置くのは安全(既に ungated)だが、view を取る `with_app` を共有してはいけない。`with_app` が `None`(再入)のときの挙動は意図的に非対称: `imm_cross_write`/`run_open_chain_async` は fail-open(`is_input_relay=false`、記録は 163-T6 のカウンタ)、`imm_cross_write` の focus_gen 再読(L319)は fail-closed(`focus_gen+1` で `verify_still_current` が失敗)。dumb executor 化で `with_app` が無くなると、この 3 つの意図的な差が型に出なくなる。
- **D3(BUG-34 横展開 E-prep)**: `fallback_write` は `with_app`(RUNTIME 排他 borrow)を握ったまま `apply_mechanism`→`romaji_pre_write`(`SendMessageTimeoutW` ベース)を呼ぶ。完全な修正(offload)は view1(捕獲時)/view2(write 完了後)の間に新しい race を作るため意図的に未実施(open_chain.rs L416-446、タスク #108 待ち)。分割で偶然この窓を広げないこと。`known-bugs/BUG-034.md`。
- **D4(ADR-086 INV-14)**: `ActuationTarget` の captured hwnd と `focus_gen` を、open と conv の両方で同一の検証済み hwnd に使い回す(`set_ime_open_then_conv_for_target` に閉じ込めて 1 回の呼び出しにする)。トークン化(S10)しても open→conv を 2 つの Cmd に割ってはいけない。
- **D5(テキスト走査ガード、約 70 件がファイルパスに依存)**: `tests/architecture_guard.rs` が `state/platform_state.rs` 22、`runtime/ime_refresh.rs` 18、`runtime/open_chain.rs` 14、`runtime/executor.rs` 13 回パスを直書きしている。他に `layer_boundary_guard.rs` 1+3、`intent_store_effective_open.rs` 3、`closed_loop_scenarios.rs` 3、`warmup_gate_focus_scope.rs` 1(platform_state.rs)。関数を移すと**ガードが空振りして通る**危険がある(マーカーが見つからなくても落ちない書き方のものがあるか未確認)。移動 PR では移動先パス・関数名の付け替えを同時に行い、空振りを検出する。具体例: `ir_apply_drift_correction` 内の `match act_policy {…}` マーカー(architecture_guard.rs:1823-1906、ADR-080 不変条件 6)、`.record_optimistic(` = 1(L1613)、`.apply_ime_open_with_view(` = 2。
- **D6(dylint の「指定関数」)**: `lints/ime_event_guard` の designated 関数 `apply_panic_reset`/`apply_hwnd_cache_restore`/`apply_key_effect_prediction`/`pass_through_observed` は platform_state.rs 内(L1141、L1242、L266、L476)。`lints/actuation_call_guard::RESTRICTED_CALLS`(`dispatch_ime_set_open`・`ir_apply_drift_correction` を許可呼び出し元として列挙、complexity-budget.md の 1-in-1-out 対象)も関数名と crate パスで識別するので、`dispatch_ime_set_open`/`ir_apply_drift_correction` を分割・移動する際は許可リストの更新が増加扱いにならないか(complexity-budget は「未発効」)を確認する。belief フィールドの private 化(`ImeModel.desired_open`/`input_mode`)は `ime_model.rs` と同じ crate に置く限り保たれるが、`ImeStateHub` が `shadow_model.{last_intent, pending, applied, force_guards, observe_miss_monitor, observations, input_barrier, app_policy}` を**直接** pub フィールドとして操作している(platform_state.rs 全体で多数)ので、`ime_model.rs` と `platform_state.rs` は同じ crate に置く必要がある。
- **D7(時計)**: S8。core 関数が `Instant::now()`/`GetTickCount64` を直読みすると、仮想時計のシナリオテスト(`closed_loop_scenarios`)が壊れる。`ime_refresh.rs` 15 件、`executor.rs` 4 件。
- **D8(`std::cell::Cell` と `!Sync`)**: `ImeStateHub.intent_override_logged: Cell<bool>`(L103)は `&self` の `effective_open` から更新するための単一スレッド前提。新 crate を別スレッドから使う設計(core をワーカーで走らせる等)にすると `!Sync` が効く。
- **D9(`spawn_local`/`!Send`)**: `AsyncChainWriter::write`・`imm_cross_write`・`run_open_chain_async` は HWND を持つので `!Send`、`#[allow(clippy::future_not_send)]`。トークン化後に `Send` になっても、実際に別スレッドへ送ると IMM32 のスレッドアフィニティに反する(`ActuationTarget::verify_still_current` と同じ制約、open_chain.rs L122-131)。
- **D10(重複規則)**: `executor.rs::update_intra_batch_applied`(L891-923)は `ime_model::apply_result_effective_open`(L110-124)と同一の写像を重複して持つ(挙動が同じことを両関数の match で確認)。分割時に片方だけ直すと乖離する。
- **D11(`executor.rs::issue_self_actuation_order` と `runtime/mod.rs::issue_actuation_order_with_origin`)**: ロジックを意図的に重複している(doc L73-90)。core に統合する際、`DecisionExecutor` に `Runtime` 依存を持ち込まない制約は維持(`with_app` 再入で `None` が返ると「授権が下りなかった」と区別できない、ADR-090 §2.A.2(1))。

## 8. 未確認の点

- `architecture_guard.rs` のパス依存ガード(約 70 件)のうち、マーカー欠落で**黙って通る**ものがどれだけあるか(個々の assert を全部は読んでいない)。
- `journal.rs`(2131 行)の ungate 可否の最終確認: `crate::` 参照を全列挙し、Win 依存が `SentKeyEvent`・`hook::current_tick_ms`(dump 2 関数)の 2 種類だけであることは確認したが、`tests` 側(journal.rs 内テスト)の Win 依存は未確認。
- `observer/ime_observer.rs::classify_fetched_snapshot`/`ime.rs::ImeSnapshot` を ungate できるか(`ImeSnapshot` の他のフィールドの型は pure の Option 群に見えたが、ime.rs 全体の他項目との関係は未確認)。
- `runtime/key_pipeline.rs`(ハーネス写し #5)の純粋核の切り出し可否: 領域外のため未調査。
- `PassthroughQueue` が Win 非依存(`runtime/transport.rs` で `windows::`/`crate::win32`/`crate::hook`/`unsafe` の grep 0)であることは確認したが、`check_keyup_symmetry`/`check_output_guard_defer` の本体は読んでいない。
- 実際に `#[cfg(windows)]` を外して Linux でコンパイルした結果(借用チェック・dead_code 警告・循環参照の有無)は未確認(ビルド禁止のため)。S1〜S7 で足りるという主張は、全 `crate::` パスと全関数の通読に基づく静的な結論。
- `runtime/mod.rs` の `Runtime` struct のフィールド数と `impl Runtime` の分散(他担当)。F の分割後に「`Runtime` が持つ `active_actuation`・`drift_giveup_*`・`kana_lock_hysteresis` などの状態をどちらへ置くか」は未設計。
