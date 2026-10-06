---
title: awase-windows の層の棚卸し(OS 非依存側へ移せる範囲・Windows 側に残す範囲)
status: 棚卸し完了(2026-10-06)。読み取りだけで、コード変更・ビルド・テストはしていない。後続: ADR-229(起草)
created: 2026-10-06
related_adr: ["ADR-163", "ADR-164", "ADR-180", "ADR-224", "ADR-156", "ADR-129"]
---

# awase-windows の層の棚卸し — 集計

> 対象コミットは develop `9df983e4`。その後 `crates/awase-windows/src` で変わったのは `tsf/gji_fsm.rs` の +56 行だけ(`cb378654` 時点で確認)。
> 担当領域ごとの生レポートは同ディレクトリの `inventory-{a1,a2,b,c,d1,d2,f1,f2}.md`。

詳細は同ディレクトリの inventory-{a1,a2,b,c,d1,d2,f1,f2}.md。分類記号: P=純粋 / P*=継ぎ目を直せば移せる / O=観測 / E=実行 / F=判断と I/O が混在(分割が要る) / G=配線。数字は本体行数(コメント込み、テスト除く)。ビルドもテストも実行していないので、ungate 後にコンパイルが通るかは全て未確認。

## 1. 領域別の内訳

| 領域 | 担当 | 本体 | P | P* | O+E+G | F | 移せる(P+P*) |
|---|---|---:|---:|---:|---:|---:|---:|
| hook / platform / journal / vk / lib | a1 | 7,439 | 3,083 | 966 | 2,295 | 1,095 | 4,049 (54%) |
| ime / imm / ime_controller ほか | a2 | 3,889 | 536 | 654 | 1,101 | 1,087 | 1,190 (31%) |
| platform_state / open_chain / executor / ime_refresh | b | ~4,776 | — | — | 320 | 1,922 | 2,534 (53%) |
| key_pipeline / runtime/mod ほか | c | 6,088 | 543 | 2,238 | 1,199 | 2,108 | 2,781 (46%) |
| message_handlers / focus_tracking / focus / observer | d1 | 6,392 | — | — | 1,760 | 1,771 | 2,568 (40%) |
| app / tray / output / src 直下その他 | d2 | 12,627 | 5,492 | 1,644 | 3,153 | 2,225 | 7,136 (57%) |
| tsf/ 直下 | f1 | 4,983 | 2,540 | 554 | 1,460 | 350 | 3,094 (62%) |
| tsf/warmup/ | f2 | 2,656 | — | — | 694 | 84 | 1,878 (71%) |
| **合計** | | **~48,850** | | | **~11,980** | **~10,640** | **~25,230 (52%)** |

- state/ の ungated 51 ファイルは別枠(f2): 本体 14,877 行のうち実コード 7,977 行、テスト 676 本。**すでに Linux で回っている**。
- 「移せる」25,230 行のうち、すでに ungated で Linux テストが回っているのは約 7,700 行(d2 約 4,200、a1 約 1,300、d1 約 860、f1 約 1,330 の概算)。**新たに Linux でテストできるようになるのは約 17,500 行**。
- gate のせいで Linux に存在しないテストは、数えられた分だけで約 180 本(c 51、d2 71、d1 36、a1 22 ほか。a2 の 447 行、b の platform_state 内テストは本数未集計)。

## 2. F(分割が要る判断混在)の所在

| 場所 | F の行数 | 主な関数 |
|---|---:|---|
| runtime/ (b+c+d1) | 約 5,800 | ime_refresh 900、key_pipeline 系、focus_tracking(63%が F。`on_focus_process_changed` 1 関数 307 行)、open_chain 533、executor 463 |
| output/ | 約 1,430 | vk_send 623、output/mod 576 |
| platform.rs | 791 | `gji_on_focus_change`(683 行) |
| ime/imm/ime_controller | 1,087 | `apply_mechanism`、`romaji_pre_write`、`ActuationTarget` 周り |
| tsf/ | 434 | `monitor_loop`(202 行)、`run_start` 84 行 ほか |

**tsf/ は F が少なく(434 行)、コルーチン群が最もきれいに移せる**。F は runtime/ と output/ に集中している。

## 3. 領域をまたぐ継ぎ目(seam)

