---
title: FCIS・代数的 Effect・monad 風の書き方で可読性を上げる案の検討(検討のみ)
created: 2026-10-06
base: origin/develop 3c8ef62a(Opus レビューは 0e4fccb7 で照合。relay_plan.rs・execute_relay は変わっていない)
status: 検討のみ。コードは変えていない。Opus レビュー round1(Blocker 0・Must 3・Should 6・Nit 5)を反映済み。所有者の判断待ち
related_adr: ["ADR-229", "ADR-218", "ADR-219", "ADR-220", "ADR-090", "ADR-156", "ADR-180"]
---

# FCIS・代数的 Effect・monad 風の書き方で可読性を上げる案の検討

所有者の依頼(2026-10-06): 「ここまでの経験、知見を元にして、もっと FCIS と代数的 Effect、Haskell Monad 的な DSL による可読性向上を考慮して、改善できるアイデアを考えて」。実測は [evidence.md](evidence.md)。

## 1. 結論

- **monad 風の型・combinator・マクロの DSL は勧めない**。
  1. 「途中で決まったら抜ける」(Haskell の `Maybe`/`Either` の do 記法で書くことの大半)は、Rust の `return`・`let … else`・`?` が既に持っている。実装済みの `decide_*` の早期 return は 2〜4 個で、上から下へ読める(evidence (b)-3)。自前の型に `?` を使う `Try` trait は stable Rust では実装できない。
  2. **抽象の層そのものが実害を作った記録がある**: BUG-027 は、委譲するラッパーが 1 メソッドだけ委譲し忘れ、trait の既定実装(何もしない)が黙って使われた不具合。汎用の Handler や combinator の層を足すと、この種類が増える。
  3. 却下済みの案(Plan/Effect-term、汎用の Handler/Facts ツールキット、宣言テーブル)と同じく、使い手が 1〜2 個の部品を足すことになる。
- **代数的 effect の考え方は、このリポジトリで既に 2 つの形で取り入れている**(§2)。① 効果をデータで返し、殻が解釈する(engine の `Decision`)。② 1 か所で割り込む(合流点の許可リスト + lint)。広げない理由も記録がある。
- **実害の記録があるのは、長い関数と、今のコードを説明していない古いコメントのほう**(evidence (a)(e))。優先順位はこの順にする: 第 1 に古いコメントの小さな整理(§4-1)、第 2 に `relay_plan` の形の見直し(§4-2、実害 0 件。作られて 1 日)。
- Haskell から借りて効くのは monad ではなく「代数的データ型(enum)で、起こり得ない組を書けなくする」ほう(案 A)。ただし書き方の約束として足すのは 1 文で済み、既存のコードを先回りして直す必要はない。

## 2. 代数的 effect への答え(既にある形と、広げない理由)

