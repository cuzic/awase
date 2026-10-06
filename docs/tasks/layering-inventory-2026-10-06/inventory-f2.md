# 棚卸し F2: tsf/warmup/ と state/ の ungated ファイル

対象: develop 9df983e4。読み取り専用。ビルド・テストは実行していない。
行数は `wc -l` と、`#[cfg(test)] mod` 境界での自前スクリプトによる分割。
本体行数にはコメント・空行を含む。

---

# 第1部 crates/awase-windows/src/tsf/warmup/

## 1. 先に結論

- warmup/ 全体は `tsf/mod.rs:49-50` の `#[cfg(windows)] pub(crate) mod warmup;` 1 行で gate されている。
- 中に `windows::` トークンは 0 個。
- 唯一 Win32 を直接叩くのは `cold_warmup.rs` だけ。
- それ以外の 10 ファイルは、次の 5 種の依存で縛られているだけ。
  - `crate::hook::current_tick_ms()`（GetTickCount64）
  - `TSF_OBS`（プロセス内 static のアトミック群）
  - `OutputActiveGuard`（static `OUTPUT_GATE` のアトミック）
  - `log_composition_probe`（中で IMM32 を同期で呼ぶ診断）
  - gate された型 3 つ（`ImeModeState` / `ColdReason` / `LiteralDetector` 系）
- 「コルーチンを OS 非依存 crate に移し、時計と effect を差し替える」構想は、このまま実現できる。
- 理由は、既に `ProbeAction`（effect 側）と `ProbeIo` trait（dispatch 側）がある点。
- 要る継ぎ目は次の 3 つ。
  - `LiteralDetector` / `TsfReadinessProbe`（tsf/probe.rs）を、グローバル読みから「スナップショットを引数に取る純粋型」にする。
  - `now_ms` と GJI 観測スナップショットを tick 入力に載せる。
  - `OutputActiveGuard` と `log_composition_probe` を effect 化する。
- `LiteralDetector` の件が最大の山。warmup/ の外（tsf/probe.rs）にある。

## 2. ファイル別の行数と分類

「本体」は `#[cfg(test)]` より前の全行（doc・import・空行を含む）。
分類は関数・型の範囲（シグネチャから閉じ括弧まで）で数えた。残りは「doc・import・空行」。

| ファイル | 全体 | 本体 | テスト | #[test] | P | P* | F | doc・import 等 |
|---|---|---|---|---|---|---|---|---|
| probe_fsm.rs | 1484 | 880 | 604 | 14 | 140 | 600 | 0 | 140 |
| literal_detect_fsm.rs | 919 | 529 | 390 | 14 | 65 | 304 | 0 | 160 |
| ms_ime_ready_coro.rs | 447 | 217 | 230 | 9 | 0 | 136 | 0 | 81 |
| gji_warmup_coro.rs | 340 | 340 | 0 | 0 | 7 | 258 | 0 | 75 |
| chrome_probe.rs | 169 | 65 | 104 | 2 | 0 | 52 | 0 | 13 |
| probe_coro_state.rs | 138 | 138 | 0 | 0 | 0 | 114 | 0 | 24 |
| warmup_strategy.rs | 136 | 136 | 0 | 0 | 104 | 0 | 0 | 32 |
| cold_warmup.rs | 123 | 123 | 0 | 0 | 0 | 8 | 84 | 31 |
| unicode_literal_observer.rs | 95 | 95 | 0 | 0 | 0 | 46 | 0 | 49 |
| tickable_fsm.rs | 92 | 92 | 0 | 0 | 0 | 44 | 0 | 48 |
| mod.rs | 41 | 41 | 0 | 0 | 0 | 0 | 0 | 41 |
| 合計 | 3984 | 2656 | 1328 | 39 | 316 | 1562 | 84 | 694 |

- O / E / G は 0。コルーチン本体は Win32 を直接呼ばない。
- `cold_warmup.rs::run_start` の中に O と E が混ざっているので F とした。
- 「移せる行数(P+P*)」は 1878（本体の 71%）。
- 「分割が要る行数(F)」は 84（`cold_warmup.rs::run_start` とその周辺）。
- 「残る行数(O+E+G)」は、F を割った後に Windows 側へ残る分で、約 40 行。
- テスト 1328 行（39 本）は gate されたファイルの中にある。
  - 全て `crate::hook::current_tick_ms()` と `TSF_OBS` の実グローバルを使う。
  - `TSF_OBS_TEST_LOCK` / `OUTPUT_GATE_TEST_LOCK` で直列化している。
  - Linux には存在せず、Windows CI（windows-build）でしか走らない。
  - 移した後に Linux で回せるかは、継ぎ目を入れたあとに決まる。
  - 入れれば、ロックなしの純粋テストに書き直せる。

## 3. 関数表（全関数）

凡例の「seam」は §6 の記号 S1〜S9 を指す。

