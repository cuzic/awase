---
id: ADR-204
title: |-
  GjiFsm 同期義務を「書き込み outcome の副産物」から切り離し、outcome ごとの処遇を1箇所の網羅 match に集約して、同期の黙殺を型とジャーナルで検出可能にする
summary: |-
  BUG-170(ADR-203)の根は `c8bc1adc` が閉じた enum `ImeOpenOutcome` に `Unwarranted` を足した際、「送っていない outcome」の処遇が
  `platform.rs::on_ime_applied_inner`・`state/ime_model.rs`(reduce)・`state/platform_state.rs::record_ime_apply_result`・`runtime/executor.rs` の各所で
  重複した非網羅 `matches!(UnsafeToToggle | NotOwned | Unwarranted)` + `unreachable!()` に書かれていたこと、および `ImeApplyAcceptance::drives_composition_side_effects() -> bool` が
  「なぜ同期しないか」を潰して bool にしていたこと。新 variant を足しても「GjiFsm 同期を落とす」という決定が誰にも問われなかった。
  本 ADR は (D1) outcome→処遇(`OutcomeDisposition`)を1つの網羅 const fn に集約、(D2) 同期の入口を証拠(`OpenEvidence`)の型で束ねる、(D3) 入口×状態の決定表を機械可読な単一データにして
  文書とテストを生成する、(D4) 「同期を見送った」ことを理由つきでジャーナルに残す、を定める。ADR-203 の挙動は変えない(その一般化・再発防止層)。
status: |-
  起票(2026-09-29)。ドラフト。opus-adversarial-consult 未実施。実装未着手。ADR-203 の実装・実機検証の後に着手する。
related_adr:
  - "ADR-203"
  - "ADR-089"
  - "ADR-090"
  - "ADR-161"
---

# ADR-204: GjiFsm 同期義務を黙って捨てられなくする

## 背景

[ADR-203](203-gji-fsm-follows-belief-open-transitions.md) は BUG-170 の**入口ごとの手当て**(level 突合・Reopen)である。同じ種類の漏れ(BUG-18/22/170)が3回起きており、
入口を足す点パッチだけでは4回目を防げない。ADR-203 が入れた `architecture_guard` の入口件数固定は「入口が黙って増減しない」ことを守るが、
**「ある入力に対して同期しない」という決定が黙ってなされる**ことは守らない。BUG-170 で実際に起きたのは後者である。

### BUG-170 で「同期を落とす」決定がなされていた箇所(worktree fix/bugreport-01M3NBQA の実測)

`ImeOpenOutcome::Unwarranted` は `src/platform.rs` の閉じた enum に足された variant(`c8bc1adc`)で、その扱いは次の各所に**個別に**書かれている:

| 箇所 | 処遇 | 形 |
|---|---|---|
| `state/platform_state.rs::record_ime_apply_result`(generation=None) | `NotSent` を返し applied/belief を書かない | 非網羅 `matches!` + 下で `unreachable!()` |
| `runtime/mod.rs::on_ime_apply_complete` | `acceptance.drives_composition_side_effects()==false` → `on_ime_applied` を呼ばない | `bool` 述語(理由が消える、`#[allow(dead_code)]` 付き) |
| `platform.rs::on_ime_applied_inner`(到達しない経路) | `settle` して return(`legacy_gji_sync_obligation` は Unwarranted に `Some` を返すのに「同期義務は無い」とコメント) | 非網羅 `matches!` + `unreachable!()` |
| `state/ime_model.rs`(reduce)、`runtime/executor.rs` | 同上の3 variant 列挙 | 非網羅 `matches!` + `unreachable!()` |

同じ3 variant(`UnsafeToToggle | NotOwned | Unwarranted`)の列挙が4〜5か所に複製され、variant を足すたびに**全部を直す義務**が型では表現されていない
(`unreachable!()` は実行時にしか落ちない)。さらに `bool` は「なぜ同期しないか」を潰すので、`Unwarranted` が「awase は書かなかったが物理キーで実 IME は動いた」ケースを含むことを
後段が区別できない。**outcome は「awase が何を書いたか」であって「実 IME の開閉が変わったか」ではない**(ADR-191 以降、物理キーは awase を経由せず IME を動かす)のに、
同期義務を outcome から導く設計(INV-42、`legacy_gji_sync_obligation`)が前提になっており、両者が混ざっている。

## 決定案(ドラフト、要レビュー)

### D1: outcome の処遇を1つの網羅 const fn に集約する

`ImeOpenOutcome` に対し、次を**唯一の**導出点とする(`state/gji_direct_mechanism.rs` 近傍、ungated):

```rust
pub enum OutcomeDisposition {
    /// 実 IME へ書いた/一致を確認した → 従来どおり belief/applied/composition を更新し GjiFsm を同期する。
    Wrote { effective_open_from: OpenFrom },
    /// 書いていない。belief/applied は書かない。ただし GjiFsm 同期の要否は `sync` が別に決める。
    NotWritten { reason: NotWrittenReason, sync: GjiSyncStance },
}
pub enum NotWrittenReason { UnsafeToToggle, NotOwned, Unwarranted }
pub enum GjiSyncStance { Sync, Skip }   // Skip は理由つきでジャーナルに残す(D4)
pub const fn disposition(outcome: ImeOpenOutcome) -> OutcomeDisposition { /* 網羅 match、`_` 禁止 */ }
```