| 代数的 effect の考え方 | このリポジトリでの形 | 広げない理由 |
|---|---|---|
| 効果をデータとして返し、実行は別の解釈役(handler)が行う | ルート crate の `Engine` は `Decision { effects: EffectVec }` を返し(`src/engine/decision.rs:130-142`)、awase-windows の `DecisionExecutor::execute_relay`/`execute_one` が実行する。awase の FCIS の最初の境界 | awase-windows の中の IME 操作まで「効果の列(Plan)」にする案は、ADR-229 の review(`229-opus-effect-plan-round1.md`)で見送り済み。効果の中身が `await` の後に読み直した値で決まり(`fallback_write`)、`run_chain` と二重の表現になるため |
| 1 か所で割り込む handler(全経路に同じ条件をかける) | 合流点の許可リスト `lints/actuation_call_guard` の `RESTRICTED_CALLS`、`decide_gate`/`apply_mechanism` への集約(ADR-180 決定1)、`architecture_guard` の件数固定 | 実害(gate を 5 経路中 1 つにだけ置いた issue #136・ADR-119、defer 側だけに条件を足した ADR-123→128)への答えは、上の形で出ている。`with_app` を内包する共有の gate helper は、再入で gate が無効になる(issue #136/BUG-90 型)ため ADR-180 で見送り済み |
| 理由の記録(Writer) | 各 `decide_*` が 1 つの理由の enum を返し、journal に載せる(ADR-229 代案 A・E1) | 1 回の判断の理由は 1 つ。蓄積する型(`Decision<T, Reason>`)の使い手が無い |

## 3. 案の一覧

| # | 案(Rust で書ける形) | 判定 | 一言の理由 |
|---|---|---|---|
| C | **今のコードを説明していないコメントだけを消す**(経緯は ADR・known-bugs にある) | **推奨(第 1)** | 実害の記録がある(BUG-027 の副因、BUG-020 の経緯が今も関数の先頭に残る)。範囲を切った小さな試行 |
| A | 判断の結果を 1 つの enum にし、理由を variant の名前にする。「bool と、真のときだけ意味を持つ値」は `Option`/variant のデータにする | 書き方の約束に 1 文足す。既存コードは §4-2 の 1 か所だけ所有者に諮る | ルート crate の `Decision` の doc と同じ流儀。BUG-020 の本来の原因(結果を見ていない)を型で防ぐ方向 |
| 5 | 殻は判断の結果を 1 回だけ match し、腕の中で元の入力を見直さない | A とセット | `execute_relay` の 2 重の場合分けは F3 が持ち込んだ(F3 前は 1 回の match) |
| 2 | 事実を 1 つの引数(`Facts`)にまとめる(Reader 風) | 既に大半できている | 7 ファイル中 5 つ。残り 4 関数(`plan_drain_step`・`plan_defer`・`plan_drain_before_send`・`plan_focus_probe`)は、次に触るときに struct にすると全数表の行も名前付きになる |
| 4 | 型状態で順序を型にする(typestate) | 既にある・計画済み | `Actuation<Requested→Warranted→Verified>`(ADR-090)。並べ替えるとコンパイルが通らないので、走査テストを消せる(V5 の判定)。新設しない |
| 1 | 理由を蓄積する判定の型 `Decision<T, Reason>`(Writer 風) | 勧めない | §2 の 3 行目 |
| 3 | `reduce(state, event) -> (state, Vec<Reason>)` | 勧めない | 返した理由を読む使い手が 0 |
| 6 | 小さな combinator(`then`・`or_else`・`guard`) | 勧めない | 標準に `bool::then`・`Option::filter`・`is_some_and`・`let … else` がある(使用 45〜104 件)。層を足すと BUG-027 型が増える |
| 7 | `given(facts).when(event).then(plan)` のテスト builder | 勧めない | 部品を足さずに、表の行を事実の struct リテラル(名前付きフィールド)で書けば足りる(案 2 の残りと同じ作業)。ADR-219 がシナリオ DSL を見送った理由と同じ |
| 8 | マクロ DSL | 勧めない | ADR-218〜220 の判定を覆す実害の記録が無い |

## 4. 推奨の中身

### 4-1. 第 1: 古いコメントの小さな試行(案 C)

- **対象**: 今のコードを説明していないコメントだけ。例: `ir_apply_drift_correction` の先頭(`ime_refresh.rs:805-811`)は、もう無い早期 return(BUG-20 の追補)の経緯を書いている。
- **やらないこと**: 経緯のコメントを一律に ADR・known-bugs へ寄せること。コード内の `BUG-` 参照は再発防止の目印として意図的に置かれている(`fix-requires-evidence.md`)。前例として、`docs/known-bugs.md` の要約を 2 ラウンド試し、削減が少なく全て revert した(分割で対処した)。
- **形**: `runtime/` の長い関数 3〜5 本(evidence (a) の表の上位)で、「今のコードと食い違う」「もう無いコードを説明する」コメントを消し、1 PR にする。
- **取りやめ条件**: 対象の関数で消せる行が合計 30 行未満なら止める(数字は仮。所有者に確認)。どのコメントが古いかの判断がレビューで割れたら、その行は残す。

### 4-2. 第 2: `relay_plan` の形(3 つの選択肢と推奨)

`plan_relay` が決めているのは「物理キーが `Suppress` なら passthrough も reinject もしない」の 1 bit だけ。固定しているのは 6 行の全数表だけで、`architecture_guard` は `plan_relay` を参照しない(参照するのは e12 の `plan_consume_effect`・e13・c23 の閾値)。journal にも載らない(`journal-replay-rebuild-study-2026-10-06/inventory.md` の `plan_relay` の行)。理由の使い手は debug ログ 1 か所(`executor.rs:453`)。F3 前の `execute_relay`(`ce8de690^`)は `Decision` を 1 回だけ match し、腕の中で `physical == Suppress` を見る形だった。

| | 内容 | 純減(概算、コンパイルしていない) | 不利 |
|---|---|---:|---|
| (i) | 効果の列を持つ 1 つの enum(付録 A) | 30〜40 行 | `Effect` は `PartialEq` を持たない(`src/engine/decision.rs:67`)ので、全数表を `assert_eq!` で書けず `matches!` になる。ログに variant 名だけを出すには名前を返す `fn` が要る(撤去するはずの理由の enum を作り直すのとほぼ同じ) |
| (ii) | `RelayAction` と `RelayReason` を、効果を持たない Copy の 1 つの enum(単位 variant 5 つ)にまとめる | 20〜35 行 | 殻の 2 重の場合分けは残る(`matches!(plan.action, ..)` が `matches!(plan, ..)` になるだけ) |
| (iii) | `plan_relay` と入力・出力の型を撤去し、殻を F3 前の 1 回の match に戻す。`plan_consume_effect`・閾値の対(e12・c23)は同じファイルに残す | 約 130〜140 行(`relay_plan.rs` の 23-100 行 + 表のテスト約 50 行 + 殻の写し約 10 行) | F3 の「事実 → 理由つきの Plan → 殻」の形(タスク表で「4 回できた、書き方として固定」)の 1 例を戻す |
| (iv) | 何もしない | 0 | — |

**推奨: (iii)、所有者が F3 の形を保ちたいなら (ii)**。理由: 判断が 1 bit で、ガードも journal も参照しないので、型を 5 つ持つ得が小さい。(i) は `Effect` が比較できないことで得が縮み、F3 は前日に Opus レビュー済みでマージされたので、作り替えの費用(レビュー 1 回)に見合わない。どれを選んでも Consume の腕(e12 の検出器が `plan_consume_effect(matches!(effect, Effect::Timer(_)))` を要求)は変えない。`execute_relay`/`drain_deferred` は defer/replay キューの再発ファミリー(ADR-156)なので、`DrainStep` は同じ PR に入れない。

### 4-3. 書き方の約束(案 A)

新しい一覧は作らず、タスク表の「書き方の明文化」(`fcis-layering-tasks-2026-10-06.md` の「`decide_*` は理由を持つ Plan の enum を返す」)に 1 文足す: 「理由から一意に決まる実行の種類は、別の enum として並べず `fn` で導く。殻がすべての腕で読むフィールドは struct に残してよい」。後半の基準で、`drift_plan` の `DriftAct { notify_diagnostic, .. }`(`step` が `SkipWarrantWouldBlock` なら常に false だが、殻が共通に読む)と `ReadDecision` の `typing_guard_bypassed` は対象外と説明できる。`ReadDecision` の `strategy` は殻(`ir_stage_strategy`)が運ぶ粗い見方なので、`ReadReason` から導く `fn` にするのが筋(次にこのファイルを触るとき。再生 fixture 11 件の `"strategy"` の行を消す必要がある)。

## 5. 所有者に聞くこと

1. 古いコメントの小さな試行(§4-1)をやるか。取りやめの基準(30 行未満なら止める、は仮)をどうするか。
2. `relay_plan` を (i)(ii)(iii)(iv) のどれにするか。推奨は (iii)、F3 の形を保つなら (ii)。
3. 案 A の 1 文を、タスク表の「書き方の明文化」に足すか。
4. 代数的 effect は「効果をデータで返す(engine の `Decision`)と、合流点 + lint の形で既に取っている。汎用の handler は再入の実害で見送った」という整理(§2)でよいか。
5. monad 風の DSL(案 1・3・6〜8)は「やらない」でよいか。

## 6. 確認できなかったこと

- 純減の行数はどれも概算で、書いてコンパイルしたものではない(未確認)。
- §4-1 で消せる行数は数えていない(未確認)。
- `std::ops::ControlFlow` に `?` を使える点は Rust の仕様の知識で、このリポジトリでは試していない。
- `plan_relay` の `const fn` を外すことは、`crates/awase-windows/Cargo.toml` の `missing_const_for_fn = "allow"` と、`const fn` を要求するガードが無いことから、影響しないと判断した(コンパイルでは確かめていない)。
- 長い関数の数え方は粗い(evidence (a))。

---

## 付録 A: 案 A + 5 を `relay_plan` に当てた場合((i) の形)

**現在**(`state/relay_plan.rs:69-98` と `runtime/executor.rs:399-492` の要約):

```rust
// core: 2 つの enum を並べて返す
pub(crate) const fn plan_relay(facts: RelayFacts) -> RelayPlan {
    let suppress = matches!(facts.physical, PhysicalKeyDisposition::Suppress);
    match facts.kind {
        RelayDecisionKind::PassThrough => if suppress {
            RelayPlan { action: RelayAction::ConsumeSuppressed, reason: RelayReason::PassThroughPhysicalSuppressed }
        } else {
            RelayPlan { action: RelayAction::RunPassthroughPipeline, reason: RelayReason::PassThroughIdle }
        },
        RelayDecisionKind::PassThroughWith => RelayPlan {
            action: RelayAction::QueueFlush { reinject: !suppress },
            reason: if suppress { RelayReason::FlushPhysicalSuppressedNoReinject } else { RelayReason::FlushWithReinject },
        },
        RelayDecisionKind::Consume => RelayPlan { action: RelayAction::ConsumeEffects, reason: RelayReason::EngineConsumed },
    }
}

// shell: Decision を写す → 判断 → もう一度 Decision で match → 腕の中で plan を見直す
let kind = match &decision { Decision::PassThrough => PassThrough, Decision::PassThroughWith { .. } => PassThroughWith, Decision::Consume { .. } => Consume };
let plan = plan_relay(RelayFacts { kind, physical });
match decision {
    Decision::PassThrough => {
        if matches!(plan.action, RelayAction::ConsumeSuppressed) { return consumed(); }
        run_passthrough_pipeline(..)
    }
    Decision::PassThroughWith { mut effects } => {
        let reinject = matches!(plan.action, RelayAction::QueueFlush { reinject: true });
        if reinject { effects.push(ReinjectKey(*raw_event)); }
        queue.extend(effects); consumed_pending()
    }
    Decision::Consume { effects } => { /* Timer は即時、他はキュー(e12) */ }
}
```

**(i) 適用後**(擬似コード):

```rust
// core: 理由が variant。運ぶ効果の列も variant が持つ。Effect が PartialEq を持たないので Eq は derive できない
#[derive(Debug)]
pub(crate) enum RelayPlan {
    PassThroughPhysicalSuppressed,
    PassThroughIdle,
    FlushWithReinject { effects: EffectVec },
    FlushPhysicalSuppressedNoReinject { effects: EffectVec },
    EngineConsumed { effects: EffectVec },
}

pub(crate) fn plan_relay(decision: Decision, physical: PhysicalKeyDisposition) -> RelayPlan {
    let suppress = matches!(physical, PhysicalKeyDisposition::Suppress);
    match decision {
        Decision::PassThrough if suppress => RelayPlan::PassThroughPhysicalSuppressed,
        Decision::PassThrough => RelayPlan::PassThroughIdle,
        Decision::PassThroughWith { effects } if suppress => RelayPlan::FlushPhysicalSuppressedNoReinject { effects },
        Decision::PassThroughWith { effects } => RelayPlan::FlushWithReinject { effects },
        Decision::Consume { effects } => RelayPlan::EngineConsumed { effects },
    }
}

// shell: 1 回だけ match。ログに理由だけを出すには `plan.name()` のような fn が別に要る
match plan_relay(decision, physical) {
    RelayPlan::PassThroughPhysicalSuppressed => consumed(),
    RelayPlan::PassThroughIdle => run_passthrough_pipeline(..),
    RelayPlan::FlushWithReinject { mut effects } => { effects.push(ReinjectKey(*raw_event)); queue.extend(effects); consumed_pending() }
    RelayPlan::FlushPhysicalSuppressedNoReinject { effects } => { queue.extend(effects); consumed_pending() }
    RelayPlan::EngineConsumed { effects } => { /* e12 のまま */ }
}
// テスト: assert!(matches!(plan_relay(Decision::PassThrough, Suppress), RelayPlan::PassThroughPhysicalSuppressed)) を 6 行
```

**(iii) 撤去後**は、F3 前の形(`git show ce8de690^:crates/awase-windows/src/runtime/executor.rs` の 373-455 行)と同じ: `match decision` の各腕で `physical == PhysicalKeyDisposition::Suppress`(または `Allow`)を 1 回見る。Consume の腕は今の `plan_consume_effect` のまま。

| 観点 | (i) | (ii) | (iii) |
|---|---|---|---|
| 読みやすさ | 殻は 1 回の match | 2 重の場合分けが残る | 殻は 1 回の match、判断は腕の中の 1 行 |
| 部品 / 撤去 | 追加 0(ログ用に名前の `fn` が要る) / 型 4 つ | 追加 0 / 型 2 つ | 追加 0 / 型 5 つと全数表 |
| テスト | `matches!` の 6 行 | `assert_eq!` の 3 列の表 | 表は消える。挙動は殻の 2 か所の比較だけ(Linux ではテストされない) |
| コンパイル時間・デバッグ | 変化なし | 変化なし | 変化なし |
| `#[cfg(windows)]`・Linux | `relay_plan.rs` は `CORE_MODULES` のまま。`Decision`・`EffectVec` はルート crate の型 | 同じ | 1 bit の判断が Linux のテストから外れる |

## 付録 B: 入力側の「bool + そのときだけ意味を持つ値」(案 A・案 2 の残り)

**現在**(`state/focus_probe_plan.rs:48`、呼び出しは `key_pipeline.rs:2730`):

```rust
pub(crate) fn plan_focus_probe(status, is_japanese_ime: bool, grace_any: bool,
                               grace_primary_reason: &'static str, shadow_on: bool) -> FocusProbeEffect
// テスト: plan_focus_probe(read(true), true, false, "gji-io", false)   // どの bool が何か覚えておく必要がある
```

**適用後**:

```rust
pub(crate) fn plan_focus_probe(status, is_japanese_ime: bool,
                               grace: Option<&'static str>, shadow_on: bool) -> FocusProbeEffect
//   match grace { Some(reason) if !effective.get() => Suppressed { reason }, _ => Record { .. } }
// 殻: grace = signals.any().then(FocusProbeGraceFlags::primary_reason)
```

同じ問題は全数表の行にもある: `(DrainItem::Held, false, Some(7), Park { remaining: 7 }, HeldStillGuarded)`(`relay_plan.rs` のテスト)の `false` には名前が無い。入力を `Facts` の struct にそろえると、表の行も `RelayFacts { kind, physical }` のように名前付きになる。小さいので、次にそのファイルを触るときでよい。

## 付録 C: 勧めない案の擬似コード(なぜ読みやすくならないか)

**案 1(`Decision<T, Reason>`)を `decide_drift_plan` に当てた場合**:

```rust
// 現在(drift_plan.rs:186-): 上から下へ、途中で決まったら return
if !f.engine_enabled || !f.japanese_ime { return DriftPlan::Idle(DriftIdle::NotActive); }
let Some(drift) = f.drift else { return DriftPlan::Idle(DriftIdle::NoDrift); };
if f.settling { return DriftPlan::DeferToSettle { drift }; }
...

// 適用後: 同じことを自前の型と combinator で
Decided::start(f)
    .guard(|f| f.engine_enabled && f.japanese_ime, DriftPlan::Idle(DriftIdle::NotActive))
    .bind(|f| f.drift.ok_or(DriftPlan::Idle(DriftIdle::NoDrift)))
    .guard(|_| !f.settling, |drift| DriftPlan::DeferToSettle { drift })
    ...
```

`return`・`let … else` は Rust の文法で、自前の `Decided` 型(約 60〜100 行と、そのテスト。概算)が要らない。後者はクロージャの中で `f` と `drift` のどちらが見えているかを追う必要があり、スタックトレースに `{{closure}}` が並び、型エラーのメッセージも長くなる。`std::ops::ControlFlow<DriftPlan, _>` と `?` で書くこともできるが、`return` と比べて読む量は減らない。

**案 3(`reduce` が理由を返す)を `ImeModel::reduce` に当てた場合**: `fn reduce(&mut self, e: &ImeEventEnvelope) -> Vec<ReduceNote>` にしても、呼び出し元の `ImeStateHub::dispatch_event` に読む処理が無い。journal は Event を既に記録しているので、返した値はどこにも使われない。
