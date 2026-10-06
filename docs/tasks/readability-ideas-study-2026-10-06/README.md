---
title: FCIS・代数的 Effect・monad 風の書き方で可読性を上げる案の検討(検討のみ)
created: 2026-10-06
base: origin/develop 3c8ef62a
status: 検討のみ。コードは変えていない。所有者の判断待ち
related_adr: ["ADR-229", "ADR-218", "ADR-219", "ADR-220", "ADR-090", "ADR-156"]
---

# FCIS・代数的 Effect・monad 風の書き方で可読性を上げる案の検討

所有者の依頼(2026-10-06): 「ここまでの経験、知見を元にして、もっと FCIS と代数的 Effect、Haskell Monad 的な DSL による可読性向上を考慮して、改善できるアイデアを考えて」。実測は [evidence.md](evidence.md)。

## 1. 結論

- **monad 風の型・combinator・マクロの DSL は、どれも勧めない**。理由は 3 つ。
  1. 「途中で決まったら抜ける」(Haskell の `Maybe`/`Either` の do 記法で書くことの大半)は、Rust の `return`・`let … else`・`?` が既に持っている。実装済みの `decide_*` の早期 return は 2〜4 個で、上から下へ読める(evidence (b)-3)。自前の型に `?` を使う `Try` trait は stable Rust では実装できない。
  2. 読みにくさが原因の実害(BUG-020・BUG-027・BUG-129 など)は、長い関数・古いコメント・実態と合わない名前によるもので、**合成の書き方が無いことが原因の記録は 0 件**(evidence (e))。
  3. 却下済みの案(Plan/Effect-term、汎用の Handler/Facts ツールキット、宣言テーブル)と同じく、使い手が 1〜2 個の部品を足すことになる。
- **Haskell から借りて効くのは monad ではなく「代数的データ型(enum)で、起こり得ない組を書けなくする」ほう**。実装済みの判断関数 7 ファイルのうち 5 つは既に「理由 = enum の variant」の形だが、`relay_plan.rs` と `ime_read_strategy.rs` は「実行の種類」と「理由」を別々の enum にして struct に並べている(20 通りを型が許し、実際に返すのは 5 通り)。これを 1 つの enum にそろえると、**型と行が減り、殻の 2 重の場合分けが消える**(案 A)。
- 規模は小さい。最初の 1 歩は `relay_plan` の 1 か所(見積もりで 50〜60 行の純減、新しい部品 0)。それ以上は「次にそのファイルを触るときに」でよい。

## 2. 案の一覧

| # | 案(Rust で書ける形) | 判定 | 一言の理由 |
|---|---|---|---|
| A | **判断の結果を 1 つの enum にし、理由を variant の名前にする。「bool と、真のときだけ意味を持つ値」は `Option`/variant のデータにする** | **推奨** | 型と行が減る。ルート crate の `Decision` の doc と同じ流儀。新しい部品なし |
| 5 | 殻は判断の結果を 1 回だけ match し、腕の中で元の入力を見直さない(書き方の約束) | **推奨(A とセット)** | `execute_relay` の 2 重の場合分けが消える。タスク表「実行は殻の 1 関数」の具体化 |
| 2 | 事実を 1 つの引数(`Facts`)にまとめる(Reader 風) | 既に大半できている | 7 ファイル中 5 つ。残りの位置引数の bool は A で直す |
| 4 | 型状態で順序を型にする(typestate) | 既にある・計画済み | `Actuation<Requested→Warranted→Verified>`(ADR-090)。次は V5(`OutputActiveGuard`)。新設しない |
| 1 | 理由を蓄積する判定の型 `Decision<T, Reason>`(Writer 風、`and_then` で連ねる) | 勧めない | 1 つの判断が持つ理由は 1 つ。蓄積の使い手は `typing_guard_bypassed` の 1 件だけ |
| 3 | `reduce(state, event) -> (state, Vec<Reason>)`(State/Writer 風) | 勧めない | 返した理由を読む使い手が 0。journal は Event を既に記録している |
| 6 | 小さな combinator(`then`・`or_else`・`guard`) | 勧めない | 標準に `bool::then`・`Option::filter`・`is_some_and`・`let … else` がある(使用 45〜104 件) |
| 7 | `given(facts).when(event).then(plan)` のテスト builder | 勧めない | 既存のタプルの全数表(plan 7 ファイルで grep 16 か所)で読める。ADR-219 がシナリオ DSL を見送った理由と同じ |
| 8 | マクロ DSL | 勧めない | ADR-218〜220 の判定を覆す実害の記録が無い |

## 3. 推奨の中身(案 A + 5)

