---
id: ADR-250
title: |-
  journal を shell と core の境界で取る(診断用の Facts/Plan の記録と Engine::on_input までの再生)。手書きログとの統合は収支が純減の組に限る
summary: |-
  いまの journal は 21 種類のエントリを殻の約 34 か所から手で書いており、境界の記録になっていない。
  核の決定関数が返す理由つき Plan(ADR-229 E1 の `OmissionBasis` など)と、受け取った Facts の射影を、殻が境界で journal に記録する(核は journal に触らない)。
  この記録の目的は診断(人と Claude が報告で読む)で、再生の入力にはしない。再生は `Engine::on_input` までに絞る(ADR-241 と矛盾させない)。
  ログとの統合は `journal-replay-rebuild-study-2026-10-06` の (c)「事象ごとに寄せる」に従い、収支が純減になる組から進める。
  利用者の既定ログ(info/warn)の行は消さない。大きく統合するには ADR-139 決定 4(生成ログは debug のみ)の改訂が要り、所有者の判断を待つ。
  過去の journal・コーパスとの互換性は持たない(所有者判断 2026-10-10)。ADR-226 の候補 E にあたる。
status: |-
  提案(起草中、Opus レビュー round2 反映済み・round3 待ち。ログ統合の規模は所有者判断待ち)
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
- **Plan**: 核の決定関数が返す理由つきの結果(`SetOpenPlan`・`DriftPlan`・`NoDrift`・`BlockReason`・`PressClaim`・`GateResult`、ADR-229 E1 で drift に付いた `OmissionBasis` など。`*_plan.rs` は 10 ファイル)。送る・送らない(**見送り**)のどちらも、理由の enum を持つ。
- **内部診断(C)**: 境界を通らない殻の出来事(Win32 の失敗、起動・終了、設定読み込み、プローブ内部)。

## 背景(実測、Opus round1 で `096dcff5` に対し裏取り済み)

### 1. いまの journal は境界で取っていない

- `JournalEntry`(`crates/awase-windows-core/src/journal.rs:261`)は 21 variant。`emit_tracing` の doc(`journal.rs:808`)は「19 variant」のまま古い。
- 構築は `journal.rs` の単体テストを除いてすべて殻の crate(`JournalEntry::X {` の構築は約 34 か所)。次は構築ではなく**参照数**の上位: `platform.rs` 20、`state/platform_state.rs` 9、`runtime/key_pipeline.rs` 7、`runtime/ime_refresh.rs` 7、
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

