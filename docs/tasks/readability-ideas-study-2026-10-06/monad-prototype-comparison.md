---
title: monad 風の判定型を decide_read_strategy で試作して読み比べた結果
created: 2026-10-06
base: origin/develop 47b1093b
branch: experiment/monad-style-decision(develop にマージしない・PR にしない)
code: crates/awase-windows/src/state/ime_read_strategy.rs の monad_style モジュール
related: README.md(可読性メモ)・evidence.md・ADR-218〜220・ADR-229
---

# monad 風の判定型を `decide_read_strategy` で試作して読み比べた結果

所有者の判断(2026-10-06): 可読性メモ(README.md)は「monad 風の DSL・combinator・マクロ DSL はやらない」と結論したが、
**小さく試作して、実際のコードで読み比べる**。この文書はその結果。メモの結論を覆す前提ではない。

## 1. 結論

- **monad 風の型(版 A・B・C)は採用しない**。読みやすさの改善のほとんどは、monad 風の形ではなく
  **小さな関数の切り出し**(`explicit_verify`・`skip`・`poll_target`)から来ていた。同じ部品を使い、元と同じ早期 return で書いた
  対照の版 D が、行数・読み順・`const fn` のどれでも A〜C より良い。
- 採用する価値があるとすれば **版 D の形(部品を切り出し、早期 return のまま)** だけ。これは monad 風の書き方ではなく、
  メモの「`return`・`let … else`・`?` で足りる」をそのまま裏付けた。D を本番に入れるかは別の話で、元の関数も 42 行で
  読めるので、このファイルを次に触るときの任意の整理にとどめる(所有者に確認)。
- 条件付きで考えられるのは **版 A(標準の `ControlFlow` と `?`)** だけ。段(ガード)が 5 個以上あり、各段が「通ったら次へ値を
  渡す」形のときは、最上位の 3 行が段の一覧として読める利点が出る。今のリポジトリの `decide_*` は早期 return が 2〜4 個
  (evidence (b)-3)で、その条件を満たす関数は見当たらない(全数は数えていない、未確認)。

## 2. 対象に選んだ関数と理由

`crates/awase-windows/src/state/ime_read_strategy.rs::decide_read_strategy`(FCIS F1、`runtime/ime_refresh.rs::ir_decide_read_strategy` の核)。

| 候補 | 行数 | 早期 return | ネスト | 選ばなかった理由 |
|---|---:|---:|---:|---|
| **`decide_read_strategy`** | 42 | 2 + 末尾の if/else | 2 段(`if typing { if !explicit_verify { return } }`) | — **選んだ**。理由の enum(`ReadReason`)を返し、**途中で決まる値(`typing_guard_bypassed`)を `let mut` で運んで 4 つの戻り値すべてに詰める**。Writer 風の「途中の情報を運ぶ」がそのまま当てはまる唯一の候補。全数テスト(128 通り)と journal の再生 fixture(`tests/journals/read_strategy/`)がある |
| `ime_set_open_plan::plan_set_open` | 9 | 0(if/else if/else) | 1 | 3 分岐だけで、monad 風に書く余地が無い |
| `deferred_gate_plan::plan_drain_before_send` | 15 | 2 | 1 | 運ぶ値が無い(Option を見るだけ) |
| `msaa_role_plan::decide_msaa_role` | 表引き | 1(`let … else`) | 1 | 表引きで、段の列ではない |
| `ime_actuation.rs` | — | — | — | 時刻つきの判断が混ざり、全数テストの形でない |

`state/drift_plan.rs`(E1 実施中)と `state/relay_plan.rs`(別 PR で変更中)は依頼どおり避けた。

## 3. 書いたもの(すべて同じファイルの `monad_style` モジュール、元の関数は変えていない)

| 版 | 形 | 新しい型・関数 |
|---|---|---|
| 元 | `let mut typing_guard_bypassed` + 早期 return 2 + 末尾 if/else。`ReadDecision { .. }` の構造体リテラルを 4 回書く | — |
| 共通の部品 | `explicit_verify(facts)`・`skip(reason, bypassed)`・`poll_target(facts, bypassed)`(3 つとも `const fn`) | 関数 3 |
| D(対照) | 部品を使い、元と同じ早期 return で書く。`const fn` | 関数 1 |
| A | 標準の `std::ops::ControlFlow` と `?` | 関数 4(入口・段の列・ガード 2) |
| B | 自前の `Decision<T, D> { Done(D), Next(T) }` と `guard`・`and_then`・`finish` | 型 1 + メソッド 3 + 関数 3 |
| C | B に Writer 風の蓄積(通った段の名前の `Vec<&'static str>`)を足す | 関数 1 |

