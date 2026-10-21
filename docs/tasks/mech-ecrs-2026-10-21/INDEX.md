---
title: 機構レベルの ECRS — 撤去・統合の候補と、擬似 IME の整備の順序
status: 調査のみ(2026-10-21、origin/develop f3c2007e)。実装は未着手。調査エージェントの主張のうち、下の「確認済み」とした分だけ統合時に grep で再確認した。それ以外は着手前に必ず根拠を再検証すること
created: 2026-10-21
related_adr: ["ADR-158", "ADR-163", "ADR-224", "ADR-241", "ADR-275"]
---

# 機構レベルの ECRS — 撤去・統合の候補と、擬似 IME の整備の順序

所有者の方針(2026-10-21): 「擬似 IME をもっと整備した上で、撤去してよい機構・統合できる機構を無くして、コードをシンプルにしたい」。
方法: 5 領域を読み取り専用で調査した(Sonnet、各自 grep と通読、cargo 未実行)。生の報告は `raw-a-actuation.md`・`raw-b-tsf.md`・`raw-c-belief.md`・`raw-d-focus.md`・`raw-e-pseudo-ime.md`。
先に行ったコード単位の ECRS は `docs/tasks/ecrs-awase-windows-2026-10-20/`。

## 1. 数

| 領域 | E(撤去可) | C(統合可) | ?(要実測・要判断) |
|---|---|---|---|
| A actuation | 4 | 2 | 2 |
| B TSF・warmup・defer | 9 | 4 | 4 |
| C belief・観測・予測 | 5 | 2 | 2 |
| D focus・ゲート | 4(+所有者判断 2) | 0(確実なもの)| 多数 |

「到達しない」「呼び出し元ゼロ」の E が大半で、**統合(同じ問題を 2 経路が解く)の確実な候補は少ない**。合流点の数は ADR-180 が既に結論しており、今回も同じ(MA-05 が最有力の C)。

## 2. 着手の三段(擬似 IME が要るかどうかで分ける)

### 第 1 段 — 擬似 IME 不要。呼び出し元ゼロ・読み手ゼロで、grep で裏付く

テストだけが使うコード・定義だけで使われていないコードの撤去。挙動は変わらない。

| ID | 対象 | 確認 |
|---|---|---|
| MA-02 | `DriftEpisode`・`ActuationOrder::into_actuation_shadow`・`Authorization`、古い `log_shadow_warrant` 文言と architecture_guard のコメント | **確認済み**(`git grep`: `DriftEpisode` は `actuation_chain.rs` 自身のテストのみ、`into_actuation_shadow` は定義のみ) |
| MA-08 | `ime.rs::check_tsf_composition_active` | **確認済み**(定義のみ) |
| MA-03 | `AttemptRecord.with_app_available`(本番で常に `true`) | 一部確認(`journal.rs:1360` が読む。値は `true` 固定。journal の形式に触るので `serde(default)` に注意) |
| MC-E1 | `key_effect_rules.rs`(340 行)・`keymap_initial_hypothesis.rs`(247 行) | **確認済み**(mod 宣言と `layer_boundary_guard` の一覧だけに現れる) |
| MC-E2 | 書き込まれない 4 つの観測スロット(`gji`・`tsf`・`hwnd_cache`・`imm_get_open_status`)、`ForceOnReason::ProfilePolicy`・`expires_at`・`purge_expired` | 一部確認(`ProfilePolicy` の本番での使用は `platform_state.rs` のテスト 1 件)。`ObservationStore::consensus`・`record`・`record_belief` はテストのみだが「型付きエントリの設計意図」があり ? |
| MD-01 | `uses_kanji_toggle` | **確認済み**(`class_names.rs` のテストのみ) |
| MD-02 | `is_chromium_widget` | **確認済み**(同上) |
| MD-03 | `classify_focus` で結果が捨てられる `ImmGetContext` | 未確認 |
| MD-04 | `FocusResyncGate::disarm`・`armed_at_ms`(focus_resync.rs) | 一部確認(`disarm` はテストのみ。同名の `armed_at_ms` は mode_key_pass.rs の別物なので混同しないこと) |
| MB-04 | `EVENT_OBJECT_NAMECHANGE` の WinEvent フック(全プロセス対象)。書き込み先 `focus_namechange` の読み手がゼロ | **確認済み**(`git grep focus_namechange` は定義・初期化・`notify()` の 3 箇所のみ)。**フックを外すので実行時の挙動が変わる**(メッセージループの起床が減るだけ) |
| MB-06 | `RuntimeOutbox`(1 バリアント `StartTsfProbe`。同じタイマー設定を `drain_output_post_send_effects` が既に行う) | 一部確認(呼び出し元は `vk_send.rs:318` と `runtime/mod.rs:2328` のみ) |

