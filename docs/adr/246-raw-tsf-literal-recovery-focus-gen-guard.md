---
id: ADR-246
title: |-
  RAW_TSF_LITERAL の回収に focus 世代の照合を足す(BUG-194)
summary: |-
  BUG-194: raw TSF リテラルの回収予約(RAW_TSF_LITERAL: backs/ESC/romaji)は record から flush までの間に focus 世代の照合が無く、
  間に FocusChange が割り込むと残った ESC/BS/romaji が新しい前面窓へ送られうる(実環境では未観測)。
  record 時に ime_mode_focus_gen を刻み、読み出し口 flush_raw_tsf_literal_recovery の先頭で照合して、違えば送らず捨てる。
status: |-
  起草(2026-10-08)。実装は先行(f9c80e57、ブランチ fix/bug194-raw-tsf-literal-focus-gen、未 push)。Opus レビュー前。
related_adr:
  - "ADR-101"
  - "ADR-103"
  - "ADR-156"
  - "ADR-212"
---

# ADR-246: RAW_TSF_LITERAL の回収に focus 世代の照合を足す(BUG-194)

> 経緯: 本 ADR は実装(f9c80e57)の**後**に起票した。ADR → Opus レビュー → 実装の順を飛ばしたため、レビューの結果次第で実装を直す/撤回する前提で書く。

## 事実(`origin/develop` `910aa224` で確かめたもの)

- 書き込みは `Output::record_raw_tsf_literal`(`output/mod.rs`)だけ。呼び出し元は `probe_io.rs` の `set_raw_literal` 1 箇所。
- 読み出しは `Output::flush_raw_tsf_literal_recovery` の 1 本(`flush_raw_tsf_literal_backspaces` の呼び出しもここだけ)。経路は `WM_DRAIN_OUTPUT_QUEUE` ハンドラ(`message_handlers.rs`)→ `platform.rs::flush_raw_tsf_literal_recovery`。ほかに `raw_recovery_owns_deferred`(`backs`/`romaji` を読むだけで書かない)がある。
- `cancel_probe`(ImeOff/FocusChange/composition reset で発火)は deferred を破棄するが `RAW_TSF_LITERAL` に触れない。特性テスト(`9afe8fac`、Windows CI で PASS)で固定済み。
- 過去の `discard_raw_recovery_if_focus_stale`(ADR-101、`3a763c4d`)は reinit 予約が Scheduled のときだけが対象で、ADR-212 P3 の reinit 撤去(`0a7f9067`)で削除された。通常回収(consecutive==0)には元からガードが無い。
- 割り込み経路: `RawTsfLiteralRecovery` と `Done` は同一の `step_probe` 内で同期に処理され、`WM_DRAIN_OUTPUT_QUEUE` は PostMessage で届く。その間にキュー済みの FocusChange が先に処理されると、残りが別窓へ届く。窓は投函 1 回分。
- 実環境での発生は**未観測**。Chrome の cold-start リテラルは CI で再現できない(ADR-193、BUG-002)。

## 決定案

1. `Output` に `raw_literal_focus_gen: Cell<FocusGen>` を足し、`record_raw_tsf_literal` が `ime_mode_focus_gen` を刻む。
2. `flush_raw_tsf_literal_recovery` の先頭で `discard_raw_recovery_if_focus_stale` を呼ぶ。刻んだ世代と現在値が違えば `backs`/`escape_composition`/`romaji` を送らずに捨て、`warn!` を出す。何も予約が無ければ何もしない。
3. deferred は FocusChange の `cancel_probe` が既に破棄しているので触れない。

## 検討した代案

| 案 | 却下理由 |
|---|---|
| `on_ime_mode_focus_changed` で `RAW_TSF_LITERAL` を即クリア | フォーカス処理から回収の static を触る。捨てた理由をログに残しにくい |
| `cancel_probe` でクリア | ImeOff・composition reset でも発火し、同一窓での正当な回収まで消える |
| 世代でなく前面 HWND を記録して比較 | 既存の世代機構と二重になる |
| 何もしない | 実害未観測。ただし被害は他窓への BS/ESC で、後から取り返せない |

## 残るトレードオフ

- 疑似 FocusChange(IME ON/OFF の副作用)でも世代は進む。その場合は正当な回収も捨て、リテラルが画面に残る側に倒れる。
- 捨てたぶんの文字は失われたままになる(ADR-100 案 L の journal 記録の対象外)。

## テスト

- `output/mod.rs::raw_tsf_literal_is_discarded_when_focus_changed_before_flush`(Windows CI のみ。特性テスト `9afe8fac` の期待を反転)。
- `tests/architecture_guard.rs::raw_tsf_literal_recovery_is_guarded_by_focus_gen`(刻印・照合が送信より前・`flush_raw_tsf_literal_backspaces()` の呼び出し 1 箇所)。

## 未決

- Q1: 世代でなく「実際に窓が変わったか(HWND/pid)」で判定すべきか。
- Q2: 捨てたことを journal に残すべきか(現状は `warn!` のみ)。
- Q3: 実害未観測のまま入れてよいか。
