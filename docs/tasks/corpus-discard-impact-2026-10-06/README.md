---
type: companion-doc
title: |-
  凍結コーパス・古い journal を捨てる影響の洗い出しと、complexity-budget.md の発効条件(TH1e)の書き直し案(2026-10-06、調査と docs の案のみ・コード変更なし)
---

# 凍結コーパスを捨てる影響と TH1e の書き直し案

前提は所有者の判断(2026-10-06): 凍結コーパス(BUG-131 の 37 件)も古い報告の journal もすべて捨ててよい
([journal 検討メモ](../journal-replay-rebuild-study-2026-10-06/README.md) の E2・E3)。本書はこの判断を覆さない。
捨てると失われる証拠は事実として書く。調べた版は develop `0e4fccb7`(#525 マージ後)。ローカルでのビルド・テストはしていない。

## 結論

1. **撤去範囲は (B)「actuation_decision の再生一式」を推奨する。** (A) コーパス 1 本だけを消すと、
   `replay_all_actuation_decision_fixtures` が `assert_ok`(0 ファイルを拒む)で落ちるので、このテストも一緒に消すことになる。
   残る `replay_record`/`replay_chain_scan`/`ReplayWriter` は手組みのレコードで自分自身を検査するだけになり、本番のコードを固定しない。
   (C)(`tests/journals/` 全体と `crates/awase-replay`)には、コーパスと関係のない回帰テストが含まれる
   (BUG-08・BUG-146 の conv 判定、ADR-108 の世代順序、F1 の read_strategy)。所有者はこれらを捨てるかどうかをまだ決めていない。
2. **コーパスは、TH1e の証明に使える材料ではもともとなかった。** 37 件は全部 `site: Sync`・機構 `GjiDirect` 1 回・IME `Gji` で、
   入力の組み合わせは 8 通りしかない。TH1e の対象として決めた `AsyncChainWriter::is_applicable`(`runtime/open_chain.rs:109-119`)は非同期の経路にあり、
   コーパスにはその経路のレコードが 1 件もない。しかも、非同期 ImmCross の attempt は再生で skip される。
   したがって、コーパスの再生で「差分ゼロ」と示しても、ADR-163 round2 R5 が退けた空証明と同じになる
   (ADR-180 `:364` も、コーパスの再生は統一の対象のコードを一度も実行しないと書いている)。
3. **発効条件の書き直し案**: 「実際の削除・統合を 1 件、凍結した特性表(純粋な `decide_*` の全数表)が変更の前後で一致し、
   かつ消した経路をわざと壊すとその表が落ちる(空証明でない)ことを示す」に置き換える(案 1、推奨)。
   「実機の記録に由来すること」という要件は外れる。**この点は緩和である**。検査する入力の範囲は 37 件(8 通り)より広くなる。
   書き直しても規約はすぐには効かない。発効には、条件を満たす 1 件の実績と、ステータス行を書き換える明示の操作が必要になる(§2.4)。

## 0. 確認した事実

| 事実 | 出典 |
|---|---|
| コーパスは 1 本 1440 行・37 件。site は全部 `Sync`、機構は全部 `GjiDirect` の 1 回、kind は全部 `Gji`、profile は `TsfNative` 33 件と `Imm32Unavailable` 4 件。`caller` は null 24 件と `DispatchImeSetOpen` 13 件。`(profile, shadow_on, belief_input_mode, open, command, outcome)` の組み合わせは 8 通り。`candidate_was_seen` は全件で欠けている(`#[serde(default)]` で読んでいる) | `crates/awase-windows/tests/journals/actuation_decision/bug-131-report-01m29kdnz.json` を python で集計 |
| 8 通りの分岐(GjiDirect の shadow が一致・不一致・不明 × 開く・閉じる)は、手書きの単体テストが既に固定している | `state/ime_actuation_decision.rs:614-746`(`gji_direct_*` の 8 本) |
| 走査規則(`run_chain`/`run_chain_async`)は、全 outcome の組み合わせを試すテストが既に固定している。RW を消しても本番の走査の網は減らない | `state/actuation_chain.rs:779`(FakeWriter)、`:889` `async_and_sync_chains_agree` |
| `ActuationDecisionRecord` は本番で作られ、journal に載る(`Serialize`)。`Deserialize` を使うのはテストだけ | 本番の生成: `ime_controller.rs:527`、`runtime/open_chain.rs:193`、`runtime/executor.rs:787`。journal への記録: `journal.rs:341`。`Deserialize` の使い手: `actuation_decision_record.rs` の `#[cfg(test)]` だけ |
| journal の `actuation decision` の debug 行は CI のチェッカーが読む。コーパスとは別物で、撤去しても影響はない | `journal.rs:948`、`tools/e2e/ime_key_matrix/check_kanji_role.py:62`、`test_log_anchors_in_rust_source.py:48` |
| `awase-replay` の使い手は 5 か所ある。コーパスはそのうち 1 か所だけ | `actuation_decision_record.rs:1278`、`tests/journal_replay.rs:43,171`、`tests/drift_correction_replay.rs:57,125`、`tests/read_strategy_replay.rs:24,38` |
| journal 検討メモの段階 2(drift correction の replay の撤去)は、まだ着手していない | `docs/tasks/journal-replay-rebuild-study-2026-10-06/README.md:82`。該当するブランチ・PR は無い(`git branch -r`・`gh pr list` で確認) |
| 原本は git 履歴から取り出せる | 投入: `1fb073a3`。#520 で書き換える前: `git show d703c5d0:crates/awase-windows/tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`(`git cat-file -e d703c5d0` で確認済み)。sha256 の前後は `docs/known-bugs/BUG-131.md:52` |

## 1. 影響の洗い出し(3 段階)

### 1.1 段階ごとの撤去対象

| | (A) コーパス 1 本 | (B) actuation_decision の再生一式(推奨) | (C) `tests/journals/` 全体と `crates/awase-replay` |
|---|---|---|---|
| 消えるファイル | `tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`(1440 行) | (A) に加えて、`actuation_decision_record.rs` の `#[cfg(test)] mod tests`(`:408-1288`、881 行)のうち再生と往復の部分。試算で約 730 行(残すもの: `event_source_kind_discards_payload_but_keeps_variant`、`actuation_decision_record_json_byte_size_is_measured` と、その補助の `inputs`・`order`・`chain_from_slice`・`attempts`)。`replay_record`(`:508`)、RW(`:631-795`、`ReplayWriter`・`poll_once`・`replay_actuation`・`replay_applicable`・`replay_chain_scan`、#499 の +337 行の大半)、`fixture_dir`(`:1269`) | (B) に加えて、`tests/journal_replay.rs`(232 行)、`tests/drift_correction_replay.rs`(210 行)、`tests/read_strategy_replay.rs`(50 行)、fixture 6 本(348 行)、`crates/awase-replay`(lib 176 行と Cargo.toml 24 行)。合わせて約 1040 行 |
| 消えるテスト | 1 本(`replay_all_actuation_decision_fixtures`)。消さないと 0 ファイルで落ちる | 15 本(17 本のうち 2 本を残す): `replay_*` 7 本、`chain_scan_*` 6 本、`actuation_decision_record_round_trips_via_json`、`deserialize_rejects_chain_longer_than_max_write_mechanisms`。`event_origin_record_round_trips_via_json` は Serialize だけの検査に直すか消す | (B) に加えて 13 本: journal_replay 3、drift_correction_replay 3、read_strategy_replay 1、awase-replay 6 |
| 撤去できる `Deserialize` | 無し(残るテストが使う) | `ActuationDecisionRecordWire` の derive、`TryFrom<Wire>`(`:376-394`)、`impl Deserialize for ActuationDecisionRecord`(`:401-406`)、`nested_optional_bool` の deserialize 側(`:98-134`、約 37 行)。`EventSourceKind`(`:147`)、`EventOriginRecord`(`:165`)、`ActuationOrderRecord`(`:184`)、`AttemptRecord`(`:206`)、`DecisionInputs`(`ime_actuation_decision.rs:44`)と、その `#[serde(default)]`(`:53`、ADR-171 で旧コーパスを読むために足したもの)、`DecisionSite`(`:91`)、`MechanismCommand`(`:107`)、`ConvAfterOpenId`(`conv_after_open.rs:13`)、`WriteMechanism`(`actuation_chain.rs:139`)、`AppImeProfile`(`focus/class_names.rs:103`)、`ImeKindId`(`state/ime_kind.rs:27`)。**ほかに使い手が無いことは、実装の PR の CI(windows-cross-check)で確かめる**(grep では、これらを持つ `Deserialize` 型は上の記録型だけだった) | (B) に加えて: `ConvClassifyFixture` とその構成型(`conv_classify.rs:17,31,79,184`)、`ImeReadStrategy`・`ReadReason`・`ReadDecision`・`ReadStrategyFacts`(`ime_read_strategy.rs:14,25,38,47`)、`DriftCorrectionFixture`/`Tick`(`ime_actuation.rs:328,344`)、`FeedbackPolicy`(`:18`)、`ActuationAction`(`:210`)、`ObservationSource`(`ime_event.rs:86`)、`Generation`(`event_origin.rs:58`) |
| **外せない `Deserialize`** | — | `Generation` は `DriftCorrectionFixture` の tick も使うので、(B) だけでは外せない(段階 2 と合わせて外す)。`ImeOpenOutcome`・`InputModeState`・`VkCode` はルートの `awase` crate または `awase-vkmap` の型で、本書の範囲外(未確認) | ルート crate の型は同じく範囲外 |
| ガード・CI | 変更なし | `architecture_guard.rs:1834`: `strip_comments_keeps_production_code_in_files_with_glob_comments` は「コメントに glob(`tests/journals/*.json`)を持つ実在の 3 ファイル」の 1 つとして `actuation_decision_record.rs` を使っている。モジュール doc(`:13`)の glob を消すと、このファイルは前提を満たさなくなる(テストは通るが、検査の意味が無くなる)。glob を持つ別のファイルに差し替える。`layer_boundary_guard.rs:497` の `CORE_MODULES` はモジュールが残るので変えない。`ci.yml` は変えない | `ci.yml:51` から `--test journal_replay --test drift_correction_replay --test read_strategy_replay` を外す(`ci_test_coverage_guard` が ci.yml とテストファイルの一致を見る)。workspace の `Cargo.toml:2` の member、`crates/awase-windows/Cargo.toml:61` の dev-dep、`Cargo.lock`、`.cargo/mutants.toml:22` の除外 |
| `allow(dead_code)` | 影響なし | 影響なし。`actuation_decision_record` は `pub mod`(`state/mod.rs:119`)で `allow(dead_code)` が付いていない。消すのはテストと `Deserialize` の実装だけで、未使用の警告は増えない(見込み、CI で確認) | `ConvClassifyFixture` は src にある fixture 専用の型。消すと、残った型の未使用を確かめる必要がある(未確認) |
| 残るもの | 再生一式は残るが、本番を固定しない | 本番の `ActuationDecisionRecord` の生成・`Serialize`・journal の `ActuationDecision` variant・debug 行(診断の用途 1。CI のチェッカーが読む)。純粋な `decide_gate`/`decide_chain`/`decide_attempt`(本番の SSOT、`ime_controller.rs`・`open_chain.rs` が呼ぶ)と、その単体・全数テスト。`awase-replay` と、ほかの再生 fixture 4 種 | 本番の記録だけ |
| 失われる証拠 | BUG-131 の実機ダンプから取り出した 37 件の作業コピー。git 履歴には残る(§0) | 同左。加えて、「記録した chain・attempt を本番の走査コードで再走査する」検査の仕組み(RW)。この検査が見つけた実回帰は 0 件(PR #499 以降、未確認の範囲を含む) | BUG-08(`jiskana-vk-kana-injection.json`)と BUG-146(`eisu-does-not-write-open-axis.json`)の実機ログから作った回帰 fixture、`ea3da7f` の回帰の再現、ADR-108 の世代順序の再生、F1 の全方針・全理由の網羅の検査。これらは fix-requires-evidence.md の (a) にあたる回帰テストで、**コーパスとは性質が違う**(期待値を人が「あるべき出力」に直した correctness の fixture) |

### 1.2 コード内の doc・コメントの追随((B) で直す)

| 場所 | 内容 |
|---|---|
| `state/actuation_decision_record.rs:1-51` | モジュール doc の「2. `tests/journals/actuation_decision/*.json`(TH1d…)」「TH1e 完了まで自動差分証明の対象外」「`ReplayWriter`…(`tests::replay_chain_scan`)」。`:274`・`:284` のフィールド doc が `replay_record` に触れている |
| `state/mod.rs:116-118` | 「本番経路への配線は別タスク(TH1d/TH1e)」 |
| `state/ime_actuation_decision.rs:82-88` | `site` の意味と `replay_record`、凍結コーパスの `caller` 15 件の書き換え |
| `ime_controller.rs:322`、`runtime/executor.rs:925` | TH1e・`replay_record` の chain 再導出への言及 |
| `journal.rs:336-340` | `ActuationDecision` の doc「実機コーパスを抽出できるようにする」 |

### 1.3 docs・ADR・規約・メモリの参照

| 種類 | 場所 | (B) での扱い |
|---|---|---|
| 規約 | `.claude/rules/complexity-budget.md:45-62`(発効条件の節。コーパスのパスと TH1e) | §2 の書き直し案 |
| 規約 | `.claude/rules/fix-requires-evidence.md:22-27`((b) を将来「再生トレースの追加」に置き換える予定。前提として ADR-162 E1/E4 の能力ベースの条件を挙げる)、`:64-70`(テストの置き場所に「ジャーナルリプレイ基盤」) | (B) では、コーパスに直接は触れていないので変えない。:22-27 の「能力ベースの前提」は §2 の書き直しと連動する(Q3)。(C) を選ぶなら :64-70 の置き場所の記述も直す |
| ガイド | `docs/journal-replay-guide.md:15-72`(「ActuationDecision コーパスの扱い」と抽出手順) | 節を撤去し、撤去の経緯と原本の取り出し方を 2〜3 行の注記にする。conv_classify の節(`:73` 以降)は残す |
| ADR | `163-…replay-harness.md`: ステータス `:8`・`:28-30`、TH1d `:43-47`、TH1e `:71`・Part C `:408-455`、D3/D4 `:553-569`、D5 の追記(#520)`:585-593`、Part D `:624-633`、今後の議論 `:769-776` | §3 |
| ADR | `163-implementation-tasks.md:161,327,352-365` | §3 |
| ADR | `158-complexity-reduction-north-star.md:8`、`158-implementation-tasks.md:668,711-740,776-786`(TH1・TH4 の依存) | §3 |
| ADR | `159-existing-io-boundary-inventory.md:8`(「残り: TH1e」) | §3 |
| ADR | `162-governance-reversal.md:8`(E1 は TH1e 未達で未発効)、`:242-248`(能力ベースの条件の原文) | §3 |
| ADR(言及のみ) | `180-…:71,120,191-195,249-259,364`、`089-…:738-741`(§9-15 を「TH1e の凍結コーパスの再生をやり直すことになる」として据え置いた)、`171-…:155-164`、`190-…:115`、`225-…:9,47`(S3 を TH1e に吸収)、`226-…:43,72`、`167-…:192`、`168-…:86`、`170-…:281` | 本文は変えない。163・162 の追記から辿れれば足りる。**ADR-089 §9-15 の据え置き理由(TH1e と衝突する)と ADR-225 の S3 の吸収先は消える**(Q6) |
| ADR 索引 | `docs/adr/index.md:168,172`(159・163 のステータス短縮) | ステータスを変えるなら 1 語だけ直す |
| known-bugs | `docs/known-bugs/BUG-131.md:51-54`(書き換えの記録と原本) | 撤去の 1 行と、原本 2 つ(`1fb073a3`・`d703c5d0`)の取り出し方を追記する |
| タスク表・棚卸し(履歴) | `docs/tasks/actuation-confluence-inventory.md:13,43-64,146`、`fcis-layering-tasks-2026-10-06.md:25,49`、`journal-replay-rebuild-study-2026-10-06/{README,inventory}.md`、`layering-inventory-2026-10-06/inventory-a2.md:209`、`effect-signature-inventory-2026-10-06/inventory-e0.md:305`、`docs/design/opus-review-adr169.md:344`、`docs/adr/review/*` | 変えない(その時点の記録)。journal 検討メモの後続タスク 2 件(`README.md:29-31`)に、本書へのリンクを 1 行足す |
| メモリ | `project_adr163_part_d_bug_report_corpus_2026_09_11.md`、`project_adr163_th1c_decision_record_replay_2026_09_10.md`、`project_adr163_th1b_actuation_decision_wiring_2026_09_10.md`、`project_adr225_226_journal_ci_integration_rejected_2026_10_04.md`、`project_journal_replay_rebuild_study_2026_10_06.md`、`project_adr229_fcis_layering_2026_10_06.md` | 撤去が develop に入った時点で、親セッションが更新する(本書の作業では触らない) |

## 2. complexity-budget.md の発効条件の書き直し案

### 2.1 いまの条件が守ろうとしたもの

ADR-162 `:242-248`(round4 TJ4 M5): 1-in-1-out を強制する前に、「削除・統合が安全だと機械的に示せる能力」が実際にあることを、1 件の実績で確かめる。
配線を確認しただけ(TF1/TF2 の完了)では、この能力を得たことにならない。狙いは、削除を義務にしたとき、削除相手を安全に消す手段が無いまま
「翌日 revert」が増える(`:236`・`:266`)のを防ぐこと。同じ条件が E4(TH4、known-bugs のコードへの回帰)の着手条件も兼ねている(`158-implementation-tasks.md:783`)。

### 2.2 コーパスが無くても、いまの条件は満たせていなかった

§0・結論 2 のとおり、コーパスは TH1e の対象(非同期の経路)を 1 件も含まない。対象を同期の経路に替えても、8 通りの入力は単体テストが既に固定している。
コーパスを捨てても、満たせる見込みのあった条件が失われるわけではない。失われるのは「実機の記録に由来する」という出自である。

### 2.3 代替の案

| 案 | 条件の文言(案) | 守るもの | 緩むもの | 評価 |
|---|---|---|---|---|
| **1(推奨)** 凍結した特性表 + 空証明でないことの検査 | 「宣言の対象(`RESTRICTED_CALLS` の許可リスト、または `tuning.rs` の定数)に触れる実際の削除・統合を 1 件行い、(i) 変更の前のコードで生成して凍結した、純粋な判断関数の全数表(入力の直積の全点の出力)が変更の後も一致し、(ii) 消した経路をわざと壊す変異を入れると (i) の表が不一致になることを示せたこと。(ii) を示せない削除(表が消した経路を通らない)は 1 件に数えない」 | 削除の安全性を機械的に示す能力を、1 件の実績で確かめる。(ii) は ADR-163 round2 R5 の空証明を退ける要件を明文にしたもの | 実機の記録に由来すること(N 本の記録トレース)が外れる。**緩和である**。I/O の順序や `with_app` の再入頻度は、いまの条件でも対象外(`complexity-budget.md:47-52`)なので変わらない | 全数表は F2〜F5 で実績がある(`fcis-layering-tasks-2026-10-06.md:159`)。入力の範囲は 37 件(8 通り)より広い。弱点: 全数表は純粋な部分にしか張れない。handler に残った I/O(`AsyncChainWriter::is_applicable` は `with_app` が要る)は検査できない。tuning 定数(時間の値)には全数表が向かない(いまの条件も同じ) |
| 2 閉ループのシナリオ | 「…削除・統合を 1 件行い、Linux の擬似 IME の閉ループ(`tests/closed_loop_scenarios.rs`)で、消した経路を通るシナリオの送信列が変更の前後で一致し、(ii) と同じ変異の検査を通ること」 | handler に残った経路も対象にできる | シナリオに無い入力は検査しない。実機の記録の要件は外れる(緩和) | 案 1 で張れない経路のための補助として併記する。単独の条件にはしない |
| 3 B5 の入力の再生 | 「記録した `KeyInput` 列を HEAD のエンジンに流す再生で…」 | 実機の記録の出自を保てる | — | B5 はまだ試作もしていない(所有者の判断 E1: BUG-105 の 1 件で試作してから)。いま条件にすると、規約は無期限に未発効のままになる。将来、案 1 に足す候補として記録だけ残す |
| 4 能力の条件を外す | 「所有者が発効を宣言した時点で発効」 | — | 安全性を示す能力の要件そのものが無くなる。**明確な緩和**(縛りは早く効くが、前提が無くなる) | 推奨しない。ADR-162 の M5 の訂正を取り消すことになり、別の判断が要る |

**自己点検**: 案 1 は条件の「中身」(削除の安全性を機械で示す能力を、1 件の実績で確かめる)を保つ。外れるのは「その示し方が実機の記録に由来すること」で、
これは緩和である。代わりに (ii) の変異の検査で、いまの文言には無かった「空証明でないこと」を明文にした。いまの文言のままでは、コーパスの再生で
(対象の経路を 1 度も通らずに)「差分ゼロ」を出せてしまい、その意味ではいまの文言のほうが弱かった。総じて、縛りを実質なくす変更にはなっていない。
ただし、条件を満たしやすくなるのは事実で、§2.4 のとおり発効に近づく。

### 2.4 書き直しても、規約はすぐには効かない

- complexity-budget.md は「起草済み・未発効」(`:3-7`)。発効条件を書き直すのは、条件の中身を替えるだけである。条件を満たす 1 件の実績(案 1 なら、宣言の対象への実際の削除と変異の検査)を作り、ステータス行を「発効」に書き換える明示の操作(所有者の承認)を経て、はじめて効く。
- F2〜F5(#504・#512・#514・#517)は判断の分割で、許可リストのエントリや tuning 定数の削除・統合ではない。そのため案 1 の「1 件」には当たらない見込みである(各 PR の差分で許可リストが減ったかどうかまでは未確認)。
- ADR-162 E1 には、もう 1 つの条件(TB0・TB1 の宣言機構があること)がある。これは満たしている(`158-implementation-tasks.md:727-728`)。
- TH1e の対象(`AsyncChainWriter::is_applicable` の統合)は、案 1 では検査できない(`with_app` が要る)。最初の 1 件には、純粋な表に載る別の対象を選ぶことになる。

### 2.5 complexity-budget.md の「発効条件」節の書き直し文案(案 1)

```markdown
## 発効条件

[ADR-162](../../docs/adr/162-governance-reversal.md) round4 TJ4 M5 の訂正により、本ルールの発効条件は
「配線確認」ではなく**能力ベース**である: 宣言の対象(`RESTRICTED_CALLS` の許可リスト、または
`tuning.rs` の定数)に触れる**実際の削除・統合を 1 件**行い、次の 2 点を示せたこと。

1. 変更前のコードで生成して凍結した、純粋な判断関数の全数表(入力の直積の全点の出力)が、
   変更後も一致する。
2. 消した経路をわざと壊す変異を入れると、1. の表が不一致になる(表が消した経路を通らない削除は
   空証明として 1 件に数えない。ADR-163 round2 R5)。

純粋な関数に取り出せない handler 側の経路は、閉ループのシナリオ(`tests/closed_loop_scenarios.rs`)の
送信列の一致と同じ変異の検査で補ってよい。I/O の順序・`with_app` の再入頻度・実 cmd/lparam の
バイト値は対象外とする。

2026-10-06 の改訂: 以前の条件は「N 本の決定レコードの再生で差分ゼロ」(ADR-163 TH1e、凍結コーパス
`bug-131-report-01m29kdnz.json`)だったが、所有者の判断でコーパスを撤去した。コーパスは 37 件すべてが
同期経路の GjiDirect で、TH1e の対象(非同期経路)を含まず、もともと空証明しか出せなかった。
「実機の記録に由来すること」の要件はこの改訂で外れた(緩和)。経緯は
docs/tasks/corpus-discard-impact-2026-10-06/README.md。この条件を満たす 1 件の実績と、所有者による
発効の宣言があるまで、本ルールは参考文書のまま強制しない。
```

## 3. ADR・BUG・docs の更新方針(本文は書き換えず、追記する)

| 対象 | 追記の内容 |
|---|---|
| ADR-163 | frontmatter の `status` の先頭に「(2026-10-06) TH1d のコーパスと再生一式(`replay_record`・RW)を所有者の判断で撤去。TH1e は取り下げ(発効条件は complexity-budget.md で案 1 に改訂)。Part D の本番の記録(`JournalEntry::ActuationDecision`)は診断用に残す」。本文の末尾に「2026-10-06 追記」の節を 1 つ足し、TH1d・TH1e・D3 の用途 2・D4・D5 の追記がどう変わるかを 5〜8 行で書く。Part B・C の本文は変えない |
| ADR-162 | `status` の「TH1e 未達のため未発効」に「(2026-10-06) 発効条件を案 1 に改訂、未発効は変わらない」を足す。E1 の節の末尾に 2〜3 行(E4 も同じ条件にするかどうかは Q3 の回答による) |
| ADR-158・158-implementation-tasks | TH1・TH4 の「依存」節の末尾に 1〜2 行(条件の改訂と、TH1d・TH1e の撤去) |
| ADR-159 | `status` の「残り: … TH1e」に「TH1e は取り下げ(2026-10-06)」を足す |
| 163-implementation-tasks | TH1d・TH1e の項に「撤去・取り下げ(2026-10-06)」を 1 行ずつ |
| BUG-131 | `:51-54` の後に「コーパスの撤去(2026-10-06、PR #…)」の 1 行と、原本の取り出し方 2 つ(`git show 1fb073a3:…`〈投入時〉・`git show d703c5d0:…`〈#520 の書き換え前〉) |
| journal-replay-guide.md | `:15-72` を撤去し、注記 2〜3 行にする(ガイドは現行の手順書なので、追記ではなく置き換える) |
| complexity-budget.md | §2.5 の文案で「発効条件」節を置き換える(規約は現行の文書なので置き換える。経緯は節の中の改訂の注記に残す) |

## 4. 進め方

| 段階 | 内容 | 撤去 | 検証 | 取りやめ条件 |
|---|---|---|---|---|
| 1(docs) | Q1〜Q3 の回答を受けて、complexity-budget.md の発効条件(§2.5)と ADR-162・158・159・163 の追記を 1 本の PR にする | 文言だけ | Opus のレビュー 1 回(規約の変更なので)。`adr-evidence-consistency` などの docs の CI が green | 所有者が案 1 を採らない |
| 2(コード、(B)) | コーパス・`replay_record`・RW・往復のテスト 2 本・`Deserialize` 一式(§1.1)・`#[serde(default)]` を消し、doc(§1.2)・`architecture_guard.rs:1834` の差し替え・journal-replay-guide・BUG-131 の追記を同じ PR に入れる | 約 2300 行(JSON 1440、テスト約 730、`Deserialize` 側約 70、ガイド約 58) | CI のみ(ローカルでビルドしない): nextest(Linux)・windows-cross-check・windows-build・clippy・`cargo machete`。`serde_json` は awase-windows の本番(journal)が使うので残る見込み | `Deserialize` を外した型に、別の使い手がいることが CI で分かった(その derive だけ残す)。所有者が本番の記録の形式の互換を求めた |
| 3 | journal 検討メモの段階 2(drift correction の replay)と合わせて、`Generation`・`ObservationSource`・`FeedbackPolicy`・`ActuationAction` の `Deserialize` を外す | 段階 2 の見積もり(約 310 行)に、`Generation`・`ObservationSource` の 2 つを足す | 段階 2 と同じ | 段階 2 の取りやめ条件と同じ |
| (4) | (C) は推奨しない。所有者が conv_classify・ime_apply・read_strategy の fixture も捨てると決めたときだけ行う | 約 1040 行 | 同上。fix-requires-evidence.md の (a) の置き場所の記述と、BUG-008・BUG-146 などの「回帰テストあり」の記述を直す | B5 の試作で `awase-replay` を使う見込みが立った |

## 5. 所有者に聞くこと

1. **撤去の範囲**: (A) コーパスだけ/(B) actuation_decision の再生一式/(C) `tests/journals/` 全体と `awase-replay`。
   推奨: (B)。(A) だけでは 0 ファイルでテストが落ち、残る再生は本番を固定しない。(C) は、コーパスと性質の違う回帰 fixture(BUG-08・BUG-146・ADR-108・F1)を巻き込む。
2. **発効条件の代わり**: 案 1(凍結した全数表 + 変異の検査)/案 1 + 案 2(閉ループで補う)/案 3(B5 を待つ)/案 4(能力の条件を外す)。
   推奨: 案 1 + 案 2 の補助(§2.5 の文案)。「実機の記録に由来すること」が外れる緩和を受け入れるかどうかの判断になる。
3. **E4(TH4、known-bugs のコードへの回帰)** は同じ能力ベースの条件を共有している。同じ書き換えを当てるか。fix-requires-evidence.md `:22-27` の「(b) を将来、再生トレースの追加に置き換える」も、この条件を前提にしている。
   推奨: 当てる。「再生トレース」の部分は「回帰テスト(全数表・閉ループのシナリオ・B5 の再生)」と読み替える 1 行を足す。
4. **本番の `ActuationDecision` の記録**(journal の variant と debug 行)は残すか。
   推奨: 残す。診断の用途(人が報告を読む)があり、CI のチェッカー(`check_kanji_role.py`)が debug 行を読む。再生をやめても、記録の価値は消えない。
5. **ADR-163 の扱い**: 「一部実装」のまま追記する/「TH1e は取り下げ」と status を変える。
   推奨: status の先頭に取り下げを書き足す(§3)。Part A〜D の実装(`decide_*` の SSOT 化と本番の記録)は残るので、「却下」にはしない。
6. **ADR-089 §9-15 と ADR-225 S3**: 前者の据え置きの理由(TH1e のコーパスの再生をやり直すことになる)と、後者の吸収先(TH1e)が無くなる。
   推奨: 今は再評価しない。163 の追記に「この 2 件の前提が消えた」と 1 行だけ書く。

## 確認できなかったこと

- `ImeOpenOutcome`・`InputModeState`(ルートの `awase` crate)と `VkCode` の `Deserialize` が、ほかで使われているかどうか。本書の範囲外とした。
- (B) で外す `Deserialize` について、grep で見つからない使い手(マクロ経由・別 crate の `awase-settings`・`awase-keymap-learn-win`・`e2e-uwp-inputsite-probe` は awase-windows に依存している)がいないこと。実装の PR の CI で確かめる。
- RW(#499)以降に、コーパスの再生が実回帰を見つけたことがあるかどうか。git log と PR の本文では見つけていない。
- 不具合報告 01M29KDNZ22KNY1FPXSKBGMW7V の原本(R2 の JSON)が今も残っているかどうか。リポジトリにあるのは、変換後の 37 件と git 履歴だけ。
- F2〜F5 の各 PR が、許可リストのエントリや tuning 定数を減らしたかどうか(§2.4)。
- テストの行数と撤去の行数は、関数の境界からの試算。実装の PR で正確な値を出す。
