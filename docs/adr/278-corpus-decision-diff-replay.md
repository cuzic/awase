---
id: ADR-278
title: |-
  実機 CI のジャーナルをコーパスとして集め、撤去・変更の候補の決定の差分を Linux で再生し、差分とコーパスの穴だけを実機で確かめる
summary: |-
  撤去・変更の候補が実機の挙動を変えないかを、実機 CI を何度も回さずに判定したい(所有者の要望、2026-10-22)。方式は 3 段:
  (1) 実機 CI(e2e-ime)を広く 1 回回し、各ジョブの本番のジャーナルを JSON のまま artifact に出す(コーパス)。
  (2) 候補のコードで、コーパスに記録された決定の入力を純粋な決定関数に通し直し、記録された出力と比べる(決定の差分)。
  (3) 食い違った決定と、擬似 IME の全列挙が「届く」と言うのにコーパスが踏んでいない状態だけを、狙った実機 CI の構成で確かめる。
  現状調査の結論: ジャーナルは型つきの記録を持つが CI では JSON に出ていない(awase.log の要約行は項目が欠ける、ADR-226 の B 却下の理由)。
  再生は純粋な核の関数だけを通し、殻(配線・呼ぶか呼ばないか)は通さない。したがって「決定が一致した」は「効果が同じ」でも
  「殻の変更が無害」でもない。殻の判断を変える撤去は、その判断の Facts が記録されていなければ、この方式では判定できない。
status: |-
  起草(2026-10-22)。段階 0(本 ADR、docs のみ)。段階 1(試験運転)・段階 2 は未着手。Opus レビューは未実施。
related_adr:
  - "ADR-082"
  - "ADR-158"
  - "ADR-162"
  - "ADR-163"
  - "ADR-222"
  - "ADR-225"
  - "ADR-226"
  - "ADR-241"
  - "ADR-250"
  - "ADR-277"
---

# ADR-278: コーパス → 決定の差分の再生 → 差分だけ実機

## 背景

所有者の要望(2026-10-22): 「CI を使いつつ、擬似 IME も使って、網羅的かつ短時間に検証する方法」。撤去・変更の候補が実機の挙動を変えないかを、
実機 CI(1 run 数十分、matrix 256 ジョブ上限)を候補ごとに回さずに判定したい。いまの撤去(ECRS の MB-02 など)は、候補ごとに目印のログを入れた
プローブ版と撤去版の 2 本を実機 CI で回して比べている(`docs/tasks/mech-ecrs-2026-10-21/probe-mb0203-result.md`、run 4 本・940 ジョブ)。

この方向の過去の判断(重ならないように先に挙げる):

| 判断 | 内容 | 本 ADR との関係 |
|---|---|---|
| ADR-163 TH1e | 削除・統合を凍結コーパスの決定レコードの再生で差分ゼロと示す(`complexity-budget.md` の発効条件) | 本方式の段 2 はその一般化。TH1e は未着手のまま |
| ADR-225 | 不具合報告の journal の自動 fixture 化は見送り(F1: 決定の再計算は「HEAD で症状が出るか」を答えない。P1: 入力内容の公開) | 報告の journal はコーパスに入れない(決定 4) |
| ADR-226 B | 「実機 PASS ログを保存し、決定差分で実機 `sc-*` を選択実行」は見送り。理由: (i) `awase.log` の行から決定レコードを復元できない、(ii) PR では `sc-*` が走らないので削減にならない、(iii) 決定差分ゼロで起きる種類の不具合(BUG-162/163/170/171)がある | (i) は JSON を出して解く(決定 1)。(ii) は用途を「PR の選択実行」から「撤去候補の事前判定」に変えて避ける。(iii) は解けないので限界に書く |
| 検討メモ E5(`docs/tasks/journal-replay-rebuild-study-2026-10-06/README.md`) | 「CI で JSON journal をダンプしない(今は)」 | 所有者が本方式を了承した(2026-10-22)ので、既定オフ・指定した run だけ出す形で見直す |
| 検討メモ E2 | 凍結コーパス(37 件)は捨ててよい | 本方式のコーパスは凍結しない(コミットごとに取り直す、決定 2) |
| ADR-241 | 打鍵から actuation まで通す再生基盤。置き換えと同時に `replay_record` 一式を捨てる(提案のまま) | 本方式は `replay_record` の残存を前提にしない。決定ごとの小さな adapter で足りる |
| ADR-250 | journal は診断用で「再生の入力にはしない」 | 本方式は journal を再生の入力にする。ただし再生するのは、記録が入力と出力の両方を持つ種類だけ(決定 3) |
| ADR-277 | 擬似 IME の遷移を 1 つの表にし、到達性を全列挙する | 本方式の (a) カバレッジの地図・(b) 写しの検査で使う(段階 2) |