**直す対象**(evidence (b)-1・(c)):

- `relay_plan.rs` の `RelayPlan { action: RelayAction, reason: RelayReason }` と、入力の `RelayDecisionKind`/`RelayFacts`。殻の `execute_relay` は `Decision` → `RelayDecisionKind` に写し、判断し、もう一度 `Decision` で match し、腕の中で `plan.action` を見直している。
- 次点: `relay_plan.rs` の `DrainStep { action, reason }`、`ime_read_strategy.rs` の `ReadDecision { strategy, reason, .. }`、`focus_probe_plan.rs` の `grace_any: bool` + `grace_primary_reason: &str`。

**第 1 段階(1 PR、CI で確かめられる形)**: `plan_relay` が `Decision` を受け取り(所有権ごと。中身の `effects` を運ぶだけで OS には触れないので純粋なまま)、理由を名前にした 1 つの enum を返す。殻はその enum を 1 回だけ match する(付録 A の擬似コード)。

- **撤去するもの**: `RelayDecisionKind`・`RelayFacts`・`RelayAction`・`RelayReason`・`RelayPlan` struct、殻の写しの match(5 行)と 2 か所の `matches!(plan.action, …)`、それを説明するコメント(`executor.rs:413-414`)。
- **見積もり**(コンパイルしていない。現在の行番号からの概算): `relay_plan.rs` の該当部 約 76 行 → 約 30 行、`executor.rs` 約 −10 行、テストの表は 4 列 → 3 列。合計 50〜60 行の純減、新しい型・関数 0。
- **成否の判定**: CI の `relay_plan` の全数表(書き直し、6 行のまま)と e12 のテスト(変えない)が通る。`architecture_guard` の e12・e13 の固定が変わらない。`relay_plan.rs` は `.cargo/mutants-awase-windows.toml` の `examine_globs` に入っている(45 行目)ので、mutants の missed が増えない。
- **取りやめ条件**: 純減にならない。殻に `unreachable!` が要る形になる。`plan_relay` を `const fn` でなくすことが他の固定(ガード)に響く。
- **注意**: `execute_relay`/`drain_deferred` は defer/replay キューの再発ファミリー(ADR-156)。`DrainStep` は同じ PR に入れない(殻の行数がほぼ変わらず、得が小さい。見積もり −13 行)。

**書き方として残すなら**: タスク表 V2(完了前チェック)に 1 行、「判断関数の戻り値に、理由から一意に決まる別の enum を並べない(理由を variant にし、要るなら `fn` で導く)」。

## 4. 勧めない理由(却下済みの案との関係)

- **案 1(`Decision<T, Reason>`)**: 理由を「蓄積する」使い手が無い。実装済みの判断は、どれも最後に 1 つの理由を返す。早期 return を `and_then` の連鎖やクロージャにすると、`decide_drift_plan` の「稼働条件 → ずれ → settle → …」の順序が読みにくくなり、スタックトレースに `{{closure}}` が並び、新規参加者が覚える型が 1 つ増える。
- **案 3(reduce が理由を返す)**: W0 の実測で、`ImeStateHub` のメソッドの半分(53 中 27)は Event を出さない。`reduce` の形をそろえるより、F の分割で判断の入口を減らすほうが先(ADR-229「根本的に変えるべきは判断の入口」)。消費者の無い戻り値は足さない。
- **案 6〜8**: ADR-218〜220 と同じ判定(実害の記録が無く、純減も小さい)。自前の combinator・マクロは、エラーメッセージが読みにくくなり、Linux と Windows の両方で同じ定義を保つ手間が増える。
- **却下済みの案との違い**: 案 A・5 は**新しい部品を作らず、型を消す**。Plan/Effect-term(効果の列をデータにして解釈する)・汎用の Handler/Facts/Tagged/Clock・宣言テーブル・書き込みの吸収/正規化は、どれも再提案していない。

## 5. 所有者に聞くこと

1. 第 1 段階(`relay_plan` の 1 つの enum 化、50〜60 行の純減見込み)を 1 PR で試してよいか。
2. 「理由から一意に決まる別の enum を並べない」を V2 の完了前チェックに足すか。次点(`DrainStep`・`ReadDecision`・`plan_focus_probe` の引数)は「次にそのファイルを触るとき」でよいか。`ReadDecision` は再生 fixture 11 件の JSON の書き換えを伴う。
3. 範囲外だが測って分かったこと: 長い関数の 3〜5 割がコメントで、1 関数に `BUG-`/`ADR-` の参照が 13〜39 件ある。`ir_apply_drift_correction` の先頭には、もう無い早期 return の経緯(BUG-20)が残っている。経緯を ADR・known-bugs に寄せ、コードには「今なぜこうか」だけを残す整理を、別に検討するか。
4. monad 風の DSL(案 1・3・6〜8)は「やらない」でよいか。