### probe_fsm.rs（本体 880）

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| DeferredOrigin / DeferredVk | 47-59 | 4 / 6 | P | VkCode は core crate 型 |
| TsfEnvSnapshot | 69-92 | 24 | P* | S5。`ime_mode: ImeModeState` が gate されたファイルの型。他の 7 フィールドは素の値 |
| ProbeObservations | 96-111 | 16 | P | |
| TransmitPlan | 115-122 | 8 | P | |
| decide_transmit_plan | 127-173 | 47 | P* | S5。TsfEnvSnapshot 経由のみ。`tuning::RAW_TSF_LITERAL_DETECT_MS*` は ungated の定数 |
| TransmitTarget と From<DetectTarget> | 177-189 | 4 / 8 | P | |
| ProbeAction | 193-269 | 77 | P | yield する型。全フィールドが OS 非依存（Generation / VkCode / String / TransmitPlan / LiteralDetectFacts） |
| ProbeTickInput | 277-281 | 5 | P* | S1。`TransmitDonePayload` / `VkSentPayload` 経由で `LiteralDetector` を抱える |
| TransmitDonePayload | 285-291 | 7 | P* | S1（detector）、S2（deadline_ms を呼び出し側が `now` から計算） |
| VkSentPayload | 296-299 | 4 | P* | S1 |
| per_vk_confirm_tags | 321-334 | 14 | P | |
| idx_u16 | 336-338 | 3 | P | |
| literal_facts | 340-360 | 21 | P* | S6。引数の `DetectionResult` が gate された probe.rs の enum（中身は 3 値の純粋 enum） |
| await_vk_detection | 371-445 | 75 | P* | S1。`visible_fencing_verdict` / `check_now` / `evidence_now` はグローバルと時計を直読み |
| run_per_vk_confirm | 448-646 | 199 | P* | S1、S4（`log_composition_probe` ×3: 559, 602, 621）、S7（`crate::output::resolve_ascii_to_vk` は `vk::ascii_to_vk` の再エクスポート、本体は ungated の const fn） |
| tsf_probe_coro_body | 653-810 | 158 | P* | S1、S2（`probe.check_outcome` が時計と TSF_OBS を読む）、S4（×3: 745, 767, 788） |
| TsfProbeCoro | 818-879 | 9 / 19 / 32 | P* | S3。`_guard: OutputActiveGuard` を保持。`new_chrome` が guard を受け取る |

### literal_detect_fsm.rs（本体 529）

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| PARTIAL_LITERAL_BS | 53 | 1 | P | |
| per_vk_recovery_params | 89-92 | 4 | P | const fn。BUG-33 追補 4 の「BS には literal の証拠を要求」がここ |
| word_level_recovery_params | 120-129 | 10 | P | |
| is_partial_literal | 139-145 | 7 | P | |
| emit_recovery_actions | 162-179 | 18 | P | |
| word_facts | 181-197 | 17 | P | |
| VetoDecision | 463-470 | 8 | P | |
| LiteralDetectCore（struct） | 204-232 | 29 | P* | S1。`detector: LiteralDetector` を保持 |
| LiteralDetectCore::new | 237-258 | 22 | P* | const fn |
| LiteralDetectCore::poll | 265-415 | 151 | P* | S1。S4（×5: 322, 338, 366, 392, 409）。S8（`tsf::observer::gji_idle_ms()` ×4、ログ引数のみ）。判定本体は env と detector の返り値だけで決まる |
| LiteralDetectCore::recovery | 417-430 | 14 | P | |
| LiteralDetectCore::veto_decision | 446-458 | 13 | P* | S2。`crate::hook::current_tick_ms()` を直読み（451）。veto の上限 `GJI_CANDIDATE_VETO_CAP_MS` と比較 |
| LiteralDetectFsm | 476-526 | 5 / 35 / 9 | P* | S1、S2、S3。`new` が `OutputActiveGuard::begin()`、`LiteralDetector::new(true)`、`current_tick_ms()` を直接呼ぶ（499-501）。生成元は `output/vk_send.rs:624` |

### ms_ime_ready_coro.rs（本体 217）

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| env_native_ready | 69-75 | 7 | P* | S5（`ImeModeState`） |
| ms_ime_ready_coro_body | 80-167 | 88 | P* | S2（`current_tick_ms()` を 3 回: 90, 96, 113）。S3（149 で `OutputActiveGuard::begin()`。BUG-58 のため Phase 2 直前のみ） |
| MsImeReadyCoro | 174-216 | 8 / 21 / 12 | P* | 入力型は `TsfEnvSnapshot`（`ProbeTickInput` ではない）。`new` が `prime()` を自前で呼ぶ |

### gji_warmup_coro.rs（本体 340）

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| GjiProbeCtx | 53-59 | 7 | P | |
| gji_coro_body | 66-244 | 179 | P* | S1、S2（`probe.check_outcome`）、S4（212）、S9（205-213 で `win32_async::spawn_local` + `sleep_ms` を、診断のためコルーチン内から起動） |
| GjiWarmupCoro | 258-340 | 7 / 31 / 41 | P* | S3。`literal_detect_guard` は `apply_transmit_done` / `apply_vk_sent` の中で `OutputActiveGuard::begin()`（325, 335）。状態更新のコールバック内でグローバルに触れている |

### 残りのファイル

| 名前 | 行範囲 | 行数 | 分類 | メモ |
|---|---|---|---|---|
| ProbeCoroState | probe_coro_state.rs 24-138 | 114 | P* | S2。`tick` のログ 1 箇所（74）と `apply_vk_sent` のログ 1 箇所（131）は時計をログ用に読むだけ。`apply_transmit_done:103` は `deadline_ms = current_tick_ms() + literal_detect_ms` で、判定に効く時計読み |
| ChromeProbe | chrome_probe.rs 11-64 | 52 | P* | `TsfProbeCoro` の 3 メソッド委譲だけ。`apply_vk_sent` の委譲漏れが BUG-27 の根本原因だった。移植時に消せる |
| UnicodeLiteralObserverFsm | unicode_literal_observer.rs 48-95 | 46 | P* | S8（`gji_write_bytes()` の読み）、S4（79）。「経過時間」は時計でなく `elapsed_ms += 10`（tick 数×10）。`OBSERVATION_WINDOW_MS = 100` |
| TickableFsm | tickable_fsm.rs 49-92 | 44 | P* | trait。`apply_transmit_done` / `apply_vk_sent` が `LiteralDetector` を引数に取る（S1）。デフォルト no-op の罠（§7 D2）。末尾の「UnicodeColdWarmupFsm のみ」節は実体が無い古いコメント |
| ImeWarmupStrategy / MsImeStrategy | warmup_strategy.rs 21-136 | 104 | P | 依存する `GjiFsm` / `GjiEvent` / `Response` は ungated。このファイルが gate されているのは親 mod のせいだけ。`Output` が `Box<dyn ImeWarmupStrategy>` で持つ |
| WarmupStarted | cold_warmup.rs 20-27 | 8 | P* | `TsfReadinessProbe` と `ColdReason` を持つ |
| ColdWarmupSequence + new + run_start | cold_warmup.rs 30-123 | 84 | F | §5 参照 |
| mod.rs | 1-41 | 41 | G | 宣言のみ。doc に存在しない `unicode_cold_warmup_fsm` が載っている（古い記述） |

