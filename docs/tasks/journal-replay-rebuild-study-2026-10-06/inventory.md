---
type: companion-doc
title: |-
  journal とリプレイ基盤の棚卸し(2026-10-06、develop 777bf1db 時点)
---

# journal とリプレイ基盤の棚卸し

[README.md](README.md) の検討の前提となる事実の一覧。develop `777bf1db`(PR #517 のマージ)時点のコードで確かめた。
行番号はこの時点のもの。確かめられなかったことは「未確認」と書く。パスは `crates/awase-windows/` からの相対パス(`src/…`・`tests/…`)。

## 1. 記録の系統(いま本番で何がどこに残るか)

| # | 系統 | 形式 | 出力先 | 消費者 |
|---|---|---|---|---|
| R1 | `UnifiedJournal`(`src/journal.rs:1196`) | `JournalEnvelope{seq, elapsed_ms, entry}` の JSON 配列(`Serialize` だけ。`Deserialize` は無い) | 4 レーンのリング(`journal_policy::LaneKind`)。ダンプは2つ: ホットキー → `%TEMP%/awase_journal_<tick>.json`(`journal.rs:1499`)、不具合報告 → 同じ形で打鍵系だけ直近 10 分に絞る(`journal.rs:1519`) | 人(と Claude)が `bug-report-fetch` スキルで python で読む。自動の読み手は 0 |
| R2 | R1 から tracing への一方向の出力(ADR-139 決定4、`journal.rs:702-` の `emit_tracing`、約 380 行。`absorb`〈`journal.rs:1309`〉から呼ばれる) | `awase::journal` ターゲットの `debug!` 行 | `awase.log`。利用者の既定は `info`(`app/bootstrap.rs:152`)なので**利用者の awase.log には出ない**。CI は `RUST_LOG=debug`(`e2e-ime.yml:1017`) | CI のチェッカー。断片 `ime open applied`・`actuation decision`・`gji fsm transition`(`tools/e2e/ime_key_matrix/test_log_anchors_in_rust_source.py:46-49`) |
| R3 | 自由な tracing 行 | テキスト | `awase.log`(awase-windows の src に tracing マクロ 740 か所、粗い grep の数) | CI のチェッカー 20 本(`check_*.py`)。読む断片は 27 個で、R2 由来は 3〜4 個、残りは R3 由来(同ファイル:22-49) |
| R4 | `shadow_send_trace`(`src/shadow_send_trace.rs:52-62`、TF2) | `[shadow-send] …` の `debug!` 行 2 種 | `awase.log` | **0**。チェッカーも anchor 表も読まない。`tools/e2e/ime_key_matrix/testdata/awase-baseline-fixed-excerpt.log` にログの抜粋として現れるだけ |
| R5 | `ImeEventLog`(`src/state/ime_event_log.rs:20`、容量 512) | `ImeEventEnvelope{EventTime{seq, monotonic: Instant, tick_ms}, event}` | メモリのみ | 本番の読み手 0(ADR-232 §背景)。撤去は PR #522(ADR-232 S1)で**未マージ** |
| R6 | 不具合報告(ADR-095/222) | `BugReportPayload`(`src/bug_report.rs:545-`)。`log_excerpt_gz`=R1 の gzip+base64、`app_log_excerpt_gz`=awase.log | report-worker(`services/report-worker/src/index.ts:77`)→ R2 バケット | worker は journal を**解凍せず、形式だけ検証して保存**。journal の中身の形とは結合していない(結合しているのは payload の `schema_version`) |

R1 への入口は3つある:
(a)`UnifiedJournal::record` の直接呼び出し、(b)`WindowsPlatform::push_journal_entry`(`src/platform.rs:126`、`pending_journal_entries` 上限 4096。`output`/`tsf` が journal を直接参照できないガードのための中継)、
(c)`win32::send_input_safe` が溜める `SENT_INPUT_TRACE`(`src/win32.rs:259-285`、上限 512。採番だけ発行時に `JournalStamper::reserve` で行う)。
(b)(c)は `drain_journal_entries`(`platform.rs:93`)で R1 に移る。

## 2. `JournalEntry` の全 variant と記録点(`src/journal.rs:253-505`、21 variant、`size_of == 264` 固定〈:518〉)

| variant | 記録する関数(ファイル:行) | 層・タイミング | 入力文字を含むか |
|---|---|---|---|
| KeyInput | `kp_run_inner`(`runtime/key_pipeline.rs:229`、本番の構築点はここだけ) | runtime。物理キー1件の処理後(処理後の要約。拡張ビット・Alt なりすまし前の vk は無い〈ADR-229 :110〉) | **含む**(vk/scan)。報告では直近 10 分 |
| TimerFired | `handle_wm_timer`(`runtime/message_handlers.rs:723`) | runtime。エンジンのタイマー | 含まない |
| ImeEvent | `ImeStateHub::dispatch_event`(`state/platform_state.rs:206`) | state。全 `ImeEvent`。`event_seq`・`tick_ms` つき(W-c、#505・#508・#516)。`Instant` は載らない | 含まない |
| ConvClassifyCall | `apply_idle_conv_check`(`runtime/key_pipeline.rs:788`) | runtime。`classify_conv_transition` の引数と戻り値 | 含まない |
| ImeActuation | `ir_apply_drift_correction`(`runtime/ime_refresh.rs:880`〈GiveUp〉、`:970`〈送信〉) | runtime。drift correction の試行 | 含まない |
| ActuationDecision | `ir_apply_drift_correction`(`ime_refresh.rs:1018`)、`run_open_chain_async`(`runtime/open_chain.rs:635,673,710`)、`dispatch_ime_set_open`(`runtime/executor.rs:800,931`)、`kp_shadow_actuate`(`runtime/key_pipeline.rs:1388`) | runtime。actuation 合流点の決定(ADR-163) | 含まない |
| SentInput | `drain_journal_entries`(`platform.rs:99`)。元は `win32::send_input_safe` | 殻。awase が送ったキー | **含む**(Unicode の `ch`)。直近 10 分 |
| DriftGiveUpDiagnostic | `ir_notify_drift_giveup_diagnostic`(`ime_refresh.rs:1085`) | runtime | 含まない |
| HookImeModeDiagnostic | `handle_wm_hook_ime_mode_diagnostic`(`message_handlers.rs:105`) | runtime。`[hook] IME-mode` のログ行と同じ情報(doc :95) | IME モードキーの vk のみ |
| DriftGiveUpIntervalEnded | `on_focus_process_changed`(`runtime/focus_tracking.rs:618`) | runtime | 含まない |
| GiveUpFollow | `ir_follow_after_literal_giveup`(`ime_refresh.rs:311`) | runtime | 含まない |
| ImeOpenApplied | `on_ime_apply_complete`(`runtime/mod.rs:932`) | runtime。generation の無い適用の結果の唯一の記録 | 含まない |
| PressWriteClaim | `ImeStateHub::claim_press_write`(`platform_state.rs:244,262`) | state | 含まない |
| FocusTransition | `record_focus_transition_if_changed`(`focus_tracking.rs:64`) | runtime。アプリ名つき | アプリ名(プロセス名) |
| GjiFsmTransition | `note_gji_transition`(`platform.rs:135`)、`advance_tsf_probe`(`platform.rs:425`) | 殻(platform、中継 b) | 含まない |
| TsfProbeStarted | `note_tsf_probe_started_from_gji_action`(`platform.rs:158`) | 殻(中継 b) | 含まない |
| TsfProbeCompleted | `note_tsf_probe_completed`(`platform.rs:182`) | 殻(中継 b) | 含まない |
| LiteralDetect | `note_literal_detect_record`(`platform.rs:207`) | 殻(中継 b) | **含む**(romaji・vk 列)。直近 10 分 |
| DeferredRecoveryFlush | `flush_raw_tsf_literal_recovery`(`platform.rs:869`)、`drain_output_post_send_effects`(`platform.rs:903`) | 殻(中継 b) | 含まない |
| ClockAnchor | `handle_wm_command`(`message_handlers.rs:1204`)、`handle_wm_dump_journal`(`:1982`)、`initialize_app`(`app/bootstrap.rs:734`) | runtime | 含まない |
| DumpTriggered | `handle_wm_command`(`message_handlers.rs:1211`)、`handle_wm_dump_journal`(`:1991`) | runtime | 含まない |

記録する関数は 26 個、ファイルは 10 個(`key_pipeline`・`message_handlers`・`platform_state`・`ime_refresh`・`open_chain`・`executor`・`platform`・`focus_tracking`・`runtime/mod`・`bootstrap`)。

同じ判断に複数のレコードが出る例: drift correction の送信 1 回で `ImeActuation`(:970)・`ImeEvent::DriftDetected`・`ActuationDecision`(:1018、StrategyChain 経路)・`ImeOpenApplied`(`on_ime_apply_complete` 経由)の 4 件と tracing 行が出る。役割は少しずつ違う(試行回数/belief の入力/機構の決定/結果)が、F4 で `DriftPlan`(`state/drift_plan.rs:149`)ができた今は、最初のものは `DriftPlan` の写しに近い。

## 3. 時刻と順序の持ち方

- envelope: `seq`(journal 全体の連番、`AtomicU64`)と `elapsed_ms`(`quanta::Clock`、journal 作成からの ms)。`ClockAnchor{tick_ms, hook_us}` で OS tick・フック時刻と対応づける。
- `ImeEvent`: 上に加えて reducer の `event_seq`・`tick_ms`(W-c)。`EventTime::monotonic`(`Instant`)はシリアライズできず載らない(`journal.rs:287-295` の doc に「再現には足りない」と明記)。
- `HubClock`(`state/hub_clock.rs:19`): `Wall{tick}`/`Manual{…}`。`ImeStateHub` の時刻の口。journal 自身は `HubClock` を使っておらず `quanta::Clock`。時刻の入口は 3 つ(ADR-232 :47)。
- 純粋核の事実の型: `DriftFacts.now: Instant`(`drift_plan.rs:67`)、`ReadStrategyFacts` は `idle_ms` など値だけ。F4 は `Instant` があるため fixture を作らず全数表にした(タスク表の見直し 4)。V4(再生したい分割は `TickMs` で持つ)は前提チェック 1 項目。

## 4. 純粋核(FCIS)の現状と、journal との関係

| 核 | ファイル | serde | `Instant` | journal に事実が載るか | テストの形 |
|---|---|---|---|---|---|
| `decide_read_strategy`(F1) | `state/ime_read_strategy.rs:81` | あり | なし | **載らない**(fixture は手組み。`tests/read_strategy_replay.rs:5-6`) | JSON fixture 11 件 |
| `plan_set_open`(F2) | `state/ime_set_open_plan.rs:39` | なし | なし | 載らない | 全数表 |
| `plan_relay`/`plan_drain_step`(F3) | `state/relay_plan.rs:69,215` | なし | なし | 載らない | 全数表 |
| `decide_drift_plan`(F4) | `state/drift_plan.rs:186` | なし | あり | 載らない(`ImeActuation` は試行回数の一部だけ) | 全数表 |
| `decide_msaa_role`(F5a) | `state/msaa_role_plan.rs:119` | なし | なし | 載らない | 表引き |
| `plan_focus_probe` | `state/focus_probe_plan.rs:48` | なし | なし | 載らない | 単体テスト |
| `decide_gate`/`decide_chain`/`decide_attempt`(ADR-163) | `state/ime_actuation_decision.rs:128,152,346` | 入力 `DecisionInputs` はレコード側に写しあり | なし | **載る**(`ActuationDecision`) | 凍結コーパス 37 件 + 単体テスト |

まだできていないもの(タスク表 `docs/tasks/fcis-layering-tasks-2026-10-06.md`): F5(focus 系の残り)、F6(Output の `Vec<Cmd>` 化。中継 (b) の `pending_journal_entries` と `SENT_INPUT_TRACE` はここに関わる)、P5 の残りの写し、E1(決定関数の理由の enum を journal に載せる、未着手)、V4。
「代数的 effect の完成」は計画上の到達点として存在しない: Plan/Effect の項の設計(`Try`・`Require`・`Bracket` など)は却下され(ADR-229「世界モデルの reduce 化と Effect 列の設計の検討結果」)、採る方針は代案 A(F の分割で `decide → Cmd` を増やす、連鎖は `run_chain` のまま)。E2〜E6 は待つ条件つき。

## 5. リプレイ基盤

| 部品 | 場所 | 中身 |
|---|---|---|
| `awase-replay` | `crates/awase-replay/src/lib.rs`(176 行、dev-only、`publish = false`) | `replay_dir<T>(dir, check) -> ReplayReport`(:57)、`assert_ok`(0 ファイル・0 件・失敗を拒む)。F1 の PR で作り、既存 4 か所を移した(`84d6ca97`) |
| `tests/journal_replay.rs`(232 行) | conv_classify と ime_apply を再生 | `ConvClassifyFixture`(`state/conv_classify.rs:185`)、`ImeEventReplayFixture` |
| `tests/drift_correction_replay.rs`(210 行) | drift_correction を再生 | `DriftCorrectionFixture`(`state/ime_actuation.rs:329`。`ActuationRecord` が `&'static str` を含み `Deserialize` できないための写し) |
| `tests/read_strategy_replay.rs`(50 行) | read_strategy を再生 | `ReadStrategyFacts`/`ReadDecision` を直接 `Deserialize` |
| `replay_record`・`replay_chain_scan` | `state/actuation_decision_record.rs` の `#[cfg(test)] mod tests`(:408-、ファイル全体 1,288 行) | 記録済み入力から `decide_*` を再計算して記録値と比べる。チェーン走査は `ReplayWriter`(RW、#499)で再生。非 Sync の ImmCross の command 再計算はしない |
| 閉ループ | `tests/closed_loop_scenarios.rs`(580 行、`#[test]` 20)、`tests/support/`(harness 716・invariants 221・pseudo_ime 509 行) | 本物の `ImeStateHub::dispatch_event`(journal への記録も含む)を通すが、**journal は読まない**。判定は `h.writes`・`h.predictions` など |
| `docs/journal-replay-guide.md` | 運用手順 | characterization corpus と明記(ADR-225 F1 の根拠) |

`tests/journals/` の fixture(7 ファイル・55 件):

| ディレクトリ | ファイル | 件数 | 由来 |
|---|---|---:|---|
| actuation_decision | `bug-131-report-01m29kdnz.json` | 37 | **実報告の journal から機械的に抽出した唯一のもの**(TH1d、`1fb073a3`)。`caller` 15 件は #520 で書き換え済み、原本は `d703c5d0` |
| conv_classify | `jiskana-vk-kana-injection.json` | 2 | BUG-008 の実機ログの値を手で転記 |
| conv_classify | `eisu-does-not-write-open-axis.json` | 2 | BUG-146 の実機ログの値を手で転記 + 手組み |
| conv_classify | `example-jiskana-recovery.json` | 1 | 既存の単体テストを転記 |
| drift_correction | `bug-43-drift-correction-tight-loop.json` | 1 | BUG-43 の記述から手組み |
| ime_apply | `adr108-focus-crossing-success.json` | 1 | ADR-108 の決定を手組み |
| read_strategy | `decide-read-strategy-cases.json` | 11 | 分岐を 1 つずつ踏む手組み |

CI: `ci.yml:51` で `journal_replay`・`closed_loop_scenarios` などを Linux で回す。実機 CI(`e2e-ime.yml` の `sc-*` 等)は **JSON journal をダンプしない**。判定は awase.log のテキスト(`check_*.py`)。`tools/`・`scripts/` で journal JSON を読むのは `scripts/fetch_latest_bug_report.py`(報告の取り出しだけ)と、リポジトリ外(未追跡)の `.claude/skills/bug-report-fetch` の手順。

## 6. プライバシー

- ADR-095 は報告を公開 issue にしない判断。リポジトリは公開(ADR-225 P1)。
- 入力内容を含む variant は `KeyInput`・`SentInput`・`LiteralDetect`(報告では直近 10 分に絞る、`journal.rs:1519-1545`)。`FocusTransition` はプロセス名を含む。
- 実機 CI の入力は合成なので、CI のログ・journal には利用者の入力は入らない(ADR-226 決定5)。

## 7. 実害の記録との対応(`docs/known-bugs/`、183 件中 "journal" を含む 45 件)

サブエージェントによる一次分類(本文の記述を grep で読んだもの。全件の精読はしていない。5 件を抜き取りで確認し、食い違いは無かった):

| 分類 | 件数 | 例 |
|---|---:|---|
| A: 報告・実機の journal が原因の特定に役立った | 17(A だけ 9・A+B 7・A+D 1) | BUG-043・077・113・117・140・141・170・171・184 |
| B: journal を見たが足りなかった | 13(B だけ 6・A+B 7) | 切り詰め(BUG-173・174・183、ADR-222 で報告時の間引きは廃止)、出力した文字が無い(BUG-112・174、のち `SentInput`)、起動直後しか残っていない(BUG-095・104) |
| B のうち journal に variant・フィールドを足したもの | 4 | BUG-074(`LiteralDetectRecord.romaji`)・075(`DetectEvidence` の delta)・090(`KeyInput.physical`)・110(`DriftGiveUp*`・`HookImeModeDiagnostic`) |
| D: replay fixture が回帰テストとして作られた | 5(D だけ 4) | BUG-008・019・097・131・146 |
| E: テスト名の列挙・設計上の言及だけ | 18 | — |

- replay fixture が**退行を検知した記録は見つからなかった**(`git log -i --grep` で `replay`/`再生` と `落ち`/`失敗`/`検知` の組み合わせを探した。ヒットは fixture 自体の整備のコミットだけ)。
- ADR-225 SP0(直近 10 件):安全レーンの fixture があれば検知できたのは 0 件。8 件は修正と同じコミットで回帰テスト済み。
- variant ごとの known-bugs での言及数: `LiteralDetect` 18、`KeyInput` 9、`ImeOpenApplied` 6、`ActuationDecision` 4、`FocusTransition` 4、`ImeActuation` 3、`GjiFsmTransition` 3、`ConvClassifyCall` 2、`HookImeModeDiagnostic` 2、その他 0〜1(`SentInput`・`PressWriteClaim`・`TimerFired`・`ClockAnchor`・`TsfProbeCompleted` は 0。`SentInput` と `PressWriteClaim` は新しいので 0 は当然)。

## 8. 撤去に関わる行数(実測、`wc -l`)

| 対象 | 行数 |
|---|---:|
| `src/journal.rs`(うちテスト :1672- 約 460、`emit_tracing` 約 380) | 2,133 |
| `src/journal_policy.rs`(うちテスト :268-) | 617 |
| `src/shadow_send_trace.rs` + 呼び出し 2 か所(`win32.rs:342`、`imm.rs:288`) | 62 + 2 |
| `src/state/ime_event_log.rs`(#522 で撤去予定) | 177 |
| `src/state/actuation_decision_record.rs`(本番 :1-407、テスト :408-) | 1,288 |
| `crates/awase-replay` | 176 |
| `tests/journal_replay.rs` + `drift_correction_replay.rs` + `read_strategy_replay.rs` | 232 + 210 + 50 |
| fixture の型 `ConvClassifyFixture`(約 20 行)・`DriftCorrectionFixture`/`Tick`(約 30 行) | 約 50 |
