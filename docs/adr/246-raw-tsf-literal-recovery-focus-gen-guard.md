---
id: ADR-246
title: |-
  RAW_TSF_LITERAL の回収に focus 世代の照合を足す(BUG-194)
summary: |-
  BUG-194: raw TSF リテラルの回収予約(RAW_TSF_LITERAL: backs/ESC/romaji)は record から flush までの間に focus 世代の照合が無く、
  間に FocusChange が割り込むと残った ESC/BS/romaji が新しい前面窓へ送られうる(実環境では未観測)。
  段の開始時に前景窓(と世代)を採り、読み出し口 flush_raw_tsf_literal_recovery の先頭で照合して、違えば ESC/BS/romaji と旧窓の deferred を送らず捨てる。
status: |-
  起草(2026-10-08)。実装は先行。Opus レビュー round 1 で初版(record 時に世代だけ刻む案)の欠陥を指摘され改訂、round 2 の N-M1(世代の遅れによる誤破棄)を受けて判断を前景窓主体に再改訂。round 3 で収束(Critical/Major なし)。PR #557 で実装済み。実機未確認。
related_adr:
  - "ADR-101"
  - "ADR-103"
  - "ADR-156"
  - "ADR-212"
---

# ADR-246: RAW_TSF_LITERAL の回収に宛先の照合を足す(BUG-194)