## 6. 確認できなかったこと

- 行数の見積もりは書いてコンパイルしたものではない。`plan_relay` の `const fn` を外すことが、ガードや clippy に響くかは未確認。
- `std::ops::ControlFlow` に `?` を使える点は Rust の仕様の知識で、このリポジトリでは試していない。
- 長い関数の数え方は粗い(evidence (a))。

---

## 付録 A: 案 A + 5 の比較(`plan_relay` と `execute_relay`)

**現在**(`state/relay_plan.rs:69-98` と `runtime/executor.rs:399-492` の要約):

```rust
// core: 2 つの enum を並べて返す(20 通りを型が許し、返すのは 5 通り)
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

**適用後**(擬似コード):

```rust
// core: 理由が variant。運ぶ効果の列も variant が持つので、起こり得ない組は書けない
pub(crate) enum RelayPlan {
    /// 物理キー抑止(KANJI)。passthrough も reinject も走らせない。
    PassThroughPhysicalSuppressed,
    /// OS に直接通す。
    PassThroughIdle,
    /// flush 出力 + キーの再注入をキューへ。
    FlushWithReinject { effects: EffectVec },
    /// flush 出力だけをキューへ(物理キー抑止なので再注入しない)。
    FlushPhysicalSuppressedNoReinject { effects: EffectVec },
    /// Engine が消費。Timer だけ即時(e12)。
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

// shell: 判断の結果を 1 回だけ match する(ログは `{plan:?}` ではなく variant 名で足りる)
match plan_relay(decision, physical) {
    RelayPlan::PassThroughPhysicalSuppressed => consumed(),
    RelayPlan::PassThroughIdle => run_passthrough_pipeline(..),
    RelayPlan::FlushWithReinject { mut effects } => { effects.push(ReinjectKey(*raw_event)); queue.extend(effects); consumed_pending() }
    RelayPlan::FlushPhysicalSuppressedNoReinject { effects } => { queue.extend(effects); consumed_pending() }
    RelayPlan::EngineConsumed { effects } => { /* e12 のまま */ }
}
```

| 観点 | 影響 |
|---|---|
| 読みやすさ | 判断が 5 行の match になり、殻は 1 回の match。`plan_relay_exhaustive` が固定していた「種別と action の対応」は型が持つ |
| 部品 / 撤去 | 追加 0 / 型 5 つと殻の写し・見直しを撤去(見積もり 50〜60 行の純減) |
| 実害の記録 | 直接の記録は無い(「action と reason の食い違い」は 0 件)。動機は流儀の統一と撤去 |
| コンパイル時間・デバッグ | 変化なし。総称型もクロージャも足さない。`Debug` の出力は variant 名(理由)になる |
| 学習コスト | 下がる(覚える型が減る)。ルート crate の `Decision` と同じ形 |
| `#[cfg(windows)]`・Linux テスト | `relay_plan.rs` は `CORE_MODULES`(Linux でテスト可)のまま。`Decision`・`EffectVec` はルート crate の型で Linux で使える。テストは `EffectVec::new()` を渡す |
| 却下済みとの違い | 効果の列を解釈する仕組みではない。既存の 1 回の判断の戻り値の形を変えるだけ |

## 付録 B: 案 A の次点(入力側の「bool + そのときだけ意味を持つ値」)

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

撤去: 引数 1 つと doc の「`grace_any` が偽のときは参照されない」の 2 行。小さいので、次にこのファイルを触るときでよい。`ReadDecision`(`strategy` を `ReadReason` から導く)も同じ扱い。再生 fixture 11 件の `"strategy"` の行を消す必要がある。

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

`return`・`let … else` は Rust の文法で、自前の `Decided` 型(約 60〜100 行と、そのテスト)が要らない。後者はクロージャの中で `f` と `drift` のどちらが見えているかを追う必要があり、型エラーのメッセージも長くなる。`std::ops::ControlFlow<DriftPlan, _>` と `?` で書くこともできるが、`return` と比べて読む量は減らない。

**案 3(`reduce` が理由を返す)を `ImeModel::reduce` に当てた場合**: `fn reduce(&mut self, e: &ImeEventEnvelope) -> Vec<ReduceNote>` にしても、呼び出し元の `ImeStateHub::dispatch_event` に読む処理が無い。journal は Event を既に記録しているので、返した値はどこにも使われない。