## 4. warmup/ の各コルーチンが yield する型と受け取る入力

コルーチン本体は 3 つ。どれも timed-fsm の `StepCoro` を使う。

| コルーチン本体 | 構築 | 入力型 I | yield 型 Y | 時計の読み方 |
|---|---|---|---|---|
| `gji_coro_body`（gji_warmup_coro.rs:66） | `GjiWarmupCoro::new` | `ProbeTickInput` | `Vec<ProbeAction>` | 直接は 0。`probe.check_outcome` と `LiteralDetector` が GetTickCount64 を読む |
| `tsf_probe_coro_body`（probe_fsm.rs:653） | `TsfProbeCoro::new_chrome`（`ChromeProbe` 経由） | `ProbeTickInput` | `Vec<ProbeAction>` | 直接は 0。同上 |
| `ms_ime_ready_coro_body`（ms_ime_ready_coro.rs:80） | `MsImeReadyCoro::new` | `TsfEnvSnapshot` | `Vec<ProbeAction>` | 直接 `current_tick_ms()` を 3 回 |

- 上の 3 本が内部で `.await` する共有 async 関数が `run_per_vk_confirm` と `await_vk_detection`。これも時計は `LiteralDetector` 経由。
- コルーチンではなく素の状態機械で `TickableFsm::tick` を実装するのが `LiteralDetectFsm` と `UnicodeLiteralObserverFsm`。

### yield する型

- `ProbeAction`（probe_fsm.rs:193-269）。
- 7 バリアント: `Transmit` / `TransmitSingleVk` / `RawTsfLiteralRecovery` / `UpgradeToTsf` / `CompositionConfirmed` / `LiteralDetectNote` / `Done`。
- 中身は Generation / VkCode / String / `TransmitPlan` / `TransmitTarget` / `LiteralDetectFacts` / usize / bool だけ。
- **OS 非依存。今のまま移せる。**

### 受け取る入力

- `TsfEnvSnapshot`（8 フィールド）は全て素の値。唯一の例外が `ime_mode: ImeModeState`。
  - `ImeModeState` は `tsf/ime_mode_fsm.rs`（`#[cfg(windows)]`）の純粋 enum 4 値。
  - 同ファイルの `from_conversion_mode` が `crate::imm::IME_CMODE_*` を参照し、`on_set_open_applied:106` が `current_tick_ms()` を読む。
  - `ImeModeState` 自体は、型だけ切り出せば Windows 依存はない。
- `ProbeTickInput` は `{ env, transmit_done: Option<_>, vk_sent: Option<_> }`。
  - この 2 つのペイロードが `LiteralDetector` を抱えている。
  - **入力側が OS 非依存でない原因はこれだけ。**

### Win32 を直接呼ぶ箇所

コルーチン 3 本の中での直接呼びは 0。間接的には 3 種。

- 時計。`TsfReadinessProbe::{check_now, check_outcome}` と `LiteralDetector::{new, new_with_pre_send_baseline, evidence_now, check_now, visible_fencing_verdict}` の計 7 箇所（tsf/probe.rs の 92, 122, 585, 613, 638, 694, 802）。
- グローバル読み。`TSF_OBS`（`gji_monitor_ok`, `gji_last_io_ms`, `gji_last_write_ms`, `gji_write_bytes`, `gji_write/read/other_ops`, `gji_candidate_show` の Baseline）。
  - `TSF_OBS` は `tsf/observer.rs` にあり `windows::` トークンは 0。プロセス内 static のアトミックなだけ。書き手は `gji_monitor.rs` と `win_event_obs.rs`。
- `log_composition_probe`（13 箇所）。これは本物の Win32。
  - `GetForegroundWindow` と IMM32 の同期呼び出しが約 9 回。`ImmGetContext`、`ImmGetCompositionStringW`×6、`ImmGetOpenStatus`、`ImmGetConversionStatus`。
  - `thread_local` の `TSF_PROBE_SNAP` と `with_app_ref` も読む。
  - `run_with_timeout` では守られていない（同ファイル内の別関数は守られている）。

### 時計は何で読んでいるか

- `crate::hook::current_tick_ms()`（`hook.rs:801`）。`GetTickCount64` の薄いラッパー。
- warmup/ の非テスト本体での直接読みは 9 箇所。
  - literal_detect_fsm.rs: 2（451, 501）
  - ms_ime_ready_coro.rs: 3
  - probe_coro_state.rs: 3（うち 2 つはログのみ）
  - cold_warmup.rs: 1（`TsfReadinessProbe::new` の起点）
- 間接読みが上の 7 箇所。
- `step_probe`（`output/mod.rs:969`）は tick ごとに `tick_t` を読むが、**ログにしか使わず、env には載せていない**。
  - 今 env に `now_ms` が無い。
  - `TsfEnvSnapshot` に `now_ms` を足し、`step_probe` が 1 回読んだ値を入れる形が最小。
- `UnicodeLiteralObserverFsm` だけは時計を読まず、tick 回数×10 を経過時間として扱う。`TIMER_TSF_PROBE` の周期が崩れると窓の長さが変わる。

## 5. cold_warmup.rs::run_start の分割案（F）

