---
id: ADR-229-companion-229-opus-fcis-round1
title: |-
  ADR-229 FCIS 設計 opus-adversarial-consult round1
type: companion-doc
related_adr:
  - "ADR-229"
  - "ADR-224"
---

# FCIS 設計ドラフト r1 の敵対的レビュー(round1)

- 対象: `fcis-design-draft-r1.md`。
- 実測の基準: `origin/develop` = `d0d42be6`。PR #492〜#495 は未マージなので、その結果は「通った事実」としてだけ使った。
- 読み取りのみ。行番号は `origin/develop` のもの。

## 0. 結論(先に)

- **方向性は良い。ただし r1 には、実コードと食い違う前提が 3 つあり、このまま ADR にすると最初の実装タスクで破綻する。**
  1. **「core = 時刻を読まない・グローバルを持たない」と「ungated = Linux でテストできる」を同一視している。** 既に ungated で「core」扱いされている `state/` には、時刻の直接読み取りもグローバルもある。2 つの概念を分けないと、F-D6 の enforcement(Q8)は導入した瞬間に既存コードで落ちる(§2 B1)。
  2. **`HubClock` を「新設しない」「呼び出し箇所が非常に多いので当面の例外」としているが、事実は逆。** `HubClock` は既にある時計の注入の仕組み(`Wall { tick: fn() -> u64 }` / `Manual`、`state/hub_clock.rs`)で、仮想時計のために作られた。`ImeStateHub` の中で時計を読むのは本番で 4 か所だけ。例外ではなく、**状態を持つ core の標準形として採用すべき**(Q3)。
  3. **F-D1(core は port を呼ばない)を全面に適用すると、既存の `Actuation<Verified>::run_chain(_async)<W: MechanismWriter>` を壊す。** このチェーン走査は型状態(ADR-090、warrant → verify を経ないと write できない)と ADR-163 の再生ハーネスを支えている。**「handler trait を引数に取る core のアルゴリズム」を、F-D1 の正式な例外 1 種として明文化する**のが正しい(Q1)。
- **段階 2(`platform_state` の ungate)は、r1 の「関数ポインタ注入」でも「引数渡し」でもなく、「`_in_scope` 版を core に、`foreground_scope()` を読む薄い殻を gated な別ファイルの `impl ImeStateHub` に」分ける形が、ほぼ最小の費用で済む。**
  - 呼び出し元(runtime/ の 13 か所)も、ガードの呼び出し元の固定も変わらない。
  - ADR-224 が懸念した「スコープ失効が見えなくなる」も、ハーネスがスコープを明示的に渡せるので解消する(Q4)。

## 1. Q1〜Q12 への回答

### Q1: open_chain、`run_chain_async`、writer trait、ReplayWriter

**事実**

- `state/actuation_chain.rs`(ungated)
  - `run_chain<W: MechanismWriter>`(`:549-576`)と `run_chain_async<W: AsyncMechanismWriter>`(`:580-607`)は、**`Actuation<Verified>` にしか生えない**(モジュール doc `:5`、`:90-111` の compile_fail 例、`pub struct Verified` は `:226`)。
  - 走査の判定は `Self::classify`/`abandon` の 1 か所(純粋)。
  - trait は `MechanismWriter`(`:613`)と `AsyncMechanismWriter`(`:621`)。テスト用の `FakeWriter`(`:778-789`)が既にある。
- 本番の writer は 2 つ。
  - 同期: `ime_controller.rs:449` の `SyncChainWriter`(`:455` で impl、`:639` で `run_chain`)。
  - 非同期: `runtime/open_chain.rs:102-160` の `AsyncChainWriter`。`:692` で `run_chain_async(&WriteMechanism::ALL, ..)`。中は `imm_cross_write`(`:223`、`.await` あり)と `fallback_write`(`:459`、本体全体が `with_app`。機構ごとに view を作り直す)。
- open_chain は「書く → 観測 → 再判定」のうち、**観測と再判定を writer の中(shell 側)で行う**。core(走査)は「`Failed` なら次の機構」しか判断しない。

**判断: 正式な例外として許す(純粋な Cmd の状態機械には書き換えない)**