## 現状調査(origin/develop `f5049ff1`、2026-10-22)

確かめたコマンド: `git grep -n 'emit_tracing\|fn report_json\|fn absorb' -- crates/awase-ime-core/src/journal.rs`、
`grep -n 'awase.log\|RUST_LOG\|upload-artifact' .github/workflows/e2e-ime.yml`、各再生テストの読み。

### (1) 本番のジャーナルに残るもの

`JournalEntry`(`crates/awase-ime-core/src/journal.rs`)は 28 種類。決定の再生に使えるかで分けると:

| 種類 | 入力 | 出力(awase の決定) | 実 IME の結果(効果) | 再生に使えるか |
|---|---|---|---|---|
| `ActuationDecision` | `gate_inputs`・各 attempt の `DecisionInputs` | chain・各 attempt の mechanism・command | 各 attempt の `outcome`(OS API の戻り値。実 IME が開いたかではない) | 使える(`ActuationDecisionRecord` は `Deserialize` あり、`decide_gate`/`decide_chain`/`decide_attempt` は `pub(crate)` なので crate 内テストから) |
| `ConvClassifyCall` | conv・current・is_cold など 6 値 | `ConvTransition` | なし | 使える(`classify_conv_transition` は pub、`ConvTransition` は `Deserialize` あり) |
| `ImeActuation` | 目標・方針・試行回数 | action | なし | 一部(`origin` が `&'static str` で `Deserialize` 不可。fixture 型を経由) |
| `ImeEvent` | reducer の入力 | (reducer の状態は記録しない) | 観測イベント(`ObserverReported` 等)は awase が読んだ値 | 使えない(`ImeEvent` は `Serialize` のみ、`Instant` を持たない、リング外の初期状態が無い。`journal.rs:603-611` の doc) |
| `DriftPlanDecided`・`ShadowToggle`・`LateKeymap`・`LangDetect`・`PressWriteClaim` | 一部(理由・根拠) | plan・verdict | なし | 使えない(Facts が全部は無い。ADR-250 で「再生の入力にしない」) |
| `GjiFsmTransition`・`TsfProbeStarted/Completed`・`KeyInput`(state_before/after) | Debug 文字列 | Debug 文字列 | なし | 使えない(文字列。件数・遷移の地図には使える) |
| `LiteralDetect` | `LiteralDetectRecord`(facts・verdict・romaji・trace) | verdict | composition の観測 | 使えない(`Serialize` のみ)。romaji・trace は入力内容 |
| `SentInput`・`ImeOpenApplied`・`MozcShm`・`IndicatorGlyph` | — | — | awase が送ったキー・適用の outcome・Mozc Readable の共有メモリの読み・字形の読み | 効果側の証拠(再生の対象ではなく、写しの検査に使う) |

実 IME の「本当の結果」(入力欄に何の文字が出たか)はジャーナルに無い。CI では `typing_stress.log`・`result.txt` の読み戻し、
`check_*.py` の判定が持つ。ジャーナルが持つのは awase の決定と awase が読んだ観測(と OS API の戻り値)までである。

### (2) 再生はどこまで通るか

既存の再生はすべて**純粋な核の関数**だけを通す。殻(`runtime/`・`output/`・`tsf/` の配線、「その決定を呼ぶか」)は通らない。

| 再生 | 通す関数 | 入力 |
|---|---|---|
| `tests/journal_replay.rs` | `classify_conv_transition`、`ImeModel::reduce` | 手書き・転記の fixture |
| `tests/drift_correction_replay.rs`・`read_strategy_replay.rs` | drift correction の判定・`decide_read_strategy` | fixture |
| `state/actuation_decision_record.rs` の `#[cfg(test)]`(`replay_record`・`replay_chain_scan`) | `decide_gate`/`decide_chain`/`decide_attempt` と、記録した outcome を返す `ReplayWriter` で `run_chain` | 凍結コーパス 37 件(BUG-131 の報告) |
| `src/key_input_replay_tests.rs`(B5 の試作) | `Engine::on_input` | `KeyInput` 列(InputContext は固定値) |

