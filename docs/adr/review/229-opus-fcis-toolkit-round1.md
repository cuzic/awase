---
id: ADR-229-companion-229-opus-fcis-toolkit-round1
title: |-
  ADR-229 FCIS 汎用部品(toolkit)opus-adversarial-consult round1
type: companion-doc
related_adr:
  - "ADR-229"
---

# FCIS の汎用部品(toolkit)の設計相談 round1

- 実測の基準: `origin/develop` = `4b559a35`。読み取りのみ。件数は `git grep` による。

## 0. 結論(先に)

- **rule of three を満たし、かつ取り出す価値があるのは 1 つだけ: 「fixture の再生ハーネス」(候補 4 の一部)。** それ以外の候補は、次の 3 つのどれかに当たるので**作らない**。
  1. 既にある(候補 2 の `timed_fsm::Response`、候補 6 の `StepCoro`、候補 8 の `CORE_MODULES`)。
  2. 使い手が 1 つ以下(候補 3 の sync/async の対、候補 5 の時計、候補 7、候補 9)。
  3. 形は 3 回以上出るが、ドメインごとに意味が違い、汎用の型にすると意味を消すだけ(候補 1、候補 3 の handler trait、世代の比較)。
- **最大の見落としは、`timed-fsm` が既に「汎用部品の箱」で、awase はその半分を使っていないこと。** awase が使っていないのは `Clock`/`MonotonicClock`/`ManualClock`、`ActionExecutor`/`AsyncActionExecutor`/`TimerRuntime`(後述)。新しい toolkit を作る前に、なぜ使っていないのか(Win32 のメッセージループや `HubClock` の 2 軸に合わないから)を認めること。そうしないと、同じ形の 2 つ目、3 つ目を作るだけになる(候補 5 の「4 つ目の時計」と同じ轍)。
- **順序の推奨**: toolkit を先に作らない。F1(`ir_decide_read_strategy` の分割)で `decide` の再生 fixture を足すとき、同じ PR で再生ハーネスを取り出す。新しい使い手が出る瞬間に取り出せば、抽象が実際の使い方に合っていることを確かめられる。

---

## 1. rule of three の判定表(実測)

