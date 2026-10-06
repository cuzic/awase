# 棚卸し A2: ime / imm / ime_controller 領域 (develop 9df983e4、読み取りのみ)

対象: crates/awase-windows/src 配下の ime.rs, imm.rs, ime_controller.rs, ime_diagnostic.rs, input_defer.rs, send_health.rs, probe_actuation_fence.rs, shadow_send_trace.rs, conv_mutation.rs、および state/ime_decision_view.rs(触れるだけ)。
行数は `wc -l` と自作スクリプト(関数の直前の doc コメント・属性行を関数に含め、次の項目の手前までを範囲とする)で数えた。**本体の行数は doc コメント込み**。F の多くは doc が行数の過半を占めるので、F の「判断ロジックの実行行」はもっと小さい(各関数 10〜30 行程度、後述)。
分類の未帰属行は、モジュール先頭の doc、import、項目間の退役コメント。

## 0. 先に結論

- **ADR-163/180 の「決定と実I/Oの分離」は、ime_controller 経路ではかなり進んでいる**。「何を送るか」「どの機構を試すか」「InputRelay で止めるか」「ROMAN 補完が要るか」は、すでに ungated の `state/ime_actuation_decision.rs` の関数に出ている(`decide_gate` :129、`is_input_relay`、`decide_chain` :153、`decide_needs_romaji_pre_write` :300、`decide_attempt` :347、private の `gji_direct_already_matches` :224)。走査規則(`run_chain`)と型状態(`ActuationOrder`→`Actuation`)も ungated の `state/actuation_chain.rs`。`MechanismWriter` trait がすでに継ぎ目になっている。
- ime_controller.rs に残る F は 3 つ: `apply_mechanism`(185 行)、`romaji_pre_write`(88 行)、`SyncChainWriter`(35 行)。`ImeController::apply`(82 行)はほぼ P* で、`SyncChainWriter` を引数で注入できれば core に移せる。
- **この領域で一番「F」が多いのは ime.rs**(666 行)。ActuationTarget の verify と、`set_ime_conv_for_target` / `set_ime_open_then_conv_for_target` のような「検証 → 書き込み → 結果の写像」の複合、`read_ime_state_full/fast` の中の分類が混ざっている。
- 継ぎ目は 3 種類でほぼ足りる: (a) `&ImeControlView<'_>` を所有スナップショットにする、(b) HWND を `WindowId` newtype にする、(c) グローバル static/thread_local を戻り値か注入に変える。
- 動かすと危ない所は §6。特に「`reset_candidate_was_seen()` を送信成功の直後に消費する」(ADR-171/BUG-113)と、「`set_ime_open_then_conv_for_target` が検証した HWND を open と conv の両方で使い回す」(INV-14)は、素朴に Cmd/Event へ割ると隙間ができる。

## 1. 領域サマリ(本体行数、分類別)

| ファイル | 全体 | 本体 | テスト | P | P* | O | E | G | F | 未帰属 | cfg(windows) の原因 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| ime.rs | 1736 | 1707 | 29 | 127 | 70 | 468 | 301 | 0 | 666 | 75 | lib.rs:54 `#[cfg(windows)] pub mod ime` |
| ime_controller.rs | 1016 | 774 | 242 | 83 | 312 | 0 | 0 | 0 | 308 | 71 | lib.rs:56 |
| imm.rs | 292 | 292 | 0 | 71 | 8 | 68 | 20 | 0 | 98 | 27 | lib.rs:60 |
| ime_diagnostic.rs | 376 | 376 | 0 | 95 | 19 | 209 | 0 | 17 | 0 | 36 | lib.rs:58 |
| input_defer.rs | 201 | 119 | 82 | 72 | 11 | 0 | 0 | 0 | 15 | 21 | lib.rs:62(`pub use` は lib.rs:121 の別 cfg) |
| send_health.rs | 151 | 122 | 29 | 25 | 63 | 0 | 0 | 0 | 0 | 34 | lib.rs:85 |
| probe_actuation_fence.rs | 283 | 230 | 53 | 17 | 87 | 0 | 0 | 0 | 0 | 126 | lib.rs:81 |
| shadow_send_trace.rs | 62 | 62 | 0 | 12 | 0 | 0 | 0 | 0 | 0 | 50 | lib.rs:87 |
| conv_mutation.rs | 66 | 54 | 12 | 0 | 8 | 0 | 0 | 0 | 0 | 46 | lib.rs:50 |
| state/ime_decision_view.rs | 153 | 153 | 0 | 34 | 76 | 18 | 0 | 0 | 0 | 25 | state/mod.rs:193 `#[cfg(windows)] pub(crate) mod ime_decision_view` |
| **合計** | **4336** | **3889** | **447** | **536** | **654** | **763** | **321** | **17** | **1087** | **511** | |

- 移せる行数(P+P*): 1190(本体の 30.6%)
- 残る行数(O+E+G): 1101(28.3%)
- 分割が要る行数(F): 1087(28.0%)
- 未帰属(doc・import): 511(13.1%)
- テスト 447 行は全部 `#[cfg(windows)]` の親モジュールの配下で、Linux のテストバイナリには存在しない(CLAUDE.md の「幽霊テスト」)。ime_controller.rs の 242 行はほぼ Linux で回せる形(後述)。

### Win トークンが無いのに gate されているファイル(この領域)
send_health.rs, probe_actuation_fence.rs, shadow_send_trace.rs, conv_mutation.rs の 4 つは `windows::` も `unsafe` も無い(imm.rs の send_ime_control_raw と win32.rs の send_input_safe から呼ばれるだけ)。gate は宣言側 `lib.rs` の `#[cfg(windows)]` だけが原因。input_defer.rs は `crate::tsf::probe_bridge::post_drain_output_queue()`(PostMessage、input_defer.rs:67)の 1 行だけが Windows 依存。

## 2. ファイル別の関数表

凡例: 範囲は doc コメント込み。F の「分割案」は「core に出す / 残す」の順。

