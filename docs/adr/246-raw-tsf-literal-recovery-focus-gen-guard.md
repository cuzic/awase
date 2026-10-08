---
id: ADR-246
title: |-
  RAW_TSF_LITERAL の回収に focus 世代の照合を足す(BUG-194)
summary: |-
  BUG-194: raw TSF リテラルの回収予約(RAW_TSF_LITERAL: backs/ESC/romaji)は record から flush までの間に focus 世代の照合が無く、
  間に FocusChange が割り込むと残った ESC/BS/romaji が新しい前面窓へ送られうる(実環境では未観測)。
  段の開始時に (focus 世代, 前景窓) を採り、読み出し口 flush_raw_tsf_literal_recovery の先頭で照合して、違えば ESC/BS/romaji と旧窓の deferred を送らず捨てる。
status: |-
  起草(2026-10-08)。実装は先行。Opus レビュー round 1 で初版(record 時に世代だけ刻む案)の欠陥 5 件を指摘され、本版で改訂。round 2 待ち。
related_adr:
  - "ADR-101"
  - "ADR-103"
  - "ADR-156"
  - "ADR-212"
---

# ADR-246: RAW_TSF_LITERAL の回収に宛先の照合を足す(BUG-194)

> 経緯: 本 ADR は実装(f9c80e57)の**後**に起票した。ADR → Opus レビュー → 実装の順を飛ばしたため、レビューの結果次第で実装を直す/撤回する前提で書く。

## 事実(`origin/develop` `910aa224` 時点。round 1 のレビューで訂正済み)