`tests::monad_style_versions_match_original` が、4 版と元の関数が全入力(128 通り)で同じ `ReadDecision` を返すこと、
C は通った段の列も期待どおりであることを確かめる。新しい crate・トレイト・マクロは作っていない。

### 3-1. 並べたコード

元(`decide_read_strategy`、コメントを除く):

```rust
pub fn decide_read_strategy(facts: &ReadStrategyFacts) -> ReadDecision {
    let typing = is_typing(facts.idle_ms);
    let mut typing_guard_bypassed = false;
    if typing {
        let explicit_verify = !facts.skip_imm_query
            && (facts.mode_key_pass_live || (facts.explicit_intent_present && facts.applied_known));
        if !explicit_verify {
            return ReadDecision { strategy: ImeReadStrategy::SkipTyping, reason: ReadReason::TypingActive, typing_guard_bypassed };
        }
        typing_guard_bypassed = true;
    }
    if facts.shift_conv_guard_active {
        return ReadDecision { strategy: ImeReadStrategy::SkipTyping, reason: ReadReason::ShiftConvGuard, typing_guard_bypassed };
    }
    if facts.skip_imm_query {
        ReadDecision { strategy: ImeReadStrategy::Blacklist, reason: ReadReason::ImmQuerySkipped, typing_guard_bypassed }
    } else {
        ReadDecision { strategy: ImeReadStrategy::OsPoll, reason: ReadReason::OsPoll, typing_guard_bypassed }
    }
}
```

D(対照、部品を使う早期 return):

```rust
pub(crate) const fn decide_plain(facts: &ReadStrategyFacts) -> ReadDecision {
    let typing = is_typing(facts.idle_ms);
    if typing && !explicit_verify(facts) {
        return skip(ReadReason::TypingActive, false);
    }
    // ここまで来た打鍵中は、明示的な IME 操作の検証のためにガードを迂回した。
    let bypassed = typing;
    if facts.shift_conv_guard_active {
        return skip(ReadReason::ShiftConvGuard, bypassed);
    }
    poll_target(facts, bypassed)
}
```

A(`ControlFlow` と `?`):

```rust
pub(crate) fn decide_cf(facts: &ReadStrategyFacts) -> ReadDecision {
    match decide_cf_steps(facts) {
        Break(decision) | Continue(decision) => decision,
    }
}
fn decide_cf_steps(facts: &ReadStrategyFacts) -> ControlFlow<ReadDecision, ReadDecision> {
    let bypassed = typing_guard_cf(facts)?;
    shift_conv_guard_cf(facts, bypassed)?;
    Continue(poll_target(facts, bypassed))
}
fn typing_guard_cf(facts: &ReadStrategyFacts) -> ControlFlow<ReadDecision, bool> {
    match (is_typing(facts.idle_ms), explicit_verify(facts)) {
        (false, _) => Continue(false),
        (true, true) => Continue(true),
        (true, false) => Break(skip(ReadReason::TypingActive, false)),
    }
}
fn shift_conv_guard_cf(facts: &ReadStrategyFacts, bypassed: bool) -> ControlFlow<ReadDecision> {
    if facts.shift_conv_guard_active { Break(skip(ReadReason::ShiftConvGuard, bypassed)) } else { Continue(()) }
}
```

B(自前の判定型と combinator。型とメソッドの定義 35 行は省略):

```rust
pub(crate) fn decide_combinator(facts: &ReadStrategyFacts) -> ReadDecision {
    typing_guard(facts)
        .and_then(|bypassed| shift_conv_guard(facts, bypassed))
        .finish(|bypassed| poll_target(facts, bypassed))
}
fn typing_guard(facts: &ReadStrategyFacts) -> Decision<bool, ReadDecision> {
    if is_typing(facts.idle_ms) {
        Decision::guard(!explicit_verify(facts), skip(ReadReason::TypingActive, false), true)
    } else {
        Decision::Next(false)
    }
}
fn shift_conv_guard(facts: &ReadStrategyFacts, bypassed: bool) -> Decision<bool, ReadDecision> {
    Decision::guard(facts.shift_conv_guard_active, skip(ReadReason::ShiftConvGuard, bypassed), bypassed)
}
```

