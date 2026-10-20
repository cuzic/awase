---
title: awase-windows の ECRS 分析 — 統合した候補と着手順
status: 調査のみ(2026-10-20、origin/develop a15081ee)。実装は未着手。各候補の根拠は調査エージェントの報告で、主要な 5 件(F-01・T03・T04・F4・F-07)だけ統合時に grep で再確認した。それ以外は着手前に必ず根拠を再検証すること
created: 2026-10-20
related_adr: ["ADR-229", "ADR-272"]
---

# awase-windows の ECRS 分析 — 統合した候補と着手順

目的: `awase-windows`(殻)を減らし、`awase-windows-core`(OS 非依存の核)を増やす。あわせて ECRS(排除・結合・交換・簡素化)で整理する。
方法: 5 領域に分けて読み取り専用で調査した(Sonnet、各自 grep と通読。cargo は未実行)。生の報告は `raw-*.md`。

## 0. 前提: 現状の数字(2026-10-20、本番コードの推定行数)

| | awase-windows | awase-windows-core | core の割合 |
|---|---|---|---|
| 分割直後(ADR-229 D4、2026-10-10) | 43,352 | 21,087 | 32.7% |
| 現在 | 48,720 | 25,024 | 33.9% |

分割後の新規コードは core 約 8,800・windows 約 8,950 でほぼ半々。**殻は減っておらず増えている。**

## 1. 結論: 効く順に 3 つの層

調査の結果、**殻を減らす最大のてこは「OS 非依存なのに殻に残っているコードを動かす」(R)で、しかもその多くは再発ファミリーの外**にある。

### 層 A — 大きな移動(R)。強い安全網があり、再発ファミリーの外が多い

| ID(報告) | 対象 | 規模 | 安全網 | 再発ファミリー | 先にやること |
|---|---|---|---|---|---|
| F-01(top) | `scancode_pairs/editor/diagram/apply.rs` と `scancode_map.rs` の純粋部 | 本体 約 2,500 + テスト 約 2,900 | 強(Linux で走る 約 130 本) | なし | `scancode_map.rs` のレジストリ I/O(`#[cfg(windows)] mod registry`)を分ける。**参照元は awase-settings とテスト 1 本だけ**(自分で grep 済み)。`awase-settings` が `awase-windows` 全体に依存している点の張り替え |
| F-02(top) | `bug_report.rs`(本体 1,694 + テスト 1,400) | 約 3,100 | 強(53 本) | なし | 殻へ依存する 2 箇所(`key_effect_io::read_persisted_table`、`probe_actuation_fence`)を引数化。serde のフィールド名・`#[serde(default)]` は変えない。先に F-05・F-06 で小掃除 |
| T08(tsf) | `gji_fsm.rs`・`tsf_gate.rs`・`cold_reason.rs`、`decide_transmit_plan` 周り | 約 3,500 | 強(Linux で既に走る) | warmup / IME belief | ADR-272 段階 C。`timed-fsm` を core の依存に足す、`architecture_guard.rs:5126,5149` のパス更新、`unreachable_pub`。先に T01・T04・T05・T07 で縮める |
| R1(focus-state) | `state/platform_state.rs`(5,030)・`sync_actuation.rs` | 約 5,800 | 強 | IME belief / actuation 合流点 | **最も重い判断**。`state/mod.rs:1-6` の所有者決定(INV-A97-1 の `pub(crate)` 強制、Opus 案 (d))の反転になる。R2(前景スコープの注入)が前提。後回し |
| F3(focus-state) | `focus/classifier.rs`・`tracker.rs` の純粋部 | 約 600 | 強(既存テストが Linux で初めて走る) | focus | `#[cfg(windows)]` の外へ分けるだけ。BUG-56/107/108 の回帰が Linux で走る |
| A4(output) | `observer/ime_observer.rs` の判断 | 約 230 + テスト 290 | 中→強 | IME belief | `architecture_guard.rs` 8509-8560 のパス更新 |
| F-03(top) | `gji_charset_autodetect.rs:90-262` の分類 | 約 170 | 強(10 本) | IME belief(BUG-115/143) | 分岐とコメントは動かさない。F-19(改名)と同時 |

層 A を全部やると、殻から約 19,000 行(テスト込み)が出る。**F-01・F-02・F3 だけで約 9,000 行**で、どれも再発ファミリーの外か、既存テストがそのまま移る。

### 層 B — 排除(E)。呼び出し元ゼロの確認済み(grep)

