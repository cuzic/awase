---
id: ADR-250
title: |-
  journal を shell と core の境界で取り(診断用の Facts/Plan の記録、再生は Engine::on_input まで)、境界を通る手書きログを journal からの生成に寄せる(ADR-139 決定 4 の改訂)
summary: |-
  いまの journal は 21 種類のエントリを殻の約 34 か所から手で書いており、境界の記録になっていない。
  核の決定関数が返す理由つき Plan(ADR-229 E1 の `OmissionBasis` など)と、受け取った Facts の射影を、殻が境界で journal に記録する(核は journal に触らない)。
  この記録の目的は診断(人と Claude が報告で読む)で、再生の入力にはしない。再生は `Engine::on_input` までに絞る(ADR-241 と矛盾させない)。
  ログとの統合は、所有者が「大きく統合する」を選んだ(2026-10-10)。journal を正にして journal → tracing へ生成する方向(ADR-139 Option C)を広げ、
  生成ログの tracing レベルと target を型ごとに宣言する(ADR-139 決定 4 の改訂)。tracing → journal の方向(Option B)は ADR-139 のとおり不採用。
  収支表は判定ではなく見積もりに使い、純増は許容する。読み手のあるログは、チェッカーを同じ PR で直してから置き換える。
  `tsf/`・`output/` の行も対象にするが、`architecture_guard.rs:403`(ADR-096 決定 2、層の境界)は緩めない。literal-detect と同じ「事実をデータとして上に渡し、`platform.rs` が記録する」方式で、出来事ごとに配線する(所有者判断 2026-10-10)。
  過去の journal・コーパスとの互換性は持たない(所有者判断 2026-10-10)。ADR-226 の候補 E にあたる。
status: |-
  提案(2026-10-10 に所有者が「大きく統合する」を選んだため決定 5 を改訂。Opus round7 で収束〈Blocker 0・Must 0〉。続けて所有者が tsf/・output/ をデータを上に渡す方式で統合すると判断し反映、Opus round9 で収束〈Blocker 0・Must 0〉。ガード :403 は緩めない)
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
- **Facts**: 核の決定関数が受け取る引数(`SetOpenFacts`・`DecisionInputs`・`ImeSnapshot`・`classify_*` の引数など)。殻が OS から読んだ値、問い合わせ時刻(`now`)を含む。所有型の Facts を取る関数では 1 件、借用状態を取る関数では「診断に要る射影」を指す(決定 1)。殻が嘘をつく読みは、その値に既に含まれる。
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
  `tsf/`・`output/` は `crate::journal` の参照を禁じられている(`architecture_guard.rs:403`)。`tsf/`・`output/` は、事実をデータとして `platform.rs` へ持ち上げて記録する方式で扱う(決定 5)。

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
- 本 ADR は 2026-10-10 の所有者の判断(ログと journal の統合を**大きく**進める、過去との非互換は不問)を受け、**(c) の「純減の組だけ」を採らない**。組ごとの収支表は残すが、判定ではなく見積もりと PR 説明に使う(決定 5)。
  ただし (c) が挙げた個別の事実(`[drift] correction` は診断で最も使われた行、`Blacklist drift correction` を消すと CI が green のまま `drift_log_fired` が 0 に化ける、派生行はフィールドを落とす)は、置き換えの順序と注意点として決定 5 に引き継ぐ。
- 本 ADR は ADR-226 の候補 E(「ログの構造化・journal への事実の集約」、所有者の着想、未レビュー・未決定)にあたる。
- ADR-225 の F1(replay は決定関数が変わったかを見るだけで、症状は判定しない)と P2(記録した出力を正にするとバグを固定する)は、下記の再生の範囲と合否に直接当たる。

## 決定(提案)

