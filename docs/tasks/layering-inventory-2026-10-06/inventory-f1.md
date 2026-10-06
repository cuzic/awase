# 棚卸し F1: crates/awase-windows/src/tsf/ 直下 (develop 9df983e4)

対象: tsf/ 直下の 13 ファイル(tsf/warmup/ は inv-f2 担当で除外)。読み取り専用で実施。
行数は `wc -l` と範囲の合計で自分で数えた。分類ごとの行数は「関数/ブロックの範囲(コメント・空行込み)」の合計で、どれにも属さない空行・use 行は G(ヘッダ)か「空行」に入れた。

## 0. 先に結論

1. **tsf/probe.rs(1328 行)は Win 中心ではない。** 本体 818 行に `windows::` トークンも `unsafe` も 0。O/E/F も 0。本体はすべて P か P*。Windows に縛っている原因は `crate::hook::current_tick_ms()`(= `GetTickCount64`)の直呼び 9 箇所と、`TSF_OBS`(グローバル static)の直読みだけ。時計と観測値を引数にすれば全部 Linux に移せる。
2. **tsf/ は ADR-030 の「observer / probe / output」できれいには分かれていない。** `probe.rs` は judgement 層を名乗るが、`LiteralDetector` の `new`/`evidence_now`/`check_now` が観測値(`TSF_OBS`)を自分で読みに行く。つまり「読む(O)」と「判断する(P)」が同じ関数に入っている。判断のコア(verdict を決める分岐)は小さく純粋で、読み取り 5 点(write_bytes、last_write_ms、show カウンタ、時計、ops 3 種)を引数にすれば切れる。
3. **既に OS 非依存で完結しているもの**: `gji_fsm.rs`(P のみ、1075 行)、`literal_facts.rs`(P のみ、256 行)、`tsf_gate.rs`(P のみ、369 行、ただし mod.rs で不要に `#[cfg(windows)]`)。この 3 つで本体 1700 行。
4. **`send.rs` は空**(コメント 4 行のみ、アイテム 0)。`mod.rs:39` の `pub mod send;` ごと削除候補。参照元も 0(`tsf::send` を grep して 0 件)。
5. 想定外: `probe_bridge.rs` の `OutputGate` の doc コメントが「`TsfGate` (in `probe.rs`)」と書いているが、実体は `tsf_gate.rs`。古い記述。

## 1. ファイル別

### 1.1 行数一覧と gate

| ファイル | 全体 | 本体 | テスト | fn 数(本体) | gate の原因 |
|---|---:|---:|---:|---:|---|
| gji_fsm.rs | 2629 | 1075 | 1554 | 29 | **ungated**(mod.rs:20-21、cfg_attr で dead_code 許可のみ) |
| probe.rs | 1327 | 818 | 509 | 47 | mod.rs:34-35 `#[cfg(windows)] pub mod probe;`、加えてテスト mod 自体も `#[cfg(windows)]`(probe.rs:820) |
| observer.rs | 892 | 736 | 156 | 46 | mod.rs:30-31。テスト mod も `#[cfg(windows)]`(observer.rs:738)。末尾で gji_monitor/win_event_obs を re-export(734-735)しているのが Windows 依存を引き込む |
| gji_monitor.rs | 668 | 538 | 130 | 17 | mod.rs:24-25。`windows::Win32::*` 直接使用。テスト 130 行は純粋な Debounce 検査だが、親が gated なので Linux では存在しない |
| tsf_gate.rs | 715 | 369 | 346 | 17 | mod.rs:42-43(`#[cfg(windows)] pub(crate) mod tsf_gate`)。**中身は Win 依存ゼロ**(timed-fsm と awase::types のみ) |
| literal_facts.rs | 473 | 256 | 217 | 4 | **ungated**(mod.rs:28-29) |
| tip_detector.rs | 349 | 349 | 0 | 11 | mod.rs:40-41。`windows::Win32::UI::TextServices`(ITfXxx) |
| ime_mode_fsm.rs | 306 | 204 | 102 | 11 | mod.rs:26-27。原因は `crate::imm::IME_CMODE_*`(2 定数)と `crate::hook::current_tick_ms()` の 2 点だけ |
| win_event_obs.rs | 242 | 242 | 0 | 5 | mod.rs:51-52。SetWinEventHook |
| output.rs | 186 | 186 | 0 | 7 | mod.rs:32-33。`INPUT`/`MapVirtualKeyW`、`crate::RAW_TSF_LITERAL`、`send_input_safe` |
| probe_bridge.rs | 149 | 149 | 0(テスト専用 static 9 行を含む) | 9 | mod.rs:36-37 |
| mod.rs | 57 | 57 | 0 | 0 | — |
| send.rs | 4 | 4 | 0 | 0 | mod.rs:38-39、**空モジュール** |
| **合計** | **7997** | **4983** | **3014** | 203 | |

gate 原因の整理: mod.rs は `gji_fsm` と `literal_facts` 以外の全サブモジュールを一律に `#[cfg(windows)]` にしている(mod.rs:9-12 の doc に「gji_fsm 以外は windows crate に依存するため」とあるが、これは `probe.rs`/`tsf_gate.rs`/`ime_mode_fsm.rs` には当てはまらない。上の通り `windows::` を持たない)。実質の理由は 3 つの外部参照だけ:
- `crate::hook::current_tick_ms`(probe.rs 9 箇所、observer.rs 1、ime_mode_fsm.rs 1、gji_monitor.rs 4、win_event_obs.rs 2)
- `crate::imm::IME_CMODE_NATIVE/KATAKANA`(ime_mode_fsm.rs:35,37)
- `crate::output::ColdReason`(probe.rs の ColdContext 等。実体は tsf/output.rs で、これは Win 型 `INPUT` を含むファイルにある)

