---
type: companion-doc
title: |-
  FCIS の境界の上で journal の取得場所とリプレイ基盤を作り直す案の検討(2026-10-06、検討のみ・コード変更なし)
---

# journal とリプレイ基盤の作り直しの検討

事実と出典(ファイル・行)は [inventory.md](inventory.md)。develop `777bf1db` 時点。所有者の前提は「過去の journal とリプレイ基盤はすべて捨ててよい」。

## 結論

- **新しい記録・再生の基盤は作らない。** FCIS で `decide` が純粋になっても、記録した入力から決定を再計算できるのは「決定関数が記録時から変わったか」までで、ADR-225 の指摘(F1: HEAD で症状が再現するかは判定できない)には答えられない。再生で退行を検知した記録は 0 件(inventory §7)。F2・F3・F5a は全数表で足り、F4 は `Instant` があるので fixture を作らなかった。再生の需要は実績から見て小さい。
- journal が実際に役立っているのは**不具合報告の診断**だけ(known-bugs で役立った 17 件、足りなかった 13 件)。作り直すなら、目的は診断に要る情報を保ったまま**記録の系統・入口・重複を減らす**ことにする。
- 推奨: 撤去だけの第 1 段階(消費者 0 の `shadow_send_trace`)→ F4 で済んだ drift の記録を `DriftPlan` 1 件にまとめる(E1 の 1 件目)→ F6 の後に journal の入口を 3 → 1 にする。各段階で純減させる。

## A. 用途の再確認

| 用途 | 実害の記録 | 今の仕組みで足りているか |
|---|---|---|
| 不具合報告からの原因の特定 | 役立った 17 件。足りなかった 13 件(切り詰め・出力文字が無い・起動直後しか無い) | おおむね足りている。切り詰めは ADR-222 で、出力文字は `SentInput` で対処済み。4 件では variant・フィールドを足して対処した |
| 報告からの再現(HEAD で症状が出るか) | — | **満たせない**。決定の再計算は症状を再現しない(ADR-225 F1)。FCIS でも変わらない |
| 回帰テスト | fixture 5 件(BUG-008・019・097・131・146)。うち 4 件は手で転記した期待値で、中身は単体テストと同じ | JSON である必要は無い。全数表・単体テストで足りる(F2・F3・F5a の実績) |
| CI の合否 | `check_*.py` は awase.log の断片 27 個を読む。journal 由来は 3〜4 個 | journal JSON は CI で使われていない。足りないという実害の記録も無い |
| 閉ループとの突き合わせ | 写しのずれの実例 0(ADR-224・P5a-1) | 閉ループは journal を読まない。必要性は未確認 |
| 撤去の材料(complexity-budget の TH1e) | 凍結コーパス 37 件。差分ゼロの証明は未達 | 発効条件の材料として残っている(E で所有者に確認) |

## B. FCIS の境界での記録点の候補

| 候補 | (1) 取得場所(今は記録する関数 26・ファイル 10・入口 3) | (2) 型・時刻・順序 | (3) 再生で分かること | (4) 行数 | (5) プライバシー | (6) Linux で試せる範囲 |
|---|---|---|---|---|---|---|
| B1 すべての `decide` の Facts と Plan を記録して再生 | `decide` の呼び出し点ごとに増える(F1〜F5a で +5 以上)。既存の variant を消さないなら純増 | Facts に serde が要る(今あるのは F1 だけ)。`DriftFacts` は `Instant` を含む(V4 で `TickMs` 化が要る) | 決定関数の差分だけ。症状の再現は不可(F1)。分割の PR では Facts の型そのものが新しいので、その PR の「挙動不変」は過去の記録では検証できない | 増える。撤去先は見当たらない | Facts は入力文字を含まない | 全部 |
| B2 殻の観測(Observe)を境界の 1 か所で記録し、核全体を再生 | 理屈では 1 か所。実際には `KeyInput` が処理後の要約で生入力を持たず、Event を出さない hub のメソッドが 27、`Runtime` の 40 フィールドは reduce を通らない(W0) | 時刻の入口が 3 つ。`Instant` が要る | 全体の再生。ただし前提の全書き込みの reduce 化が必要 | 大きく増える | 生入力を記録することになり、ADR-095 と衝突 | 全部(前提が満たせれば) |
| B3 実行(Execute)の結果を記録 | 既にある(`SentInput`・`ImeOpenApplied`・`ActuationDecision` の outcome) | 既存の envelope で足りる | 再生はしない。診断用 | 0 | `SentInput` は 10 分に絞り済み | — |
| **B4 1 回の判断につき殻で 1 レコード(理由つきの Plan)に寄せる** | Plan ができたサンドイッチから、重複する variant を Plan 1 件に置き換える。入口は F6 の後に 3 → 1 | Plan は serde(`Serialize` だけでよい。読み戻さない)。時刻は既存の envelope | 再生しない。理由の enum で「なぜ省略したか」を診断で読める(E1 と同じ) | 段階ごとに純減(D) | Plan は入力文字を含まない | Plan の組み立ては全部 |

ADR-225 F1 への答え: **答えられない。** 純粋化で再計算は安く確実になるが、分かるのは関数の差分だけで、症状は殻と IME のやり取りから出る。B4 は再生を目標にしないので、この問題を前提にしない。