| 観点 | A: 例外として許す(推奨) | B: `next(state, event) -> Cmd` へ書き換える |
|---|---|---|
| 利点 | 型状態の保証(`Verified` を経ないと write できない)をそのまま保てる。`FakeWriter` と、ADR-163 の ReplayWriter 案(記録済みの `(mechanism, outcome)` を返す writer)でチェーン走査をそのまま再生できる。本番の変更が 0 | 「core は port を呼ばない」が例外なしに成り立つ |
| 費用 | F-D1 に例外が 1 種類できる | write を「Cmd を返す」形にすると、`Actuation<Verified>` の消費(アフィン性)を Cmd のトークンで表し直す必要がある。open_chain の await をまたぐ再観測(`fallback_write` の view の作り直し)を shell が Event ごとに再構築する形になるが、これは今 `fallback_write` がしていることと同じで、何も減らない |
| 再発ファミリー | 影響なし | **actuation 合流点**(ADR-119/BUG-90。gate を 3 関数すべてで独立に再検出する設計、`with_app` の再入で fail-open する B-1 の意味論)と **BUG-34 型の窓**(`open_chain.rs:426-436` が名指しで見送った view1/view2 の分割)に直接触れる。ADR-180 決定2(費用対効果が負)と同じ轍 |

- 例外の定義(文言案): 「core のアルゴリズムが、途中の結果で次の手を決める場合に限り、**効果の実行役(handler)の trait を引数に取ってよい**。trait の本番実装は shell に置き、core は trait 以外の環境依存を持たない。テストは偽物の handler(`FakeWriter`・ReplayWriter)で行う」。これは代数的 effect の handler の形そのもので、FCIS とも矛盾しない(core は「効果を要求する」だけで、実行しない)。
- 例外に入るもの: `run_chain(_async)`。warmup の `StepCoro` も Cmd を yield する形なので、例外ではなく F-D4 の標準形でよい。
- ReplayWriter との整合: 例外として許せば、ReplayWriter は既存の trait の実装 1 つで書ける。テスト側だけで、本番の変更は不要。前回の私のレビューの A′(再生できない分岐 4 → 1)と同じ。**書き換え案 B を選ぶと、この再生ハーネスの延長が使えなくなる。**

### Q2: F-D1(core は port を呼ばない)は現実的か

「結果を見て次の OS 呼び出しを決める」連鎖は、open_chain 以外にもある(読んだ範囲)。

1. **ImmCross が `Failed` のときの再読み取り**: `imm_cross_write`(`open_chain.rs:223-410`)が `Failed` の後に `read_ime_state_fast()` を読み、`AlreadyMatched` か `Failed` かを決める(`post_failed_reobservation`)。
2. **`romaji_pre_write`**(`ime_controller.rs::apply_mechanism` の先頭): ブロックしうる `SendMessageTimeoutW` を同期で呼び、続けて機構を実行する(BUG-34 E-prep)。
3. **per-VK confirm**(`tsf/warmup/probe_fsm.rs:470-503`): 送信 → `vk_sent` → 判定 → 次の VK。ただしこれは `StepCoro` で既に Cmd/Event の形になっている。
4. **drift correction の再送**(`ir_apply_drift_correction`、Blind/Read の再送打ち切り)と、**conv の actuation**(書く → 確認 → 再試行)。
5. **focus の分類**: 同期の分類 → MSAA → UIA と、読み取りが段階的に続く。

**破綻する場面**: 「先に読む Facts」で済むのは、**読むものの集合が判断の前に決まっている**場合だけ。1・2・5 は読むかどうか自体が判断に依存する(条件付きの B 読み)。

- 全部を先に読むと、余計なプロセス間呼び出しが増える。IMM32 のプロセス間呼び出しは最初に約 5 秒ブロックしうる(`send_health.rs` の doc、BUG-34)。
- 読み取りを Cmd にして turn を分けると、F-D5-1(1 turn で最後まで)に反し、観測と実行の間に窓ができる。

→ **F-D1 には「条件付きの同期 B 読み」の扱いが要る。** 推奨は、Q1 の handler 例外を「読み取りの handler」にも広げること(例: `trait ImeProbe { fn read_open(&mut self) -> ProbeRead; }` を引数に取る core)。そうすれば turn を分けずに済む。Q1 の例外と同じ 1 種類の例外として定義できる。

### Q3: `HubClock` は例外か標準か(実測)