見積り: 本番 約 −1,200 行(うち MC-E1 の 587 行が最大)。1 PR にまとめず、領域ごとに 3〜4 PR。

### 第 2 段 — 「到達しない」ことの証明が要る(推論+ガードで固定してから撤去)

| ID | 対象 | 要るもの |
|---|---|---|
| MA-01 | 同期チェーンの ImmCross 腕 `SetOpenCrossProcessSync`(同期チェーン 3 入口はどれも ImmCross が先頭で適用可能なとき非同期へ回す) | `DecisionInputs` を全列挙する純粋テスト(Linux)。**推論は報告のとおりで、入口 3 つの条件は確認した**が、全列挙で固めてから撤去 |
| MB-01 | cold コルーチンの inline LiteralDetect(Phase 3 / Phase 6)。per-VK confirm が必ず先に取り、`cold_seq` は毎回新規採番 | windows-build でしか走らない(`output/` は cfg(windows))。**先に `debug_assert!(!plan.needs_literal)` を `Transmit` アームに足して CI の e2e で通ることを確認してから撤去**。warmup 再発ファミリー: fix-requires-evidence (a)/(b) 必須 |
| MB-05 | `session_expired`・`COMPOSITION_TIMEOUT_MS`(`send_keys` が先に `mark_send()` するため `elapsed` が常に 0ms。実機ログ 32 件すべて 0ms) | `architecture_guard.rs` の固定(8363〜8447)を同じ PR で更新。実機ログの母数が小さい(94 ディレクトリ、主に Windows Terminal×GJI)ので、**撤去前に別構成のログで再確認** |
| MB-02 | warm パスの `LiteralDetectFsm`(設置条件が矛盾: TsfGate が Probing かつ非 Tsf モード)。約 −330 行+テスト | **実害の実機ログを 1 回確認してから**(BUG-210 の修正はこの経路向けで、実害ログは未確認と BUG-210 が書く) |

### 第 3 段 — 擬似 IME の整備が先(統合・挙動の等価性が要る)

| ID | 対象 | 先に要る擬似 IME の能力 |
|---|---|---|
| MA-05 | `kp_shadow_actuate` が `dispatch_set_open` の判断(claim → imm_first → 順序発行 → async/sync)を並行実装 | ME-2(`dispatch_set_open` を擬似 IME に配線) |
| MC-C1 | `ImeModel::last_intent` と `IntentStore` が同じ状態を 2 か所に持つ(state/mod.rs・intent_store.rs のコメントは「未配線」と言うが、配線されている=コメントが古い) | ME-1 → ME-2。**ADR が先** |
| MC-C2 | 「外部変化に追随する」3 経路(ADR-187/188/205)が同じ後半をコピー | ME-4 |
| MD-07〜10 | 窓切替直後の保持・読み・先同期の重なり(MD-10 に `ir_post_focus_change_snapshot` 側の placeholder ガード欠落の疑い=BUG-163 の趣旨との食い違い) | 窓モデル(ME-5) |
| MB-10 | 2 本のコルーチン(cold の TSF と Chrome)の統合(MB-01・02・03 の後) | MB-01 が済んでから |

### 所有者判断(コードの正否ではなく方針)

- **MD-05** インジケーターのメニュー読み取り(約 1,270 行+テスト)。既定 off で belief には使わないと ADR-273 が決定済み。
- **MD-06** 字形の取り込み。一太郎の実機確認待ちなので撤去は勧めない。
- **ime_profile_driver.rs**(400 行): 呼び出し元ゼロだが ADR-090 が意図して凍結。
- MB-07(`CompositionOutput` の将来の使い道)・MB-09(ADR-103 決定 5-a との整合)・MB-16(MS-IME で `EVENT_OBJECT_IME_*` が発火するかの実測)。