## C. 却下済みの案との照合

- 報告 journal の自動 fixture 化(ADR-225): 提案しない。凍結コーパスを増やす案も出さない。
- 共通シナリオの形式・共通 oracle・選択実行・遅延の較正(ADR-225 S2、ADR-226 A〜D): 提案しない。
- journal からの完全再生・全書き込みの reduce 化(ADR-232「採らない案」): B2 がこれに当たるので推奨しない。
- DSL・宣言テーブル・汎用ツールキット(ADR-218〜220、toolkit round1): B4 は共通の trait・Plan の型を作らない。各 `decide` の既存の Plan を記録に流用するだけ。
- ADR-226 候補 E(チェッカーを journal の JSONL で読む)は未レビュー。本検討では決めない。ただし、チェッカーが読む断片 27 個のうち journal 由来は 3〜4 個なので、E を進めるには約 23 個の事実を journal に移す必要があり、journal は増える。

## D. 推奨案と段階

推奨は B4。各段階は挙動を変えず、CI だけで判定する(ローカルでビルドしない)。

| 段階 | 内容 | 撤去するもの | 足すもの | 検証 | 取りやめ条件 |
|---|---|---|---|---|---|
| 1(今すぐ可) | `shadow_send_trace` を消す | `src/shadow_send_trace.rs` 62 行と呼び出し 2 か所。送信内容は `SentInput`(SendInput)と既存の `[ime-io]` 行に残る | 0 | windows-cross-check・windows-build が green。`test_log_anchors_in_rust_source.py` が green(`[shadow-send]` は表に無い) | `[shadow-send]` を読む消費者が見つかる。または所有者が TF2(ADR-159 段階2)を残すと決める |
| 2(F4 済み) | drift correction の `ImeActuation`(2 か所)を、`DriftPlan` の記録 1 件に置き換える(E1 の 1 件目) | `JournalEntry::ImeActuation`、`ActuationRecord`(使い手は journal・replay・`journal.rs` のテストだけ)、`DriftCorrectionFixture`、`tests/drift_correction_replay.rs` 210 行、fixture 1 本、`emit_tracing` の腕 | `DriftPlan` 系の `Serialize`、variant 1 つ(差し引き約 −200 行の見込み。実装の PR で実測する) | 前提: BUG-43 の「試行が有界」を `decide_drift_plan` の全数表か `FeedbackPolicy` の単体テストが既に固定していることを着手前に確認する(**未確認**)。known-bugs で `ImeActuation` に言及する 3 件(BUG-068・110・142)が使った情報(試行回数・方針・打ち切り)が `DriftPlan` から読めること | 上の前提が成り立たず、テストを足すと純減にならない |
| 3(F6 の後) | journal の入口を 3 → 1 にする。`pending_journal_entries`(platform)と `SENT_INPUT_TRACE`(win32)の中継を、F6 で Output が返す `Vec<Cmd>`/結果に畳む | 中継の 2 つのバッファと `install_sent_input_stamp_source` | 0 の見込み | 発行時刻での採番(ADR-096 B-4)が保たれること。`architecture_guard` の output/tsf → journal 禁止が残ること | F6 で Output が記録を返す形にならない |

推奨しないもの: B1(実績から見て需要が小さく、純増で、F1 に答えない)、B2(ADR-232 で却下済み、ADR-095 と衝突)、新しい再生の crate・形式。手で組んだ JSON fixture(5 ファイル・17 件)を単体テストに移すことも、実害が無いので「次にその核を触るときに」とする。

## E. 所有者に聞くこと

1. 「すべて捨ててよい」に凍結コーパス(`tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`、37 件)を含めるか。complexity-budget.md の発効条件(TH1e)はこのコーパスでの再生を前提にしているので、捨てるなら発効条件を書き直す必要がある。推奨は残す(B4 はこれに触れない)。
2. 段階 1 で `shadow_send_trace` を消し、ADR-159 段階2(TF2)の「将来の突き合わせ」を閉じてよいか。
3. 報告との互換: report-worker は journal を解凍せず中身と結合していないので、variant を変えても worker の変更は要らない。ただし `bug-report-fetch` スキルの読み方は variant 名に依存する。古い版の報告の journal を新しい手順で読めなくてよいか(ADR-217 の「旧形式パーサを持たない」と同じ扱いでよいか)。
4. 段階 2 を E1 の 1 件目として扱ってよいか(タスク表では E1 は「書き方の明文化と V4 の後」)。

## F. 前提条件

- 段階 1: 前提なし。
- 段階 2: F4(#514)はマージ済み。タスク表の順序(書き方の明文化を先に)に従うなら、その後。
- 段階 3: F6(未着手)の完了が前提。
- 「代数的 effect の完成」は計画上の到達点として存在しない(Plan/Effect の項の設計は ADR-229 で却下、E2〜E6 は待つ条件つき)。本案はそれを待たずに、F の分割が済んだ核から順に進める。

## 確認できなかったこと

- known-bugs 45 件の分類はサブエージェントの一次分類で、精読は抜き取りの 5 件だけ。
- 段階 2 の前提(BUG-43 の有界性が既存テストで固定されているか)と、差し引きの行数。
- `[shadow-send]` がリポジトリの外(個人のスクリプト等)で読まれているか。
