---
id: ADR-224
title: |-
  閉ループのハーネスが写している ImeStateHub の配線を、ungate する(案A)か純粋関数へ切り出す(案C)か。見逃しが出るまで着手しない
summary: |-
  閉ループ(擬似 IME、`tests/closed_loop_scenarios.rs`)は `ImeStateHub`・`runtime/` が `#[cfg(windows)]` で host から見えないため、数行の配線を `tests/support/harness.rs` に写している。
  案A(`ImeStateHub`/`journal`/`ime_event_log` を host へ ungate)は `foreground_scope` を host 用 stub にするとスコープ失効など Windows 固有挙動が見えなくなり、`architecture_guard`(約6000行)・`layer_boundary_guard` のテキスト走査が広く壊れる。
  案C(写している配線だけを ungated な純粋関数へ切り出し、本番とハーネスの両方から呼ぶ)は影響範囲が小さい。写しが原因の見逃しは 2026-10-04 時点で実例なし。決定: 実例が出るまで着手しない。出たら案C を先に、案A はその後。
status: |-
  起草(2026-10-04、未決定・実装なし)。着手条件=「ハーネスの写しが本番とずれていたせいで閉ループが通ったまま実機/CI で不具合が出た」実例が1件出ること。
related_adr:
  - "ADR-163"
  - "ADR-164"
  - "ADR-209"
  - "ADR-215"
---

# ADR-224: 閉ループの写しを、ungate で無くすか、純粋関数へ切り出すか

## 背景

閉ループ(擬似 IME + awase の純粋な状態遷移層)は、host(Linux)で動くことが価値。しかし本番の配線の一部は `#[cfg(windows)]` の中にある:
`state/platform_state.rs`(`ImeStateHub`、3308行)、`runtime/`(モジュールごと `#[cfg(windows)]`)、`win32.rs::foreground_scope`。
そのため `tests/support/harness.rs` は、次の配線を**写して**いる(写し元はハーネス冒頭の doc に列挙済み):
`apply_key_effect_prediction`・`effective_open_at`・`warrant_context`/`issue_actuation_order`・`record_explicit_intent`/`write_*`・`kp_predict_key_effect`・`ir_apply_drift_correction` の検知手前まで。
写していないもの: 通過マーク(ADR-187)、`ImeApplyRequested`/`applied` の往復、ForceGuard、TSF warmup、Engine の `on_input`。

リスクは「本番が変わって写しが古くなり、閉ループが通ったまま実機で壊れる」こと。2026-10-04 時点で、**写しが原因の見逃しは実例なし**。

## 選択肢

- **案A: `ImeStateHub`/`journal`/`ime_event_log` を host へ ungate。**
  - 利点: 写しが消え、閉ループが本物の Hub を通る。
  - 費用: `foreground_scope`(Win32 の前面窓)を host 用 stub にする必要があり、通過マークのスコープ失効など Windows 固有の挙動が閉ループから**見えないまま**になる(「通っているように見えて通っていない」)。
    `tests/architecture_guard.rs`(約6000行)・`layer_boundary_guard.rs`(約470行)が `cfg(windows)` 前提のテキスト走査をしており、広く壊れる。ガード側の書き換え量が見積もれていない。
- **案B: 現状維持 + 写しのずれを検知するガードを足す。** ハーネスの doc に書かれた写し元の関数名・行が本番に存在することをテキスト走査で固定する(`ci_test_coverage_guard.rs` と同じ流儀)。
- **案C: 写している配線だけを、ungated なモジュールの純粋関数へ切り出す。** 本番(`ImeStateHub` のメソッド)とハーネスの両方がそれを呼ぶ。Hub 自体は gated のまま。写しが「呼び出し順」だけになる。

## 決定(暫定)

**実例が出るまで着手しない**(2026-10-04 時点の見積もりに基づく暫定判断。着手するかどうかは所有者が決める)。着手条件=写しが原因の見逃しが1件出ること。出たら **案C を先に**(影響が写し元のメソッドに閉じる)、案A は案C で足りないと分かってから。
案B は費用が小さいので、条件を待たずに足してよい(本 ADR では実装しない)。

## 先に確かめること(着手時)

1. 見逃しが「写しのずれ」なのか「写していない部分(通過マーク・往復・ForceGuard・warmup)」なのかを分ける。後者は案A でも案C でも直らない(別のハーネス拡張の話)。
2. 案A を選ぶ場合、`foreground_scope` stub で隠れる挙動の一覧(`mode_key_pass` のスコープ失効)と、ガードテストの書き換え行数を先に実測する。
3. 案C の切り出し対象は、ハーネスの写し元 doc の各行を1つずつ、本番側メソッドの本体が純粋関数の1回呼び出しになるか確認して決める。

## 追記(2026-10-06、ADR-229)

- 案A が懸念した「`architecture_guard`・`layer_boundary_guard` のテキスト走査が広く壊れる」は、**その場で gate を外すだけなら当てはまらない**(ADR-229 の Opus round1 で確認。ガード内の `cfg(windows)` はコメント 4 か所だけで、ガードはファイルをテキストとして読むので gate の有無に影響されない)。壊れるのはファイルや関数を移したとき、ガードが固定する文字列を変えたときだけ(例: `architecture_guard.rs:1382` の `HubClock::wall(crate::hook::current_tick_ms)`)。ガードの費用見積もりには根拠が無かった。`foreground_scope` を stub にすると挙動が隠れる、という懸念は別で、そちらは有効。
- `ime_event_log`・`journal` の ungate は、形式上は案A の一部だが、ADR-229 の段階 0(着手しやすい単位 T3・T4)で扱う。着手条件(見逃しの実例)の判断は本 ADR の決定のまま変えない。`ImeStateHub`(`platform_state.rs`)の ungate は、ADR-229 でも後ろの段階で、案C が先という順番は変えない。