| 中身 | 区分 | 行 |
|---|---|---|
| `ActuationTarget::capture(focus_gen).await` | E/O | 76 |
| `get_ime_conversion_mode_raw_timeout_async(50).await` と `cmode_has` のログ | O | 80-87 |
| `set_ime_conv_for_target(..)` と `with_app` で世代再読み | E（F2 ではなく conv 書き込み） | 94-97 |
| `get_foreground_window_class()`（unsafe） | O | 105 |
| `self.output.composition.increment_cold_start_count()` / `last_cold_reason()` | state 更新 | 104, 106 |
| `conv_mutation_allowed` で書き込み有無を分岐 | 判断 | 45-93 |
| `TsfReadinessProbe::new(current_tick_ms(), cold_seq, 0)` | 組み立て | 112-117 |

- core に出すもの: 「事前待機なしで per-VK confirm に進む」という決定と、`cold_seq` の発行。どちらも小さい。
- 残すもの: conv の読み書き（async タスク）。ここは Windows 側の dumb executor で良い。
- 注意点: `capture` は最初の await より前に置くこと。opus レビュー指摘 2026-08-08、ADR-086 INV-14。
  - 分割すると、「どの await より前」という順序が core と executor の間に分かれる。
  - 順序を型で固定する手当てが要る。

## 6. 継ぎ目（seam）一覧

| 記号 | 継ぎ目 | 現れるファイル数 | 現れる関数・型 | 直し方 |
|---|---|---|---|---|
| S1 | `LiteralDetector` と `TsfReadinessProbe` が時計と `TSF_OBS` を直読み | 8 | `TransmitDonePayload` / `VkSentPayload` / `ProbeTickInput` / `await_vk_detection` / `run_per_vk_confirm` / `tsf_probe_coro_body` / `gji_coro_body` / `LiteralDetectCore`（struct、new、poll）/ `LiteralDetectFsm` / `TickableFsm` の 2 メソッド / `ChromeProbe` / `ProbeCoroState` | `GjiObsSnapshot { now_ms, monitor_ok, last_io_ms, last_write_ms, write_bytes, write/read/other_ops, show_count, candidate_visible, literal_session_gen }` を tick 入力に載せる。`LiteralDetector` を所有する値（baseline のみ、`Cell` は内部可変で OK）にして、`check_now(&self, obs: &GjiObsSnapshot, deadline)` に変える |
| S2 | `current_tick_ms()` の直読み | 4 | `ms_ime_ready_coro_body` / `LiteralDetectCore::veto_decision` / `LiteralDetectFsm::new` / `ProbeCoroState::{tick, apply_transmit_done, apply_vk_sent}` / `ColdWarmupSequence::run_start` | `TsfEnvSnapshot.now_ms` を追加。`apply_*` には `now_ms` 引数を足す |
| S3 | `OutputActiveGuard`（static `OUTPUT_GATE` のアトミック、フックが見る） | 5 | `TsfProbeCoro._guard` / `GjiWarmupCoro.literal_detect_guard` / `LiteralDetectFsm._guard` / `ms_ime_ready_coro_body` / `ChromeProbe::new` の引数 | コルーチンは「いま gate を保持したい」を値で返す（`wants_output_gate()` か `ProbeAction::HoldGate(bool)`）。実際の RAII は Windows 側の executor が機械の寿命に紐づけて保持 |
| S4 | `log_composition_probe` の同期 Win32 診断 | 4 | probe_fsm 6、literal_detect 5、gji 1、unicode 1 の計 13 箇所 | `ProbeAction::DiagCompositionProbe { cold_seq, label }` を yield して executor が実行。診断専用で判定に効かない |
| S5 | gate された純粋型 `ImeModeState` | 2 | `TsfEnvSnapshot` / `env_native_ready` | `ime_mode_fsm.rs` から `ImeModeState`（4 値 enum）だけを ungated に出す |
| S6 | gate された純粋型 `DetectionResult` と `ColdReason` | 4 | `literal_facts()` / `gji_coro_body` / `cold_warmup` / `WarmupStarted` | `DetectionResult` は ungated の `literal_facts.rs` に移す。`ColdReason` は `tsf/output.rs`（`windows::` の VK 入力型を import している別目的のファイル）にあるので、enum と `is_confirm_key` / `requires_settle` を ungated に出す |
| S7 | 再エクスポート経由のパス | 1 | `probe_fsm.rs:461` の `crate::output::resolve_ascii_to_vk` | `crate::vk::ascii_to_vk`（ungated の const fn）を直接参照 |
| S8 | `tsf::observer::` のスカラー直読み | 2 | `LiteralDetectCore::poll`（`gji_idle_ms` ×4、ログのみ）/ `UnicodeLiteralObserverFsm::tick`（`gji_write_bytes`、判定に使う） | S1 の snapshot に `gji_idle_ms` と `write_bytes` を含める |
| S9 | コルーチン内からの effect 起動 | 2 | `gji_coro_body:205`（`win32_async::spawn_local` + `sleep_ms`）/ `cold_warmup::run_start`（`spawn_local`） | gji 側は診断 1 本のみ。`ProbeAction` にするか削る。run_start 側は §5 |

## 7. 新 crate への移動候補の順序案（warmup/ 分）

1. **継ぎ目ゼロで今すぐ移せる**（約 380 行、P）。
   - `literal_detect_fsm.rs` の純関数 6 つと `VetoDecision`。
   - `probe_fsm.rs` の型群: `DeferredOrigin` / `DeferredVk` / `ProbeObservations` / `TransmitPlan` / `TransmitTarget` / `ProbeAction` / `per_vk_confirm_tags` / `idx_u16`。
   - `warmup_strategy.rs`。`gji_fsm`（2629 行、ungated）と一緒に動く。
2. **S5・S6・S7 の型移しだけで移せる**。
   - `TsfEnvSnapshot` と `decide_transmit_plan`（71 行）。
   - `literal_facts()`。
3. **S1 と S2 の核心**。ここが山。
   - tsf/probe.rs の `LiteralDetector`（約 315 行）と `TsfReadinessProbe`（約 110 行）を snapshot 引数型に書き換える。
   - あわせて `now_ms` を env に足す。
   - 終わると 4 から先が機械的になる。
