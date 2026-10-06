# E0: Effect(命令)の署名の棚卸し

- 実測の基準: `origin/develop` `2c9c3eb8`(`git archive` を scratchpad に展開して読んだ。メイン作業ツリーには触れていない)。読み取りのみ。ビルド・テスト・他エージェント起動なし。
- 先に読んだもの: CLAUDE.md、fix-requires-evidence.md、ime-belief-architecture.md(未読の部分あり→§9)、ADR-229 の関連節、`docs/adr/review/229-opus-effect-plan-round1.md`、`229-effect-plan-design-draft-r1-rejected.md`、`world-model-write-inventory-2026-10-06/README.md` と `inventory-w0-a.md` の ImeEvent 表(再利用)。
- 行番号は `2c9c3eb8` のもの。「本番」= `#[cfg(test)]` を除く。

## 0. 範囲と分類の約束

`grep -rn "enum .*(Effect|Action|Command|Cmd|Order|Gate|Mechanism|Request|Op|Event|Entry|Output|Disposition|Plan)"` で全洗いし、次の 4 種に分けた。**「命令」= 実行器が OS・グローバル・キュー・タイマーに作用する値**。

| 種別 | 意味 | 型 |
|---|---|---|
| **C 命令** | 実行器(関数)があり、作用がある | `Effect`(`InputEffect`/`TimerEffect`/`ImeEffect`/`UiEffect`)、`KeyAction`、`timed_fsm::TimerCommand<T>`、`output::TimerCommand`、`MechanismCommand`、`ActuateCmd`、`ProbeCmd`(読み取り)、`ProbeAction`、`GjiAction`、`RuntimeRequest`、`GateAction`、`HalfWidthAlnumEffect`、`FocusProbeEffect`、`TrayCommand`、`EngineCommand`、`WM_*`(13 個。enum ではなく定数) |
| **S 選択・計画** | 命令を選ぶ入力、または命令の「種類」だけを返す純粋な決定 | `WriteMechanism`、`ImmCrossOp`、`ConvAfterOpen(Id)`、`DeferGate`、`GateResult`、`DecisionSite`、`ActuationAction`、`HookWatchdogAction`、`KanjiRolePlan`、`PhysicalKeyDisposition`、`HalfWidthAlnumAction`、`RetryPlan`、`WarningDialogAction`、`Decision`、`ImeOpenOutcome`(結果型) |
| **R 記録・イベント** | 状態の更新入力、または記録 | `ImeEvent`、`OpenApplyReason`、`JournalEntry`、`GjiEvent`、`GateEvent`、`SyncKeyGateEvent` |
| **E エンジン内部** | `src/engine` の FSM の内部の選択型。OS には出ない | `ParseAction`、`OutputUpdate`、`GuardAction`、`SoloTapAction`、`WarnAction`、`ActionOutcome`(timed-fsm) |

補足: `ShadowImeAction`(3)・`ShadowImeEffect`(3)・`SpecialKey`(14)・`ImeOperation`(2)・`GjiTimer`(1)・`GateTimer`(1)・`PendingDiscardReason`(4)・`DecisionSite`(11) は「命令の部品または診断ラベル」。表には載せず、§8 の集計にだけ入れた。

## 1. エンジン → プラットフォーム: `Effect`(`src/engine/decision.rs`)

共通の事実(以下の行に繰り返さない):

- **生成元**: `NicolaFsm`(`fsm_adapter.rs::response_to_effects` が `timed_fsm::Response` を変換。**タイマー命令を先、`SendKeys` を後**に並べる。`fsm_adapter.rs:267-278`)、`Engine::transition_activation`(`engine.rs:~410-436`、`SetOpen`→`EngineStateChanged` の順)、`ime_set_open_effects`(`engine.rs:864`)、`apply_engine_on_with_ime_recovery`(`engine.rs:824`)。
- **実行器**: `DecisionExecutor::dispatch_effect`(`runtime/executor.rs`)。入口は 4 つ — `execute_from_hook`(キーのフック経路)、`execute_from_loop`(ホットキー・フォーカス・refresh)、`drain_deferred`、`on_output_guard_timer`。
- **Effect 列は journal に載らない**。`JournalEntry::KeyInput` が持つのは `DecisionKind::{PassThrough, PassThroughWith{effect_count}, Consume{effect_count}}` だけ(`journal.rs:109-129`、`key_pipeline.rs:229`)。実際に OS へ出たものは `SentInput`(`win32::drain_sent_input_trace` 経由、`platform_state.rs:~215`)にだけ残る。**よって「どの Effect を出したか」は再生できない**(件数と、結果としての送信だけが分かる)。

| 型::variant | 生成元(本番) | 実行する所 | OS への作用 | 結果が返るか | 順序の制約 | 冪等 | 可換 | 吸収 | journal |
|---|---|---|---|---|---|---|---|---|---|
| `InputEffect::SendKeys(Vec<KeyAction>)` | `response_to_effects`、`transition`/flush 系(`fsm_adapter.rs:64`、`release_all_pending_output`) | `dispatch_effect`(`executor.rs:661`) → `WindowsPlatform::send_keys`(`platform.rs:940`) → `Output::send_keys`(`output/mod.rs:691`) → KeyAction ごとに §2 | SendInput(VK/Unicode)。`Romaji` は TSF/GJI の probe(warmup)を起動しうる(§5) | なし(fire-and-forget)。結果は `SentInput` と、後の観測 | (1) `execute_relay` の `Consume` では **Timer だけ即時、他はキュー**(`executor.rs:430-445`)。よって `[Timer Set, SendKeys]` は実行上 `[Timer, …, SendKeys]`。(2) `prev_elapsed_ms` は `mark_send()` より前に読む(`output/mod.rs:698`)。(3) `mark_send()` は SendInput より前(`:711`、output in-flight guard の基準点)。(4) `send_keys` 内で `ImeDiagnosticSnapshot::capture` を呼ぶな(メッセージポンプが回りフックが SendInput より先に走る。`:705-709` の注記)。(5) モードキーを含む `SendKeys` は `arm_mode_key_pass_mark` と `TIMER_IME_REFRESH` 20ms を**送信の前に**足す(`executor.rs:636-657`)。(6) 同一バッチで ReinjectKey は OUTPUT_GUARD の対象、それ以外は `reinject_guard_passed=false` に戻す(`executor.rs:244-270`) | **No**(2 回出せば 2 文字) | **No**(順序が出力文字列) | **No**(合体・省略は文字を落とす) | 件数のみ(上記) + `SentInput` |
| `InputEffect::ReinjectKey(RawKeyEvent)` | `Decision::PassThroughWith`(`executor.rs:418`)、`enqueue_reinject`(`:360`)、`drain_deferred` の guard_held(`:237`)、`engine.rs:332`(flush 後の通過) | `execute_one`→`handle_reinject`(`executor.rs:~517-560`) | SendInput(`RawKeyEvent::reinject`、`wScan:0`。`spawn_local` の中で、`OutputActiveGuard` を**先に**取る) | なし | (1) `OutputActiveGuard::begin()` を `spawn_local` の**前**に取る(`handle_reinject`。理由: RUNTIME 借用中の SendInput でフックが再入し「いが l になった」)。(2) 確認キー(Space/Enter/Esc)の KeyDown は `on_reinject_key` を `reinject` の**前**に呼ぶ(`:~545`)。(3) `reinject_wait_remaining` が確認キーを composition 出力の後に回す(`drain_deferred`)。(4) KeyUp 対称性は `PassthroughQueue::check_keyup_symmetry`(Down が defer されたら Up も reinject、`executor.rs:~395`) | **No** | **No**(KeyDown/Up の対と順序) | **No** | 件数のみ。`SentInput` |
| `TimerEffect::Set{id,duration}` | `response_to_effects`(`fsm_adapter.rs:267`) | `dispatch_effect`→`platform_rt.set_timer`→`Win32Timer::set`(`timer.rs`) | `SetTimer(NULL,0,ms)`。**同じ論理 ID を再設定すると旧 OS タイマーを `KillTimer` して新しい OS ID を割り当てる** | なし(`WM_TIMER`→`handle_wm_timer`→`engine.on_timeout` が後で来る) | (1) `Consume` では即時実行(キューを通らない。理由: `deferred_engine_timers` の os_id 照合が stale を有効と誤判定する。`executor.rs:430-436`)。(2) OUTPUT_GATE/FOCUS_RESYNC 中に発火したら `(logical_id, os_id)` を退避し、drain 後に `current_os_id == os_id` のときだけ `on_timeout`(`message_handlers.rs:~686-700`、`:1831-1850`) | **論理的には冪等(最後の値が残る)、観測上は No**: Set;Set で os_id が変わる→退避済みの発火が「別のタイマー」とみなされて捨てられる | **No**(同 ID で最後が勝つ。別 ID 同士は可換) | **上書き=許容**(`timed_fsm::TimerRuntime` doc の不変条件)。ただし os_id が変わる副作用あり→「Set;Set=Set」は等式として**不成立** | `TimerFired`(発火時のみ。Set/Kill 自体は記録されない) |
| `TimerEffect::Kill(id)` | `response_to_effects`(`fsm_adapter.rs:273`) | `platform_rt.kill_timer`→`Win32Timer::kill` | `KillTimer`(未設定なら何もしない) | なし | 上記(1)と同じ(即時) | **Yes**(未設定への Kill は no-op。`timer.rs:kill`) | 別 ID とは可換。同 ID の Set とは非可換 | Kill;Kill=Kill は成立。`Set;Kill`→`Kill` に畳むと、その間に `WM_TIMER` が発火しうる点が失われる(未確認: 実害の記録なし) | なし |
| `ImeEffect::SetOpen{open, press}` | `transition_activation`(`engine.rs:427`)、`ime_set_open_effects`(`:864`)、`apply_engine_on_with_ime_recovery`(`:824`)。`press` は `Decision::stamp_set_open_press`(`decision.rs:234`)がエンジン入口で打鍵 ID を載せる(ADR-208 D1) | `dispatch_effect`(`executor.rs:607`)→`dispatch_ime_set_open`(`:689-`)→ §3 | §3 の 3 機構のどれか(IMM 書き込み / SendInput VK) | **同期経路は `Some((open, ImeOpenOutcome))`、async(ImmCross 先頭)は `None`+`WM_ASYNC_IME_APPLY_COMPLETE`**(`executor.rs:~890`) | §3 の表(最も制約が多い) | **No**(`press` が同じなら 2 回目は書かない=予約。ADR-208) | **No**(`SetOpen(true);SetOpen(false)` の順) | **危険**: BUG-141 / ADR-208(§7) | `ActuationDecision`、`ImeOpenApplied`、`PressWriteClaim`、`ImeEvent`(`ImeApplyRequested`/`Succeeded`/`Failed`) |
| `UiEffect::EngineStateChanged{enabled}` | `transition_activation`(`engine.rs:434`) | `dispatch_effect`(`executor.rs:625-634`、match の前に `set_conv_mode_authority` と `hook::set_engine_enabled`)→`update_tray` | トレイのアイコン、conv 書き込み権限のグローバル(`Output::conv_mutation_allowed`)、フックスレッドが読む `ENGINE_ENABLED` の atomic | なし | **`SetOpen` の後**(`transition_activation` が SetOpen→EngineStateChanged の順に積む)。match の前に `set_conv_mode_authority` を呼ぶ(`platform_rt` への変換前に `&mut platform` が要る) | Yes(同じ値を再設定) | `SetOpen` とは非可換(順序が conv 権限に効く)。未確認: 他 Effect との可換性は検証していない | 同値の連続は吸収できそう。未確認(実害の記録なし) | なし |