> 経緯: 本 ADR は初版の実装(PR #557 の最初のコミット)の**後**に起票した。ADR → Opus レビュー → 実装の順を飛ばしたため、レビューの結果次第で実装を直す/撤回する前提で書く。

## 事実(`origin/develop` `910aa224` 時点。round 1 のレビューで訂正済み)

- 書き込みは `Output::record_raw_tsf_literal` だけ(呼び出し元は `probe_io.rs` の `set_raw_literal` 1 箇所、`RawTsfLiteralRecovery` アーム)。読み出しは `Output::flush_raw_tsf_literal_recovery` の1本(`flush_raw_tsf_literal_backspaces()` の呼び出しもここだけ)。経路は `WM_DRAIN_OUTPUT_QUEUE` → `platform.rs::flush_raw_tsf_literal_recovery`。
- **record から flush までの窓では `cancel_probe` は発火しない**(初版の誤り)。`cancel_probe` を呼ぶのは `GjiAction::CancelProbe` ハンドラ1箇所で、`gji_current_probe_id() == Some(id)` のときだけ。窓の中では `mark_cold_raw_tsf` で GjiFsm が NotStarted に落ち、同じ `step_probe` の `Done` → `finish_probe_stage` が `take_probe_id()` で probe_id を抜く。FocusChange が来ても `CancelProbe` は出ない。したがって `finish_probe_stage` が「raw recovery 側に委ねる」と残した `pending_deferred`(旧窓で打った後続キー)は FocusChange で破棄されず、flush 末尾の `flush_stale_deferred_vks_after_recovery` が新しい前面窓へ送る。
- 危険の窓は「投函 1 回分」ではない(初版の誤り)。record は literal 検出時(`RAW_TSF_LITERAL_DETECT_MS` 300ms、long idle で 500ms)に走り、BS 等の対象はその前に旧窓へ送った文字。warm 経路の `LiteralDetectFsm`(`vk_send.rs`)は GjiFsm の probe を持たないので FocusChange で止まらず、検出が窓切替の後に来うる。
- `ime_mode_focus_gen` は focus の debounce(既定 50ms)と非同期 prefetch の後でしか進まない。一方 `WM_DRAIN_OUTPUT_QUEUE` は段末で即投函される。OS の前景窓は変わったのに世代がまだ、という区間に flush が来ると世代の照合をすり抜ける。
- 前例: ADR-101 の `discard_raw_recovery_if_focus_stale`(`3a763c4d`、`0a7f9067` で削除)は reinit 予約が Scheduled のときだけが対象で、`pending_deferred` も捨て、journal に `DiscardedStale` を残していた。通常回収には元からガードが無い。`DeferredRecoveryOutcomeSummary::DiscardedStale` と `journal_policy` の対応はその名残りとして残っていた。
- `GiveUpTracker`(`tsf/literal_facts.rs`)は「最初の VK 送信時」の世代を刻む(PR #480 Opus r1 M1)。
- 実環境での発生は未観測。Chrome の cold-start リテラルは CI で再現できない(ADR-193、BUG-002)。

## 決定

1. **判断は純関数**: `state/raw_recovery_plan.rs::plan_raw_recovery(recorded: Option<StageOrigin>, now: StageOrigin) -> Send | DiscardStale`。`StageOrigin = { focus_gen, foreground: ForegroundScope }`。記録が無ければ `Send`(従来の挙動)。**主な物差しは前景窓**: 記録時か flush 時のどちらかで前景が取れていれば、前景窓が違うときだけ捨てる(取れていた窓が `INVALID` になったときも捨てる)。**前景が両方取れていないときだけ世代の不一致で決める**。Linux で単体テストが走る。
   - 世代を主にしない理由(round 2 N-M1): 窓切替直後の最初の打鍵は、debounce(50ms)と prefetch で `ime_mode_focus_gen` が進む前に、focus-sync の `injection_mode` 更新だけで送信経路に入る。その打鍵が張る段(`ChromeProbe`・`MsImeReadyCoro` 等、`CancelProbe` で止まらないもの)は「旧窓の世代 N・新しい窓 B」で刻まれ、段の途中で世代が N+1 に進む。世代の不一致だけで捨てると、B 宛ての正しい回収と、段の途中に B で打った deferred(ユーザーの打鍵)を失う。修正前より悪くなる経路である。
   - 代償: 同じ前景窓の中のフォーカス移動(Chrome のアドレスバーとコンテンツ等)は捉えられない。窓をまたぐ誤配送(別プロセスへの BS/ESC)は防ぐ。
2. **宛先は段の開始時に採る**: `Output::install_pending_tsf`(probe/LiteralDetect のインストール口、唯一の `warmup_coord.install_pending_tsf` 呼び出し元)が `StageOrigin`(`ime_mode_focus_gen` と `win32::foreground_scope()`)を `StageRecord` に刻む(warm 経路と Unicode 観察は送信の直後に install するが、同じメッセージ処理の中で同期に続くので宛先は同じ)。`record_raw_tsf_literal` は段の宛先を `raw_literal_origin` に引き継ぐ。検出時の世代は使わない。
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

- 前景が一瞬別窓になる(ダイアログ、Alt+Tab 画面、UAC、ロック、awase 自身のトレイメニュー等)と、正当な回収も捨てる。リテラルが画面に残る側に倒れる。判断は前景窓が主なので、Chrome のタブ・アドレスバー・コンテンツ間の移動(同じ最上位窓の中)では捨てない。その代わり同じ窓内の移動は捉えられない(Q3)。journal の `DiscardedStale` の件数でこの頻度を後から測る。
- 捨てるときは `pending_deferred` を全部捨てる(ADR-101 の前例と同じ)。前景窓が変わった後に新窓で打って deferred に積まれたキーも一緒に失う。窓は段末から flush までの短い間(LL フックが投函済みメッセージより先に処理されうる分)。`DeferredVk` に退避時の宛先を持たせれば旧窓のものだけ捨てられる(round 2 n1)が、今回は入れない。journal の `DiscardedStale.deferred_vk_count` で規模を測る。
- 予約は段ごとに1つだけ持てる(`record` は `store` で上書き)。上書きは flush の前に二重に record されない現状の経路では起きない(`RawTsfLiteralRecovery` の record は1箇所)。

## テスト

- `state/raw_recovery_plan.rs` の単体テスト(Linux で走る): 一致、記録なし、**世代だけ進んで前景が同じなら送る(N-M1 の時系列)**、世代は同じで前景だけ変わった、前景が消えた、前景が最初から不明(世代で決める)。
- `output/mod.rs::raw_tsf_literal_and_deferred_are_discarded_when_foreground_changed_before_flush`(Windows CI のみ): 段中に deferred 2 件 → record → 段末で probe を外す → `cancel_probe` を介さず、前景窓が変わった宛先で flush 判断(`discard_raw_recovery_if_moved_at` に宛先を渡し、実機の前景に依存させない)→ 破棄(backs 2・deferred 2 が `DiscardedStale` に出る)。宛先が同じなら残る。世代だけ進んでも残る。予約が無い回は deferred を捨てない。
- `tests/architecture_guard.rs::raw_tsf_literal_recovery_is_guarded_by_stage_origin`: 空白を除いて、`install_pending_tsf` の刻印、`record_raw_tsf_literal` の引き継ぎ、`plan_raw_recovery` の使用、flush の先頭の早期 return(ESC/BS 送信より前)、全ソースで `flush_raw_tsf_literal_backspaces()` の呼び出しが1箇所、を固定する。

## 未決

- Q1: 前景窓が一瞬変わる場面(ダイアログ、最小化)での誤破棄の頻度。journal で測る。
- Q3: 同じ前景窓の中の移動を捉える必要があるか。必要なら、デバウンスしない focus 連番を WinEvent の即時処理で進めて `StageOrigin` に使う案(round 2 N-M1 案1)に進む。安い代案として、`GetGUIThreadInfo` の `hwndFocus` も比べる案がある(Chrome のアドレスバーとページは別 hwnd)。今回は入れない。
- Q2: 破棄時に deferred を全部捨てる粒度。`DeferredVk` に退避時の宛先を持たせれば旧窓のものだけ捨てられるが、今回は入れない(トレードオフ節)。

## Q3 追補: 同じ前景窓の中の移動を `hwndFocus` で捉える(起草 2026-10-08、未実装)

実環境でこの経路が起きた記録はまだ無い(BUG-194 は実機未確認)。ここでは「ガードが働くことを CI で固定する」範囲に限って決める。発生頻度の測定は引き続き journal の `DiscardedStale` で行う。

### 決定案(案2: `hwndFocus` を足す)

1. `StageOrigin` に `focus_hwnd: isize`(`GetGUIThreadInfo` の `hwndFocus`。取れない・null は 0)を足す。
2. `plan_raw_recovery`: 前景窓が記録時・flush 時とも有効で**同じ**、かつ `focus_hwnd` が**両方 0 でなく**違うときは `DiscardStale`。どちらかが 0 なら今までどおり送る(判断材料が無いときは従来の挙動)。
3. 採取は `win32::foreground_scope()` と同じ場所に `focus_hwnd()` を足す。`GetGUIThreadInfo` は対象スレッドのハングで止まりうる(`get_gui_thread_info_with_timeout` の doc)ので、タイムアウト付きの既存ラッパーを短い上限(`ime.rs:940` の前例 30ms)で使い、タイムアウト時は 0 とする。

### 却下・保留
- 案1(デバウンスしない focus 連番を WinEvent で進める): focus 遷移の再発ファミリーに触れ、世代の遅れの問題を別の形で持ち込む。実害の記録が無いので入れない。

### テスト
- `state/raw_recovery_plan.rs` の単体(Linux): 前景同じ・`focus_hwnd` 違い → 捨てる、片方 0 → 送る、前景違いは従来どおり、世代だけ進んで `focus_hwnd` 同じ → 送る(N-M1 を壊さない)。
- `output/mod.rs` の Windows テスト: 既存の `discard_raw_recovery_if_moved_at` に `focus_hwnd` を渡し、deferred 込みで破棄されることを固定する。
- `architecture_guard`: `focus_hwnd` の採取が段の開始と flush の両方で `current_stage_origin` 1 本を通ることを固定する。

### 未決(Opus レビューで確認)
- 段の開始(`install_pending_tsf`)で `GetGUIThreadInfo` を呼ぶコスト。頻度は warmup 段の開始のみで打鍵ごとではない前提だが、実測していない。
- TSF native アプリ(Chrome 等)で、入力中に `hwndFocus` が正当に揺れる経路が無いか(誤破棄=リテラルが画面に残る側)。揺れるなら案2は入れない。