- `ImeStateHub` の本番コードで時計を読むのは **4 か所**(`platform_state.rs:189` `record_at(.., self.clock.now_instant())`、`:834` `effective_open()` の中の `now_tick()`、`:854` `now_instant()`、`:1209` `record_miss(self.clock.now_instant())`)と、構築の 1 か所(`:137` `HubClock::wall(crate::hook::current_tick_ms)`)。テストは `:1847-1863`(`HubClock::manual`、`advance_ms`)。
- 一方、時計を使う入口 `ImeStateHub::effective_open()` の呼び出し元は **runtime/ に 34 か所**(key_pipeline 20、mod 5、ime_refresh 4、focus_tracking 3、message_handlers 2)と、`platform_state.rs` の中に 2 か所。
- `HubClock` は **`Instant` と tick(ms)の 2 つの時間軸**を 1 つの値で供給する(`hub_clock.rs:1-12` の doc。`IntentStore` の TTL は tick、観測の鮮度は `Instant`)。r1 の F-D3 は `now: TickMs` しか書いておらず、**`Instant` の時間軸を見落としている**。
- **判断: `HubClock` を「状態を持つ core に時計を注入する標準形」として採用する。**
  - 時計を値として持つ(実時計は shell が関数ポインタで渡し、テストは `Manual`)のは、FCIS の「環境は外から注入」に合っている。
  - `now` を引数に統一すると、`effective_open()` の 34 か所の呼び出し元に時刻の読み取りを足すことになり、見通しが悪くなるだけ。
  - 状態を持たない関数(`decide_*`、`plan_core`、`hwnd_cache` のような小さな型)は、`now` を引数で受ける。
  - 「core は時刻を読まない」は、「core は**壁時計を直接**読まない(読むなら注入された clock 経由)」に修正する。
- **注意**: 時計の抽象は既に 3 つある(`state::hub_clock::HubClock`、`timed_fsm::Clock`/`ManualClock`、`quanta::Clock`。最後のものは journal の `DumpTriggerTracker` で使用)。**4 つ目を作らないこと**を ADR に書く。統合は急がない。

### Q4: `foreground_scope()` と `ImeStateHub` のメソッドの呼び出し元(実測)

- `platform_state.rs` で `foreground_scope()` を呼ぶのは 11 メソッドに 1 回ずつ: `arm_mode_key_pass_mark`(`:308`)、`mode_key_pass_expiry_wait_ms`(`:317`)、`note_awase_write_for_mode_key_pass`(`:324`)、`mode_key_pass_window_remaining_ms`(`:348`)、`mode_key_pass_mark_live`(`:356`)、`expire_mode_key_pass_mark`(`:379`)、`arm_external_change_watch`(`:418`)、`external_change_watch_remaining_ms`(`:432`)、`follow_external_change`(`:456`)、`align_after_expired_mode_key_pass`(`:532`)、`invalidate_intents_if_mode_key_pass_live`(`:562`)。
- **スコープを引数に取る `_in_scope` 版が、既に 5 つある**: `note_awase_write_for_mode_key_pass_in_scope`(`:327`)、`mode_key_pass_mark_live_in_scope`(`:334`)、`invalidate_intents_if_mode_key_pass_live_in_scope`(`:359`)、`drop_intents_for_mode_key_pass_in_scope`(`:385`)、`align_after_expired_mode_key_pass_in_scope`(`:536`)。テスト用の `test_foreground_scope()` もある(`:2561`)。
- 11 メソッドの、`platform_state.rs` の外の本番の呼び出し元は **13 か所・4 ファイル**: executor 1、key_pipeline 2、runtime/mod 4、ime_refresh 6(`git grep '\.<method>('` で数えた)。`architecture_guard.rs:3767-3775` は `.arm_external_change_watch(`・`.follow_external_change(` の呼び出し元ファイルを固定している。
- `tests/support/harness.rs` にスコープの概念は無い(`scope` の grep で 0 件)。`arm/follow_external_change` の副作用だけを写している(`harness.rs:22-24`)。
- **3 案の比較**