| # | 継ぎ目 | 規模 | 難度 |
|---|---|---|---|
| S-A | gated な小さな型・定数を ungated へ出す(`ColdReason`、`DetectionResult`、`ImeModeState`、`ForegroundScope`、`ImeUpdate`/`ImeObs`、`ActiveImeKind`、`SentKeyEvent`、`InjectionHint`、マーカー定数 3 値 ほか) | 約 20 型、計数百行 | 機械的 |
| S-B | 時計の直読み(`hook::current_tick_ms`、`Instant::now`)→ `now_ms` 引数。`state::TickMs` が既にある | 十数ファイル(a1 約14箇所、b 19件、d1 6ファイル、d2 5ファイル16行、f1 約9関数ほか) | 機械的 |
| S-C | HWND → `HwndId`/`WindowId`。`HwndId(usize)` は既にあるが、HWND/生 usize/HwndId の 3 通りが混在 | d1 12ファイル、a2 28項目 | 型付けのみ |
| S-D | 借用ビュー(`ImeControlView<'_>`、`FocusFacts<'a>`)→ 所有スナップショット。`DecisionInputs`(所有・Copy)が既にある | ime_controller の 15 項目、open_chain 3、executor 1 ほか | 中 |
| S-E | グローバル static / atomic の引数化(`HOOK_STATE`、`TSF_OBS`、`OUTPUT_GATE`、`INPUT_DEFER`、`RAW_TSF_LITERAL`、`SEND_HEALTH` 等) | hook.rs 27関数 ほか | 中(ADR-164) |
| S-F | `&mut Runtime`/`with_app` の殻の分離(判断と実行を分ける) | F の約 21 関数(b)+ message_handlers 13 呼び出し | 大 |
| S-G | await をまたぐ失効(世代・fence・ticket)を Event + reducer に | probe_io、conv_actuation、open_chain、ms_ime_ready、focus ticket | 大(INV-45) |
| S-H | `OutputActiveGuard`(RAII)→ `GateAcquire`/`GateRelease` の Cmd | tsf_warmup_coord 3、sender 4、vk_send 2 ほか | 大(ADR-156) |
| S-I | Win32 の呼び出し面 → ポート(S 即時 / B ブロックしうる / A 完了待ち)。タイムアウトを `Result<T, WaitError{TimedOut,PoolFull,WorkerFailed}>` の値に | `use windows::` を持つのは 164 ファイル中 37 | 設計 |
| S-J | 永続化・レジストリ・FS の注入 | classifier(`ImmCapabilityStore` ほか)、autostart、msime_*、app/mod 9関数 | 小〜中 |
| S-K | 可視性 `pub(crate)`→`pub`、`architecture_guard.rs` のファイルパス直書き約 70 件 | b | 機械的だが漏れやすい |

## 4. 移動順序の統合案(依存の少ない順)

0. **挙動を変えない機械作業**: S-A(型の切り出し)、すでに Win トークンの無い gated ファイルの gate 解除(`tsf_gate`、`hwnd_cache`、`tracker`、`gji_observer`、`state/ime_event_log`)。journal は 3 点(`SentKeyEvent`、`current_tick_ms` ×2)直せば 1,682 行と 22 本が Linux で回る。
1. **S-B(時計の注入)**: 全域で一括。
2. **`platform_state.rs` の ungate**(b): 本体 1,813 行は Win32/unsafe/with_app 無し。前提 5 つ(ForegroundScope 型移動、ImeUpdate/ImeObs 移動、journal の SentKeyEvent、`ImeStateHub::new(tick_fn, scope_fn)`、可視性)。これで閉ループハーネスの写し 7 系統のうち 6 が本物の呼び出しになり、c 領域の移動の前提も整う。
3. **S-D(ビューの所有化)** → `ImeController::apply`(`MechanismWriter` を引数化)と `ActuationTarget` の純粋部分。
4. **tsf/ の `LiteralDetector`/`TsfReadinessProbe` をスナップショット引数に**(f1/f2 の山)→ warmup コルーチン群(`GjiWarmupCoro`、`TsfProbeCoro`、`MsImeReadyCoro`)。続いて S-H の gate を値で返す形に。
5. **F の分割**(易しい順): `ir_decide_read_strategy` → `plan_set_open` → `execute_relay`/`drain_deferred` → `ir_apply_drift_correction` → `focus` 系(classify_focus ほか)→ `vk_send`/Output の `Vec<Cmd>` 化。
6. **最後**: `open_chain` の 3 関数(INV-45・BUG-34)、`on_focus_process_changed`(307 行)、`monitor_loop`、`handle_hook_key_event` の defer 判断。journal replay で先に回帰網を張る。
7. **crate の物理分割は最後**: 先に「同一 crate 内で gate を外す」を済ませ、`architecture_guard.rs` のパス直書き(約 70 件)・`layer_boundary_guard` を機械的に付け替えられる状態にしてから切る。

## 5. 移す前に削除を検討する対象(撤去が主目的)

