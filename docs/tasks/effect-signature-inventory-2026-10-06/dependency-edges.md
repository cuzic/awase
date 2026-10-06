---
title: Effect の依存辺(順序の制約)— E0 §11 を辺の表に書き直したもの
status: 起草(2026-10-06)。所有者の方針(「ほとんどの処理は順序の考慮不要。たまにあるが、順序というより DAG 的な依存関係グラフ」)を受けて作成
created: 2026-10-06
related_adr: ["ADR-229", "ADR-208", "ADR-156", "ADR-180", "ADR-163"]
---

# Effect の依存辺(順序の制約)

E0(`inventory-e0.md` §11)は、順序の制約が暗黙のものを 22 件挙げた。所有者の指摘(2026-10-06): **ほとんどの処理は順序を気にしなくてよい。たまにあるが、それは全体の線形な順序ではなく、少数の「これはこれより前」という依存関係(DAG の辺)**。これに合わせて、E0 の 22 件を**辺の表**に書き直す。

## 原則

- **既定は「辺が無い=自由に並べ替えてよい」**。辺が無い組には順序の制約が無い。
- **実行時に DAG をスケジューリングしない**。コードの順序のまま実行する。DAG は、F の分割が辺を保っているかを確かめる**検証の道具**(Opus の Effect 設計レビュー: Stage の全順序は、既存の暗黙の順序を壊す危険が最も高い)。
- **F の分割の PR は、その PR が触る辺を本文に書き、その辺を固定するテストを足す**(触らない辺のテストは足さない。順序契約のテストを全体には作らない)。

## 辺(真の順序の制約。before → after)

