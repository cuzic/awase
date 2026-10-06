---
type: companion-doc
title: |-
  FCIS の境界の上で journal の取得場所とリプレイ基盤を作り直す案の検討(2026-10-06、検討のみ・コード変更なし)
---

# journal とリプレイ基盤の作り直しの検討

事実と出典は [inventory.md](inventory.md)、ログと journal の重複は [log-journal-duplication.md](log-journal-duplication.md)。
最初の棚卸しは develop `777bf1db`、Opus レビュー(round1・2)の指摘の裏取りは `1732f830`(#522 マージ後)で行った。

**所有者の前提**: (1) 過去の journal とリプレイ基盤はすべて捨ててよい。(2) 障害対応と replay のため、文字キーを含む VK 列を記録に残すのは必須。利用者が不具合報告の操作で共有する記録(サーバーへ)にはプライバシーの制約を置かない。本メモは VK 列を減らす・捨てる案を出さない(下の段階のどれも `KeyInput`・`SentInput`・`LiteralDetect` を減らさない)。公開リポジトリ・公開 issue に入力が出る経路は別に扱う(G 節)。

## 結論

- **記録側**: 記録の系統と重複を減らす。先にログ(awase.log)と journal の重複を整理し(所有者の優先。log-journal-duplication.md)、続いて B4(1 回の判断につき殻で 1 レコード)を進める。診断の主経路は awase.log(known-bugs で journal かログに触れる 96 件のうち、ログだけのものが 51 件)なので、journal だけを作り直しても診断は良くならない。
- **再生側**: 初版の「新しい再生の基盤は作らない」は撤回する。初版は「決定の再計算」(記録した Facts から `decide` を再計算)と「入力の再生」(記録した物理入力の列を HEAD の分類器とエンジンに流す)を区別していなかった。
  - **決定の再計算**は、ADR-225 F1 のとおり「決定関数が記録時から変わったか」しか分からない。用途はリファクタの回帰網(タスク表 `:71`・`:102` が `open_chain` など最後の分割で予定)で、ここは需要がある。
  - **入力の再生**(B5)は、**エンジン側の不具合なら F1(HEAD で症状が出るか)に答えられる**。実例は BUG-105(報告の打鍵間隔 d1=11.7ms・d2=100.8ms をエンジンのテストで再現)と BUG-145(実機ログの押下間隔 90.1〜108.8ms をエンジンのテストで再現)。どちらも人がタイミングを読んで手でテストにしたもので、道具は無い。IME とのやり取りが絡む不具合には効かない。
- **推奨**: 記録側の整理(重複の整理 → B4)と、B5 の最小形を組み合わせる。B5 の最小形は、`[engine-input]` の行にしか無い InputContext(ime_on・input_mode・japanese・composing)と拡張ビットを `KeyInput` に移し(これがログと journal の重複の解消にもなる)、テスト側の変換関数 1 つで報告の `KeyInput` 列を `Engine::on_input` に流す。新しい crate・形式は作らない。B6(閉ループへの注入)は目的の優先順位(E1)を聞いてから。

## A. 用途の再確認

| 用途 | 実害の記録 | 今の仕組み |
|---|---|---|
| 不具合報告からの原因の特定 | known-bugs 183 件中、journal かログ(awase.log・実機ログ・app_log)に触れるもの 96 件。journal に触れる 45 件の一次分類では役立った 17・足りなかった 13(inventory §7) | ログが主、journal が従。重複がある(log-journal-duplication.md) |
| 報告から HEAD での再現 | エンジン側: BUG-105・145 は報告・実機ログのタイミングから手でエンジンのテストを作った。IME 側: 閉ループに手で写した例(BUG-162 など、`closed_loop_scenarios.rs:4-130`) | **道具が無い**。エンジン側は B5 で、IME 側は B6 で、写せる範囲でだけ答えられる |
| リファクタの回帰網 | RW(#499)で凍結コーパスのチェーン走査を再生。タスク表 `:71`・`:102` が最後の分割(`open_chain`・`on_focus_process_changed`・`monitor_loop`)で「journal replay で回帰網を先に張る」と予定 | 決定の再計算の用途。F2・F3・F5a は全数表で足りたが、入力空間が小さいものを先に分けた結果でもある |
| 回帰テスト | fixture 5 件(BUG-008・019・097・131・146)。うち 4 件は手で転記した期待値 | 全数表・単体テストでも書ける |
| CI の合否 | `check_*.py` は awase.log の断片 27 個を読む。journal 由来は 4 個 | CI は JSON journal をダンプしない(E5) |
| 閉ループとの突き合わせ | 写しのずれの実例 0(ADR-224・P5a-1) | 閉ループは journal を読まない |

退行の検知の記録が見つからないこと(inventory §7)は、需要が小さい根拠にはしない。回帰網は手元で落ちて直ればコミットに痕跡が残らず、全数表・golden が退行を捕まえた件数との比較もしていない。

## B. 作り直す場合の候補

| 候補 | (1) 取得場所(今は記録する関数 26・ファイル 10・中継の入口 3) | (2) 型・時刻・順序 | (3) 再生で分かること | (4) 行数 | (5) 公開リポジトリ・公開 CI ログへの露出 | (6) Linux で試せる範囲 |
|---|---|---|---|---|---|---|
| B1 すべての `decide` の Facts と Plan を記録して再計算 | `decide` の呼び出し点ごとに増える(F1〜F5a で +5 以上) | Facts に serde(今は F1 だけ)。`DriftFacts` は `Instant`(V4 で `TickMs` 化) | 決定関数の差分。症状は分からない(F1)。分割の PR では Facts の型が新しいので、その PR の挙動不変は過去の記録で検証できない | 増える | Facts は VK 列を含まない | 全部 |
| B2 殻の観測を境界で全部記録し、核全体を再生 | 理屈では 1 か所。実際は Event を出さない hub のメソッド 27、`Runtime` の 40 フィールドが reduce を通らない(W0) | 時刻の入口 3 つ。`Instant` が要る | 全体の再生(前提の全書き込みの reduce 化が要る) | 大きく増える | 生入力を含む。報告の共有では制約にならない | 前提が満たせれば全部 |
| B3 実行の結果を記録 | 既にある(`SentInput`・`ImeOpenApplied`・`ActuationDecision`) | 既存の envelope | 再生しない。診断用 | 0 | `SentInput` は VK 列 | — |
| **B4 1 回の判断につき殻で 1 レコード** | 重複する記録を 1 件に寄せる。中継の入口は F6 の後に 3 → 1 | 既存の envelope | 再生しない。理由の enum で診断(E1 と同じ) | 段階ごとに純減(D) | VK 列には触れない | 全部 |
| **B5 `KeyInput` 列を HEAD の分類器と `Engine::on_input` に流す(エンジン側の不具合)** | 記録は今の `KeyInput`(`kp_run_inner`、1 か所)。足りないもの: InputContext の 4 項目(今は `[engine-input]` の行 `key_pipeline.rs:153-157` にだけある)、拡張ビット、Alt なりすまし前の vk、畳み込まれたオートリピート(`journal.rs:1373`) | `timestamp_us`(フックの時刻)で順序とタイミングを持つ。親指の押下時刻は前のイベントから導ける | **エンジン側の不具合なら HEAD で症状が出るか**。IME とのやり取り・defer・warmup が絡むものは分からない | 記録側は既存フィールドの追加(`[engine-input]` の行の撤去と相殺)。変換関数とテスト補助で数十〜百数十行の見込み(未実測) | 報告から作った再現列をテストに書くと公開リポジトリに入る。BUG-105・145 の既存テストと同じ形(G 節) | 分類器 `classify_key`(`hook.rs:187`)は純粋だが `#[cfg(windows)]` の `hook.rs` にある。`HookConfig`(親指キー)と配列(yab)が要る。報告に `config_toml`・`layout_yab` が添付されていれば使える(添付は任意) |
| B6 報告の入力列と観測の列を閉ループの擬似 IME へ流す | 閉ループの入口(`tests/support/harness.rs`)に、手書きの代わりに報告から変換した列を入れる | 擬似 IME の時刻は仮想時計 | クセの目録(QUIRKS)が写した範囲でだけ症状が出る(BUG-162 の再現、`4298f077` の Q4 の実績) | 変換と、写していない入力の扱いで増える(未実測) | 同上 | 全部(閉ループは Linux) |

ADR-225 F1 への答え: **IME とのやり取りが絡む不具合には、決定の再計算でも入力の再生でも答えられない(B6 で、クセを写した範囲だけ)。エンジン単体の不具合には、入力の再生(B5)で答えられる。** FCIS で `decide` が純粋になったことは、決定の再計算を安くするが、この区別は変えない。

## C. 却下済みの案との照合

- 報告 journal の自動 fixture 化(ADR-225): 見送り理由のうちプライバシー(P1)は、前提 (2) で報告の共有については理由にならない。F1 はエンジン側には当たらない(BUG-105・145)。P2(バグの出力を固定して修正で落ちる)は、人が期待値を書けば避けられる(ADR-225 決定4 の S1' と同じ)。よって、**エンジン側に限り、人が数打鍵に縮めて期待値を書く形の B5 は、ADR-225 の見送り理由のどれにも当たらない**。全レーンの自動 fixture 化(人が期待値を書かない形)は引き続き提案しない。
- 共通シナリオの形式・共通 oracle・選択実行・遅延の較正(ADR-225 S2、ADR-226 A〜D): 提案しない。B6 は閉ループの既存の入口に入力を入れるだけで、oracle を共通にしない。
- journal からの完全再生(ADR-232「採らない案」): B2 がこれに当たるので推奨しない。
- DSL・宣言テーブル・汎用ツールキット: B4・B5 とも共通の trait・形式を作らない。

## D. 推奨案と段階

各段階は挙動を変えず、CI だけで判定する(ローカルでビルドしない)。

| 段階 | 内容 | 撤去 | 追加 | 検証 | 取りやめ条件 |
|---|---|---|---|---|---|
| 0 | ログと journal の重複の整理(log-journal-duplication.md の第 1 段階) | 同文書 | 同文書 | 同文書 | 同文書 |
| 1(一時停止) | `shadow_send_trace` を消す | `src/shadow_send_trace.rs` 62 行と呼び出し 2 か所 | ImeControl の `lparam` を `[ime-io] cross_process` の行(`imm.rs:279-283`)に足す(フィールド 1 つ)。lparam(開閉の向き・conv の値)を残しているのは今は `shadow_send_trace::record_ime_control`(`imm.rs:286-287`)だけ | windows-cross-check・windows-build が green | 初版の「消費者 0」は誤り。コードの読み手は 0 だが、`docs/ime-passive-model-expected-results.md` が `[shadow-send]` を INV-3/4/5 の書き込み数の判定の基礎にしている(:163・:185・:193・:219・:229)。この判定を作る予定があるなら撤去しない(E4) |
| 2 | drift correction の replay を消す(`ImeActuation` の記録自体は残す) | `DriftCorrectionFixture`/`DriftCorrectionTick`(doc 込み約 67 行、`state/ime_actuation.rs:293-359`)、`tests/drift_correction_replay.rs` 210 行、fixture 30 行、`ci.yml:51` の `--test drift_correction_replay`、fixture のためだけの `Deserialize`(関連の doc の書き換えを含む)。合計 約 310 行 | 0 | BUG-43 の「試行が有界」は `state/drift_plan.rs:472`(max 以上で Send に戻らない)と `:601`(全数表で Blind は `attempts < MAX`)が既に固定している(確認済み)。CI が green | `drift_correction_replay.rs` だけが固定している性質が見つかる |
| 3(F6 の後) | 中継の入口を 3 → 1(`pending_journal_entries`・`SENT_INPUT_TRACE` を F6 の戻り値に畳む。フックの診断キュー〈`hook.rs:1393`〉も対象) | 中継のバッファ | 0 の見込み | 発行時刻での採番(ADR-096 B-4)が保たれること | F6 で Output が記録を返す形にならない |
| 4(B5 の最小形) | `KeyInput`(`KeyEventSummary`)に InputContext の 4 項目と拡張ビットを足し、`[engine-input]` の行の同じ項目を消す。テスト側に「`KeyInput` 列 → `RawKeyEvent` 列」の変換関数を 1 つ置く | `[engine-input]` の行の重複項目(チェッカー `check_startup`・`check_drift_recovery*`・`check_keymatrix` の書き換えを伴う。log-journal-duplication.md の #5) | フィールド 5 つ前後と変換関数 | BUG-105 と BUG-145 の既存のエンジンのテスト(`src/engine/tests.rs`)の入力を `KeyInput` 列の形で書き直し、変換関数経由でも同じ結果になること | 変換に、分類器の外の殻の処理(defer・Alt なりすまし・フックの再判定)の写しが要ると分かる(写しを増やすことになる) |

段階 2 は初版では `ImeActuation` を `DriftPlan` に置き換える案だったが、`DriftAct` の `ActuationSnapshot` が `Instant`(`sent_at`・`gave_up_at`)を持つので、そのままは記録できず、要約を作ると今の `ActuationRecord` と同じものになる。そのため variant は残し、replay だけを消す形に改めた。variant を残すので、`tests/architecture_guard.rs:3485` の BUG-163 の順序ガード(`"JournalEntry::ImeActuation"` を目印に使う)は変わらない。将来 variant を消すときは、このガードの目印を付け替える。

推奨しないもの: B1(用途は回帰網だけで、タスク表の最後の分割の時点で、その分割の Facts に限って作れば足りる)、B2(ADR-232 で却下済み、前提が大きい)、新しい再生の crate・形式。

## E. 所有者に聞くこと

1. 作り直しの目的の優先順位: (a) 報告から HEAD での再現テストを作る(B5・B6)、(b) FCIS の残りの分割の回帰網(決定の再計算)、(c) 診断の情報量(記録側の整理)。どれを取るかで段階 4 と B6 の要否が変わる。
2. 凍結コーパス(`bug-131-report-01m29kdnz.json`、37 件)を残すか。残す費用の実績: #520 は死んだ variant を消すためにコーパスの `caller` 15 件を書き換えた。RW(#499)は再生のために 257 行を足した。捨てる場合は complexity-budget.md の発効条件(TH1e)を書き直す。
3. 報告の保存期間と古い報告: R2 にある過去の報告の journal を、新しい形式で再生・診断できる必要があるか(ADR-225 P3・ADR-217 の「旧形式パーサを持たない」と同じ扱いでよいか)。report-worker は journal を解凍せず中身と結合していないので、worker の変更は要らない。`bug-report-fetch` スキルの読み方は variant 名に依存する。
4. `docs/ime-passive-model-expected-results.md` の INV-3/4/5 の書き込み数の判定を作る予定があるか。無ければ段階 1 を再開する(lparam は `[ime-io]` の行に移す)。
5. CI で JSON journal をダンプして成果物にするか(今は awase.log だけ。ADR-225 F3)。B5・B6 を CI の実機の結果から作るなら要る。
6. 報告に載る入力の範囲: journal の `KeyInput`・`SentInput`・`LiteralDetect` は報告時に直近 10 分に絞っている(`journal.rs:1519-1545`)。ただし awase.log の `[key-output] KeyInput(batched|tsf): romaji=…`(`output/vk_send.rs:228`・`:406`、**info**)は利用者の既定ログに打鍵ごとに残り、報告の `app_log_excerpt_gz` は awase.log の末尾を圧縮前で最大 16MiB 載せる(`bug_report.rs:25`)。つまり**入力内容は今も 10 分では絞れていない**。journal の窓を外しても新たに出るのは構造化された vk/scan とタイミングだけ。窓を外して B5 の入力を長く取るか。
7. B5 を望むか。望むなら、報告 1 件から数打鍵に縮めた再現列を(BUG-105・145 と同じく)公開リポジトリのテストに書く運用でよいか(G 節)。
8. G 節の判断(手で貼る経路の運用)。

## F. 前提条件

- 段階 0・2: 前提なし。段階 1: E4 の回答。段階 3: F6(未着手)。段階 4: E1・E7 の回答。変換関数は `classify_key` を使うので、`hook.rs` からの分類器の切り出し(ADR-229 段階 H「フックの薄型化」の一部)か、テスト側からの呼び出しの工夫が要る(どちらが小さいかは未確認)。
- 「代数的 effect の完成」は計画上の到達点として存在しない(Plan/Effect の項の設計は ADR-229 で却下、E2〜E6 は待つ条件つき)。待たずに、前提が揃った段階から進める。

## G. 公開リポジトリ・公開 issue への実利用者の入力の扱い(所有者の判断事項)

経路は 3 つある。
(a) 利用者が報告の操作でサーバーへ送る記録: 前提 (2) のとおり VK 列を含めてよい。
(b) 自動の経路(fixture・CI のログ): いまは無い。実報告由来の fixture は `bug-131-report-01m29kdnz.json` だけで、利用者の文字キーの VK は含まない(含むのは awase が送った IME 制御キー `SendVk` 22〈0x16 `VK_IME_ON`〉・26〈0x1A `VK_IME_OFF`〉だけ)。実機 CI の入力は合成。
(c) **診断の過程で、known-bugs・ADR・GitHub issue・PR 本文に報告やログの抜粋を手で貼る経路**: 既にあり、実例がある。BUG-105 は report `01M1GDQVBET5DBX3MY4BRGQFW1` から、入力しようとした文(「しょうにん」)・出た文(「しいゔにん」)・打鍵の並びを本文に書き、公開の GitHub issue #140 として起票した(`BUG-105.md:15-19`)。BUG-050 は利用者の報告のログ抜粋に `romaji="la"`・`romaji="ki"` を含む(`BUG-050.md:20,27`)。量は 1〜数文字の断片。
判断が要るのは (c) のいまの運用(このまま許容するか、報告由来の入力文は要約して載せるか、同意の文言を報告画面に足すか)と、B5 で再現列をテストに書く場合の扱い。

## 確認できなかったこと

- known-bugs の分類: journal に触れる 45 件はサブエージェントの一次分類(抜き取り 5 件で照合)。ログだけに触れる 51 件と、エンジン側の不具合の件数は数えていない(`src/engine` に触れる BUG は粗い grep で 27 件だが、エンジン単体の入力列で再現できる種類かは精査していない)。
- B5 の変換関数と B6 の変換の行数。`classify_key` を Linux のテストから使う最小の方法。
- `docs/ime-passive-model-expected-results.md` の INV-3/4/5 の判定の計画が今も生きているか(2026-09-24 の文書)。
- `[shadow-send]` をリポジトリの外で読むものがあるか。