この ADR が新しく決めることは 2 つ。(1) 核が返した理由つき Plan と、受け取った Facts の射影を、**殻が境界で journal に記録する**こと(診断用)。(2) 境界を通る手書きログを**journal からの生成に寄せる**こと(ADR-139 決定 4 の改訂。所有者判断 2026-10-10)。
再生は ADR-241 に任せ、本 ADR は再生の範囲を `Engine::on_input` までに絞る。中身の多くは ADR-229 E1(#532 で drift に適用済み)の続きである。

### 決定 1: 記録の単位は「核が返した Plan」と「受け取った Facts の射影」。目的は診断

- 出力の単位は、核が返した **Plan(送る・見送るのどちらも理由の enum つき)**。ADR-229 E1 で drift に付いた `OmissionBasis`(`NoDrift`・`DriftIdle`・`GiveUpPark`・`DriftStep` の `basis()`、`drift_correction.rs:43,81`、`drift_plan.rs:105,152`)を最初の対象にする。
- 入力の単位は、**所有型の Facts を取る関数**(`plan_set_open(SetOpenFacts)`、`plan_core`、`*_plan.rs`)では Facts 1 件。
  借用状態を取る関数(`evaluate_drift(&ImeModel, now)`、`dispatch_set_open(&mut ImeStateHub, …, sink)`。後者は `CommandSink` の戻り値が途中で判断に入る ADR-229 F-D1 の handler 例外)では、**診断に要る射影**(判断に効いた値の抜粋)を記録する。ADR-229 F-D3 の「借用ビュー → 所有型の Facts」への縮小が済むまで、全量の Facts は目標にしない。対象の関数は段階ごとに列挙する。
- この記録は**人と Claude が不具合報告で読むための診断**であり、再生の入力にしない(決定 4)。そのため `Deserialize` の追加(`&'static str` の 6 か所、`EventTime.monotonic: Instant` の数値化)は要らない。
- **記録規則は edge 記録**: 前の tick と理由(`DriftPlan` の variant + `basis`)が**変わったときだけ**記録し、同じ理由の連続は 1 件に畳む(ADR-169 の打鍵の畳み込みと同じ考え方)。**edge の判定は殻が `record` を呼ぶ前に行う**(畳んだ分は `record` を呼ばず、ログも出ない。件数と最後の時刻は次の edge のレコードに載せる)。ADR-169 のように journal の中で畳んで `emit_tracing` を毎回呼ぶ作りにすると、`StillParked`・`NotDrifting` の tick ごと(既定 500ms)に debug 行が出て、CI(`RUST_LOG=debug`)で 1 時間 7,200 行の純増になる。
  - 畳んだレコードは**件数と最後の時刻**を持つ(parked が何分続いたかを報告から読めるようにする)。edge の判定キーには**フォーカスの世代**を含める(フォーカスが変わったら同じ理由でも新しい 1 件にする)。
  - 「`Idle` 以外」では両方向に外れる。`Idle` の中に `NoDrift::StaleObservation`(ADR-233 の測定対象)や `NotExplicitIntent`(issue #189・BUG-110)があり、逆に `GiveUp(StillParked)`・`GiveUp(CooldownPending)` は parked の間ポーリングの tick ごと(既定 500ms)に返る(`drift_plan.rs:93-115`、殻は `Idle` を `trace!` にして return する〈`runtime/ime_refresh.rs:892-895`〉)。
  - 既存の `ImeActuation{GiveUp}` は `GiveUp` のどの variant でも記録されるため、parked の間ポーリングの tick ごとに積まれている。**段階 1 の edge 判定に含めるか(`GiveUp` の連続を畳むか)を一緒に決める**。
  - `Send` と `GiveUp(FirstTime)` は既存の `ImeActuation`(`action = Send/GiveUp`)と `ActuationDecision` に記録されている。journal の中に二重を作らないよう、既存の `ImeActuation` に `basis` を足すか、新しいレコードから外すかを段階 1 で決める。段階 1 で新しく増える情報は、`SkipWarrantWouldBlock`・`Confirmed`・`DeferToSettle`・`Rearm`・`Idle` の各理由と `basis`。
- **診断用の記録が保証すること**: (i) 不具合報告の JSON に載る(ring の保持、決定 6)。(ii) 人が読める(`emit_tracing` の行に `basis` などの理由が出る。`?`/`%` 禁止なので enum は `variant_name` で出す)。(iii) 記録の有無が挙動を変えない。型の互換や決定性は保証しない。
- 内部診断(C)は journal に入れず、手書きの tracing のまま残す。診断的な 6 variant(背景 1)は、読み手(人)があるので一括では消さない。消せるのは `TsfProbeCompleted`・`DriftGiveUpIntervalEnded` 程度という見込み(実測ではない)で、各 variant の扱いは該当する段階の PR で表にして決める。

### 決定 2: 核は journal に触らない。殻が境界で記録する(ADR-229 代案 A)

- 核の決定関数は理由つきの Plan を**返す**(既存の形)。殻が境界で `{ caused_by, facts の射影, plan }` の 1 レコードとして記録する。`&mut JournalSink` を核に渡さない(ADR-229 代案 A・E1・F-D3 と同じ形)。
- `&self` の読み取りの中の見送り(`[identity-gate]`、`observation_store.rs:950-960` は `effective_open` の読みごとに呼ばれる)と、`reduce` の中の fence(`[key-effect-fence]`、`ime_model.rs:699`、入力から決まる派生値)は記録しない。
- `sync_actuation.rs` が `hub.journal.record` を直接呼んでいる点は、`ActuationDecisionRecord` を戻り値で返す形に揃えるかどうかを段階で別に決める。
- `caused_by` は seq だけに頼らず、order / generation / 押下 ID でも辿れるようにする。畳み込んだ repeat の Plan の `caused_by` が、畳まれて消えた seq を指さないようにする。**非同期経路**(`WM_ASYNC_IME_APPLY_COMPLETE` → `on_ime_apply_complete`、`open_chain.rs:206-213` は `with_app` が `None` のとき記録を捨てて数えるだけ)の相関は、drift(同期)の段階では不要なので、**非同期経路を記録に載せる段階で決める**。
- 構築場所を固定するガードを足す場合、既存の `architecture_guard.rs:403`・`:6123`・`:6167` との関係(置き換えか併存か)を同じ PR で書く。

### 決定 3: 見送りの理由は、調査で足りなかった決定から 1 つずつ載せる(ADR-229 E1 の続き)

- 全 288 件の再分類はしない。既存の理由 enum(`NoDrift`・`BlockReason`・`SetOpenPlan`・`PressClaim`・`GateResult`・`DriftPlan`・`physical_disposition` の `suppress_reason`)を記録に載せる。新しい `Withheld` 列挙を第 2 の分類表として作らない(ADR-229 M3「Plan は `run_chain` と二重の表現になる」と同型を避ける)。
- **段階 1 は drift correction の Plan と `OmissionBasis` を journal に載せることだけ**(#532 は殻の `basis=` をログに出すだけで journal には載せていない)。記録規則は決定 1 の edge 記録。
- 殻にある見送り(`runtime/`・`tsf/`・`output/`)のうち、`tsf/`・`output/` は、決定 5 の「データを上に渡す方式」で記録する(`architecture_guard.rs:403` は緩めない)。**決定 5 の置き換えのために足す記録は、この「1 つずつ」の基準の外**(段階 4 の PR ごとに、置き換える組に必要な記録を足す)。「1 つずつ」は、置き換えを目的としない診断用の Plan の追加(段階 1 の drift など)に掛かる。下の FCIS の記述は `runtime/` の分割に関するもの。FCIS のタスク表(`docs/tasks/fcis-layering-tasks-2026-10-06.md:212-218`)では F1〜F6 がマージ済みで F5d は「分けない」と決まっており、`tsf/tsf_gate.rs`・`output/probe_io.rs`・`runtime/key_pipeline.rs` の shift-conv-guard・key-effect-predict が今後の分割に入るかは未確認(段階 0 で確かめる)。入っていなければ「分割の予定に無く、本 ADR では対象外」と扱う。

### 決定 4: 再生は `Engine::on_input` までに絞る。記録した Facts/Plan は再生に使わない

- 再生の目標は **`Engine::on_input` まで**(B5 で実証済み)。**再生のハーネスは ADR-241 決定 3(B5 の補助をハーネスへ移す)と段階 5 の範囲**で、本 ADR は作らない。本 ADR が決めるのは**記録の形式**だけ。入力は打鍵とタイマーで、`KeyInput` に足りないエンジンの InputContext(B5 は固定値で与えた。`key_input_replay_tests.rs:7-8`)は、`[engine-input]` の `[diag-ctx]` が持っている値を `KeyInput` に足す。
  これは 10-06 の検討 #5「`KeyInput` に移せば重複が減るのと同時に再生の材料がそろう」にあたり、統合の数少ない純減候補でもある(段階 3 で実測してから決める)。
- 核の決定関数 1 つずつを「記録した Facts → 記録した Plan」で再生することは**しない**。ADR-241(`:115`)は同種の再生(凍結コーパス 37 件の `replay_record`)を「単体テストを超える検査をしていない」として捨てると決めており、所有者も 2026-10-06 に了承している。作り直すと決定 8 と矛盾する。
- 端から端までの状態の再構築(ring の先頭時点の belief・台帳・GjiFsm・engine の FSM を含む)も目標にしない。根拠: B5 の結果と ADR-229 W0。snapshot を足すかは ADR-241 段階 2 の結果を見て別 ADR で決める。
- 記録した Plan を正解とみなさない(ADR-225 P2)。本物の報告に対しては、HEAD が記録した Plan と**一致しない**ことが「バグが直った」を意味する(`key_input_replay_tests.rs:418-431` が既にそう検査している)。再生は症状を判定しない(ADR-225 F1)。

### 決定 5: 境界を通る手書きログは journal からの生成に寄せる(ADR-139 決定 4 の改訂。所有者判断 2026-10-10)

**方向は journal → tracing のまま(ADR-139 Option C)。** tracing → journal の方向(Option B、`tracing_subscriber::Layer` で journal へ流す案)は、ADR-139 決定 4 が退けた理由(`Layer::enabled` はフィールド値を見られない、抑制カウンタが持てない、**`with_app` の再入で `try_borrow_mut` が失敗し、入れ子のイベントが黙って捨てられる**)がそのまま残るので採らない。
ADR-139 は「journal 記録約 49 箇所に対して `log::` は 736 箇所と規模もカーディナリティも違うので統合対象ではない」とも書いた。本 ADR はこの結論(「両者は本来統合対象ではない、本当の重複はもっと狭い」)を、**境界を通る出来事(A・B)については journal の記録に寄せる**方向へ改める。C(内部診断)は対象外のまま。

- **ADR-139 決定 4 の改訂点(3 つ)**: 「型ごと」ではなく、**置き換えた手書き行の出力条件と同じ条件の arm** にだけ適用する。
  1. **レベル**: `emit_tracing` の arm のうち、手書き行を置き換えたものだけを、その手書き行の現在のレベル(info/warn)で出す。それ以外の記録は今のまま debug。
     - 型で決められない理由: レベルは型ではなく出来事の条件で決まる組がある。例: `FocusChange [pid→pid]`(info、`runtime/focus_tracking.rs:576-577`)はプロセスが変わる経路だけで出るが、`FocusTransition` は `changed.any()`(hwnd・class など、どの軸でも)で記録される(`focus_tracking.rs:39-64`、ADR-096 B-3)。`FocusTransition` 型を info にすると、info 行は手書きより増える。`DriftPlan` も `[drift] correction` は warn だが、同じ型の他の variant は debug・trace(`ime_refresh.rs:894`)。
     - 実装の形: tracing の level と target は呼び出し点の static metadata で定数でなければならず、型のメソッドが返す実行時の値は `tracing::event!` に渡せない。実装は `emit_tracing` の arm(必要なら variant や条件の分岐)ごとに、`tracing::warn!(target: "awase_windows::runtime::ime_refresh", …)` のような定数のマクロを書く形になる。`?`/`%`/ワイルドカード禁止のガード(`architecture_guard.rs:6123`)の下で arm が増える分は、収支に入れる。
  2. **target**: 置き換えた arm だけ、置き換えた手書き行の現在の target(モジュールのパス)にする。他の記録は今のまま `awase::journal`。これで `RUST_LOG=awase::journal=debug`(ADR-139 `:495` の文書化された使い方)は、置き換えていない記録については引き続き効き、`RUST_LOG=awase_windows::tsf=debug` のようなモジュール単位の絞り込みは、置き換えた arm について効く。ADR-139 への追記にこの変更(置き換えた arm は `awase::journal` から外れる)を含める。
  3. **ADR-139 の「variant ごとに個別判断すると、新しい呼び出し元が増えるたびに前提が崩れる」(`:486-495`)への答え**: info 以上で出す arm の一覧(型・variant・条件・レベル)を `architecture_guard.rs` で固定する(名前と件数の表)。新しい info/warn の arm を足すときにレビューに掛かる。さらに、**info 以上の arm を持つ型の構築点(`JournalEntry::X {` の呼び出し元)の件数も固定する**(`journal_key_input_construction_is_limited_to_key_pipeline`〈`architecture_guard.rs:6167`〉と同じ形)。arm だけでは、新しい呼び出し元を足してもガードが通り、利用者のログが増えるため。
- **対象**: A・B のうち、journal の記録(決定 1 の Plan と Facts の射影)で置き換えられる手書き行。**出来事ごとに出る手書き行は、畳む記録(決定 1 の edge 記録の Plan)では置き換えず、出来事ごとの記録(`ImeActuation` など)で置き換える。** 例: `[drift] correction`(warn)は送信の試行ごとに 1 行出て、`check_invariants.py:8`(I1)と `check_startup.py:18` が件数を数える。`DriftPlan` で置き換えると試行 1・2・3 回目が 1 行に畳まれ、件数が黙って減り、BUG-43 型(無限再送)の行の数が見えなくなる。置き換え先は試行ごとに記録される `ImeActuation` で、`DriftPlan` ではない。生成行は、**置き換えた arm だけ**全フィールドを `key=value` で出す(既存の 21 型すべてには広げない。`RUST_LOG=debug` の CI で打鍵ごとに出る `KeyInput` のバイト数を増やさないため。`emit_tracing` の `?`/`%` 禁止〈`architecture_guard.rs:6123`〉に従い、enum は `variant_name` で出す)。
- **出力を増やさない**: 置き換えは 1:1(手書き 1 行 → 生成 1 行)。生成で 1 出来事が複数行になる、または debug の複製が info に昇格して行が増える形は認めない。**行数は、既定 `info` の下での出力件数(手書きの info/warn 行の件数と同じ)で測る。さらにバイト数でも測る**(ADR-139 決定 2 の懸念は行数ではなくバイト数。フィールドを全部出すと、行数が同じでも大きさは増える)。1 出来事が複数の手書き行だった組(`[drift] correction` と `Blacklist drift correction`、`[engine-input]` と `CTRL MISMATCH`)を 1 行に寄せるのは、減る側なので制約に反しない。CI は `RUST_LOG=debug`(`e2e-ime.yml:1140`)で、`awase.log` が 20MB(`app/logging.rs:24`)を超えて `.old` に回ると前半の行が消えても green のままになる(`awase.log.old` を読む workflow は無い)。現状の `awase.log` の大きさ(バイト数)を段階 0 で 1 run 測り、**閾値をそこから決める**(例: CI の最長の run で `awase.log` が 20MB の 1/2 を超えない)。段階ごとに増分を測る(ADR-139 決定 2 の 747MB の懸念)。
- **純増を許容する**: 所有者が選んだ。組ごとの収支表(撤去行・追加行・移す消費者)は**見積もりと PR 説明のため**で、純減でなくても置き換える。10-06 の検討の (c) の「純減の組だけ」は採らない。
- **置き換えの順序**: 読み手のない組 → 読み手のある組(チェッカー・`ANCHORS`・`architecture_guard.rs:2498` の `DRIFT_SEND_LOG_MARKER`・testdata〈`tools/e2e/ime_key_matrix/testdata/*.awase.log`、22 本〉を**同じ PR で**更新してから置き換える)→ 10-06 の検討が「推奨しない」とした組(`[drift] correction`〈診断で最も使われた行、warn として利用者のログに出る唯一の drift 補正の痕跡〉、`Blacklist drift correction`〈消すと CI は green のまま `drift_log_fired` が 0 に化ける〉、`[engine-input]`〈本文の `mods(c=true … phys_ctrl=true` まで読まれる〉)を**最後**。最後の組は、置き換え後の行が、読み手のフィールドと文言を保つか、読み手を先に直せることを確かめてから行う。
- **記録点の位置**: 置き換え後の生成行が、元の手書き行と**同じ位置(同じ関数・同じ前後関係)**で出ることを置き換えの条件にする。順序を読むチェッカー(`check_reopen.py` など)がある組、`architecture_guard.rs:2498-2521`(match ブロック内は `tracing::debug!` のみ、実送信は `tracing::warn!` で始まる唯一の箇所、という BUG-43/163 の順序ガード)がある組が対象。ガードの目印は、文言ではなく記録の呼び出し(`JournalEntry::ImeActuation` の構築位置)に付け替える。`ImeActuation` の構築は `runtime/ime_refresh.rs` の同じ関数に 2 か所(`:961`、`:1052`)あるので、**送信側(`action = Send` の arm)の構築だと特定できる形**(関数名と arm、または送信の直前という位置)で目印を書く。`#[tracing::instrument]` の span 名が行の前置きに入る点(testdata の `on_ime_apply_complete{…}:` など)は、記録点が別関数になると変わるので、testdata も同じ PR で直す。
- **フィールド落ちに注意**: 派生行(`emit_tracing`)はトップレベルのフィールドしか出さず(例: `FocusTransition` の `from`/`to`、`ImeEvent` の中身の大半は出ない)、チェッカーを派生行へ移すには journal 側にフィールドを足す必要がある。足す量は段階ごとの見積もりに入れる。
- **置き換えない経路(技術的な制約)**:
  - `journal.record()` は `&mut` を要求するので、`with_app` が `None` になる再入の経路の出来事は記録が捨てられる(ADR-139 決定 4 の理由 3、`open_chain.rs:206-213` が同型)。そのような経路の手書きは残す。段階ごとに、置き換え対象の出来事が `with_app` の中から出ているかを確かめる。
  - `SentInput` とフックの診断キューは、出す場所がフックスレッド、または journal を持たない自由関数なので、手書きの行を残す(上の「`tsf/`・`output/` の行の扱い」の理由を参照)。`pending_journal_entries` を経る型は、push 時の生成を段階 0 で確かめる。
- C(内部診断)は手書きのまま。読み手のある文言(`startup:`・`[hook-watchdog]`・`IMM capability cache cleared`・`Keyboard Layout Emulator starting`)は変えない。
- **`tsf/`・`output/` の行の扱い(所有者判断 2026-10-10: 「データを上に渡す方式で統合」)**: `architecture_guard.rs:403`(`output/`・`tsf/` の本番コードは `crate::journal` を参照しない、ADR-096 決定 2・round3。層の境界を守る規約で、`output/`・`tsf/` が journal を知らないようにする意図)は**緩めない**。
  - 方式は literal-detect(ADR-096 round3 C-1〜C-3)と同じ: 事実を ungated の純粋なデータ型(例: `tsf/literal_facts.rs`)にして `dispatch_probe_actions` → `StepProbeResult` → `WindowsPlatform::advance_tsf_probe` と持ち上げ、`JournalEntry` への変換は `platform.rs` だけが行う。
  - 出来事ごとに配線が要る。手書きログの出る位置が `tsf/`・`output/` の中から `platform.rs` の記録点へ移るので、「記録点の位置」の条件(同じ関数・同じ前後関係)を**満たせない組がある**。順序を読むチェッカーがない組、または順序の変化が許容できる組だけを置き換え、満たせない組は手書きのまま残す。段階 0 の台帳で組ごとに判定する。
  - `[gji-fsm]` の 36 件は、「… ignored」「… のため無視」の**理由**が多い(背景 2)。手書き行は `tsf/gji_fsm.rs` の `on_event`/`on_timeout` の**内側**にあり、`GjiEvent` がどの経路から来ても出る。一方 `GjiFsmTransition` を記録するのは `platform.rs` の `note_gji_transition`(`:133-137`)と probe tick(`:425`)だけである。`GjiEvent` を FSM へ渡す呼び出し点(`gji_on_event(`)は、`platform.rs`(5 か所。ほかに `gji_on_long_idle` の 1 経路。段階 0 の台帳)のほかに `output/mod.rs:1159`(`WarmupComplete`/`WarmupAborted`)と `output/vk_send.rs:223`・`:401`(`KeyInput`)の 3 か所がある(`git grep -nE "gji_on_event\(" -- crates/awase-windows/src`。`output/mod.rs:348-352` と `output/tsf_warmup_coord.rs:121` は委譲と定義で、呼び出し点ではない)。`runtime/key_pipeline.rs:1699` と `tsf/warmup/warmup_strategy.rs:9` の `GjiEvent::` は doc コメントで、ディスパッチ点ではない。
    - **置き換えるのは、`GjiEvent` の全ディスパッチ点(上の `gji_on_event(` の呼び出し点)で理由つきの記録がそろってから**。そろう前に手書き行を消すと、`output/`・`tsf/warmup/`・`runtime/` から入ったイベントの行が黙って消え、ANCHORS とチェッカー(`check_invariants.py`・`check_reopen.py`)が読む `[gji-fsm] StartComposition while engine off` で起きれば、CI は green のまま件数が 0 になる(ADR-119 の「全合流点を洗い出さずに 1 か所に置く」と同型)。
    - `output/`・`tsf/` のディスパッチ点は、理由を戻り値で `platform.rs` まで持ち上げる(literal-detect と同じ)。ディスパッチ点の件数は `architecture_guard.rs` で固定する(改訂点 3 の構築点の件数の固定と同じ形)。数えるのは `GjiEvent::` の出現数ではなく、**コメントを除いた `gji_on_event(` の呼び出し点**。platform 以外のディスパッチ点は、すでに応答を platform に返している(`output/mod.rs:1159` は戻り値で、`vk_send.rs` は `push_key_response` で積み `platform.rs:937` が取り出す)ので、足りないのは、platform がそれらの応答を受けたときの理由つきの記録である。
    - 理由の返し方: `GjiFsm` は `timed-fsm`(crates.io に独立して公開している crate)の `TimedStateMachine` を実装しており(`gji_fsm.rs:591`)、`on_event`/`on_timeout` の戻り値 `Response<GjiAction, GjiTimer>` の形は変えられない。理由は `GjiAction` の variant(例: `GjiAction::Ignored { reason }`)として返し、各ディスパッチ点が action の実行と同じ流れで持ち上げる。`tsf/gji_fsm.rs` は journal を参照せず、データを返すだけ(`state_label()` と同じ)。
  - 保留キュー(`pending_journal_entries`、`platform.rs:126-131`)を経る型の生成ログ: `push_journal_entry` は push の時点で `self.stamper.stamp(entry)` に**完全な中身**を渡しており、`GjiFsmTransition`・`TsfProbeStarted`・`TsfProbeCompleted`・`LiteralDetect`・`DeferredRecoveryFlush` はこの経路。したがって**push の時点で生成ログを出せる可能性がある**(drain まで待たない。drain 前の panic・ハング、手書き行との順序のずれの懸念を避けられる)。ただし `emit_tracing` は `journal.rs` の private メソッドで、`absorb` でも呼ばれるため、**二重に出さない仕組み**(push 時に出したものは `absorb` で出さない)が要る。実装の形は段階 0 で確かめる。**`SentInput` とフックの診断キューは生成に置き換えない**。中身は発生時に確定しているが、出す場所の問題である: `SentInput` は `win32.rs` の自由関数 `send_input_safe` が journal を持たず、seq だけ `reserve` して drain 時に組み立てる(`win32.rs:366-380`)。フックの診断(`HookImeModeDiagnosticRecord`)は push がフックスレッドで(`hook.rs:1490`、`:1387`)、そこでログを出すと `hook_callback` のログマクロ数の固定(7 件)とフックの予算に反する。「中身が確定するなら置き換えてよい」とは読まないこと。
- この決定の採用時に、ADR-139 の決定 4 に「ADR-250 決定 5 で、レベルと target を型ごとの宣言に改訂した」と追記する(ADR-139 の status の追記も同様)。

### 決定 6: ring は種類のレーンを残す。保持は時間で保証する

- 単一 ring にしない。単一にすると、ADR-169 の畳み込み(`record_key_input` が「`key_input` レーンの `back()` が直前の `KeyInput`」に依存)が成り立たない。
- レーンは、**1 レコード `{ caused_by, facts の射影, plan }` を決定関数の種類で分ける**。facts と plan を別レーンにはしない(追い出しの時期が違って片方だけが残るため)。
  現行の 4 レーン(State: `ImeEvent`・`FocusTransition`・`ImeOpenApplied`・`ClockAnchor`・`DumpTriggered`、Timing: `GjiFsmTransition`・`LiteralDetect`・`TsfProbe*`・`HookImeModeDiagnostic`・`DeferredRecoveryFlush`、Actuation: `ImeActuation`・`SentInput`・`ActuationDecision`・`PressWriteClaim`・`Drift*`・`GiveUpFollow`・`ConvClassifyCall`・`TimerFired`、KeyInput: `KeyInput`)との対応は**段階 2 で決める。例は仮**(Key / Timer・その他の入力 / Plan〈種類別〉)。
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

## 所有者の判断(2026-10-10)

ログと journal の統合の規模について、所有者は**「大きく統合する」**を選んだ(選択肢: 純減の組だけ〈Opus round4 で収束した版〉/ 先に第 3 の案〈チェッカーが読む行だけ構造化〉/ 大きく統合する)。
決定 5 はこれに基づく。選ばれなかった案との差は次のとおり。

- 利点(選んだ理由の側): 1 つの源から構造化されたログが出る。チェッカーが文言の正規表現ではなくフィールドを読めるようになり、文言が変わって 0 件で通る事故(d47645eb、`test_log_anchors_in_rust_source.py` の背景)が型で防げる。
- トレードオフ(受け入れたもの): 純増になりうること、ADR-139 決定 2(747MB の実測による肥大の懸念)、10-06 の検討が「推奨しない」とした組に触れること。順序(読み手のない組から、推奨しない組は最後)と、行数を増やさない 1:1 の制約で緩和する。
- **`tsf/`・`output/` の行(2026-10-10 の所有者判断)**: 「データを上に渡す方式で統合」を選んだ(選択肢: 対象外のまま / データを上に渡す方式 / ガードを緩める)。ガード :403 の理由は ADR-096 決定 2・round3(層の境界を守り、「ついでに journal を呼べば早い」という誘惑を構造的に止める。設計書 `journal-literal-detect-capture.md` の Y-2)で、技術的な制約ではなく設計上の意図なので緩めない。代償: 出来事ごとの配線、ログの出る位置の変化(決定 5)。
- 第 3 の案(チェッカーが読む行だけを `key=value` に構造化)は、置き換えの途中で読み手のある組を直すときの最初の一歩として、決定 5 の順序に取り込まれる。

## 合否の基準(提案)

1. 再生: **ADR-241 段階 5 の合否に従う**(他の ADR の作業の結果で本 ADR の採否が決まる形になることを明記する)。本 ADR 側の確認は、`KeyInput` に InputContext を足した記録から、`Engine::on_input` の再生に要る材料がそろうこと(本物の報告 1 件で確かめる。評価できる時期は段階 3 以降)。
2. 各段階の PR に、組ごとの表(撤去行・追加行・移す消費者)を付ける(**見積もりと説明のため。純減は条件にしない**)。置き換えは 1:1 で、**既定 `info` の下での出力件数が増えないこと、CI の `awase.log` のバイト数が段階 0 で決めた閾値以下であること**を段階ごとに測って示す(取りやめ条件と同じ基準)。
3. 変更後の e2e チェッカー(フィールドまで読む `check_consistency.py`・`check.py`・`check_run_validity.py` を含む)と `test_log_anchors_in_rust_source.py` が全て通る。
4. 設定画面の「打鍵の行をすべて削除」が、変更後の型でも打鍵を含む行を削除する(件数 0 でないことを確認する)。
5. 診断用の記録: 段階 1 の後、drift の見送り(`NoDrift::StaleObservation` など)が、不具合報告の journal に理由(variant + `basis`)つきで載り、同じ理由の連続が 1 件に畳まれる。

## 取りやめ条件(提案)

- 段階 0 の台帳で、置き換えると読み手(チェッカー・workflow・ガード)を壊さずには直せない組、または `with_app` の再入で記録が落ちる経路にある組が分かったとき。**その組は手書きのまま残す**(組単位の見送り)。
- 統合の PR 群が、置き換え後の e2e チェッカーと `test_log_anchors_in_rust_source.py` の全通過を保てない、または `awase.log` のバイト数が段階 0 で決めた閾値を超える(CI で `.old` に回る)と測れたとき。その段階で止め、決定 1〜3(Plan の記録)は残す。
- ADR-241 段階 5 の合否が満たされないとき(`Engine::on_input` の再生が成り立たないとき)。その場合は `KeyInput` への InputContext の追加を取りやめ、診断用の記録(決定 1〜3)だけを残す。

## 複雑性の収支(見積もり、段階 0 で行数に直す)

- 追加: Plan と Facts の射影の記録の型、理由 enum の記録への載せ替え、`caused_by` の相関、記録の型ごとの全フィールドの表示(`emit_tracing` は `?`/`%` 禁止なので、深い型は展開が要る)、`KeyInput` への InputContext の追加、チェッカーの書き換え。
  診断用に限ったので、`ImeEvent` 一式の `Deserialize` と `EventTime.monotonic` の数値化は**要らない**(round2 B1 の推奨で外れた)。
- 撤去: 置き換えた組の手書き行、`KeyInput` の InputContext と重なる `[engine-input]` の重複、診断 variant のうち読み手の無いもの(`TsfProbeCompleted`・`DriftGiveUpIntervalEnded` 程度)、ADR-241 側の再生一式とコーパス。
- 現時点の見込みは**純増に近い**(D から読み手のある組を引くと大きく減る)。所有者は純増を許容した(2026-10-10)。段階 0 の表で規模を見積もり、各段階の PR で実測する。

## 段階案

0. **〔実施済み 2026-10-10。結果は [docs/tasks/adr250-stage0-ledger-2026-10-10.md](../tasks/adr250-stage0-ledger-2026-10-10.md)。報告 journal のサイズの実測だけ未了〕コード変更なし(または `ANCHORS` の拡充だけ)**: 事実の訂正の反映、全域 grep による読み手の台帳(`tools/**`・`.github/**`・`crates/*/tests/**`・`docs/tasks/*verification*`)と `ANCHORS` の拡充、組ごとの収支表、CI の `awase.log` の大きさの測定、報告 journal のサイズ測定。
1. drift correction の Plan と `OmissionBasis` を journal に載せる(決定 1 の edge 記録。「`Idle` 以外」ではない)。ログは手書きのまま残す。記録の型を足す場合は、決定 7 の `contains_typed_text()` を同じ PR で入れる。
2. ring のレーンの見直し(測定の後)。10 分窓の扱いは、決定 7 の削除対象がそろった後。
3. `KeyInput` に InputContext を足す(記録の形式)。再生のハーネスは ADR-241 段階 5。ADR-241 段階 2 の結果を待つ。
4. ログの統合(決定 5)。4-0: arm ごとの定数のマクロと、info 以上の arm の一覧を固定するガードを足す。**機構だけで、既定は今のまま debug・`awase::journal`**(各 arm のレベルと target の変更は、その手書き行を消す PR と同じ PR で入れる。単独で先に入れると、置き換えまでの間 info 行が二重に出る)。ADR-139 決定 4 の改訂の追記もここ。4-1: 読み手のない組。4-2: 読み手のある組(チェッカー・`ANCHORS`・ガード・testdata を同じ PR で更新。位置の条件も満たす)。4-2b: `tsf/`・`output/` の行(データを上に渡す方式。`[gji-fsm]` は全ディスパッチ点で記録がそろってから。`GjiAction` に variant を足すと `gji_fsm.rs` の単体テストの期待値が変わるので、収支に入れる。`platform.rs:516` の `match` にはワイルドカードが無く、variant を足すとコンパイラが処理漏れを止める)。4-2b のうち ANCHORS・チェッカーの読み手がある行(`[gji-fsm] StartComposition while engine off`・`stale confirm 検出`・`[raw-tsf-literal] flush escape=` など)は 4-2 の条件(チェッカー・`ANCHORS`・ガード・testdata を同じ PR で更新)も満たすこと。4-3: 10-06 の検討が「推奨しない」とした組(`[drift] correction`・`Blacklist drift correction`・`[engine-input]`)。非同期経路の記録は、この段階以降に相関の形を決めてから。

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

### round3

- M1(段階 1 の記録規則): 決定 1(edge 記録、`Send`/`GiveUp(FirstTime)` の二重の扱い)・決定 3・合否 5。
- S1(ADR-241 とのハーネスの所有): 決定 4・合否 1・取りやめ条件・段階 3。S2(診断用の保証): 決定 1・合否 5。S3(所有者への確認の公平さ): 所有者への確認。
- S4(ADR-229 段階 5 の予定): 背景 3・決定 3。S5(レーンの対応): 決定 6。N1(見込み): 決定 1。N2(Facts の用語): 用語。
- round4(収束後の Should 2 件): 段階 1 の記録規則を決定 1 の edge 記録に揃えた。畳んだレコードの件数・最後の時刻と、edge のキーへのフォーカス世代を決定 1 に追記した。

### 所有者判断(2026-10-10、「大きく統合する」)の反映先

- 決定 5 を全面改訂(ADR-139 決定 4 の改訂、行数を増やさない 1:1、置き換えの順序、置き換えない経路)。「所有者への確認」を「所有者の判断」に置き換え。
- summary・title・status・背景 5・決定の冒頭・合否 2・取りやめ条件・複雑性の収支・段階 4。
- round5: M1(型ごとの宣言では 1:1 を保てない)は決定 5 の改訂点 1〜3(arm ごとの定数・置き換えた arm だけ・一覧をガードで固定)。S1(4-0 の二重)は段階 4。S2(記録点の位置)は決定 5 の「記録点の位置」。S3(target)は改訂点 2。S4(バイト数)は「出力を増やさない」・「対象」。S5(閾値)は「出力を増やさない」・取りやめ条件。N1 は決定 5 の冒頭。
- round6: M1(edge 記録と 1:1)は決定 1(edge の判定は `record` の前)と決定 5 の「対象」(`ImeActuation` で置き換える)。M2(統合の範囲)は決定 5 の「統合の範囲」と「所有者の判断」の未決。S1(構築点の件数)は改訂点 3。S2(合否 2)は合否 2。N1 は改訂点の見出し。
- round7: Should 3 件は summary(統合範囲の限定)、決定 5 の記録点の位置(送信側の特定)、決定 1(`ImeActuation{GiveUp}` の edge 判定)。
- 所有者判断(tsf/・output/ をデータを上に渡す方式で統合): summary・決定 3・決定 5(「`tsf/`・`output/` の行の扱い」・置き換えない経路)・所有者の判断・段階 4-2b。
- round8: M1(`[gji-fsm]` の全ディスパッチ点)・S2(`GjiAction` の variant で理由を返す)は決定 5 の `[gji-fsm]` の項。S1(中継の 2 型を残す理由)は決定 5 の「`tsf/`・`output/` の行の扱い」と「置き換えない経路」。S3 は決定 3。N1 は段階 4-2b。N2 は index.md。
- round9(収束): 経路の列挙の誤り(doc コメントを経路に数えていた)を決定 5 の `[gji-fsm]` の項で訂正し、ガードの数え方を `gji_on_event(` の呼び出し点にした。`GjiAction` の variant 追加のテスト期待値の変更を段階 4-2b に追記した。