### 1.2 分類別の本体行数

| ファイル | 本体 | P | P* | O | E | F | G | 空行/その他 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| gji_fsm.rs | 1075 | 1075 | 0 | 0 | 0 | 0 | 0 | 0 |
| literal_facts.rs | 256 | 256 | 0 | 0 | 0 | 0 | 0 | 0 |
| tsf_gate.rs | 369 | 369 | 0 | 0 | 0 | 0 | 0 | 0 |
| probe.rs | 818 | 458 | 329 | 0 | 0 | 0 | 14 | 17 |
| ime_mode_fsm.rs | 204 | 139 | 65 | 0 | 0 | 0 | 0 | 0 |
| observer.rs | 736 | 71 | 81 | 212 | 0 | 0 | 372 | 0 |
| gji_monitor.rs | 538 | 99 | 0 | 147 | 0 | 248 | 35 | 9 |
| win_event_obs.rs | 242 | 0 | 0 | 16 | 98 | 0 | 124 | 4 |
| tip_detector.rs | 349 | 0 | 56 | 165 | 0 | 67 | 45 | 16 |
| probe_bridge.rs | 149 | 0 | 0 | 0 | 4 | 0 | 122 | 23(うちテスト用 static 9、ヘッダ 10) |
| output.rs | 186 | 73 | 23 | 0 | 36 | 35 | 13 | 6 |
| mod.rs | 57 | 0 | 0 | 0 | 0 | 0 | 57 | 0 |
| send.rs | 4 | 0 | 0 | 0 | 0 | 0 | 0 | 4(空) |
| **合計** | **4983** | **2540** | **554** | **540** | **138** | **350** | **782** | **79** |

(P+P* = 3094 行 = 本体の 62%。F が 350 行、O+E+G が 1460 行。コメントが多く、コードだけの行数は gji_fsm 801 / probe 429 / observer 279 / gji_monitor 366 / tip_detector 256 / win_event_obs 196 / tsf_gate 185 / literal_facts 156 / ime_mode_fsm 119 / output 116 / probe_bridge 36 / mod 32 = 2971 行。)

### 1.3 関数表

#### tsf/probe.rs(本体 1-818)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| TsfReadinessProbe(struct+doc) / GjiProbeOutcome | 15-71 | 57 | P | 内部に `Cell<u64>`(settled_at_ms)を持つ。単スレッド前提 |
| TsfReadinessProbe::new | 72-81 | 10 | P | |
| TsfReadinessProbe::check_outcome | 83-102 | 20 | P* | seam: 時計(`hook::current_tick_ms`)と `TSF_OBS.gji_monitor_ok`/`gji_last_io_ms` の直読み。`ProbeObs { now_ms, monitor_ok, gji_last_io_ms }` を引数にする |
| TsfReadinessProbe::check_now | 104-152 | 49 | P* | 同上。判断ロジック(2 フェーズ + settle + margin)は純粋。doc(109-120)に「本番は min_ms=0/total_max_ms=0 を渡すので実質常に true」とある(= 通常は分岐に入らない) |
| WarmEpoch(struct+定義+mark_cold 等) | 155-205, 224-242 | 71 | P | `Cell` のみ |
| WarmEpoch::ms_since_last_send / update_last_send_ms | 206-222 | 17 | P* | seam: 時計。`now_ms` を引数に |
| ColdContext(全体) | 244-328 | 85 | P | `crate::output::ColdReason` を使う。seam: ColdReason を ungated な場所に移す |
| CompositionState::mark_composition_cold | 361-395 | 35 | P* | カウンタ操作の分岐は純粋(RawTsfLiteralRecovery は連続カウント加算、FocusChange/SetOpenTrue はリセット、確認系キーは last_send を touch)。seam: 時計(`ms_since_last_send`/`update_last_send_ms` 経由) |
| CompositionState::on_focus_changed / ms_since_last_send / update_last_send_ms | 397-419 | 21 | P* | 同上 |
| CompositionState その他(struct、new、委譲 getter/setter) | 330-360, 420-480 | 95 | P | 薄い委譲 |
| LiteralDetector(struct+doc) / DetectionResult / From | 482-558 | 77 | P | |
| LiteralDetector::EPOCH_FENCE_GRACE_MS / COMPOSITION_BYTES_THRESHOLD | 560-573, 621-625 | 19 | P | 定数。`tuning::GJI_SAMPLE_INTERVAL_MS` 参照 |
| LiteralDetector::new / new_with_pre_send_baseline | 575-619 | 45 | P* | 実体は O: `TSF_OBS`/`observer::gji_*_ops` から baseline を読んで構造体に詰める。seam: `LiteralObs`(show_counter, write_bytes, ops×3, last_write_ms)を引数で受ける形に |
| LiteralDetector::evidence_now | 627-667 | 41 | P* | 診断専用の `DetectEvidence`(pure 型は literal_facts.rs)を組む。読み取り 6 点 + 時計。seam 同上 |
| LiteralDetector::check_now | 669-745 | 77 | P* | verdict 決定の本体(write_confirmed → fresh/stale、show_confirmed → grace、deadline)。読み取り 3 点 + 時計だけが縛り |
| LiteralDetector::grace_hold_verdict | 747-781 | 35 | P | 既に `now` を引数に取る純粋関数(`Cell` 状態のみ) |
| LiteralDetector::visible_fencing_verdict | 783-807 | 25 | P* | seam: 時計 + `gji_last_write_ms` |
| LiteralDetector::veto_eligible | 809-816 | 8 | P | |
| use/ヘッダ + 区切り空行 | — | 31 | G/空行 | |