| # | 候補 | 同じ形の箇所(awase で) | 判定 |
|---|---|---|---|
| 1 | Facts / Snapshot の共通の約束 | 所有型の入力は多い(`DecisionInputs`、`PredictInput`、`TsfEnvSnapshot`、`ProbeTickInput`、`ObservedState`、`WarrantContext<'a>`…)。**derive がばらばら**: `DecisionInputs` は `Copy + serde`(`ime_actuation_decision.rs:44`)、`TsfEnvSnapshot` は `Copy + Default` で serde 無し(`probe_fsm.rs:68`)、`ObservedState` は `Clone + Copy` だけ(`ime_decision_view.rs:41`)、`WarrantContext<'a>` は借用(`open_warrant.rs:115`)。**これらを総称として受け取る汎用コードは 0** | **作らない**(使い手が無い trait は約束にならない) |
| 2 | Plan / Cmd | `timed_fsm::Response<A, T>` は既に awase の 16 ファイルで使われている(`Response` の import 12 + `{Response, …}` 5、`TimerCommand` 10 余り。engine の NICOLA FSM、`tsf_gate`、`gji_fsm`、`gate.rs` ほか)。`ProbeAction`(warmup)と `MechanismCommand`(actuation)はドメインの enum | **既にある**(`Response`)。新設しない |
| 3a | handler trait(core に trait、shell に本番の実装、テストに偽物) | **形としては 6 か所以上**: `MechanismWriter`/`AsyncMechanismWriter`(`actuation_chain.rs:613/621`)、`ProbeIo`(`output/probe_io.rs:23`、`FakeProbeIo` `:738`)、`WarrantJudge`(`explicit_press.rs:685`、偽物 3)、`GjiSyncSink`(`gji_direct_mechanism.rs:130`、`RecordingSink`)、`ImeDriver`(`awase-keymap-learn/exec.rs:83`、Real/Sim)、`InjectionSender`(`output/sender.rs:8`、3 実装) | **形は 3 回以上出るが、汎用の型は作らない**。シグネチャはどれもドメイン固有で、`trait Handler<Cmd> { fn run(&mut self, c: Cmd) -> Ev; }` に寄せると型の意味(例: `write(mechanism, open) -> ImeOpenOutcome`)が消える。**取り出すべきものは型ではなく規約**で、ADR-229 F-D1 の handler の例外(閉じた列挙・偽物が必須)として既に書けている |
| 3b | sync/async の二重化 | awase の中で**1 つだけ**: `MechanismWriter`/`AsyncMechanismWriter` と `run_chain`/`run_chain_async`(`actuation_chain.rs:549/580`。判定は `classify` の 1 か所に集約済み)。`timed_fsm` の `ActionExecutor`/`AsyncActionExecutor`/`TimerRuntime` は、**awase の中に実装が 0**(実装は timed-fsm 自身のテスト・doc・`tokio_support.rs:122` だけ) | **作らない**(使い手 1)。ADR-123 → 128 の事故は defer と drain の 2 窓口の話で、sync/async の二重化の事故ではない(混同しないこと) |
| 4a | fixture の再生ハーネス(dir を読む → 型にする → 純関数を通す → 失敗を集める → 0 件を拒む) | **4 か所**、ほぼ同じ 20 行前後の手書き: `tests/journal_replay.rs` の `load_fixtures`(`:35`)と `read_dir`(`:48`、ConvClassify)・2 本目(`:199-215`、ImeEventReplay)、`tests/drift_correction_replay.rs` の `load_fixtures`(`:52`)・`fixture_dir`(`:59`)・`read_dir`(`:72`)・`assert!(total > 0)`(`:131`)、`state/actuation_decision_record.rs` の `fixture_dir`(`:1269`)・`load_fixtures`(`:1273`)。**実害の記録もある**: `drift_correction_replay.rs:10-15` が、`journal_replay.rs` が `tests/journals/` 直下の全 `*.json` を無条件にパースするので、別の形式を同じ階層に置くと衝突する。だからサブディレクトリにした、と書いている(ハーネスごとの約束が暗黙で、衝突が実際に起きた) | **作る**(唯一の該当)。§3 |
| 4b | 再生の判定(`decide(facts)` を再計算して記録と比べる) | 4 つとも、比べる中身がドメインごとに違う(conv の遷移、drift の打ち切り、actuation の gate/chain/command/走査、ime_apply の世代) | **作らない**。判定はハーネスに渡す closure のまま |
| 5 | Clock | 抽象は 3 つあるが、**それぞれ使い手は 1 か所**: `HubClock` は `ImeStateHub`(`platform_state.rs` で 4 か所)と `ime_event_log`。`quanta::Clock` は `journal.rs` だけ(24 行)。`timed_fsm::Clock`/`ManualClock`/`MonotonicClock` は awase の中に**使い手 0** | **作らない・統合しない**。統合の利益は使い手 3 つの見通しの改善だけで、`HubClock` の 2 軸(`Instant` + tick)を `timed_fsm::Clock`(`now_ms` だけ)では表せない。`journal` の `quanta` は `DumpTriggerTracker` のテストの mock で使っていて、`HubClock::Manual` に寄せても行数も意味も変わらない |
| 6 | Coroutine(`StepCoro` の複数 effect 化、`start()`/`resume(reply)` 型) | `StepCoro` の使い手は 3(`gji_warmup_coro`・`ms_ime_ready_coro`・`probe_fsm`)で、**既にある**。request/response 型の連鎖用の型を求める使い手は 0(open_chain は async fn で、書き換えない方針。ADR-229 F-D5-3) | **作らない**。以前の判断(往復は `yield_step(ch, vec![Action]).await` の 1 行、prime で解決済み、`vk_sent` の欠落という意味のある分岐を隠す)と変わらない |
| 7 | Turn / Driver、`Tagged<T>{gen, id}` | 汎用の駆動ループは 0。世代は、`Generation(u64)`(`event_origin.rs:60`、cold_seq)、`ApplyGeneration(NonZeroU64)`(`generation.rs:26`)、`PressId(u64)`(`src/types.rs:227`)、生の `u32` の `focus_gen`(`focus_gen: u32` などの生のフィールドが 22 か所)。「捕まえた世代と今を比べる」は 3 回以上あるが、どれも 1 行の `==`(`ime.rs:1042` `verify_gen_only`、`focus_resync.rs` の `open_if_current`、`output/mod.rs` の confirm gate の世代) | **作らない**。`Tagged<T>` はドメインの区別(フォーカスか、反映要求か、押下か)を消す。代わりに価値があるのは、**生の `u32` の `focus_gen` を newtype にする**こと(S-C と同じ型付け)。toolkit ではなく個別の PR で |
| 8 | 純粋さのガードの汎用化 | `CORE_MODULES` は 1 か所(`layer_boundary_guard.rs`、#498) | **作らない**。`state/` 以外(`tsf/`・`focus/`)へ広げるときに、同じテストの対象ディレクトリを足すだけ |
| 9 | 契約テスト(偽物と本物に同じシナリオ) | 0 | **作らない**。ADR-229 D5 のとおり後段。最初は QUIRKS の項目に e2e の観測記録を紐づける程度 |

---

## 2. 置き場所(4a だけが対象)

| 案 | 評価 |
|---|---|
| (a) timed-fsm に足す | 不適。テキストの fixture を serde で読むのはタイマー FSM と無関係で、依存ゼロの約束(serde が要る)と、crates.io の API の約束を両方崩す |
| (c) ルートの `awase` クレート | 不適。本番の crate にテスト専用のコードを入れることになる |
| (d) `awase-windows` の ungated モジュール | 不適。`#[cfg(test)]` にすると、統合テスト(`tests/*.rs`)からは見えない。`pub` にすると、本番の API にテスト専用のものが載る |
| **(b) workspace の非公開の小さな crate**(例: `crates/awase-replay`、`publish = false`、依存は `serde` と `serde_json`)を、`awase-windows` の **dev-dependency** にする | **推奨**。dev-dependency は src の `#[cfg(test)]` の単体テスト(`actuation_decision_record.rs` のように `pub(crate)` の型を使うもの)と、`tests/*.rs` の統合テストの**両方**から使える。本番のバイナリには入らない |

- timed-fsm への昇格の道筋は**持たない**(性質が違う)。将来 `awase-linux`/`awase-macos` でも fixture の再生が要るようになったら、同じ dev-only の crate を共有する。
- **Linux の CI**: dev-only の crate は Linux の `cargo nextest run --workspace --lib` でビルドされる。`cargo machete` の誤検出(dev-dep の未使用の判定)に注意。最初の使い手と同じ PR で入れれば避けられる。

---

## 3. 最小の API スケッチ(4a のみ)

```rust
/// `dir` 直下の `*.json`(パス順)を `Vec<T>` として読み、各要素を `check` に通す。
/// fixture が 0 件なら失敗。失敗は「ファイル名 / 添字: 理由」で全件を集めてから 1 回で報告する。
pub fn replay_dir<T: serde::de::DeserializeOwned>(
    dir: &std::path::Path,
    mut check: impl FnMut(&T) -> Result<(), String>,
) -> ReplayReport;

pub struct ReplayReport { pub files: usize, pub cases: usize, pub failures: Vec<String> }
impl ReplayReport { pub fn assert_ok(&self) { /* 0 件の拒否 + failures の表示 */ } }
```

- **1 ディレクトリ = 1 形式**を規約にし、doc に書く(`drift_correction_replay.rs:10-15` の衝突を構造で防ぐ)。
- `journal_replay.rs` が `tests/journals/` 直下を読む件は、**直下の JSON を `tests/journals/conv_classify/` へ移す**ことで解消する(ファイルの移動だけ)。

**作らないものと、作る条件**

| 候補 | 作る条件(何が起きたら) |
|---|---|
| 1 Facts の trait | Facts を総称として受け取る汎用コード(例: 任意の `decide` を同じ形で journal に記録する仕組み)が、実際に 3 つ現れたとき |
| 3b sync/async の共通化 | awase の中で sync/async の対が 3 組に増えたとき、または片方だけを直した事故が記録されたとき |
| 5 Clock の統合 | `timed_fsm::Clock` を awase で使う箇所が現れ、`HubClock` と同じ値を読む必要が出たとき |
| 6 StepCoro の拡張 | 1 回の yield で別の種類の応答を待ち分ける必要がある warmup が 3 つ目として現れ、今の `Vec<ProbeAction>` + `ProbeTickInput` の形で書けないと示せたとき |
| 7 `Tagged<T>` | 世代の比較の誤り(取り違え)が BUG として記録されたとき。それでも先にやるのは newtype |
| 9 契約テスト | ADR-229 D5 の段階 |

---

## 4. 作る場合の効果(4a)

- **見通し**: 再生ハーネスの約束(1 ディレクトリ 1 形式、0 件の拒否、失敗の全件報告)が 1 か所に書かれる。今は 4 か所に暗黙で書かれていて、1 つ(journal_replay)は「直下を全部読む」という、他と衝突する約束を持っている。
- **replayability**: FCIS の F の分割では、`decide` を出すたびに journal から再生できるかを確かめる(タスク表 §3 の共通の注記)。新しい再生の family を足す費用が、型 1 つ + closure 1 つに下がる。F1〜F6 で 3〜6 個の新しい family が見込まれる。
- **複雑性**: 同じ手書きのループ 4 つ(+ 今後の分)を 1 つにする。判定(4b)は closure に残すので、汎用化で意味が消えることは無い。
- **過去の轍との比較**
  - ADR-218(GuardRule 宣言テーブル): 見送られた理由は、汎用チェッカーがガードの意味を変えることと、削減量が小さいこと。4a は判定の汎用化をしない(closure のまま)。読み込みと集計だけを共通にするので、意味は変わらない。
  - ADR-219(シナリオの DSL): DSL を作らない。
  - ADR-180 決定2(レコード組み立ての統一): 本番のレコード組み立てには触れない。テスト側だけ。
  - ADR-053: `StepCoro` に触れない。
  - **ただし削減量自体は小さい**(4 か所 × 約 20 行)。正当化は削減量ではなく、「F の分割で増える使い手」と「暗黙の約束の衝突の実害(drift のサブディレクトリの経緯)」。F の分割が止まるなら、作る価値は薄い。

---

## 5. 順序(推奨は 1 つ)

**toolkit を先に作らない。F1 の PR で、F1 の `decide` の再生 fixture を足すのと同時に `awase-replay` を作り、既存の 4 か所を移す。**

- 理由: 抽象は、新しい使い手が実際に書かれる瞬間に、その使い手と既存の 4 つの両方に当てはまるかで確かめられる。先に作ると、仮定の使い手に合わせた API になる。
- P2〜P5(platform_state の核と殻、ungate、ハーネスの写しの置き換え)は fixture の再生と無関係なので、並行して進めてよい。
- 既存の 4 か所を移すのは同じ PR でも、直後の PR でもよい。ただし `journal_replay.rs` の直下の JSON をサブディレクトリへ移すのは、移行と同じ PR で行う(1 ディレクトリ 1 形式の規約を、同時に成り立たせるため)。

---

## 6. 反論・見落とし・欠けている論点

1. **「汎用部品」の前に、timed-fsm の使っていない半分の扱いを決める。** awase が `timed_fsm` の `Clock` 系と `ActionExecutor`/`AsyncActionExecutor`/`TimerRuntime` を使っていないのは、Win32 のメッセージループ(`win32_async::spawn_local` とタイマー ID)や `HubClock` の 2 軸と合わないから(**未確認**だが、使い手 0 の事実と整合する)。新しい toolkit で同種の trait を作ると、これらと二重になる。toolkit を検討する前提として、「awase は timed-fsm の FSM の型(`Response`・`TimerCommand`・`TimedStateMachine`・`StepCoro`)だけを使い、駆動側の trait は使わない」と ADR に書いておく。
2. **「形が同じ」と「同じもの」を混同しない**(memory の DSL の教訓と同じ)。handler trait は 6 か所あるが、それは「規約が守られている」ことの証拠で、共通の型が要る証拠ではない。汎用の `Handler<Cmd>` を作ると、どの trait にも当てはまり、どの trait の意味も表さない型が増えるだけ。
3. **toolkit が重複を増やす恐れ**: 候補 5(時計)と候補 3b(sync/async)は、作れば既存の抽象の 4 つ目・3 つ目になる。ADR-229 の「時計の抽象を 4 つ目に増やさない」は、ほかの候補にも同じ原則として広げる(「既存の抽象に使い手 0 の半分があるうちは、同じ役割の抽象を新設しない」)。
4. **実際に価値がありそうな「汎用でない」改善**: (i) 生の `u32` の `focus_gen` の newtype 化(22 か所の生のフィールド。取り違えを型で防ぐ。S-C と同じ)、(ii) Facts のうち replay に使うものに `serde` を足す(F の分割で `decide` を出すときに、その入力型にだけ足す。共通の trait は作らない)。どちらも toolkit ではなく、F の分割の各 PR の中で行う。
5. **所有者の方針(「StepCoro に限らず汎用部品全体を考える」)への答え**: 考えた結果、汎用部品として取り出す根拠があるのは再生ハーネスだけで、残りは既にあるか、まだ早いか、型にすべきでないもの、という結論を、判定表とともに所有者に示すのがよい。「作らない」と、「何が起きたら作るか」(§3 の表)をセットで記録しておけば、同じ提案が繰り返されたときの歯止めになる(experiments.md と同じ考え方)。