### 2.1 ime_controller.rs(本体 774、テスト 242 = 775-1016)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| trait ImeOpenStrategy + `is_applicable` | 48-62 | 15 | P* | `&ImeControlView<'_>` を借用。中身は ungated の `key_sequence_policy::*_applicable` を呼ぶだけ |
| ImmCrossProcessStrategy + impl | 64-75 | 11 | P* | `key_sequence_policy::imm_cross_applicable(view.focus.profile)` |
| GjiDirectStrategy + impl | 77-112 | 35 | P* | `gji_direct_applicable(view.observed.active_ime_kind.into())`。大半が BUG-113 の doc |
| MsImeDirectStrategy + impl | 114-145 | 31 | P* | 同上 `ms_ime_direct_applicable` |
| static ×3、`strategy_for`、`mechanism_is_applicable` | 147-170 | 22 | P* | seam: view の借用を `DecisionInputs`(所有、Copy)に替える。**3 つの strategy 構造体と trait は、`is_applicable(profile, kind)` の自由関数 1 個に潰せる(移すのでなく削除候補)** |
| `apply_mechanism` | 172-356 | 185 | **F** | §3.1 |
| `romaji_pre_write` | 358-445 | 88 | **F** | §3.2。実行行は約 22 行、残りは doc |
| SyncChainWriter + `MechanismWriter` impl | 447-483 | 35 | **F** | §3.3 |
| struct ImeController(ZST) | 485-504 | 20 | P | doc のみ |
| `chain_record` | 506-515 | 10 | P | 配列詰めるだけ |
| `actuation_decision_record` | 517-539 | 23 | P | ungated の `ActuationDecisionRecord` を組む |
| `log_shadow_warrant` | 541-570 | 30 | P | `&ActuationOrder`(ungated)を読んで tracing のみ |
| `ImeController::apply` | 572-654 | 82 | **P\*** | gate(`decide_gate`)→ `order.into_actuation()` → `verify(FocusImplicit)` → `run_chain(chain, &mut writer)` → record。**唯一の Windows 依存は `SyncChainWriter { view, .. }` の構築**(と `view.focus.class_name` の warn ログ)。seam: `apply<W: MechanismWriter>(order, inputs: DecisionInputs, writer: &mut W)` にする |
| `imm_cross_is_first_applicable` | 656-671 | 16 | P* | view 借用だけが縛り |
| `caps_chain_for` | 673-695 | 23 | P* | `decide_chain(view.into())` の薄皮 |
| `first_applicable_name`(+ skipping_imm) + impl ×2 | 697-723 | 27 | P* | golden 用の観測点。`ime_key_sequence_golden.rs` が使う |
| `characterize_strategy`(pub) | 725-773 | 49 | P* | `ActiveImeKind` / `AppImeProfile` から view を組む。golden が使う |
| テスト 4 + ヘルパ | 775-1016 | 242 | T | `ActuationOrder::issue` と `ImeControlView` を使う。view が所有型になれば Linux で回る。`caps_chain_matches_legacy_all_scan` などは純粋 |

移せる(P+P*)395、残る 0、F 308、未帰属 71。**この file は O/E/G を持たない**(実 I/O は `crate::ime::*` の呼び出しだけで、F の中に入っている)。