この ADR が新しく決めることは 2 つ。(1) 核が返した理由つき Plan と、受け取った Facts の射影を、**殻が境界で journal に記録する**こと(診断用)。(2) 手書きログとの統合を**収支が純減の組に限る**こと。
再生は ADR-241 に任せ、本 ADR は再生の範囲を `Engine::on_input` までに絞る。中身の多くは ADR-229 E1(#532 で drift に適用済み)の続きである。

### 決定 1: 記録の単位は「核が返した Plan」と「受け取った Facts の射影」。目的は診断

- 出力の単位は、核が返した **Plan(送る・見送るのどちらも理由の enum つき)**。ADR-229 E1 で drift に付いた `OmissionBasis`(`NoDrift`・`DriftIdle`・`GiveUpPark`・`DriftStep` の `basis()`、`drift_correction.rs:43,81`、`drift_plan.rs:105,152`)を最初の対象にする。
- 入力の単位は、**所有型の Facts を取る関数**(`plan_set_open(SetOpenFacts)`、`plan_core`、`*_plan.rs`)では Facts 1 件。
  借用状態を取る関数(`evaluate_drift(&ImeModel, now)`、`dispatch_set_open(&mut ImeStateHub, …, sink)`。後者は `CommandSink` の戻り値が途中で判断に入る ADR-229 F-D1 の handler 例外)では、**診断に要る射影**(判断に効いた値の抜粋)を記録する。ADR-229 F-D3 の「借用ビュー → 所有型の Facts」への縮小が済むまで、全量の Facts は目標にしない。対象の関数は段階ごとに列挙する。
- この記録は**人と Claude が不具合報告で読むための診断**であり、再生の入力にしない(決定 4)。そのため `Deserialize` の追加(`&'static str` の 6 か所、`EventTime.monotonic: Instant` の数値化)は要らない。
- 毎 tick 通る `Idle` は記録しない(ポーリングごとに積むと ring を占める。#532 も `Idle` は `trace!`)。記録するのは `Idle` 以外。
- 内部診断(C)は journal に入れず、手書きの tracing のまま残す。診断的な 6 variant(背景 1)は、読み手(人)があるので一括では消さない。実際に消せるのは `TsfProbeCompleted`・`DriftGiveUpIntervalEnded` 程度で、各 variant の扱いは該当する段階の PR で表にして決める。

### 決定 2: 核は journal に触らない。殻が境界で記録する(ADR-229 代案 A)

- 核の決定関数は理由つきの Plan を**返す**(既存の形)。殻が境界で `{ caused_by, facts の射影, plan }` の 1 レコードとして記録する。`&mut JournalSink` を核に渡さない(ADR-229 代案 A・E1・F-D3 と同じ形)。
- `&self` の読み取りの中の見送り(`[identity-gate]`、`observation_store.rs:950-960` は `effective_open` の読みごとに呼ばれる)と、`reduce` の中の fence(`[key-effect-fence]`、`ime_model.rs:699`、入力から決まる派生値)は記録しない。
- `sync_actuation.rs` が `hub.journal.record` を直接呼んでいる点は、`ActuationDecisionRecord` を戻り値で返す形に揃えるかどうかを段階で別に決める。
- `caused_by` は seq だけに頼らず、order / generation / 押下 ID でも辿れるようにする。畳み込んだ repeat の Plan の `caused_by` が、畳まれて消えた seq を指さないようにする。**非同期経路**(`WM_ASYNC_IME_APPLY_COMPLETE` → `on_ime_apply_complete`、`open_chain.rs:206-213` は `with_app` が `None` のとき記録を捨てて数えるだけ)の相関は、drift(同期)の段階では不要なので、**非同期経路を記録に載せる段階で決める**。
- 構築場所を固定するガードを足す場合、既存の `architecture_guard.rs:403`・`:6123`・`:6167` との関係(置き換えか併存か)を同じ PR で書く。

### 決定 3: 見送りの理由は、調査で足りなかった決定から 1 つずつ載せる(ADR-229 E1 の続き)

- 全 288 件の再分類はしない。既存の理由 enum(`NoDrift`・`BlockReason`・`SetOpenPlan`・`PressClaim`・`GateResult`・`DriftPlan`・`physical_disposition` の `suppress_reason`)を記録に載せる。新しい `Withheld` 列挙を第 2 の分類表として作らない(ADR-229 M3「Plan は `run_chain` と二重の表現になる」と同型を避ける)。
- **段階 1 は drift correction の Plan と `OmissionBasis` を journal に載せることだけ**(#532 は殻の `basis=` をログに出すだけで journal には載せていない)。`Idle` 以外を記録する。
- 殻にある見送り(`runtime/`・`tsf/`・`output/`)は、ADR-229 段階 5 が済むまで対象外と明記する。

### 決定 4: 再生は `Engine::on_input` までに絞る。記録した Facts/Plan は再生に使わない

- 再生の目標は **`Engine::on_input` まで**(B5 で実証済み)。入力は打鍵とタイマーで、`KeyInput` に足りないエンジンの InputContext(B5 は固定値で与えた。`key_input_replay_tests.rs:7-8`)は、`[engine-input]` の `[diag-ctx]` が持っている値を `KeyInput` に足す。
  これは 10-06 の検討 #5「`KeyInput` に移せば重複が減るのと同時に再生の材料がそろう」にあたり、統合の数少ない純減候補でもある(段階 3 で実測してから決める)。
- 核の決定関数 1 つずつを「記録した Facts → 記録した Plan」で再生することは**しない**。ADR-241(`:115`)は同種の再生(凍結コーパス 37 件の `replay_record`)を「単体テストを超える検査をしていない」として捨てると決めており、所有者も 2026-10-06 に了承している。作り直すと決定 8 と矛盾する。
- 端から端までの状態の再構築(ring の先頭時点の belief・台帳・GjiFsm・engine の FSM を含む)も目標にしない。根拠: B5 の結果と ADR-229 W0。snapshot を足すかは ADR-241 段階 2 の結果を見て別 ADR で決める。
- 記録した Plan を正解とみなさない(ADR-225 P2)。本物の報告に対しては、HEAD が記録した Plan と**一致しない**ことが「バグが直った」を意味する(`key_input_replay_tests.rs:418-431` が既にそう検査している)。再生は症状を判定しない(ADR-225 F1)。

### 決定 5: ログとの統合は収支が純減の組から。info/warn の行は消さない

- 判定は 2026-10-06 の検討の (c) を引き継ぐ。**組ごとに「撤去行・追加行・移す消費者」を表にして、純減のときだけ**手書きを消す。表は各段階の PR に付ける。
- 利用者の既定ログ(`info`)に出る行(`[drift] correction`〈warn〉、`[gji-fsm] ImeOff/FocusChange with N pending input(s) — discarding`〈warn〉、`[gji-fsm] StartComposition while engine off`〈warn、ANCHORS 対象〉、`FocusChange [pid→pid]`〈info〉)は、`emit_tracing` が全部 debug のままでは、生成に置き換えると利用者の `awase.log` から消え、遡れる範囲が ring の容量に縮む。**これらは消さない側に分類する。**
  型ごとの tracing レベルや target を変えて統合を大きくする場合は、ADR-139 決定 4 の改訂として別の決定が要る(下記「所有者への確認」)。
- 生成ログの target は全部 `awase::journal` になり、`RUST_LOG=awase_windows::tsf=debug` のようなモジュール単位の絞り込みが効かなくなる。生成に置き換えるなら、型ごとの target を決める。
- **中継を経る型(`pending_journal_entries`〈`platform.rs:126-131`、上限 4096〉、`SENT_INPUT_TRACE`〈seq だけ先に取り、中身は drain 時に組み立てる、`journal.rs:1225-1231`、`platform.rs:94-118`〉、フックの診断キュー)は、生成ログに置き換えず、手書きの行を残す。**
  stamp 時点で出せる中身が無い型があり、フックスレッドでログを出すと `hook_callback` のログマクロ数の固定(7 件)とフックの薄型化に反する。これで、順序のずれと drain 前の panic・ハングで最後の数件が消える懸念(issue #165 の hook_starved 型)も無くなる。
- CI は `RUST_LOG=debug`(`e2e-ime.yml:1140`)。debug 行が増えると `awase.log` が 20MB(`app/logging.rs:24`)を超えて `.old` に回り、チェッカーも workflow も `awase.log.old` を読まない。現状の CI の `awase.log` の大きさを 1 run 測ってから段階を進める。
- C(内部診断)は手書きのまま。読み手のある文言(`startup:`・`[hook-watchdog]`・`IMM capability cache cleared`・`Keyboard Layout Emulator starting`)は変えない。

### 決定 6: ring は種類のレーンを残す。保持は時間で保証する

- 単一 ring にしない。単一にすると、ADR-169 の畳み込み(`record_key_input` が「`key_input` レーンの `back()` が直前の `KeyInput`」に依存)が成り立たない。
- レーンは、**1 レコード `{ caused_by, facts の射影, plan }` を決定関数の種類で分ける**(Key / Timer・その他の入力 / Plan〈種類別〉)。facts と plan を別レーンにはしない(追い出しの時期が違って片方だけが残るため)。
- ポーリングの頻度: IME refresh は既定 500ms(`src/config.rs:417` の `ime_poll_interval_ms`、設定で変えられる。`awase-settings/src/main.rs:4194`)で、**1 時間に 7,200 回は無操作時の下限**。打鍵中は `schedule_ime_refresh(20)`(`key_pipeline.rs:876,1525,1640,1656`)が加わる。
  今もポーリングの観測は `ImeEvent` として State レーンに入っている(ADR-222 の「State は 13〜16 分」はその結果)。Facts の記録で**増える分**は、今の量との差分で見積もる。
- 打鍵の畳み込みは Key レーンの中だけで行う。保持は件数ではなく「各レーンが最低 N 分」で決める。N は、本物の報告 1 件の `DumpTriggered.evicted_*`・`oldest_elapsed_ms_*` で測ってから決める。
- 報告 journal の 10 分窓(`journal.rs:1498-1522`)は、所有者が 2026-10-06 に「外す(外す前に報告サイズを 1 件で測る)」と判断済み。外す時期は上の測定の後で、**決定 7 の削除対象がそろった後**とする。

### 決定 7: 不具合報告のプレビューの約束を型で守る

- 「入力内容を含む行」を型の属性(`contains_typed_text()` のようなもの)で決め、設定画面の削除対象を文字列一致ではなくその属性に基づける。入力内容を含む型の一覧(打鍵・`SentInput`・`LiteralDetect`・送った romaji を含む Plan・読んだ文字列を含む Facts の射影)は、記録の型を足す最初の段階で固定する。
- `KeyInput` の型や名前を変える段階では、同じ PR で settings 側(`"type":"KeyInput"` の一致、`bug_report.rs:947-954`)と `check_consistency.py:58`、`architecture_guard.rs:6167` を直す。
- 10 分窓を外す PR は、削除対象の属性化より後に入れる。窓を外すと、設定画面の削除が利用者にとって唯一の手段になる。

### 決定 8: ADR-241 との関係

- 再生は ADR-241 の範囲。本 ADR は、記録の形式(診断用)と、`Engine::on_input` までの再生に要る `KeyInput` の材料を決める。ADR-241 段階 5(記録の入力)の採否は、段階 2 の結果を待つ。
- 既存の再生一式とコーパス 5 種の撤去は ADR-241 側に一本化する(本 ADR は二重に主張しない)。
- 互換性は持たない(所有者判断 2026-10-10)。不具合報告の `schema_version` を上げるときは Worker を先にデプロイする(ADR-222 の前例)。コーパスに本物の報告から作った入力文を入れる場合は、所有者の訂正(2026-10-06)に従い、リポジトリには合成または伏せた形で置く。

## 所有者への確認

2026-10-10 の意向は「ログと journal の統合も目的の 1 部」。本 ADR の現在の答えは**「統合は小さい。純減の組だけ、info/warn は消さない」**で、それ以上に進めるには次の判断が要る。

- 生成ログ(`emit_tracing`)の tracing レベルと target を型ごとに変える(ADR-139 決定 4 の改訂)か。変えると、`[drift] correction` などの info/warn の行も journal からの生成に置き換えられるが、10-06 の検討が「推奨しない」とした組(診断で最も使われた行、CI が読む行)に触れる。
- 純増になる組でも統合を進めるか(その場合、取りやめ条件 1 を外す)。

## 合否の基準(提案)

1. 再生: `Engine::on_input` までの再生で、本物の報告(合成でない)1 件の打鍵とタイマーの列を HEAD に流し、修正コミットの前後で結果が変わること。**評価できる時期は段階 3 以降**で、それまでは判定しない。または ADR-241 決定 4 と同じ mutants の判定(打鍵からの再生でだけ落ちる変異が 1 つ以上)。
2. 各段階の PR に、組ごとの収支表(撤去行・追加行・移す消費者)を付け、**純減の組だけ**手書きを消す。
3. 変更後の e2e チェッカー(フィールドまで読む `check_consistency.py`・`check.py`・`check_run_validity.py` を含む)と `test_log_anchors_in_rust_source.py` が全て通る。
4. 設定画面の「打鍵の行をすべて削除」が、変更後の型でも打鍵を含む行を削除する(件数 0 でないことを確認する)。

## 取りやめ条件(提案)

- 段階 0 の台帳と収支表で、統合の対象になる組が純減にならない(ほとんどが純増)と分かったとき。その場合は決定 1〜3(Plan の記録)だけを残し、ログとの統合をやめる。
- **段階 3 の再生(`Engine::on_input` まで)が合否 1 を満たさないとき。**ADR-241 段階 2 の結果と合わせて再判断する。

## 複雑性の収支(見積もり、段階 0 で行数に直す)

- 追加: Plan と Facts の射影の記録の型、理由 enum の記録への載せ替え、`caused_by` の相関、記録の型ごとの全フィールドの表示(`emit_tracing` は `?`/`%` 禁止なので、深い型は展開が要る)、`KeyInput` への InputContext の追加、チェッカーの書き換え。
  診断用に限ったので、`ImeEvent` 一式の `Deserialize` と `EventTime.monotonic` の数値化は**要らない**(round2 B1 の推奨で外れた)。
- 撤去: 純減の組の手書き行、`KeyInput` の InputContext と重なる `[engine-input]` の重複、診断 variant のうち読み手の無いもの(`TsfProbeCompleted`・`DriftGiveUpIntervalEnded` 程度)、ADR-241 側の再生一式とコーパス。
- 現時点の見込みは**純増に近い**(D から読み手のある組を引くと大きく減る)。段階 0 の表で確かめ、取りやめ条件を適用する。

## 段階案

0. **コード変更なし(または `ANCHORS` の拡充だけ)**: 事実の訂正の反映、全域 grep による読み手の台帳(`tools/**`・`.github/**`・`crates/*/tests/**`・`docs/tasks/*verification*`)と `ANCHORS` の拡充、組ごとの収支表、CI の `awase.log` の大きさの測定、報告 journal のサイズ測定。
1. drift correction の Plan と `OmissionBasis` を journal に載せる(`Idle` 以外)。ログは手書きのまま残す。記録の型を足す場合は、決定 7 の `contains_typed_text()` を同じ PR で入れる。
2. ring のレーンの見直し(測定の後)。10 分窓の扱いは、決定 7 の削除対象がそろった後。
3. `KeyInput` に InputContext を足し、`Engine::on_input` までの再生(B5 の延長)を作る。ADR-241 段階 2 の結果を待つ。
4. 二重の撤去は収支が純減の組だけ。info/warn の行は消さない。非同期経路の記録は、この段階以降に相関の形を決めてから。

## リスク・未決

1. 借用状態の関数の「射影」に何を入れるか(診断に要る値の範囲)。段階ごとに決める。
2. 型ごとの tracing レベルと target(ADR-139 決定 4 の改訂になるか)。所有者の判断待ち。
3. 非同期経路の `caused_by` の相関の形(order / generation / 押下 ID のどれを正にするか)。
4. 機械分類の信頼度。`[msime-ready]`・`[tsf-probe]`・`[hook-watchdog] カナリア確認待ち中のため送信をスキップ`(`runtime/mod.rs:1957`)は gate 判断で B 寄りだが、タグ規則で C に入っている。
5. ring の保持時間 N の値(測定待ち)。

## 反映先

### round1

- B1(再生の前提): 決定 4・合否 1・取りやめ条件。B2(核の sink): 決定 2・決定 3。
- M1(10-06 の検討): 背景 5・決定 5。M2(info/warn): 決定 5。M3(台帳): 背景 4・段階 0。M4(段階 1 で壊れるもの): 決定 7・段階 1。
- M5(ring): 決定 6。M6(ADR-241): 決定 8。M7(収支・取りやめ条件): 複雑性の収支・取りやめ条件。M8(ADR-225/226): 背景 5・決定 8。M9(事実の誤り): 背景 1〜4。
- S1〜S9・N1〜N4: Facts は決定 1、非同期は決定 2、中継は決定 5、CI のログ量は決定 5、フックは決定 5、`[gji-fsm]` は背景 2、ガードは決定 2、related_adr は frontmatter。

### round2

- B1(決定 4(b) が ADR-241 の捨てた再生の作り直し): 決定 4 を `Engine::on_input` までにし、決定 1 で Facts/Plan の記録を診断用に位置づけ、複雑性の収支から `Deserialize` を外した。
- M1(段階番号と合否 1 の時期): 合否 1・取りやめ条件・段階 3。M2(借用状態の Facts): 決定 1。M3(stamp 時点の生成): 決定 5。
- M4(#532 と `Idle`): 用語・決定 1・決定 3・段階 1。M5(目的が薄れた): summary・決定の冒頭・所有者への確認。
- S1(ポーリング頻度): 決定 6。S2(レコードとレーン): 決定 6。S3(診断 6 variant): 決定 1。S4(`KeyInput` の InputContext): 決定 4・段階 3。S5(10 分窓と削除の順序): 決定 6・決定 7。S6(非同期の相関の段階): 決定 2・段階 4。
- N1(34 か所): summary・背景 1。N2(検討文書の名前): summary。N3(Observation): リスクから削除。N4(status): frontmatter。