`Decision` の変形(`PassThrough`/`PassThroughWith`/`Consume`、3)は命令ではないが、**`PassThrough` に Effect を足すと `PassThroughWith`(Consume ではない)に格上げされる**(`decision.rs:~255`)。`force_consume` が KeyUp を `Consume` に揃える(ADR-112 決定 2、`Engine::on_input` の唯一の出口)。`physical == Suppress` なら reinject を積まない(`executor.rs:~395-415`)。

## 2. 出力: `KeyAction`(9 variant。`src/types.rs:383`、実行は `Output::send_keys`、`output/mod.rs:714-800`)

共通: **実行器は 1 つ**(`Output::send_keys`)。**結果は返らない**。OS 作用は SendInput(`win32::send_input_safe`。`SentInput` トレースを溜める)。全 variant とも冪等ではなく、可換でもない。

| variant | 実行 | 特記(順序・副作用) | 吸収 |
|---|---|---|---|
| `Key(vk)` / `KeyUp(vk)` | `injector.send_key(vk, up)` | `Key` の解放は `OutputHistory` の解放索引が追う(`KeyUp` を足し忘れるとキー固着。`release_all_pending_output`、ADR-112)。Down/Up の対称性は `KeyLifecycle`/`UpDuty` が保つ | **No** |
| `SpecialKey(sk)`(14 種) | `send_key(special_key_to_vk(sk), false)` | 押下のみ | No |
| `Char(ch)` | `sender.send_char`(mode: Unicode/VK/TSF) | モードで経路が違う(`OutputSession::begin` が解決) | No |
| `Romaji(s)` | `sender.send_romaji`(Unicode は `KEYEVENTF_UNICODE`、VK/TSF は §5 の cold/warm 判定→probe) | **cold の判定は `mark_send` 前の `prev_elapsed_ms`**。`send_romaji_batched`/`send_romaji_as_tsf` は `defer_if_probe_in_flight`(`DeferGate::Enforced`)→ `drain_pending_deferred_before_send_if_queue_only` → `assess_warmth` の順(**drain は `assess_warmth` より前**、さもないと warm/cold が flush 前の状態で決まり probe が汚染された `last_send_ms` を証拠に読む。`vk_send.rs:~60-75`) | **No**(モーラ列は順序が意味を持つ。`pending_deferred` は意図的に FIFO) |
| `KeySequence(s)` | `sender.send_key_sequence` | IME がキーストロークを変換する | No |
| `CtrlChord(vk)` | `injector.send_ctrl_chord` | 1 回の SendInput バッチで自己完結(`OutputHistory` の対象外) | No |
| `Suppress` | なし | 何もしない(ログのみ) | 吸収できる(no-op) |
| `Sequence(Vec<KeyAction>)` | 1 回の `send_keys` の中で全要素を同期実行(待機を含まない。ADR-115 決定 4・11) | ネストは 1 段(`:786-795` で `Sequence` 内の `Sequence` は no-op) | No |

## 3. IME 書き込み(actuation)

### 3.1 型と実行器

`ImeEffect::SetOpen` が入口。ほかに入口が 5 つ(`fix-requires-evidence.md` の「IME actuation 合流点」の表のとおり。ADR-119/180)。**同期は `ImeController::apply`(`ime_controller.rs:577`)、非同期は `run_open_chain_async`**。どちらも `Actuation<Verified>::run_chain(_async)` が連鎖を回す(走査規則は `state/actuation_chain.rs::falls_through`: `Failed` のときだけ次へ。`UnsafeToToggle`/`NotOwned`/`Unwarranted` は次へ進まない)。