| # | before → after | 理由・根拠 | 触る F |
|---|---|---|---|
| e1 | `plan()` → `kp_latch_keyup_to_keydown_disposition` → `KeyInput` の journal 記録 → `kp_stage_execute` | `physical` を 1 回だけ確定して記録と実配送の両方へ渡す(BUG-173 追補、BUG-90) | key_pipeline の分割 |
| e2 | `kp_stage_shadow_ime_toggle` → `engine.on_input`(ただし `engine_owns_open_key` は shadow の判断**前**の belief で組む) | `ctx` は shadow の書き込み後の belief で組む(ADR-208 D1、PR #419 M-4)。`pre_ctx` と `ctx` が時刻違いで存在 | key_pipeline の分割 |
| e3 | `claim_press_write` → order の発行 →(同期で何も送らなければ)`release_press_write` | ADR-208 L1、BUG-113。release は sync だけ・`outcome_sent_nothing` だけ | F2(`plan_set_open`)ほか |
| e4 | `timer.kill(TIMER_IME_REFRESH)` → 書き込み。`record_confirmed(false)` → async 書き込み | PR #408 Opus M-1、ADR-098 決定 5/6-a | key_pipeline(`kp_shadow_actuate`) |
| e5 | `prev_elapsed_ms` の読み取り → `mark_send()` → `SendInput`。`send_keys` の中で診断スナップショットを取らない | `WH_KEYBOARD_LL` の再入、output in-flight guard の基準点 | F6(Output) |
| e6 | 検出ベースラインの取得 → `SendInput` | BUG-027/029/030/033、ADR-079(`new_with_pre_send_baseline`) | F6、warmup |
| e7 | `drain_pending_deferred_before_send_if_queue_only` → `assess_warmth` | ADR-123 変更 A+C 決定 4-3 | F6 |
| e8 | `drain_pending_composition_events` → `step_probe` | VK_A+BS で SHOW+HIDE が first tick 前に終わる IPC race | warmup |
| e9 | `last_user_explicit_off_ms` の更新 → `event_log.record_at` → `reduce` → `journal.record` | journal が reduce の後なので、reduce 前の状態を再構成できない(W-c の `event_seq`/`tick_ms` は `record_at` の `EventTime`) | 世界モデル |
| e10 | `ImeOpenApplied` → `post_ime_refresh` → `record_ime_apply_result` → `on_ime_applied`(`UnsafeToToggle` でも早期 return しない) | BUG-34 横展開 D-prep(pending が残留する固着) | on_ime_apply_complete |
| e11 | `install_pending_tsf()` → `RuntimeRequest::StartTsfProbe` の push | H-4-a | outbox |
| e12 | `execute_relay` の Consume: **Timer は即時、他はキュー**(Timer がキューを追い越す) | `deferred_engine_timers` の `(logical_id, os_id)` 照合(「というのは→とはいうの」) | **F3** |
| e13 | `OutputActiveGuard::begin()` → `spawn_local`(reinject と async actuation の両方) | await 中のフック分配 | F3・F6(ADR-156) |
| e14 | `was_user_enabled` の読み取り → `on_command` | issue #137 3 周目 | toggle_engine |
| e15 | `Engine::on_command(ToggleEngine/SwapLayout)` → `discard_ime_open_request` | ADR-092 Step4b | engine |
| e16 | IMC 経路: `commit_enter_imc` → `actuate_conv_mode`(無条件に前)。GJI 経路: `SendInput` 成功 → `commit_enter_gji`(成功後) | 半角英数トグル §3 原則 2(同じ enum の兄弟で commit の位置が逆) | key_pipeline |
| e17 | `romaji_pre_write`(同期ブロック) → `apply_mechanism` の機構の実行(最初に)。`SendHealth` の gate は意図的に無し | BUG-34 横展開(skip すると ROMAN が次のトグルまで固着) | ime_controller |

## 辺ではない制約(順序ではなく、対・整合・再検査)

E0 の 22 件のうち、次の 5 件は、**順序ではなく別の種類の制約**。DAG の辺には入れない。

| # | 種類 | 内容 | 根拠 |
|---|---|---|---|
| c18 | 整合 | `SyncChainWriter::write` と `apply_mechanism` が、同じ view で `decide_attempt` を 2 回決める(記録用と実行用)。片方の入力を変えると記録と実送信が乖離 | ADR-163 TH1b-2b。F の分割の候補(command を渡して 1 回にする) |
| c19 | 再検査 | `decide_gate` を await の前後で毎回判定し直す。`with_app` を内包する共有ヘルパーにしない(再入でゲートが無効化) | ADR-180 決定 1、issue #136/BUG-90 |
| c20 | 表 | `WM_*` の再入の扱い(`with_app` で捨てる vs `with_app_or_repost` で再 post)が handler ごとにバラバラ。捨ててよい WM の表が無い | issue #137 |
| c21 | 対 | `INPUT_DEFER` の `defer_during_output`(post しない)と `replay_later`(post する)の非対称 | ADR-156 |
| c22 | 対 | defer 側の `raw_recovery_owns_deferred()` は gate で切り替え、drain 側は gate に関わらず常に見る | ADR-128、round4-3 |
| c23 | 対 | executor キューの defer 側(`run_passthrough_pipeline` の `output_in_flight`)と drain 側(`reinject_wait_remaining`)が同じ出力ガード閾値(`relay_plan::output_guard_remaining_ms`)を使う。残る非対称: defer 側 `has_pending` は `has_pending_tsf_work()` を OR(BUG-58)、drain 側は確定キー KeyDown のみ | ADR-156、ADR-123→128。F3 で固定(`architecture_guard`・`relay_plan` の境界テスト)。c21・c22 は F3 では触らない |

## 使い方

- F の分割の PR は、本文に「この PR が触る辺: eN(…)」を書き、**触る辺を固定するテストを足す**(例: F3 は e12・e13)。触らない辺には何もしない。
- 辺が増える(新しい暗黙の順序を見つけた)ときは、本表に足し、必要ならガードにする。
- 辺の数が多いのは、`key_pipeline` と Output(F6)。ここを分けるときに、最も辺を壊しやすい。