4. S1・S2 後に移る群。
   - `LiteralDetectCore`
   - `ProbeCoroState`
   - `await_vk_detection` と `run_per_vk_confirm`
   - `MsImeReadyCoro`
   - `TickableFsm`
5. S3・S4 を effect 化してから移す群。
   - `GjiWarmupCoro`
   - `TsfProbeCoro`（`ChromeProbe` は消す）
   - `LiteralDetectFsm`
   - `UnicodeLiteralObserverFsm`
6. 最後まで Windows 側に残すもの。
   - `cold_warmup.rs::run_start` の conv 読み書きと前景ウィンドウクラス取得。
   - `OutputActiveGuard` の実体。
   - `step_probe` が snapshot を組み立てる処理。
   - `ProbeIo` の `Output` 実装。

## 8. 危険箇所（分けると隙間ができるもの）

- **D1 baseline は SendInput の前に取る（最重要）**。
  - `output/probe_io.rs:545-552` で `LiteralDetector::new_with_pre_send_baseline(gji_write_bytes(), false)` を SendInput の前に作っている。
  - baseline は write_bytes、候補ウィンドウ SHOW カウンタ、ops 3 種、`epoch_send_ms`。
  - 継ぎ目を入れて「core が次の tick で baseline を取る」形にすると、送信中の SHOW や GJI I/O を baseline に含めてしまう。
  - すると正しく合成できた文字が SuspectedLiteral と誤判定され、backspace で消える。
  - 参照: BUG-024、BUG-027 追補 5、BUG-029、BUG-030、BUG-033 追補 3・4、ADR-079（epoch fencing）、ADR-122。
  - baseline は executor が送信と同じ同期ステップで取り、event として core に返す。
- **D2 デフォルト no-op の罠**。
  - `TickableFsm::apply_vk_sent` / `apply_transmit_done` はデフォルト実装が no-op で、委譲を 1 つ忘れても気づけない。
  - BUG-027 の根本原因がこれ（`ChromeProbe` の委譲漏れ、毎回確実に再現）。
  - さらに `pending_vk_sent` / `pending_transmit_done` は「apply で置き、次の tick で消費」のスロット。
  - 上書きされると `overwritten_unconsumed` のログが出るだけ。
  - event 入力にするなら、メソッドを必須にするか、enum 入力で書く。
- **D3 `OutputActiveGuard` の寿命と BUG-58**。
  - guard が active の間、フックは物理キーを INPUT_DEFER に退避する。
  - MS-IME の Phase 1（IMC 待ち）で保持すると 5000ms フリーズした（BUG-058）。
  - フォーカス変更で `pending_tsf` の Box が破棄されると guard が drop で解放される。
  - effect 化したときに、機械が破棄された場合（フォーカス変更、`discard_*`）の解放を executor 側が保証しないと、フックが永久に deferred のままになる。
- **D4 診断の同期 IMM32**。
  - `log_composition_probe` は `run_with_timeout` なしで IMM32 を約 9 回、tick 内で同期に呼ぶ。
  - BUG-034（`SendMessageTimeoutW` のブロック）と同じ種類のリスクを診断のために負っている。
  - 実害が出たかは**未確認**（実機ログでの確認はしていない）。
  - effect 化するとき、同時にタイムアウト隔離するか削るかを決める。
  - また `TSF_PROBE_SNAP` の thread_local は `runtime/message_handlers.rs:503` が事前にセットする暗黙の受け渡し。effect 化で順序が変わらないようにする。
- **D5 snapshot の鮮度が 10ms 劣化する**。
  - 今の `LiteralDetector::check_now` と `visible_fencing_verdict` は、呼ぶたびに `gji_last_write_ms` と時計をライブで読む。
  - snapshot 化すると、判定は最大 1 tick（10ms）古い値に基づく。
  - `EPOCH_FENCE_GRACE_MS` は `GJI_SAMPLE_INTERVAL_MS(10) × 2 = 20ms`。誤差の大きさが猶予と同程度。
  - `probe_fsm.rs:506-512` のコメントも、env のライブ読み → tick snapshot への置換で挙動が変わりうることを既に警告している。
  - snapshot 化の前後で、ADR-079 の fencing 判定（`evidence_fresh` / grace hold）の再現テストを先に用意する。
- **D6 時計の基準がコルーチンごとに違う**。
  - `UnicodeLiteralObserverFsm` は tick 数ベース、他は GetTickCount64 ベース。
  - GetTickCount64 の分解能は約 15.6ms で、tick（10ms）より粗い。
  - `apply_transmit_done` の `deadline_ms` は tick 時刻ではなく executor が呼んだ時刻から計算している。
  - 仮想時計に差し替える際は、この 3 つの基準を揃えるか、意図して残すかを決める。
- **D7 `StepCoro::prime` の前提**。
  - 最初の `step()` は入力を消費しないため、`new` で `prime()` しておかないと install 後の最初の tick の入力が捨てられる。
  - `ProbeCoroState::new` は `debug_assert!`、`MsImeReadyCoro::new` は自前で複製。
  - crate をまたいで構築を移す際に、片方だけ落ちても release では気づけない。
- **D8 INV-45 との関係（未確認の部分あり）**。
  - `TsfEnvSnapshot.gji_active` = `gji_is_active_ime()` = `gji_monitor_ok && tsf_active_kind == 1`。
  - この `tsf_active_kind` は CLSID 判定由来で、`ImeKindId`（推測値）そのものではない。
  - ただし `needs_literal`（`decide_transmit_plan`、`tsf_probe_coro_body:681`）の判断に使われ、false なら safety net（LiteralDetect）が丸ごとスキップされる。
  - INV-45 の言う「非対称な選択に推測値を使わない」に当たるかは、**ADR-089 本文を読んでいないので未確認**。
  - 移植しても入力は snapshot の 1 フィールドのままなので、構造は変わらない。