要点: `evidence_is_fresh = !fencing_active || last_write_ms >= epoch_send_ms` が 3 箇所に重複(check_now 723-726、visible_fencing_verdict 803-805、evidence_now 639-648)。1 関数に出して純粋化するのが分割の第一歩。

#### tsf/observer.rs(本体 1-736)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| ChangeCounter / Baseline | 26-65 | 40 | P | `AtomicU32` ラッパ。std のみ。Linux に移せる |
| TsfObservations(struct+Default+new、22 フィールド) | 67-295 | 229 | G | グローバル `static TSF_OBS` の中身。書き手は 3 つ: gji-io-monitor スレッド、WinEvent コールバック、runtime(literal_session 系) |
| 単純アクセサ(gji_last_io_ms, gji_attach_ms, gji_monitor_ok, gji_candidate_visible, ime_composition_active, ime_show_seq, ime_change_seq) | 297-348 | 52 | O | 返す型: u64/bool/u32 |
| active_ime_kind | 349-362 | 14 | P* | `tsf_active_kind: u8` → enum の写像。seam: u8 を引数に |
| ime_kind_detected / ms_ime_native_identified | 364-381 | 18 | O | bool |
| table_ime_kind | 383-400 | 18 | P* | (active_kind, ms_native_identified) → `Option<ImeKindId>`。seam: 2 値を引数に |
| current_tip_identity | 402-414 | 13 | P* | 同上 → `TipIdentity` |
| set_ms_ime_native_identified / set_tsf_active_kind / set_ime_product_name | 416-441 | 26 | G | 観測ストアへの書き込み(swap で変化検出) |
| TSF_OBS static / TSF_OBS_TEST_LOCK / tsf_obs() | 443-489 | 47 | G | テストロック 14 行を含む |
| 名前付きライブ読み(gji_last_io_ms, last_write_ms, write_bytes, write_ops, read_ops, other_ops, other_bytes, current_ime_product_name, candidate_was_seen, gji_candidate_visible_now, ime_composition_active_now, reset_candidate_was_seen) | 491-602(下の別掲を除く) | 約 105 | O | 全部 1-3 行の atomic load。`reset_candidate_was_seen` だけ書き込み |
| gji_io_is_attach_artifact | 501-506 | 6 | P | 既に純粋な const fn(BUG-176) |
| gji_idle_ms | 508-511 | 4 | P* | 時計 |
| gji_is_active_ime | 557-567 | 11 | P* | 2 atomic の AND。seam: 引数化 |
| literal_session_confirmed | 604-624 | 21 | P* | atomic の世代と比較。現在の呼び出し元は `probe.rs::evidence_now` だけ(診断専用) |
| literal_session_confirmed_gen_snapshot | 626-638 | 13 | O | `Option<Generation>` を返す。snapshot 用 |
| mark_literal_session_confirmed / reset_literal_session_confirmed | 640-666 | 27 | G | atomic store |
| take_pending_start/end_composition / discard_pending_composition_events | 668-701 | 34 | O/G | swap(false)。drain は 1 回限り(読むと消える)なので純粋ではない |
| ActiveImeKind / From<ActiveImeKind> for ImeKindId | 703-727 | 25 | P | |
| re-export(gji_monitor, win_event_obs) | 729-735 | 7 | G | |

#### tsf/gji_monitor.rs(本体 1-538)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| GJI_PROCESS_PREFIXES / is_gji_process | 22-38 | 15 | P | プロセス名の前方一致判定(大文字小文字無視) |
| find_gji_pid | 40-97 | 58 | O | ToolHelp スナップショット → `Option<(pid, name)>` |
| GjiIoDelta | 99-125 | 27 | P | |
| GjiMonitor(struct, try_attach, getters, Drop) | 127-261 から sample を除く | 89 | O | `OpenProcess`/`CloseHandle`。`unsafe impl Send` |
| GjiMonitor::sample | 175-220 | 46 | F | `GetProcessIoCounters` の読みと、「delta.any() なら last_change_ms を now に更新、write_ops>0 なら last_write_change_ms も更新」という判断(時計込み)が同居。分割案: O は `IoCounters{6 u64}` を返すだけ、core 側 `IoTracker::update(counters, now_ms) -> (delta, change_ms, write_change_ms)` |
| Debounce<T> / ImeKindDebounce / TipIdentityDebounce | 263-319 | 57 | P | 汎用の「同じ新値が 2 回続いたら確定」。テスト 130 行もそのまま Linux で回せる(今は親が gated で存在しない) |
| start_monitor_thread | 321-334 | 14 | G | `win32_worker::WorkerThread` 起動 |
| monitor_loop | 336-537 | 202 | F | 詳細は下記 |

monitor_loop の中身(F の分割案):
- COM STA 初期化、TSF プロファイル生成: 残す(E)。
- 「2 秒ごとに IME 種別をポーリング」「GJI 再接続間隔」というスケジューリング(`next_clsid_check_ms`、`next_attach_ms`)、デバウンスへの投入、「`TSF_OBS` のどのフィールドを何で更新するか」: core 側へ。`MonitorState { next_attach, next_clsid, debounces } + fn step(now, ObsInputs) -> Vec<ObsUpdate>` の形にすれば、`ObsUpdate`(SetKind、SetIdentity、SetIo{...}、PostImeKindChanged)を Windows 側 executor が適用するだけになる。
- `WM_IME_KIND_CHANGED` の post は E。