| ID | 対象 | 規模 | 備考 |
|---|---|---|---|
| T03 | `TsfReadiness`(`tsf_gate.rs:313-368` と再エクスポート) | -75 | **自分で grep 済み**(定義と再エクスポートのみ)。`tsf_readiness()` は ADR-212 P4 で撤去済み |
| T04 | `negative_evidence_count`(`probe.rs`) | -45 | **自分で grep 済み**(probe.rs 外 0 件)。ADR-227:99 の「配線し直す(M5)」の扱いを所有者に確認 |
| F4 | `update_focus_info` → `FocusTracker::update` → `CurrentFocus::update`(3 段の転送) | -16 | **自分で grep 済み**(呼び出し 0) |
| F-07 | `ime::get_ime_conversion_mode_raw` | -10 | **自分で grep 済み**(コード呼び出し 0、コメントのみ) |
| T14 | `TsfObservations::gji_monitor_ok()`・`gji_candidate_visible()` | -12 | 未再確認 |
| F-06 | `bug_report.rs` のテスト専用ラッパー 2 つ | -25 | 未再確認。F-02 の前に |
| A19 | `tsf_warmup_coord.rs::defer_vks_if_in_flight`(`#[cfg(test)]` 専用) | -14 | 未再確認 |
| A20 | `mozc_shm.rs::thread_of_window` | -6 | 未再確認 |
| T02 | `gji_warmup_coro.rs:196-215` の到達不能な分岐 | -20 | ADR-272 の Opus レビューと tsf 調査の 2 回で一致。消すと `win32_async` 依存が消え、ungate の余地。`probe_io.rs` に `debug_assert!` を先に足すと安全 |
| T05 | `ChromeProbe`(`TsfProbeCoro` の純粋な newtype) | -110 | 委譲漏れ(BUG-27)の温床。guard 0 件 |
| T07 | `CompositionState` の 2 層(`WarmEpoch`/`ColdContext`)を 1 つに | -120 | 外から使われない(grep 済み、tsf 報告) |
| R15 | 撤去済み関数の経緯を語るコメント | -70(コメント) | `docs/experiments.md` に同内容があるか確認してから |
| F1(focus-state) | UIA 非同期パイプライン(結果を捨ててログを出すだけ) | -440 | **所有者判断が要る**。`BUG-012.md` が「診断ログ用に稼働継続、安全な再有効化には別設計が要る」と明記 |

### 層 C — 結合・簡素化(C/S)。小さいが再発源を潰す

