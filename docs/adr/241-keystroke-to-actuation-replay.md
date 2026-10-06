---
id: ADR-241
title: |-
  打鍵から actuation の決定まで通す再生基盤(第 1 段階: エンジンの SetOpen から同期の機構チェーンまで)
summary: |-
  所有者は 2026-10-06 に、新しい再生基盤の範囲として案 C「打鍵から actuation まで通す」を選んだ(B6 と同規模、1000 行超)。
  既存の再生一式(`replay_record`・RW・凍結コーパス・`Deserialize` 一式)は、この基盤への置き換えと同時に捨てる。
  背景の実測: キー列とシェル状態から IME 制御の送信が決まる BUG(KA)は 184 件中 48 件(サブエージェントの一次分類、確度は中)。
  そのうち、キー列を key_pipeline・executor まで通す回帰テストを持つものは、実機 CI を除くと 0 件。
  閉ループは `Engine::on_input` を呼ばず、書き込みを bool で扱い、`MechanismCommand`(送信の種類)を作らない。
  B5 の試作はエンジンの `SetOpen` までしか再生できない。凍結コーパスは単体テストを超える検査をしていない。
  決定(第 1 段階): 同期経路の判断(`ImeController::apply` の本体と `dispatch_ime_set_open` の gate→claim→plan→chain)を
  ungated な核の関数へ移す。本番と閉ループのハーネスが同じ関数を呼ぶ。I/O は既存の `MechanismWriter` の実装で差し替える。
  B5 の補助は `tests/support` へ移し、ハーネスの前段にする。出力は本番と同じ `ActuationDecisionRecord` の列で、期待値は人が書く。
  対象は BUG-141 の 2 つの形: 元の窓(msedge・GJI。HEAD では ADR-208 D1 が送信を保証する)と、TsfNative×GJI(`candidate_was_seen` が効く)。
  同じ段階で既存の actuation の再生一式(コード約 800 行・データ 1440 行・テスト 15 本)を撤去する。
  合否は、修正を外した mutator(ADR-240)で再生が落ち、かつ既存の単体テスト・全列挙テストが落ちないことで示す。
status: |-
  提案(起草中、Opus レビュー前)
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
  - "ADR-089"
  - "ADR-219"
---

# ADR-241: 打鍵から actuation の決定まで通す再生基盤(第 1 段階)