### 2.2 ime.rs(本体 1707、テスト 29 = 1424-1452)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| `set_ime_open_cross_process` | 21-51 | 31 | E | live で `get_gui_thread_info_with_timeout(150ms)`(同期)→ `set_ime_open_for_target`。`Instant::now()` を直接読む(:39) |
| `set_ime_open_for_target(hwnd, open)` | 53-99 | 47 | E | `ImmGetDefaultIMEWnd` → `actuate_ime_control(SetOpenStatus, 150ms)`。診断で `gji_candidate_visible_now()`(global、:92)を読む。`Instant::now()` ×1 |
| `send_ime_mode_key(vk)` | 101-178 | 78 | **F** | Win キー押下中なら送らない(`hook::win_key_held()`)、`HeldModifiers::read()`、ALT は解放しない、Ctrl/Shift を解放→VK→復元、`send_input_safe`。**「どの INPUT 列を組むか」は P にできる**(held を値で受けて `Vec<InputSpec>` を返す)。残すのは win_key_held の読み取りと SendInput |
| `send_ime_mode_key_with_shift_release_prefix` | 180-276 | 97 | **F** | 同上+ synthetic Shift↑、`VK_DBE_HIRAGANA` だけ scan 付き。分岐は P 化できる。戻り値 false=未送信の契約(BUG-16 追補)は executor の結果として残す |
| `get_ime_conversion_mode_raw` | 278-290 | 13 | O | `Option<u32>`(conv 生値) |
| `get_ime_conversion_mode_raw_timeout` | 292-312 | 21 | O | 同上 |
| `get_ime_conversion_mode_for_hwnd` | 314-330 | 17 | O | HWND 引数 |
| `get_foreground_window_class` | 332-349 | 18 | O | `String` |
| `modify_conv_mode` | 351-402 | 52 | E | 読み→クロージャ `f(conv)`→差分があれば `SetConversionMode`。`f` は純粋。大半は退役した `set_ime_romaji_mode` の注意書き |
| `detect_ime_open_for_hwnd` | 404-413 | 10 | O | `Option<bool>` |
| `detect_ime_conversion_for_hwnd` | 415-423 | 9 | O | `Option<u32>` |
| `detect_kana_for_hwnd` | 425-457 | 33 | O | HIMC で `ImmGetConversionStatus`。最後の 6 行(native/roman → kana 判定)は P |
| struct `ImeSnapshot` | 459-495 | 37 | P | 所有型の観測結果(`Option<bool>` 等)。そのまま core の観測型にできる |
| `read_ime_state_full_with_timeout` | 497-528 | 32 | O | `run_with_timeout`(worker) |
| `read_ime_state_full` | 530-627 | 98 | **F** | §3.4 |
| `offload_unsafe` | 629-638 | 10 | E | `win32_async::offload` の糖衣 |
| `read_ime_state_full_async` / `read_ime_state_fast_async` | 640-652 | 12 | O | |
| `set_ime_open_cross_process_async` | 654-659 | 6 | E | |
| `set_ime_romaji_mode_for_hwnd(hwnd, target_conv)` | 661-695 | 35 | E | クロージャは `target_conv.unwrap_or(conv\|ROMAN)`(P)。残りは書き込み |
| `set_ime_hiragana_mode_cross_process` | 697-737 | 41 | E | live の hwnd 解決+ `(conv\|NATIVE\|FULLSHAPE\|ROMAN)&!KATAKANA`(マスクは P) |
| `..._async` | 739-744 | 6 | E | |
| `get_ime_conversion_mode_raw_timeout_async` | 746-759 | 14 | O | |
| `get_ime_conversion_mode_fenced_async` | 761-807 | 47 | **F(小)** | §3.5。フェンス値 `current() != fence_at_call` で Abandoned を返す判断が worker 上に入っている。実行行は約 15 行 |
| `keyboard_layout_info` | 809-821 | 13 | O | `(bool, u32)` |
| `read_ime_state_fast` | 823-917 | 95 | **F** | §3.4 |
| struct `FastImeProbeResult` | 919-929 | 11 | P | |
| `get_focused_hwnd` / `get_focused_hwnd_async` | 931-967 | 36 | O | `HWND`(`SendableHwnd` で Send を偽装) |
| struct `ActuationTarget { hwnd, focus_gen }` + `impl` 行 | 969-989 | 20 | P* | seam: `HWND` → `WindowId`(u64/usize newtype) |
| `capture` / `capture_blocking` | 990-1023 | 33 | O | hwnd を 1 回読んで `ActuationTarget` を作る |
| `verify_gen_only` | 1025-1048 | 24 | P* | `const fn`、gen 比較のみ。HWND を持つ型だけが縛り |
| `verify_still_current`(async) | 1050-1076 | 27 | **F(小)** | hwnd を非同期に読み(O)→ gen を読み直し→比較。比較は `compare`(P*)。§3.5 |
| `compare` | 1078-1089 | 12 | P* | HWND の等値だけ。`WindowId` にすれば P |
| enum `TargetVerifyOutcome` | 1091-1104 | 14 | P* | `Current(HWND)` |
| `get_ime_conv_for_target` | 1106-1135 | 30 | O | target の hwnd で conv を読む |
| enum `ActuationOutcome` / `AbortReason` | 1137-1160 | 23 | P | |
| `set_ime_romaji_mode_for_target_blocking` | 1162-1211 | 50 | **F(小)** | `verify_gen_only` で判定→ `Aborted`/`Written`/`Failed` に写像+ログ。実行行約 20 |
| `set_ime_conv_for_target`(async) | 1213-1286 | 74 | **F** | §3.5。verify → 写像 → offload 書き込み → 写像 |
| enum `ConvAfterOpen` + `From<ConvAfterOpenId>` | 1288-1307 | 19 | P | **ungated の `ConvAfterOpenId`(state/conv_after_open.rs)と完全な重複ミラー。統合して片方を消せる** |
| struct `ImmCrossOutcome` | 1309-1321 | 13 | P | `open_timed_out` を含む |
| `set_ime_open_then_conv_for_target`(async) | 1323-1422 | 100 | **F** | §3.5 |
| `check_tsf_composition_active` | 1454-1488 | 35 | O | HIMC。`bool` |
| `capture_composition_snapshot` + `read_imm_string/i32/bytes` | 1490-1617 | 125 | O | `CompositionSnapshot`(診断専用) |
| struct `CompositionSnapshot` | 1619-1642 | 24 | P | |
| `is_toggle_key_on` / `is_caps_lock_on` | 1644-1661 | 17 | O | `GetKeyState` |
| `toggle_caps_lock` | 1663-1696 | 34 | E | `send_input_safe` |
| `set_ime_mode_for_target` | 1698-1736 | 39 | E | open→(ON のときだけ)conv マスクの RMW |
| テスト `actuation_target_tests` | 1424-1452 | 29 | T | `HWND(raw as *mut _)` を作る。`WindowId` 化後は Linux で回る(`compare` の 2 テスト) |

移せる(P+P*)197、残る(O+E)769、F 666、未帰属 75。

### 2.3 imm.rs(本体 292、テストなし)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| IMC_*/IME_CMODE_*/GCS_* 定数、`lang_id_from_hkl`、`cmode_has` | 14-58 | 45 | P | `awase-vkmap` か core の ime 定数へ |
| `ImmContextGuard`(+ `new`/`himc`/`Drop`) | 60-98 | 36 | O | HIMC の RAII |
| `get_ime_wnd` | 100-113 | 14 | O | `ImmGetDefaultIMEWnd` |
| enum `ProbeCmd` + `raw` | 115-128 | 13 | P | `IMC_GET*` の写像 |
| enum `ActuateCmd` + `raw` | 130-143 | 13 | P | `IMC_SET*` の写像。**core の `MechanismCommand` に相当する語彙が未だ無い**(§3.1) |
| `probe_ime_control` | 145-162 | 18 | O | `Option<usize>` |
| `actuate_ime_control` + thread_local `PROBE_TIMED_OUT` | 164-183 | 20 | E | actuation の唯一の入口(`actuation_call_guard`) |
| `reset_probe_timed_out` / `take_probe_timed_out` | 185-193 | 8 | P* | seam: thread_local の副チャネルを `send_ime_control_raw` の戻り値(`Ok(v)`/`TimedOut`/`Rejected`)にする |
| `send_ime_control_raw` | 195-292 | 98 | **F** | §3.6。実 I/O は `SendMessageTimeoutW` と `GetLastError` の 2 行だけで、周りに 4 種の計測が付いている |

移せる 79(P+P*)、残る 88(O+E)、F 98、未帰属 27。

### 2.4 ime_diagnostic.rs(本体 376、`with_app_ref` を 3 回使う、診断専用)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| thread_local `TSF_PROBE_SNAP` + set/clear | 28-46 | 17(=thread_local 9+関数 8) | G | handle_wm_timer が RUNTIME を排他借用中の `with_app_ref` 失敗(BorrowError)を避ける回避策。RuntimeDiagnosticSnapshot を運ぶ |
| struct `AppStateView` / `ImeDiagnosticSnapshot` | 48-102 | 54 | P | 所有型 |
| `ImeDiagnosticSnapshot::capture` | 104-194 | 91 | O | `GetForegroundWindow`、`GetKeyboardLayout`、`get_gui_thread_info_with_timeout`、`with_app_ref(Runtime::diagnostic_snapshot)`。判断なし |
| `ImeDiagnosticSnapshot::log` | 196-235 | 40 | P | 整形のみ |
| `capture_imc` | 237-268 | 32 | O | `run_with_timeout` で worker へ |
| `log_composition_probe` | 270-356 | 87 | O | 取得+約 45 行の整形(整形部は P にできるが診断なので動かす価値は低い) |
| `resolve_injection_mode_label` | 358-376 | 19 | P* | `InjectionHint`/`AppKind` → 文字列の写像。`with_app_ref` 依存 |