## 4. 読み比べ

### (a) 行数

空行と `//` コメントを除いたコードの行数(rustfmt 済みのファイルで数えた。構造体リテラルは rustfmt が複数行に展開する)。

| 版 | 判定本体 | 部品(共通 23 行を含む) | 合計 | 元との比 |
|---|---:|---:|---:|---:|
| 元 | 36 | 0 | **36** | 1.0 |
| D(対照) | 12 | 23 | **35** | 1.0 |
| A(`ControlFlow`) | 25 | 23 | **48** | 1.3 |
| B(自前の型) | 24 | 23 + 型 26 | **73** | 2.0 |
| C(B + 蓄積) | 14 | B の 73 を前提 | **87** | 2.4 |

- テスト: 元の全数テスト(34 行)はどの版でもそのまま使える。比較テストは 40 行足した(試作の確認用で、採用するなら不要)。
- A〜C の行数は、部品(共通の 23 行)を切り出した後の差。**部品の切り出しだけ(D)では行数は変わらず、monad 風の形にすると増える**。
- B の型と combinator(26 行)は他の判定でも再利用できる、という反論はあり得る。ただし再利用先が無い(このリポジトリに
  `and_then` を持つ自前の判定型は他に無い)うちは、26 行は 1 関数のための費用。

### (b) 読み順

- **元**: 上から読めるが、`typing_guard_bypassed` が `let mut` で始まり、途中で `true` になり、4 つの戻り値に詰められる。
  「この戻り値の `typing_guard_bypassed` はいくつか」を知るには、その戻り値より上の流れを全部追う必要がある。
  `ReadDecision { .. }` の 5 行のリテラルが 4 回並び、違いは `strategy`・`reason` の 2 語だけ。
- **D**: 元の分かりにくさ 2 つ(`let mut` と 4 回のリテラル)が両方消える。`bypassed = typing` の 1 行で
  「ガードを抜けた打鍵中 = 迂回した」が読める。ネストも無くなる(`typing && !explicit_verify(facts)` の 1 条件)。
  上から下へ読め、関数の外へ飛ぶのは部品の 3 つだけ。
- **A**: 最上位の `decide_cf_steps` の 3 行が「打鍵中ガード → Shift 変換ガード → 読み方」という**段の一覧**として読め、
  これは元より良い。ただし、各段の中身(何で抜けるか)は別の関数にあり、**判定の全体を知るには 4 つの関数を行き来する**。
  `?` が「ここで決まったら抜ける」を意味することは、`Result` の `?`(エラーで抜ける)と向きが逆に見える
  (`Break` は失敗ではなく決定)。入口の `Break(d) | Continue(d) => d` は、`ControlFlow` から値を取り出す標準のメソッド(`into_value`)が stable でない(筆者の知識、このリポジトリでは試していない)ために書いた 3 行。
- **B**: `.and_then(|bypassed| …)` の鎖は短いが、`Decision<bool, ReadDecision>` の `bool` が何を運んでいるかは型から読めない
  (元は `typing_guard_bypassed` という名前があった)。`Decision::guard(cond, done, carry)` は位置引数 3 つで、**「cond が真なら抜ける」**。
  標準の `Option::filter`・`bool::then` は「真なら残す」で、向きが逆。`typing_guard` の中に `if` が残り、分岐は消えずに移っただけ。
  名前の `Decision` は、ルート crate の `src/engine/decision.rs::Decision`(エンジンの結果)と同じ名前で、取り違えの元になる。
- **C**: 通った段の名前(`"typing_guard"` など)を文字列で別に書くので、関数名と文字列がずれても検出されない
  (比較テストが今は固定しているが、テストを足さない段は固定されない)。

### (c) エラーメッセージ・デバッグ

- この関数はパニックしないので、`#[track_caller]` の出番は無い(どの版も同じ)。
- スタックトレース・デバッガのステップ実行: A は関数が 4 段に分かれ、B は `and_then`/`finish` の中のクロージャ
  (`{{closure}}`)を経由する。元と D は 1 フレーム(部品は `const fn` で小さい)。ステップ実行で追う手数は 元 ≒ D < A < B。
- コンパイルエラー: B は総称型 `Decision<T, D>` と `impl FnOnce` の組なので、型を間違えたときのエラーが長くなる
  (試作中に実際に詰まった箇所は無かったが、推測。未確認)。
- clippy(`pedantic`・`nursery` を deny): 結果は §5。

