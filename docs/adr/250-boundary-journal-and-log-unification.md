---
id: ADR-250
title: |-
  journal を shell と core の境界で取り(核が受け取った Facts と返した Plan)、手書きログとの二重を収支が純減になる組から減らす
summary: |-
  いまの journal は 21 種類のエントリを殻の約 20 か所から手で書いており、境界の記録になっていない。
  記録の単位を「核の決定関数が受け取った Facts」と「返した Plan(理由つき)」にし、殻が境界で記録する(核は journal に触らない)。
  ログとの統合は ADR-241 の検討(2026-10-06)の (c)「事象ごとに寄せる」に従い、収支が純減になる組から進める。
  再生の範囲は「Engine::on_input まで」と「核の決定関数 1 つずつ(Facts → Plan)」に絞り、端から端までの状態の再構築は目標にしない。
  過去の journal・コーパスとの互換性は持たない(所有者判断 2026-10-10)。ADR-226 の候補 E にあたる。
status: |-
  提案(起草中、Opus レビュー round1 反映済み・round2 待ち)
related_adr:
  - "ADR-082"
  - "ADR-096"
  - "ADR-132"
  - "ADR-139"
  - "ADR-163"
  - "ADR-169"
  - "ADR-222"
  - "ADR-225"
  - "ADR-226"
  - "ADR-229"
  - "ADR-241"
---

# ADR-250: journal を shell と core の境界で取り、手書きログとの二重を減らす