#### tsf/win_event_obs.rs(本体 242、テストなし)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| 定数・ヘッダ(クラス名 2、IME イベント定数 3) | 1-28 | 28 | G | |
| WinEventHookGuard(RAII) | 30-50 | 21 | E | `UnhookWinEvent` |
| install_observation_hooks | 52-128 | 77 | E | `SetWinEventHook` ×3(NAMECHANGE / SHOW-HIDE / IME_SHOW-IME_CHANGE) |
| observation_event_proc | 130-225 | 96 | G | `extern "system"` コールバック。(event, クラス名) を `TSF_OBS` の flag/counter 更新に翻訳する入口。純粋に切れる部分は `classify(event, class) -> Option<ObsEvent>` の `match`(約 40 行)で、残りはログ整形。翻訳先の core Event: `CandidateShown`/`CandidateHidden`/`ImeShow`/`ImeHide`/`ImeChange`/`NameChange(CASCADIA)` |
| hwnd_class_name | 227-242 | 16 | O | `GetClassNameW` → `String` |

#### tsf/tip_detector.rs(本体 349、テストなし)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| ヘッダ・statics(GJI_CLSID, PROFILE_DESCRIPTIONS) | 1-46 | 45 | G | プロセス内 `OnceLock`/`RwLock` |
| create_profile_ctx | 50-68 | 19 | O | `CoCreateInstance` → `(ITfInputProcessorProfileMgr, ITfInputProcessorProfiles)` |
| discover_and_cache_gji_clsid | 72-93 | 22 | F | キャッシュ方針(1 回だけ発見、冪等)+ 列挙呼び出し。小さい |
| find_gji_clsid | 95-126 | 32 | O | `EnumProfiles(0x0411)`。「description に "Google" を含む」判定が 1 行混入(P 化できる) |
| enabled_ja_tips | 128-154 | 27 | O | `Vec<EnabledJaTip>` を返す(型は state::ime_kind 側で ungated) |
| query_active_kind | 156-200 | 45 | F | `GetActiveProfile` の読みの後に `identify_tip`/`identify_hkl_by_enabled_tips`(純粋、state::ime_kind に既出)を呼び、ついでに `TSF_OBS.set_ime_product_name` へ書く。分割案: O は `ActiveProfileRaw { clsid:u128, profile_type, langid, desc }` を返す。写像(identity → ActiveImeKind)は core |
| query_tip_identity_on_current_sta | 202-236 | 35 | O | `pub`。学習プロセス(awase-keymap-learn-win)から呼ばれる。`TSF_OBS` に触らないよう設計済み |
| dump_profiles | 238-289 | 52 | O | 診断ログ + `cache_profile_description` への書き込み |
| fmt_guid | 293-308 | 16 | P* | seam: `windows::core::GUID` → `u128` |
| cache_profile_description / cached_profile_description / profile_description_matches | 310-349 | 40 | P* | `TF_INPUTPROCESSORPROFILE` を引数に取るが使うのは clsid/langid/guid の 3 値。seam: `(u128, u16, u128)` タプル化 |

純粋な判断(`identify_tip` 等)は既に `state/ime_kind.rs` に出ており、tip_detector 側には薄い COM 呼び出ししか残っていない。分離はほぼ完了している。

#### tsf/probe_bridge.rs(149 行)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| OutputGate(3 atomic)+ OUTPUT_GATE static | 11-65 | 55 | G | グローバル static。`lib.rs:118` で re-export。外部参照 59 箇所(`OUTPUT_GATE` の grep) |
| OUTPUT_GATE_TEST_LOCK | 67-75 | 9 | T 支援 | `#[cfg(test)]` の static |
| OutputActiveGuard(begin/Drop) | 77-139 | 63 | G | 深度カウンタの 0→1 / 1→0 遷移は純粋。Drop が `INPUT_DEFER` と `post_drain_output_queue` を触る。`real` フィールド(BUG-65 追補 2)は noop_for_test 用 |
| WM_DRAIN_OUTPUT_QUEUE(WM_APP+18) | 141-144 | 4 | G | カスタムメッセージ |
| post_drain_output_queue | 146-149 | 4 | E | `post_to_main_thread` |

#### tsf/output.rs(186 行)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| INJECTED_MARKER / TSF_MARKER / IME_KANJI_MARKER | 14-27 | 14 | P | 単なる `usize` 定数。hook 側が見分けに使う |
| ColdReason(11 variant)+ is_confirm_key / requires_settle | 29-79 | 51 | P | 純粋な enum と述語。`probe.rs`・`warmup/`・`output/`・`runtime/`・`platform.rs` など 10 ファイルが参照。tsf/output.rs 自体が Win 型を import するので ColdReason だけ切り出す価値が高い(seam: ungated な場所へ移動) |
| make_scan_key_input | 81-110 | 30 | E | `MapVirtualKeyW` を呼んで `INPUT` を組む |
| make_tsf_key_input | 112-117 | 6 | E | 上の薄いラッパ |
| make_key_input_ex | 119-141 | 23 | P* | OS 呼び出しなしで `INPUT` を組むだけ。seam: `KeySpec{vk, up, extra}` を core が返し、`INPUT` 変換は Windows 側 |
| kana_for_romaji_static | 143-150 | 8 | P | `awase::kana_table`(core)の `LazyLock` |
| flush_raw_tsf_literal_backspaces | 152-186 | 35 | F | `RAW_TSF_LITERAL`(グローバル static)から `backs`/`escape_composition` を swap で取り、「ESC 先頭 + BS を n 回」の列を組み、`send_input_safe` で送る。分割案: core `plan_literal_flush(n, escape) -> Vec<KeySpec>`、Windows が swap と送信 |