| ID | 対象 | 規模 | 安全網 | 再発ファミリー | 備考 |
|---|---|---|---|---|---|
| R1(runtime) | `handle_wm_timer` が `build_ctx` を手書きで複製 | -17 | 中 | 物理キー押下ラッチ | `architecture_guard` が既知の穴として件数 1 で固定。**最も確実な C** |
| R2(runtime) | `is_guard_pending() \|\| is_toggle_active()` が 3 箇所 | -6 | 強(述語を足せば) | 半角英数トグル | 3 箇所のうち 1 つだけ直し忘れる再発パターン |
| R5(runtime)+A2(output)+A1(output) | 親指キー設定が bootstrap と reload で二重。起動と reload のキー設定手順も二重 | -50〜-120 | 中(guard が二重を固定) | BUG-103 型(起動時と reload の乖離) | `Engine::apply_thumb_config` / `derive_thumb_engine_config` を core に。VK 定数は ADR-019 のため殻で解決して渡す |
| R4(runtime) | 「トグル中なら復元、でなければ書き戻し」が 3 回 | -25 | 中(閉ループが覆うか未確認) | IME belief + 半角英数 | **擬似 IME のカバーが効く候補**。3 つ目だけ journal でなく tracing |
| T09(tsf) | `LiteralDetector::new` と `new_with_pre_send_baseline` | -30 | 強 | warmup | `architecture_guard.rs:9474-9527`(#704 のガード)が名前を走査。両方の名前を残して委譲にする |
| A3(output) | `send_romaji_batched_gated` と `send_romaji_as_tsf_gated` の前処理 | -45 | 無(guard で固定されていない) | warmup | drain は `assess_warmth` より前、という順序をヘルパーに閉じ込める |
| R2(focus-state) | 前景スコープの注入(`shell.rs` の 17 ラッパーと guard の形の固定を消す) | -280 | 強 | IME belief | R1(focus-state) の前提。FCIS P2 の設計意図との衝突を所有者に確認 |
| A5(output) | `Output` → `warmup_coord` の 1 行委譲 約 20 本 | -80 | 中(`.warmup_coord.` の件数 guard が複数) | warmup | T17(tsf)と同じ対象 |

## 2. 推奨しない・見送る(理由つき)

| 候補 | 理由 |
|---|---|
| F-13(`clear_hook_latches_*` の統合) | doc と `architecture_guard` が「別々に持つこと」を意図して固定。削減は約 40 行 |
| F-11(`hook_callback` の分割) | 文字列走査 guard が相対位置を固定。BUG-62/78/131/132 の再発履歴。真理値表テストを core に足してから別 PR |
| R7(runtime、ImmCross の async 実行の統合) | fix-requires-evidence.md の 2026-09-23 の棚卸しで「構造上必要」と結論済み |
| T21(`ImeModeFsm` と `ImeModel::input_mode` の二重) | 意味のある差分がある(`confirmed`、`last_vk_send_ms`)。ADR で扱う案件 |
| T15(`gji_other_bytes`) | 所有者判断で残すと明記されたもの |
| T18(`tip_detector` の 2 関数) | 別プロセス用で差分に意味がある |
| `lifetime_counter` 等の「似た単調カウンタ」の統合 | doc が「意味論が違うので統合しない」と明記 |
| R13(`RuntimeOutbox` の撤去) | `send_keys` 側の起動が全経路で必ず通るかが未確認 |

## 3. 要判断(所有者)

1. **T01 `TsfReadinessProbe` の待ちロジック(約 -300)**: tsf 調査は「本番の生成元が全て `(0, 0)` なので到達しない」とし、ADR-272 の Opus レビューも同じ結論。一方 output 調査は `probe.rs:110-117` の doc(「今 0 が渡っているから分岐を削除しないこと」)を理由に E にしない。**意見が割れている**。doc は tuning の出し入れの履歴(`tuning-constants.md`)を根拠にする。
2. **F1 UIA 非同期の撤去(約 -440)**: BUG-012 の再有効化計画の有無。
3. **T04 の ADR-227 M5**、**R1(focus-state) の INV-A97-1 の反転**。
4. 層 A の移動先: `awase-windows-core` か、専用 crate か。`scancode_*` は意味的に「awase-windows-core(IME 状態の判断)」と合わない。「core の割合を上げる」のが目的なら core、「殻を減らす」だけなら専用 crate や `awase-settings` でもよい。

## 4. 擬似 IME のカバー範囲との関係(重要)

当初は「擬似 IME のカバーを増やしてから ECRS」の順序で考えたが、調査の結果、**層 A・B の大半は擬似 IME を必要としない**(強い単体テストが既にある、または呼び出し元ゼロの削除)。
擬似 IME の拡張が安全網として効くのは、belief / 半角英数トグルに触る **R4(runtime)・R2(runtime)・R1(focus-state)・T21** などに限られる。

擬似 IME が覆っていない範囲(`harness.rs` 冒頭と `QUIRKS` の `modeled: None`):
通過マーク(ADR-187)と mode-key-follow、`ImeApplyRequested`/`applied` の往復・ForceGuard・TSF warmup・Engine の `on_input`、
`ir_apply_drift_correction` の Blind/Read の再送打ち切り・settle 待ち・conv ラッチ、クセ `Q-key-latency-gji`・`Q-imm-probe-bimodal`・`Q-ext-off-chrome-gji`、MS-IME の格子(1 本だけ)。
mode-key-follow を足すには、`ImeStateHub` の `arm_direct_external_change_watch_in_scope`・`follow_direct_read_in_scope`・通過マークの `_in_scope` を `pub` にし、
`PLATFORM_STATE_PUB_FNS`(`architecture_guard.rs:7709`)と `harness.rs` を同じ PR で更新する(ADR-229 の可視性の方針 (a))。`#[ignore]` の `stale_eisu_blocks_engine_…` を外せる。

## 5. 着手順の提案

| 順 | PR | 内容 | 効果 |
|---|---|---|---|
| 1 | 層 B の確認済み 4 件 | T03・T04(M5 確認後)・F4・F-07 | 約 -150 行、リスクほぼなし。調査の信頼性の試金石にもなる |
| 2 | F3 | `focus/classifier.rs`・`tracker.rs` の純粋部を ungate | Linux で約 14 本のテストが走る。BUG-56/107/108 の回帰 |
| 3 | F-02 の前段 | F-05・F-06(`bug_report.rs` の小掃除)→ 殻依存の引数化 → F-02 の移動 | 約 -3,100 行 |
| 4 | F-01 | `scancode_*` の移動(移動先を先に決める) | 約 -5,400 行(テスト込み) |
| 5 | 層 C の確実なもの | R1(runtime)・R2(runtime)・T05・T07 | 再発源の除去 |
| 6 | 擬似 IME 拡張 → R4・R5 | mode-key-follow をハーネスに足してから | belief に触る結合 |
| 7 | T08 / R1(focus-state) | 重い移動。ADR-272 段階 C と所有者決定の見直し | 約 -9,000 行 |

## 6. 注意

- 規模は行数の目分量。cargo は動かしていない。
- 各報告の「読めなかった範囲」に、未発見の重複が残っている可能性がある(`key_pipeline.rs` の約 2/3、`message_handlers.rs` の一部、`ime.rs` の actuation、`imm.rs` ほか)。
- 着手する PR ごとに、(1) 候補の根拠の grep を自分で再実行、(2) `architecture_guard.rs` / `layer_boundary_guard.rs` に固定されていないか確認、(3) 再発ファミリーなら `fix-requires-evidence.md` の (a)/(b)、を行うこと。
- 誤診の前例: 「到達しない」「使われていない」と書いたコードが `dyn Trait`・マクロ・文字列走査で参照されていたことがある。