## 9. 未確認

- `tsf/probe.rs` と `tsf/observer.rs` の各関数を functionally 分類してはいない（担当外）。S1 に必要な snapshot フィールドは `check_now` / `evidence_now` / `check_outcome` の読み取りから拾った。他に漏れがある可能性がある。
- `output/vk_send.rs` / `output/mod.rs` / `output/tsf_warmup_coord.rs` 側で、`TickableFsm` の `Box<dyn>` をどう保持・破棄するかは数カ所の呼び出し位置だけ見た。D3 の「破棄経路」の網羅は未確認。
- 古い記述: `warmup/mod.rs` の doc と `tickable_fsm.rs` の末尾コメントが、存在しない `UnicodeColdWarmupFsm` を指している。
- ADR-053（StepCoro）と ADR-163（決定の I/O 分離と再生ハーネス）に、この構想と整合する方針が既にあるかは本文を読んでいない。

---

# 第2部 state/ の ungated ファイル（概況）

## 10. 事前数字の検証

事前の粗い数字は「約 14,000 行が純関数で Linux テスト可」。

- `state/` は 54 ファイル（`mod.rs` を含む）。
- gate されているのは 3 つ（`platform_state` / `ime_decision_view` / `ime_event_log`）。残る 51（mod.rs + 50 モジュール）が ungated。

| 区分 | ファイル数 | 全体 | 本体（コメント・空行含む） | 本体のうち実コード行 | テスト | #[test] |
|---|---|---|---|---|---|---|
| ungated | 51 | 30,777 | 14,877 | 7,977 | 15,900 | 676 |
| gated（state/ 内） | 3 | 3,646 | 2,082 | 1,180 | 1,564 | 53 |

- 14,000 という数字は、ungated の「本体」14,877 行（コメント・空行込み）とほぼ一致する。
- ただし実コード行（コメント行と空行を除く）は 7,977 行。**約 14,000 行の「コード」ではない**。
- 「純関数」という言い方も正確ではない。
  - ungated の多くは、状態を持つ型と reducer。例: `ime_model`（本体 1,167）、`observation_store`（942）、`key_effect_*`（計 2,482）、`explicit_press`（1,193）、`actuation_chain`（689）、`intent_store`、`force_guard`。
  - それでも OS 非依存で、Linux で回る。
- 676 本のテストが ungated ファイルの `#[cfg(test)] mod` にあり、gate 属性が付いたテスト mod は 0。Linux で走る構造になっている（実行はしていない）。

## 11. ungated なのに Win/OS 依存を持つ箇所

item 単位の `#[cfg(windows)]`（コメントでなく属性）が ungated ファイルの中に 11 個ある。

| ファイル | 行 | 対象 | 依存 |
|---|---|---|---|
| ime_event.rs | 36, 46 | `HwndId::to_hwnd` と `From<HWND>` | `windows::Win32::Foundation::HWND`。`HwndId(usize)` は ungated の newtype で、この変換 2 つだけ gate |
| key_effect_predictor.rs | 570, 580 | `get_gji`, `get_native` | `gji_charset_autodetect::*`、`msime_key_assignment::*` |
| key_effect_runtime.rs | 377, 385, 393, 408, 440, 816 | `table_file_path`, `last_attempt_file_path`, `table_file_stamp`, `load_and_log`, `current_fingerprint_probe`, `get_for_keymap` | `crate::app::find_config_path()`（gate された module） |
| probe_admission.rs | 325 | `admit_epoch_in_app` | `crate::runtime::Runtime`、`crate::focus::classify::root_hwnd_of` |

それ以外の非純粋な箇所:

- `key_effect_runtime.rs` は ungated のまま `std::fs`（`metadata` / `read_to_string`）を使う（485, 495）。ファイル I/O が state/ に入っている。Linux でも動く。
- `ime_model.rs:435, 504` で `Instant::now()`（`effective_open()` のデフォルトと `EventTime`）を直読み。
  - `hub_clock.rs` に `HubClock::{Wall, Manual}` という仮想時計の前例がある（`ImeStateHub` が使い、`tests/support/harness.rs` が `Manual` で駆動）。
  - warmup/ の時計注入で再利用できる設計の先例。
- `probe_admission.rs:71` に `static REJECTION_COUNTERS`（`LifetimeCounter`）。プロセス内 static。
- `ime_profile_driver.rs:186, 188` に unit 型の `static` 2 つ（`IMM_CROSS_DRIVER` / `TSF_NATIVE_DRIVER`）。無害。
- `gji_direct_mechanism.rs:240` に `std::thread::panicking()`。無害。
- `focus::class_names::AppImeProfile` を 6 ファイルが import（`explicit_press` / `ime_actuation_decision` / `key_sequence_policy` / `observation_store` / `physical_disposition` / `actuation_decision_record` のテスト）。`focus::class_names` は ungated なので問題なし。
- `windows::` トークンが本体にあるのは `ime_event.rs` の上記 2 箇所のみ。`HWND` という語が出るファイル（`conv_mode.rs`・`probe_admission.rs` のテスト定数など）は、`HwndId` の別名を使っているだけ。
- gate されている 3 ファイルのうち 2 つは、単独で見ると OS 非依存。
  - `ime_event_log.rs`（178 行）。`std::time::Instant` と ungated の型だけ。gate の理由が見当たらず、ungated にできそう（`mod.rs:202-203`）。
  - `ime_decision_view.rs`（154 行）。`crate::tsf::observer::{TsfObservations, ActiveImeKind, candidate_was_seen()}` を使うので gate は妥当。借用 `ImeControlView<'_>` を持つ P* 候補（他の担当領域）。
  - `platform_state.rs`（3,314 行）は対象外。

## 12. `#[cfg_attr(not(windows), allow(dead_code))]` の全件