### (d) 新規参加者の学習コスト

- `ControlFlow` はこのリポジトリで `crates/win32-worker/src/lib.rs` の `sleep_ms` に 1 か所あるだけで、`?` と組み合わせた使い方は 0 件。
  A を入れると、読み手は「`ControlFlow` に `?` を使うと `Break` で抜ける」を新しく覚える必要がある。
- B は自前の型なので、読み手は `guard`・`and_then`・`finish` の 3 つの意味(と `guard` の向き)を、このファイルを開いて覚える必要がある。
  BUG-027(委譲の層が 1 メソッドを委譲し忘れ、既定の何もしない実装が黙って使われた)のような「層そのものが作る見落とし」は、
  この試作の規模では起きなかったが、部品が他のファイルへ広がるほど増える種類。
- D は Rust の基本(早期 return・小さな関数)だけで読める。

### (e) `const fn` と Linux テストへの影響

- **D と部品 3 つは `const fn` にできた**。A は `?`、B・C はクロージャ(`impl FnOnce`)を使うので `const fn` にできない。
  元の関数は `const fn` ではないが、同じ形のまま `const fn` にできる(`let mut` と早期 return は const fn で使える)。
  `crates/awase-windows/Cargo.toml` は `missing_const_for_fn = "allow"` なので、どの版も lint は出ない。
- Linux のテスト: `state/` は `#[cfg(windows)]` の外なので、比較テストを含め全版が Linux の `cargo test`/nextest で走る。
  どの版でも違いは無い。
- 実行時: C は呼ぶたびに `Vec` を確保する。`ir_decide_read_strategy` は IME 状態の読み取りのたびに呼ばれるので、
  C を本番に入れると読み取りのたびに確保が 1 回増える。A・B・D は確保なし。

### (f) 理由の持たせ方(ログ・journal)

- 元・D・A・B: 1 回の判断の理由は `ReadReason` の 1 つで、`ir_decide_read_strategy`(`runtime/ime_refresh.rs:410`)がそれで
  ログを分け、journal の再生 fixture(`tests/journals/read_strategy/decide-read-strategy-cases.json`)は `ReadDecision` を
  そのまま比べる。4 版とも `ReadDecision` は同じなので、ログ・journal・fixture は変わらない。
- C の「通った段の列」は `ReadReason` から一意に決まる(`TypingActive` → 1 段、`ShiftConvGuard` → 2 段、それ以外 → 3 段。
  比較テストの `expected_trail` がそのまま導出式)。**新しい情報が無い**ので、journal に載せる理由も無い。
  載せるなら `ReadDecision` の serde 形(fixture の JSON)を変えることになる。これはメモの「1 回の判断の理由は 1 つ。蓄積する型の使い手が無い」を実物で確かめた形。

## 5. CI の結果

push 先: `experiment/monad-style-decision`(コミット 2d430b09)。`ci.yml` は push の対象ブランチに入っていないので、
`workflow_dispatch` で起動した(run 37422836301)。

- pre-push の `cargo xwin check --target x86_64-pc-windows-msvc -p awase-windows`: 通過(ローカル)。
- CI(Linux): `test`(nextest。`state::ime_read_strategy::tests::monad_style_versions_match_original` と元の全数テスト・
  `read_strategy_replay` が PASS)・`clippy`・`fmt`・`dylint`・`machete`・`audit` が success。
- CI(Windows): `windows-build`(`cargo clippy -p awase-windows -- -D warnings`〈`pedantic`・`nursery` は Cargo.toml で deny〉と
  Windows 上の nextest。比較テスト PASS)・`windows-cross-check`・`windows-settings` が success。
  **clippy は 4 版のどれにも警告を出さなかった**(`#[allow(dead_code)]` は付けた。本番から呼ばないため)。
- `workflow_dispatch` は mutants ジョブも起動するが、試作の確認には不要なので、上のジョブが終わった後に run を取り消した
  (mutants・`windows-package` の結果は無い)。

## 6. 確認できなかったこと

- 他の `decide_*`/`plan_*` に段が 5 個以上のものがあるかは全数を数えていない(§1 の A の条件)。
- B のコンパイルエラーが長くなるかは、試作中に間違えなかったので確かめていない(推測)。
- 読みやすさの判断は書いた本人(1 人)のもので、別の読み手で確かめていない。
- 版 D を本番に入れた場合のレビュー・mutants の結果は見ていない(この試作は本番の関数を差し替えていない)。
