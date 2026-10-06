---
type: companion-doc
title: |-
  凍結コーパス・古い journal を捨てる影響の洗い出しと、complexity-budget.md の発効条件(TH1e)の書き直し案(2026-10-06、調査と docs の案のみ・コード変更なし)
---

# 凍結コーパスを捨てる影響と TH1e の書き直し案

前提は所有者の判断(2026-10-06)である。journal 検討メモ冒頭の前提(1)は「過去の journal とリプレイ基盤はすべて捨ててよい」
([README.md:12](../journal-replay-rebuild-study-2026-10-06/README.md))。E2 は凍結コーパス(BUG-131 の 37 件)を、E3 は古い報告の journal をすべて捨ててよいとした。
本書はこの判断を覆さない。捨てると失われる証拠は事実として書く。

調べた版は develop `0e4fccb7`(#525 マージ後)。Opus のレビューを反映した時点の develop は `47b1093b` で、差分は docs の 4 ファイルだけ、本書の対象のコードは変わっていない。
ローカルでのビルド・テストはしていない。

## 結論

1. **撤去範囲は (B)「actuation_decision の再生一式」を推奨する。** (A) コーパス 1 本だけを消すと、
   `replay_all_actuation_decision_fixtures` が `assert_ok`(0 ファイルを拒む。`crates/awase-replay/src/lib.rs:28-32`)で落ちるので、このテストも一緒に消すことになる。
   残る `replay_record`/`replay_chain_scan`/`ReplayWriter` は手組みのレコードで自分自身を検査するだけになり、本番のコードを固定しない。
   (C)(`tests/journals/` 全体と `crates/awase-replay`)は所有者の許可の範囲内だが、使わないことを推奨する。
   (C) には、人が期待値を直した correctness の回帰 fixture(BUG-08・BUG-146・ADR-108・F1)が含まれる。これらは凍結コーパスと性質が違い、fix-requires-evidence.md の (a) にあたる。
2. **コーパスと RW では、TH1e はもともと証明できなかった。** 理由は 2 つある。
   - 構造上の理由(強いほう): RW は非同期の writer の適用判定を `AsyncChainWriter::is_applicable` で計算しない。
     「記録に ImmCross の attempt があるか」で代用している(`state/actuation_decision_record.rs:706-709`・`:731-735`)。
     TH1e の対象は `AsyncChainWriter::is_applicable` の統合(`runtime/open_chain.rs:109-119`)である。非同期のレコードがあっても、ハーネスは本番の `AsyncChainWriter` を走らせないので、統合を検証できなかった。
   - データの理由: 37 件は全部 `site: Sync`・機構 `GjiDirect` の 1 回・IME `Gji` で、非同期の経路のレコードが 1 件もない。
   - 失われる計画: ADR-163 の計画では、TH1e の母数は 37 件だけではなかった。TH1d'(Part D)の自動収集で、非同期のレコードを集める予定だった。
     決定 D4(`163-…:559-569`)は「収集は TH1e を待たず今から始める」「TH1e が着地した時点で、それ以前に集まった非同期 ImmCross のレコードを遡って再生できるようにする」と定めている。
     E3(古い journal を捨てる)と (B)(`Deserialize` を外す)で失われるのは、37 件に加えてこの計画である。
3. **規約が縛る対象(I/O 側の関数)の削除を検査できる手段は、いまの Linux 側には無い。** 1-in-1-out が縛るのは、
   `RESTRICTED_CALLS` の許可呼び出し元(`send_input_safe` を呼ぶ `transmit`・`send_ime_mode_key` など、`apply_ime_open_with_view` を呼ぶ `dispatch_ime_set_open`・`ir_apply_drift_correction`。`output/`・`runtime/`・`imm.rs` の関数)と、`tuning.rs` の定数である。
   - 純粋な `decide_*` の全数表は、これらの呼び出し箇所を通らない。
   - 閉ループのシナリオも通らない(`runtime/` と `ImeStateHub` を写しで代用している。`tests/closed_loop_scenarios.rs:12-23`)。
   - B5 の入力の再生(未試作)は、エンジンの出力までしか見ない。
   - 実際に通るのは、windows-latest の実機 CI だけである。比較に要る `SentInput` の中身は debug 行に出ていない(件数だけ、`journal.rs:899-911`)。CI での JSON ダンプは、所有者の判断 E5 で「今はしない」。
   - したがって、代わりの条件を Linux 側の検査で書くと、「永久に満たせない(空文)」か「対象外の削除で満たして発効してしまう(安全装置の無い発効)」のどちらかになる。
     所有者に選んでもらう(§2.4・Q2)。推奨は、当面は未発効のまま据え置くこと。条件は実機 CI での送信列の比較に書き換えておき、CI のダンプ(E5)を見直すときに一緒に判断する。

## 0. 確認した事実

| 事実 | 出典 |
|---|---|
| コーパスは 1 本 1440 行・37 件。site は全部 `Sync`、機構は全部 `GjiDirect` の 1 回、kind は全部 `Gji`、profile は `TsfNative` 33 件と `Imm32Unavailable` 4 件。`caller` は null 24 件と `DispatchImeSetOpen` 13 件(#520 の前は null の 15 件が `ReassertExplicitPhysicalKey` 13・`ForceOnRomajiCorrection` 2)。`command` があるのは 35 件。`candidate_was_seen` は全件で欠けている(`#[serde(default)]` で読んでいる) | `crates/awase-windows/tests/journals/actuation_decision/bug-131-report-01m29kdnz.json` を python で集計 |
| 入力の組み合わせ `(profile, shadow_on, belief_input_mode, open)` は 8 通りあるが、GjiDirect の command は `shadow_on`(一致・不一致・不明)× 開く・閉じる の 6 セルで決まる(profile と belief は効かない、`state/ime_actuation_decision.rs:346-379`)。6 セルは `gji_direct_*` の単体テストが固定している(`:614-746`、8 本のうち 2 本は `candidate_was_seen` の検査)。chain は `chain_matches_caps_table`(`:426`)が、romaji の前書きは `needs_romaji_pre_write_never_fires_for_gji_mechanisms`(`:490`)が固定している。コーパスは、既存の単体テストを超える検査をしていない | 同上 |
| 走査規則(`run_chain`/`run_chain_async`)は、全 outcome の組み合わせを試すテストが既に固定している。RW を消しても、本番の走査を検査するテストは減らない | `state/actuation_chain.rs:779`(FakeWriter)、`:889` `async_and_sync_chains_agree` |
| `ActuationDecisionRecord` は本番で作られ、journal に載る(`Serialize`)。`Deserialize` を使うのはテストだけ | 生成: `ime_controller.rs:521`(定義。呼び出しは `:592`・`:617`・`:646`)、`runtime/open_chain.rs:193`、`runtime/executor.rs:787`。journal: `journal.rs:341`。`Deserialize` の使い手: `actuation_decision_record.rs` の `#[cfg(test)]` だけ |
| journal の `actuation decision` の debug 行は、CI のチェッカーが読む。コーパスとは別物で、(B) の撤去では変わらない | `journal.rs:948`、`tools/e2e/ime_key_matrix/check_kanji_role.py:62`、`test_log_anchors_in_rust_source.py:48` |
| `awase-replay` の使い手は 5 か所ある。コーパスはそのうち 1 か所だけ | `actuation_decision_record.rs:1278`、`tests/journal_replay.rs:43,171`、`tests/drift_correction_replay.rs:57,125`、`tests/read_strategy_replay.rs:24,38` |
| journal 検討メモの段階 2(drift correction の replay の撤去)は、まだ着手していない | `docs/tasks/journal-replay-rebuild-study-2026-10-06/README.md:82`。該当するブランチ・PR は無い |
| 原本は git 履歴から取り出せる。投入時(`1fb073a3`)と #520 の書き換え前(`d703c5d0`)は同一の内容(sha256 `ab33e0e5…`、`BUG-131.md:52` と一致) | `git show d703c5d0:crates/awase-windows/tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`。N-1 適用前の旧形式の生データは git に無く、R2 の報告 JSON にしか無い(R2 に今も残っているかは未確認) |
| `SentInput` の debug 行は `issue_us`・`accepted`・`event_count` だけで、送ったキーの中身を出さない。中身は journal の JSON ダンプにしか無い | `journal.rs:899-911`、所有者の判断 E5(CI の JSON ダンプは今はしない) |

## 1. 影響の洗い出し(3 段階)

### 1.1 段階ごとの撤去対象

| | (A) コーパス 1 本 | (B) actuation_decision の再生一式(推奨) | (C) `tests/journals/` 全体と `crates/awase-replay`(許可済み・推奨しない) |
|---|---|---|---|
| 消えるファイル | `tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`(1440 行、データ) | (A) に加えて、`actuation_decision_record.rs` の `#[cfg(test)] mod tests`(`:408-1288`、881 行)のうち再生と往復の部分(試算で約 730 行)。残すもの: `event_source_kind_discards_payload_but_keeps_variant`、`actuation_decision_record_json_byte_size_is_measured` と、その補助の `inputs`・`order`・`chain_from_slice`・`attempts`。消すもの: `replay_record`(`:508`)、RW(`:631-795`。`ReplayWriter`・`poll_once`・`replay_actuation`・`replay_applicable`・`replay_chain_scan`、#499 の +337 行の大半)、`fixture_dir`(`:1269`) | (B) に加えて、`tests/journal_replay.rs`(232 行)、`tests/drift_correction_replay.rs`(210 行)、`tests/read_strategy_replay.rs`(50 行)、fixture 6 本(348 行、データ)、`crates/awase-replay`(lib 176 行と Cargo.toml 24 行)。合わせて約 1040 行 |
| コードの純減 | 0(データだけ) | 約 800 行(テスト約 730、`Deserialize` 側約 70)。ほかにデータ 1440 行、ガイドの節 約 58 行(docs) | (B) に加えて、コード約 690 行とデータ 348 行 |
| 消えるテスト | 1 本(`replay_all_actuation_decision_fixtures`)。消さないと 0 ファイルで落ちる | 17 本のうち 15 本: `replay_*` 7 本(`:824`・`:983`・`:1004`・`:1048`・`:1092`・`:1178`・`:1277`)、`chain_scan_*` 5 本(`:1160`・`:1172`・`:1234`・`:1243`・`:1256`)、`actuation_decision_record_round_trips_via_json`、`deserialize_rejects_chain_longer_than_max_write_mechanisms`、`event_origin_record_round_trips_via_json`(`EventOriginRecord` の `Deserialize` を外すと往復の検査は成り立たない。JSON の形は残す `json_byte_size` のテストが通るので、書き直さずに消す) | (B) に加えて 13 本: journal_replay 3、drift_correction_replay 3、read_strategy_replay 1、awase-replay 6 |
| 撤去できる `Deserialize` | 無し(残るテストが使う) | `ActuationDecisionRecordWire` の derive、`TryFrom<Wire>`(`:376-394`)、`impl Deserialize for ActuationDecisionRecord`(`:401-406`)、`nested_optional_bool` の deserialize 側(`:98-134`、約 37 行)。`EventSourceKind`(`:147`)、`EventOriginRecord`(`:165`)、`ActuationOrderRecord`(`:184`)、`AttemptRecord`(`:206`)、`DecisionInputs`(`ime_actuation_decision.rs:44`)と、その `#[serde(default)]`(`:53`、ADR-171 で旧コーパスを読むために足したもの)、`DecisionSite`(`:91`)、`MechanismCommand`(`:107`)、`ConvAfterOpenId`(`conv_after_open.rs:13`)、`WriteMechanism`(`actuation_chain.rs:139`)、`AppImeProfile`(`focus/class_names.rs:103`)、`ImeKindId`(`state/ime_kind.rs:27`)。ほかの `Deserialize` 型・他の crate・tools からの参照は、git grep では無かった。最終確認は実装の PR の CI(windows-cross-check)で行う | (B) に加えて: `ConvClassifyFixture` とその構成型(`conv_classify.rs:17,31,79,184`)、`ImeReadStrategy`・`ReadReason`・`ReadDecision`・`ReadStrategyFacts`(`ime_read_strategy.rs:14,25,38,47`)、`DriftCorrectionFixture`/`Tick`(`ime_actuation.rs:328,344`)、`FeedbackPolicy`(`:18`)、`ActuationAction`(`:210`)、`ObservationSource`(`ime_event.rs:86`)、`Generation`(`event_origin.rs:58`) |
| 外せない `Deserialize` | — | `Generation` は `DriftCorrectionTick.epoch` も使うので、(B) だけでは外せない(段階 2 と合わせて外す)。`ImeOpenOutcome`・`InputModeState`(ルートの `awase` crate)・`VkCode`(`awase-vkmap`)は本書の範囲外(未確認) | ルート crate の型は同じく範囲外 |
| 撤去で外れる制約(利点) | — | `MechanismCommand` の `unreachable!` の 2 variant(`ime_controller.rs:326`)の撤去は、「凍結コーパスに 35 件の `"command"` があり、型を分けると replay の互換に響く」という理由で止めてあった(ADR-229 `:282`)。(B) でこの理由が無くなる。ADR-089 §9-15 の据え置きの理由(TH1e のコーパスの再生をやり直すことになる、`089-…:738-741`)も無くなる | 同左 |
| ガード・CI | 変更なし | `architecture_guard.rs:1834` の `strip_comments_keeps_production_code_in_files_with_glob_comments` は、「コメントに glob(`tests/journals/*.json`)を持つ実在の 3 ファイル」の 1 つとして `actuation_decision_record.rs` を使っている。モジュール doc(`:13`)の glob を消すと、このファイルは前提を満たさなくなる(テストは通るが、検査の意味が無くなる)。glob を持つ別のファイルに差し替える。`layer_boundary_guard.rs:497` の `CORE_MODULES` は、モジュールが残るので変えない。`ci.yml` は変えない | `ci.yml:51` から `--test journal_replay --test drift_correction_replay --test read_strategy_replay` を外す(`ci_test_coverage_guard` が ci.yml とテストファイルの一致を見る)。workspace の `Cargo.toml:2` の member、`crates/awase-windows/Cargo.toml:61` の dev-dep、`Cargo.lock`、`.cargo/mutants.toml:22` の除外も直す |
| `allow(dead_code)` | 影響なし | 影響なし(見込み、CI で確認)。`actuation_decision_record` は `pub mod`(`state/mod.rs:119`)で、`allow(dead_code)` が付いていない。消すのはテストと `Deserialize` の実装だけ | `ConvClassifyFixture` は src にある fixture 専用の型。消した後に残る型が未使用にならないか、確認が要る(未確認) |
| 残るもの | 再生一式は残るが、本番を固定しない | 本番の `ActuationDecisionRecord` の生成・`Serialize`・journal の `ActuationDecision` variant・debug 行(人が読む診断と、CI のチェッカーが読む用途)。純粋な `decide_gate`/`decide_chain`/`decide_attempt`(本番の SSOT、`ime_controller.rs`・`open_chain.rs` が呼ぶ)と、その単体・全数テスト。`awase-replay` と、ほかの再生 fixture 4 種 | 本番の記録だけ |
| 失われる証拠 | BUG-131 の実機ダンプから取り出した 37 件の作業コピー。git 履歴には残る(§0) | 同左。加えて、TH1d' の自動収集で集めた非同期のレコードを、後で遡って再生する計画(結論 2)。「記録した chain・attempt を本番の走査コードで再走査する」仕組み(RW)。RW が実回帰を見つけた記録は、git log と PR の本文には無い(未確認の範囲を含む) | BUG-08(`jiskana-vk-kana-injection.json`)と BUG-146(`eisu-does-not-write-open-axis.json`)の実機ログから作った回帰 fixture、`ea3da7f` の回帰の再現、ADR-108 の世代順序の再生、F1 の全方針・全理由の網羅の検査 |

### 1.2 本番の記録を残す場合の負担((B) でも変わらない)

1 レコードは約 577 バイトある。Actuation lane の中では、既存の `ImeActuation` の 7〜8 倍を使う(`actuation_decision_record.rs` の `json_byte_size` テストのコメント)。
この負担は、用途 2(再生による差分の証明)も含めて正当化されていた。(B) の後に残る用途は、用途 1(人が報告を読む)と、`check_kanji_role.py` が読む debug 行だけになる。
debug 行は `JournalEntry::ActuationDecision` のログ出力から出ているので、記録と切り離せない。lane の負担は変わらず、再生の価値だけが消える。
将来の選択肢として、軽量化(attempts を 1 件目だけにする、など)を残す(Q4)。

### 1.3 コード内の doc・コメントの追随((B) で直す)

| 場所 | 内容 |
|---|---|
| `state/actuation_decision_record.rs:1-51` | モジュール doc の「2. `tests/journals/actuation_decision/*.json`(TH1d…)」、「TH1e 完了まで自動差分証明の対象外」、「`ReplayWriter`…(`tests::replay_chain_scan`)」。`:274`・`:284` のフィールド doc が `replay_record` に触れている |
| `state/mod.rs:116-118` | 「本番経路への配線は別タスク(TH1d/TH1e)」 |
| `state/ime_actuation_decision.rs:82-88` | `site` の意味と `replay_record`、凍結コーパスの `caller` 15 件の書き換え |
| `ime_controller.rs:322`、`runtime/executor.rs:925` | TH1e・`replay_record` の chain 再導出への言及 |
| `journal.rs:336-340` | `ActuationDecision` の doc「実機コーパスを抽出できるようにする」 |

### 1.4 docs・ADR・規約・メモリの参照

| 種類 | 場所 | (B) での扱い |
|---|---|---|
| 規約 | `.claude/rules/complexity-budget.md:45-62`(発効条件の節。コーパスのパスと TH1e) | §2 |
| 規約 | `.claude/rules/fix-requires-evidence.md:22-27`((b) を将来「再生トレースの追加」に置き換える予定。前提として ADR-162 E1/E4 の能力ベースの条件を挙げる)、`:64-70`(テストの置き場所に「ジャーナルリプレイ基盤」) | :22-27 は Q3 の回答に従う。:64-70 は (B) では変えない((C) を選ぶなら直す) |
| ガイド | `docs/journal-replay-guide.md:15-72`(「ActuationDecision コーパスの扱い」と抽出手順) | 節を撤去し、撤去の経緯と原本の取り出し方を 2〜3 行の注記にする。conv_classify の節(`:73` 以降)は残す |
| ADR(現行の判断の根拠) | `229-…:137`(F-D1 の例外 1。`run_chain(_async)` を Cmd の状態機械に書き換えない理由に「型状態〈ADR-090〉と ADR-163 の再生ハーネス〈ReplayWriter〉を支えている」を挙げる) | (B) で理由の片方が無くなる。例外は、型状態(ADR-090)と FakeWriter(`actuation_chain.rs:761-790`)を使うテストで保てる。その旨を追記する |
| ADR(現行の判断の根拠) | `229-…:212`(代案 A「再生は `awase-replay` + ReplayWriter」)、`229-…:282`(`unreachable!` の 2 variant の撤去を止めた理由) | :212 は「RW は撤去」と追記する。:282 は §1.1 のとおり、撤去を止める理由が無くなったと追記する |
| ADR(見送りの理由に引用) | `232-…:126`(見送りの理由に `replay_record` を引く) | 本文は変えない。見送りの結論は ADR-225 の理由で保たれる |
| ADR | `163-…replay-harness.md`: ステータス `:8`・`:28-30`、TH1d `:43-47`、TH1e `:71`・Part C `:408-455`、D3/D4 `:553-569`、D5 の追記(#520)`:585-593`、Part D `:624-633`、今後の議論 `:769-776` | §3 |
| ADR | `163-implementation-tasks.md:161,327,352-365` | §3 |
| ADR | `158-complexity-reduction-north-star.md:8`、`158-implementation-tasks.md:668,711-740,776-786`(TH1・TH4 の依存) | §3 |
| ADR | `159-existing-io-boundary-inventory.md:8`(「残り: TH1e」) | §3 |
| ADR | `162-governance-reversal.md:8`(E1 は TH1e 未達で未発効)、`:233-262`(依存節。ADR-159 の記録・再生基盤を依存先にしている。能力ベースの条件の原文は `:242-248`) | §3。TF2 は #524 で撤去済み。(B) の後は、ADR-159 の記録・再生基盤は actuation について何も残らない |
| ADR(言及のみ) | `180-…:71,120,191-195,249-259,364`、`089-…:738-741`、`171-…:155-164`、`190-…:115`、`225-…:9,47`(S3 を TH1e に吸収)、`226-…:43,72`、`167-…:192`、`168-…:86`、`170-…:281` | 本文は変えない。163・162 の追記から辿れれば足りる。ADR-089 §9-15 の据え置き理由と ADR-225 の S3 の吸収先は無くなる(Q6) |
| ADR 索引 | `docs/adr/index.md:168,172`(159・163 のステータスの短縮) | ステータスを変えるなら、短縮表示だけ直す |
| known-bugs | `docs/known-bugs/BUG-131.md:51-54`(書き換えの記録と原本) | 撤去の 1 行と、原本の取り出し方(`d703c5d0`。`1fb073a3` と同一)、N-1 前の旧形式は R2 にしか無いことを追記する |
| タスク表 | `fcis-layering-tasks-2026-10-06.md:49`(RW の目的は「F-D1 の handler 例外の実証」)、`:25` | :49 は ADR-229 :137 の追記と対で直す |
| 棚卸し(履歴) | `docs/tasks/actuation-confluence-inventory.md:13,43-64,146`、`journal-replay-rebuild-study-2026-10-06/{README,inventory}.md`、`layering-inventory-2026-10-06/inventory-a2.md:209`、`effect-signature-inventory-2026-10-06/inventory-e0.md:305`、`docs/design/opus-review-adr169.md:344`、`docs/adr/review/*` | 変えない(その時点の記録)。journal 検討メモの後続タスク 2 件(`README.md:29-31`)に、本書へのリンクを 1 行足す |
| メモリ | `project_adr163_part_d_bug_report_corpus_2026_09_11.md`、`project_adr163_th1c_decision_record_replay_2026_09_10.md`、`project_adr163_th1b_actuation_decision_wiring_2026_09_10.md`、`project_adr225_226_journal_ci_integration_rejected_2026_10_04.md`、`project_journal_replay_rebuild_study_2026_10_06.md`、`project_adr229_fcis_layering_2026_10_06.md` | 撤去が develop に入った時点で、親セッションが更新する(本書の作業では触らない) |

## 2. complexity-budget.md の発効条件の書き直し案

### 2.1 いまの条件が守ろうとしたもの

ADR-162 の依存節(`:233-240`)は、こう書いている。削除の安全性を示す装置が無い状態で削除を義務にすると、検証できない削除を強制するだけになり、「翌日 revert」が増える。
そこで round4 TJ4 M5(`:242-248`)は、条件を能力ベースにした。「実際の削除・統合を 1 件、N 本の記録トレースの再生で、送信列の差分ゼロとして検証できたこと」である。
配線を確認しただけ(TF1/TF2 の完了)では、この能力を得たことにならない。complexity-budget.md(`:47-52`)は、比べる対象を「attempts 列・`MechanismCommand` 列」と明確にしている。

**この規約での「発効条件の緩和」の意味**: 条件を緩めると、1-in-1-out の縛りは早く効く(開発側への制約が早まる)。その代わりに、縛りの前提にある安全装置の要件が弱まる。
「規約が緩む=制約が減る」という意味ではない。

### 2.2 何なら縛りの対象の削除を検査できるか

縛りの対象は、`RESTRICTED_CALLS` の許可呼び出し元と `tuning.rs` の定数である(complexity-budget.md「ルール」節)。
許可呼び出し元は、どれも I/O 側の関数である(`lints/actuation_call_guard/src/lib.rs:55-`。`send_input_safe` を呼ぶ `transmit`・`send_ime_mode_key`・`reinject` など。`actuate_ime_control` を呼ぶ `set_ime_open_for_target`。`apply_ime_open_with_view` を呼ぶ `dispatch_ime_set_open`・`ir_apply_drift_correction`)。

| 手段 | 縛りの対象の削除を検査できるか | できること | できないこと |
|---|---|---|---|
| 凍結コーパスの再生(いまの条件) | できなかった | 記録した決定を `decide_*` で再計算する。同期の走査を再走査する | 非同期の writer を走らせない(結論 2)。I/O の呼び出し箇所を通らない |
| 純粋な `decide_*` の全数表(F2〜F5 で実績あり) | **できない** | 判断の全入力を検査する | I/O の呼び出し箇所を通らない。許可リストのエントリを消しても、表は定義上変わらない。「消した経路をわざと壊すと表が落ちる」を満たせない(ADR-163 round2 R5 が退けた空証明と同じ形) |
| 閉ループのシナリオ(`tests/closed_loop_scenarios.rs`) | **できない** | `state/` の純粋な層(`ImeModel::reduce`・`check_drift_correction`・予測器)の合成と、時間・初期条件の持ち越しを検査する | `runtime/` と `ImeStateHub` は写しで代用している(`:12-23`)。本番の `runtime/` を壊しても落ちない |
| P5(ハーネスの写しを本物に置き換える、ADR-229 タスク表 `:48`) | 一部だけ(予定) | `platform_state` の 5 系統が本物の呼び出しになる。drift・予測・warrant の配線の写しが無くなる | `runtime/` は `#[cfg(windows)]` のまま見えない。`output/`・`imm.rs` の I/O も対象外。完了しても、許可呼び出し元の削除は検査できない |
| B5 の入力の再生(未試作、所有者の判断 E1) | **できない** | 記録した `KeyInput` 列を HEAD のエンジンに流し、タイマーの発火まで再現する(BUG-105・145 型) | エンジン(ルートの `awase` crate)の出力までで、`runtime/`・`output/` の送信は通らない |
| 実機 CI での送信列の比較(windows-latest、`e2e-ime.yml` の `sc-*` など) | **できる(唯一)** | 変更の前と後で同じシナリオを流し、`SentInput` 列(または `actuation decision` の debug 行)を比べる。本物の `runtime/`・`output/`・`imm.rs` を通る。比べるのは送信列なので、いまの条件の趣旨にいちばん近い | 中身の比較には、`SentInput` の JSON ダンプが要る(debug 行は件数だけ、§0)。CI のダンプは E5 で「今はしない」。フォーカスを奪われるなどの非決定性があり、`continue-on-error` の部分集合がある。シナリオに無い入力は見ない。1 回の比較に実機ジョブ 2 回ぶんの時間がかかる。比べる道具(2 つの journal の `SentInput` 列の差分)はまだ無い |
| tuning 定数 | どの手段もできない | — | 時間の値の削除の安全性は、実機での実測でしか示せない(tuning-constants.md の実測の義務が既にある)。いまの条件でも同じだった |

### 2.3 選択肢

| 案 | 条件の中身 | 縛りの対象の削除を検査できるか | 緩むもの | 評価 |
|---|---|---|---|---|
| **① 当面は未発効のまま据え置く(推奨)** | 発効条件を ② の文言に書き換える。ただし、達成の期限は付けない。CI のダンプ(E5)を見直すときに、② の道具を作るかどうかを一緒に決める | — | 無し(縛りは効かないまま) | いまの状態と実質同じで、正直である。コーパスを捨てても、規約の効き方は変わらない(もともと満たせなかったので) |
| ② 実機 CI での送信列の比較 | 「宣言の対象に触れる実際の削除・統合を 1 件、windows-latest の実機 CI で、変更の前と後に同じシナリオ群を流し、`SentInput` 列が一致することを示す。さらに、変更前のコードの消す経路に変異を入れて流すと、一致しなくなることを示す(空証明でない)」 | できる | 「N 本の記録トレース(実機の記録)」が、「実機でのシナリオの実行」に替わる(記録の出自が外れる。緩和)。比べる対象は送信列のまま | 趣旨にいちばん近い。前提として、CI の JSON ダンプ(E5 の見直し)と、2 つの journal の `SentInput` 列を比べる道具が要る。非決定性のため、比較は同じ構成を複数回流して多数決にする、などの決め事も要る |
| ③ 対象を絞る | 「判断のロジックの抽出と許可リストの削除を 1 つの PR にし、ロジックの部分は変更前に凍結した全数表(出力に `MechanismCommand`〈送る/送らない、どのキー〉を含む)の一致と変異の検査で示す」 | **できない**。I/O の呼び出し箇所そのものは検査しないままになる | 2 つ緩む。(a) 記録の出自が外れる。(b) 比べる対象が、送信列(attempts・`MechanismCommand` の列)から純粋な関数の出力に替わる | 満たせば発効するが、発効後に義務になる削除(I/O の呼び出し元を消す削除)の安全は示されない。ADR-162 依存節が防ごうとした、検証できない削除の強制と同じになる。推奨しない |
| ④ 能力の条件を外す | 「所有者が発効を宣言した時点で発効」 | — | 安全装置の要件そのものが無くなる(明確な緩和。縛りは最も早く効く) | ADR-162 の M5 の訂正を取り消す判断になる。推奨しない |

**自己点検**: 初版(`10cb00d3`)は「全数表 + 変異の検査」を推奨し、「縛りを実質なくす変更にはなっていない」と書いた。これは誤りだった。
全数表も閉ループも、縛りの対象(I/O 側の関数)の削除を検査できない。そのため初版の案は、満たせなければ空文になる。満たせば、対象外の削除で発効してしまう(③ と同じ)。
本版では、検査できる唯一の手段(②)を条件の文言として残し、その道具ができるまでは発効しない(①)ことを推奨する。
① は、いまより縛りを弱めも強めもしない。② の採否は、CI のダンプ(E5)と道具を作る費用の判断に依存する。

### 2.4 書き直しても、規約はすぐには効かない

- complexity-budget.md は「起草済み・未発効」(`:3-7`)である。発効条件を書き直すのは、条件の中身を替えるだけである。
  条件を満たす 1 件の実績を作り、ステータス行を「発効」に書き換える明示の操作(所有者の承認)を経て、はじめて効く。
- F2〜F5(#504・#512・#514・#517)は判断の分割で、許可リストのエントリや tuning 定数の削除・統合ではない。どの案の「1 件」にも当たらない見込みである(各 PR の差分で許可リストが減ったかどうかまでは未確認)。
- ADR-162 E1 には、もう 1 つの条件(TB0・TB1 の宣言機構があること)がある。これは満たしている(`158-implementation-tasks.md:727-728`)。

### 2.5 complexity-budget.md の「発効条件」節の書き直し文案(① + ② の文言)

```markdown
## 発効条件

[ADR-162](../../docs/adr/162-governance-reversal.md) round4 TJ4 M5 の訂正により、本ルールの発効条件は
「配線確認」ではなく**能力ベース**である: 宣言の対象(`RESTRICTED_CALLS` の許可呼び出し元、または
`tuning.rs` の定数)に触れる**実際の削除・統合を 1 件**行い、次の 2 点を示せたこと。

1. windows-latest の実機 CI で、変更の前と後に同じシナリオ群を流し、`SentInput` 列
   (送信列)が一致する。
2. 変更前のコードの、消す経路に変異を入れて同じシナリオ群を流すと、1. の列が一致しなくなる
   (シナリオが消す経路を通らない削除は、空証明として 1 件に数えない。ADR-163 round2 R5)。

純粋な判断関数の全数表や、Linux の閉ループのシナリオは、宣言の対象(I/O 側の関数)を通らない
ため、この条件の代わりにならない。I/O の順序の細部・`with_app` の再入頻度・実 cmd/lparam の
バイト値は対象外とする。tuning 定数の削除は、tuning-constants.md の実測で示す。

2026-10-06 の改訂: 以前の条件は「N 本の決定レコードの再生で差分ゼロ」(ADR-163 TH1e、凍結コーパス
`bug-131-report-01m29kdnz.json`)だったが、所有者の判断でコーパスと再生一式を撤去した。コーパスは
37 件すべてが同期経路の GjiDirect で、再生ハーネスは非同期の writer を走らせないため、もともと
TH1e を証明できなかった。「実機の記録に由来すること」の要件は、この改訂で「実機での実行」に
替わった(緩和: 縛りが効く条件が満たしやすくなる)。比べる対象は送信列のまま。1. の比較に要る
CI での journal のダンプと比較の道具はまだ無い(2026-10-06 時点)。作るかどうかは CI のダンプ
(journal 検討メモ E5)を見直すときに決める。経緯は docs/tasks/corpus-discard-impact-2026-10-06/README.md。
この条件を満たす 1 件の実績と、所有者による発効の宣言があるまで、本ルールは参考文書のまま強制しない
(期限は付けない)。
```

## 3. ADR・BUG・docs の更新方針(本文は書き換えず、追記する)

| 対象 | 追記の内容 |
|---|---|
| ADR-163 | frontmatter の `status` の先頭に「(2026-10-06) TH1d のコーパスと再生一式(`replay_record`・RW)を所有者の判断で撤去。TH1e は取り下げ(発効条件は complexity-budget.md で改訂)。Part D の本番の記録(`JournalEntry::ActuationDecision`)は診断用に残す」。本文の末尾に「2026-10-06 追記」の節を 1 つ足し、TH1d・TH1e・D3 の用途 2・D4(遡った再生の計画は撤回)・D5 の追記がどう変わるかを 5〜8 行で書く。RW が非同期の writer を代用していたこと(結論 2)もここに書く。Part B・C の本文は変えない |
| ADR-162 | `status` の「TH1e 未達のため未発効」に「(2026-10-06) 発効条件を改訂、未発効は変わらない」を足す。**依存節(`:233-262`)の末尾に**「依存先を ADR-159 の記録・再生基盤から、実機 CI での送信列の比較(complexity-budget.md の改訂)に替えた。TF2(#524)と ADR-163 の再生一式は撤去済み」を追記する。E4 の扱いは Q3 の回答による |
| ADR-229 | `:137` の末尾に「(2026-10-06) ReplayWriter は撤去。例外は型状態(ADR-090)と FakeWriter のテストで保つ」。`:212` に「RW は撤去」。`:282` に「凍結コーパスの撤去で、`unreachable!` の 2 variant の撤去を止めていた理由は無くなった」 |
| ADR-158・158-implementation-tasks | TH1・TH4 の「依存」節の末尾に 1〜2 行(条件の改訂と、TH1d・TH1e の撤去) |
| ADR-159 | `status` の「残り: … TH1e」に「TH1e は取り下げ(2026-10-06)」を足す |
| 163-implementation-tasks | TH1d・TH1e の項に「撤去・取り下げ(2026-10-06)」を 1 行ずつ |
| fcis-layering-tasks | `:49` の RW の行に「撤去(2026-10-06)、F-D1 の例外は ADR-229 :137 の追記のとおり」 |
| BUG-131 | `:51-54` の後に「コーパスの撤去(2026-10-06、PR #…)」の 1 行と、原本の取り出し方(`git show d703c5d0:…`。投入時の `1fb073a3` と同一)、N-1 適用前の旧形式の生データは R2 の報告 JSON にしか無いこと |
| journal-replay-guide.md | `:15-72` を撤去し、注記 2〜3 行にする(ガイドは現行の手順書なので、追記ではなく置き換える) |
| complexity-budget.md | §2.5 の文案で「発効条件」節を置き換える(規約は現行の文書なので置き換える。経緯は節の中の改訂の注記に残す) |

## 4. 進め方

| 段階 | 内容 | 撤去 | 検証 | 取りやめ条件 |
|---|---|---|---|---|
| 1(docs) | Q1〜Q3 の回答を受けて、complexity-budget.md の発効条件(§2.5)と ADR-162・158・159・163・229 の追記を 1 本の PR にする | 文言だけ | Opus のレビュー(規約の変更なので)。`adr-evidence-consistency` などの docs の CI が green | 所有者が ①・② のどちらも採らない(③・④ を選ぶ場合は文案を作り直す) |
| 2(コード、(B)) | コーパス・`replay_record`・RW・往復のテスト 3 本・`Deserialize` 一式(§1.1)・`#[serde(default)]` を消す。doc(§1.3)、`architecture_guard.rs:1834` の差し替え、journal-replay-guide・BUG-131 の追記を同じ PR に入れる | コード約 800 行、データ 1440 行、ガイド約 58 行 | CI だけで判定する(ローカルでビルドしない): nextest(Linux)・windows-cross-check・windows-build・clippy・`cargo machete`。`serde_json` は awase-windows の本番(journal)が使うので残る見込み | `Deserialize` を外した型に、別の使い手がいることが CI で分かった(その derive だけ残す)。所有者が本番の記録の形式の互換を求めた |
| 3 | journal 検討メモの段階 2(drift correction の replay)と合わせて、`Generation`・`ObservationSource`・`FeedbackPolicy`・`ActuationAction` の `Deserialize` を外す | 段階 2 の見積もり(約 310 行)に、`Generation`・`ObservationSource` の 2 つを足す | 段階 2 と同じ | 段階 2 の取りやめ条件と同じ |
| 4(別判断) | `MechanismCommand` の `unreachable!` の 2 variant の撤去(ADR-229 `:282`)。(B) で止める理由が無くなる | 未見積もり | 別の PR、CI | ADR-229 の棚卸し README §5 の判断に従う |
| (5) | (C) は許可済みだが推奨しない。行うのは、所有者が conv_classify・ime_apply・read_strategy の fixture も捨てると改めて選んだときだけ | コード約 690 行、データ 348 行 | 同上。fix-requires-evidence.md の (a) の置き場所の記述と、BUG-008・BUG-146 などの「回帰テストあり」の記述も直す | B5 の試作で `awase-replay` を使う見込みが立った |

## 5. 所有者に聞くこと

1. **撤去の範囲**: (A) コーパスだけ/(B) actuation_decision の再生一式/(C) `tests/journals/` 全体と `awase-replay`(許可済み)。
   推奨: (B)。(A) だけでは 0 ファイルでテストが落ち、残る再生は本番を固定しない。(C) は許可の範囲内だが、凍結コーパスとは性質の違う correctness の回帰 fixture(BUG-08・BUG-146・ADR-108・F1)を巻き込む。
2. **発効条件**: ① 未発効のまま据え置く(条件の文言は ② にする)/② 実機 CI での送信列の比較(CI のダンプと比較の道具を作る)/③ 対象を絞る(全数表。I/O は未検証)/④ 能力の条件を外す。
   推奨: ①。縛りの対象(I/O 側の関数)の削除を検査できるのは ② だけで、② の道具はまだ無い。③・④ は「緩和」で、縛りが早く効く代わりに、強制される削除の安全を示す手段の要件が弱まる(③ は比べる対象も送信列から純粋な関数の出力に替わる)。
3. **E4(TH4、known-bugs のコードへの回帰)の扱い**: E4 はコードを消さない。散文の known-bugs を再生トレースへ移す作業で、本当の前提は「バグを再現できる媒体があること」である。
   (B) で、actuation の「再生トレース」という媒体は無くなる。問いは「E4 を E1 の能力の条件から切り離し、fix-requires-evidence.md `:22-27` の『(b) を将来、再生トレースの追加に置き換える予定』を撤回して (a)(回帰テスト)に寄せるか」。
   推奨: 切り離す。B5 の試作の結果次第で、再生の媒体として再検討する。ADR-162 round4 S1 が E4 を「コード削除を伴う」側に入れた分類も、その追記で見直す。
4. **本番の `ActuationDecision` の記録**(journal の variant と debug 行)を残すか。
   推奨: 残す。人が報告を読む診断の用途があり、CI のチェッカー(`check_kanji_role.py`)が debug 行を読む。ただし、lane の負担(1 件約 577 バイト、`ImeActuation` の 7〜8 倍)は変わらず、再生の価値だけが消える(§1.2)。軽量化は将来の選択肢として記録する。
5. **ADR-163 の扱い**: 「一部実装」のまま追記する/「TH1e は取り下げ」と status を変える。
   推奨: status の先頭に取り下げを書き足す(§3)。Part A〜D の実装(`decide_*` の SSOT 化と本番の記録)は残るので、「却下」にはしない。
6. **ADR-089 §9-15 と ADR-225 S3**: 前者の据え置きの理由(TH1e のコーパスの再生をやり直すことになる)と、後者の吸収先(TH1e)が無くなる。
   推奨: 今は再評価しない。163 の追記に、この 2 件の前提が消えたことを 1 行だけ書く。

## 確認できなかったこと

- `ImeOpenOutcome`・`InputModeState`(ルートの `awase` crate)と `VkCode` の `Deserialize` が、ほかで使われているかどうか(本書の範囲外とした)。
- (B) で外す `Deserialize` に、grep で見つからない使い手(マクロ経由など)がいないこと。awase-windows に依存する `awase-settings`・`awase-keymap-learn-win`・`e2e-uwp-inputsite-probe` の参照は、git grep では無かった。最終確認は実装の PR の CI で行う。
- RW(#499)以降に、コーパスの再生が実回帰を見つけたことがあるかどうか。git log と PR の本文では見つけていない。
- 不具合報告 01M29KDNZ22KNY1FPXSKBGMW7V の原本(R2 の JSON、N-1 適用前の旧形式)が今も残っているかどうか。
- F2〜F5 の各 PR が、許可リストのエントリや tuning 定数を減らしたかどうか(§2.4)。
- 実機 CI(`e2e-ime.yml`)のどのシナリオが、どの許可呼び出し元を通るか。② を採るときに、変異の検査が成り立つシナリオがあるかを先に確かめる必要がある。
- テストの行数と撤去の行数は、関数の境界からの試算である。実装の PR で正確な値を出す。