調べた版は develop `4f59292a`(#501 マージ後。B5 の試作 #533 を含む)。ローカルでのビルド・テストはしていない。
行番号は、特に断らない限りこの版の `crates/awase-windows/` からの相対パス。

## 用語

- **actuation**: awase が IME の開閉を変えるために送るもの(`VK_IME_ON`/`VK_IME_OFF` の SendInput、`ImmSetOpenStatus` 相当など)。
  送信の種類は `MechanismCommand`(`state/ime_actuation_decision.rs:109`)で表す。
- **KA**: キー列とシェルの状態(押下台帳・applied・belief・ラッチ)から actuation の判断が決まり、症状がその送信に出る BUG。
- **核 / 殻**: ADR-229 の FCIS の語。核は Linux でも動く純粋な判断(`state/`)、殻は Win32 を呼ぶ側(`runtime/`・`ime_controller.rs` などで `#[cfg(windows)]`)。
- **写し**: 閉ループのハーネスが、殻の配線を手で書き直した部分(`tests/support/harness.rs:13-20`)。本番が変わっても追随しない。
- **mutator / nofix**: 修正だけを外す書き換え(ADR-240、`docs/adr-233-ablation-ab` ブランチで ADR-240 に改番中)。

## 背景(実測)

### 1. 打鍵から actuation まで通さないと再現できない BUG の数

known-bugs 184 件(`ls docs/known-bugs/BUG-*.md | wc -l`。BUG-046 は欠番で、本文は `BUG-045.md:64-67` にある)を分類した。
タイトルを全件読み、KA の候補約 75 件は本文の状態欄・症状・テスト欄を読んだ(サブエージェントによる一次分類。全文を精読したのは約 15 件)。

| 分類 | 件数 | 確度 |
|---|---:|---|
| KA(うち実 IME の応答も絡む KA+IME は約 25) | 48 | 中(本文を読んで判定) |
| エンジン単体で再現(BUG-105・145 型。B5 の範囲) | 約 11 | 一部未確認 |
| IME 側(cold-start・warmup・literal 検出・Chrome/GJI 固有) | 約 80 | 未確認(タイトルで割り振り) |
| その他(設定 GUI・インストーラ・診断ほか) | 約 40 | 未確認 |

KA の例(検証の手段):

- BUG-113: 物理の半角/全角を 1 回押すと「@」が出る。shadow toggle と Engine の `SetOpen` が二重に送っていた。押下 ID の予約(ADR-208 L1)で修正。検証は単体テストと全列挙テストの P5、実機 CI の wt-probe(0/40)。
- BUG-141: Ctrl+無変換の 2 回目・3 回目が `AlreadyMatched` になり、何も送られない(fix `040536bf`、ADR-171)。検証は `gji_direct_*` の単体テストだけ(`BUG-141.md` の frontmatter と本文)。
- BUG-156: 予測が belief だけを動かし、applied が古いまま残る。そのため `VK_IME_OFF` が省かれる。検証は単体テストと CI の cal-verify-blind。
- BUG-181: 2 回目の F2 で押下 ID が `None` になり、開けない。検証は `vk.rs` の host テストだけで、実機は未検証。
- BUG-152: ImmCross がタイムアウトした後、冪等でない KanjiToggle が二重に効く。検証は CI の sc-dbe と sc-kanji-msime-native(3/3)。

**キー列を key_pipeline・executor まで通す回帰テストを持つ KA は、実機 CI を除くと 0 件**。今ある検証は次のどれか。

- 純粋な判断関数の単体テスト(例: BUG-141 は `gji_direct_already_matches` の入力を直接組む)
- 1 押下ぶんの判断の合成(`state/explicit_press.rs` と `tests/explicit_press_exhaustive.rs`)
- 実機 CI

ADR-240 の実測では、状態欄に「CI 検証済み」とある BUG は 19 件ある。そのうち修正を外したビルドで差を確かめたのは BUG-170 の 1 件だけ。

### 2. 今の 3 つの手段が KA に届かない理由

| 手段 | 届く範囲 | 届かない理由(出典) |
|---|---|---|
| 閉ループ(`tests/closed_loop_scenarios.rs`、20 本) | 本物の `ImeStateHub`(P5a-1、#518)、予測器、`check_drift_correction`、Engine の活性遷移 | `Engine::on_input` を呼ばない。文字キーは擬似 IME へ直接渡す(`harness.rs:25-27`)。Engine が `SetOpen` を出すと panic する(`harness.rs:612-626`)。書き込みは「warrant が下りたか」と bool の `write_open` だけ(`issue_write`、`harness.rs:592-610`、`issue_actuation_order` の写し)。gate・押下台帳・機構チェーン・`MechanismCommand` を通らない |
| B5(`src/key_input_replay_tests.rs`、#533) | `KeyInput` の列 → HEAD の `Engine::on_input`/`on_timeout`。BUG-105 を再現 | エンジンの `SetOpen { open, press }` で止まる。送るか・何を送るかは殻(`runtime/executor.rs::dispatch_ime_set_open`、`:739-934`)と `ImeStateHub` が決める(`docs/tasks/journal-replay-rebuild-study-2026-10-06/b5-prototype-result.md` の比較表) |
| 既存の actuation の再生(`state/actuation_decision_record.rs` の `mod tests` `:409-1288`、`#[test]` 17 本。凍結コーパス 1440 行・37 件) | 記録した `DecisionInputs` から `decide_*` を再計算する。同期の走査を再走査する | 打鍵や belief の経緯は入力に無い。37 件は全部 `Sync`・`GjiDirect` の 1 回だけで、`gji_direct_*` の単体テストを超える検査をしていない。走査規則は FakeWriter のテスト(`state/actuation_chain.rs:779`・`:889`)が既に固定している(`docs/tasks/corpus-discard-impact-2026-10-06/README.md` §0) |
| 実機 CI(`e2e-ime.yml` の `sc-*`) | 本物の殻と IME | 1 回の判定に実機ジョブが要る。フォーカスの非決定性がある。修正を外したビルドとの比較はほぼしていない(ADR-240) |

**1 押下の合成(`explicit_press.rs`、2056 行)は、殻の順序の写しである**。`explicit_press_delivery_with` の doc(`state/explicit_press.rs:20-24`)は、「本番の `kp_run_inner` と同じ順序で再現する」「L0 では本番側は本関数を呼ばない」と書いている。
本番からの呼び出しは 0 件である。

```sh
git grep -n "explicit_press_delivery_with" crates/awase-windows/src | grep -v state/explicit_press.rs   # key_pipeline.rs:966 のコメントだけ
```

部品(`plan_core`・`decide_*`・`PressLedger`)は本物だが、それらをつなぐ順序と状態の受け渡しはこのモジュールが自分で持っている。

### 3. 殻の配線を Linux から呼べない箇所(第 1 段階が対象にする部分)

エンジンの明示 `SetOpen` が actuation になるまでの経路:

1. `kp_run_inner` → `engine.on_input`(`runtime/key_pipeline.rs:204`)
2. `kp_stage_post_decision` → `ImeStateHub::handle_engine_set_open`(`key_pipeline.rs:1422`。ハブは本物で、Linux で動く)
3. executor のバッチの始め: `applied_snapshot = ime.model().applied`(`executor.rs:162`・`:185`)
4. `dispatch_ime_set_open`(`executor.rs:739-934`、gated):
   1. `explicit_press_applied_pair`(D1。`engine_press_unknowns_applied` は TsfNative の窓では無効)
   2. `build_ime_control_view`(gated。`tsf::observer` の IME 種別と `candidate_was_seen` を読む)
   3. `decide_gate`
   4. `claim_press_write`(ハブ)
   5. `plan_set_open`(核、`state/ime_set_open_plan.rs`)
   6. 同期なら `apply_ime_open_with_view` → `ImeController::apply`(`ime_controller.rs:577-640`、gated): `decide_gate` → `into_actuation`(warrant)→ `run_chain(chain, SyncChainWriter)` → `decide_attempt` → `apply_mechanism`(I/O)
   7. 何も送らなければ `release_press_write`
5. バッチの後: `dispatch_outcomes` → `on_ime_apply_complete` → `ImeStateHub::record_ime_apply_result`(`runtime/mod.rs:920-968`)

このうち gated なのは 4 の判断部分と、`DecisionInputs` を作る変換(`state/ime_decision_view.rs:143-153`)だけである。
`decide_gate`/`decide_chain`/`decide_attempt`(`state/ime_actuation_decision.rs:128,152,346`)、`run_chain`(`state/actuation_chain.rs:549`)、`MechanismWriter`(`:613`)、`plan_set_open`、押下台帳、`record_ime_apply_result` は、すでに ungated である。
機構を適用できるかの判定(`ImmCrossProcessStrategy::is_applicable` など、`ime_controller.rs:70,107,140`)の中身は、`key_sequence_policy` の ungated な関数に (profile, IME 種別) を渡すだけである。

```sh
sed -n 577,640p crates/awase-windows/src/ime_controller.rs        # ImeController::apply: view を除けば核の部品だけでできている
grep -n "fn is_applicable" -A2 crates/awase-windows/src/ime_controller.rs
```

### 4. 事実の訂正: BUG-141 の元の窓では、HEAD で `candidate_was_seen` は効かない

BUG-141 の報告は msedge(`Chrome_WidgetWin_1`、`profile=Imm32Unavailable`)である。HEAD の `dispatch_ime_set_open` は押下つきの書き込みで applied を未知にする(ADR-208 D1、`explicit_press_applied_pair`、`state/ime_actuation_decision.rs:208-219`)。
この未知化を止めるのは「実質 TsfNative」の窓だけである(`engine_press_unknowns_applied`、`:186-188`)。`Chrome_WidgetWin_1` はその窓の一覧に無い(`focus/class_names.rs:51-60`・`:259-265`)。
したがって、元の窓で 2 回目の Ctrl+無変換が送られることは、HEAD では D1 が保証している。`candidate_was_seen` の判定(`:223-229`)が効くのは、TsfNative の窓の Engine 経路と、`press=None` の書き込みである。
第 1 段階の対象(決定 4)は、この事実に合わせて 2 つの形に分けた。

## 決定

### 決定 1: 範囲と完成像(方向)

再生基盤は、閉ループのハーネスを拡張して作る(第 3 のハーネスは作らない)。完成像は次のとおり。

- **入力**: 人が数打鍵に縮めた手書きのシナリオ(E7)。後の段階で、報告の journal の `KeyInput`・`ImeEvent`・`TimerFired` を入れる(決定 5 の段階 5)。
- **前段**: B5 の補助(`KeyInput` → `RawKeyEvent`、エンジンのタイマーのループ)で、HEAD の `Engine::on_input`/`on_timeout` を通す。
- **中段**: エンジンの効果のうち `SetOpen` を、本番と同じ核の関数(決定 2)に渡す。押下台帳・applied・belief は本物の `ImeStateHub`(P5a-1)が持つ。物理 IME キーの shadow toggle の経路は段階 2 で同じ核に合流させる。
- **後段**: `run_chain` に渡す `MechanismWriter` の実装だけを再生用にする(擬似 IME に `MechanismCommand` を当てて outcome を返す)。本番では `SyncChainWriter`(`ime_controller.rs:456-`)がこの位置にあり、`apply_mechanism` で実際に送る。
- **観測**: OS の読み取り(IME の開閉・conv・候補ウィンドウの表示)は擬似 IME(`tests/support/pseudo_ime.rs`)が与える。後の段階で、記録した観測も与えられるようにする。
- **出力**: 本番と同じ `ActuationDecisionRecord`(`state/actuation_decision_record.rs`、journal に載る型)の列と、各ステップの擬似 IME の真の状態。期待値(`attempts` の `command` の列、最終の開閉)は人が書く。

**本番と再生は、同じ核の関数を呼ぶ**。再生の側に残る「殻の配線の写し」は数行の呼び出しの並びだけで、決定 3 に一覧を書く。写しの行数が増える段階は採らない。

### 決定 2: 第 1 段階で核へ移すもの(挙動は変えない移動)

| 移すもの | 移し先(案) | 本番の殻に残るもの |
|---|---|---|
| `ImeController::apply` の本体(`ime_controller.rs:577-640`): gate の早期 return、`into_actuation`、`run_chain`、記録の組み立て | `state/` の新しい関数 `apply_sync<W: MechanismWriter>(order, inputs, writer) -> (ImeOpenOutcome, ActuationDecisionRecord)`。試行の記録(`attempts`)は writer から受け取る | `ImeController::apply`: view → `DecisionInputs` に変換し、`SyncChainWriter` を作って呼ぶだけ(約 10 行) |
| `dispatch_ime_set_open` の判断(`executor.rs:747-815` と同期の分岐 `:906-930`): D1 の applied の未知化、gate の拒否の記録、claim、`plan_set_open`、同期の適用、何も送らなかったときの `release_press_write`、`caller` の記録 | 同じファイルの関数 `dispatch_set_open<W>(hub, facts, open, press, writer) -> DispatchResult`。`DispatchResult` は `NotOwned(record)`・`SkipDuplicate`・`Async(order)`・`Sync(outcome, record)` | Facts を集める(`current_app_profile`、`is_effectively_tsf_native(class_name)`、`tsf::observer` の IME 種別と `candidate_was_seen`)。`Async` のときの `spawn_local` 以降(`:823-904`)はそのまま殻 |
| `DecisionInputs` を作る変換(`ime_decision_view.rs:143-153`) | 核の関数 `DecisionInputs::from_facts(...)`。`From<&ImeControlView>` はこれを呼ぶ | — |
| `imm_cross_is_first_applicable`(`ime_controller.rs:666-670`)とその下の適用可否 | 核(`decide_chain` と `key_sequence_policy::*_applicable` だけで書ける) | `mechanism_is_applicable` は核を呼ぶ |
| `candidate_was_seen` を消費する規則(`ime_controller.rs:247-268`: GjiDirect の OFF の送信に成功したら消費する) | 核の const fn(例: `consumes_candidate_evidence(mechanism, open, outcome)`) | `apply_mechanism` は核の答えを見て `reset_candidate_was_seen()` を呼ぶ |

核へ出す `pub` は `apply_sync`・`dispatch_set_open`・`DecisionInputs::from_facts` の 3 つまでにする(P4・P5 の可視性の方針、`docs/adr/review/229-opus-visibility-policy.md`。集合は `architecture_guard` で固定する)。
これは汎用の Effect/Handler 基盤ではない。差し替え口は ADR-089 からある `MechanismWriter` 1 つで、`run_chain` は書き換えない(ADR-229 F-D1 の例外 1 をそのまま守る)。

### 決定 3: 再生の側(テスト)の第 1 段階

1. B5 の補助(`src/key_input_replay_tests.rs` の補助の部分、389 行)を `tests/support/` へ移す。依存はすべて `pub`(`scanmap::scan_to_pos`、`state::alt_impersonation::resolve_thumb_key`、`vk::VkCodeExt`)なので移せる。
   BUG-105 のテスト 2 本と fixture は一緒に移し、扱いは段階 5 で決める。
2. ハーネスに `press(RawKeyEvent)` を足す。順に次を呼ぶ。
   1. `Engine::on_input`
   2. 効果の `SetOpen` に対して `hub.handle_engine_set_open`
   3. `dispatch_set_open`(本物、writer は擬似 IME)
   4. `hub.record_ime_apply_result`
3. 擬似 IME に `MechanismCommand` を適用する口を足す(`SendVk(VK_IME_ON/OFF)` と `SetOpenCrossProcessSync`)。あわせて、クセとして候補ウィンドウの表示(`candidate_was_seen` を立てる)を足す。
4. `user_set_open`(既存のシナリオで 6 か所。閉じる 5・開く 1)を、既定の `keys.ime_off`/`ime_on`(Ctrl+無変換/Ctrl+変換、`src/config.rs:598-599`)の `press` に置き換える。`issue_write` の `WriteOrigin::ExplicitUserCommand` の分岐(`issue_actuation_order` の写し)と `handle_engine_decision` の panic を消す。
   既存の 20 本の結果が変わったら、ADR-224 の規則(写しのずれの実例として記録して止める)に従う。

再生の側に残る写し(第 1 段階の後):

- `handle_engine_set_open` を `dispatch` の前に呼ぶ順序(`key_pipeline.rs:1422`)
- バッチの始めに applied を写す順序(`executor.rs:162`)
- 完了をバッチの後に返す順序(`runtime/mod.rs:971`)

どれも呼び出し 1 行ずつで、ハーネスの doc の「写しの一覧」(`harness.rs:13-20`)に書く。消える写しは `issue_write` の明示の分岐である。drift の分岐は段階 4 まで残る。

### 決定 4: 第 1 段階の対象(2 つのシナリオ)と検証

| シナリオ | 窓・IME | 打鍵と観測 | 期待値(人が書く) | 修正を外す mutator(CI で当てる) |
|---|---|---|---|---|
| A: BUG-141 の元の窓 | `Imm32Unavailable`・`Chrome_WidgetWin_1`・GJI(擬似 IME の ATOK 格子)、IME ON | Ctrl+無変換 → 擬似 IME が外部から開く(Chrome では観測できない)→ Ctrl+無変換 | 2 回とも `attempts[0].command == SendVk(VK_IME_OFF)`、最後に擬似 IME が閉 | M-A: `dispatch_set_open` の中の `explicit_press_applied_pair(applied, open, unknowns_applied)` の第 3 引数を `false` にする(D1 を外す)。期待: 2 回目が `command: None`(AlreadyMatched)になり、再生が落ちる |
| B: BUG-141 型の TsfNative 窓 | `TsfNative`・GJI、IME ON | Ctrl+無変換 → 候補ウィンドウの表示(擬似 IME のクセ)→ Ctrl+無変換 → Ctrl+無変換 | 2 回目は `SendVk(VK_IME_OFF)`、3 回目は `None`(ADR-171 の「1 回の証拠につき再送は 1 回」) | M-B: `DecisionInputs::from_facts` で `candidate_was_seen: false`(BUG-141 の根本原因だった配線漏れと同じ形)。M-C: 消費の規則を無効にする(3 回目も送る) |

**合否の判定(ADR-240 の条件)**: 判定基準は最初の CI の run の前に PR 本文に書く。

- 修正あり(HEAD)で、新しいシナリオが PASS する。
- M-A・M-B・M-C を当てたビルドで、それぞれ対応するシナリオが FAIL する。
- 同じ mutator の下で、既存の単体テスト(`cargo nextest run --workspace --lib`)と `explicit_press_exhaustive` がどうなるかを記録する。
- **再生だけが落ちる mutator が 1 つ以上あること**が、この段階の価値の証明になる。見込み:
  - M-A は全列挙テストが自分の順序の写しで D1 を合成しているので、落ちないはず(未確認)。
  - M-B は `gji_direct_*` が入力を直接組むので、落ちないはず(未確認)。
- mutator は `tools/e2e/ime_key_matrix/ablations/` と同じく、置換元が 1 回だけ現れることを assert する Python の置換にする。Linux の test ジョブで当てる 1 回限りの workflow_dispatch か、PR の一時コミットで当て、run ID を PR 本文に残す(ローカルでは走らせない)。

### 決定 5: 段階(各段階は 1〜数 PR。全体は方向であり、段階 2 以降は段階ごとに所有者が決める)

| 段階 | 内容 | 撤去 | 追加の見積もり | 検証(CI) | 取りやめ条件 |
|---|---|---|---|---|---|
| **1(本 ADR で決める)** | 決定 2〜4 | 既存の actuation の再生一式: `actuation_decision_record.rs` の `mod tests` のうち再生・往復の部分(約 730 行、`#[test]` 15 本)、テスト専用の `Deserialize` 一式(約 70 行)、凍結コーパス 1440 行、`docs/journal-replay-guide.md:15-72` の節(約 58 行)、`MechanismCommand` の `unreachable!` の 2 variant を消せない理由(ADR-229 `:282`)。ハーネスの `issue_write` の明示の分岐と `handle_engine_decision`(約 25 行) | 本番: 移動が主で純増 +40〜60 行(未実測)。テスト: ハーネスと擬似 IME +170 前後、シナリオ 2 本 +80 前後。B5 の補助 389 行は移動のみ | 決定 4。windows-cross-check と windows-build が green(殻の移動を見る) | (a) 再生だけが落ちる mutator が 0 個(既存のテストと検出力が同じ)。そのときは案 B″(再生一式を撤去するだけ)に切り替え、核への移動は戻す。(b) 移動で挙動が変わる(windows-build の既存テストか実機 CI の sc-dbe・sc-kanji が変わる)。(c) テスト側の追加が 500 行を超える |
| 2 | 物理 IME キーの経路(`kp_stage_shadow_ime_toggle` `key_pipeline.rs:899-1177`・`kp_shadow_actuate` `:1237-1399`)の判断を核へ移す。`dispatch_set_open` と同じ claim → applied → imm_first の骨格を共有する(今は 2 入口に重複している) | 2 入口の重複の骨格。`architecture_guard.rs:6520` の 2 入口ごとの件数ガードが 1 関数のガードになる。`explicit_press.rs` の順序の写しを核の呼び出しに置き換えられるかを調べる(行数は未確認) | 本番の移動 +150〜250、テスト +150 前後 | BUG-113(同じ押下で 2 経路、P5)、BUG-181(2 回目の F2。押下 ID は core の `is_press_start` から導ける)、BUG-156 を mutator で | 物理キーの経路に `tsf::observer` の読み取りが混ざり、Facts にまとめられない |
| 3 | エンジンの組み立ての共有(`app/bootstrap.rs:279`・`:1174-` から、`keys.*`・`SpecialKeyCombos`・親指・単独タップの設定を純粋な関数へ。`runtime::thumb_forced_open_actions`・`migrate_legacy_solo_tap_actions` を ungated な場所へ) | B5 の `build_engine` の部分的な写し(約 40 行。`set_thumb_shift_faces_enabled` の抜けがある、b5-prototype-result.md 詰まった点 5)とハーネスのエンジンの組み立て | +100 前後(未実測) | 既存の B5 と閉ループのテストが同じ結果 | `runtime/` の依存の整理が移動で済まない |
| 4 | 非同期(ImmCross が先頭の窓)と drift correction の actuation。`run_open_chain_async` の判断と `ir_apply_drift_correction` の適用を同じ核に通す。完了の届く位置は既定で「その入力の処理の直後」とし、シナリオに明示した位置でも届けられるようにする | ハーネスの残りの写し(`issue_write` の drift の分岐、`align_placeholder_desired` の写し、`ir_apply_drift_correction` の前半) | +200〜300(未実測) | BUG-152(ImmCross のタイムアウト後の二重 KanjiToggle)、BUG-163 を mutator で | ADR-180 の 3 関数の独立した再検出(INV-45)を崩す必要が出る |
| 5 | 記録の入力: 報告の journal の `KeyInput`・`ImeEvent`・`TimerFired`・`ImeOpenApplied` をハーネスへ入れる。タイマーと非同期の完了は記録の seq の位置で発火する(B5 の方式) | B5 の BUG-105 の重複テスト(scenarios.rs と同じ入力・期待値)を、記録の seq の分岐を通す fixture に置き換えるか消す | `ImeEvent` の読み込み(今は `Serialize` のみ、`journal.rs:282-295`)、`TimerFired` のレーンの変更(Actuation レーンから KeyInput レーンへ)、+200〜300(未実測) | 報告由来の数打鍵の再現で、修正前後の差 | 所有者が E5(CI の JSON ダンプ)・E1 の優先順位を変えない限り着手しない |

全体の見積もり: 追加は段階 1〜5 で約 1000〜1400 行(本番の移動を含む。すべて未実測)。撤去は次のとおり。

- 段階 1 で、コード約 800 行・データ 1440 行・テスト 15 本
- 段階 2〜4 で、ハーネスの写し(約 100〜150 行)と 2 入口の重複
- `drift_correction_replay`(約 310 行)は journal 検討メモの段階 2 として別に撤去する。本 ADR の段階 4 の後なら、その性質(試行の有界性)を再生でも見られる

純増になる段階(2・4・5)は、表の「検証」の列の BUG を修正前後の差で再現できることを根拠にする。差が出なければ、その段階は取りやめる。

### 決定 6: 時刻の持ち方

- **エンジンのタイマー**: B5 の 2 方式を残す。既定は仮想時計で、期限どおりに発火する。記録の入力では、`TimerFired` の seq の位置で発火させる。
  B5 で分かったとおり、遅れの無い仮想時計は OS のタイマーより必ず早く発火するので、BUG-105 のような競争の不具合は記録の順序でしか再生できない。
  手書きのシナリオで競争を扱うときは、「ここでタイマーが発火する」をシナリオに明示する。
- **ハブの時計**: ハーネスの `HubClock::manual` を、エンジンと同じ仮想時刻で進める(閉ループで実施済み)。
- **非同期の完了**: 段階 4 まで扱わない(段階 1 の対象は同期の経路だけ)。
- **遅延のモデル**(実機の遅延を確率や分布で写すもの): 作らない。ADR-226 の候補 D(実機ログからの遅延の較正)と同じ理由で見送る。

## 却下した案

- **汎用の Effect/Handler 基盤、Plan/Effect の項**: ADR-229(`229-…:212`、`docs/adr/review/229-effect-plan-design-draft-r1-rejected.md`)で却下済み。本 ADR の差し替え口は既存の `MechanismWriter` 1 つだけである。
- **DSL・宣言テーブル・TOML のシナリオランナー**: ADR-219(エンジンテストの DSL は見送り)、ADR-218・220、閉ループの TOML ランナーと Rhai の見送り(2026-10-04)と同じ理由。シナリオは今の閉ループと同じ Rust のビルダーで書く。
- **全記録の自動 fixture 化**: ADR-225 の P2(バグの出力を固定し、修正のたびに落ちる)。期待値は人が書く(E7)。
- **閉ループの写しを増やして殻の経路を真似る**: `explicit_press.rs` が 1 押下ぶんでこれをしており、本番から呼ばれない順序の写しが 2056 行の中にある。写しは本番が変わっても落ちない。本 ADR は写しを 1 行の呼び出しの並びに限り、判断は核へ移してから呼ぶ。
- **journal からの完全再生(B2)**: ADR-232 で却下済み(全書き込みの reduce 化が前提)。
- **再生基盤を作らず、既存の再生一式を消すだけ(案 B″)**: 所有者が案 C を選んだので採らない。ただし決定 5 の段階 1 の取りやめ条件 (a) のときの退き先として残す。

## リスクと限界

- **殻で Facts を集める 1 行は検査されない**。`tsf::observer::candidate_was_seen()` を Facts に入れる行、`is_effectively_tsf_native(class_name)` を渡す行は gated のまま残る。BUG-141 の根本原因(変換が値を写していなかった)と同じ種類の漏れが殻の側で起きても、再生は落ちない。核の変換の漏れ(M-B)は落ちる。
- **IME の応答は擬似 IME が写した範囲だけ**。KA+IME の約 25 件のうち、クセの目録に無い挙動は再現できない。
- **残る写し 3 行**(決定 3)は、本番の順序が変わっても追随しない。
- **候補ウィンドウのラッチ**は、表示(擬似 IME)と送信時の消費(核)だけを扱う。フォーカス移動での解除(`runtime/focus_tracking.rs:593`)と `platform.rs:1144` の解除は扱わない。
- **ガードが壊れる**。`into_actuation` の呼び出し元を固定する `architecture_guard.rs:2831-2910` と、`press_id_is_claimed_and_carried_at_every_order_issuing_entry`(`:6520-`)は、本体の移動で期待値が変わる。どのガードが何本壊れるかは未確認で、段階 1 の PR で列挙する。
- **未確認の点**: 行数の見積もりはすべて未実測(B5 の補助 389 行・ハーネス 713 行・擬似 IME 509 行・`dispatch_ime_set_open` 195 行・`kp_shadow_actuate` 163 行・`kp_stage_shadow_ime_toggle` 279 行だけが実測)。KA の 48 件は確度が中。M-A・M-B で既存のテストが落ちないことは CI で確かめるまで未確認。`actuation_call_guard`(dylint)が `apply_sync` の移動で反応するか(`RESTRICTED_CALLS` は `apply_ime_open_with_view` などで、`run_chain` は含まない見込み)は未確認。

## 所有者に聞くこと

1. **第 1 段階の対象**
   - 選択肢: (a) BUG-141 の 2 つの形(決定 4 の A・B)/(b) BUG-113・181(物理キーの経路。段階 2 の移動が先に要る)/(c) BUG-156
   - 推奨は (a)。エンジンの経路だけで済み、移す殻が最も少ない。A は「HEAD で何が送信を保証しているか」(D1)を、B は ADR-171 の規則を、どちらも本番の関数で固定できる。
2. **既存の actuation の再生一式を段階 1 と同じ PR 群で撤去するか**
   - 選択肢: 撤去する/段階 2 の後まで残す
   - 推奨は撤去する。所有者の方針(置き換えと同時に捨てる)に合う。また、コーパスの 37 件は `GjiDirect` の同期の 1 回だけで、段階 1 の再生が同じ `run_chain`・`decide_attempt` を本物の経路から通す。
3. **閉ループの `user_set_open` を、段階 1 で本物の経路(Ctrl+無変換の打鍵)に置き換えるか**
   - 選択肢: 置き換える/新しいシナリオだけで使う
   - 推奨は置き換える(6 か所)。写しが 1 系統減る。既存の 20 本の結果が変われば写しのずれの実例になり、ADR-224 の規則で止まる。
4. **段階 5(報告の journal を入力にする)を方向に残すか**
   - 選択肢: 残す(着手は E5・E1 の見直しのとき)/外す
   - 推奨は残す。B5 の結果で、競争の不具合は記録の順序が要ると分かっている。ただし `ImeEvent` の読み込みとレーンの変更が要るので、段階 2 の結果を見てから決める。

## 関連する既存文書への追記案(段階 1 の PR 群で行う。本 ADR の起草では書き換えない)

- `docs/tasks/journal-replay-rebuild-study-2026-10-06/b5-prototype-result.md` の「新基盤の範囲の選択肢」の案 C の行: 「所有者が選んだ(2026-10-06)。設計は ADR-241」。
- `docs/tasks/corpus-discard-impact-2026-10-06/README.md` の Q1: 「撤去は ADR-241 の段階 1 と同時、範囲は (B)」。
- ADR-163 の status の先頭: 「TH1e は取り下げ(ADR-241 の段階 1 で再生一式を撤去)」(corpus-discard-impact の Q5 の回答どおり)。
- `.claude/rules/fix-requires-evidence.md:22-27`: 「(b) を将来、再生トレースの追加に置き換える予定」を撤回する(Q3 の回答どおり)。テストの置き場所(`:64-70`)の「ジャーナルリプレイ基盤」の行は、閉ループのハーネス(`tests/support/`)を指すよう直す。
- ADR-224 の status: 「ハーネスは ADR-241 の再生基盤の本体になる。写しの一覧は `harness.rs` の doc」。
- ADR-229 `:137`(F-D1 の例外 1 の理由): RW の撤去後は、型状態(ADR-090)と FakeWriter・再生の writer が例外を支える、と追記する。`:212` と `:282` も corpus-discard-impact §1.4 のとおりに追記する。
- `docs/tasks/fcis-layering-tasks-2026-10-06.md`: 段階 1 の移動(`apply_sync`・`dispatch_set_open`)を F の行として足す。`CORE_MODULES` と mutants の `examine_globs` への追加(V1)を同じ PR で行う。
- `tests/closed_loop_scenarios.rs:12-23` の「host から見える層」の表: executor と `ImeController::apply` の判断を「核を呼ぶ」に直す。
- `docs/known-bugs/BUG-141.md`: HEAD の元の窓では D1 が送信を保証していること(背景 4)と、再生のシナリオ名。

## 再確認のコマンド

```sh
ls docs/known-bugs/BUG-*.md | wc -l                                                     # 184
sed -n 13,27p crates/awase-windows/tests/support/harness.rs                              # 写しの一覧と on_input を呼ばないこと
sed -n 612,626p crates/awase-windows/tests/support/harness.rs                            # SetOpen で panic
sed -n 739,934p crates/awase-windows/src/runtime/executor.rs                              # dispatch_ime_set_open
sed -n 186,188p crates/awase-windows/src/state/ime_actuation_decision.rs                  # D1 を止めるのは TsfNative だけ
sed -n 51,60p crates/awase-windows/src/focus/class_names.rs                               # Chrome_WidgetWin_1 は TsfNative の一覧に無い
grep -c "#\[test\]" crates/awase-windows/src/state/actuation_decision_record.rs           # 17
wc -l crates/awase-windows/tests/journals/actuation_decision/*.json                       # 1440
git grep -n "explicit_press_delivery_with" crates/awase-windows/src | grep -v state/explicit_press.rs
grep -n "\.user_set_open(" crates/awase-windows/tests/closed_loop_scenarios.rs | wc -l   # 6
```