| 案 | 呼び出し元の変更 | ガード | ADR-224 の懸念(スコープ失効が見えない) |
|---|---|---|---|
| 引数渡し(13 か所で `foreground_scope()` を読んで渡す) | 13 か所・4 ファイル | `:3767-3775` は呼び出し元のファイルを見るだけなので通る | 解消(ハーネスが任意のスコープを渡せる) |
| 関数ポインタ注入(`ImeStateHub::new(tick_fn, scope_fn)`) | 0 | `:1382` が落ちる(文字列が変わる) | **解消しない**。`fn()` は状態を捕まえられないので、ハーネスがフォーカスの切り替えを表すにはグローバルが要る |
| **推奨: 核と殻に分ける**。11 メソッドすべてに `_in_scope(scope)` 版を用意して `platform_state.rs`(ungated)に置き、`foreground_scope()` を読んで `_in_scope` を呼ぶ薄い殻を、gated な別ファイル(例: `state/platform_state_shell.rs`、`#[cfg(windows)] mod`)の `impl ImeStateHub` に置く | **0**(殻のメソッド名は今と同じ) | `:3767-3775` は呼び出し元が変わらないので通る。`:1382` は、`new()` を `platform_state.rs` の中で `#[cfg(windows)]` にしておけば通る(Linux 用には ungated な `with_clock(clock)` を足す) | 解消(ハーネスは `_in_scope` を直接呼び、スコープを明示的に変えられる) |

- 推奨案は、`HubClock` と同じ「環境は殻で読み、核は値を受ける」形を、時計とスコープの両方に一貫して当てはめるものになる。

### Q5: core の中のログ

- **許す**。`tracing` は観測用の出力で、core の判断に読み戻されない(戻り値に影響しない)。禁止すると診断が失われ、BUG の調査がしにくくなる。
- 線引き: **replay や不具合報告に要るものは、ログではなく戻り値か journal のレコード**(`ActuationDecisionRecord`、`JournalEntry`)にする。ログには依存しない。
- 既存の制約に注意する。
  - ADR-139 の `emit_tracing` の検査(`architecture_guard.rs:5165-5205`。`?`/`%` 書式の禁止)。
  - `decision3_instrument_targets_…`(`:5071-`)は、一覧のファイルに `#[tracing::instrument]` が 1 つ以上あることを要求する。判断を core へ移すとき、元のファイルから instrument が消えるとこのガードが落ちる。移したら一覧を見直す。

### Q6: `&mut State` か、新しい State を返すか

- **`&mut self` を許す**。既存の reducer は `ImeModel::reduce(&mut self, &envelope)`。純粋さの要件は「入力が同じなら出力も状態も同じ」(決定性)で、`&mut` はこれに反しない。新しい State を返す形は clone の費用だけ増えて、見通しは良くならない。
- 禁止すべきなのは、`&mut` 越しの**隠れた環境**(グローバル・時計・`with_app`)のほう。

### Q7: executor を trait にする範囲(契約テスト)

