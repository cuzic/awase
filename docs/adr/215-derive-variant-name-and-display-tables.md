---
id: ADR-215
title: |-
  variant 名や Display の手書き対応表を strum/thiserror の derive に置き換える(ADR-139 決定4の一部を上書き)
summary: |-
  `journal.rs` には、enum の variant 名を文字列にする手書きの `match` が14関数あり、variant の追加のたびに更新が要った(更新漏れは、`_ =>` を書かないという約束だけが安全装置だった)。
  これを `strum::IntoStaticStr` の derive と `variant_name()` 1本に置き換えた(コミット `b3f922f9`、挙動は変えない)。あわせて、エラー型 `WireLenOverflow` の `Display` を `thiserror`、`KeyboardModel` の `Display` を `strum::Display` にした。
  この変更は ADR-139 決定4(判別子文字列は journal.rs 内に閉じた private fn で持ち、core crate の型に手を入れない)を上書きする。ADR-019(OS 非依存)が禁じるのは `windows-rs`・`cfg(target_os)`・VK 数値で、`strum` は該当しないため、core crate への依存追加は許容する。
  当初この ADR は、`Option<bool>` を3値の型にする決定 A と、bool 引数を enum にする決定 B も含んでいたが、Opus レビュー(round1)で、型を足す前に消費者を撤去するほうが先と指摘され取り下げた(ADR-216)。
status: |-
  提案(2026-10-02)。Opus 敵対的レビュー(round1)で決定 C は採用可と判定された(条件: ADR-139 決定4の上書きの記録、doc コメントの修正。本ファイルと ADR-139 の追記で対応)。実装済み(`refactor/strum-thiserror-derive`、未マージ)。
related_adr:
  - "ADR-019"
  - "ADR-139"
  - "ADR-216"
---

# ADR-215: variant 名・Display の手書き対応表を derive に置き換える

## 背景

`crates/awase-windows/src/journal.rs` は、tracing の判別子文字列(ADR-139 決定4)のために、enum の variant 名を返す private fn を手書きしていた
(`decision_kind_str`、`ime_event_kind_str`、`ime_open_outcome_str` など14関数、約100行)。`ImeEvent` のような大きな enum に variant を足すたびに、
この対応表の更新が要り、`_ =>` を書かないというコメント上の約束(と `architecture_guard` の文字列検査)だけが漏れを防いでいた。

## 決定

- **variant 名は `strum::IntoStaticStr` を derive し、`journal.rs` の `variant_name()` 1本で取る。** 対象は15 enum。ほとんどは `awase-windows` 内の型だが、
  `ImeOpenOutcome`・`InputModeState`・`KeyClassification` は core crate `awase` の型で、そこにも derive を足した。
  文字列は variant 名そのままで、journal の JSON(serde、variant 名そのまま)との表記の一致は維持される(ADR-139 決定2)。
- **エラー型は `thiserror`、エラーではない型の `Display` は `strum::Display` を使う。** `WireLenOverflow` は `thiserror`、`KeyboardModel` の `Display` は `strum::Display`(lowercase)。
  `ClassifyReason`/`RejectReason` のような「理由」の型は、`Error` を実装すると意味が紛れ、`ClassifyReason` は Debug と Display で引用符の有無も違うため、手書きのままにする。
- **新規の依存 `strum` を、root crate `awase` と `awase-windows` に足す。**
- **`architecture_guard` の変更**: `journal.rs` 内の variant 名の出現数(2)の許容を3テストから撤去し、`emit_tracing` のワイルドカード検査の開始マーカーを `fn decision_kind_shape(` に移した。
  許容件数(2)が、journal.rs の該当件数 0 の下で `_ => 0` の既定に落ちるため、検査はむしろ厳しくなる。走査範囲は `emit_tracing` を引き続き含む。

### ADR-139 決定4の上書き

ADR-139 決定4(`docs/adr/139-tracing-metrics-observability-migration.md`)は「判別子文字列は enum の型自体に `as_str()` を生やすのではなく、`journal.rs` 内に閉じた private fn として実装する
(core crate の型に手を入れず、ADR-019 の依存追加議論を避けるため)」と決めていた。本 ADR はこの文を上書きする。理由と範囲は次のとおり。

- ADR-019 の制約は、core crate が OS 依存にならないこと(`windows-rs`・`cfg(target_os)`・VK 数値の禁止)。`strum` は OS 非依存の derive マクロで、この制約に触れない。
  ADR-139 が避けたかった「依存追加の議論」は、本 ADR で行った。
- 決定4が守りたかったもう1つの点(判別子を journal の JSON と同じ表記にする、`_ =>` を書かず variant の追加をコンパイルで検知する)は、derive が variant の追加に自動で追従するため、
  むしろ強くなる。
- 変わらないもの: `emit_tracing` の `?`/`%` 禁止(決定4の第2項)、`seq`/`elapsed_ms` の明示(第1項)。

## 決定しないこと

- `Option<bool>` を型にする案、bool 引数を enum にする案。Opus round1 は、前者を、新しい型の前に消費者の撤去が先(`WarmupImeOn` が ADR-212 で消費者ごと撤去された前例)、
  かつ ADR-214 との意味の食い違い・`From`/`Into` の穴があるとして不採用とし、後者は ADR 自身の基準を満たす候補が無い(`kanji_role_plan`・`is_press_start` は本番の呼び出し元が1箇所、`assign` はテスト専用)と判定した。
  前者の「罠の読み手を減らす」目的は、ADR-216(診断専用コードの撤去)で、型を足さずに進める。
- `KeyboardModel` の `FromStr` の置き換え(`strum::EnumString` + `ascii_case_insensitive`)。エイリアス(`jp`、`jis109`、`ansi`、`us104`)の扱いと `Err` の型(現状は `String`)が変わるため、今回は手書きのまま残す。
- `BugReportImeKind` の `as_str`/`FromStr`。`as_str` が `const fn` で、`Err = ()` が呼び出し元(`awase-settings`)の型に影響するため。

## 検証

- `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib` は警告なしで通る。`cargo test --lib`(1067 件)、`architecture_guard`(115 件)、`layer_boundary_guard`、`golden_scenarios` が通る。
- `journal.rs` は `cfg(windows)` のため、tracing に出る文字列の実行確認は Linux ではできず、`windows-build` CI に任せる。ただし derive 対象の多く(`AppImeProfile`、`ImeKindId`、`DecisionSite`、`WriteMechanism`、`ImeOpenOutcome` 等)は
  `cfg(windows)` ではない。「strum の文字列 == serde の variant 名」を全 variant で比べるテストを Linux で書けるが、必須とはしない(未実施。後続でやるなら別コミットにする)。

## 未確定・リスク

- `variant_name<T: Into<&'static str>>` は `&'static str` 自体も受け取る(`variant_name("x")` がコンパイルできる)。害はないが、「判別子は enum から取る」を型では強制しない。
  bound を `strum` 固有の trait に絞る案はあるが、優先度は低いので見送る。
- `strum` が core crate の依存に入ったことで、今後 core の enum に derive が増える可能性がある。その場合も、OS 非依存である限り許容する(ADR-019)。

## 参考

- コミット `b3f922f9`(実装)、`crates/awase-windows/src/journal.rs`(`variant_name`)、`crates/awase-windows/tests/architecture_guard.rs`
- `docs/adr/139-tracing-metrics-observability-migration.md` 決定2・決定4
- ADR-216(`Option<bool>` の罠の読み手を、型を足さずに減らす)