- 4〜5か所の重複 `matches!` と `unreachable!()` をこの関数の呼び出しに置き換える。新 variant を足すと**ここでだけ**コンパイルエラーになり、処遇(特に GjiFsm 同期の要否)の決定を強制できる。
- `ImeApplyAcceptance::drives_composition_side_effects() -> bool` は廃止し、`disposition` の結果を運ぶ(理由を潰さない)。
- 別案(レシートを outcome 生成箇所で作る): INV-43 の `ActuationReceipt` は `on_ime_applied_inner` の中で作られるため、その手前の early return(`drives_composition_side_effects==false`)を**通り抜けて**義務ごと消えた。
  レシートを `ImeOpenOutcome` の全生成箇所(`ime_controller.rs`・`open_chain.rs`・`executor.rs` の各 return)で作ると義務は消えにくいが、生成箇所が多く変更が広い。
  D1 の集約 fn のほうが変更が小さく、`unreachable!()` も消せる。**要レビュー**(どちらか、または併用)。

### D2: 同期の入口を「証拠」の型で束ねる

同期の入口は、outcome 由来(receipt)・予測・shadow toggle・`sync_direction`・level 突合・IME 種別同期・フォーカス復帰 presync など、ADR-203 決定9 が列挙した通り分散している。
入口ごとに `GjiFsmSync` を直接作るのをやめ、次の証拠型を1つの関数に渡す:

```rust
pub enum OpenEvidence { ActuationOutcome{..}, PredictedOn, ShadowToggleOn, SyncDirectionOn, LevelMismatch, KindRedetected, FocusRestored{..} }
pub const fn gji_obligation(evidence: OpenEvidence, gji: GjiStateKind, ctx: ..) -> GjiSyncDecision   // Sync(GjiFsmSync) | Skip(SkipReason)
```

`gji_on_ime_on`/`gji_on_ime_off` の直接呼び出しは private 化し、入口は `gji_obligation` の結果を `sync_gji` に渡すだけにする。`architecture_guard` の入口件数表は
「証拠の種類 = 入口」の対応表(D3)に置き換える。ADR-203 の (i)(ii) はこの表の2行になる。

### D3: 入口 × GjiFsm 状態の決定表を単一の機械可読データにし、文書とテストを生成する

`(OpenEvidence, GjiStateKind) → GjiSyncDecision` の表を1か所のデータ(または全数列挙可能な純粋関数)にし、次を機械生成する:
(a) ADR-203/204 に載せる決定表(ドリフトを CI で検出、ADR-161「散文の権威を剥奪し単一仕様から生成」と同じ方針)、(b) 全数テスト(`証拠の全 variant × 状態の全 variant` を列挙し、
「どの組も明示的に Sync か Skip(理由)」であること、`_ =>` が無いこと)。表に無い組がコンパイル/テストで検出されるので、「表にない入口は黙って何もしない」が構造的に起きなくなる。

### D4: 「同期しない」決定を理由つきでジャーナルに残す

`GjiSyncDecision::Skip(reason)` を実行したとき、`JournalEntry::GjiSyncSkipped { evidence, gji_state, reason }` を残す。BUG-170 の journal では
`ImeOpenApplied{outcome:Unwarranted}` の後に GjiFsm の遷移が**何も無い**ことは読めたが、それが意図された Skip か漏れかは区別できなかった。理由つきで残せば、
e2e・ジャーナルリプレイで「Skip が想定外の理由・状態で出た」を検出でき(ADR-203 検証の I1/I2 不変条件と併用)、実機の bug report からも入口漏れを一目で特定できる。
件数が多い Skip(例: 既に一致)は ring への集約(件数のみ)を検討する。

## 範囲外

- ADR-203 の挙動(level 突合・Reopen・OFF を同期しない)は変えない。本 ADR はその配線の表現方法を変える。
- `ImeOpenOutcome` の variant の意味や、warrant(ADR-090)の強制方針は変えない。
- StaleConfirm→ESC の問題(BUG-171)。

## 検討する代替と却下理由(暫定)

- **現状維持(入口件数ガードのみ)**: 入口の増減は検出できるが、outcome の処遇の黙殺(BUG-170 の実際の形)は検出できない。不採用。
- **全 `matches!` を grep ガードで固定**: 同じ variant 列挙が複製されている根は残り、新 variant のたびにガードを更新する運用になる。D1 の網羅 match のほうがコンパイラが強制する。
- **`ImeOpenOutcome` を「書き込み結果」と「実 IME の状態変化の証拠」の2 enum に分割**: 概念的には最も正しいが変更が広い(`message_handlers.rs` の u8 符号化・journal・reduce)。
  D1(処遇の集約)+ D2(証拠型)で概念を分けつつ既存 enum は保つ案を先に採る。要レビュー。

## 検証方針(案)

- `disposition` の全 variant 網羅テストと、「4〜5か所の重複 `matches!(… Unwarranted …)` が0件」を `architecture_guard` で固定(D1)。
- 決定表の全数テスト(D3)と、生成物と文書の差分検出。
- `GjiSyncSkipped` の journal 出力を ジャーナルリプレイ・e2e の不変条件に組み込む(D4)。

## 未決事項

- D1 の `NotWritten.sync` を `Sync` にするのは `Unwarranted` のうち何か(ADR-203 は「送信直前の level 突合と ON 系イベントで補う」方針で、outcome 由来の同期は復活させていない)。
  outcome 由来の同期を `Unwarranted` で復活させるか、`Skip` のまま証拠型側(D2)に任せるかを決める(Opus round2 が指摘した「Unwarranted の SetOpen の target が実 IME と一致する保証が無い」との整合)。
- 証拠型 `OpenEvidence` の粒度と、`FocusRestored`/`KindRedetected`(GjiFsm 作り直し)を入口として持つか。
- ADR-065(state 層は `GjiFsm` に依存できない)との整合: `gji_obligation` は ungated な `GjiStateKind`(`OffCold`/`OnCold`/`OnWarm`/`OnComposing` の写し)を受け取り、実 `GjiFsm` は触らない。
- ADR-161 の生成基盤(存在する範囲)を流用できるか。