## 3. 擬似 IME の整備(ME 番号は raw-e-pseudo-ime.md)

今のハーネスは、本番の `ImeStateHub`・`ImeModel::reduce`・warrant・drift の検知・key-effect 予測・`Engine`(`RefreshState`/`FocusChanged` のみ)を実コードで通す。`runtime/`・`ime_controller.rs`・`output/` は 1 本も通らない。写しは約 160 行。

| 順 | 内容 | 規模 | 何が検証できるようになるか |
|---|---|---|---|
| ME-1 | 写し(`settle()` の drift 分岐、`PredictInput` の `unreadable`/`observable_toggle_guard`/半角英数の track)を、既にある本物の核(`decide_drift_plan` など)の呼び出しに置換。古い表(closed_loop_scenarios.rs:18-22)も直す | 1 PR | drift の `is_japanese_ime` の 2 重の関門(今は区別できない)、ADR-209/260 の予測の相互作用 |
| ME-2 | ADR-241 PR-B(所有者承認済み・未着手): ハーネスを crate 内へ移し、`CommandSink` に擬似 IME を差して `dispatch_set_open`・`apply_sync` を通す | 数 PR | `MechanismCommand` のバリアント列(MA-05 の統合、`decide_attempt` の再計算) |
| ME-3 | 純粋な判断(`decide_gate`・`decide_drift_plan`・`plan_set_open`・`warm_send_plan`・`relay_plan`・`deferred_gate_plan` など)の**決定表の全列挙 golden**(`explicit_press_exhaustive` の方式)。候補の PR の**前**に足す | 判断 1 系統 1 PR | 撤去・統合の「差分ゼロ」の主証拠。TH1e(N 本の再生)の代わり(ADR-241 §5 が再生一式の撤去を決めているため) |
| ME-4 | 通過マーク・外部変化・placeholder 揃え・直接読みの 4 機構を 1 つのハーネスに載せる。`#[ignore]` の `stale_eisu_…` を有効化 | 1〜2 PR | MC-C2 |
| ME-5 | 窓・スレッド表(TsfNative/Plain/InputRelay の構築子を足す) | 1〜2 PR | MD-07〜10 |
| ME-6 | TSF の composition/literal の最小の模擬(測定済みの Chrome×GJI cold/warm に限る) | 撤去候補が出てから | 写しのリスク最大。第 2 段の MB-01/02 は grep とログで足りるので作らない |
| ME-7 | Q6 をキー押下にも効かせる、「明示意図あり×古い読み」シナリオ | 小 | MS-IME 本体の機構を触るとき |

`complexity-budget.md` の発効条件(削除・統合を N 本の再生で差分ゼロと証明)は、ME-3 の全列挙で代える形が現実的(所有者判断、据え置き)。

## 4. 推奨する順序

1. **第 1 段**(擬似 IME 不要、約 −1,200 行)。領域ごとに PR を分ける。
2. **ME-1**(写しを本物に置換。写しがずれていた実例が出たら記録して止める)。
3. **第 2 段**を 1 件ずつ: MA-01(全列挙テストを添えて)→ MB-01(debug_assert を先に出して CI で通す)→ MB-05 → MB-02。warmup 再発ファミリーなので各 PR に fix-requires-evidence の (a)/(b)。
4. **ME-2 → ME-3**、続けて第 3 段(MA-05・MC-C1・MC-C2)。
5. 所有者判断の項目は並行して聞く。

## 5. この索引の限界

- 調査は Sonnet の読み取りで、cargo 未実行。確認済みとしたのは統合時に grep した分だけ。
- 実機ログによる「0 件」(MB-01・MB-02・MB-05)は母数が小さい(Tsf モード送信 32 件)。「その構成で到達しなかった」までしか言えない。
- 擬似 IME は実機の応答を持たない。写しが増えるほど等価性の主張は「模擬に対して」になる。
