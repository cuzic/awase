---
id: ADR-241
title: |-
  打鍵から actuation の決定まで通す再生基盤(第 1 段階: エンジンの SetOpen から同期の機構チェーンまで)
summary: |-
  所有者は 2026-10-06 に、新しい再生基盤の範囲として案 C「打鍵から actuation まで通す」を選んだ(B6 と同規模、1000 行超)。
  既存の再生一式(`replay_record`・RW・凍結コーパス・`Deserialize` 一式)は、この基盤への置き換えと同時に捨てる(決定済み)。
  この撤去は案 B″(撤去だけ)でも同じ量なので、収支は B″ を 0 とした差分で書く。

  背景の実測:
  - KA(キー列とシェル状態から IME 制御の送信が決まる BUG)は 184 件中 48 件(サブエージェントの一次分類)。
  - そのうち、複数押下の状態を持ち越し、hook より上で起き、機構が現存し、打鍵から通さないと再現できないものは、48 件を並べ直すと約 14 件(付録)。
  - キー列を key_pipeline・executor まで通す回帰テストは、実機 CI を除くと 0 件。
  - 閉ループは `Engine::on_input` を呼ばず、`MechanismCommand` を作らない。B5 はエンジンの `SetOpen` で止まる。

  決定(第 1 段階):
  - 同期経路の判断を ungated な核へ移す。対象は `ImeController::apply` の本体と、`dispatch_ime_set_open` の gate→claim→plan→chain。
  - 試行の記録と `decide_attempt` も核の writer に置き、殻と再生の差し替え口は「決まった command を送る」だけにする。
  - `kp_stage_post_decision` の SetOpen の部分(generation・`handle_engine_set_open`・`record_explicit_intent`)をハブの 1 関数にする。
  - 本番と閉ループのハーネスが同じ関数を呼ぶ。
  - 対象のシナリオ: ADR-208 D1 の固定(msedge×GJI)と、BUG-141(TsfNative×GJI の `candidate_was_seen`)。

  合否: 移した核の関数への直接の単体テストを同じ PR で書き、それを対照にする。
  `cargo mutants` で単体テストと既存テストを生き延びた変異のうち、打鍵からの再生でだけ落ちるものが 1 つ以上あることで判定する。
  無ければ、ハーネスへの配線をやめ、移動と単体テストだけを残す。
status: |-
  提案(起草中、Opus レビュー round1 反映済み)
related_adr:
  - "ADR-163"
  - "ADR-171"
  - "ADR-208"
  - "ADR-224"
  - "ADR-225"
  - "ADR-226"
  - "ADR-229"
  - "ADR-232"
  - "ADR-240"
  - "ADR-234"
  - "ADR-119"
  - "ADR-180"
  - "ADR-090"
  - "ADR-089"
  - "ADR-219"
---

# ADR-241: 打鍵から actuation の決定まで通す再生基盤(第 1 段階)