| 型::variant | 生成元 | 実行する所 | OS への作用 | 結果 | 順序の制約 | 冪等 | 可換 | 吸収 | journal |
|---|---|---|---|---|---|---|---|---|---|
| `WriteMechanism::ImmCross` | `caps(p,k).chain`(同期)、`WriteMechanism::ALL`(非同期。await 後の再抽選のため。`open_chain.rs` 冒頭) | `apply_mechanism`(同期、`ime_controller.rs:201`)/`imm_cross_write`(非同期、`open_chain.rs:~215`) | IMM32 クロスプロセス(`IMC_SETOPENSTATUS`、+`Targeted` なら `IMC_SETCONVERSIONMODE`)。SendInput なし | `ImeOpenOutcome`: `AppliedWithoutSendInput`/`Failed`/`UnsafeToToggle`(Aborted)/`AlreadyMatched`(Failed 後の再読で一致) | §3.2 | 値としては冪等(同じ open を再設定)。**ただし `PressLedger` が 2 回目を止める** | No | **`Failed` 後の再読 `imm_cross_reobservation_already_matches`(新鮮な Win32 読み値で判定。`None`=未知は常に false)** — shadow ベースの省略とは別物と明記(`ime_actuation_decision.rs:240-253`) | `ActuationDecision`(`AttemptRecord.post_failed_reobservation`) |
| `WriteMechanism::GjiDirect` | 同上 | `apply_mechanism` / `fallback_write`(`open_chain.rs:459`) | SendInput(`VK_IME_ON/OFF`。VK は `key_sequence_policy::ime_key_for` が SSOT) | `Applied`/`AlreadyMatched`/`UnsafeToToggle`(Win キー押下中で未送信) | §3.2 | 値としては冪等(絶対指定) | No | **`gji_direct_already_matches(shadow_on, open, candidate_was_seen)`**(`ime_actuation_decision.rs:224`) — BUG-141・ADR-208 の中心 | 同上 |
| `WriteMechanism::MsImeDirect` | 同上 | 同上 | SendInput(`VK_IME_ON/OFF`) | `Applied`/`UnsafeToToggle` | §3.2 | 値としては冪等 | No | 省略規則なし(常に送る。`decide_attempt`) | 同上 |
| `MechanismCommand::SetOpenCrossProcessSync(bool)` | `decide_attempt`(site=Sync, ImmCross) | `apply_mechanism` | IMM32(同期。`SendMessageTimeoutW` を含み**エンジンスレッドをブロックしうる**) | — | — | — | — | — | `AttemptRecord.command` |
| `MechanismCommand::SetOpenCrossProcessAsyncUntargeted(bool)` | `imm_cross_write`(`ImmCrossOp::Untargeted`、shadow-toggle OFF) | `set_ime_open_cross_process_async` | IMM32(async。宛先を捕獲しない=ADR-086 INV-14 の未移行) | — | — | — | — | — | 同上 |
| `MechanismCommand::SetOpenThenConvForTarget{open, conv_after_open}` | `imm_cross_write`(`Targeted`) | `ime::set_ime_open_then_conv_for_target` | IMM32(同一の検証済み hwnd へ open→conv) | `ImmCrossOutcome{open, conv, open_timed_out}` | open と conv は**同一クロージャ・同一 hwnd**(別々に verify すると別窓へ ROMAN が着弾。`ime.rs:~1285`) | — | — | — | 同上 |
| `MechanismCommand::SendVk(VkCode)` | `decide_attempt`(Gji/MsIme) | `send_ime_mode_key` | SendInput | `bool`(Win キー押下中は false→`UnsafeToToggle`) | — | — | — | — | 同上 |
| `ActuateCmd::SetOpenStatus(bool)` / `SetConversionMode(u32)` | `set_ime_open_cross_process*`、`set_ime_conv_for_target`、`romaji_pre_write`、`conv_actuation` | `imm::actuate_ime_control`(唯一の入口。dylint `actuation_call_guard::RESTRICTED_CALLS`) | `SendMessageTimeoutW(IMC_SET*)` | `Option<usize>`(タイムアウトで None) | 検証済み hwnd(`ActuationTarget::verify_still_current`)の後 | 値としては冪等 | No | — | なし(上位の `ActuationDecision`) |
| `ProbeCmd::GetOpenStatus`/`GetConversionMode` | focus probe、idle conv check、`read_ime_state_fast` | `imm::probe_ime_control` | `SendMessageTimeoutW(IMC_GET*)`(読み取り) | `Option<usize>` | — | **Yes**(読み取り) | 書き込みとは非可換(先後で値が変わる) | 読み取りは合体可 | `ObserverReported`(変換後) |
| `ConvAfterOpen{Skip,Write(Option<u32>)}`/`ConvAfterOpenId` | `decide_dispatch_conv_after_open`(`ime_actuation_decision.rs`:`open && belief != ObservedKana` なら Write) | `set_ime_open_then_conv_for_target` | 上記の conv 書き込み | `ImmCrossOutcome.conv` | **open が `Written` のときだけ**conv を書く。さらに await 後に focus_gen を読み直す(`ime_mode_focus_gen`) | — | — | — | `AttemptRecord.command` |
| `ImmCrossOp::{Targeted, Untargeted}` | 呼び出し元が起案時に決める(`dispatch_ime_set_open`、`kp_shadow_actuate`) | `AsyncChainWriter` | — | — | `Targeted` は起案時の `focus_gen` と捕獲した hwnd を持つ | — | — | — | — |
| `GateResult::{NotOwned, Proceed}`(`decide_gate`) | `decide_gate`(純関数) | 4 関数が独立に再判定(ADR-180 決定 1) | なし | — | **await の前後で毎回判定し直す**(フォーカスが InputRelay へ移るため。`with_app` を内包する共有ヘルパーにすると `fallback_write` から再入してゲートが恒久的に無効化=issue #136/BUG-90 型) | — | — | **InputRelay の窓では書かない=省略が正解**(BUG-90 決定 4) | `ActuationDecision`(site=`DispatchImeSetOpen` 等) |
| `DecisionSite`(11)・`ActuationAction`(`Send`/`GiveUp`、drift の再送上限) | 呼び出し元がラベルとして渡す | 記録専用(`ReassertExplicitPhysicalKey`/`ForceOnRomajiCorrection` は撤去済みの経路のラベルが enum に残存。未確認: 参照ゼロかは数えていない) | なし | — | — | — | — | — | `ActuationDecisionRecord.site/caller` |

### 3.2 actuation の順序の制約(暗黙のものを含む。§6 に再掲)

`dispatch_ime_set_open`(`executor.rs:689-`)の順序:

1. `unknowns_applied` を決め、`build_ime_control_view(explicit_press_applied_pair(applied, open, unknowns_applied))`。**view の `shadow_on` は `Option<bool>`**。`unwrap_or(false)` に潰すな(fix-requires-evidence.md 表の `ControlLog.shadow_on` の行、ADR-098 決定 1-b)。
2. `decide_gate`(NotOwned なら使い捨ての order を発行して `ActuationDecision` を記録し返る。`executor.rs:~742-770`)。
3. **`claim_press_write`(order の発行の直前)**。`!claim.writes()` なら `DUPLICATE_OUTCOME = UnsafeToToggle` を完了に流して返る(`AlreadyMatched` を返すと書いていない押下が `applied=Confirmed` になる。PR #419 Opus M-1。`press_ledger.rs:33-39`)。
4. `imm_cross_is_first_applicable`: **true(async)**: `applied_snapshot = Optimistic(open)`(intra-batch のため)→`issue_self_actuation_order(..).with_press(press)`→`OutputActiveGuard::begin()`→`spawn_local { capture → run_open_chain_async → post_async_ime_apply_complete → drop(guard) }`。**false(sync)**: `order`→`apply_ime_open_with_view`→**何も送らなかったときだけ `release_press_write`**(`outcome_sent_nothing`: `UnsafeToToggle`/`NotOwned`/`Unwarranted`。`Failed`・`AlreadyMatched` は解かない。PR #419 M-2)→`record.caller` を設定→`ActuationDecision` を journal。
5. `execute_one` が `update_intra_batch_applied`(`Failed` は `!open` を `Confirmed` にする=「失敗なら逆の状態だった」とみなす)。その後 `dispatch_outcomes`→`on_ime_apply_complete`。

`kp_shadow_actuate`(`key_pipeline.rs:1238`、もう 1 つの order 起案入口)の順序:

1. `claim_press_write(.., Shadow)`(Engine と同じ押下で衝突したら Engine が優先)。
2. `note_explicit_ime_action(tick)`。
3. **`timer.kill(TIMER_IME_REFRESH)` を書き込みの前に**(ADR-213 P2c で ActivationSync を撤去したため、予約済みの refresh→drift correction が同じ向きを重ねる=BUG-113 型。`:1262-1265`)。
4. `explicit_press_applied_pair`→view→ImmCross 先頭なら **`record_confirmed(false)` を async 書き込みの前に**(OFF のとき。ADR-098 決定 5/6-a の「直後に実書き込みを伴うので laundering ではない」)→order→spawn。
5. 書き込み後、await 中に `ime_mode_focus_gen` が進んでいたら完了を `UnsafeToToggle` に落とす(**generation の無い完了**は新しい窓の `applied` を `Confirmed` にするため。PR #408 Opus M-3。ただし generation=None のまま完了を投げる点が BUG-098 として未修正)。

`on_ime_apply_complete`(`runtime/mod.rs:920`)の順序: `ImeOpenApplied` を journal → **`post_ime_refresh`(UnsafeToToggle でも必ず。`Aborted(GenStale)` の取りこぼしを 20ms 後の refresh で拾うため)** → `record_ime_apply_result`(reduce。UnsafeToToggle は pending だけ解放して `applied` を動かさない) → `acceptance.drives_composition_side_effects()` のときだけ `on_ime_applied`(`platform.rs:~1140-1180`: `reset_candidate_was_seen` → `ime_mode_fsm.on_set_open_applied`(`wrote_open_state()` のとき) → `mark_composition_cold` → `receipt.settle`=`sync_gji`)。

`apply_mechanism`(`ime_controller.rs:201`)の順序: `romaji_pre_write`(ROMAN 補完、**`SendMessageTimeoutW` で同期ブロックしうる**。`SendHealth::blocking_allowed` の gate は「skip 時に再試行する手段がなく静かに固着する」ため**意図的に入れていない**。`:421-433`) → `decide_attempt`(**`SyncChainWriter::write` が先に 1 回 `decide_attempt`、`apply_mechanism` が中でもう 1 回**=決定が 2 回走る。記録用と実行用で、入力が同じ view であることだけで整合している。`ime_controller.rs:~450-470`) → 実送信。`GjiDirect` の OFF 送信成功時に `reset_candidate_was_seen`(ADR-171「1 回の desync 証拠につき再送は 1 回」。`on_ime_applied` でも再度リセット=2 か所)。

## 4. タイマー(12 の論理 ID)

論理 ID: エンジン 2(`TIMER_PENDING=1`、`TIMER_SPECULATIVE=2`、`fsm_types.rs:11-14`)+ `lib.rs:279-299` の 10(`IME_REFRESH`101・`HOOK_WATCHDOG`102・`POWER_RESUME`103・`OUTPUT_GUARD`104・`TSF_PROBE`105・`TSF_GATE`106・`IME_OFF_RESCUE`107・`GJI_LONG_IDLE`108・`FOCUS_RESYNC`109・`HOOK_WATCHDOG_CANARY_CHECK`110)。命令型は 3 つ: `TimerEffect`(エンジン→executor)、`timed_fsm::TimerCommand<T>`(`NicolaFsm`・`GjiFsm`・`TsfGateMachine` の `Response.timers`)、`output::TimerCommand{Continue, Kill}`(`Output::step_probe`/`pending_tsf_timer`→`apply_timer_command`、`platform.rs:928`)。**3 型とも実行器は `Win32Timer::set/kill` の 1 か所**。

`Win32Timer::set` は同じ論理 ID で再度呼ぶと旧 OS タイマーを `KillTimer` して新しい OS ID を割り当てる(`timer.rs:set`)。`resolve(wparam)` が OS ID→論理 ID、`current_os_id` が逆引き。

| 論理 ID | Set の箇所(本番) | Kill の箇所 | 特記 |
|---|---|---|---|
| `TIMER_IME_REFRESH` | `platform.rs:1016`(`post_ime_refresh` 20ms)、`executor.rs:649`(mode-key-follow 20ms)、`runtime/mod.rs:1080`(`schedule_ime_refresh(delay)`。呼び出し元ごとに 0/50/500ms 等) | `message_handlers.rs:1026, 1058`、`runtime/mod.rs:1056`(`spawn_ime_refresh`)、`key_pipeline.rs:1265, 1417` | **Set 5 経路・Kill 5 経路が別々の遅延で同じ ID を奪い合う**。最後に書いた者が勝つ=可換でない |
| `TIMER_POWER_RESUME` | `message_handlers.rs:1029, 1061`(3s) | `:490`(発火時に自分で kill) | 一発 |
| `TIMER_OUTPUT_GUARD` | `executor.rs:330` | `:275, 287` | **不変条件: `guard_held.is_some() ⟺ タイマー登録済み`**(`executor.rs:~120` の doc)。型では守られていない |
| `TIMER_TSF_PROBE` | `output::TimerCommand::Continue`(`output/mod.rs:1017`、`tsf_warmup_coord.rs:283`)→`apply_timer_command` | `Kill`(`output/mod.rs:991, 1035`、`platform.rs:555` の `CancelProbe`) | 10ms tick。`platform.rs:advance_tsf_probe` が terminal(Kill)かで `consume_literal_detect_trace` の挙動が変わる |
| `TIMER_TSF_GATE` | `runtime/mod.rs:1774`(フォーカス変更で 500ms) | `message_handlers.rs:519`(発火時)、`runtime/mod.rs:1229` | `TsfGateMachine` は `GateTimer::WarmupTimeout` を出すが、実際の Set は `runtime/mod.rs:1774` の直書き(`TimerCommand` 経由ではない) |
| `TIMER_IME_OFF_RESCUE` | `runtime/mod.rs:887`(`set_ime_off_rescue_pending`) | `:878` | **`pending_ime_off_rescue=Some(..)` と Set は常にペア**(doc に「一元化」とあるが型での強制はない) |
| `TIMER_GJI_LONG_IDLE` | `platform.rs:506`(`GjiTimer::LongIdle`) | `platform.rs:511`、`message_handlers.rs:550` | `GjiFsm` の `Response.timers` を `dispatch_gji_response_from` が**actions の前に**処理(`:498-513`) |
| `TIMER_FOCUS_RESYNC` | `runtime/mod.rs:1091` | `message_handlers.rs:561`、`key_pipeline.rs:611` | `open_if_current(generation)` が世代不一致なら何もしない(二重 drain 防止) |
| `TIMER_HOOK_WATCHDOG` | `runtime/mod.rs:1784`(3s) | なし | 常駐 |
| `TIMER_HOOK_WATCHDOG_CANARY_CHECK` | `runtime/mod.rs:1947` | `message_handlers.rs:663`(発火時) | 一発 |
| `TIMER_PENDING`/`SPECULATIVE` | `TimerEffect::Set` | `TimerEffect::Kill` | §1。OUTPUT_GATE/FOCUS_RESYNC 中は発火を退避→drain 後に os_id 照合で replay |

`timed_fsm` の約束: 「`Set` は同じ ID のタイマーを置き換える」(`TimerRuntime` の doc、`dispatch.rs:12-18`)、「`Kill` は未設定なら no-op」(`response.rs`)、「`on_event`/`on_timeout` は副作用を持たない。`consumed=false` のとき状態は不変」(`machine.rs:36-48`)。**`Response::dispatch` はタイマー→アクションの順**(`dispatch.rs:~88`)で、`fsm_adapter::response_to_effects` と `dispatch_gji_response_from` も同じ順だが、`DecisionExecutor` は `Consume` でタイマーだけ即時、アクションはキュー(§1)のため**実効の順は逆転している**。

## 5. warmup / probe(`tsf/`、`output/probe_io.rs`)

| 型::variant | 生成元 | 実行する所 | OS への作用 | 結果 | 順序の制約 | 冪等 | 可換 | 吸収 | journal |
|---|---|---|---|---|---|---|---|---|---|
| `ProbeAction::Transmit{cold_seq, plan, romaji, target}` | `TsfProbeCoro`/`GjiWarmupCoro`/`ChromeProbe`(probe_fsm.rs) | `output/probe_io.rs::run_probe_actions`(`:408-`)→`io.transmit_tsf`/`transmit_chrome` | SendInput(romaji の VK 列) | `ze_bs_count`(戻り)→`machine.apply_transmit_done` | **検出ベースライン(`LiteralDetector::new`)は送信前**(`:~490`)。`gate_is_bypass()`(TSF のみ)を送信前に確認。送信後に `apply_transmit_done` | **No** | No | **No**(romaji は 1 回だけ) | `LiteralDetect`(`LiteralDetectTraceItem::Verdict`)、`SentInput` |
| `ProbeAction::TransmitSingleVk{..}` | 同上(BUG-24 追補、cold 直後の 1 文字目) | 同上(`:517-`) | SendInput(VK 1 個+shift) | なし→`apply_vk_sent(detector, deadline_ms)` | **ベースラインは SendInput の前**(`new_with_pre_send_baseline`。BUG-027/029/030。BUG-027 は `ChromeProbe` が `apply_vk_sent` を委譲していなかった=結果の取りこぼし) | No | No(`idx` の順) | No | `LiteralDetect`(`VkSent`) |
| `ProbeAction::RawTsfLiteralRecovery{..}` | literal 判定(`LiteralDetectCore`) | 同上(`:581-`)→`io.set_raw_literal`+`mark_cold_raw_tsf` | 後続で BS(+必要なら再送)を予約 | なし | **trace への push は romaji が move される前**(`:~590`、clone が要る)。`consecutive==0` で再送、それ以外は give-up(BS のみ)。VK_IME_OFF→ON の reinit は実 Chrome×GJI で 0/10 と効かず ADR-212 P3 で撤去 | No | No | **give-up は意図的な「省略」**(BUG-027/074: romaji が失われる→journal に `romaji: Some` を残す) | `LiteralDetect` |
| `ProbeAction::UpgradeToTsf` | `UnicodeLiteralObserverFsm` | 同上(`:575`)→`DispatchResult::LearnedTsf` | `InjectionModeStore` の学習(永続) | `Ended(UpgradedToTsf)` | `Done` が後続に入っている | Yes(学習済みへの再学習) | — | 未確認 | `LiteralDetect` |
| `ProbeAction::CompositionConfirmed{mark_literal_session,..}` | 判定 | 同上(`:632`) | `reset_consecutive_count`、`mark_literal_session_confirmed(cold_seq)`(世代付き。BUG-39) | なし | **本物の confirm が挟まれたら連続カウントを必ずリセット**(BUG-027 追補 4)。per-VK では `mark=false`、最終確認だけ `true` | Yes | — | — | `LiteralDetect` |
| `ProbeAction::LiteralDetectNote` / `Done` | 判定 | 同上 | なし(trace)/ 段の終了 | — | `Done` で `break 'stage` | Yes | — | — | `LiteralDetect` |
| `GjiAction::StartProbe{probe_id, params}` | `GjiFsm`(`ImeOn`/`FocusChange`/`KeyInput` 等で) | `dispatch_gji_response_from`(`platform.rs:516-`) | probe の開始(`pending_tsf` のインストール、`OUTPUT_GATE` ガード)。`Unicode` mode は即 `WarmupComplete` | なし(以後 `WarmupComplete`/`Aborted` の `GjiEvent`) | `gji_store_probe_id`→`active_tsf_probe_started_ms`→`reset_probe_tick_counters`→`note_tsf_probe_started`(`TsfProbeStarted` を journal)。**`TsfProbeStarted` と `Completed` を `probe_id` で突合できるよう `cold_seq` に `probe_id` を入れない**(ADR-123 指摘) | No(probe_id が進む) | No | No | `TsfProbeStarted`/`TsfProbeCompleted`/`GjiFsmTransition` |
| `GjiAction::CancelProbe{probe_id}` | `GjiFsm` | 同(`:~565`) | `cancel_probe`(pending_tsf/ガード/probe_id を一括)+`timer.kill(TIMER_TSF_PROBE)` | なし | **`gji_current_probe_id() == Some(probe_id)` のときだけ**実行(古い probe_id は無視=冪等) | **Yes** | — | 同 ID の連続は吸収できる | `TsfProbeCompleted("Canceled")` |
| `GjiAction::SendInput{..}` / `SendInputDirect(..)` | (`#[allow(dead_code)]`) | **無視**(`platform.rs:~585`: 「実際の送信は Output が担う」) | なし | — | — | — | — | — | なし |
| `GjiAction::DiscardPending{count, reason}` | `GjiFsm` | ログのみ(実データの破棄は FSM 内部で完了済み) | なし | — | — | — | — | — | なし(ログ) |
| `GateAction::InitiateHold` / `DrainHeld` | `SyncKeyGate`(`src/gate.rs`)、`TsfGateMachine` | `HoldingGate`(timed-fsm) | なし(プロセス内のバッファ) | `DrainHeld` はバッファを返す | `try_hold_key` は `kp_run_inner` の**最初の段**(`key_pipeline.rs:~46`)。TSF タイムアウトで held を `INPUT_DEFER.replay_later` | `InitiateHold` は冪等 | — | — | なし |
| `RuntimeRequest::StartTsfProbe` | `Output`(`install_pending_tsf()` の**後**) | `Runtime::drain_runtime_requests`(`WM_EXECUTE_EFFECTS`/`WM_DRAIN_OUTPUT_QUEUE` の末尾)→`pending_tsf_timer()` で `TIMER_TSF_PROBE` を Set | タイマー | なし | **probe を先にインストールしてから push する**(`outbox.rs:15-20` の doc。型での強制なし)。drain は全キー処理の後(`message_handlers.rs:~1862`) | **No**(二重 push で probe が二重起動しうる。未確認: `install_pending_tsf` 側の防御) | — | — | なし |

追加の暗黙の順序(`platform.rs:395-400`): **`advance_tsf_probe` は `step_probe` の前に `drain_pending_composition_events` を呼ぶ**(VK_A+BS のバッチで SHOW+HIDE が最初の tick 前に終わると `composition_was_seen` が立つ前に tick が見てしまい、Phase 1 即再送に落ちて IPC race が再発する)。

## 6. キュー・自己メッセージ・defer / replay

| 対象 | 生成元 | 実行する所 | OS への作用 | 結果 | 順序の制約 | 冪等 | 可換 | 吸収 | journal |
|---|---|---|---|---|---|---|---|---|---|
| `INPUT_DEFER`(`defer_during_output` / `replay_later` / `take_all`、`input_defer.rs`) | フック(`OUTPUT_GATE.active` 中)、`TsfGate` のタイムアウト | `handle_wm_drain_output_queue`(`message_handlers.rs:~1770-1850`)→`deliver_key_event(.., DeferredReplay)` | なし(キュー)。drain が `WM_EXECUTE_EFFECTS` を post | なし | **`take_all` は `timestamp` 昇順に並べ替える**(積んだ順ではない。`input_defer.rs:75-95`)。`defer_during_output` は post しない(drain は `OutputActiveGuard::drop` の担当)。`replay_later` は post する。`Runtime` を掴めなければ queue を `INPUT_DEFER` へ戻し `DRAIN_RERUN_PENDING` を立てる | No | No | **容量 1024 で最古を捨てる**(情報の欠落。吸収ではなく喪失) | なし(`overflow_count` はログ) |
| `OutputActiveGuard`(`begin`/`Drop`、`tsf/probe_bridge.rs:100-126`) | `send_keys`(`OutputSession::begin`)、`handle_reinject`、`dispatch_ime_set_open`(async 先頭)、`kp_shadow_actuate`、probe コルーチン等(本番で 12 か所。round1 Q4 の数え) | RAII `Drop` | `OUTPUT_GATE.active` の atomic。1→0 で `WM_DRAIN_OUTPUT_QUEUE` を post | なし | 深度カウント。**async 先頭の書き込みは spawn の前に begin**(await 中のフックを defer 側へ退避するため) | 深度で冪等(対になる Drop) | — | — | なし |
| `DeferGate::{Enforced, Exempt}`(`output/vk_send.rs:22`) | `send_romaji_*`(通常=Enforced、`*_bypass_gate`=Exempt) | `defer_respecting_gate` | なし | — | **defer 側は `raw_recovery_owns_deferred()` を gate で切り替え、drain 側は gate に関わらず常にチェックする**(非対称。ADR-128、round4-3) | — | — | **ADR-156/ADR-123→128 の回帰は「同じキューの 2 窓口(defer/drain)の片方だけに条件を配線した」** | `DeferredRecoveryFlush` |
| `FOCUS_RESYNC`(`focus_resync.rs`: `arm`/`disarm`/`consume_and_close`/`open_if_current(generation)`) | フォーカス復帰 | `handle_wm_timer`(`TIMER_FOCUS_RESYNC`)、`kp_trigger_focus_resync` | gate の atomic | `open_if_current -> bool` | **世代が一致するときだけ閉じる**(古い期限切れを無視)。engine タイマーの発火は OUTPUT_GATE と FOCUS_RESYNC の**両方**で退避(`message_handlers.rs:~690`。BUG-77 追補) | 世代で冪等 | — | — | なし |
| `WM_EXECUTE_EFFECTS`(WM_APP+15) | `kp_stage_execute`(`result.has_pending`)、`message_handlers.rs:136, 1817`、`key_pipeline.rs:82, 2504` | `handle_wm_execute_effects`→`drain_deferred`+`drain_runtime_requests` | 自己 PostMessage | なし | **`with_app` が `None`(再入)なら捨てる**(`app/mod.rs:517`、`let _ =`)。`recover_pending_drain_request` が補う(DRAIN 側のみ) | Yes(通知) | Yes | 連続した post は 1 回に畳める(同じ drain を回すだけ)。未確認 | なし |
| `WM_DRAIN_OUTPUT_QUEUE`(WM_APP+18) | `OutputActiveGuard::drop`、`replay_later`、`message_handlers.rs:43, 571` | `handle_wm_drain_output_queue`(`DRAIN_PENDING`/`DRAIN_RERUN_PENDING` で多重化を制御) | 自己 PostMessage | なし | **`WM_TIMER` より優先される**ので drain は再アームされたタイマーより先に走る(`message_handlers.rs:~680` の理由)。drain の最後に `deferred_engine_timers` を os_id 照合で replay | Yes | — | `DRAIN_PENDING` で畳む | なし |
| `WM_ASYNC_IME_APPLY_COMPLETE`(+22) | `post_async_ime_apply_complete`(`message_handlers.rs:859`。`encode_outcome` で wparam/lparam にパック) | `handle_wm_async_ime_apply_complete`→`on_ime_apply_complete` | 自己 PostMessage | **`ImeOpenOutcome`(+generation)を運ぶ**(唯一の「結果を返す」自己メッセージ) | **`with_app_or_repost_with`(再入なら repost)**。repost で完了の処理順が入れ替わりうる(未確認: 実害の記録なし) | **No**(2 回処理すると `applied` を 2 回書く) | No | **No** | `ImeOpenApplied` 他 |
| `WM_KANA_LOCK_WARNING_CHANGED`(+26) | `key_pipeline.rs:2540, 2544` | `handle_wm_kana_lock_warning_changed` | トレイ表示 | なし | **値を wparam/lparam に積まない**。dispatch 時に毎回ライブの `warned()` を読む(repost で順序が入れ替わっても収束する=冪等にするための設計。`lib.rs:345-352`) | **Yes(意図的)** | Yes | **吸収できる**(最新の値だけ読む) | なし |
| `WM_HOOK_IME_MODE_DIAGNOSTIC`(+27) | `hook.rs:1504` | `handle_wm_hook_ime_mode_diagnostic`(`with_app_or_repost`) | なし(journal へ吸い上げ) | なし | フックの診断ログを `HookImeModeDiagnostic` へ | Yes | — | — | `HookImeModeDiagnostic` |
| `WM_KEY_FROM_HOOK`(+19) | `hook_channel.rs:208, 301`(`post_to_main_thread_quiet`) | `app/mod.rs:578`(`HOOK_KEYS.consume_all`→`handle_hook_key_event`) | 自己 PostMessage | なし | `WAKE_PENDING=false` を先に。`dropped>0` なら `mark_needs_engine_resync` | Yes(通知。キーは ring に溜まっている) | — | 連続した post は畳める(ring を全部消費) | なし |
| `WM_PANIC_RESET`(+16)・`WM_IME_KIND_CHANGED`(+21)・`WM_FOCUS_KIND_UPDATE`(+12)・`WM_RELOAD_CONFIG`(+10)・`WM_DUMP_JOURNAL`(+20)・`WM_DUPLICATE_INSTANCE`(+17)・`WM_ENGINE_QUIT_REQUEST`(+25) | `panic_detect.rs:101`、`gji_monitor.rs:364/395/441`、`focus/uia.rs:285`、トレイ/設定、`app/mod.rs:640`、`bootstrap.rs:1091`、`win32.rs:115` | `app/mod.rs:505-607` の match | 自己 PostMessage / quit | なし(`WM_FOCUS_KIND_UPDATE` は結果を wparam に積む。`FOCUS_KIND_UPDATE_NO_APP_KIND` センチネル) | `with_app` の再入の扱いがバラバラ: `PANIC_RESET` だけ repost、`IME_KIND_CHANGED`/`DUPLICATE_INSTANCE`/`DUMP_JOURNAL`/`EXECUTE_EFFECTS` は `let _ = with_app`(**再入なら捨てる**)。`WM_RELOAD_CONFIG` は `with_app` を経由しない | 通知は概ね冪等 | — | — | `DumpTriggered`(`WM_DUMP_JOURNAL`) |
| `PassthroughQueue`(`runtime/transport.rs:26`: `check_keyup_symmetry`/`check_output_guard_defer`) | `run_passthrough_pipeline`(`executor.rs:~480`) | `enqueue_reinject` | — | `Option<RawKeyEvent>`(defer するなら reinject するイベント) | A(KeyUp 対称性)→B(output guard defer)の順。**Down が defer されたら Up も reinject**(Down=reinject・Up=OS 直通の非対称を防ぐ。BUG-131/132 と同型の Down/Up 非対称ファミリー) | — | — | No | 件数のみ |
| `PhysicalKeyDisposition::{Allow, Suppress}` | `plan()`(`state/physical_disposition.rs`、純関数)→`kp_latch_keyup_to_keydown_disposition`(`key_pipeline.rs:222, 271`) | `execute_relay`(`PassThrough` 時のみ参照。`Consume` では参照しない。`executor.rs:~395`) | フックの結果(OS へ届けるか) | — | **`plan()` の直後・`KeyInput` journal の記録の前に `kp_latch_keyup_to_keydown_disposition`**(BUG-173 追補。記録されるのは `plan()` の判定値で、実際に OS へ届いたかではない) | — | — | **KeyUp は最初の KeyDown の配送に揃える**(Down=Suppress・Up=Allow の非対称防止) | `KeyInput.physical`(`PhysicalDispositionSummary`) |

## 7. 吸収が危険な実例(根拠表)

次の 7 件は、**「同じ(または包含される)書き込みを省略・合体した」ことが実害になった、または実害を防ぐための明示的な例外が既にある**もの。Plan 案(round1)の M1 を支える。

| # | 場所 | 吸収(省略)の内容 | 何が壊れたか / 何が防いでいるか | 根拠 |
|---|---|---|---|---|
| 1 | `gji_direct_already_matches`(`ime_actuation_decision.rs:224`) | shadow(`applied`)が「既に向きと一致」なら `VK_IME_OFF` を送らない | **BUG-141**: GJI×Edge で 2・3 回目の Ctrl+無変換が `AlreadyMatched` で握りつぶされ、候補ウィンドウの再表示=desync の証拠(`candidate_was_seen`)を持っているのに判定に渡っていなかった。初回調査は journal の `Applied` だけを見て「成功」と誤報告 | `docs/known-bugs/BUG-141.md`、ADR-171(`040536bf`)。現行: `(open || !candidate_was_seen)` を条件に含める |
| 2 | 同上(明示キー) | 押下の書き込みでも `applied` を根拠に省略 | **ADR-208 S-1**: GJI×Blind 窓(Imm32Unavailable/TsfNative)で、古い `applied` により絶対指定キーが何度押しても省略され続ける固着 | ADR-208(L1)。現行: `explicit_press_demotes_applied`/`explicit_press_applied_pair`(押下の order は `shadow_on` を未知にする)。**TsfNative は BUG-124 型の「@」の実機 A/B(L3')が済むまで未適用**(`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE=false`) |
| 3 | `fallback_write`(`open_chain.rs:~470-490`) | ImmCross が `Failed` を返した後の `GjiDirect` で shadow ベースの already-matched | 「自分がこれから送ろうとしている値」を「送信前に書いた belief(`kp_shadow_actuate` の `record_confirmed(false)`)」で握り潰す循環→IME が閉じないまま awase だけ「収束した」と誤記録 | **BUG-113 追補**(Opus round3)。現行: `view.control.shadow_on = None` で上書きしてから `decide_attempt` |
| 4 | `claim_press_write` の `Duplicate`(`press_ledger.rs`) | 同じ押下・同じ向きの 2 回目を書かない | 二重送信(BUG-113: Engine と shadow が同じ打鍵で両方書く)を防ぐのが目的。**ただし重複の完了に `AlreadyMatched` を返すと、書いていない押下が `applied=Confirmed` になる**(PR #419 Opus M-1)。`DUPLICATE_OUTCOME = UnsafeToToggle` にした | `press_ledger.rs:33-39`。また同期で何も送らなかったときの予約の解放(M-2、`outcome_sent_nothing`) |
| 5 | `kp_stage_shadow_ime_toggle` の no-op(belief が既に向きと一致) | 書かない | 物理キーが Suppress される窓では誰も IME に届けない(**INV-L1: 物理が届くか awase が書くかの「ちょうど一方」**)。ADR-208 D4/L3a で、Suppress の窓なら no-op でも書くよう例外化(`kp_shadow_noop_write`、`key_pipeline.rs:1187`)。Allow の窓で書くと二重(BUG-113) | ADR-208 D4/L3a |
| 6 | `GateResult::NotOwned`(InputRelay) | 書かない(丸ごと吸収) | これは**正しい**吸収。ただし gate を 1 か所にしか置かなかったため、実際の呼び出し経路 5 つのうち shadow-toggle 経路が素通りした(issue #136/ADR-119)。現在は 4 関数が独立に再判定(ADR-180) | fix-requires-evidence.md の issue #136 の項 |
| 7 | `INPUT_DEFER` の容量超過 | 最古を捨てる | キー欠落(情報の喪失)。1024 は「drain 詰まりの兆候」として警告ログ | `input_defer.rs:~100-120` |

**既に「吸収が安全」と確認できているもの**: `Kill`(未設定なら no-op)、`CancelProbe`(`probe_id` 不一致は無視)、`WM_KANA_LOCK_WARNING_CHANGED`(値を運ばず毎回読み直す)、`EngineStateChanged` の同値の連続(未確認)、`imm_cross_reobservation_already_matches`(新鮮な Win32 読み値。`None` は常に false)。**共通点は「省略の根拠が陽性の確認済み証拠(新鮮な読み取り・世代・ID)であること」**で、`shadow`/`applied`(awase 自身の記録)を根拠にした省略だけが事故を起こしている。

## 8. 記録・イベント側(命令ではないが、順序の制約の相手)

### 8.1 `ImeEvent`(20)— `inventory-w0-a.md` の表を再利用

発行元・書くフィールドは同ファイル §1 を参照(再掲しない)。E0 として追加で確認した順序: **`ImeStateHub::dispatch_event`(`platform_state.rs:165-199`)は「(`UserImeSetIntent` なら `last_user_explicit_off_ms` を更新)→`event_log.record_at`→`shadow_model.reduce`→`journal.record(ImeEvent)`」の順**。journal の記録は reduce の**後**。本番に発行元が無い variant は 2(`UserImeToggleIntent`、`UserChangedInputMode`。w0-a)。`OpenApplyReason`(6)は `ImeOpenApplied` の理由ラベルで、`ReassertExplicitPhysicalKey`/`ForceOnRomajiCorrection` 相当の経路は ADR-179 で撤去済みだが enum に `ImmBrokenForceOn`/`Bootstrap`/`ExplicitKeyReassert` が残っているかは**未確認**(variant の参照数を数えていない)。

### 8.2 `JournalEntry`(22)の本番の生成元

| variant | 生成元(本番) | 備考 |
|---|---|---|
| `KeyInput` | `key_pipeline.rs:229` の 1 か所(architecture_guard が出現数を固定) | `Decision` は件数のみ。`physical` は `plan()`+latch 後の値 |
| `TimerFired` | `message_handlers.rs:723` | **engine タイマーの発火のみ**(OUTPUT_GATE 中の退避・replay 経路 `:1831` は記録しない=未確認の穴) |
| `ImeEvent` | `platform_state.rs:199`(`dispatch_event` の 1 か所) | tick/Instant/seq を持たない(w0-a) |
| `ConvClassifyCall` | `key_pipeline.rs:788` | |
| `ImeActuation` | `ime_refresh.rs:809, 963`(drift correction) | |
| `ActuationDecision` | `ime_refresh.rs:1008`、`key_pipeline.rs:1388`、`executor.rs:759, 883`、`open_chain.rs:636, 674, 711` | **5 つの合流点それぞれが自分で記録する**(`caller`/`site` で区別) |
| `SentInput` | `platform.rs:99`(`drain_journal_entries` が `win32::drain_sent_input_trace` を変換) | seq/elapsed は送信時に採番済み(遅れて drain しても因果順を保つ) |
| `DriftGiveUpDiagnostic` / `DriftGiveUpIntervalEnded` / `GiveUpFollow` | `ime_refresh.rs:1076` / `focus_tracking.rs:618` / `ime_refresh.rs:318` | |
| `HookImeModeDiagnostic` | `message_handlers.rs:105` | |
| `ImeOpenApplied` | `runtime/mod.rs:932` | **`record_ime_apply_result` の前**に記録 |
| `PressWriteClaim` | `platform_state.rs:235, 253` | claim と release の両方 |
| `FocusTransition` | `focus_tracking.rs:64` | |
| `GjiFsmTransition` | `platform.rs:135, 425` | |
| `TsfProbeStarted` / `TsfProbeCompleted` / `LiteralDetect` | `platform.rs:158 / 182 / 207` | `probe_id` で突合 |
| `DeferredRecoveryFlush` | `platform.rs:869, 903` | |
| `GjiReinitRetryCompleted` | **本番の生成元なし**(`git grep` で `journal.rs` の定義・lane 分類・表示 `:1149` と、`platform.rs:890` のコメントだけ) | ADR-212 P3/P5 で reinit を撤去した名残とみて**死んだ variant の疑い**(未確認: テストでの使用) |
| `ClockAnchor` / `DumpTriggered` | `message_handlers.rs:1206, 1984`、`bootstrap.rs:734` / `:1213, 1993` | |

journal に**載らない**命令: `TimerEffect::Set`/`Kill`、`InputEffect` の中身、`GjiAction`(`GjiFsmTransition` に状態遷移は載る)、`RuntimeRequest`、`WM_*` の post、`INPUT_DEFER` の出し入れ、`ProbeCmd`/`ActuateCmd` 単体(上位の `ActuationDecision` と `ObserverReported` を通じて間接的に)。

### 8.3 `EngineCommand`(9)・`TrayCommand`(14)

| variant | 生成元 | 実行 | 順序・冪等 |
|---|---|---|---|
| `EngineCommand::ToggleEngine` | トレイ/ホットキー(`runtime/mod.rs:1004`) | `Engine::on_command`(`engine.rs:635`) | **`on_command` の前に `was_user_enabled` を取る**(`toggle_enabled` が同期的に書き換えるため。`runtime/mod.rs:~990`)。実行後 `discard_ime_open_request`。**冪等でない**(トグル) |
| `ForceEngineOn` | トレイ「状態をリセット」(`mod.rs:1023`) | `force_enable_and_activate` | 冪等 |
| `InvalidateContext(reason)` / `FocusChanged` / `RefreshState` | `mod.rs:1032`、`ime_refresh.rs:410, 1116`、`mod.rs:1415`、`message_handlers.rs:124` | `flush`/`handle_focus_changed`/`check_active_transition` | `FocusChanged` は **flush→`release_pending_and_reinject`→`check_active_transition` の順**。flush は `ThumbRawVkEmission::Denied`(別窓へ生 VK を注入しない) |
| `SwapLayout`/`ReloadKeys`/`UpdateFsmParams`/`SetNgramModel` | 設定/トレイ | 同 | 設定の適用。`SwapLayout` は保留キーを flush する副作用があり `discard_ime_open_request` が要る |
| `TrayCommand`(14) | `handle_wm_command`(`message_handlers.rs:1177-1286`) | 各ハンドラ(`launch_settings`、`restart_as_admin`、`toggle_engine`、…) | 概ねユーザー操作 1 回=1 実行。`Toggle`/`ToggleAutoStart` は非冪等、`ResetState`/`ClearImmCache` は冪等寄り(未確認) |

### 8.4 その他の計画型(`HalfWidthAlnumEffect`・`FocusProbeEffect`・`HookWatchdogAction` ほか)

- **`HalfWidthAlnumEffect`(4)**: 計画は純粋(`state/half_width_alnum.rs`)、実行は `kp_shift_conv_guard_key_up`(`key_pipeline.rs:1945-2090`)の手書きの match。**IMC 経路の順序**: `commit_enter_imc`(latch。`actuate_conv_mode` の**前に無条件**)→`note_explicit_ime_action`→`actuate_conv_mode`→`confirm_gate_deadline_override_ms.set`→`bump_shift_conv_guard_gen`→診断の async 読み取り→`apply_input_mode_correction(ObservedEisu)`。**GJI 経路は逆**: `send_gji_half_width_alnum_toggle` が**成功した後にだけ** `commit_enter_gji`(真の commit-on-success)。同じ variant 名の兄弟で commit の位置が反対。
- **`FocusProbeEffect`(3)**: `plan_focus_probe`(純粋)→`apply_focus_probe`(`key_pipeline.rs:2727-2760`)が薄いインタプリタ。`Record{release_guard}`/`Suppressed{reason}`(grace 中の `false` 観測は記録しない)/`NotObservable{release_guard}`。
- **`HookWatchdogAction`(9)**: `decide`(純粋)が返し `evaluate_hook_watchdog` が `SendCanary`/`ReinstallWithoutCanary` のときだけ実行。`Skip*` の 7 variant は「実行しない」を理由で区別するだけの記録用(命令としては no-op)。
- **`KanjiRolePlan`(3: `KeepStatic`/`Passive`/`Derive`)**: `state/key_effect_runtime.rs:665-683`。純粋。**`ShadowImeAction`(3)/`ShadowImeEffect`(3)** は `RawKeyEvent.ime_relevance` の注釈で、Down/Up のラッチ(`latch_step`、BUG-131/132/173)が非対称を防ぐ。
- **`ImeOpenOutcome`(7)**: `wrote_open_state()`(`Applied`/`AppliedWithoutSendInput` のみ true)と `falls_through`(`Failed` のみ)と `outcome_sent_nothing`(`UnsafeToToggle`/`NotOwned`/`Unwarranted`)と `ImeEvent::from_apply_outcome`(`Applied`/`AppliedWithoutSendInput`/`AlreadyMatched`→Succeeded)の **4 つの別々の述語**が同じ 7 値を別の切り方で分類する。どれも網羅 match ではなく `matches!` のものがあり(`outcome_sent_nothing`、`falls_through`)、variant を足すと黙って片側に落ちる(未確認: `wrote_open_state` だけは網羅 match に書き直されている)。

## 9. 集計

**調査した enum: 54 型・variant 244**(`state` 側の 10 数型を含む。スクリプトで数えた)。内訳(種別):

| 種別 | 型数 | variant 数 | 備考 |
|---|---|---|---|
| C 命令(実行器あり) | `Effect` 系 5(`Effect` ラッパー 4 を含む)+ `KeyAction` 1 + `TimerCommand` 2 + `MechanismCommand` 1 + `ActuateCmd`・`ProbeCmd` 2 + `ProbeAction`・`GjiAction` 2 + `RuntimeRequest` 1 + `GateAction` 1 + `HalfWidthAlnumEffect`・`FocusProbeEffect` 2 + `TrayCommand` 1 + `EngineCommand` 1 = 19 型 | 4+2+2+1+(2)+2+9+2+4+2+7+5+1+2+4+3+14+9 ≈ **79**(`Effect` ラッパー 4・`SpecialKey` 14 を除外すると命令 variant は 約 65) | 数え方: `InputEffect` 2・`TimerEffect` 2・`ImeEffect` 1・`UiEffect` 1 |
| 自己メッセージ(enum でない) | 13 定数 | 13 | `WM_*`。ほかに論理タイマー ID 12 |
| S 選択・計画 | 15 型 | 約 55 | |
| R 記録・イベント | 6 型 | 約 64(`ImeEvent` 20・`JournalEntry` 22・`GjiEvent` 11・`GateEvent` 4・`OpenApplyReason` 6・`SyncKeyGateEvent` 2) | |
| E エンジン内部 | 6 型 | 17 | 個別の深掘りは**未確認** |

**OS への副作用の種類(命令ごと)**:

| 副作用 | 命令 |
|---|---|
| SendInput | `InputEffect::SendKeys`/`ReinjectKey`、`KeyAction` 全般(`Suppress` を除く 8)、`MechanismCommand::SendVk`、`ProbeAction::Transmit`/`TransmitSingleVk`/`RawTsfLiteralRecovery`(後続の BS 予約) |
| IMM/`SendMessageTimeoutW` | `MechanismCommand::SetOpen*`(3)、`ActuateCmd`(2)、`ProbeCmd`(2。読み取り) |
| SetTimer/KillTimer | `TimerEffect`(2)、`timed_fsm::TimerCommand`(2)、`output::TimerCommand`(2) |
| PostMessage(自己) | `WM_*` 13、`RuntimeRequest::StartTsfProbe`(間接) |
| トレイ/グローバル atomic | `UiEffect::EngineStateChanged`、`TrayCommand` |
| なし(プロセス内のキューや記録) | `GateAction`、`GjiAction::DiscardPending`/`SendInput`(無視)、`ProbeAction::Done`/`Note`/`CompositionConfirmed`(グローバル atomic のみ)、`Decision` |

**結果が返るか**: 結果を返す命令は `MechanismCommand`/`WriteMechanism` の実行(`ImeOpenOutcome`)、`ActuateCmd`(`Option<usize>`)、`ProbeCmd`、`ProbeAction::Transmit`(`ze_bs_count`)、`set_ime_open_then_conv_for_target`(`ImmCrossOutcome`)、`GateAction::DrainHeld`(バッファ)、`PassthroughQueue::check_*`(`Option<RawKeyEvent>`)、`WM_ASYNC_IME_APPLY_COMPLETE`(結果を運ぶ唯一の自己メッセージ)。**`Effect` の 6 variant・`KeyAction` 9・`TimerCommand` 6・`GjiAction` 5・`RuntimeRequest` 1 は結果なし**(結果は後の観測・`WM_TIMER`・`GjiEvent` として別経路で戻る)。

**冪等**: 確認できた=`TimerEffect::Kill`/`TimerCommand::Kill`、`GjiAction::CancelProbe`(`probe_id` 一致のときのみ)、`WM_KANA_LOCK_WARNING_CHANGED`(意図的設計)、`ProbeCmd`(読み取り)、`ActuateCmd` の値(絶対指定。ただし `PressLedger` が 2 回目を意図的に止める)、`WriteMechanism::GjiDirect/MsImeDirect` の VK(絶対指定 `VK_IME_ON/OFF`)。**確認できなかった/成り立たない**=`TimerEffect::Set`(論理的には冪等だが os_id が変わり観測上は非冪等)、`InputEffect::*`・`KeyAction::*`(非冪等)、`ImeEffect::SetOpen`(`press` で意図的に非冪等)、`WM_ASYNC_IME_APPLY_COMPLETE`(2 回処理で `applied` を 2 回書く)、`RuntimeRequest::StartTsfProbe`(未確認)。

## 10. 法則の候補と反例(根拠つき)

| 等式 | 判定 | 根拠 |
|---|---|---|
| `Kill(id); Kill(id) = Kill(id)` | **成立** | `Win32Timer::kill`(`timer.rs`)は `to_os.remove` が `None` なら何もしない。`timed_fsm::TimerCommand::Kill` の doc |
| `Set(id,d1); Set(id,d2) = Set(id,d2)`(論理) | **成立(論理のみ)** | `TimerRuntime` doc の不変条件。`Win32Timer::set` も旧マッピングを置き換える |
| 同上(観測込み) | **不成立** | `Set` ごとに新しい OS ID を割り当てる。`handle_wm_timer` が OUTPUT_GATE 中の発火を `(logical_id, os_id)` で退避し、drain 後に `current_os_id == os_id` のときだけ replay する(`message_handlers.rs:~690, 1831-1850`)。`Set;Set` を `Set` に畳むと、退避済みの発火が別のタイマーとして捨てられる。実例(コメント): 「というのは→とはいうの」(drain 中に旧タイマー kill→新タイマー set で別の文字に属する) |
| `Set(id); Kill(id) = Kill(id)` | **未確認(観測上は不成立の疑い)** | Set→Kill の間に `WM_TIMER` が発火しうる。実害の記録は見つけていない |
| 別 ID の Timer は他の命令と可換 | **おおむね成立** | ただし `Consume` では Timer だけ即時でキューを追い越す(§1)ので、「Timer と `SendKeys` の相対順は保たれない」が現行の仕様 |
| `SetOpen(x); SetOpen(y) = SetOpen(y)` | **不成立・危険** | §7 の #1〜#5。`PressLedger` の予約・`applied` の更新・`ImeOpenApplied` の journal・`post_ime_refresh` の 20ms Set が、**各書き込みの副作用として観測される** |
| 書き込み(`SetOpen`)と `SendKeys` の可換性 | **不成立** | 順序が出力文字列(IME の ON/OFF で同じ romaji が別の結果になる)。`on_ime_applied` が `mark_composition_cold` を呼ぶため、後続の `SendKeys` の cold/warm 判定が変わる |
| `post_ime_refresh`(`Set(TIMER_IME_REFRESH,20ms)`)の重複 | **吸収可能だが、他の Set と競合** | `on_ime_apply_complete` は `UnsafeToToggle` でも必ず Set する。同じ ID を 5 経路が別遅延で Set/Kill する(§4) |
| `claim_press_write` + `release_press_write` | **対になるが、同期=解放あり・async=解放なしの非対称** | `dispatch_ime_set_open` と `kp_shadow_actuate` の両方で、sync は `outcome_sent_nothing` で解く。async は「完了が後から届く」ため解かない(次の押下で直る) |
| `Decision` への `prepend_effects`/`stamp_set_open_press`/`push_effect` | **成立(順序保存)** | `decision.rs:~245-300`。`prepend_effects` は空なら no-op、`PassThrough` を `PassThroughWith` に格上げ。`stamp_set_open_press` は既に ID を持つものを変えない(冪等) |
| `ReinjectKey` のバッチ内の可換 | **不成立** | Win_DOWN→X_DOWN→X_UP→Win_UP を個別にガードすると Win が 150ms 以上スタックして後続が Win+key に誤解釈される。先頭の reinject が guard を通ったら残りはまとめて送出(`executor.rs:~200-215`) |
| `INPUT_DEFER` の enqueue 順 = drain 順 | **不成立(意図的)** | `take_all` は `timestamp` 昇順に整列する |

## 11. 順序の制約が暗黙のもの(ガード・型が無く、呼び出しの順だけで保たれている)

F の分割(判断の入口を `decide → Cmd` に出す)で**壊しやすい**順に。「ガード」欄は、私が `git grep` で確認できた範囲(architecture_guard/lint/テスト名)。確認できなかったものは「未確認」。

| # | 順序 | 場所 | 根拠(BUG/ADR) | 型・ガード |
|---|---|---|---|---|
| 1 | `plan()`→**`kp_latch_keyup_to_keydown_disposition`**→`KeyInput` journal→`kp_stage_execute` | `key_pipeline.rs:211-250` | BUG-173 追補、BUG-90(journal 記録用に独立再計算していた乖離窓) | `physical` を 1 回だけ確定して両方へ渡す(コメント)。型での強制なし。「`plan()` の判定値が記録される」注記あり |
| 2 | **`kp_stage_shadow_ime_toggle` を `engine.on_input` の前に**(ctx は shadow の書き込み**後**の belief で組む。ただし `engine_owns_open_key` は shadow の判断**前**の belief で組む) | `key_pipeline.rs:~125-175` | ADR-208 D1(PR #419 Opus M-4) | 型なし。`pre_ctx` と `ctx` の 2 つの `build_input_context` が時刻違いで存在(ADR-208 で意図) |
| 3 | `claim_press_write`→order 発行→(sync: 何も送らなければ `release_press_write`) | `executor.rs:689-`、`key_pipeline.rs:1238` | ADR-208 L1、BUG-113 | `architecture_guard::press_id_is_claimed_and_carried_at_every_order_issuing_entry`(入口 2 つに 3 点の配線があることを固定)。**release の条件**(sync だけ、`outcome_sent_nothing` だけ)は型でなく呼び出し側 |
| 4 | `kp_shadow_actuate`: `timer.kill(TIMER_IME_REFRESH)` を**書き込みの前に**、`record_confirmed(false)` を**async 書き込みの前に** | `key_pipeline.rs:1265, 1290` | PR #408 Opus M-1、ADR-098 決定 5/6-a | 型なし。コメントのみ |
| 5 | `mark_send()` を SendInput の前、`prev_elapsed_ms` を `mark_send()` の前に読む。`send_keys` 内で診断スナップショットを取らない | `output/mod.rs:698-711` | 「境界dえ」(WH_KEYBOARD_LL の再入)、output in-flight guard の基準点 | 型なし |
| 6 | 検出ベースラインを SendInput の前に取る(`new_with_pre_send_baseline`) | `output/probe_io.rs:~490, 517-` | BUG-027/029/030/033 | 型なし(送信後にベースラインを取る呼び出しが書けてしまう) |
| 7 | `drain_pending_deferred_before_send_if_queue_only`→**`assess_warmth`**(drain が先) | `vk_send.rs:~60-75` | ADR-123 変更 A+C 決定 4-3(round3 指摘) | 型なし |
| 8 | `drain_pending_composition_events`→`step_probe` | `platform.rs:395-400` | VK_A+BS で SHOW+HIDE が first tick 前に終わる IPC race | 型なし |
| 9 | `ImeStateHub::dispatch_event`: `last_user_explicit_off_ms` 更新→`event_log.record_at`→`reduce`→`journal.record` | `platform_state.rs:165-199` | ADR-229 w0-a | 型なし。journal が reduce の後なので、journal の `ImeEvent` から reduce 前の状態を再構成できない |
| 10 | `on_ime_apply_complete`: `ImeOpenApplied`→`post_ime_refresh`→`record_ime_apply_result`→`on_ime_applied` | `runtime/mod.rs:920-990` | BUG-34 横展開 D-prep(早期 return で pending が残留する固着)、opus F3 | 型なし。`UnsafeToToggle` でも早期 return しない(コメント) |
| 11 | `probe` を `install_pending_tsf()` してから `RuntimeRequest::StartTsfProbe` を push | `outbox.rs:15-20` | H-4-a | doc のみ |
| 12 | `execute_relay` の `Consume`: **Timer は即時、他はキュー** | `executor.rs:430-445` | `deferred_engine_timers` の os_id 照合(「というのは→とはいうの」) | コメントのみ。`Decision` 内の `[Timer, SendKeys]` の並びと**実行順が一致しない**ことが暗黙 |
| 13 | `OutputActiveGuard::begin()` を `spawn_local` の**前**に取る(reinject と async actuation の両方) | `executor.rs:~560`(`handle_reinject`)、`:~800`(async 先頭) | 「いが l になった」、await 中のフック分配 | RAII で release の漏れは防げるが、**取得位置**は型でなく慣習 |
| 14 | `toggle_engine`: `was_user_enabled` を `on_command` の**前**に取る | `runtime/mod.rs:~990` | issue #137 3 周目の指摘 | コメントのみ |
| 15 | `Engine::on_command(ToggleEngine/SwapLayout)` の後に `discard_ime_open_request` | `engine.rs:660, 671` | ADR-092 Step4b(外部イベントによる単独タップ確定を適用しない) | コメントのみ。`on_command` の他のアームに足し忘れても検出されない |
| 16 | IMC 経路の半角英数: `commit_enter_imc` を `actuate_conv_mode` の**前に無条件**、GJI 経路は SendInput **成功後**に `commit_enter_gji` | `key_pipeline.rs:~1995, 2048` | 半角英数トグル §3 原則 2 | 型なし(同じ enum の兄弟で commit の位置が逆) |
| 17 | `ImmCross` の `romaji_pre_write`(同期ブロック)を `apply_mechanism` の最初に。`SendHealth` の gate は意図的に無し | `ime_controller.rs:208, 421-433` | BUG-34 横展開(skip すると ROMAN が次のトグルまで固着) | 型なし。「入れない」ことが決定(再提案されやすい) |
| 18 | `SyncChainWriter::write` が `decide_attempt` を 1 回、`apply_mechanism` が中でもう 1 回(記録用と実行用の二重決定) | `ime_controller.rs:~455-470` | ADR-163 TH1b-2b | **同じ view を渡すことだけで整合**。片方の入力を変えると記録と実送信が乖離。型・テストでの強制は**未確認** |
| 19 | `gate`(`decide_gate`)を await の前後で**毎回**判定し直し、`with_app` を内包する共有ヘルパーにしない | `open_chain.rs`(3 関数)、`executor.rs` | ADR-180 決定 1、issue #136/BUG-90 | `ime_open_actuation_entry_points_are_accounted_for` 等(入口の数)。**再入でゲートが無効化される**点はコメントのみ |
| 20 | `WM_*` の再入の扱い(`with_app` で捨てる vs `with_app_or_repost` で再 post)が handler ごとにバラバラ | `app/mod.rs:505-607` | issue #137(WARN/CLEAR の順序入れ替え) | 型なし。どの WM が捨てられてよいかの表は無い |
| 21 | `INPUT_DEFER` の `defer_during_output`(post しない)と `replay_later`(post する)の非対称 | `input_defer.rs` | ADR-156(2 窓口の片側だけに条件を配線) | doc のみ |
| 22 | defer 側の `raw_recovery_owns_deferred()` は gate で切り替え、drain 側は gate に関わらず常に見る | `vk_send.rs:~90-105` | ADR-128、round4-3 | doc のみ(「非対称」と明記) |

`Effect` 列の生成から実行までの**順序を 1 か所で決めている場所は無い**。順序は (a) `response_to_effects` の並び(タイマー→SendKeys)、(b) `execute_relay` の分岐(Timer 即時)、(c) キューの FIFO、(d) `kp_run_inner` の段の並び、(e) 各段の中の関数の呼び出し順、の 5 層に分散している。F の分割で `decide → Cmd` を作ると、(a)(b)(c) の層が 1 つの `Vec<Cmd>` に畳まれ、**#12(Timer 即時)と #1・#2・#3(段の間の belief 更新)が最も壊れやすい**。

## 12. 未確認の点

- `src/engine` の内部 enum(`ParseAction`・`OutputUpdate`・`GuardAction`・`SoloTapAction`・`WarnAction`)の各 variant の「実行器」「冪等」は**深掘りしていない**(OS に出ない FSM の内部の選択型として種別 E に置いた。`NicolaFsm` の外には出ない)。
- `ReinjectKey` を engine が直接返す箇所(`engine.rs:332`)の発火条件、`PassThrough` の `reinject` 経路(`runtime/transport.rs::PassthroughQueue` の全メソッド)の詳細。
- `JournalEntry::GjiReinitRetryCompleted` が本当に本番で構築されないか(`git grep` では定義・分類・表示のみ。テストの使用は見ていない)。`OpenApplyReason` の `ImmBrokenForceOn`/`Bootstrap`/`ExplicitKeyReassert`・`DecisionSite` の `ReassertExplicitPhysicalKey`/`ForceOnRomajiCorrection`/`ForceOnBootstrap` に本番の参照が残っているか(ADR-179 で経路は撤去済み)。
- `TimerFired` の journal が OUTPUT_GATE 退避後の replay(`message_handlers.rs:1831-1850`)を記録しない件の実害。
- `Set;Kill`→`Kill` の畳み込みの実害、`StartTsfProbe` の二重 push の防御、`EngineStateChanged` の同値連続の吸収は、コード・BUG のどちらにも記録が見つからず**未確認**。
- `WM_*` の「再入で捨てる」ハンドラ(`EXECUTE_EFFECTS`・`IME_KIND_CHANGED`・`DUPLICATE_INSTANCE`・`DUMP_JOURNAL`・`FOCUS_KIND_UPDATE`)で実際に消えた通知の記録。`recover_pending_drain_request` が補うのは DRAIN 側のみ。
- `ime-belief-architecture.md` は全文を読んでいない(`ImeEvent` の dispatch 規約と BUG の背景節)。w0-a の分類を再利用した部分はそれに依存する。
- `ActuationDecisionRecord` から `replay_record` で再生できる範囲(ADR-163 TH1b/TH1d)の詳細は見ていない。journal 全体からの再現は w0 README のとおり不可。
- 命令型の「variant の総数」は enum 54 型の合計 244。**`DecisionSite`・`PendingDiscardReason`・`SpecialKey` などラベル・部品の型を含む**ので、命令だけの数は §9 の約 65(C 種別の見積もり。`Effect` ラッパーと `SpecialKey` を除外)。正確な「命令 variant 数」は分類の約束に依存する。