出力はログのみ。**Windows 側に残してよい**。

### 2.5 input_defer.rs(本体 119、テスト 82)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| struct `InputDeferQueue`、Debug/Default/new | 11-42 | 26 | P | `Mutex<VecDeque<RawKeyEvent>>`(std)。RawKeyEvent は core の型 |
| `static INPUT_DEFER` | 23 | 1 | P* | global static。hook thread と main thread の両方から触る(`lib.rs:121` で再 export)ので static のまま |
| `defer_during_output` | 44-53 | 10 | P* | hook thread から呼ぶ。poison 回復あり |
| `replay_later` | 55-69 | 15 | **F(小)** | 「キューが増えたら `post_drain_output_queue()`(PostMessage)」。split: 「増えたか」を bool で返し、呼び出し側(executor)が post する |
| `take_all` | 71-94 | 24 | P | 時刻順に並べ替え |
| `pending_len_nonblocking` | 96-100 | 5 | P | `try_lock` |
| `push_with_cap` | 102-118 | 17 | P | |
| テスト 4 | 120-201 | 82 | T | `RawKeyEvent` を作るだけ。Linux で回る形(いまは gate されている) |

参照元は 8 ファイル、29 箇所(app/mod.rs 4、executor.rs 5、key_pipeline.rs 3、message_handlers.rs 8、runtime/mod.rs 3、vk_send.rs 1、gji_fsm.rs 1、probe_bridge.rs 3)。ADR-156「defer/replay キューの解放条件」の再発ファミリーなので、移動時は defer 側/drain 側の両窓口を見ること。

### 2.6 send_health.rs / probe_actuation_fence.rs / conv_mutation.rs / shadow_send_trace.rs

| ファイル | 内容 | 分類 | メモ |
|---|---|---|---|
| send_health.rs 本体 122 | 定数 3、`SendHealth` static(Atomic 3)、`record(elapsed_ms, now_ms)`、`last_elapsed_ms`、`consecutive_slow`、`blocking_allowed(now_ms)`、`is_blocking_allowed`(純粋) | P 25(定数+is_blocking_allowed)/ P* 63 | 時計は引数 `now_ms` で受けていて注入済み。seam は「global static → 所有構造体」だけ。worker thread からも書くので Atomic のまま Arc で共有。呼び出し元 `blocking_allowed`: key_pipeline.rs:2792、focus_tracking.rs:882、message_handlers.rs:1342 |
| probe_actuation_fence.rs 本体 230(doc が 114 行) | `ProbeFence` static(AtomicU64+LifetimeCounter×4)、`bump`、`current`、`record_abandoned/spawned`、カウンタ読み出し ×4、`FencedProbeOutcome` | P 17(enum)/ P* 87 | worker thread から読むので static/Atomic のまま |
| conv_mutation.rs 本体 54 | `AtomicU64` の `bump`/`current` のみ | P* 8 | 同上 |
| shadow_send_trace.rs 62 | `tracing::debug!` 2 関数のみ。蓄積しない | P 12 | 引数は全部プレーン値 |

この 4 つは「物理 syscall 境界(`imm::send_ime_control_raw` と `win32::send_input_safe`)で bump する」ことが設計の核(probe_actuation_fence.rs:15-39)。**core 側へ移すのは型とカウンタ本体まで。bump の呼び出し地点は Windows 側に残す**(§6 D3)。

### 2.7 state/ime_decision_view.rs(別担当、触れるだけ。本体 153)

