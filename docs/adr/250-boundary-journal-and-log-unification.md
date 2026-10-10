---
id: ADR-250
title: |-
  journal を shell と core の境界で取り、ログと統合する(境界の入力・出力を唯一の記録源にする)
summary: |-
  いまの journal は 21 種類のエントリを殻の約 40 か所から手で書いており、境界の記録になっていない。
  入力(打鍵・タイマー・IME イベント・フォーカス・観測結果)と出力(決定・見送り)の 2 種類に絞り、
  人が読むログはその記録から生成する。手書きの tracing のうち journal と二重のもの(下限 73 件)は消す。
  過去の journal・コーパスとの互換性は持たない(所有者判断 2026-10-10)。
status: |-
  提案(起草中、Opus レビュー前)
related_adr:
  - "ADR-082"
  - "ADR-096"
  - "ADR-163"
  - "ADR-169"
  - "ADR-222"
  - "ADR-225"
  - "ADR-226"
  - "ADR-229"
  - "ADR-241"
---

# ADR-250: journal を shell と core の境界で取り、ログと統合する

調べた版は develop `2ccf4c7e`(#576 マージ後)。ローカルでのビルド・テストはしていない。
件数は 2026-10-10 に `git grep` と規則による機械分類で数えた値で、手作業の全件レビューはしていない
(分類の境界は ±数十件の誤差。「決定」の根拠にする前に再実測する、と各節に書く)。

## 用語

- **境界**: 殻(`crates/awase-windows`、Win32 を呼ぶ側)と核(`crates/awase-windows-core` と root の `awase`、純粋な判断)の間。
  核へ入るもの(入力)と核から出るもの(出力)がある。ADR-229 の FCIS の語に従う。
- **入力(A)**: 打鍵、タイマー発火、IME イベント、フォーカス変化、殻が OS から読んだ観測結果。
- **出力(B)**: 核が決めたもの。送信 command、belief の更新、**見送り**(送らない・書かない決定と、その理由)。
- **内部診断(C)**: 境界を通らない殻の出来事(Win32 の失敗、起動・終了、設定読み込み、プローブ内部)。

## 背景(実測)

### 1. いまの journal は境界で取っていない

- `JournalEntry`(`crates/awase-windows-core/src/journal.rs:261`)は 21 variant。
  構築している場所は、`journal.rs` 自身の単体テストを除いてすべて殻(`crates/awase-windows/src/**`)。
  `platform.rs` 20、`focus/thread_scope.rs` 10、`runtime/ime_refresh.rs` 9 など(`git grep -c "journal::\|JournalEntry"`)。
- 入力と結果が 1 件に混ざる variant がある: `KeyInput`(`on_input` の入力と `state_before`/`state_after`/decision)、
  `TimerFired`(timer_id と前後の状態)、`ConvClassifyCall`(引数と戻り値)、`ActuationDecision`(gate の入力と試行の連鎖)。
- `ImeEvent` から belief を再構築できない。`journal.rs:283-312` の doc に残りが書いてある:
  `Instant` を記録しない、`Deserialize` が無い、初期状態の snapshot が無い、`reduce` を通らない書き込みは記録されない。
- 診断だけの variant が 6 種ある(`DriftGiveUpDiagnostic`・`HookImeModeDiagnostic`・`DriftGiveUpIntervalEnded`・
  `TsfProbeStarted`・`TsfProbeCompleted`・`SentInput`)。再生側の読み手も、ログ行を読む CI も見つからなかった。
- コーパス `crates/awase-windows/tests/journals/` の 5 種のうち、実機ダンプの `JournalEntry` JSON は 2 つ
  (`key_input/bug-105-tight-d1.json`、`actuation_decision/bug-131-*.json`)だけ。
  残る `conv_classify`・`drift_correction`・`ime_apply` は手で書き写した別形式で、`JournalEntry` を読んでいない。

### 2. ログと journal が二重になっている

- 手書きの tracing 呼び出しは 727 件(マクロ呼び出しの実数。`git grep` の 775 行には `#[tracing::instrument]` 31 行とコメントが混ざる)。
  うち `emit_tracing`(journal 由来)の 21 件を除いた 705 件を機械分類した:
  A(入力)143、B(出力)288、C(内部診断)201、D(journal と二重)73(下限)。
- `emit_tracing`(`journal.rs:814`)は journal の全 variant を `target: "awase::journal"` の debug ログへ複製する。ログの約 10% が複製行(ADR-222 の実測)。
- D の最大は `[gji-fsm]`(`GjiFsmTransition` と約 35 件)、次が `[literal-detect]`(`LiteralDetect` と 12 件)。
- 手書きログの構造化は進んでいない。フィールド付き(`key = value`)は 1 件(`runtime/open_chain.rs:208`)だけで、
  75.6% が format 文字列への埋め込み。journal 由来の 21 件は全部フィールド付き。

### 3. 出力側に穴がある

B の 288 件には、gate・skip・fence の**見送りの理由**が多く含まれる
(`[tsf-gate] held`、`[identity-gate] hwnd不一致`、`[drift-skip]`、`[key-effect-fence]`、`[shadow-toggle] 同じ押下で既に書いた` など)。
これらは journal の variant に対応が無い。送ったものは記録に残るが、**送らなかった理由は手書きログにしか無い**。

### 4. 読み手(消してはいけないもの)

- CI・e2e チェッカーが読む journal 由来のログ行は 6 種(`key input`・`actuation decision`・`ime open applied`・
  `focus transition`・`gji fsm transition`・`literal detect`)。固定は `tools/e2e/ime_key_matrix/test_log_anchors_in_rust_source.py` の `ANCHORS`(約 32 件)。
- `gji fsm transition` の `trigger` は自由な文字列で、`BeliefSync:` や `StartComposition` の断片を正規表現が読む(`check_reopen.py:50-51`、`check_invariants.py:55`)。
- 手書きログで読み手のあるもの: `[engine-input]`(本文の `mods(c=true … phys_ctrl=true` まで)、`[drift] correction`、`[hook] IME-mode vk=`、
  `[warrant-shadow]`、`[vk-send]`、`send_keys: mode=`、`stale confirm 検出` ほか。`startup:`・`[hook-watchdog]`・`IMM capability cache cleared` は C だが読み手がある。
- JSON 本体を読む CI は無い。読むのは再生テスト(`key_input_replay_tests.rs`)、`ActuationDecision` のコーパス、不具合報告の整形(`awase-settings/src/bug_report.rs`)。
- `crates/awase-windows/tests/architecture_guard.rs` が `hook_callback` 内のログマクロ数(7 件)と `#[tracing::instrument]` の存在(約 20 ファイル)を固定している。

## 決定(提案)

### 決定 1: 記録の単位を「境界を越えた 1 回」にする

`JournalEntry` を、次の 3 種に再編する。

- `Input`: 核へ渡したもの。種類は `Key`・`Timer`・`ImeEvent`・`Focus`・`Observation`。
- `Output`: 核が返したもの。種類は `Command`(送信・belief 更新)と `Withheld { reason }`(見送り)。
  各 `Output` は、起因した `Input` の `seq` を `caused_by` に持つ。
- `Shell`: 内部診断(C)。境界を通らないものだけ。

現行の 21 variant は、入力と出力に分けて型へ移す(対応表は ADR の実装段階で作る)。
`KeyInput`・`TimerFired`・`ConvClassifyCall`・`ActuationDecision` は入力と出力に割る。

### 決定 2: 入力に「観測結果」を入れる。読んだ値をそのまま記録する

IME の状態は殻が読む値で、読み方によって嘘をつく(CLAUDE.md の前提)。
再生で効くのは「殻が読んだ値を核へ渡した」時点の値だけで、読む前の実状態は再現できない。
よって `Observation` は、**読んだ値・読んだ手段(source)・時刻**を、核へ渡した形のまま記録する。
失敗した読み(タイムアウト・不一致)も、「失敗した」という入力として記録する(読み手の判断材料になる)。

`ImeEvent` は、時刻(`Instant` の代わりに単調な数値)を含めて `Deserialize` できる形にし、
`reduce` を通らない belief の書き込みは、`Input` として記録する口を作るか、書き込み自体をなくす
(`.claude/rules/ime-belief-architecture.md` の「belief の更新口は 1 つ」に沿う)。

### 決定 3: 見送りを型で記録する

`Withheld { reason }` の `reason` は列挙型にする(文字列にしない)。最初の候補は、
`TsfGateHeld`・`IdentityGateMismatch`・`DriftSkip(kind)`・`KeyEffectFenced`・`PressAlreadyClaimed`・`ShadowAlreadyApplied` など。
B の 288 件を再分類して、実際の列挙を決める(この ADR では列挙の中身を決めない)。

### 決定 4: 出力は核が自分で記録する

出力の記録は、核の関数が受け取った `JournalSink`(`&mut` 引数)へ書く。殻から手で作らない。
入力は殻が境界を越える点で記録する(`key_pipeline` の入口、タイマーの入口、観測の受け渡し点)。
これで「journal の構築が殻に散らばっている」(背景 1)が、入力の数か所と核の中だけになる。
`architecture_guard.rs` に、`JournalEntry` を構築してよい場所を固定するガードを足す。

### 決定 5: ログは journal から生成する

- 人が読むログ行は、journal の `Input`/`Output` から生成する表示(`emit_tracing` の後継)にする。書式は型ごとに固定し、全フィールドを `key=value` で出す。
- D(二重)の 73 件は、手書きを消す。読み手のあるもの(`[gji-fsm] StartComposition`・`[engine-input]`・`[drift] correction`・`[hook] IME-mode`・`[warrant-shadow]`・`[vk-send]`・`send_keys:`・`stale confirm 検出`)は、
  チェッカーと `ANCHORS` を**同じ PR で**更新する。
- C(内部診断)は手書きの tracing のまま残す。読み手のある文言(`startup:`・`[hook-watchdog]`・`IMM capability cache cleared`・`Keyboard Layout Emulator starting`)は変えない。
- A・B のうち境界を通る手書きログは、journal の記録に置き換えて消す。件数は段階ごとに実測する。

### 決定 6: 互換性は持たない

過去の journal・凍結コーパス・古い不具合報告との互換性は持たない(所有者判断、2026-10-10)。
不具合報告の `schema_version` を上げ、Worker を先にデプロイする(ADR-222 の前例)。
既存の再生一式とコーパス 5 種は、新しい記録の再生に置き換えるのと同時に撤去する(ADR-241 の決定と同じ扱い)。

### 決定 7: ADR-241 との関係

ADR-241 は「再生ハーネス」、本 ADR は「再生の入力になる記録」を決める。
ADR-241 の段階 1(同期の判断を核へ移す)は本 ADR と独立に進められる。
ADR-241 の再生は、本 ADR の `Input` 列を HEAD の核へ流し、`Output` 列と比べる形にする。

## 合否の基準(提案)

1. B5 の前例(BUG-105)を、新しい `Input` 列の再生で HEAD の核に流し、記録した `Output` と一致する。
2. 統合後、手書きの tracing 呼び出しが減った件数を PR ごとに実測して書く(目標は D の 73 件を 0 にすること)。
3. 既存の e2e チェッカーが、変更後のログ行で全て通る(`test_log_anchors_in_rust_source.py` を含む)。

## リスク・未決(Opus レビューで特に見てほしい点)

1. **観測結果の粒度**: 全部の読みを記録すると量が増える(UIA・MSAA・TSF プローブ)。どこまでを `Input` にするか。
2. **ring の構成**: 現行は 4 レーン(ADR-096)。打鍵が他を押し出す事故への対策だった。単一の ring にすると再発しないか。レーンを残すなら何で分けるか。
3. **核が書く出力の記録**: `JournalSink` を引数で渡すと、純粋な関数のシグネチャが変わる。ADR-229 の「核は純粋」に反しないか(戻り値に記録を載せる案との比較)。
4. **機械分類の信頼度**: `[msime-ready]`・`[tsf-probe]` は gate 判断で B 寄りだが、タグ規則で C に入っている。C(201 件)は過大の疑いがある。
5. **E2E チェッカー**: `[engine-input]` の本文断片への依存は、書式の順序でも壊れる。型付きフィールドへの移行時に、チェッカーを先に直す順序でよいか。
6. **複雑性予算**: 新しい型(`Input`/`Output`/`Withheld`)を足す一方で、撤去できる量(診断 6 種、二重の 73 件、再生一式、コーパス)を見積もり直すこと(`.claude/rules/complexity-budget.md` は未発効)。

## 段階案(順序は Opus レビューの後に確定)

1. 型の定義と `KeyInput`/`TimerFired` の分割(入力と出力)。再生で BUG-105 を通す。
2. `Withheld` と見送り理由の列挙。B の 288 件の再分類。
3. `ImeEvent` の完全化(時刻・`Deserialize`・直接書き込みの扱い)と `Observation`。
4. D の 73 件の削除と、チェッカー・`ANCHORS`・ガードテストの同時更新。
5. ring の構成の見直し、不具合報告の schema 更新、コーパス・既存再生の撤去。
