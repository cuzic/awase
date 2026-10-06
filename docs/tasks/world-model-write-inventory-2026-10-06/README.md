---
title: 世界モデル(状態)の書き込み点の棚卸し W0 — 集計
status: 棚卸し完了(2026-10-06、読み取りのみ)。後続: docs/tasks/fcis-layering-tasks-2026-10-06.md の「世界モデルと Effect 列の検討結果」
created: 2026-10-06
related_adr: ["ADR-229", "ADR-212", "ADR-208", "ADR-164"]
---

# 世界モデル(状態)の書き込み点の棚卸し W0 — 集計

所有者の構想「ワールドモデルを Redux/Elm Architecture のように reduce する」に向けて、現状の書き込み点を実測した(origin/develop `ee54b2de` 時点)。生レポートは同ディレクトリの `inventory-w0-{a,b,c}.md`(a: `ImeStateHub`・`ImeModel`・`ImeEvent`、b: `PlatformState`・`Runtime`・フォーカス・エンジン、c: グローバル/`static`/atomic)。

## 結論: 仮説は成立する(`reduce` を通る部分は小さく、再生もできない)

| 観点 | 実測 |
|---|---|
| `ImeEvent` | **20 variant**(約 65 ではなかった)。うち 2 つ(`UserImeToggleIntent`・`UserChangedInputMode`)は本番に発行元が無い |
| `ImeModel` | 16 フィールドのうち `reduce` だけが書くのは 10 個 |
| `ImeStateHub` | `&mut self` メソッド 53 個のうち、Event を一切出さないものが 27 個。`reduce` を通る状態は `shadow_model` だけ(14 フィールド中) |
| `Runtime` | 40 フィールドのうち `ImeEvent` の `reduce` を通るものは 0。`FocusStore`・`GateStore`・`KeymapStore` の 16 フィールドもすべて `pub` で `reduce` を通らない。範囲内の約 196 の状態のうち、`ImeEvent` で書かれるのは 2 つだけ |
| グローバル | `static` 58 件・`thread_local!` 6 セル。`reduce` を通る書き込みは 0。`HOOK_STATE`(23 フィールド): フックのみ 2、メインのみ 7、両方 14。`TSF_OBS`(22 フィールド)は全て直接の store(書き手は `WinEventProc` と `gji-io-monitor` の 2 系統) |
| 世界モデルの一部をグローバルが持つ例 | `candidate_was_seen`・IME 同定の 3 フィールド・`ime_composition_active`・`literal_session_confirmed_gen`・`OUTPUT_GATE`/`INPUT_DEFER`/`FOCUS_RESYNC`・`SEND_HEALTH` のブレーカ状態 |
| journal からの再現 | **できない**。`JournalEntry::ImeEvent` が `tick_ms`・`Instant`・event_log の `seq` を持たず、`ImeEvent` は `Serialize` のみ。`applied`/`pending`/`input_barrier`/`force_guards`/`intent_store`/`belief` への直接書き込みは記録が無い。`event_log` は本番で読み手がいない。エンジンも、journal の `KeyInput` が `RawKeyEvent` 全体と `InputContext` を持たないので再生できない |
| 時刻の入口 | 3 つが独立(reducer 用の `EventTime`、`HubClock`、呼び出し側の `Instant::now()`・`current_tick_ms()`・`foreground_scope()`) |
| reducer | 4 系統(`ImeModel`・エンジン・`GjiFsm`・`ProbeCoroState`)があり、互いに状態を複写し合っている |
| 失効判定 | カウンタが 8 種類、フォーカス hwnd のコピーが 4 か所、`await` またぎの失効の手書きの値の一致比較(`last_explicit_ime_action_ms`、`key_pipeline.rs:425/664`)が分散 |
| フォーカス遷移 | 入口 4〜5 個、書く関数 約 14 個、9 種の型にまたがる。`force_guards` と cold マークは 2 か所で二重に消している。WinEvent の入口は `ime_mode_focus_gen` を更新せず、デバウンス後の経路だけが更新する |
| 死んだ状態 | `Runtime.state_dependent_key_warning_dialog`(宣言と初期化だけで、一度も使われていない) |

## Event 化の難度(グローバル、`inventory-w0-c.md`)

低: `RAW_TSF_LITERAL`、`thread_local` 2 つ(`PROBE_TIMED_OUT`・`TSF_PROBE_SNAP`)、`pending_*_composition`。中: `TSF_OBS` のうち `gji-io-monitor` が書く 7 フィールドと WinEvent 系。高: フックスレッドが書く `HOOK_STATE`、syscall の前後の同期点が要る `CONV_MUTATION_SEQ`・`PROBE_FENCE`。

## 危険箇所

`await` をまたぐ失効トークンの比較が 3 点に分散、`OUTPUT_GATE` と `FOCUS_RESYNC` の 2 ゲートの合成、`WinEventProc` の再入、問い合わせの名前の `&mut self` メソッドが OS の前面ウィンドウに依存して latch の状態を変える、`effective_open()` の値で `applied` を `Confirmed` に書く経路(`runtime/mod.rs:1409`、`ime_refresh.rs:618`)。