#### tsf/ime_mode_fsm.rs(本体 1-204)

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---:|---|---|
| ImeModeState / Default | 1-31 | 31 | P | |
| ImeModeState::from_conversion_mode | 32-42 | 11 | P* | seam: `crate::imm::IME_CMODE_NATIVE/KATAKANA`(0x0001/0x0002)。値を awase core 側の ConvMode 定数に寄せる |
| ImeModeFsm(struct/new/getter/is_native_ready/unconfirm/on_conversion_mode_read/on_conversion_mode_hint/on_focus_changed) | 44-203 から on_set_open_applied を除く | 148 | P | `on_focus_changed` は既に `now_ms` を引数に取る(良い形) |
| ImeModeFsm::on_set_open_applied | 88-112 | 25 | P* | seam: `hook::current_tick_ms()` を `now_ms` 引数化(他の `on_focus_changed` と揃う) |

(P=139、P*=65 は範囲合計。on_set_open_applied 25 + from_conversion_mode 11 + 他の P* の端数 29 = 65。端数は doc/空行。)
利用元: `Output.ime_mode_fsm: RefCell<ImeModeFsm>`(output/mod.rs:98)、borrow は 7 箇所(output/vk_send.rs、conv_actuation.rs)。

#### tsf/tsf_gate.rs(本体 1-369)

全体が P(369 行)。`TsfGateMachine`(timed-fsm の `TimedStateMachine` 実装、113-184)、`TsfGate`(`HoldingGate<TsfGateMachine, RawKeyEvent>` ラッパ、203-305)、`TsfReadiness`(342-366、bool 3 つの合成述語)。tracing 以外の副作用なし、時計なし(500ms の timer は `Response::with_timer` で宣言するだけで、実際の SetTimer は呼び出し側 `TIMER_TSF_GATE`)。依存は `timed_fsm`(独立 crate)と `awase::types::RawKeyEvent`(core)。**ungated にするだけで Linux に移せる**(mod.rs の `#[cfg(windows)]` を外すのが唯一の変更)。テスト 346 行(19 テスト)もそのまま回る見込み(未確認: 実際に Linux でビルドして確認はしていない)。

#### tsf/gji_fsm.rs(本体 1-1075)

全体が P。型: `FocusEpoch`/`ProbeId`/`ProbeParams`/`StageEndReason`/`PendingInput`/`ColdKind`/`ProbeStatus`/`ComposingWarmup`/`GjiState`/`GjiEvent`/`GjiAction`/`PendingDiscardReason`/`GjiTimer`(43-310)、`GjiFsm`(312-1029、`TimedStateMachine` 実装)、`long_idle_ms_for`(1031)、`GjiState::state_label`(1039-)。

最大の関数は `on_event`(600-997、398 行)。時計は持たず、`gji_idle_ms` を `GjiEvent` の必須パラメータとして受け取る(BUG-033 追補 3・4 の教訓、ime-belief-architecture.md の「ImeModel 以外への適用範囲」節)。依存は `crate::tuning`(定数のみ)と `state::injection_mode::InjectionMode`(ungated)。tracing 呼び出しが 26 箇所。テスト 1554 行(52 テスト)は既に Linux で回る。
→ 新 crate へ移すときの seam は「`tuning::*` 定数の参照先」と「`InjectionMode` の所在」の 2 点のみ。

#### tsf/literal_facts.rs(本体 1-256)

全体が P。`LiteralVerdict`/`DetectRoute`/`DetectPath`/`DetectTarget`/`DetectEvidence`(診断専用フィールド 9 個を含む)/`LiteralDetectFacts`/`LiteralDetectRecord`/`GiveUpEvidence`/`GiveUpTracker`(note_vk_sent、note_record、145-197)/`GiveUpFollowDecision` + `giveup_follow_decision`(199-240、ADR-227 の判断)/`LiteralDetectTrace`。依存は `state::event_origin::Generation` と serde/strum のみ。既に Linux でテスト済み(217 行、10 テスト)。ADR-227 の判断部分がここに切り出されている点は、他ファイルが目指す形の手本。

#### tsf/mod.rs(57)/ tsf/send.rs(4)

mod.rs は G(モジュール宣言と re-export、`TsfGate` 等の公開、`query_tip_identity_on_current_sta` の再公開)。send.rs は空。

## 2. 移せる行数 / 残る行数 / 分割が要る行数

| 区分 | 行数(本体) | 内訳 |
|---|---:|---|
| 移せる(P+P*) | 3094 | P 2540(gji_fsm 1075、tsf_gate 369、literal_facts 256、probe 458、ime_mode_fsm 139、output 73、gji_monitor 99、observer 71)+ P* 554(probe 329、ime_mode_fsm 65、observer 81、tip_detector 56、output 23)。**P* のうち 460 行以上は seam が「時計」か「`TSF_OBS` 直読み」の 2 種類だけ** |
| 残る(O+E+G) | 1460 | O 540、E 138、G 782 |
| 分割が要る(F) | 350 | gji_monitor::sample 46 + monitor_loop 202、tip_detector::discover 22 + query_active_kind 45、output::flush_raw_tsf_literal_backspaces 35 |
| 空行/空 | 79 | |