数えると 46 か所 / 11 ファイル（`cfg_attr(not(windows)` の全形で数えた。`dead_code` 単独形は 45 か所 + `conv_mode.rs:189` の `allow(dead_code, unused_variables)` 形が 1 か所）。事前の「46 か所 / 11 ファイル」は**正確だった**。

| ファイル | 件数 | 備考 |
|---|---|---|
| state/mod.rs | 18 | 下の表。全てモジュール単位 |
| state/conv_mode.rs | 8 | `ConvModeTarget`（+impl）、`ConvMutationReason`（+impl）、`ConvActuationOutcome`、`ConvReadSource`、`ConvModeMgr`（+impl、`unused_variables` 併記） |
| vk.rs | 4 | `vk_may_mutate_conv`、`ascii_to_vk`、`vk_pair_to_ascii`、`build_symbol_to_vk` |
| gji_charset_autodetect.rs | 4 | `classify_thumb_key_ime_actions`、`ModeKeyCandidate`、`classify_mode_key_ime_action`、`classify_vk_in_ime_keys` |
| state/belief.rs | 2 | `ImeBelief` と impl |
| state/scoped_latch.rs | 2 | `disarm`、`is_armed` |
| state/probe_admission.rs | 2 | `record_hwnd_mismatch`、`for_sync` |
| tsf/mod.rs | 2 | `gji_fsm`（モジュール）、`literal_facts`（モジュール） |
| lib.rs | 2 | `msime_legacy_keymap`、`keymap`（モジュール） |
| state/ime_model.rs | 1 | `drives_composition_side_effects` |
| lifetime_counter.rs | 1 | `read` |
| 合計 | 46 | 11 ファイル |

### state/mod.rs の 18 モジュールと本番の呼び出し元

「全て gate 側」は、本番呼び出し元の全部が `#[cfg(windows)]` のモジュールであること（grep による確認。ビルドはしていない）。