- **既にあるものを核にし、新しい trait は増やさない**。
  - actuation: `MechanismWriter`/`AsyncMechanismWriter`(Q1)。
  - 送信: `SentInput` の journal(`win32.rs` の `SentInputBatch` と `SentKeyEvent`。#495 で `SentKeyEvent` は journal へ移る)と `shadow_send_trace`。
- 閉ループの擬似 IME(`tests/support/pseudo_ime.rs`、QUIRKS)は「IME の偽物」。`ime_key_sequence_golden` は戦略選択の表。journal replay は判断の入力の再生。役割が違い、重複していない。
- D5 の「同じテストを本物と偽物に流す」契約テストは、本物の側が windows-latest の実 IME(smoke の e2e)なので、費用が大きい。**最初は QUIRKS の各項目に「本物で観測した記録(e2e のログ)」への参照を付ける程度にとどめる**のが現実的。全面的な契約テストは後段。

### Q8: enforcement の最小形

- **コンパイラで守れる範囲は思ったより広い。** `with_app` は `lib.rs` で `#[cfg(windows)]`(`:224-238`)。`crate::hook`・`crate::win32`・`crate::imm`・`runtime` もモジュールごと gated。だから ungated なモジュールがこれらを参照すると、Linux のビルドが落ちる(#492〜#495 で実証)。
- テキスト走査が要るのは、**壁時計の直接読み取り(`Instant::now()`・`SystemTime::now()`)と、可変のグローバル(`static` + atomic/Mutex、`thread_local!`)だけ**。
- ただし既存の ungated の `state/` で、本番コードに次がある(`#[cfg(test)] mod tests` より前で数えた)。
  - `Instant::now()`: `state/ime_model.rs:435`(`effective_open()`)・`:504`、`state/hub_clock.rs`(2。Wall の実装なので正当)、`state/ime_event_log.rs`(1)
  - 可変の static: `state/probe_admission.rs` の `REJECTION_COUNTERS`
  - 不変の static: `state/ime_profile_driver.rs` の `IMM_CROSS_DRIVER`/`TSF_NATIVE_DRIVER`。不変のディスパッチ表で、問題ない(ADR-164 の C-immutable)。
- → `CORE_MODULES` を宣言してガードを入れるなら、**初期の許可リストにこれらを載せる必要がある**。載せないと導入した瞬間に落ちる。
- ADR-218〜220 との関係: 見送られたのは「GuardRule 宣言テーブル + 汎用チェッカー」で、ガード全体を作り直す案だった。**定数 1 つ + テスト 1 本**は、既存の `DECISION3_FILES` と同じ流儀なので矛盾しない。新しい汎用機構は作らない。
- **追加の注意**: Q4 の推奨案のように、core のファイルの中に `#[cfg(windows)]` の殻を書くと、テキスト走査を逃れる経路になる。殻は**別ファイル(gated な mod)に置く**ことをルールにする。そうすれば「core のファイルに `#[cfg(windows)]` の項目が無い」も同じガードで検査できる。#493 の `#[cfg(any(windows, test))]`(dead_code を避けるための属性)は許可する。

### Q9: 実装の順

- 段階 2(`platform_state` の ungate)を F の分割より前に置くのは**正しい**。
  - 閉ループの写し 7 系統のうち 5 系統(`apply_key_effect_prediction`、`effective_open_at`、`warrant_context`/`issue_actuation_order`、`record_explicit_intent`/`write_*`、`arm/follow_external_change`)が本物の呼び出しになる。
  - F の分割で作る `decide_*` の置き場所も整う。
  - 残る 2 系統(`kp_stage_key_effect_track`/`kp_predict_key_effect`、`ir_apply_drift_correction` の前半)は runtime/ にある(round1 の誤りの訂正どおり、7 → 1 ではなく 7 → 2)。
- 段階 2 の前提(r1 §5-2)は、#494・#495 がマージされれば `HwndImeSnapshot` と journal が片付く。残りは次の 4 つ。
  - (i) `ForegroundScope` の型を ungated へ移す(`win32.rs:46`、`pub use` で再公開)。
  - (ii) `crate::observer::ime_observer::{ImeUpdate, ImeObs}` を移す(`platform_state.rs:1191`。observer はモジュールごと gated)。
  - (iii) Q4 の核と殻の分割。
  - (iv) `new()` を `#[cfg(windows)]` にし、ungated な `with_clock` を足す。
- **より安全な順**: (i)(ii) → (iii)(まだ gated のままで分割だけ。挙動は不変で windows-build で確認できる)→ (iv) と ungate → ハーネスの写しを 1 系統ずつ本物に置き換える。

### Q10: 再発ファミリーを悪化させる恐れ

| ファミリー | 恐れ | 手当て |
|---|---|---|
| actuation 合流点(ADR-119、BUG-90) | open_chain を Cmd の状態機械にすると、3 関数それぞれでの再検出と B-1 の fail-open が崩れる | Q1 のとおり書き換えない |
| defer/replay キュー(ADR-156) | `OutputActiveGuard` を Cmd にすると、defer 側と drain 側の片方だけを配線する事故(ADR-123 → 128)の型 | F-D4 の記述どおり 2 窓口を必ず対で。最後の段階に置く |
| warmup | スナップショットを引数にすると判定が最大 10ms 古くなる(epoch fence は 20ms)。baseline を `SendInput` の前に取る順序(BUG-027/029/030/033、ADR-079) | 段階 4 で、順序をテストで固定してから |
| IME belief | `platform_state` の ungate と核・殻の分割は、`foreground_scope()` を読む時点を同じに保てば意味は変わらない | 殻は「読んでから即 `_in_scope`」の 1 行に限る |
| 物理キー押下ラッチ | フックの薄型化(H)で、リングに載らない経路が `physical_key_state` を更新する(a1) | ADR-229 D6 のとおり別段階 |
| conv mode、focus 遷移 | `on_focus_process_changed`(307 行)、conv_actuation の書く → 確認 → 再試行 | Q2 の handler 例外で書けるか、先に確かめる |

### Q11: 実害の記録が弱いまま進める妥当性

- #492〜#495 で、**挙動を変えずに 76 本のテストが Linux で回るようになった**(19 + 5 + 30 + 22。#494 の 6 本は新規)。ガードは壊れなかった。BUG の発見は 0。費用が小さいことは実証された。効果(見逃しの減少)はまだ実証されていない。
- 根拠として足りないのは、「Linux で回るようになったテストやガードが、実際に回帰を捕まえうる」ことの証拠。代わりに使える測れる指標を 2 つ挙げる。
  1. **「Linux で実行されないから」を理由にしたテキストガードの数**(`architecture_guard.rs:1293-1300` の BUG-51 の配線、`:4855` の bug116、`:5653` の bug173 など)。ungate の後、本物のテストに置き換えて減らせる数。
  2. **閉ループの写し 7 系統**が、本物の呼び出しに置き換わった数。
- ADR-224 との関係: 段階 2 は ADR-224 の案A に当たる。ADR-224 が案A を後回しにした理由は、(a) ガードが広く壊れる(根拠なし。ADR-224 に追記済み)と、(b) `foreground_scope` を stub にすると挙動が隠れる、の 2 つ。(b) は Q4 の推奨案で解消する。**ADR-224 の決定を「段階 2 は Q4 の形で進める」に改訂する**のが筋(所有者の判断)。

### Q12: 欠けている論点

1. **「ungated」と「純粋」を分けた 2 段の定義**(B1)。
2. **時間軸が 2 つ**(`Instant` と tick)あること(Q3)。
3. **フックスレッドは別スレッド**: turn の概念はエンジンスレッドだけに当てはまる。フックの同期判定(通す/消す)と、ADR-129 のスナップショットの埋め込みは、turn の外にある。
4. **`with_app` の再入**: 「turn の入口で 1 回借りる」は、`spawn_local` のタスクが再入で `None` を受ける経路(B-1 の fail-open、`dispatch_engine_message` の非対称)と衝突しうる。D3-4 を後段に回すのは妥当だが、そのとき何を守るかを書いておく。
5. **Cmd id と世代**(F-D5-2): 新しい id を発明しない。既存の `focus_gen`・`ApplyGeneration`(ADR-106)・`PressId`(ADR-208)・`cold_seq: Generation` を再利用し、対応表を ADR に置く。
6. **replay の入力**: FCIS の価値(replayability)は、journal が判断の入力を記録しているかで決まる。`KeyInput` は `plan_core` の入力(profile・`shadow_toggled`・kind)を持たない(前回の私の B′)。core を作るたびに、その入力が journal から復元できるかを確かめる。
7. **型状態(ADR-090)**: `Actuation<Warranted>` → `Verified` は既に「core が効果を要求できる条件」を型で表している。FCIS の Cmd 化で失わないこと(Q1)。
8. **mutants の `examine_globs` の維持**: ungate したファイルを `.cargo/mutants-awase-windows.toml` に足す運用(T1 の S2 と同じ)。

## 2. ドラフトへの指摘

| ID | 重大度 | 指摘 | 直し方 |
|---|---|---|---|
| B1 | **Blocker** | §1 の core の定義(時刻・グローバルを持たない)と F-D6(ungate すればコンパイラが保証)が、2 つの別の性質を混同している。既に ungated の `state/` に、`Instant::now()`(`ime_model.rs:435, 504`、`ime_event_log.rs`)や可変の static(`probe_admission.rs` の `REJECTION_COUNTERS`)がある。このまま ADR にすると、F-D6 のガードは導入した時点で落ちる。§5 の「core に移す」の意味も曖昧になる | 2 段に分ける。**Tier-1「portable」**: ungated(Linux でコンパイルとテストができる。コンパイラが守る)。**Tier-2「pure core」**: Tier-1 に加えて、壁時計の直接読み取りと可変のグローバルが無い(テキスト走査で守る。初期の許可リストは Q8 の 5 件)。段階 0〜2 は Tier-1 の作業、F の分割は Tier-2 の作業と明記する |
| M1 | **Must** | F-D3 の時刻の行: `HubClock` を「新設しない/例外」としているが、`HubClock` は既にある注入の仕組みで、本番で読むのは 4 か所。`Instant` の時間軸の記述も無い | Q3 のとおり、「状態を持つ core は注入された clock(`HubClock`)、状態を持たない関数は `now` 引数。壁時計の直接読み取りは shell だけ」に書き直す。時計の抽象を 4 つ目にしない |
| M2 | **Must** | F-D1 が `run_chain(_async)<W>` の扱いを保留している(Q1)。保留のままだと、F の分割の実装者がチェーン走査を Cmd 化しに行く恐れがある(BUG-34・ADR-119 の領域) | handler trait の例外を F-D1 に明文化する(Q1・Q2)。対象を列挙し、例外に入れる条件は「途中の結果で次の手を決める core アルゴリズム」に限る |
| M3 | **Must** | F-D1 の「S/B は shell が先に読んで Facts にする」は、条件付きの B 読み(ImmCross Failed 後の再読み取り、`romaji_pre_write`、focus の段階的な分類)で破綻する。先に読むと余計なプロセス間呼び出しが増え、Cmd にすると F-D5-1 の窓ができる | Q2 のとおり、読み取りの handler を例外に含める。F-D5-1 と矛盾しないことを ADR の本文で示す |
| M4 | **Must** | 段階 2 の方法(§5-2)が「`ForegroundScope` の型移動と引数化」「`ImeStateHub::new(tick_fn, scope_fn)`」のまま。関数ポインタ注入では ADR-224 の懸念が解消しない(`fn()` は状態を捕まえられない) | Q4 の「核(`_in_scope`)と殻(gated な別ファイルの `impl ImeStateHub`)の分割」に変える。呼び出し元の変更は 0、`:1382` も通る形 |
| S1 | Should | F-D2 の「既にある実例」に、`ObservedState`/`FocusFacts` を「所有・Copy」として挙げている。`FocusFacts<'a>` は借用、`ObservedState` は gated な `crate::tsf::observer::ActiveImeKind` を持ち、構築時にグローバル(`candidate_was_seen()`)を読む(`state/ime_decision_view.rs:21, 52, 87`、ファイルごと gated) | 実例から外し、S-D(借用ビューの所有化)の対象として挙げ直す。正しい実例は `DecisionInputs`・`decide_*`・`plan_core`・`explicit_press_delivery`・`ImeModel::reduce` |
| S2 | Should | F-D6 の「ungate はコンパイラが保証」は、ungated のファイルの中の `#[cfg(windows)]` 項目(dump 関数、`suppress_reason` の cfg など)を見ていない。殻を core のファイルに混ぜると抜け道になる | Q8 のとおり、殻は gated な別ファイルに置くルールにし、「core のファイルに `#[cfg(windows)]` 項目が無い(`any(windows, test)` は可)」をガードに含める |
| S3 | Should | F-D5-2 の「Cmd id」が新しい仕組みに読める | 既存の世代(`focus_gen`・`ApplyGeneration`・`PressId`・`cold_seq`)の対応表にする(Q12-5) |
| S4 | Should | F-D3 の「HWND/HIMC → `HwndId`」で、HIMC の扱いが書かれていない(HIMC はプロセス内のハンドルで、core に渡す意味が無い) | HIMC は shell の中に閉じる(core に出さない)と明記する |
| S5 | Should | §5 の順に、`decision3_…` の instrument のガード、mutants の `examine_globs`、DECISION3/pre-push/fix-requires-evidence の表の追随(#493 の S2 と同じ型)の手順が無い | レシピ R1〜R6 の共通の後処理として、「表・pre-push・mutants・instrument の一覧を見直す」を 1 行足す |
| N1 | Nit | §0-3・4(フックの薄型化、追記して reduce)は「後段」とあるが、F-D5 と重なる部分(turn)がある | turn がエンジンスレッドだけに当てはまることを明記する(Q12-3) |
| N2 | Nit | R4 の例に「T4 `SentKeyEvent`」とある | #495 で journal へ移った事実に合わせ、「使い手(journal)の側へ移す」と書く(依存の向きの改善例として) |

## 3. 次のラウンドで直すべき点(収束のための一覧)

1. B1: Tier-1/Tier-2 の 2 段の定義を §1 に入れ、§4・§5 の各レシピと段階がどちらの Tier の作業かを書く。
2. M1: F-D3 の時刻の行を Q3 の形に書き直す(`HubClock` を標準形にし、`Instant` と tick の 2 軸を明記し、時計の抽象を増やさない)。
3. M2・M3: F-D1 に handler trait の例外を明文化する(定義・条件・対象の列挙: `run_chain(_async)`、条件付きの B 読み)。F-D5-1 との整合を示す。
4. M4: 段階 2 の方法を Q4 の核・殻の分割に変え、前提 4 つ(Q9 の (i)〜(iv))と順序を書く。ADR-224 の改訂(所有者の判断)を未決定に入れる。
5. S1〜S5 の反映。
6. Q8 の enforcement の初期の許可リスト(5 件)と、殻を別ファイルにするルールを F-D6 に書く。
7. 指標を足す: 「Linux で実行されないことを理由にしたガードの数」、閉ループの写しの数、Tier-2 の許可リストの件数。

## 4. 最初の実装タスク案(3〜5 個)

いずれもローカルでビルドしない(CI のみ)。挙動不変。1 タスク = 1 PR。

| # | 単位 | 内容 | 成否の判定 | 取りやめ条件 | CI |
|---|---|---|---|---|---|
| P1 | 型移動 | `ForegroundScope`(`win32.rs:46-60`)を ungated(例: `state/foreground_scope.rs`)へ移し、`win32.rs` で `pub use`。`ImeUpdate`/`ImeObs`(`observer/ime_observer.rs:22-45`)を ungated(例: `state/ime_update.rs`)へ移し、`observer::ime_observer` で `pub use` | Linux と Windows でビルドが通る。新しい警告が 0(pub なので出ない見込み)。呼び出し元の変更 0 | 型に gated な依存が見つかる(`ImeUpdate` は `InputModeState` などだけのはずだが**未確認**) | test・windows-cross-check・windows-build |
| P2 | 核と殻の分割(まだ gated) | `platform_state.rs` の 11 メソッドすべてに `_in_scope(scope)` 版を用意し(5 つは既存)、`foreground_scope()` を読む殻を `#[cfg(windows)]` の別ファイル `state/platform_state_shell.rs` の `impl ImeStateHub` に移す。殻は「読む → `_in_scope` を呼ぶ」の 1 行だけ | windows-build の lib テスト(`platform_state` の 48 本)と `architecture_guard`(`:3767-3775` の呼び出し元の固定)が green。runtime/ の呼び出し元の差分 0 | 殻が 1 行で書けないメソッドが出る(スコープを 2 回読む、など) | windows-build・test(ガード) |
| P3 | 時計の構築の分割 | `ImeStateHub::new()` を `#[cfg(windows)]` にし(本体は `HubClock::wall(crate::hook::current_tick_ms)` のまま、`platform_state.rs` 内)、ungated な `with_clock(clock: HubClock)` を足す | `architecture_guard.rs:1382` が書き換えなしで green | `new()` を使う ungated のコードがある | 同上 |
| P4 | 段階 2 本体 | `state/mod.rs:186-188` の `platform_state` の gate を外す(前提: #494・#495・P1〜P3 がマージ済み) | Linux で `state::platform_state::tests::*` が回る(48 本の見込み)。Linux のビルドで新しい警告が無い(`pub(crate)` で Linux で未使用の項目には `#[cfg(any(windows, test))]`。`allow(dead_code)` は増やさない) | `allow` を足さないと消えない警告が 3 件を超える。または Linux で落ちるテストがある(BUG として扱う) | test・windows-* |
| P5 | ハーネスの写しの置き換え(1 系統ずつ) | 最初は `effective_open_at`(写しが最も短く、`HubClock::Manual` で仮想時計とそろえられる)。`tests/support/harness.rs` の写しを本物の `ImeStateHub` の呼び出しに置き換える | `closed_loop_scenarios` の全シナリオの結果が不変。ハーネスの写しが 7 → 6 | 本物を呼ぶと結果が変わる。これは写しのずれの発見なので、ADR-224 の着手条件を満たす実例として記録して止める | test(`--test closed_loop_scenarios`) |

**並行して進められる別の線**: ReplayWriter(Q1 の例外の実証。テストだけで本番の変更 0)。前回の私の A′ で、再生できない分岐 4 → 1。P1〜P5 とファイルが重ならない。