注意: G の 782 行のうち約 372 行(observer.rs の `TsfObservations` 定義と setter)は「グローバル観測ストア」で、中身は atomic の集合。構造として core に置けるのは**「観測値を載せる型」**であって、**グローバルとしての保持**ではない。core 側が `ObsSnapshot` を受け取る形にすれば、この 372 行は Windows 側に残る。

## 3. ファイルが持つ Windows 側の依存の種類と件数

(コメントを除く。件数は本体のみ、grep による)

| 依存の種類 | 件数 | 現れるファイル |
|---|---:|---|
| `crate::hook::current_tick_ms()`(= `GetTickCount64`、時計の直読み) | 本体 約 17(probe 9、gji_monitor 4、win_event_obs 2、observer 1、ime_mode_fsm 1) | probe.rs、gji_monitor.rs、win_event_obs.rs、observer.rs、ime_mode_fsm.rs |
| `TSF_OBS`(グローバル static、`pub(in crate::tsf)`、tsf/ 外は `tsf_obs()` 経由) | 本体 約 84(gji_monitor 25、observer 29、win_event_obs 15、probe 9、tip_detector 5、probe_bridge 1) | 6 ファイル。tsf/ 外からの `tsf_obs()`/`tsf::observer::` 参照は 18 ファイル(platform.rs、ime.rs、ime_controller.rs、output/*、runtime/*、state/ime_decision_view.rs 等) |
| `OUTPUT_GATE`(グローバル static) | 定義 1、外部参照 59 | probe_bridge.rs(定義)、lib.rs:118 で re-export |
| `RAW_TSF_LITERAL`・`INPUT_DEFER`(`crate::` 直下のグローバル static) | 3 | output.rs:166-167、probe_bridge.rs:131 |
| スレッド | 2(`gji-io-monitor` ワーカー = `win32_worker::WorkerThread` と、WinEvent の OUTOFCONTEXT コールバックがメッセージループスレッドで動く) | gji_monitor.rs、win_event_obs.rs |
| COM(STA、`CoInitializeEx` / `CoCreateInstance`) | 2 箇所 | gji_monitor.rs:342、tip_detector.rs:57 |
| TSF インターフェース(`ITfInputProcessorProfileMgr`、`ITfInputProcessorProfiles`、`TF_INPUTPROCESSORPROFILE`) | 型 3 種、呼び出しは `EnumProfiles` 3 箇所 + `GetActiveProfile` 2 + `GetLanguageProfileDescription` 3 | tip_detector.rs のみ |
| ToolHelp / `OpenProcess` / `GetProcessIoCounters`(プロセス I/O カウンタ) | 5 API | gji_monitor.rs のみ |
| WinEvent(`SetWinEventHook`/`UnhookWinEvent`/`GetClassNameW`) | 3 API(フック 3 本) | win_event_obs.rs のみ |
| SendInput 系(`INPUT`、`MapVirtualKeyW`、`send_input_safe`) | 3 | output.rs のみ |
| カスタム WM の post(`post_to_main_thread`) | 3(`WM_IME_KIND_CHANGED` ×2〜3、`WM_DRAIN_OUTPUT_QUEUE` ×1) | gji_monitor.rs(3 呼び出し)、probe_bridge.rs(1)。定義: `lib.rs:333` と probe_bridge.rs:144 |
| HWND | 1 引数型(`hwnd_class_name`、`observation_event_proc`) | win_event_obs.rs のみ |
| `crate::imm::IME_CMODE_*` 定数 | 2 | ime_mode_fsm.rs |
| `with_app` / `crate::APP` | **0** | tsf/ 直下には現れない |
| `thread_local` | **0** | 同上 |

### 3.1 COM/TSF 固有の依存の一覧(依頼分)

- ITf 系: `ITfInputProcessorProfileMgr`(`GetActiveProfile`、`EnumProfiles`)、`ITfInputProcessorProfiles`(`GetLanguageProfileDescription`)。使うのは tip_detector.rs だけ。
- アパートメント: `gji-io-monitor` スレッドが `COINIT_APARTMENTTHREADED` で STA 初期化(gji_monitor.rs:342)。COM オブジェクトは生成スレッドに束縛される(tip_detector.rs の doc 冒頭)。`query_tip_identity_on_current_sta` は呼び出し側の STA に依存(学習プロセスが使う)。
- カスタムメッセージ: `WM_IME_KIND_CHANGED`(WM_APP+21、gji_monitor.rs が post)、`WM_DRAIN_OUTPUT_QUEUE`(WM_APP+18、probe_bridge.rs が post)。どちらも main スレッドへの `PostMessage`。core の Event に翻訳される入口は message_handlers 側(tsf/ 外)で、tsf/ 内に受け口はない。
- スレッド: 上記ワーカー 1 本。書き込み先 `TSF_OBS` は atomic のみ(+ `RwLock<Option<String>>` 1 個)なのでロック競合はほぼない。
- 時計: `GetTickCount64` 直読み(ms 単位、ワーカースレッドと main の両方から)。

## 4. seam 一覧

| seam の種類 | 関数数 | ファイル数 | 該当 |
|---|---:|---:|---|
| 時計の直読み → `now_ms` 引数化 | 約 9(probe.rs: check_outcome, check_now, ms_since_last_send ×2, update_last_send_ms ×2, mark_composition_cold 経由, LiteralDetector::new ×2, evidence_now, check_now, visible_fencing_verdict)+ ime_mode_fsm.rs::on_set_open_applied + observer.rs::gji_idle_ms | 3 | probe.rs、ime_mode_fsm.rs、observer.rs。`ime_mode_fsm::on_focus_changed` は既に引数化済みで手本になる |
| `TSF_OBS` 直読み → 所有スナップショット(`GjiObs`/`LiteralObs`)を引数に | 約 8(probe.rs: check_outcome, check_now, LiteralDetector::new ×2, evidence_now, check_now, visible_fencing_verdict + observer.rs: gji_is_active_ime, literal_session_confirmed, active_ime_kind, table_ime_kind, current_tip_identity) | 2 | probe.rs、observer.rs。**スナップショット型 `ObservedState::from_snapshot()` の考え方は既に state/ime_decision_view.rs にある**(observer.rs:8 の doc)。probe.rs は「tick 境界外での非一貫観測を避ける」ルール(observer.rs:8-9)の例外になっている |
| `crate::output::ColdReason` を ungated へ | 参照 10 ファイル(tsf/ 内: probe.rs 5 箇所) | 1(定義)+ 参照側 10 | tsf/output.rs の ColdReason(51 行)だけを切り出せば probe.rs の ColdContext/CompositionState を ungated にできる |
| `crate::imm::IME_CMODE_*` → core の ConvMode 定数 | 1 | 1 | ime_mode_fsm.rs:33-42 |
| `windows::core::GUID` / `TF_INPUTPROCESSORPROFILE` → 値タプル | 4 | 1 | tip_detector.rs: fmt_guid, cache_profile_description, cached_profile_description, profile_description_matches |
| `INPUT` 構築 → `KeySpec` | 3 | 1 | output.rs: make_key_input_ex(P*)、make_scan_key_input / make_tsf_key_input(E) |
| グローバル static の swap/取り出し → 計画関数 + 実行 | 3 | 3 | output.rs::flush_raw_tsf_literal_backspaces、gji_monitor.rs::monitor_loop(TSF_OBS 書き込み)、tip_detector.rs::query_active_kind(set_ime_product_name 書き込み) |
| 単に `#[cfg(windows)]` を外すだけ | 2 ファイル | 2 | tsf_gate.rs(369 行 + テスト 346)、gji_monitor.rs の `Debounce`(57 行 + テスト 130 を別ファイルへ) |
| 重複ロジックの 1 関数化(`evidence_is_fresh`) | 3 箇所 | 1 | probe.rs:639-648、723-726、803-805 |

## 5. 新 crate への移動候補の順序案(依存の少ない順)

1. **tsf_gate.rs**(369 行 + テスト 346): 変更ゼロ。mod.rs の `#[cfg(windows)]` を外すだけ(`timed_fsm`・`awase::types` のみ依存)。
2. **gji_fsm.rs**(1075 + テスト 1554)と **literal_facts.rs**(256 + テスト 217): 既に ungated。依存は `tuning` 定数、`InjectionMode`、`Generation`。そのまま移せる。
3. **tsf/output.rs の `ColdReason`**(51 行)と 3 つの MARKER 定数(14 行): 切り出して ungated へ。これで 4 の前提が揃う。
4. **gji_monitor.rs の `Debounce`**(57 行 + テスト 130)と `is_gji_process`(15 行)と `GjiIoDelta`(27 行): 純粋部分を別ファイルへ。
5. **probe.rs の `WarmEpoch`/`ColdContext`/`CompositionState`**(約 350 行): 時計を `now_ms` 引数にして移す(ColdReason が前提)。
6. **ime_mode_fsm.rs**(204 + テスト 102): `from_conversion_mode` の定数と `on_set_open_applied` の時計の 2 点を直す。
7. **probe.rs の `TsfReadinessProbe`**(約 125 行): `ProbeObs`(now_ms、monitor_ok、gji_last_io_ms)を引数にする。
8. **probe.rs の `LiteralDetector` + `DetectionResult`**(約 336 行): `LiteralObs` スナップショットを導入し、`evidence_is_fresh` を 1 関数に集約してから移す。ここが唯一「関数の形を変える」作業。
9. **observer.rs の `ChangeCounter`/`ActiveImeKind` と P* 5 関数**(約 150 行): 引数化。`TsfObservations` 本体とグローバル `TSF_OBS` は Windows 側に残す。
10. **F の分割**: output::flush_raw_tsf_literal_backspaces(35)→ tip_detector::query_active_kind(45)→ GjiMonitor::sample(46)→ monitor_loop(202)。monitor_loop は最後(タイミング方針とデバウンスの適用を core の `MonitorState::step` に出す。最も大きく、スレッド境界の変更を伴う)。

Windows 側に残る: win_event_obs.rs 全体(242)、tip_detector.rs の COM 呼び出し、GjiMonitor の OpenProcess/GetProcessIoCounters、probe_bridge.rs(OutputGate と WM post)、observer.rs のグローバル `TSF_OBS`。

## 6. 危険箇所

1. **`evidence_is_fresh` の 3 重複(probe.rs:639-648、723-726、803-805)。** 分割時にどれか 1 箇所だけ直すと、診断用 `DetectEvidence.evidence_fresh` と実際の verdict がずれる。BUG-75 と ADR-079(epoch fencing)が絡む。1 関数にまとめてから動かすこと。
2. **`grace_hold_verdict` の `show_stale_hold_since_ms: Cell<Option<u64>>`**(probe.rs:528、747-781)。`check_now` 経路と `visible_fencing_verdict` 経路は「1 detector につきどちらか一方しか通らない」ことを doc(757-762)だけで保証している。純粋化して状態を外に出すとき、この前提が崩れる設計にしないこと(ADR-079、BUG-27 追補5、BUG-30)。
3. **await をまたぐ失効**: `LiteralDetector` は構築時点の `epoch_send_ms` と baseline を保持し、tick ごとに `TSF_OBS` を読み直して判定する。core 側で `LiteralObs` を「構築時 1 回」取る形にすると、**tick ごとの最新観測**が必要な `check_now` が古いスナップショットで判定する事故になる。snapshot は「baseline(1 回)」と「now の観測(tick ごと)」の 2 種類に分けること。`TsfReadinessProbe` も同様(`settled_at_ms` が tick をまたぐ状態)。
4. **グローバル観測ストアの tick 境界ルール**: observer.rs:8-9 は「判断層は `ObservedState::from_snapshot()` 経由のスナップショットを使え、`tsf_obs()` を直接呼ぶな」と書くが、probe.rs はこのルールの外で `TSF_OBS` を直読みしている(tsf/ 内なので `pub(in crate::tsf)` が許す)。スナップショット化は INV-45 の「推測値で非対称な選択をしない」の延長で、tick 内で一貫した観測にする意味もある。ただし `TSF_OBS_TEST_LOCK`(observer.rs:457-470、BUG-65)が示す通り、テストは static を共有している。引数化すればこのロックは不要になる(効果として良い)。
5. **`GjiFsm`/`GjiMonitor` の belief 監査**: `GjiEvent` が `gji_idle_ms` を必須にしているのは、`gji_candidate_visible_now()` の素の AtomicBool 読みで belief を書き換えて起きた実機バグ(BUG-033 追補 3・4、ime-belief-architecture.md 末尾節)への対策。monitor_loop を core に出すときも、`TSF_OBS` 書き込みを「観測 → 純粋な更新計画 → 適用」にそろえ、直接 belief を触らないこと。
6. **デバウンス 2 系統(`ImeKindDebounce` と `TipIdentityDebounce`)**(gji_monitor.rs:293-294、BUG-179・レビュー round3 NR1)。分離済みだが、`query_active_kind` が 1 回の呼び出しで 2 値を返す前提で作られている。F の分割で `ActiveProfileRaw` を返すように変えるとき、2 つのデバウンスが同じ入力から駆動される前提を維持する。
7. **`WM_IME_KIND_CHANGED` の post 条件**(gji_monitor.rs:364、395、441)。「プロセス存在ではなく CLSID 結果の変化時のみ」「消失時は発行しない」という意図(doc 460-461、472-473)が `monitor_loop` のコメントに散在。`ObsUpdate::PostImeKindChanged` に分けるなら、発行条件をテストで固定してから(BUG-34 型の「隙間」を作らない)。
8. **drain は 1 回限り**: `take_pending_start/end_composition`(observer.rs:673-688)は swap(false) で読むと消える。スナップショット化すると「同じ tick に 2 回読む」で結果が変わる。純粋な読み取りとして扱わず、E 相当の「取り出し」として別扱いにすること。`discard_pending_composition_events`(697-701)を IME OFF / FocusChange の GjiFsm 通知の**直前**に呼ぶ順序(doc 690-696)は、core 側の Event 順序として保存すること。
9. **`OutputActiveGuard`**(probe_bridge.rs:77-139): `Drop` 内で `post_drain_output_queue` を呼び、`TsfProbeData` がガードを保持し続けることで `OUTPUT_GATE.active` を維持する(doc 82-83)。core に深度カウンタだけを出すと、**Drop による自動 post** が Windows 側に残り、片方だけ変えると defer/replay の解放窓口が片方だけ変わる(fix-requires-evidence.md の defer/replay 行、ADR-156、ADR-123→ADR-128 の回帰)。移さないほうが安全。
10. **ColdReason の移動**: 10 ファイルが参照し、journal(`crate::journal`)にも載る。名前と variant 順を変えない(診断・journal の互換)。

## 7. 未確認

- tsf_gate.rs と probe.rs の P 部分が実際に Linux でビルド・テストできるかは**未確認**(ビルド・実行は禁止のため。`windows::` トークンと unsafe が無いことを grep で確認しただけ)。
- `gji_monitor.rs` の tests 130 行(`ime_kind_debounce_tests`)が `Debounce` 以外に依存しないかは冒頭 20 行しか読んでいない(未確認)。
- 外部参照の数(`OUTPUT_GATE` 59、`tsf_obs()` 30、`ColdReason` 参照 10 ファイル)は grep の単純件数で、コメント中の言及を含む可能性がある。
- `observation_event_proc` が走るスレッド(OUTOFCONTEXT でメッセージループスレッド)と `gji-io-monitor` の 2 スレッドが `TSF_OBS` に書く際の順序保証(Relaxed/Acquire/Release が混在、観測値は個別 atomic)の妥当性は評価していない。
- probe.rs の `TsfReadinessProbe::check_now` は doc(109-120)通り本番で実質 `true` 固定の可能性があり、実際の呼び出し元(`cold_warmup.rs::run_start`、`vk_send.rs`)は tsf/warmup(inv-f2 担当)と output/ にあるため、この領域では確認していない。
- 分類ごとの行数は範囲合計で、コメントの多い領域(observer.rs はコメント 376 行 / コード 279 行)では行数の比率がロジック量を過大に見せる。