調べた版は develop `4f59292a`(#501 マージ後。B5 の試作 #533 を含む)。ローカルでのビルド・テストはしていない。
行番号は、特に断らない限りこの版の `crates/awase-windows/` からの相対パス。

Opus レビュー round1(Blocker 1・Must 5・Should 6・Nit 5)の要点と反映先:

- B1(合否の基準): 決定 4
- M1(写しの一覧の不足): 決定 2・3
- M2(`record_ime_apply_result` は gated): 背景 3・所有者への質問 3
- M3(writer が写しになる): 決定 2
- M4(収支): 決定 5
- 追加(team-lead の決定): ADR-240 との役割分担は決定 7
- M5(合流点の作り直しの承認): リスク・所有者への質問 1
- S1〜S6・N1〜N5: 該当箇所

## 用語

- **actuation**: awase が IME の開閉を変えるために送るもの(`VK_IME_ON`/`VK_IME_OFF` の SendInput、`ImmSetOpenStatus` 相当など)。
  送信の種類は `MechanismCommand`(`state/ime_actuation_decision.rs:109`)で表す。
- **KA**: キー列とシェルの状態(押下台帳・applied・belief・ラッチ)から actuation の判断が決まり、症状がその送信に出る BUG。
  これは「打鍵から通さないと再現できない」より広い(付録)。
- **核 / 殻**: ADR-229 の FCIS の語。
  - 核は Linux でも動く純粋な判断(`state/`)。
  - 殻は Win32 を呼ぶ側(`runtime/`・`ime_controller.rs`・`state/platform_state/shell.rs` などで、`#[cfg(windows)]`)。
- **写し**: 閉ループのハーネスが、殻の配線を手で書き直した部分(`tests/support/harness.rs:13-20`)。本番が変わっても追随しない。
- **対照の単体テスト**: 核へ移した関数を、FakeWriter などで直接呼ぶ短いテスト(1 本 30 行以内)。打鍵から通す再生の価値は、これと比べて測る。

## 背景(実測)

### 1. 打鍵から actuation まで通さないと再現できない BUG の数

known-bugs は版 `4f59292a` で 184 件ある(`git ls-tree --name-only 4f59292a docs/known-bugs/ | grep -c 'BUG-'`。今の develop では 186 件。BUG-046 は欠番で、本文は `BUG-045.md:64-67` にある)。
分類の方法: タイトルを全件読み、KA の候補約 75 件は本文の状態欄・症状・テスト欄を読んだ(サブエージェントによる一次分類。全文を精読したのは約 15 件)。

| 分類 | 件数 | 確度 |
|---|---:|---|
| KA(うち実 IME の応答も絡む KA+IME は約 25) | 48 | 中(本文を読んで判定) |
| エンジン単体で再現(BUG-105・145 型。B5 の範囲) | 約 11 | 一部未確認 |
| IME 側(cold-start・warmup・literal 検出・Chrome/GJI 固有) | 約 80 | 未確認(タイトルで割り振り) |
| その他(設定 GUI・インストーラ・診断ほか) | 約 40 | 未確認 |

KA の 48 件を、この基盤で要るかどうかの観点で並べ直した(付録に BUG 番号の一覧。並べ直しは一次分類の要約に基づき、未確認)。
観点は 4 つ: 機構が現存するか、hook より上か、複数押下の状態の持ち越しが要るか、非同期が要るか。

| 区分 | 件数 | この基盤との関係 |
|---|---:|---|
| 機構が撤去済み(ADR-178/179・191・206・212 など) | 約 15 | 修正前後の差を見るには古いツリーが要る。対象外 |
| 1 回の判断の単体テスト・golden で足りる(BUG-152 は 1 押下の `run_chain_async` と FakeWriter で足りる) | 約 10 | 要らない |
| hook・OS より下(BUG-181 は hook の `physical_key_state`、BUG-154 は再注入の scan) | 約 5 | 届かない。ハーネスは `RawKeyEvent` から始まり、`was_down` を自分で作る |
| 未再現・実害記録なし(BUG-098・184 など) | 約 4 | 対象外 |
| **複数押下の状態を持ち越し、hook より上で起き、機構が現存する** | **約 14** | **この基盤の対象**(BUG-113・141・156・159・173・046・014 など) |

**キー列を key_pipeline・executor まで通す回帰テストを持つ KA は、実機 CI を除くと 0 件**。今ある検証は次のどれかである。

- 純粋な判断関数の単体テスト(例: BUG-141 は `gji_direct_already_matches` の入力を直接組む)
- 1 押下ぶんの判断の合成(`state/explicit_press.rs` と `tests/explicit_press_exhaustive.rs`)
- 実機 CI

ADR-240 の実測では、状態欄に「CI 検証済み」とある BUG は 19 件ある。そのうち修正を外したビルドで差を確かめたのは BUG-170 の 1 件だけ。

### 2. 今の 3 つの手段が届かない理由

| 手段 | 届く範囲 | 届かない理由(出典) |
|---|---|---|
| 閉ループ(`tests/closed_loop_scenarios.rs`、20 本) | 本物の `ImeStateHub`(P5a-1、#518)、予測器、`check_drift_correction`、Engine の活性遷移 | `Engine::on_input` を呼ばない(`harness.rs:25-27`)。Engine が `SetOpen` を出すと panic する(`:612-626`)。書き込みは「warrant が下りたか」と bool の `write_open` だけ(`issue_write`、`:592-610`、`issue_actuation_order` の写し)。gate・押下台帳・機構チェーン・`MechanismCommand` を通らない |
| B5(`src/key_input_replay_tests.rs`、#533) | `KeyInput` の列 → HEAD の `Engine::on_input`/`on_timeout`。BUG-105 を再現 | エンジンの `SetOpen { open, press }` で止まる(`docs/tasks/journal-replay-rebuild-study-2026-10-06/b5-prototype-result.md` の比較表) |
| 既存の actuation の再生(`state/actuation_decision_record.rs` の `mod tests` `:409-1288`、`#[test]` 17 本。凍結コーパス 1440 行・37 件) | 記録した `DecisionInputs` から `decide_*` を再計算する。同期の走査を再走査する | 打鍵や belief の経緯は入力に無い。37 件は全部 `Sync`・`GjiDirect` の 1 回で、単体テストを超える検査をしていない。走査規則は FakeWriter のテスト(`state/actuation_chain.rs:779`・`:889`)が既に固定している(`docs/tasks/corpus-discard-impact-2026-10-06/README.md` §0) |
| 実機 CI(`e2e-ime.yml` の `sc-*`) | 本物の殻と IME | 1 回の判定に実機ジョブが要る。フォーカスの非決定性がある。修正を外したビルドとの比較はほぼしていない(ADR-240) |

**1 押下の合成(`explicit_press.rs`、2056 行)は、殻の順序の写しである**。

- doc(`state/explicit_press.rs:20-24`)は、「本番の `kp_run_inner` と同じ順序で再現する」「L0 では本番側は本関数を呼ばない」と書いている。
- 本番が使うのは `select_shadow_intent`(`:102`)・`engine_set_open_filtered_by_chord`(`:144`)・`shadow_noop_write_target`(`:752`)だけである。
- 合成のモデル(`attempt_write` `:775-817`、`explicit_press_delivery_with`/`_after` `:818-1039`、`dual_route_*` `:1040-1110`、`ime_after_press`〜`state_after_press` `:1111-1191`)は、本番から呼ばれない。

```sh
git grep -n "explicit_press_delivery_with" crates/awase-windows/src | grep -v state/explicit_press.rs   # key_pipeline.rs:966 のコメントだけ
```

### 3. エンジンの明示 SetOpen が actuation になるまでの経路と、Linux から呼べない箇所

1. `kp_run_inner`(`runtime/key_pipeline.rs:38-`)。shadow toggle の判定 → `engine.on_input`(`:204`)→ `PhysicalKeyDisposition::plan`(`:216`、本体は核の `state/physical_disposition.rs::plan_core`)。
2. `kp_stage_post_decision`(`:1402-1450`)。`SetOpen` のとき次を行う。
   1. `timer.kill(TIMER_IME_REFRESH)`
   2. `allocate_event_generation()`
   3. `handle_engine_set_open(open, ctrl, generation, tick)`(`pub(crate)`。Ctrl を押したままの OFF では `CtrlImeChord` の barrier を立て、barrier 中の 2 回目の OFF をフィルタする。`platform_state.rs:625-660`・`ime_model.rs:1084-1096`)
   4. 戻り値が真なら `record_explicit_intent(.., Command, ..)`(`pub`。**授権に効く**: `issue_self_actuation_order` の warrant は IntentStore を読む)
3. `kp_run_inner` の後半(`key_pipeline.rs:250-258`)。Ctrl の KeyUp で `on_ctrl_key_up`(`pub(crate)`、`platform_state.rs:669`)を呼び、barrier を解く。
4. executor のバッチの始めに `applied_snapshot = ime.model().applied`(`executor.rs:162`・`:185`)。
5. `dispatch_ime_set_open`(`executor.rs:739-934`、gated)。
   1. D1 の applied の未知化(`explicit_press_applied_pair`。`engine_press_unknowns_applied` は実質 TsfNative の窓では偽)
   2. `build_ime_control_view`(gated。`tsf::observer` の IME 種別と `candidate_was_seen` を読む)
   3. `decide_gate`(拒否なら記録を積んで返す)
   4. `claim_press_write`
   5. `plan_set_open`(核、`state/ime_set_open_plan.rs`)
   6. 同期なら `apply_ime_open_with_view` → `ImeController::apply`(`ime_controller.rs:577-640`、gated): `decide_gate` → `into_actuation`(warrant)→ `run_chain(chain, SyncChainWriter)`
   7. `SyncChainWriter::write`(`:460-485`)が `decide_attempt` を呼び、`apply_mechanism`(I/O)を呼び、`AttemptRecord` を組む。`apply_mechanism`(`:201-`)も中でもう一度 `decide_attempt` を呼ぶ(`:218`)
   8. 何も送らなければ `release_press_write`
6. バッチの後に `dispatch_outcomes` → `on_ime_apply_complete`(`runtime/mod.rs:920-968`)。
   1. journal の `ImeOpenApplied`
   2. `post_ime_refresh`
   3. `record_ime_apply_result`
   4. 受理されれば `on_ime_applied`(GjiFsm の warm/cold)

**ungated なもの**: `decide_gate`/`decide_chain`/`decide_attempt`(`state/ime_actuation_decision.rs:128,152,346`)、`run_chain`(`state/actuation_chain.rs:549`)、`MechanismWriter`(`:613`)、`plan_set_open`、`PressLedger`、`handle_engine_set_open`・`on_ctrl_key_up`・`record_explicit_intent`(ハブの本体)。
機構を適用できるかの判定(`ime_controller.rs:70,107,140`)の中身は、`key_sequence_policy` の ungated な関数に (profile, IME 種別) を渡すだけである。

**gated なもの**:

- 5 の判断部分
- `DecisionInputs` を作る変換(`state/ime_decision_view.rs:143-153`。モジュールごと `#[cfg(windows)]`、`state/mod.rs:223`)
- **`record_ime_apply_result`**(`state/platform_state/shell.rs:151`。`shell.rs` は `#[cfg(windows)] mod shell;`、`platform_state.rs:21-22`)

ungated なのは private の `record_ime_apply_result_in_scope`(`platform_state.rs:1125`)だけである(初版の「ungated」は誤りだった。M2)。
記録系(`record_*` とその `_in_scope` 版)は `pub` にしない(INV-A97-1)。この方針は `architecture_guard.rs:6589-6597`(`PLATFORM_STATE_PUB_FNS`)と `production_hub_is_unreachable_from_outside_the_crate` が固定し、呼び出し元の件数は `RECORDERS`(`:2100`)が固定している。

同期の判断の中に `with_app`・HWND・OS の読み取りは無い。HWND の捕獲(`romaji_pre_write`)と送信は `apply_mechanism` の中だけにある(Opus round1 で独立に確認)。

```sh
sed -n 577,640p crates/awase-windows/src/ime_controller.rs        # ImeController::apply
sed -n 456,485p crates/awase-windows/src/ime_controller.rs        # SyncChainWriter::write
sed -n 1402,1450p crates/awase-windows/src/runtime/key_pipeline.rs # kp_stage_post_decision
sed -n 18,23p crates/awase-windows/src/state/platform_state.rs    # #[cfg(windows)] mod shell;
grep -n "fn record_ime_apply_result" crates/awase-windows/src/state/platform_state.rs crates/awase-windows/src/state/platform_state/shell.rs
```

### 4. 事実の訂正: BUG-141 の元の窓では、HEAD で `candidate_was_seen` は効かない

- BUG-141 の報告は msedge(`Chrome_WidgetWin_1`、`profile=Imm32Unavailable`)である。
- HEAD の `dispatch_ime_set_open` は、押下つきの書き込みで applied を未知にする(ADR-208 D1、`state/ime_actuation_decision.rs:208-219`)。
- この未知化を止めるのは「実質 TsfNative」の窓だけである(`engine_press_unknowns_applied`、`:186-188`)。`Chrome_WidgetWin_1` はその一覧に無い(`focus/class_names.rs:51-60`・`:259-265`)。

したがって、元の窓で 2 回目の Ctrl+無変換が送られることは、HEAD では D1 が保証している。
`candidate_was_seen`(`:223-229`)が効くのは、TsfNative の窓の Engine 経路と、`press=None` の書き込みである。
なお、BUG-141 の本物の desync(閉じたまま候補ウィンドウが出続ける)は、実機でも再現していない(`BUG-141.md` の状態欄)。

## 決定

### 決定 1: 範囲と完成像(方向)

再生基盤は、閉ループのハーネスを拡張して作る(第 3 のハーネスは作らない)。完成像は次のとおり。

- **入力**: 人が数打鍵に縮めた手書きのシナリオ(E7)。記録の入力は段階 5 の方向として残す。
- **前段**: B5 の補助(`KeyInput` → `RawKeyEvent`、エンジンのタイマーのループ)で、HEAD の `Engine::on_input`/`on_timeout` を通す。
- **中段**: エンジンの `SetOpen` を、本番と同じハブの関数と核の関数(決定 2)に渡す。押下台帳・applied・belief・IntentStore・Ctrl chord の barrier は、本物の `ImeStateHub` が持つ。物理 IME キーの経路は、段階 2 で同じ核に合流させる。
- **後段**: 核の writer(決定 2)が `decide_attempt` を呼び、試行を記録する。再生で差し替えるのは「決まった `MechanismCommand` を擬似 IME に当てて outcome を返す」部分だけ。
- **観測**: OS の読み取り(IME の開閉・conv・候補ウィンドウの表示)は擬似 IME(`tests/support/pseudo_ime.rs`)が与える。
- **出力**: 本番と同じ `ActuationDecisionRecord` の列と、各ステップの擬似 IME の真の状態。期待値(`attempts` の `command` の列、最終の開閉)は人が書く。

**本番と再生は、同じ核の関数を呼ぶ**。合否に使う値(`attempts[].command`)は核が作り、再生の側では作らない。
再生の側に残る写しは決定 3 に全部列挙する。写しの行数が増える段階は採らない。

### 決定 2: 第 1 段階で核へ移すもの(挙動は変えない移動)

| 移すもの | 移し先(案) | 本番の殻に残るもの |
|---|---|---|
| `SyncChainWriter::write` の判断(`ime_controller.rs:460-485`): `decide_attempt`、`AttemptRecord` の組み立て、`candidate_was_seen` の消費の判定(`:247-268` の規則) | 核の writer `CoreSyncWriter<S>`。`MechanismWriter` を実装する。中で `decide_attempt` → `sink.send(mechanism, command, open)` → 試行の記録 → 消費の判定を返す。`S` は「決まった command を送って outcome を返す」だけの小さな trait | `S` の本番の実装: 今の `apply_mechanism` から `decide_attempt` の再計算(`:218`)を外し、渡された command を送るだけにする(重複の除去)。`reset_candidate_was_seen()` は核の答えを見て殻が呼ぶ |
| `ImeController::apply` の本体(`:577-640`): gate の早期 return、`into_actuation`、`run_chain`、記録の組み立て | 核の関数 `apply_sync<S>(order, inputs, sink) -> (ImeOpenOutcome, ActuationDecisionRecord)` | `ImeController::apply`: view → `DecisionInputs` に変換して sink を作り、呼ぶだけ(約 10 行) |
| `dispatch_ime_set_open` の判断(`executor.rs:747-815` と同期の分岐 `:906-930`): D1 の未知化、gate の拒否の記録、claim、`plan_set_open`、同期の適用、`release_press_write`、`caller` の記録 | 核の関数 `dispatch_set_open<S>(hub, facts, open, press, sink)`。戻り値の variant は今の分岐と 1 対 1 に限る(`NotOwned`・`SetOpenPlan::SkipAlreadyClaimed`・`AsyncImmCross` → order を返す・`SyncChain` → outcome と記録)。`SetOpenPlan` はそのまま内側の判断に使い、新しい計画の型は作らない(ADR-229 の Plan/Effect の項の却下に近づけない) | Facts の収集(`current_app_profile`、`is_effectively_tsf_native(class_name)`、`tsf::observer` の IME 種別と `candidate_was_seen`)。`Async` のときの `spawn_local` 以降(`:823-904`)はそのまま |
| `kp_stage_post_decision` の SetOpen の部分(`key_pipeline.rs:1417-1435`): `allocate_event_generation`・`handle_engine_set_open`・`applied` が真のときの `record_explicit_intent` | ハブの 1 関数(例: `on_engine_set_open_request(open, ctrl, tick) -> (ApplyGeneration, bool)`) | `timer.kill(TIMER_IME_REFRESH)` とログ |
| `DecisionInputs` を作る変換(`ime_decision_view.rs:143-153`) | 核の関数 `DecisionInputs::from_facts(...)`。`From<&ImeControlView>` はこれを呼ぶ | — |
| `imm_cross_is_first_applicable`(`ime_controller.rs:666-670`)と適用可否 | 核(`decide_chain` と `key_sequence_policy::*_applicable` だけで書ける) | `mechanism_is_applicable` は核を呼ぶ |

- **`run_chain` は書き換えない**(ADR-229 F-D1 の例外 1)。核へ移すのは `MechanismWriter` の実装の 1 つ(`CoreSyncWriter`)で、走査規則と型状態(ADR-090)はそのまま。汎用の Effect/Handler 基盤でもない(差し替え口は `S` の 1 つ)。
- `open_chain.rs::fallback_write`(非同期の ImmCross 以降)も `apply_mechanism` を呼ぶ。段階 1 では、`fallback_write` の側で `decide_attempt` を呼んで command を渡す 1 行を足し、非同期の構造には触れない(ADR-180 の INV-45 を守る)。
- **同期の合流点の宣言が変わる**(所有者への質問 1)。executor が `apply_ime_open_with_view` を呼ばなくなるので、次が変わる。
  - `architecture_guard.rs:2027` の `(".apply_ime_open_with_view(", 2)`
  - `lints/actuation_call_guard/src/lib.rs` の `RESTRICTED_CALLS` の該当行
  - `fix-requires-evidence.md` の「IME actuation 合流点」行(現存 5 エントリ)
  - ADR-119 の「`ImeController::apply` が同期経路の唯一の合流点」
  - `raw_mechanism_write_sites_are_confined_to_chain_writers`(`architecture_guard.rs:3204-3211`)の前提

### 決定 3: 再生の側(テスト)の第 1 段階

1. B5 の補助(`src/key_input_replay_tests.rs` の補助の部分、389 行)を、ハーネスと同じ場所へ移す(場所は所有者への質問 3)。
   BUG-105 のテスト 2 本と fixture は一緒に移し、扱いは段階 5 で決める。
2. ハーネスに `press(RawKeyEvent)` を足す。本番の `kp_run_inner` の順に、次を呼ぶ。
   1. `PhysicalKeyDisposition::plan_core`。キーが擬似 IME に届くか(Allow/Suppress)を決める
   2. `Engine::on_input`
   3. `SetOpen` なら、ハブの `on_engine_set_open_request`(決定 2)
   4. `dispatch_set_open`(sink は擬似 IME)
   5. 完了の記録(場所の制約は所有者への質問 3)
   6. Ctrl の KeyUp なら `on_ctrl_key_up`
   予測器の写し(`kp_stage_key_effect_track`/`kp_predict_key_effect`、既存)をどこで呼ぶかは、本番の順を確かめて同じ PR の doc に書く。
3. 擬似 IME に、`MechanismCommand`(`SendVk(VK_IME_ON/OFF)`・`SetOpenCrossProcessSync`)を当てる sink を足す。クセとして、候補ウィンドウの表示(`candidate_was_seen` を立てる)と、外部からの開閉(観測できない)を足す。
4. `user_set_open`(既存のシナリオで 6 か所。閉じる 5・開く 1)を、既定の `keys.ime_off`/`ime_on`(Ctrl+無変換/Ctrl+変換、`src/config.rs:598-599`)の打鍵に置き換える。あわせて次を消す。
   - `issue_write` の `WriteOrigin::ExplicitUserCommand` の分岐(`issue_actuation_order` の写し)
   - `handle_engine_decision` の panic
   既存の 20 本の結果が変わったら、ADR-224 の規則(写しのずれの実例として記録して止める)に従う。所有者への質問 2。

**再生の側に残る写し(第 1 段階の後、全部)**:

| 写し | 本番の場所 | 扱い |
|---|---|---|
| 1〜6 の呼び出しの順序 | `kp_run_inner`・`kp_stage_post_decision` | ハーネスの doc に列挙する |
| バッチの始めに applied を写す | `executor.rs:162`・`:185` | 同上 |
| 完了をバッチの後に返す | `runtime/mod.rs:971` | 同上 |
| 完了の側の journal・`post_ime_refresh`・`on_ime_applied`(GjiFsm の warm/cold) | `runtime/mod.rs:929-967` | **写さない**(段階 1 のシナリオは GjiFsm の状態に依存しない)。依存するシナリオが要るときに段階 4 で扱う |
| `timer.kill(TIMER_IME_REFRESH)` | `key_pipeline.rs:1416` | 写さない(ハーネスに OS のタイマーは無い) |
| 候補ウィンドウのラッチのフォーカス移動での解除 | `runtime/focus_tracking.rs:593`・`platform.rs:1144` | 写さない(リスク節) |
| 予測器(既存の写し) | `kp_stage_key_effect_track`/`kp_predict_key_effect` | そのまま(段階 4 で外す) |

消える写し: `issue_write` の明示の分岐。drift の分岐は段階 4 まで残る。

### 決定 4: 第 1 段階の対象(2 つのシナリオ)と合否

| シナリオ | 窓・IME | 打鍵と観測 | 期待値(人が書く。観測できる出力だけ) |
|---|---|---|---|
| A: ADR-208 D1 の固定(BUG-141 の元の窓、BUG-149/150 型) | `Imm32Unavailable`・`Chrome_WidgetWin_1`・GJI(擬似 IME の ATOK 格子)、IME ON | Ctrl↓ 無変換↓↑ Ctrl↑ → 擬似 IME が外部から開く(Chrome では観測できない)→ Ctrl↓ 無変換↓↑ Ctrl↑ | 2 回とも `attempts[0].command == SendVk(VK_IME_OFF)`、最後に擬似 IME の真の状態が閉 |
| B: BUG-141(TsfNative 窓) | `TsfNative`・GJI、IME ON | Ctrl+無変換 → 候補ウィンドウの表示(クセ)→ Ctrl+無変換 → Ctrl+無変換(各回 Ctrl↑ を挟む) | 2 回目は `SendVk(VK_IME_OFF)`、3 回目は `command: None`(ADR-171 の「1 回の証拠につき再送は 1 回」) |

**合否の判定**: 判定基準は最初の CI の run の前に PR 本文に書く。

1. **対照を同じ PR で書く**。決定 2 で移した核の関数(`CoreSyncWriter`・`apply_sync`・`dispatch_set_open`・`on_engine_set_open_request`・`DecisionInputs::from_facts`)のそれぞれに、FakeWriter・sink を渡して直接呼ぶ単体テストを書く(1 本 30 行以内)。
   例: `dispatch_set_open` を 2 回呼び、2 回目の `attempts[0].command` を見る。
2. **変異を数える**。経路上のファイルに `cargo mutants` を当てる(Linux、ADR-234 の core の mutants と同じ仕組み)。
   - 対象: 移した核の関数、`state/ime_actuation_decision.rs`、`state/platform_state.rs` の `handle_engine_set_open`/`on_ctrl_key_up`/`on_engine_set_open_request`、エンジンが `SetOpen` に press を載せる箇所(`src/engine/engine.rs:1113` の `stamp_set_open_press`)、`src/types.rs::is_press_start`
   - 「対照の単体テスト+既存テスト(`--lib`・`explicit_press_exhaustive`・`closed_loop_scenarios`)」で生き残った変異を一覧にする。
3. **価値の証明**: 生き残った変異のうち、**打鍵からの再生(シナリオ A・B)でだけ落ちるものが 1 つ以上**あること。
   - 再生の判定は観測できる出力(`attempts[].command` の列と擬似 IME の真の状態)で行う。mutator が書き換えた値そのもの(例: `unknowns_applied` の値や `candidate_was_seen` のフラグ)を判別に使わない(ADR-240 の a9 の教訓)。
   - 候補の仮説は次のとおりで、どれも未確認。
     - (M-D) 2 回目の押下に press を載せない。D1 は `press.is_some()` のときだけ効くので、A が落ちるはず。ただし `src/engine/tests.rs:8877-8936` がエンジンの press の付与を検査しているので、既存テストで落ちる見込みが高い
     - (M-E) chord の barrier を Ctrl↑ で解かない(`on_ctrl_key_up` の解除条件)
     - (M-F) `on_engine_set_open_request` が `record_explicit_intent` を呼ぶ条件を反転する。授権が下りず `Unwarranted` になる
   - 初版の M-A〜M-C(移した関数の中の書き換え)は、対照の単体テストで落ちるので価値の証明には使わない。
4. **取りやめ条件 (a)**: 3 を満たす変異が 0 個なら、ハーネスへの配線(決定 3 の 2〜4)をやめる。核への移動(決定 2)と対照の単体テストは残す。これだけで、FCIS の F 系の作業として D1(シナリオ A の判断)と BUG-141(B の判断)を Linux で固定できる。
5. CI だけで判定する(ローカルでは走らせない)。mutants の run ID と、生き残った変異の一覧を PR 本文に残す。

ADR-240 との関係: ADR-240 は実機 CI のシナリオを回帰の証拠と数える規約で、閉ループ・単体テストは対象外としている(`240-…:127`)。
本 ADR の 3 は ADR-240 の範囲の拡張ではない。段階 1 の取りやめを決めるための、本 ADR だけの判定である。役割分担は決定 7。
ADR-240 が develop に入る前でも判定できる(依存しない)。

### 決定 5: 段階と収支

撤去は決定済みの事項である: 既存の actuation の再生一式は、新しい基盤への置き換えと同時に捨てる。

- `actuation_decision_record.rs` の `mod tests` のうち再生と往復の部分(約 730 行、`#[test]` 17 本のうち 15 本)
- 残す 2 本: `event_source_kind_discards_payload_but_keeps_variant` と `actuation_decision_record_json_byte_size_is_measured`
- テスト専用の `Deserialize` 一式(約 70 行)
- 凍結コーパス 1440 行
- `docs/journal-replay-guide.md:15-72` の節

これは案 B″ でも同じ量なので、**下の収支は B″ を 0 とした差分**で書く。

| 段階 | 内容 | B″ に対する追加(未実測の見積もり) | B″ に対する撤去 | 検証(CI) | 取りやめ条件 |
|---|---|---|---|---|---|
| **1(本 ADR で決める)** | 決定 2〜4 | 本番: 移動が主で純増 +40〜80 行。`apply_mechanism` の `decide_attempt` の重複は消える。対照の単体テスト +100 前後。ハーネスと擬似 IME +170 前後、シナリオ 2 本 +80 前後。ガードと lint の宣言の更新 | ハーネスの `issue_write` の明示の分岐と `handle_engine_decision`(約 25 行) | 決定 4。windows-cross-check と windows-build が green(殻の移動を見る) | (a) 決定 4 の 4。(b) 移動で挙動が変わる(windows-build の既存テストか、実機 CI の sc-dbe・sc-kanji の結果が変わる)。(c) テスト側の追加が 500 行を超える |
| 2 | 物理 IME キーの経路(`kp_stage_shadow_ime_toggle` `key_pipeline.rs:899-1177`・`kp_shadow_actuate` `:1237-1399`)の判断を核へ移す。`dispatch_set_open` と同じ claim → applied → imm_first の骨格を共有する | 本番の移動 +150〜250、テスト +150 前後 | 2 入口の重複の骨格。`architecture_guard.rs:6520` の 2 入口ごとの件数ガード。`explicit_press.rs` の合成のモデルのうち `attempt_write`(`:775-817`)と `explicit_press_delivery_after`(`:818-1039`)を核の呼び出しに置き換えられれば 0〜約 265 行(未確認。全列挙に要る型 `PressState`・`Delivery` は残る) | BUG-113(同じ押下で 2 経路、P5)、BUG-156、BUG-159 を、決定 4 と同じ方式で | 物理キーの経路に `tsf::observer` の読み取りが混ざり、Facts にまとめられない |
| 3 | エンジンの組み立ての共有(`app/bootstrap.rs:279`・`:1174-` から、`keys.*`・`SpecialKeyCombos`・親指・単独タップの設定を純粋な関数へ。`runtime::thumb_forced_open_actions`・`migrate_legacy_solo_tap_actions` を ungated な場所へ) | +100 前後 | B5 の `build_engine` の部分的な写し(約 40 行。`set_thumb_shift_faces_enabled` の抜けがある、b5-prototype-result.md 詰まった点 5)とハーネスのエンジンの組み立て | 既存の B5 と閉ループのテストが同じ結果 | `runtime/` の依存の整理が移動で済まない |
| 4 | drift correction の actuation(`ir_apply_drift_correction` の適用を `apply_sync` に通す)と、必要なら非同期(ImmCross が先頭の窓)。完了の届く位置は既定で「その入力の処理の直後」、シナリオに明示した位置でも届けられる | +200〜300 | ハーネスの残りの写し(`issue_write` の drift の分岐、`align_placeholder_desired` の写し、`ir_apply_drift_correction` の前半、予測器の写し)約 100〜150 行 | BUG-157、BUG-163 を、決定 4 と同じ方式で | (i) ADR-180 の 3 関数の独立した再検出(INV-45)を崩す必要が出る。(ii) `fallback_write` から `with_app` を内包するヘルパーを呼ぶ必要が出る(再入でゲートが無効になる、issue #136/BUG-90 型) |
| 5(方向のみ) | 記録の入力: 報告の journal の `KeyInput`・`ImeEvent`・`TimerFired`・`ImeOpenApplied`。タイマーと完了は記録の seq の位置で発火する(B5 の方式) | `ImeEvent` の読み込み(今は `Serialize` のみ、`journal.rs:282-295`)、`TimerFired` のレーンの変更、+200〜300 | B5 の BUG-105 の重複テスト | 報告由来の数打鍵の再現で修正前後の差 | 段階 2 の結果を見て改めて決める |

**全体の収支(B″ を 0 として)**:

- 追加: 段階 1〜4 で約 +800〜1100 行(本番の移動を含む)。段階 5 を含めると +1000〜1400 行。
- 撤去: ハーネスの写し約 125〜175 行、2 入口の重複、`explicit_press.rs` の 0〜約 265 行(未確認)。
- **純増は約 +500〜1000 行**(段階 1〜4)。

純増の根拠は、各段階の「検証」の列の BUG で、決定 4 の 3(単体テストでは落ちず、打鍵からの再生でだけ落ちる変異)を示せることに限る。示せなかった段階は取りやめる。
段階 2 の骨格の共有は、ADR-180 決定 2 が見送った「3 実装の記録の組み立ての統一」とは違う。今回共有するのは判断の骨格(claim → applied の未知化 → imm_first)で、記録の組み立ては決定 2 で既に核の `CoreSyncWriter` と `apply_sync` に 1 本化される。

### 決定 6: 時刻の持ち方

- **エンジンのタイマー**: B5 の 2 方式を残す。既定は仮想時計で、期限どおりに発火する。記録の入力(段階 5)では、`TimerFired` の seq の位置で発火させる。
  遅れの無い仮想時計は OS のタイマーより必ず早く発火する(B5 の結果)。そのため、手書きのシナリオで競争を扱うときは「ここでタイマーが発火する」をシナリオに明示する。
- **ハブの時計**: ハーネスの `HubClock::manual` を、エンジンと同じ仮想時刻で進める(閉ループで実施済み)。
- **非同期の完了**: 段階 4 まで扱わない(段階 1 のシナリオは同期の経路だけを通る)。
- **遅延のモデル**(実機の遅延を確率や分布で写すもの): 作らない(ADR-226 の候補 D と同じ理由)。

### 決定 7: ADR-240(ablation)との役割分担(team-lead の決定、2026-10-06)

| | ADR-241(本 ADR) | ADR-240 |
|---|---|---|
| 受け持つ不具合 | 同期の判断(打鍵から actuation の決定まで)。Linux の再生で受ける | 再生が通らない部分(非同期・実機固有の IME とのやり取り)。実機 CI の再現シナリオで受ける |
| 修正を外す書き換え | 再生の合否用の mutator(決定 4)。置き場所は再生のテストの側(段階 1 の PR で決める。実機 CI の `tools/e2e/ime_key_matrix/ablations/` とは混ぜない) | 実機 CI 側の ablation(`ablations/bug<NNN>-*.sh`)。D1a の条件(前提と症状の分離・observed 件数・observed 0 は INVALID)で判定する |
| 資産の扱い | 両方を残す。片方を他方に寄せない | 同左 |

**`.claude/rules/fix-requires-evidence.md:22-26` の (b) の「将来、再生トレースの追加に置き換える予定」の節は、本 ADR だけが書き換える**(新しい再生基盤を導入するのは本 ADR だから)。ADR-240 の側では書き換えない。
書き換えは段階 1 の PR 群で行い、次の 2 項目にする(文案)。

- 同期の判断(打鍵から actuation の決定まで)に触れる fix は、(a) の一つとして、ADR-241 の再生のシナリオ(閉ループのハーネス)を足してよい。期待値は人が書く。
- IME とのやり取りが絡む不具合で、再生が通らない部分(非同期・実機固有)は、実機 CI の再現シナリオで受ける。ADR-240 の D1a の条件(前提と症状の分離・observed 件数・observed 0 は INVALID)を満たしたものだけを (a) と数える。

ADR-159・ADR-162 E1/E4 の能力ベースの前提条件への参照は、この書き換えで外す(TH1e は ADR-163 で取り下げる。追記案参照)。

## 却下した案

- **汎用の Effect/Handler 基盤、Plan/Effect の項**: ADR-229(`229-…:212`、`docs/adr/review/229-effect-plan-design-draft-r1-rejected.md`)で却下済み。本 ADR の差し替え口は「command を送る」の 1 つだけで、`dispatch_set_open` の戻り値は今の分岐と 1 対 1 に限る。
- **DSL・宣言テーブル・TOML のシナリオランナー**: 次の判断と同じ理由で採らない。シナリオは今の閉ループと同じ Rust のビルダーで書く。
  - ADR-219(エンジンテストの DSL は見送り)
  - ADR-218・220
  - 閉ループの TOML ランナーと Rhai の見送り(2026-10-04)
- **全記録の自動 fixture 化**: ADR-225 の P2(バグの出力を固定し、修正のたびに落ちる)。期待値は人が書く(E7)。
- **閉ループの写しを増やして殻の経路を真似る**: `explicit_press.rs` が 1 押下ぶんでこれをしており、本番から呼ばれない合成のモデルが約 1000 行ある。写しは本番が変わっても落ちない。本 ADR は判断を核へ移してから呼び、合否に使う値を写しの側で作らない。
- **再生用の writer に `decide_attempt` と試行の記録を持たせる**(初版の決定 2): 合否に使う `attempts[].command` を写しが作ることになるので採らない(round1 M3)。
- **journal からの完全再生(B2)**: ADR-232 で却下済み(全書き込みの reduce 化が前提)。
- **再生基盤を作らず、撤去だけをする(案 B″)**: 所有者が案 C を選んだので採らない。ただし、段階 1 の取りやめ条件 (a) のときは B″ に近い形(移動と単体テストは残す)になる。

## リスクと限界

- **同期の合流点の宣言の作り直し**(決定 2 の最後の箇条)。挙動を変えない移動でも、ADR-119/158/159 の宣言、ガード、lint の宣言が変わる。B6 規模の本番の変更なので、所有者の承認を要する(質問 1)。
- **殻で Facts を集める 1 行は検査されない**。次の行は gated のまま残る。
  - `tsf::observer::candidate_was_seen()` を Facts に入れる行
  - `is_effectively_tsf_native(class_name)` を渡す行
  BUG-141 の根本原因(変換が値を写していなかった)と同じ種類の漏れが殻の側で起きても、再生は落ちない。核の変換の漏れは、対照の単体テストが落とす。
- **IME の応答は擬似 IME が写した範囲だけ**。KA+IME の約 25 件のうち、クセの目録に無い挙動は再現できない。
- **写し**(決定 3 の表)は、本番の順序が変わっても追随しない。
- **候補ウィンドウのラッチ**は、表示(擬似 IME)と送信時の消費(核)だけを扱う。フォーカス移動での解除は扱わない。
- **ガードが壊れる**。`architecture_guard.rs` の `:2027`・`:3204-3211`・`:6520-` と、`PLATFORM_STATE_PUB_FNS`(`:6597`)。本数は段階 1 の PR で列挙する。`:2831-2910` は `Actuation::request(` の件数なので、変わらない見込み。
- **未確認の点**:
  - 行数の見積もりはすべて未実測。実測は B5 の補助 389 行・ハーネス 713 行・擬似 IME 509 行・`dispatch_ime_set_open` 195 行・`kp_shadow_actuate` 163 行・`kp_stage_shadow_ime_toggle` 279 行・`explicit_press.rs` 2056 行だけ。
  - KA の 48 件と、並べ直した 14 件の確度は中以下(付録)。
  - 決定 4 の 3 を満たす変異が実際にあるかは、CI で mutants を回すまで分からない。
  - `actuation_call_guard`(dylint)が移動で反応するかは未確認。
  - 番号の衝突: develop には既に 238・239 が 2 本ずつある。マージ時に 240・241 の衝突を確かめる。

## 所有者に聞くこと

1. **同期の合流点を核(`apply_sync`・`CoreSyncWriter`)へ移し、宣言を更新してよいか**
   - 更新するもの: ADR-119 の宣言・`fix-requires-evidence.md` の合流点の行・`RESTRICTED_CALLS`・`architecture_guard` の件数ガード
   - 選択肢: 承認する/承認しない(段階 1 は行わず、B″ だけ行う)
   - 推奨は承認する。理由は 2 つ。
     - 判断の中に `with_app`・HWND・OS の読み取りは無く、ADR-180 が見送った 3 案の繰り返しにならない(Opus round1 で確認)。
     - `apply_mechanism` の `decide_attempt` の重複が消える。
2. **閉ループの `user_set_open` を、段階 1 で本物の経路(打鍵)に置き換えるか**
   - 選択肢: 置き換える(6 か所)/新しいシナリオだけで使う
   - 推奨は置き換える。写しが 1 系統減る。既存の 20 本の結果が変われば写しのずれの実例になり、ADR-224 の規則で止まる。
3. **ハーネスの置き場所と、記録系の扱い(INV-A97-1)**
   - 前提: 完了の記録(`record_ime_apply_result`)は gated で、記録系は `pub` にしない方針。`handle_engine_set_open`・`on_ctrl_key_up` も `pub(crate)` である。
   - 選択肢:
     - (a) 閉ループのハーネスとシナリオを crate の中の `#[cfg(test)]` モジュールへ移す(B5 と同じ置き方)。`pub(crate)` のまま呼べるので、`production_hub_is_unreachable_from_outside_the_crate` の前提は変わらない。P5 でハーネスのために `pub` にした 13 個(`PLATFORM_STATE_PUB_FNS`)は `pub(crate)` に戻せる。完了の記録には `_in_scope` 版を `pub(crate)` にして呼ぶ。
     - (b) `tests/` に残し、scope を引数に取る `pub` の完了の入口を足す。INV-A97-1 と P5a-1 の「外から届く口が無いから安全」という前提に触れる。
   - 推奨は (a)。公開面が増えず、むしろ減る。
   - 費用: ファイルの移動(約 2000 行、純増なし)、`ci.yml:51` の `--test closed_loop_scenarios` と `ci_test_coverage_guard` の更新。`architecture_guard` の `harness.rs` を指す走査の付け替えは未確認。
4. **決定 4 の 3 を満たす変異が無かったとき、ハーネスへの配線をやめてよいか**(移動と単体テストは残す)
   - 選択肢: やめる/配線も残す
   - 推奨はやめる。打鍵から通すことの価値が示せない行数は、撤去を主目的とする方針に合わない。

(初版の「撤去を同時に行うか」は決定済みなので質問から外した。段階 5 は段階 2 の結果を見て改めて聞く。)

## 関連する既存文書への追記案(段階 1 の PR 群で行う。本 ADR の起草では書き換えない)

- `docs/adr/index.md`: 本 ADR の 1 行(起草の段階では team-lead がまとめて足す)。
- `docs/tasks/journal-replay-rebuild-study-2026-10-06/b5-prototype-result.md` の「新基盤の範囲の選択肢」の案 C の行: 「所有者が選んだ(2026-10-06)。設計は ADR-241」。
- `docs/tasks/corpus-discard-impact-2026-10-06/README.md` の Q1: 「撤去は ADR-241 の段階 1 と同時、範囲は (B)」。
- ADR-163 の status の先頭: 「TH1e は取り下げ(ADR-241 の段階 1 で再生一式を撤去)」(corpus-discard-impact の Q5 の回答どおり)。
- `.claude/rules/fix-requires-evidence.md:22-26`: 決定 7 の文案に書き換える(本 ADR だけが行う。ADR-240 の第 1 段階からは、この文の書き換えを外す)。`:64-70` の「ジャーナルリプレイ基盤」の行は、段階 1 の後に閉ループのハーネスの場所を指すよう直す。
- `fix-requires-evidence.md` の「IME actuation 合流点」行と ADR-119: 質問 1 の承認後、合流点の内訳を `apply_sync`・`CoreSyncWriter` に直す。
- ADR-224 の status: 「ハーネスは ADR-241 の再生基盤の本体になる。写しの一覧は `harness.rs` の doc」。
- ADR-229: 次のとおり追記する(`:212`・`:282` は corpus-discard-impact §1.4 のとおり)。
  - `:137`(F-D1 の例外 1 の理由): RW の撤去後は、型状態(ADR-090)・FakeWriter・核の `CoreSyncWriter` が例外を支える。
  - `:212`
  - `:282`
- `docs/tasks/fcis-layering-tasks-2026-10-06.md`: 段階 1 の移動を F の行として足す。`CORE_MODULES` と mutants の `examine_globs` への追加(V1)を同じ PR で行う。
- `tests/closed_loop_scenarios.rs:12-23` の「host から見える層」の表: executor と `ImeController::apply` の判断を「核を呼ぶ」に直す。
- `docs/known-bugs/BUG-141.md`: HEAD の元の窓では D1 が送信を保証していること(背景 4)と、シナリオ B の名前。シナリオ A は D1 の固定として ADR-208 側に書く。

## 付録: KA 48 件の並べ直し(一次分類の要約に基づく。未確認)

- **機構が撤去済み(約 15)**: 032・118・119・122・123・124・125・131・133・135・136・137・143・174・175
- **1 回の判断の単体テスト・golden で足りる(約 10)**: 010・050・052・097・116・146・152・153・158・182
- **hook・OS より下(約 5)**: 062・067・090・154・181
- **未再現・実害記録なし(約 4)**: 098・128・184・187
- **複数押下の状態を持ち越し、hook より上(約 14、この基盤の対象)**: 014・037・046・110・113・117・121・140・141・142・156・157・159・173

## 再確認のコマンド(版 `4f59292a`)

```sh
git ls-tree --name-only 4f59292a docs/known-bugs/ | grep -c 'BUG-'                       # 184
git show 4f59292a:crates/awase-windows/tests/support/harness.rs | sed -n 13,27p           # 写しの一覧と on_input を呼ばないこと
git show 4f59292a:crates/awase-windows/tests/support/harness.rs | sed -n 612,626p         # SetOpen で panic
git show 4f59292a:crates/awase-windows/src/runtime/executor.rs | sed -n 739,934p          # dispatch_ime_set_open
git show 4f59292a:crates/awase-windows/src/runtime/key_pipeline.rs | sed -n 1402,1450p    # kp_stage_post_decision
git show 4f59292a:crates/awase-windows/src/state/platform_state.rs | sed -n 18,23p        # #[cfg(windows)] mod shell;
git show 4f59292a:crates/awase-windows/src/state/platform_state/shell.rs | grep -n "fn record_ime_apply_result"
git show 4f59292a:crates/awase-windows/src/state/ime_actuation_decision.rs | sed -n 186,188p   # D1 を止めるのは TsfNative だけ
git show 4f59292a:crates/awase-windows/src/focus/class_names.rs | sed -n 51,60p           # Chrome_WidgetWin_1 は無い
git show 4f59292a:crates/awase-windows/src/state/actuation_decision_record.rs | grep -c "#\[test\]"   # 17
git show 4f59292a:crates/awase-windows/tests/closed_loop_scenarios.rs | grep -c "\.user_set_open("  # 6
git show 4f59292a:crates/awase-windows/tests/architecture_guard.rs | sed -n 6597,6611p    # PLATFORM_STATE_PUB_FNS(13 個)
```
