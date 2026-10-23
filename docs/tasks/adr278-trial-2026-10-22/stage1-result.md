---
type: companion-doc
title: |-
  ADR-278 段階 1(試験運転)の結果: e2e のジャーナルをコーパスにして決定を再生し、MB-02 の撤去(#803)と突き合わせた
---

# ADR-278 段階 1 の結果(2026-10-22〜23)

ADR-278(PR #805、起草)の段階 1。作ったものは 3 つ:

1. **コーパスの出力**(既定オフ): `journal.rs` が target `awase::journal_json` の TRACE で envelope を 1 行の JSON に出し、殻(`app/bootstrap.rs`)が
   環境変数 `AWASE_JOURNAL_JSON` のファイルへ分けて書く。e2e-ime の入力 `journal_json=on` で `result-*/journal.jsonl` になる。
2. **再生**: `crates/awase-ime-core/src/corpus_replay.rs`(テスト専用)。`ConvClassifyCall`(`classify_conv_transition`)と
   `ActuationDecision`(`replay_record`: gate・chain・command・チェーン走査)を、記録の入力で再計算して記録の出力と比べる。ほかの種類は件数だけ。
   e2e-ime の `corpus-replay` ジョブが、同じ run(`journal_json=on`)か別の run(`replay_run=<ID>`)のコーパスを、その ref のコードで再生する。
3. **題材**: MB-02 の撤去(#803)。基準版 = `fdefcefb`(#803 の直前の develop)、候補版 = `f5049ff1`(#803 のマージ)。

## 回した run

| run ID | ref(コード) | 内容 | 結果 |
|---|---|---|---|
| 39879808322 | `ci/adr278-corpus-base` `f0ab802f`(基準版 `fdefcefb` + 段階 1 の a・b) | e2e 30 ジョブ(下の `only`)、`journal_json=on`。同じ run の corpus-replay(基準版のコーパスを基準版で再生) | 成功。再生 947 件・食い違い 0 |
| 39879811249 | `feat/adr278-corpus-replay-trial` `c0224de5`(候補版 `f5049ff1` + 段階 1) | 同じ構成、`journal_json=on`。候補版のコーパスを候補版で再生 | 成功。再生 950 件・食い違い 0 |
| 39882331634 | `feat/adr278-corpus-replay-trial` `c0224de5` | `replay_run=39879808322`: **基準版のコーパスを候補版のコードで再生**(2 コミットの比較の本体) | 成功。再生 947 件・食い違い 0 |
| 39882334262 | `ci/adr278-corpus-base` `6ee6980d` | `replay_run=39879808322` + MB-02 のプローブの Facts を基準版の `plan_literal_detect` で再生(このブランチだけの一時テスト) | 成功。下の表 |

`only`: `ts-tsf-gji-40ms,ts-tsf-msime-40ms,tsx-tsf-gji-20ms-cold,sc-reopen-tsf-gji-gap600,sc-reopen-tsf-msime-gap600,sc-driftrecovery-gji-tsf,sc-driftrecovery-msime-tsf,sc-eisu-km-tsf-*,sc-keymatrix-tog-tsf-*`(30 ジョブ)。
確かめたコマンド: `gh run download <run> -n corpus-replay` の `replay.txt`・`mb02.txt`。

## コーパスの中身(基準版 run 39879808322、30 ファイル・46,465 行・1 ジョブあたり約 1MB)

| 種類 | 件数 | 再生 |
|---|---|---|
| `KeyInput` | 14,452 | しない(入力内容) |
| `SentInput` | 9,112 | しない(入力内容) |
| `HookImeModeDiagnostic` | 7,960 | しない |
| `ImeEvent` | 4,138 | しない(`Deserialize` 無し) |
| `GjiFsmTransition` | 3,698 | しない(文字列) |
| `ShadowToggle` | 1,958 | しない |
| `ActuationDecision` | 947 | **する**(`replay_record`) |
| `ImeOpenApplied` | 947 | しない(効果側の記録) |
| `PressWriteClaim` | 937 | しない |
| `DriftPlanDecided` | 589 | しない(Facts が全部は無い) |
| `TsfProbeCompleted` / `TsfProbeStarted` | 516 / 440 | しない |
| `LiteralDetect` | 399 | しない(`Serialize` のみ、romaji を含む) |
| `FocusTransition` | 180 | しない |
| `ClockAnchor` | 150 | — |
| その他(`ImeActuation` 19・`FirstLiteralObserve` 11・`GiveUpFollow` 10・`DriftGiveUp*` 2) | 42 | しない |
| `ConvClassifyCall` | **0** | この構成では 1 件も出ない |

リポジトリには、`ActuationDecision` の記録のうち中身が同じものを 1 件に畳んだ 29 件(20KB、6 構成)だけを入れた
(`crates/awase-ime-core/tests/journals/corpus/e2e-fdefcefb-run39879808322.jsonl`。基準版で取ったので、PR の CI はこれを候補版以降のコードで再生する)。
入力内容を含む種類は入れていない。

## MB-02 との突き合わせ

| 問い | 結果 | 読み |
|---|---|---|
| 基準版のコーパスを候補版で再生して、記録された決定が変わるか | 947 件中 0 件(run 39882331634) | **MB-02 についての証拠にならない**(ADR-278 限界 3)。再生した決定関数は #803 で変わっていない: `git diff --stat fdefcefb f5049ff1 -- crates/awase-ime-core crates/awase-windows-base crates/awase-key-effect src` は `literal_facts.rs` の doc 1 行と `tuning.rs` の撤去した定数 17 行だけ。差分 0 は再生の前から分かっていた |
| 撤去した決定(`plan_literal_detect`)はコーパスにあるか | 無い | warm パスの「設置するか」は journal に記録されない(限界 2)。`LiteralDetect` の記録は warm パス由来かを区別する項目を持たない(基準版 399 件・候補版 403 件、verdict の内訳もほぼ同じ) |
| プローブの Facts を記録とみなして基準版で再生すると | 4,800 件すべて `Skip(GateNotProbing)`、記録の plan と一致、Install 0 件(run 39882334262) | 候補版は設置しない(Install 0 件と同じ)ので、決定の差分は 0。**プローブの結果(到達 0 回)と整合**。ただし同じデータ(`[reach-probe]` の行)からの導出で、独立の証拠ではない |
| e2e の判定(参考、再生ではない) | 11 構成の `rc` は基準版・候補版で同じ。不変条件は候補版の `sc-reopen-tsf-gji-gap600` 1 回だけ `inv_rc=1`(基準版は 3 回とも 0) | 1 回だけの差。この構成は Vk モードで warm パスに入らない(プローブ A で warm 0 件)ので、撤去したコードは動いていない。原因は未確認 |

結論: **この方式は MB-02 のような殻の配線の撤去を判定できない**。判定できたのは、撤去した判断の Facts を誰かが記録したとき
(今回はプローブの目印のログ)だけで、それはプローブそのものと同じ手間がかかる。方式が効くのは、変更が**再生対象の決定関数に触れる**
撤去・統合(actuation の gate・chain・command の判断)で、その場合は 1 回のコーパスで何度でも Linux で比べられる(今回の実例は無い)。

## 方式の限界として分かったこと(ADR-278 への追記の候補)

1. `ConvClassifyCall` は今回の 30 ジョブで 0 件。再生対象は実際には 1 種類(`ActuationDecision`)だけだった。
2. `replay_record` は非 Sync の ImmCross の command を再計算しない(ADR-163 の既知の穴)。今回の 947 件にこの組が何件あったかは数えていない(未確認)。
3. 同じ構成でも run ごとに記録の件数が揺れる(`ActuationDecision` 947 / 950)。コーパスは「その run が踏んだ入力」で、固定の網羅ではない。
4. コーパスの大きさ: 1 ジョブ約 1MB(入力内容の種類が 8 割)。artifact の保持は 7 日。

## 段階 2(設計だけ。実装は次の PR)

- **カバレッジの地図**: 擬似 IME の真の状態 `TrueState`(開閉・conv・段階)に、コーパスの記録を写す必要がある。いまのジャーナルで写せるのは
  `ConvClassifyCall` の conv(awase が読んだ値)と `ActuationDecision` の `gate_inputs`(profile・kind・belief の input_mode)だけで、
  **開閉・段階の真値は記録に無い**(belief と観測だけ)。最小の形は「`(profile, kind, belief_input_mode, order.open)` の組がコーパスに何種類あるか」と、
  擬似 IME の全列挙(`pseudo_ime_exhaustive.rs`)が使う profile × kind の組との突き合わせ(踏んでいない組の一覧)。これは真の状態の地図ではなく、
  **決定の入力の地図**であることを明記する。
- **写しの検査**: 擬似 IME の予測を実機の結果と比べるには、押下の直前と直後の真の状態が要る。ジャーナルにあるのは awase が読んだ観測
  (`ImeEvent` の `ObserverReported`、Mozc Readable の `MozcShm`)だけで、`ImeEvent` は `Deserialize` が無い。最小の形は Mozc Readable の構成
  (`MozcShm` が TIP の実状態を読む)に限り、`KeyInput`(vk)と前後の `MozcShm` の読みを組にして格子のセルと照合すること。
  `ImeEvent` の読み取り用の型を足すか、`MozcShm` の記録だけで足りるかを先に決める(未決)。

## 未確認

- `ActuationDecision` の再生が、実際の撤去・統合で食い違いを出せるか(今回の題材は殻の撤去なので出ない。食い違いの検出は単体テスト
  `replay_detects_a_tampered_command` と `tampered_conv_classify_result_is_reported` だけで確かめた)。complexity-budget の TH1e には当たらない。
- 実利用の journal での再生(報告の journal はコーパスに入れない)。