| 対象 | 規模 | 根拠 |
|---|---|---|
| `focus/uia.rs` の UIA 経路 + `handle_wm_focus_kind_update` + 委譲 + bootstrap のワーカー起動 + `WM_FOCUS_KIND_UPDATE` | 約 400 行・COM ワーカー 1 本 | 結果は BUG-12 以来使われていない(d1、削除してよいかは未確認) |
| `tsf/send.rs`(空)と `pub mod send;` | 数行 | f1 |
| `evidence_is_fresh` の 3 重複(probe.rs:639-648, 723-726, 803-805) | 小 | f1 |
| `executor.rs::update_intra_batch_applied`(`apply_result_effective_open` と同じ写像) | 小 | b |
| `ime::ConvAfterOpen` と `state::conv_after_open::ConvAfterOpenId` の重複型 | 小 | a2 |
| `SyncChainWriter::write` と `apply_mechanism` の `decide_attempt` 二重呼び出し | 小 | a2 |
| `MechanismCommand` の `unreachable!` 2 variant(同期用の部分型に分ける) | 小 | a2 |
| `#[cfg_attr(not(windows), allow(dead_code))]` 46 か所/11 ファイル(`state/mod.rs` の 18 モジュール 7,524 行、うち 13 は呼び出し元が全て Windows 側) | 46 か所 | f2。ungate で自然に消える |
| `docs/layer-boundaries.md` B-1 の旧名 `crate::APP`(実体は `RUNTIME`/`with_app*`) | doc | d2 |

## 6. 見つかった不具合候補・危険箇所(未確認、known-bugs への記録は未実施)

- `ctrl_consumed_since_down` のライブ読みが ADR-129 型の窓を持つ(a1。起きる条件は未確認)。
- overflow ラッチ中に Alt を押すと、なりすまし判定が更新されない(a1)。
- `kp_restore_kana_from_half_width` が `MicrosoftIme` を条件にしており INV-45 に反する可能性(c)。
- `dispatch_engine_message` の 16 アームで再入時の扱いが非対称(捨てる 11、再 post 5)(d2)。
- `spawn_local` 内の `with_app` 呼び出し 10 個の戻り値の扱いが不揃い(5 つが `let _ =`、`key_pipeline.rs:1354` だけ fail-open)(c)。
- `get_gui_thread_info_with_timeout` は超過がフォールバック値(`thread_id=0`)に紛れて見えない。`run_with_timeout` の `None` は超過・プール満杯・ワーカー異常終了を区別しない(a1)。
- `HookGuard::drop` は WM_TIMER 内で最大 500ms メインスレッドを止める。`install_hook` のスピン待ちが無制限、`foreground_window_is_elevated` に時間制限なし(a1)。
- `RAW_TSF_LITERAL` はコメント上「Ctrl+C ハンドラ別スレッドから参照」だが、領域内の読み書きは main スレッドのみに見える(d2。他領域の読み手は未確認)。
- 起動時の既定戦略の食い違いの可能性(d2)。

## 7. 事前の見積りの訂正

- 「state/ の約 14,000 行が純関数」→ 実コードは 7,977 行で、状態を持つ reducer も多い(f2)。
- 「platform_state.rs 3,314 行」→ 本体 1,813 行、残りはファイル内テスト(b)。
- 粗い grep で「Win 中心」とした `tsf/probe.rs` は、本体 818 行に `windows::` も `unsafe` も無かった(f1)。
- `crate::APP` はもう存在しない(d2)。

## 8. フック薄型化(畳み込み)構想への含意(a1)

- `HOOK_STATE` は 23 フィールド。**リングに載らない経路(飲み込み、`disable_apps` バイパス、overflow 素通し)も `physical_key_state` を更新する**。畳み込みで再計算するには、これらもリングに載せるか、判定つきで運ぶ必要がある。
- `RawKeyEvent` には拡張ビットと、Alt なりすまし前の vk が無い。
- `journal` の `KeyInput` は処理後の要約で、生入力(拡張ビット・`extra_info`・`press_id`・親指スナップショット・飲み込んだイベント)を持たず、生入力の再生には足りない。
- フックの戻り値(通す/消す)を決める同期判定はフックに残る。

## 9. 未確認(全体)

- ungate 後にコンパイルが通るか(全担当がビルド禁止で未実施)。
- 各 `allow(dead_code)` が今も必要か。
- `tsf/` 側の型が gated のため、`output/` の移設は tsf/ の ungate が前提(d2)。
- `hook::*` や `tsf_obs()` が純粋な atomic 読みかどうか(d1)。
- INV-45 の定義そのものは未読の担当あり(a2)。