- gate 原因: ObservedState が Win-gated の `tsf::observer::ActiveImeKind` と `TsfObservations` を持つ(state/mod.rs:193)。
- `FocusFacts<'a>`(借用 `&str`+`AppImeProfile`(ungated)+`focus_gen`)、`ObservedState`、`ControlLog { shadow_on: Option<bool> }`、`ImeControlView<'a>` は P*。`impl From<&ImeControlView> for DecisionInputs`(:143)は P。
- `ObservedState::from_snapshot`(:77-94、18 行)は `tsf::observer::candidate_was_seen()`(global)と `TsfObservations` を読む O。これは Windows 側に残す。
- seam: `active_ime_kind: ActiveImeKind` → `ImeKindId`(`From<ActiveImeKind> for ImeKindId` は `tsf/observer.rs:720` に全域で既にある)、`class_name: &str` → `String`(用途は warn ログと診断だけ)。`composition_active` / `ime_show_seq` / `ime_change_seq`(ADR-117、issue #138 診断用)は ime_controller.rs の **ログ 3 箇所でしか使われていない**ため、`DecisionInputs` に入れずにログ専用の別構造体へ切り出すと view がほぼ `DecisionInputs` + `focus_gen` + `class_name` に縮む。
- 注意: `ImeKindId` に落とすと `MsIme` と「未同定」の区別は `table_ime_kind()`(tsf/observer.rs:393、ms_ime_native_identified を見る)の側に残る。view 経路は `From`(未同定 MS-IME → `MsIme`)を使っているので、ime_controller 経路の挙動は現状どおり(**変えないこと**。未確認: table_ime_kind を使う別経路との使い分けを意図的に変えて良いかは別担当の範囲)。

## 3. 「決定と実I/Oの分離」の境界: どこまで済み、どこが F か

### 済んでいるもの(core 側、ungated)
| 決定 | 場所 | 呼ばれる側 |
|---|---|---|
| InputRelay で止める | `decide_gate` / `is_input_relay`(ime_actuation_decision.rs:129) | `ImeController::apply`(ime_controller.rs:588-600)ほか |
| 機構チェーン | `decide_chain`(:153) | `caps_chain_for`(ime_controller.rs:679) |
| ROMAN 補完の要否 | `decide_needs_romaji_pre_write`(:300) | `romaji_pre_write`(ime_controller.rs:411) |
| 1 機構分の「何を送るか」 | `decide_attempt`(:347) → `(romaji_pre_write, Option<MechanismCommand>)` | `apply_mechanism`(:218)と `SyncChainWriter::write`(:462)の**2 回** |
| GjiDirect の already-matched | `gji_direct_already_matches`(:224、`shadow_on == Some(open)` かつ OFF なら `!candidate_was_seen`) | `decide_attempt` 内 |
| 走査規則・フォールスルー・型状態 | `Actuation::run_chain`(state/actuation_chain.rs) | `ImeController::apply` |
| 判断の記録 | `ActuationDecisionRecord`(state/actuation_decision_record.rs) | `actuation_decision_record`(ime_controller.rs:521) |

### 3.1 `apply_mechanism`(ime_controller.rs:172-356、185 行)— F
- 構造: `romaji_pre_write`(E 呼び出し) → `decide_attempt`(P) → `match command` で 3 つの Win32 呼び出しにディスパッチ。
  - `SetOpenCrossProcessSync` → `ime::set_ime_open_cross_process`(E)→ `AppliedWithoutSendInput`/`Failed`
  - `SendVk(vk)`(GjiDirect)→ `ime::send_ime_mode_key`(E)→ 成功なら OFF のとき **`tsf::observer::reset_candidate_was_seen()`(global の書き込み)**→ `Applied`、失敗なら `UnsafeToToggle`(BUG-16 追補)
  - `SendVk(vk)`(MsImeDirect)→ 同様に `Applied`/`UnsafeToToggle`
  - `SetOpenThenConvForTarget`/`SetOpenCrossProcessAsyncUntargeted` → `unreachable!`(:312-333)
  - `None` → `AlreadyMatched`(debug_assert で GjiDirect のみ)
- core に出す: 「コマンド+Win32 の成否 → `ImeOpenOutcome`」の写像(`Applied`/`UnsafeToToggle`/`AppliedWithoutSendInput`/`Failed`/`AlreadyMatched`)と、「GjiDirect の OFF 成功なら candidate_was_seen を消費する」という**後処理を `PostEffect` として戻す**部分。
- 残す(E): `set_ime_open_cross_process(open)` と `send_ime_mode_key(vk)` の 2 呼び出しだけを持つ executor(`trait MechanismExecutor { fn set_open_sync(&mut self, open) -> bool; fn send_vk(&mut self, vk) -> bool; }` 程度)。
- 気づき: `decide_attempt` の戻り型 `MechanismCommand` は 4 variant だが、同期経路が受け取れるのは 2 variant(`SetOpenCrossProcessSync`、`SendVk`)。残り 2 variant は `unreachable!` で、**core 側が将来 `DecisionSite::Sync` でこの 2 variant を返す変更を入れると本番で panic する**(ADR-163 TH1e の「非同期 ImmCross も decide_attempt に統合」と衝突する地点)。型で分ける(`SyncCommand`)のが安全。
- 気づき: `view.observed.composition_active/ime_show_seq/ime_change_seq` を 3 箇所の info ログに出している(issue #138 診断)。

### 3.2 `romaji_pre_write`(ime_controller.rs:358-445、88 行、実行行約 22)— F
- 判断は `decide_needs_romaji_pre_write`(core 済み)。残りは「`ActuationTarget::capture_blocking(focus_gen)`(O、`GetGUIThreadInfo` 30ms)→ `set_ime_romaji_mode_for_target_blocking(target, focus_gen)`(E)→ `!= Written` ならログ」。
- 世代照合は同一値を渡して比較するので恒真(doc :371-387 が自認)。
- 分割案: `Cmd::RomajiPreWrite { focus_gen }` → `Event::RomajiPreWritten(ActuationOutcome)`。ただし「同期の途中で挟む」ステップなので、`MechanismExecutor::romaji_pre_write(focus_gen) -> ActuationOutcome` を trait に足すだけで足りる(Cmd/Event 往復にしない)。
- **E がこのスレッドで同期ブロックしうる**(`SendMessageTimeoutW`、BUG-34 横展開)。`send_health::blocking_allowed` の gate を入れない判断が :419-429 に書かれている(再試行の仕組みが無いまま gate を入れると ROMAN 補完が次のトグルまで無言で欠落する)。**分けるときにこの判断を保つこと**。

### 3.3 `SyncChainWriter`(ime_controller.rs:447-483、35 行)— F
- `write()` が `decide_attempt` を呼び(**`apply_mechanism` の中でもう一度同じ決定が走る**)、`AttemptRecord` を組んで配列に詰める。`with_app_available: true`、`shadow_on_before_bug113_override: None`、`post_failed_reobservation: None` は同期経路で埋められない固定値。
- 分割案: 記録の組み立て(P)を core の `run_chain` 側の writer ラッパにして、Windows 側は `MechanismExecutor` の 1 実装だけにする。決定の二重呼び出し(`SyncChainWriter::write` と `apply_mechanism`)は、`apply_mechanism(mechanism, open, command, executor)` に command を渡す形で 1 回にできる。純粋なので差異は出ないが、将来どちらか片方だけ入力を変えると食い違う余地がある。

### 3.4 ime.rs の観測側 F
- `read_ime_state_full`(:530-627、98 行): 観測(HKL、hwnd、class、`detect_*`)の途中に、(1)日本語レイアウト判定(`lang_id == LANGID_JAPANESE`)、(2)`is_tsf_native_window(class)` なら `ime_on=None`・`is_tsf_native=true` で早期 return、(3)conv ビットから `is_romaji` を導出(NATIVE なし→`None`、ROMAN あり→`Some(true)`、それ以外→ `detect_kana_for_hwnd` で二重確認)、(4)`probe_timed_out = ime_on.is_none() && take_probe_timed_out()`。
  - core に出す: (2)(3)の分類(`classify_full_probe(raw: RawFullProbe) -> ImeSnapshot`、`is_tsf_native_window` は ungated の focus/class_names.rs:51 にあり、conv ビット判定は imm.rs の P 定数のみ)。(3)の「二重確認」は遅延評価なので `direct_kana: impl FnOnce() -> Option<bool>` を取る形にする。
  - 残す(O): hwnd 解決、`probe_ime_control` ×2、`detect_kana_for_hwnd`。
- `read_ime_state_fast`(:823-917、95 行): (1)非日本語なら `ime_on=Some(false)`、(2)`AppImeProfile::resolve(class, relay_apps, process_name_closure)`、(3)`!profile.can_read_imm32_open_status()` なら `ime_on=None`、(4)`probe_ime_control(open, 20ms)`、(5)conv を読んでログに出すだけ(:902-911、**結果は使われない**。診断だけで 20ms の追加 SendMessage をしている)。
  - core に出す: (1)(3)の判断 `decide_fast_probe(is_japanese, profile) -> Probe | Skip(Option<bool>)`。
  - seam: `input_relay_apps_snapshot()`(`INPUT_RELAY_APPS: OnceLock<RwLock<Vec<String>>>`、worker からも読む例外、CLAUDE.md が説明)を引数 `&[String]` にして、プロセス名解決のクロージャ(`get_window_process_id`/`get_process_name`、Win32)を呼び出し側が評価した結果を渡す。
  - 気づき: (5) は削除してよい可能性が高い(未確認。journal や診断ログの grep 運用で使われていないかは見ていない)。

### 3.5 ime.rs の actuation ターゲット周り(ActuationTarget / verify_still_current / set_ime_*_for_target)
- `ActuationTarget { hwnd, focus_gen }` はフィールド private で、`verify_still_current` か `verify_gen_only` を経由しないと hwnd を取り出せない(ADR-086 INV-14)。
- `verify_still_current`(:1050-1076): `get_focused_hwnd_async().await` で現在の hwnd を読み、**その直後に** `read_current_focus_gen()` を呼ぶ(opus 指摘 2026-08-08: gen を先に固定すると hwnd クエリ中の FocusChange を見逃す)。比較は `compare`(P*)と gen の `!=`(P)。
- `set_ime_conv_for_target`(:1213-1286): verify → `Aborted(GenStale/TargetMoved)`(+ログ) / `Current(hwnd)` なら offload で `set_ime_romaji_mode_for_hwnd` → `Written`/`Failed`。
- `set_ime_open_then_conv_for_target`(:1323-1422): verify →(同じ offload クロージャの中で)open を書き、`open_ok` なら conv(`ConvAfterOpen::Write(target)`)を書く。`open_timed_out` は thread_local `PROBE_TIMED_OUT` を同一クロージャ内で reset/take して得る(:1387-1389)。
- core に出す(P): `verify(captured: (WindowId, gen), current: (WindowId, gen)) -> TargetVerifyOutcome`、「`open_ok` かつ `Write` のときだけ conv」の規則、3 値の outcome 写像。
- 残す: hwnd の読み取り(O)、open/conv の書き込み(E)。**検証と書き込みを同じ executor 呼び出しの中に置くこと**(§6 D2)。

### 3.6 `imm.rs::send_ime_control_raw`(:195-292、98 行)— F
実行行は約 50。`SendMessageTimeoutW` と `GetLastError` の他に次が入っている:
1. `cmd == IMC_SETCONVERSIONMODE` なら `conv_mutation::bump()`(global)
2. `is_actuation = !matches!(cmd, GET*)`(P)なら `probe_actuation_fence::bump()`(global、**必ず syscall より前**)
3. `hook::current_tick_ms()` ×2、`hook::now_timestamp_us()` ×2(時計の直接読み)
4. `state::imm_evidence::send_failure_is_timeout(last_error, elapsed_us, timeout_ms)`(ungated の純粋関数)→ thread_local `PROBE_TIMED_OUT` に書く
5. `tracing::debug!`、`is_actuation` なら `shadow_send_trace::record_ime_control`、`send_health::record(elapsed, end_ms)`(**end_ms は debug! のコスト前に確定する**。ADR-140 コードレビュー指摘)
- core に出す: `classify_imc(cmd) -> ImcKind { Probe, Actuation, ConvActuation }`(P)と、計測の結果を受ける純粋な `fn on_imc_result(kind, elapsed_us, last_error, timeout_ms) -> ImcBookkeeping`。
- 残す(E): bump 2 つ・時計・`SendMessageTimeoutW`・`GetLastError`。戻り値を `ImcSend { value: Option<usize>, timed_out: bool }` にして thread_local を消す(`PROBE_TIMED_OUT` の読み手は ime.rs 3 箇所:`read_ime_state_full`、`set_ime_open_then_conv_for_target` の 2 行)。

## 4. Windows 側の依存の種類と件数(この領域)

| 種類 | 件数・場所 |
|---|---|
| HWND を引数/フィールドに持つ | ime.rs 23 項目(`set_ime_open_for_target`、`get_ime_conversion_mode_for_hwnd`、`modify_conv_mode`、`detect_*` ×3、`set_ime_romaji_mode_for_hwnd`、`get_focused_hwnd`(+async)、`ActuationTarget`/`capture`/`capture_blocking`/`verify_gen_only`/`verify_still_current`/`compare`/`TargetVerifyOutcome`、`get_ime_conv_for_target`、`set_ime_conv_for_target`、`set_ime_open_then_conv_for_target`、`set_ime_romaji_mode_for_target_blocking`、`check_tsf_composition_active`、`capture_composition_snapshot`、`set_ime_mode_for_target`)。imm.rs 5(`ImmContextGuard::new`、`get_ime_wnd`、`probe_ime_control`、`actuate_ime_control`、`send_ime_control_raw`)。ime_diagnostic.rs 1(`capture_imc` は生 usize) |
| HIMC(IMM32) | `ImmContextGuard` の利用: `detect_kana_for_hwnd`、`check_tsf_composition_active`、`capture_composition_snapshot`(+ `read_imm_*` ×3) |
| `SendMessageTimeoutW` | imm.rs の 1 箇所(唯一のチョークポイント) |
| `SendInput` | `send_ime_mode_key`、`..._with_shift_release_prefix`、`toggle_caps_lock` の 3 箇所(`win32::send_input_safe` 経由)。`HeldModifiers::read()` ×2 |
| `crate::hook::*` | ime.rs `win_key_held()` ×2、imm.rs `current_tick_ms()` ×2・`now_timestamp_us()` ×2 |
| 時計の直接読み | imm.rs 上記 4、ime.rs `Instant::now()` ×2(:39、:84)。send_health は `now_ms` を引数で受ける(注入済み) |
| `with_app` / `with_app_ref` | ime_diagnostic.rs 3(`capture`、`log_composition_probe`、`resolve_injection_mode_label`)。ime.rs / ime_controller.rs / imm.rs は 0(rules の B-1 どおり) |
| グローバル static | `INPUT_DEFER`、`SEND_HEALTH`、`PROBE_FENCE`、`CONV_MUTATION_SEQ`、`INPUT_RELAY_APPS`(ime.rs:867、`input_relay_apps_snapshot()`)、`tsf::observer::reset_candidate_was_seen`(ime_controller.rs:268)/`candidate_was_seen`(view :87)/`gji_candidate_visible_now`(ime.rs:92)の global、`IMM_STRATEGY` ×3(ZST) |
| thread_local | `PROBE_TIMED_OUT`(imm.rs:182)、`TSF_PROBE_SNAP`(ime_diagnostic.rs:33) |
| worker 実行 | `win32_async::offload`(`offload_unsafe` ほか 2 箇所)、`win32::run_with_timeout`(ime.rs `read_ime_state_full_with_timeout`、ime_diagnostic.rs `capture_imc`)、`get_gui_thread_info_with_timeout` ×5 |
| PostMessage | input_defer.rs:67 の 1 箇所(`post_drain_output_queue`) |
| COM / UIA / MSAA / TSF の API | この領域では 0。ただし TSF の観測型 `tsf::observer::{ActiveImeKind, TsfObservations}` を view が参照 |
| `unsafe` | ime.rs/imm.rs/ime_controller.rs/ime_diagnostic.rs の先頭に `#![allow(unsafe_code)]`。ime_controller.rs は実際の unsafe 呼び出しが F の 5 箇所(`apply_mechanism` 3、`romaji_pre_write` 2) |

## 5. 継ぎ目一覧と新 crate への移動順

### 5.1 seam 一覧
| seam | 現れる数 | 直し方 |
|---|---|---|
| `&ImeControlView<'_>` 借用 | ime_controller.rs の 15 項目(trait+3 strategy+`mechanism_is_applicable`+`apply_mechanism`+`romaji_pre_write`+`SyncChainWriter`+`apply`+`imm_cross_is_first_applicable`+`caps_chain_for`+`first_applicable_name`×2+`characterize_strategy`)。runtime 側の参照は別担当(open_chain.rs 8 箇所など) | `DecisionInputs`(所有・Copy、すでにある)+ `focus_gen: u32` に縮める。class_name と診断 3 値はログ専用の別 struct |
| `ActiveImeKind`(Win-gated)を view が持つ | `.into()` 3 箇所(ime_controller.rs:108,141,414)+ view 1 | `ImeKindId` を直接持つ |
| HWND | §4 の 23+5 項目 | `WindowId(usize)` newtype。ime.rs の P* 4 項目(`ActuationTarget`、`verify_gen_only`、`compare`、`TargetVerifyOutcome`)は即 P になる |
| thread_local の副チャネル | 2(`PROBE_TIMED_OUT` と `TSF_PROBE_SNAP`)。前者の読み手 3 | 戻り値に載せる(前者)。後者は診断専用なので G のまま |
| グローバル static のカウンタ | 4(`SEND_HEALTH`、`PROBE_FENCE`、`CONV_MUTATION_SEQ`、`INPUT_DEFER`) | 型だけ core、static は Windows 側(worker thread から読むので Atomic/Arc)。bump の呼び出し地点は Windows 側に残す |
| global 状態の副作用書き込み | 1(`reset_candidate_was_seen`) | `PostEffect` として戻す。ただし §6 D1 |
| 時計の直接読み | 6(§4) | `now_ms` 引数。send_health はすでにその形 |
| 決定の二重呼び出し | 1(`SyncChainWriter::write` と `apply_mechanism` の両方で `decide_attempt`) | command を渡して 1 回にする |
| `MechanismCommand` が実際の経路より広い | 1(`unreachable!` 2 variant) | 同期用の部分型に分ける |
| ConvAfterOpen の重複型 | 1 組(`ime::ConvAfterOpen` と `state::conv_after_open::ConvAfterOpenId`) | 片方を消す |
| 同一の定数/enum が core にもある | `IME_CMODE_*` など imm.rs 定数(core の kana/romaji 判定側との重複は未確認) | 未確認 |

### 5.2 新 crate(awase-runtime)への移動順(依存が少ない順)
1. **純粋な型と定数**(1 ファイル内、依存ゼロ): imm.rs の定数・`ProbeCmd`/`ActuateCmd`・`lang_id_from_hkl`/`cmode_has`、ime.rs の `ImeSnapshot`/`FastImeProbeResult`/`CompositionSnapshot`/`ActuationOutcome`/`AbortReason`/`ImmCrossOutcome`、`ConvAfterOpen`(`ConvAfterOpenId` に統合)。計約 190 行、変更が機械的。
2. **計測カウンタの型**: `send_health`(`is_blocking_allowed` 純粋、定数)、`probe_actuation_fence`(`FencedProbeOutcome`、カウンタ型)、`conv_mutation`、`shadow_send_trace`。計約 330 行(うち大半が doc)。static の置き場だけ決める(ADR-164)。
3. **`InputDeferQueue`**: `replay_later` の PostMessage を呼び出し側に出してから。std の Mutex だけで、テスト 82 行が Linux で回り始める。
4. **`ImeControlView` の所有化**(`DecisionInputs`+`focus_gen`)。これが 5〜6 の前提。
5. **`ImeController::apply`**(`MechanismWriter` を引数に)+ `chain_record`/`actuation_decision_record`/`log_shadow_warrant`/`imm_cross_is_first_applicable`/`caps_chain_for`/`characterize_strategy` と strategy 3 構造体の削除。約 320 行(うち 242 行のテストが Linux で回るようになる)。
6. **`ActuationTarget` の純粋部分**(`WindowId` 化、`verify`、`compare`、`TargetVerifyOutcome`)。ime.rs のテスト 29 行も Linux で回る。
7. **F の分割**(最後): `apply_mechanism`(→ executor trait + 結果写像)、`romaji_pre_write`、`send_ime_mode_key*` の INPUT 列計画、`read_ime_state_full/fast` の分類、`send_ime_control_raw` の分類、`set_ime_conv_for_target`/`set_ime_open_then_conv_for_target` の写像。
8. **残す**: ime.rs の O/E 全般、imm.rs の O/E、ime_diagnostic.rs、`ObservedState::from_snapshot`。

## 6. 危険箇所

- **D1: `reset_candidate_was_seen()` の消費タイミング**(ime_controller.rs:268、ADR-171、BUG-113)。GjiDirect の OFF 送信が成功した瞬間にその場で証拠を消し、次の apply が同じ証拠を再度読んで二重送信するのを防いでいる。`PostEffect` を戻して core が後から消す形にすると、`SendInput` と消費の間に別の apply(同一押下の `shadow_toggle_off_sync` / `engine_decision_sync` の 2 経路、ime_controller.rs:88-100)が割り込める窓ができ、BUG-113(Windows Terminal+GJI で余分な「@」)が再発しうる。消費は executor の同じ同期呼び出し内に置くか、押下 ID の台帳(`claim_press_write`、ADR-208 L1)で閉じること。
- **D2: 検証した HWND の使い回し**(ime.rs:1326-1332、INV-14、BUG-59 追補と同型)。`set_ime_open_then_conv_for_target` は open と conv を**同じ `offload` クロージャ**で書いて、1 回の `verify_still_current` の結果を両方に使う。`Cmd::SetOpen` と `Cmd::SetConv` に割って別々に verify すると、open の完了待ちの間にフォーカスが動いて conv が別ウィンドウに着弾する。複合 Cmd(`OpenThenConv`)のままにすること。`verify_still_current` の「hwnd を読み終えた直後に gen を読む」順序(:1050-1058)も、Cmd/Event 往復にすると main thread での gen 読み取りが 1 往復後ろにずれる。**await をまたぐ失効**に直接かかる。
- **D3: 計測 bump の地点**(probe_actuation_fence.rs:15-39、ADR-140)。「論理呼び出し箇所ごとに bump すると未発見の経路で穴が開く」ため、物理 syscall 境界(`send_ime_control_raw` と `win32::send_input_safe`)に置いている。core が Cmd を出す側で bump する設計に寄せると、Cmd を経由しない書き込み経路が bump されなくなる。さらに `get_ime_conversion_mode_fenced_async` の checkpoint 2 は worker thread 上で `current()` を読む(同 :34-39)ので、フェンス比較は Cmd/Event の往復にできない。**フェンス比較(§3.5 の F 小)は Windows 側の executor に残す**。
- **D4: 同期経路の blocking**(BUG-34 横展開、`docs/known-bugs/BUG-034.md`)。`apply_mechanism` の ImmCross 同期アーム(`set_ime_open_cross_process`、live の `GetGUIThreadInfo` 150ms+`SendMessageTimeoutW` 150ms)と `romaji_pre_write`(`capture_blocking` 30ms+`SendMessageTimeoutW` 50ms×2)は main/hook thread で同期にブロックしうる。`send_health::blocking_allowed` は `romaji_pre_write` に意図的に入っていない(:419-429)。executor 化で「呼ぶ側が Cmd を作る側と別スレッド」になっても、同期の保証(すぐ結果を返す)を勝手に async にしないこと。
- **D5: `unreachable!`**(§3.1)。core の `decide_attempt` が Sync で `SetOpenThenConvForTarget`/`...Untargeted` を返すようになると、本番の同期経路で panic する。
- **D6: `ImeOpenOutcome` の写像に BUG-16 追補の意味がある**(`UnsafeToToggle` を `Applied` にすると `applied_snapshot` がラッチされ、以降の再試行が全部 no-op になって belief ON × 実 IME OFF が固定される)。`apply_mechanism` の写像を core へ出すときは、`send_ime_mode_key` の `false`(Win キー押下中など)と `Applied` を区別するテストを一緒に持っていくこと。`docs/known-bugs/BUG-016.md`。
- **D7: `input_defer` の分割**(ADR-156)。`replay_later` の PostMessage を呼び出し側へ出すと、呼び出し側が増えるほど「post し忘れ」が起きうる(キューは積まれたのに drain が走らない)。`defer_during_output` 側(hook thread、post しない設計)と `replay_later` 側(post する)で意図的に非対称。
- **D8: 診断のための余分な I/O**: `read_ime_state_fast` の conv 読み取り(:902-911、結果を使わない 20ms の `SendMessageTimeoutW`)と `set_ime_open_for_target` の `gji_candidate_visible_now()`(:92)。executor を「判断なし」にしたとき、この種のログ専用読み取りは F の判断ではなくログ専用 O として切り離す(挙動は変えない)。
- INV-45 に関わる箇所はこの領域では**見つけていない**(未確認: INV-45 の定義自体を読んでいない)。

## 7. 未確認

- INV-45 との関係(上記)。
- `read_ime_state_fast` の conv 読み取り(§3.4 (5))を削除してよいかどうか(ログ運用の確認をしていない)。
- `ObservedState.active_ime_kind` を `ImeKindId` に替えたときの `table_ime_kind()`(未同定 MS-IME が `None`)との意味の違いが、他経路(`state/key_effect_*`、別担当)に影響しないか。ime_controller 経路は `From`(全域)を使うので現状維持で足りるはず。
- imm.rs の `IME_CMODE_*` が core(`awase::...`)の kana/romaji 判定と重複していないか(grep していない)。
- `runtime/open_chain.rs`(別担当)側の `fallback_write` / `imm_cross_write` が `ime::set_ime_open_then_conv_for_target` の結果をどう `ImeOpenOutcome` に写像するか。ここで分割した場合の await 越しの失効は open_chain 側の読みが要る。
- 呼び出し元の件数: 本ファイルでは `crate::ime::*` の参照元を grep で数えただけで、各呼び出しがどのスレッド(hook/main/worker)から来るかは全件は確認していない。`send_ime_mode_key` は doc でメインスレッドとされているが、実際の呼び出し元の確認は未了。
- テストの Linux 化の可否は、型を見ての判断であり、実際には移していない(コンパイル未確認)。