調べた版は develop `2ccf4c7e`(#576 マージ後)。ローカルでのビルド・テストはしていない。
件数は 2026-10-10 に `git grep` と規則による機械分類で数えた値で、全件の手作業レビューはしていない
(分類の境界は ±数十件)。Opus round1 の裏取りで見つかった誤りは本文に反映済み(末尾の「round1 の反映先」)。
集計の範囲は `crates/awase-windows` と `crates/awase-windows-core` で、root の `awase`(`src/`、tracing 約 32 行)は含まない。

## 用語

- **境界**: 殻(`crates/awase-windows`、Win32 を呼ぶ側)と核(`crates/awase-windows-core` と root の `awase`、純粋な判断)の間。ADR-229 の FCIS の語に従う。
- **Facts**: 核の決定関数が受け取る引数(`SetOpenFacts`・`DecisionInputs`・`ImeSnapshot`・`classify_*` の引数など)。殻が OS から読んだ値、問い合わせ時刻(`now`)を含む。殻が嘘をつく読みは、その値に既に含まれる。
- **Plan**: 核の決定関数が返す理由つきの結果(`SetOpenPlan`・`DriftPlan`・`NoDrift`・`BlockReason`・`PressClaim`・`GateResult` など。`*_plan.rs` は 10 ファイル)。送る・送らない(**見送り**)のどちらも、理由の enum を持つ。
- **内部診断(C)**: 境界を通らない殻の出来事(Win32 の失敗、起動・終了、設定読み込み、プローブ内部)。

## 背景(実測、Opus round1 で `096dcff5` に対し裏取り済み)

### 1. いまの journal は境界で取っていない

- `JournalEntry`(`crates/awase-windows-core/src/journal.rs:261`)は 21 variant。`emit_tracing` の doc(`journal.rs:808`)は「19 variant」のまま古い。
- 構築は `journal.rs` の単体テストを除いてすべて殻の crate。上位は `platform.rs` 20、`state/platform_state.rs` 9、`runtime/key_pipeline.rs` 7、`runtime/ime_refresh.rs` 7、
  `runtime/message_handlers.rs` 6、`output/mod.rs` 6(`git grep -c "journal::\|JournalEntry" -- crates/awase-windows/src`)。
  - 例外の前例: `state/sync_actuation.rs:341,386` は ADR-241 決定 2 で「核」と定義された `dispatch_set_open` の中にあり、核が `hub.journal.record(..)` で自分で書いている。
- 入力と結果が 1 件に混ざる variant: `KeyInput`・`TimerFired`・`ConvClassifyCall`・`ActuationDecision`。
- `ImeEvent` から belief を再構築できない(`journal.rs:289-296`): `Instant` を記録しない、`Deserialize` が無い、ring から落ちた古い分の初期状態の snapshot が無い、`reduce` を通らない書き込みは記録されない。
  ADR-229 W0 の実測では、世界モデルの書き込みが `reduce` を通る部分は小さい(`ImeStateHub` のメソッド 53 個中 Event を出さないものが 27、`Runtime` の 40 フィールドは 0)。
- 診断的な variant 6 種(`DriftGiveUpDiagnostic`・`HookImeModeDiagnostic`・`DriftGiveUpIntervalEnded`・`TsfProbeStarted`・`TsfProbeCompleted`・`SentInput`)には、**機械の読み手は無い**。
  ただし人と Claude の不具合診断が読んでいる: `TsfProbeStarted.pending_deferred_len` は `BUG-074.md:223-225`、`SentInput` は `docs/bug-reports-triage.md:148`(PR #468 で LINE「いまは→いいい」報告のために足した)、
  `DriftGiveUpDiagnostic`/`HookImeModeDiagnostic` は `BUG-110.md:45`。**`SentInput` は送信そのもの(出力)で、診断として消せない。**
- コーパス: `crates/awase-windows/tests/journals/` は 5 種(`conv_classify`・`drift_correction`・`ime_apply`・`key_input`・`read_strategy`)。
  `key_input/bug-105-tight-d1.json` は**合成**(同 `README.md`: `state_*`・`decision`・`physical`・`seq`・`elapsed_ms` は推定で埋めた)。
  `actuation_decision/bug-131-*.json` は `crates/awase-windows-core/tests/journals/` にあり、`JournalEnvelope` ではなく `ActuationDecisionRecord` の配列。
  `conv_classify`・`drift_correction`・`ime_apply` は手で書き写した別形式。**実機ダンプの `JournalEntry` JSON をそのまま使うコーパスは無い。**

### 2. ログと journal の二重

- 手書きの tracing 呼び出しは `tracing::{level}!(` で 728 件(`git grep -ohE "tracing::(trace|debug|info|warn|error)!\s*\(" -- crates/awase-windows/src crates/awase-windows-core/src | wc -l`)。
  `emit_tracing` の 21 件を除く 707 件を機械分類した(2026-10-10): A(入力)143、B(出力)288、C(内部診断)201、D(journal と二重に見える)73。D は下限ではなく**「二重らしく見える行」の目安**で、B と重なる(下記)。
  40 件の抜き取りで B の比率は合った(約 42%)。
- `emit_tracing` は全 variant を `target: "awase::journal"` の **debug** ログへ複製する(ADR-139 決定 4)。約 10% の複製(ADR-222)は `RUST_LOG=debug` の環境の値で、利用者の既定 `info` では 0%。
- D の最大は `[gji-fsm]`(約 35 件)だが、その多くは「… ignored」「… のため無視」「discarding N pending」という**見送りの理由**で、`GjiFsmTransition`(trigger・state_before・state_after)に理由は載らない。
  消すなら理由を型に足す必要があり、純減にならない。
- 手書きログの構造化はほぼ無い(フィールド付きは `runtime/open_chain.rs:208` の 1 件)。75.6% は format 文字列への埋め込み。

### 3. 見送りの理由の記録は、半分は既にある

- 型で既に残っているもの: 押下台帳の二重(`PressWriteClaim { verdict: "duplicate" }`、`state/platform_state.rs:235,250`。`[shadow-toggle] 同じ押下で既に書いた` に対応)、gate の NotOwned(`ActuationDecision`、`chain_len: 0`、`sync_actuation.rs:327-341`)、
  `ImeActuation.action = GiveUp`、`GiveUpFollow.outcome`、`DeferredRecoveryFlush` の discard、`ImeOpenApplied.outcome = Unwarranted`。
- 抜けているもの: warmup・TSF gate・focus・shift-conv-guard・key-effect 系。例: `[tsf-gate] held queue full`(`tsf/tsf_gate.rs:299`)、`[do-transmit] gate=Bypass, skipping`(`output/probe_io.rs:423`)、`[shift-conv-guard] … スキップ`(`runtime/key_pipeline.rs:2268`)、`[key-effect-predict] no prediction`。
- 抜けている見送りのほとんどは**殻のファイル**にある(`skip|スキップ|無視|ignored|見送|held|fence|gate` を含む tracing の粗い数え: `runtime/` 59、`tsf/` 42、`output/` 18、core 11、`state/`〈殻の crate〉3)。
  `tsf/`・`output/` は `crate::journal` の参照を禁じられている(`architecture_guard.rs:403`)。ADR-229 の段階 5(`F` の分割)が済むまで、これらを核の Plan にはできない。

### 4. 読み手(消してはいけないもの)

- **人と Claude の不具合診断**が JSON と生成ログの主な読み手(`/bug-report-fetch`、`docs/bug-reports-triage.md`)。JSON 本体を読む機械は、再生テスト(`key_input_replay_tests.rs`)、`ActuationDecision` のコーパス、不具合報告の整形(`awase-settings`)だけ。
- ログ行を機械が読む例(`ANCHORS` は 32 件だが**台帳としては足りない**):
  - フィールドまで読む: `check_consistency.py:58`(`key input seq=… vk_code=75 is_down=true … decision="…"`)、`check.py:76`・`check_invariants.py:57`(`outcome="Unwarranted"`)、`check_run_validity.py:39`(`profile="…"`)。
    `test_log_anchors_in_rust_source.py` は文言しか見ない(doc に明記)ので、フィールドの消失を検出しない。
  - 手書きの行: `Blacklist drift correction`(`check_drift_correction.py:28`、`check_drift_recovery.py:65`、`check_drift_recovery_chrome.py:5`)、`[mode-key-follow]`(`mode_key_pass_timeline.py`)、`Engine activated/deactivated`(`effect_learning.py`)、
    `e2e-ime.yml:1285-1286` の Select-String、`e2e-uwp-inputsite-hook-watchdog-probe.yml:75,102`、`tools/e2e/config_verify/run.py`、`tools/e2e/ime_key_matrix/testdata/*.awase.log`(22 本)。
  - `architecture_guard.rs`: `:2498` の `DRIFT_SEND_LOG_MARKER = "[drift] correction: observed="`(ログ文言を順序ガードの目印にしている)、`:403`(output/tsf は journal を参照しない)、`:6123`(`emit_tracing` に `?`/`%`/ワイルドカード禁止)、`:6167`(`KeyInput` の構築は 1 か所)、`hook_callback` のログマクロ数 7 件。
- 設定画面の不具合報告プレビューは、`"type":"KeyInput"` の文字列一致で「打鍵の行をすべて削除」する(`crates/awase-settings/src/bug_report.rs:947-954`)。ADR-222 で所有者が決めた約束。`SentInput`(`ch` を持つ)と `LiteralDetect` は削除対象外のまま。

### 5. 過去の検討との関係

- 2026-10-06 の検討(`docs/tasks/journal-replay-rebuild-study-2026-10-06/log-journal-duplication.md` §3-4、Opus round1〜4 で収束、所有者判断あり)は、
  (a) journal を正にする / (b) ログを正にする / (c) 事象ごとに寄せる を比べて **(c) を推奨**した。第 1 段階(`[press-ledger]`・`[giveup-follow]` の撤去)は実施済み(`git grep "\[press-ledger\]"` は 0 件)。
  組ごとの収支: `Blacklist drift correction` は純増の見込みで保留(消すと CI は green のまま `drift_log_fired` が 0 に化ける)、`[drift] correction` は推奨しない(診断で最も使われた行、warn として利用者のログに出る唯一の drift 補正の痕跡)、`[engine-input]` はほぼ同じか純増。
  所有者メモ: 「派生行はフィールドを落とすので、チェッカーを派生行へ移すには journal 側にフィールド追加が要り、純減にならない組がある」。
- 本 ADR は 2026-10-10 の所有者の意向(ログと journal の統合、過去との非互換は不問)を受けるが、**(c) の組ごとの収支判定は変えない**。(a) への拡大は純減になる組だけで行う。
- 本 ADR は ADR-226 の候補 E(「ログの構造化・journal への事実の集約」、所有者の着想、未レビュー・未決定)にあたる。
- ADR-225 の F1(replay は決定関数が変わったかを見るだけで、症状は判定しない)と P2(記録した出力を正にするとバグを固定する)は、下記の再生の範囲と合否に直接当たる。

## 決定(提案)

### 決定 1: 記録の単位は「核の決定関数が受け取った Facts」と「返した Plan」

- 入力の単位は、API の生の読みではなく**核の関数が受け取った Facts 1 件**(問い合わせ時刻 `now` を含む)。UIA・MSAA・TSF プローブ・`run_with_timeout` のワーカー内の読みを追わない。量は判断の回数で決まり、記録点は核の関数の入口の数に収まる。
  失敗した読み(タイムアウト)は Facts の `Option`/`Result` の値として入る。
- 出力の単位は、核が返した **Plan(送る・見送るのどちらも理由の enum つき)**。
- 打鍵・タイマー・IME イベント・フォーカスは、Facts の一種として扱う(`Engine::on_input` への入力)。
- 内部診断(C)は journal に入れず、手書きの tracing のまま残す(`Shell` の汎用エントリは作らない)。

### 決定 2: 核は journal に触らない。殻が境界で記録する(ADR-229 代案 A)

- 核の決定関数は理由つきの Plan を**返す**(既存の形)。殻が境界で `{ caused_by, facts, plan }` として記録する。`&mut JournalSink` を核に渡さない(ADR-229 代案 A・E1・F-D3「replay や不具合報告に要るものは、ログではなく戻り値か journal のレコードにする」と同じ形)。
- `&self` の読み取りの中の見送り(`[identity-gate]`、`observation_store.rs:950-960` は `effective_open` の読みごとに呼ばれる)と、`reduce` の中の fence(`[key-effect-fence]`、`ime_model.rs:699`、`Input` から決まる派生値)は記録しない。
- `sync_actuation.rs` が `hub.journal.record` を直接呼んでいる点は、`ActuationDecisionRecord` を戻り値で返す形に揃えるかどうかを段階 1 で別に決める。
- 相関: `caused_by` は seq だけに頼らず、order / generation / 押下 ID でも辿れるようにする(非同期の ImmCross 完了〈`WM_ASYNC_IME_APPLY_COMPLETE` → `on_ime_apply_complete`〉は前の turn の order が起因。`open_chain.rs:206-213` は `with_app` が `None` のとき `ActuationDecisionRecord` を捨てて数えるだけ)。
  畳み込んだ repeat の `Plan` の `caused_by` が、畳まれて消えた seq を指さないようにする。
- 構築場所を固定するガードを足す場合、既存の `architecture_guard.rs:403`・`:6123`・`:6167` との関係(置き換えか併存か)を同じ PR で書く。

### 決定 3: 見送りの理由は、調査で足りなかった決定から 1 つずつ Plan に載せる(ADR-229 E1)

- 全 288 件の再分類はしない。調査で理由が足りなかった決定から 1 つずつ、**既存の理由 enum**(`NoDrift`・`BlockReason`・`SetOpenPlan`・`PressClaim`・`GateResult`・`DriftPlan`・`physical_disposition` の `suppress_reason` など)を記録に載せる。
  新しい `Withheld` 列挙を第 2 の分類表として作らない(ADR-229 M3「Plan は `run_chain` と二重の表現になる」と同型を避ける)。
- 殻にある見送り(`runtime/`・`tsf/`・`output/`)は、ADR-229 段階 5 が済むまで対象外と明記する。最初の候補は `NoDrift`(`[drift-skip]`、ADR-233 の測定対象)。

### 決定 4: 再生の範囲を絞る

- 目標は 2 つ: (a) `Engine::on_input` まで(B5 で実証済み)、(b) 核の決定関数 1 つずつ(記録した Facts → 記録した Plan)。**端から端までの状態の再構築(ring の先頭時点の belief・台帳・GjiFsm・engine の FSM を含む)は目標にしない**。
  根拠: B5 の結果「入力の再生は BUG-105 を HEAD で再現できるが、actuation の決定は再生できない」と、ADR-229 W0(`reduce` を通らない書き込みが多い)。
- ring の先頭時点の snapshot を足すかは、ADR-241 段階 2 の結果を見て別 ADR で決める。この ADR では足さない。
- 記録した Plan を正解とみなさない(ADR-225 P2)。本物の報告に対しては、HEAD が記録した Plan と**一致しない**ことが「バグが直った」を意味する(`key_input_replay_tests.rs:418-431` が既にそう検査している)。再生が見るのは「決定関数が変わったか」であり、症状は判定しない(ADR-225 F1)。

### 決定 5: ログとの統合は収支が純減の組から。info/warn の行は消さない

- 判定は 2026-10-06 の検討の (c) を引き継ぐ。**組ごとに「撤去行・追加行・移す消費者」を表にして、純減のときだけ**手書きを消す。表は各段階の PR に付ける。
- 利用者の既定ログ(`info`)に出る行(`[drift] correction`〈warn〉、`[gji-fsm] ImeOff/FocusChange with N pending input(s) — discarding`〈warn〉、`[gji-fsm] StartComposition while engine off`〈warn、ANCHORS 対象〉、`FocusChange [pid→pid]`〈info〉)は、
  `emit_tracing` が全部 debug のままでは、生成に置き換えると利用者の `awase.log`(不具合報告の `app_log_excerpt_gz`)から消え、遡れる範囲が ring の容量に縮む。
  **これらは消さない側に分類する。** 型ごとの tracing レベルを入れる場合は ADR-139 決定 4 の改訂として別に書く。
- 生成ログの target は全部 `awase::journal` になり、`RUST_LOG=awase_windows::tsf=debug` のようなモジュール単位の絞り込みが効かなくなる。生成に置き換えるなら、型ごとの target を決める。
- 生成は journal へ積む時点(stamp 時点)で行う。中継(`pending_journal_entries`〈`platform.rs:126-131`、上限 4096〉、`SENT_INPUT_TRACE`、フックの診断キュー)の drain を待つと、
  C として残る手書き行と順序がずれ、drain 前の panic・ハング(issue #165 の hook_starved 型)で最後の数件が出ない。
- フックスレッド(`hook.rs`、`hook_callback` 内のログマクロ数は 7 件で固定)の記録は、構造体を組み立ててキューに積む仕事が増える。`LowLevelHooksTimeout` の予算との関係を段階 1 で測る。
- CI は `RUST_LOG=debug`(`e2e-ime.yml:1140`)。debug 行が増えると `awase.log` が 20MB(`app/logging.rs:24`)を超えて `.old` に回り、チェッカーも workflow も `awase.log.old` を読まない(`git grep "awase.log.old" -- tools .github` は 0 件)。現状の CI の `awase.log` の大きさを 1 run 測ってから段階を進める。
- C(内部診断)は手書きのまま。読み手のある文言(`startup:`・`[hook-watchdog]`・`IMM capability cache cleared`・`Keyboard Layout Emulator starting`)は変えない。

### 決定 6: ring は種類のレーンを残す。保持は時間で保証する

- 単一 ring にしない。レーンを「入力の種類」で分け直す(例: Key / Observation・Timer / その他の入力 / Plan / 診断)。
  - 単一 ring にすると、ADR-169 の畳み込み(`record_key_input` が「`key_input` レーンの `back()` が直前の `KeyInput`」という不変条件に依存)が成り立たない。
  - Facts の記録を増やすと、IME refresh のポーリング(`schedule_ime_refresh`、通常 500ms、`runtime/mod.rs:1083` → 1 時間に 7,200 回)が打鍵を押し出す(ADR-096 の事故の逆向き)。
- 打鍵の畳み込みは Key レーンの中だけで行う。保持は件数ではなく「各レーンが最低 N 分」で決める。N は、本物の報告 1 件の `DumpTriggered.evicted_*`・`oldest_elapsed_ms_*` で測ってから決める。
- 報告 journal の 10 分窓(`journal.rs:1498-1522`、`KeyInput`・`LiteralDetect`・`SentInput` に掛かる)は、所有者が 2026-10-06 に「外す(外す前に報告サイズを 1 件で測る)」と判断済み。外す時期は上の測定の後。

### 決定 7: 不具合報告のプレビューの約束を型で守る

- 「入力内容を含む行」を型の属性(`contains_typed_text()` のようなもの)で決め、設定画面の削除対象を文字列一致ではなくその属性に基づける。入力内容を含む型(打鍵・`SentInput`・`LiteralDetect`・送った romaji を含む Plan・読んだ文字列を含む Facts)の一覧は段階 1 で固定する。
- 段階 1 で `"type":"KeyInput"` の文字列一致が壊れる場合は、同じ PR で settings 側を直す(削除 0 件のまま打鍵を含む報告が送られる退行を防ぐ)。

### 決定 8: ADR-241 との関係

- 本 ADR は ADR-241 段階 5(記録の入力)の**記録形式だけ**を決める。採否は ADR-241 段階 2 の結果を待つ(ADR-241 の取りやめ条件を外さない)。
- 既存の再生一式とコーパス 5 種の撤去は ADR-241 側に一本化する(本 ADR は二重に主張しない)。
- 互換性は持たない(所有者判断 2026-10-10)。不具合報告の `schema_version` を上げるときは Worker を先にデプロイする(ADR-222 の前例)。コーパスに本物の報告から作った入力文を入れる場合は、所有者の訂正(2026-10-06「報告経由のサーバ共有は制約にしない。公開 issue・known-bugs には要約して載せる」)に従い、リポジトリには合成または伏せた形で置く。

## 合否の基準(提案)

1. 再生: 本物の報告(合成でない)1 件で、HEAD の決定関数の出力が修正コミットの前後で変わること。または ADR-241 決定 4 と同じ mutants の判定(打鍵からの再生でだけ落ちる変異が 1 つ以上)。
2. 各段階の PR に、組ごとの収支表(撤去行・追加行・移す消費者)を付け、**純減の組だけ**手書きを消す。
3. 変更後の e2e チェッカー(フィールドまで読む `check_consistency.py`・`check.py`・`check_run_validity.py` を含む)と `test_log_anchors_in_rust_source.py` が全て通る。
4. 設定画面の「打鍵の行をすべて削除」が、変更後の型でも打鍵を含む行を削除する(件数 0 でないことを確認する)。

## 取りやめ条件(提案)

- 段階 0 の台帳と収支表で、統合の対象になる組が純減にならない(ほとんどが純増)と分かったとき。その場合は決定 2〜3(Plan の記録)だけを残し、ログとの統合をやめる。
- 段階 1 の再生が、合否 1 を満たさないとき。ADR-241 段階 2 の結果と合わせて再判断する。

## 複雑性の収支(見積もり、段階 0 で行数に直す)

- 追加: Facts/Plan の記録の型、理由 enum の記録への載せ替え、`ImeEvent` 一式の `Deserialize`(`&'static str` のフィールドが阻む: `GiveUpFollow.outcome`・`PressWriteClaim.source/verdict`・`DeferredRecoveryFlush.trigger`・`DriftGiveUpIntervalEnded.reason`・`ActuationRecord.origin`・`KeyEventSummary.key_class`)、
  `EventTime.monotonic: Instant`(`state/ime_event.rs:41`)の数値化〈reducer の時刻の型の変更〉、`caused_by` の相関、型ごとの全フィールドの表示(`emit_tracing` は `?`/`%` 禁止なので、`ImeEvent` の中身は 20 variant ぶんの展開が要る)、チェッカーの書き換え。
- 撤去: 純減の組の手書き行、診断 variant のうち読み手の無いもの(`SentInput` は除く)、ADR-241 側の再生一式とコーパス。
- 現時点の見込みは**純増に近い**(D から読み手のある組を引くと大きく減る)。段階 0 の表で確かめ、取りやめ条件を適用する。

## 段階案

0. **コード変更なし(または `ANCHORS` の拡充だけ)**: 事実の訂正の反映、全域 grep による読み手の台帳(`tools/**`・`.github/**`・`crates/*/tests/**`・`docs/tasks/*verification*`)と `ANCHORS` の拡充、組ごとの収支表、CI の `awase.log` の大きさの測定、報告 journal のサイズ測定。
1. 理由の enum を Plan の記録に載せる(ADR-229 E1 の形)。調査で理由が足りなかった決定 1 つから(候補: `NoDrift`)。ログは手書きのまま残す。同じ PR で、`KeyInput` の分割・settings の削除・`check_consistency.py` の更新を要する場合は同時に入れる。
2. ring のレーンの見直し(測定の後)と、報告 journal の 10 分窓の扱い。
3. Facts の記録と `Engine::on_input` までの再生(B5 の延長)。ADR-241 段階 2 の結果を待つ。
4. 二重の撤去は収支が純減の組だけ。info/warn の行は消さない。

## リスク・未決

1. `Observation` ではなく Facts を単位にしたとき、Facts を持たない読み(殻の中で完結する分岐)の扱い。
2. 型ごとの tracing レベルと target(ADR-139 決定 4 の改訂になるか)。
3. 非同期経路の `caused_by` の相関の形(order / generation / 押下 ID のどれを正にするか)。
4. 機械分類の信頼度。`[msime-ready]`・`[tsf-probe]`・`[hook-watchdog] カナリア確認待ち中のため送信をスキップ`(`runtime/mod.rs:1957`)は gate 判断で B 寄りだが、タグ規則で C に入っている。
5. ring の保持時間 N の値(測定待ち)。

## round1 の反映先

- B1(再生の前提): 決定 4・合否 1・取りやめ条件。B2(核の sink): 決定 2・決定 3。
- M1(10-06 の検討): 背景 5・決定 5。M2(info/warn): 決定 5。M3(台帳): 背景 4・段階 0。M4(段階 1 で壊れるもの): 決定 7・段階 1。
- M5(ring): 決定 6。M6(ADR-241): 決定 8。M7(収支・取りやめ条件): 複雑性の収支・取りやめ条件。M8(ADR-225/226): 背景 5・決定 8。M9(事実の誤り): 背景 1〜4。
- S1(Facts): 決定 1。S2(問い合わせ時刻): 用語・複雑性の収支。S3(非同期): 決定 2。S4(中継): 決定 5。S5(CI のログ量): 決定 5。S6(フック): 決定 5。S7(`[gji-fsm]`): 背景 2。S8(ガード): 決定 2。S9(related_adr): frontmatter。
- N1〜N4: 背景 1・集計の範囲・背景 4・status。