- 書き込みは `Output::record_raw_tsf_literal` だけ(呼び出し元は `probe_io.rs` の `set_raw_literal` 1 箇所、`RawTsfLiteralRecovery` アーム)。読み出しは `Output::flush_raw_tsf_literal_recovery` の1本(`flush_raw_tsf_literal_backspaces()` の呼び出しもここだけ)。経路は `WM_DRAIN_OUTPUT_QUEUE` → `platform.rs::flush_raw_tsf_literal_recovery`。
- **record から flush までの窓では `cancel_probe` は発火しない**(初版の誤り)。`cancel_probe` を呼ぶのは `GjiAction::CancelProbe` ハンドラ1箇所で、`gji_current_probe_id() == Some(id)` のときだけ。窓の中では `mark_cold_raw_tsf` で GjiFsm が NotStarted に落ち、同じ `step_probe` の `Done` → `finish_probe_stage` が `take_probe_id()` で probe_id を抜く。FocusChange が来ても `CancelProbe` は出ない。したがって `finish_probe_stage` が「raw recovery 側に委ねる」と残した `pending_deferred`(旧窓で打った後続キー)は FocusChange で破棄されず、flush 末尾の `flush_stale_deferred_vks_after_recovery` が新しい前面窓へ送る。
- 危険の窓は「投函 1 回分」ではない(初版の誤り)。record は literal 検出時(`RAW_TSF_LITERAL_DETECT_MS` 300ms、long idle で 500ms)に走り、BS 等の対象はその前に旧窓へ送った文字。warm 経路の `LiteralDetectFsm`(`vk_send.rs`)は GjiFsm の probe を持たないので FocusChange で止まらず、検出が窓切替の後に来うる。
- `ime_mode_focus_gen` は focus の debounce(既定 50ms)と非同期 prefetch の後でしか進まない。一方 `WM_DRAIN_OUTPUT_QUEUE` は段末で即投函される。OS の前景窓は変わったのに世代がまだ、という区間に flush が来ると世代の照合をすり抜ける。
- 前例: ADR-101 の `discard_raw_recovery_if_focus_stale`(`3a763c4d`、`0a7f9067` で削除)は reinit 予約が Scheduled のときだけが対象で、`pending_deferred` も捨て、journal に `DiscardedStale` を残していた。通常回収には元からガードが無い。`DeferredRecoveryOutcomeSummary::DiscardedStale` と `journal_policy` の対応はその名残りとして残っていた。
- `GiveUpTracker`(`tsf/literal_facts.rs`)は「最初の VK 送信時」の世代を刻む(PR #480 Opus r1 M1)。
- 実環境での発生は未観測。Chrome の cold-start リテラルは CI で再現できない(ADR-193、BUG-002)。

## 決定

1. **判断は純関数**: `state/raw_recovery_plan.rs::plan_raw_recovery(recorded: Option<StageOrigin>, now: StageOrigin) -> Send | DiscardStale`。`StageOrigin = { focus_gen, foreground: ForegroundScope }`。世代が違う、または前景窓が違えば `DiscardStale`。記録が無ければ `Send`(従来の挙動)。前景が取れない(`INVALID`)ことが記録時も flush 時も同じなら前景は判断に使わず世代だけで決める。Linux で単体テストが走る。
2. **宛先は段の開始時に採る**: `Output::install_pending_tsf`(probe/LiteralDetect のインストール口、唯一の `warmup_coord.install_pending_tsf` 呼び出し元)が、最初の VK 送信より前に `StageOrigin`(`ime_mode_focus_gen` と `win32::foreground_scope()`)を `StageRecord` に刻む。`record_raw_tsf_literal` は段の宛先を `raw_literal_origin` に引き継ぐ。検出時の世代は使わない。
3. **照合は flush の先頭の1箇所**: `discard_raw_recovery_if_moved` が `DiscardStale` と判断したら、`backs`/`escape_composition`/`romaji` と `pending_deferred` を捨て、`RawRecoveryOutcome::DiscardedStale { backs, romaji_present, deferred_vk_count }` を返して早期 return する(ESC/BS も romaji 再送も `flush_stale_deferred_vks_after_recovery` も走らない)。journal の `DiscardedStale` を再び使う。
4. **予約が無い回は何もしない**: flush は drain のたびに走るので、`RAW_TSF_LITERAL` に予約が無ければ判断せず、無関係な deferred も捨てない。
5. 前景窓の読み取りは `win32::foreground_scope()`(非ブロッキングの `GetForegroundWindow`)。`output/` は既に `crate::win32::` を使っている。

## 検討した代案

| 案 | 却下理由 |
|---|---|
| 初版: record 時に世代だけ刻み、flush で照合 | 刻む時点が検出時で遅い(warm 経路で窓切替後に刻まれ素通り)。世代は実際の前景変化より遅れる。deferred が新窓へ送られる。journal に残らない |
| `on_ime_mode_focus_changed` で即クリア | フォーカス処理から回収の static を触る。世代の遅れ(debounce)を解決しない |
| `cancel_probe` でクリア | 窓の中では発火しない(上記)。ImeOff・composition reset でも発火し正当な回収を消す |
| ESC/BS だけ守り、romaji 再送と deferred は新窓へ送る(縮小案) | romaji も deferred も旧窓宛てなので、新窓へ出せば同じ誤配送 |
| 何もしない | 被害は他窓への BS/ESC で取り返せない |

## 残るトレードオフ

- 疑似 FocusChange、または前景が一瞬別窓になる(IME の候補窓などは前景にならないが、ダイアログ等)と、正当な回収も捨てる。リテラルが画面に残る側に倒れる。Chrome ではタブ・アドレスバー・コンテンツ間で focus が連続して動くため、cold-start 直後に捨てる頻度が想定より高い可能性がある。journal の `DiscardedStale` の件数でこの頻度を後から測る。
- 捨てるときは `pending_deferred` を全部捨てる(ADR-101 の前例と同じ)。FocusChange の後に新窓で打って deferred に積まれたキーも一緒に失う。窓は flush までの短い間だけ。
- 予約は段ごとに1つだけ持てる(`record` は `store` で上書き)。上書きは flush の前に二重に record されない現状の経路では起きない(`RawTsfLiteralRecovery` の record は1箇所)。

## テスト

- `state/raw_recovery_plan.rs` の単体テスト(Linux で走る): 世代一致、記録なし、世代が進んだ、**世代は同じで前景だけ変わった**、前景が消えた、前景が最初から不明。
- `output/mod.rs::raw_tsf_literal_and_deferred_are_discarded_when_focus_changed_before_flush`(Windows CI のみ): 段中に deferred 2 件 → record → 段末で probe を外す → `cancel_probe` を介さず FocusChange → 破棄(backs 2・deferred 2 が `DiscardedStale` に出る)。宛先が同じなら残る。予約が無い回は deferred を捨てない。
- `tests/architecture_guard.rs::raw_tsf_literal_recovery_is_guarded_by_stage_origin`: 空白を除いて、`install_pending_tsf` の刻印、`record_raw_tsf_literal` の引き継ぎ、`plan_raw_recovery` の使用、flush の先頭の早期 return(ESC/BS 送信より前)、全ソースで `flush_raw_tsf_literal_backspaces()` の呼び出しが1箇所、を固定する。

## 未決

- Q1: 前景窓が一瞬変わる場面(ダイアログ、最小化)での誤破棄の頻度。journal で測る。
- Q2: 破棄時に deferred を全部捨てる粒度(新窓で打ったキーを残す案は、`DeferredVk` に世代が無いので採れない)。