### (3) 形式

- 不具合報告: `UnifiedJournal::report_json` の JSON 配列(`{seq, elapsed_ms, entry: {type: ...}}`)。打鍵・`SentInput`・`LiteralDetect` は直近 10 分。
  レーンはリング(State 2048・Timing 2048・Actuation 6144・KeyInput 8192 件)で、古いものは消える。報告では gzip+base64(ADR-222)。
- ホットキーのダンプ: 同じ形を `%TEMP%/awase_journal_<tick>.json` に。
- CI(e2e-ime): JSON は出ていない。`RUST_LOG=debug` の `dist/awase.log` に、記録のたびに `awase::journal` の debug 行が 1 行出る
  (`JournalEnvelope::emit_tracing`、`absorb`・`record_key_input` の 2 か所から)。この行は要約で、`ActuationDecision` は最初の attempt だけ、
  `ConvClassifyCall` は結果を持たない。artifact は `result-<構成>-<回>`(保持 7 日)。awase は `Stop-Process -Force` で止めるので、終了時のダンプは走らない。

### (4) 2 つのコミットの決定を比べる最小の形

**記録が出力を含むなら、基準版で記録したコーパスを候補版のコードで再生し、記録と食い違う件を数えればよい**(候補版だけをビルドする)。
既存の `replay_record` と `journal_replay.rs` はこの形(HEAD の再計算 = 記録)をしている。コーパスを基準版で取ることが前提で、
別のコミットで取ったコーパスを使うなら、そのコミットと基準版で再生する関数が同じこと(`git diff <取得> <基準> -- <関数のファイル>` が空)を先に確かめる。
記録が出力を持たない、または取得が基準版でないときは、同じコーパスを両方のコードで再生して出力を比べる(両方のビルドが要る)。

### (5) 限界(先に書く)

1. **決定の一致 ≠ 効果の一致。** 同じ決定でも、実 IME のタイミング・状態が違えば結果は変わる。ADR-226 の (iii) のとおり、BUG-162/163/170/171 は決定差分ゼロの側で起きた。
2. **記録されない決定は判定できない。** 撤去・変更が殻の判断(呼ぶか・設置するか・待つか)を変え、その判断の Facts がジャーナルに無ければ、再生は何も言わない。
   「再生が差分 0」と「その撤去が無害」は別の主張で、後者には該当の決定が記録されていることの確認が要る。
3. **決定関数が変わらない変更では、再生は自明に一致する。** 再生の差分 0 は、変更が再生対象の関数に触れたときだけ情報を持つ。
4. **コーパスに無い経路は分からない。** e2e の構成が通らない経路(実利用だけの経路、CI に無いアプリ)は、0 件でも到達しないとは言えない。
5. **コーパスの陳腐化。** 記録の型が変わると古いコーパスは読めない(過去の形式は持たない、ADR-217・ADR-250)。コミットごとに取り直す。
6. **リングと要約の欠け。** リングは古い記録を捨てる。`awase.log` の要約行からは再生できない。

## 決定

### 決定 1: コーパスは本番のジャーナルの JSON を、tracing の専用 target で 1 記録 1 行に出す(既定オフ)

- `JournalEnvelope::emit_tracing`(全記録が通る 1 か所)で、target `awase::journal_json` が TRACE で有効なときだけ、envelope を `serde_json` で 1 行に出す。
  既定の `RUST_LOG`(info・debug)では出ない。リングを通らないので消えず、`Stop-Process -Force` でも書いた分は残る。
- 殻(`app/bootstrap.rs`)は、環境変数 `AWASE_JOURNAL_JSON` がファイルを指すときだけ、この target の行をそのファイルへ追記で分けて書く
  (`MakeWriterExt::with_filter`・`or_else`)。`awase.log` を大きくしない(20MB のローテーションで `check_*.py` が読む前半を失わない)・
  `check_*.py` が読む行に混ぜないため。env も `RUST_LOG` も無い利用者の環境では何も変わらない。
- e2e-ime の `workflow_dispatch`・`workflow_call` に入力 `journal_json`(既定オフ)を足し、オンの run だけ上の 2 つを設定して `journal.jsonl` を `result-*` に入れる。
- 新しい journal の種類は足さない。

### 決定 2: コーパスの置き場