| モジュール | 全体行 | 本体行 | 本番呼び出し元 | 区分 |
|---|---|---|---|---|
| alt_impersonation | 394 | 111 | hook.rs, app/bootstrap.rs, runtime/mod.rs | 全て gate 側 |
| app_suppression | 137 | 76 | hook.rs, focus/classifier.rs, focus/tracker.rs, runtime/* に加え、ungated の focus/class_names.rs:155 と keymap.rs:171,179 | 一部 ungated（その ungated 側が Linux で生きているかは未確認） |
| win_key_guard | 40 | 19 | hook.rs | 全て gate 側 |
| layout_language | 54 | 26 | observer/layout_observer.rs | 全て gate 側 |
| half_width_alnum | 642 | 300 | output/mod.rs, platform_state, runtime/key_pipeline.rs | 全て gate 側 |
| explicit_press | 2057 | 1193 | platform_state, runtime/key_pipeline.rs（+ tests/explicit_press_exhaustive.rs） | 全て gate 側 |
| conv_classify | 930 | 203 | journal.rs, platform_state, key_pipeline、ungated の state/evidence.rs（`ConvSyncReason` 型） | 一部 ungated |
| eisu_recovery | 411 | 205 | observer/gji_observer.rs, platform_state, key_pipeline（ime_event.rs は doc 言及のみ） | 全て gate 側 |
| ime_actuation_decision | 939 | 381 | journal, ime_controller, ime_decision_view, open_chain, key_pipeline, executor, ime_refresh、ungated の actuation_decision_record / ime_model / explicit_press | 一部 ungated |
| drift_correction | 104 | 104 | platform_state（+ tests/support/harness.rs） | 全て gate 側 |
| imm_evidence | 131 | 60 | imm.rs, observer/ime_observer.rs, runtime/ime_refresh.rs | 全て gate 側 |
| mode_key_pass | 711 | 305 | platform_state, runtime/mod.rs, key_pipeline | 全て gate 側 |
| external_change_watch | 274 | 136 | platform_state（+ harness） | 全て gate 側 |
| injection_mode | 21 | 21 | output/types.rs、tsf/gji_fsm.rs（ungated だが、その gji_fsm 自体の呼び出し元は warmup_strategy / platform / output で全て gate 側） | 実質 gate 側 |
| keymap_latch | 113 | 60 | platform_state のみ | 全て gate 側 |
| post_bypass | 106 | 44 | runtime/message_handlers.rs のみ | 全て gate 側 |
| focus_probe_plan | 257 | 75 | runtime/key_pipeline.rs のみ | 全て gate 側 |
| key_sequence_policy | 203 | 143 | ime_controller.rs、ungated の ime_actuation_decision / physical_disposition / explicit_press | 一部 ungated |
| 合計 | 7,524 | 3,462 | | |

- 「ungated で Linux テストはされているが、本番では Windows でしか呼ばれない」ファイルが 18 モジュール、全体 7,524 行（本体 3,462 行）ある。
- これらは「dead_code 抑止だけで生きている」と言える。
  - Linux の本番ビルドでは未使用。テスト（`#[cfg(test)]`）が参照するから残っているだけ。
  - 純粋性のおかげで Linux でテストできる、という設計意図は成り立っている。
- 逆に言うと、新 crate に移すときのコスト感としては、これらは**そのまま移せる**。
  - 移した後は、呼び出し元（hook / runtime / platform_state）が dumb executor 側になり、core の API を呼ぶ形になる。
- 各 `allow(dead_code)` が今も必要かは**未確認**。外して Linux で `cargo check` すれば分かる。
  - 例: `app_suppression` は ungated の呼び出し元があるので、属性が不要になっている可能性がある。
  - `lib.rs` の `keymap`、`tsf/mod.rs` の `gji_fsm` も同様。

## 13. state/ ungated の概況表

「実コード」はコメント行・空行を除いた本体行。

| ファイル | 全体 | 本体 | 実コード | テスト | #[test] | 備考 |
|---|---|---|---|---|---|---|
| ime_model | 3756 | 1167 | 701 | 2589 | 81 | belief の reducer。`Instant::now()` ×2 |
| observation_store | 2248 | 942 | 437 | 1306 | 51 | |
| key_effect_predictor | 2108 | 911 | 631 | 1197 | 47 | cfg(windows) 2 |
| explicit_press | 2057 | 1193 | 829 | 864 | 16 | |
| key_effect_runtime | 2046 | 913 | 584 | 1133 | 61 | cfg(windows) 6、`std::fs` |
| open_warrant | 1424 | 243 | 123 | 1181 | 19 | |
| actuation_chain | 1009 | 689 | 313 | 320 | 16 | |
| key_effect_table | 992 | 658 | 594 | 334 | 15 | 表データが主 |
| actuation_decision_record | 994 | 400 | 214 | 594 | 11 | |
| ime_actuation_decision | 939 | 381 | 170 | 558 | 32 | |
| conv_classify | 930 | 203 | 67 | 727 | 28 | |
| hook_watchdog | 731 | 297 | 89 | 434 | 23 | |
| mode_key_pass | 711 | 305 | 183 | 406 | 13 | |
| half_width_alnum | 642 | 300 | 149 | 342 | 7 | |
| evidence | 571 | 378 | 236 | 193 | 9 | |
| gji_direct_mechanism | 521 | 350 | 136 | 171 | 9 | |
| ime_actuation | 517 | 361 | 131 | 156 | 14 | |
| state_dependent_key_warning | 516 | 280 | 236 | 236 | 12 | |
| app_ime_policy | 508 | 241 | 98 | 267 | 17 | |
| intent_store | 451 | 225 | 108 | 226 | 14 | |
| probe_admission | 448 | 352 | 156 | 96 | 4 | cfg(windows) 1、static 1 |
| force_guard | 420 | 178 | 84 | 242 | 17 | |
| eisu_recovery | 411 | 205 | 56 | 206 | 16 | |
| conv_mode | 410 | 251 | 132 | 159 | 11 | `ConvModeMgr` は `use` 側が cfg(windows)、型自体は ungated |
| ime_profile_driver | 401 | 214 | 63 | 187 | 8 | |
| ime_event | 767 | 673 | 231 | 94 | 7 | HwndId の変換 2 つが cfg(windows) |
| press_ledger | 340 | 178 | 102 | 162 | 10 | |
| event_origin | 279 | 172 | 60 | 107 | 12 | |
| external_change_watch | 274 | 136 | 100 | 138 | 9 | |
| focus_probe_plan | 257 | 75 | 37 | 182 | 7 | |
| physical_disposition | 249 | 249 | 90 | 0 | 0 | テストなし（`tests/explicit_press_exhaustive.rs` が外から検証） |
| keymap_initial_hypothesis | 248 | 160 | 94 | 88 | 5 | |
| alt_impersonation | 394 | 111 | 52 | 283 | 16 | |
| key_sequence_policy | 203 | 143 | 49 | 60 | 6 | |
| mod.rs | 204 | 204 | 100 | 0 | 0 | `TickMs` 定義、cfg(windows) の再エクスポート |
| 以下は 200 行未満 | | | | | | |
| generation | 176 | 114 | 59 | 62 | 6 | |
| input_barrier | 165 | 76 | 38 | 89 | 6 | |
| ime_kind | 153 | 97 | 52 | 56 | 3 | |
| app_suppression | 137 | 76 | 32 | 61 | 8 | |
| focus_resync_policy | 113 | 52 | 20 | 61 | 5 | |
| keymap_latch | 113 | 60 | 22 | 53 | 5 | |
| post_bypass | 106 | 44 | 32 | 62 | 4 | |
| hook_state | 106 | 106 | 54 | 0 | 0 | テストなし |
| drift_correction | 104 | 104 | 51 | 0 | 0 | テストなし |
| scoped_latch | 102 | 59 | 41 | 43 | 3 | |
| hub_clock | 101 | 79 | 49 | 22 | 2 | 仮想時計の前例 |
| imm_evidence | 131 | 60 | 21 | 71 | 2 | |
| transition | 76 | 58 | 16 | 18 | 1 | |
| belief | 62 | 62 | 25 | 0 | 0 | |
| layout_language | 54 | 26 | 12 | 28 | 4 | |
| win_key_guard | 40 | 19 | 4 | 21 | 3 | |
| conv_after_open | 41 | 26 | 8 | 15 | 1 | |
| injection_mode | 21 | 21 | 6 | 0 | 0 | |

（並び順は概ね全体行数の降順だが厳密ではない。数字は全て自前スクリプトの出力。）

## 14. 第2部の移動候補と未確認

- 移動候補の順序。
  1. 呼び出し元が全て gate 側の 13〜14 モジュール（§12 の「全て gate 側」）。すでに OS 非依存で、Windows 側の依存を持たない。
  2. 型・reducer 本体（`ime_model` / `observation_store` / `intent_store` / `force_guard` / `actuation_chain`）。`Instant::now()` を `HubClock` 経由に寄せれば完全に決定的になる。
  3. item 単位の cfg(windows) を持つ 4 ファイル（`ime_event` / `key_effect_predictor` / `key_effect_runtime` / `probe_admission`）。分け方は「型と判定は core、パス解決とファイル I/O と Runtime 参照は Windows 側」。
  4. gate されている `ime_event_log.rs` を ungated にする（OS 非依存に見える）。
- 未確認。
  - ungated の 676 本が実際に Linux で通るか。実行していない（構造上は走る）。
  - 各 `allow(dead_code)` が今も必要か（§12）。
  - `key_effect_runtime` の `std::fs` を core 側に置いてよいか。新 crate が std::fs を許すかの方針次第。
  - `ime_model.rs` の `Instant::now()` ×2 が実際に Linux のテストで差し替えられているか。`HubClock` 経由の部分と直読み部分の境界は確認していない。
