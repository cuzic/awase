---
id: ADR-225
title: |-
  不具合報告 journal・CI 実機・閉ループ・replay の連携は、まず実害を測り(SP0)、抽出手順の縮小版(S1')だけを候補に残す
summary: |-
  当初案(S1〜S7: 報告→fixture 自動変換、共通シナリオ IR、CI journal の自動 fixture 化、擬似 IME 較正、fuzz、BUG frontmatter の定期再検証、自動トリアージ)は、Opus レビュー(round1)で前提の誤りが見つかった。
  (F1)replay(`replay_record`)はレコード1件ごとに決定を再計算して記録値と比べるだけで、「HEAD で症状が再現するか」は判定できない。(F2)「同じ journal 形式」は存在しない(`JournalEntry` は Serialize のみ、fixture 型は4種、閉ループは Rust ビルダー、`sc-*` は awase.log のテキスト判定)。
  (P1)文字キーの VK 列は入力文そのもので、リポジトリは公開(`private=false`)。ADR-095 が公開 issue を避けた判断を fixture のコミットが迂回する。(P2)自動 fixture 化は、バグの出力を固定し、修正のたびに落ちる。(P4)CI から R2 を読むには生打鍵を含むバケットの read トークンが要る。
  決定: S2(IR)・S5・S6・S7 は見送り、S3 は ADR-163 TH1e に吸収。残すのは S1'(`bug-report-fetch` スキルに、入力内容を含まないレーンだけを fixture 形式で書き出す手順を足す)のみ。それも SP0(直近の BUG で、安全レーンの fixture があれば検知できたかを人手で数える)の結果が 2 件以上のときに限る。
status: |-
  見送り(2026-10-04、SP0 の結果: 直近 10 件で「安全レーンの fixture が報告時にあれば検知できた」と言えるのは 0 件、弱い候補 2 件〈BUG-141・BUG-170〉)。S1' も実施しない。以下は起草時の記述:
  起草(2026-10-04、Opus round1 反映済み・未決定・実装なし)。着手条件=SP0 で「安全レーンの fixture があれば修正または後の退行を検知できた BUG」が直近 10 件中 2 件以上。0〜1 件なら本 ADR は「見送り(根拠: SP0)」で閉じる。
related_adr:
  - "ADR-095"
  - "ADR-163"
  - "ADR-217"
  - "ADR-222"
  - "ADR-224"
---

# ADR-225: journal・CI 実機・閉ループ・replay の連携

## 背景(当初の問題意識)

不具合を直すたびに、報告 journal の取得、fixture 化、`sc-*` の手書き、閉ループの手書きを人が繰り返している。
これらを自動で繋げば、報告がそのまま回帰テストになるのではないか、という発想から起票した。

## Opus レビュー(round1)で判明した事実

| # | 事実 | 根拠 |
|---|---|---|
| F1 | replay は「決定関数が記録時から変わったか」を見るだけで、症状の有無は判定できない | `state/actuation_decision_record.rs::replay_record`(494 行付近)。`docs/journal-replay-guide.md` も characterization corpus と明記 |
| F2 | 「同じ journal 形式」は無い。`JournalEntry` は `Serialize` のみで読み戻せない。fixture は `ConvClassifyFixture`・`ImeEventReplayFixture`・`DriftCorrectionFixture`・`ActuationDecisionRecord` の4型。閉ループは Rust ビルダー、`sc-*` は awase.log のテキスト判定(`check_*.py`) | `journal.rs:197`、`tests/support/harness.rs`、`e2e-ime.yml` |
| F3 | CI は awase.log を既に artifact 化している。JSON journal のダンプは CI で起動されていない | `e2e-ime.yml:1148-1171` |
| F4 | 擬似 IME の真値は既に CI 実機の格子学習(`grid-tables/*.json`)由来。未較正なのは遅延だけ | `closed_loop_scenarios.rs` 冒頭 doc |
| P1 | 文字キーの VK 列は入力文の再構成そのもの(ADR-095 B-1 が「キーロガー出力に近い」と明記)。リポジトリは公開(`private=false`) | `docs/adr/095-*.md:83` |
| P2 | 自動生成した fixture は、バグ出力を固定し、そのバグを直すコミットで落ちる | `journal-replay-guide.md`「バグに気づいたときの手順」4 |
| P3 | `ActuationDecisionRecord` に版タグが無く、旧形式の報告を読むには旧形式パーサが要る(ADR-217 と衝突) | `actuation_decision_record.rs` |
| P4 | R2 の読み取りは現在メンテナ個人の OAuth。CI に置くと生打鍵を含むバケットの read トークンになる | `bug-report-fetch/SKILL.md` |
| P5 | 閉ループは Chrome/TSF・`Engine::on_input`・warmup を写さない。報告の主症状(Chrome リテラル化、cold-start、高速打鍵の文字消失)は写さない側にある | ADR-224 背景、`pseudo_ime.rs` |

## 決定

1. **S2(共通シナリオ IR)・S5(揺らぎ fuzz)・S6(BUG frontmatter の定期再検証)・S7(自動トリアージ)は見送る。**
   3バックエンドの入力能力が互いに素(F2、P5)で、共通 IR は和集合型(= ADR-218〜220 で見送った DSL と同型)にしかならない。
   S5 の例示(ts-chrome 高速打鍵)は閉ループでは再現できない。S6・S7 は「HEAD で再現するか」を前提にしており F1 で成立しない。
2. **S3 は ADR-163 TH1e に吸収する。** 実機 journal を同じコミットのバイナリで replay してもトートロジーで通る。
   意味があるのは「develop の CI journal を PR の HEAD で replay して決定差分を出す」ことで、これは TH1e そのもの。新規の仕組みは足さない。
3. **S4 は、遅延の較正だけが未実施**であることを前提に、必要になった時点で別途扱う(ADR-224 の着手条件は変えない)。
4. **S1 は S1' に縮める**: `bug-report-fetch` スキルに、次の allowlist のレーンだけを fixture 形式で書き出す python ステップを足す(新 crate・xtask は作らない)。
   - 安全(入力内容を含まない): `ActuationDecision`・`ConvClassifyCall`・`ImeActuation`・`ImeEvent`。
   - 出さない: `KeyInput`・`LiteralDetect`(要確認)・awase.log 由来の打鍵列。これらは fixture・CI・issue・PR に載せない(リポジトリは公開)。
   - コミット前に人が `expected` の意味を判断する(バグ出力の固定化を避ける)。
5. **CI から R2 を読まない**を既定とする(P4)。R2 キーを frontmatter 等に書かない(90 日で失効)。
6. **schema**: 報告時の `app_version` が HEAD と同じ型を持つ範囲だけを扱い、旧形式パーサは持たない(ADR-217)。
7. **着手条件(SP0)**: コード変更ゼロ・数時間。`docs/known-bugs/` で report_id を引いている BUG の直近 10 件について、
   「安全レーンの fixture が報告時にあれば、その修正または後の退行を検知できたか」を人手で判定して数える。
   2 件以上なら S1' を実施。0〜1 件なら本 ADR は「見送り(根拠: SP0)」で閉じる(ADR-224 と同じ「実害が出るまで着手しない」基準)。

## 代案の扱い(Opus 提示)

- S3 の代わりに、既存の `check_invariants.py`(awase.log の行を数える)に検出器を足す方が、CI→回帰の経路として既に動いている。個別の不具合ごとに検討する。

## 未決

- SP0 の判定を誰がいつ行うか(本セッションで実施可能)。
- `LiteralDetect` が入力テキスト断片を含むか(S1' の allowlist 入りの前に確認)。

## SP0 の結果(2026-10-04)

対象: `docs/known-bugs/` で report_id を持つ BUG の直近 10 件(BUG-140・141・167・170・171・173・174・175・183・184)。
判定基準: 報告時に安全レーン(`ActuationDecision`・`ConvClassifyCall`・`ImeActuation`・`ImeEvent`)の fixture をコミットしていたら、
その修正、または後の退行を検知できたか。

| BUG | 判定 | 理由 |
|---|---|---|
| 140 | 不可 | 原因は config(`right_thumb_key` と `ime_detect.on` の重複)。修正は config 読み込み時の無効化で、回帰テストも config 側 |
| 141 | 弱い | 判定関数 `gji_direct_already_matches` の入力に `candidate_was_seen` を足す修正。報告時のレコードにその入力が無く、旧形式のため replay できない(P3)。ユニットテスト2本と architecture_guard が既にある |
| 167 | 不可 | 設定の文字列解釈。journal に現れない。回帰テストあり |
| 170 | 弱い | GjiFsm の同期漏れ。報告時のレコードは Unwarranted/NotSent を示すが、修正は GjiFsm 側で、`gji_fsm.rs` と `gji_direct_mechanism.rs` に回帰テストが既にある |
| 171 | 不可 | `LiteralDetect`/StaleConfirm(安全レーン外)。未修正 |
| 173 | 不可 | 物理キーの配送判断(`PhysicalKeyDisposition::plan`)で、`ActuationDecision` ではない。`plan_tests` と architecture_guard が既にある |
| 174 | 不可 | journal に文字が出ず(切り詰めあり)、原因も未確定のまま機構を撤去 |
| 175 | 不可 | 機構ごと撤去済み。回帰テストあり |
| 183 | 不可 | 入力言語切替。安全レーンに現れない。未修正 |
| 184 | 不可 | 再現せず。未修正 |

結論:
- 「検知できた」は 0 件、弱い候補が 2 件(141・170)で、どちらも同じコミットで回帰テストが入っている。着手条件(2 件以上)を満たさないため **S1' は実施しない**。
- 構造的な理由もある。fixture は報告時の(バグを含む)決定を記録するので、(a)修正すると落ちる = 修正を検知するだけで、(b)バグが再発しても記録と一致して通る = 退行を検知できない。退行を検知するには人が `expected` を「あるべき出力」へ書き直す必要があり、それは手書きの回帰テストと同じである。
- 実際、10 件のうち 8 件は修正と同じコミットで回帰テスト(ユニット・architecture_guard)が入っていた。journal 由来の fixture が足りないことで見逃した実例は無い。
- 判定は BUG ファイルの記述に基づく。報告 journal の実物でレーンの中身を確かめたわけではない。この点が結論を覆す可能性は低いが、反証が出たら再開する。