- 大きいもの(run 全体)は artifact のまま。コミット SHA は run が持つ。保持は 7 日なので、撤去の判定に使う run は、判定の文書に run ID と SHA を書く。
- リポジトリには小さな代表だけを入れる(`crates/awase-windows/tests/journals/corpus/` など、1 ファイル数百 KB 以内)。入れるのは決定 3 の再生対象の種類だけ。

### 決定 3: 再生は「記録が入力と出力の両方を持つ種類」だけ、純粋な関数を通して記録と比べる

- 対象(段階 1): `ConvClassifyCall`(`classify_conv_transition`)。`ActuationDecision`(`replay_record`)は crate 内テストなので、段階 1 では対象に入れるかを試験運転で決める。
- 比べるのは決定の出力だけ。不一致は「どの記録(構成・回・seq)で、記録と再計算がどう違うか」を全件出す。
- 2 コミットの比較は (4) の最小の形(基準版で取ったコーパスを候補版で再生)を既定にする。

### 決定 4: 入力内容の扱い

- CI のコーパスの入力は合成(`typing_stress` の乱数・固定の語)で、利用者の入力ではない。それでも、リポジトリに入れる代表からは
  `KeyInput`・`SentInput`・`LiteralDetect`・`FocusTransition`(プロセス名・クラス名)を除く(再生に使わず、大きい)。
- 不具合報告の journal はコーパスに入れない(ADR-225 P1)。報告を再生に使うときは手元だけで、結果の要約だけを文書に書く。

### 決定 5: 差分だけ実機

- 再生で食い違った決定は、その記録の構成(`result-<構成>-<回>`)を `only=` で狙って実機 CI を回す。
- 擬似 IME の全列挙(ADR-277 `TransitionTable::reachable`)が届くと言う状態のうち、コーパスの記録が踏んでいない状態を一覧にし、
  踏む構成があればそれを、無ければ「未検証の穴」として撤去の判定の文書に書く(段階 2)。
- 該当の決定が記録されていない撤去(限界 2)は、これまでどおりプローブ(目印のログ)を入れた実機 CI で到達を数える。本方式はそれを置き換えない。

## 何が検証できて、何ができないか

| 問い | 本方式で | 補足 |
|---|---|---|
| 候補の変更で、記録された決定(再生対象の関数)の出力が変わるか | できる | コーパスが踏んだ入力に限る |
| 候補の変更で、殻の配線・呼び出しの有無が変わるか | できない | 該当の Facts を記録していれば、その決定として再生できる |
| 候補の変更で、実 IME の結果(文字)が変わるか | できない | 実機 CI(差分だけ)。擬似 IME は写した範囲だけ |
| どの状態・遷移を CI が踏んだか | 段階 2 で一部 | 擬似 IME の抽象状態への写像が要る |
| 擬似 IME の予測が実機と合うか(写しの忠実度) | 段階 2 で一部 | ジャーナルに実 IME の観測(読み・共有メモリ・字形)がある記録だけ |

## 段階

| 段階 | 内容 | 合否 |
|---|---|---|
| 0 | 本 ADR(docs のみ) | — |
| 1 | 試験運転: (a) 決定 1 の JSON 行と e2e の入力、(b) コーパスを読んで `ConvClassifyCall` などを再生する Linux のテスト、(c) MB-02 の撤去(#803)を題材に、再生の結果をプローブの結果(到達 0 回)と突き合わせる | 再生が全件の結果を返すこと。MB-02 で限界 2・3 のどちらに当たるかを記録すること(「差分 0」を MB-02 の無害の証拠とは扱わない) |
| 2 | 擬似 IME のカバレッジの地図と写しの検査の最小形。大きくなるなら設計の追記だけ | 未定 |

`complexity-budget.md` の発効条件(TH1e)との関係: 段階 1 で `ActuationDecision` の再生がコーパスで動き、actuation の決定に触れる実際の削除・統合で差分 0 を示せたときに、
その条件の文言(「N 本の決定レコードの再生で差分ゼロ」)に当たる。殻だけの撤去(MB-02 など)では当たらない。

## 見送り

- 殻全体の再生(ADR-232 の「採らない案」、検討メモ B2)。
- `awase.log` の要約行からの復元(ADR-226 の B で不可と確認済み)。
- PR ごとの実機 CI の選択実行(ADR-226 の (ii)。PR では `sc-*` が走らない)。
