---
title: FCIS による層の引き直し — 実装タスク表(ADR-229)
status: FCIS 設計は Opus round3 で収束(2026-10-06)。T1〜T7 は PR #492〜#495 でマージ済み(`42bc25ea`)。所有者判断済み(2026-10-06): ADR-224 改訂、S2 実施、1 turn 入口集約とフックの薄型化は将来の目標。P0・P1・RW・S2 は並行して実装中。P2〜P5・F1〜F6 は未着手
created: 2026-10-06
related_adr: ["ADR-229", "ADR-224", "ADR-163", "ADR-164", "ADR-180", "ADR-156", "ADR-129"]
---

# FCIS による層の引き直し — 実装タスク表

設計は [ADR-229](../adr/229-os-independent-core-and-dumb-windows-executor.md) の「FCIS による改訂」節。根拠は [層の棚卸し](layering-inventory-2026-10-06/README.md)、Opus のレビュー記録(`docs/adr/review/229-opus-review-round1.md`・`229-opus-fcis-round1.md`・`229-opus-fcis-round2.md`)。

## 0. 全タスク共通のルール

1. **ローカルでビルド・テストをしない**(ユーザーの指示。ディスクが満杯になる)。使ってよいのは `cargo fmt -p awase-windows`(rustfmt のみ)と git・gh・grep・読み取り。成否は CI で判定する。
2. **1 タスク = 1 PR**。PR 先は `develop`。作業は `git worktree add ~/rust-nicola-worktrees/<名前> -b <ブランチ> origin/develop` の中で行う(`worktree-per-session.md`)。PR の本文は「概要・壊れるガード・成否の判定・取りやめ条件・補足」。本文の更新は `gh api -X PATCH repos/cuzic/awase/pulls/<N> -F body=@file`(`gh pr edit --body` は GraphQL エラーになる)。
3. **着手前の確認(PR #493 の教訓)**: テストが呼ぶ関数・型・定数・macro が gated 側にないかを、全て grep で確認する。ungate・テストの移動・型の移動では、移した先で参照が解決できるか、呼び出し元がどのモジュールかを先に洗う。
4. **共通の後処理**: `fix-requires-evidence.md` の表、`.githooks/pre-push` の正規表現、`.cargo/mutants-awase-windows.toml` の `examine_globs`、`decision3_…`(`architecture_guard.rs`)の instrument 一覧、「Linux で実行されないから」を理由にしたガードのコメントと件数を、見直す。
5. **`allow(dead_code)` を増やさない**。Linux で未使用の `pub(crate)` 項目: テストも使うなら `#[cfg(any(windows, test))]`、使わないなら `#[cfg(windows)]`。外から到達できる `pub` 項目には何も付けない(PR #494 の S4)。3 か所を超えて必要になったら止めて報告する。
6. **CI の見方**(ローカルで見ない代わりに、ログで確かめる): `gh pr checks <N>` で全ジョブ。Linux の `test` ジョブのログ(`gh api repos/cuzic/awase/actions/jobs/<job id>/logs`。job id は `gh pr checks` の URL の末尾)で、①追加・移動したテスト名が列挙されて PASS、②`^warning` と `never used`/`never read` が 0 件。`windows-build`(clippy `-D warnings` と lib テスト)と `windows-cross-check` が green。run 全体が完了するまで `gh run view --log` は使えないが、`gh api .../jobs/<id>/logs` は完了済みのジョブなら読める。
7. 実装後は Opus の PR レビュー(読み取り専用、形態 b: 指摘 → 修正 → 同じレビュアーに再確認)を受けてからマージする。マージは所有者の許可を得てから。
8. **1 つの PR で ADR・ガード・実装の論点を混ぜない**(特に `CORE_MODULES` のガードは P0 として単独で入れる)。

## 1. 済(Tier-1 の最初の弾。PR #492〜#495 としてマージ済み、`42bc25ea`、2026-10-06)

| ID | 内容 | PR | 新たに Linux で回るテスト |
|---|---|---|---:|
| T2 | `tsf/tsf_gate.rs` の gate 解除(R1) | #492 | 19 |
| T3 | `state/ime_event_log.rs` の gate 解除(R1) | #492 | 5 |
| T5 | 空の `tsf/send.rs` の削除 | #492 | 0 |
| T1 | `plan_tests`(30 本)を `state/physical_disposition.rs` へ移す。`suppress_reason` も移動(R3) | #493 | 30 |
| T7 | `focus/hwnd_cache.rs` に `TickMs` を渡して gate を外す(R2)。6 本を追加 | #494 | 6(新規) |
| T4 | `journal.rs` の gate 解除。`SentKeyEvent` を journal へ移す(R4) | #495 | 22 |

ADR-229 の gate 一覧からは journal・ime_event_log・tsf_gate・hwnd_cache が外れた。`README`(棚卸し)の数字は変えない(棚卸し時点の記録)。

## 2. 第 1 弾の次(Tier-1 の完成と Tier-2 の入口)

| ID | 単位 | 内容 | 前提 | 成否の判定 | 取りやめ条件 |
|---|---|---|---|---|---|
| **P0** | Tier-2 のガード | `architecture_guard.rs` に定数 `CORE_MODULES`(初期 45 ファイル、付録 A)とテスト 1 本を足す。各ファイルの本番コード(`#[cfg(test)] mod tests` より前、コメント行を除く)が、Tier-2 の 4 規則(壁時計 `Instant::now()`/`SystemTime::now()`、`static`(不変も含む)と `thread_local!`、ファイル内の `#[cfg(windows)]` 項目(`mod` 宣言と `#[cfg(any(windows, test))]` は対象外)、FS/環境変数/レジストリ)に違反しないことを確かめる。実装上の注意は §2 の直後「P0 の実装上の注意」 | なし(単独で先に入れる。#492〜#495 のマージ後に再実測してから載せる) | Linux の `test`(`architecture_guard`)が green。`CORE_MODULES` の全ファイルで違反 0 | 偽陽性が多く、規則の書き分けが複雑になる(例: 5 規則以上、または許可リストが必要になる)。そのときは止めて報告 |
| **P1** | 型移動(R4・R5 の準備) | `ForegroundScope`(`win32.rs:46-60`)を ungated(例: `state/foreground_scope.rs`)へ、`ImeUpdate`/`ImeObs`(`observer/ime_observer.rs:22-45`)を ungated(例: `state/ime_update.rs`)へ移し、元の場所で `pub use` する | **#495 のマージ後**(`win32.rs` が重なる) | Linux と Windows でビルドが通る。新しい警告 0。呼び出し元の変更 0。test・windows-cross-check・windows-build が green | 型に gated な依存が見つかる(`ImeUpdate` は `InputModeState` などだけのはずだが**未確認**) |
| **P2** | 核と殻の分割(まだ gated、R5) | `platform_state.rs` の **13 メソッド**を `_in_scope(scope)` 版(5 つは既存)と殻に分ける。内訳: 10 メソッド(`arm_mode_key_pass_mark`・`mode_key_pass_expiry_wait_ms`・`mode_key_pass_window_remaining_ms`・`mode_key_pass_mark_live`・`expire_mode_key_pass_mark`・`arm_external_change_watch`・`external_change_watch_remaining_ms`・`follow_external_change`・`align_after_expired_mode_key_pass`・`invalidate_intents_if_mode_key_pass_live`)+ 連鎖の 3 メソッド(`note_awase_write_for_mode_key_pass` を呼ぶ core の `record_optimistic`・`record_confirmed`、それを呼ぶ `record_ime_apply_result`)。殻は **`platform_state.rs` の子モジュール**(`#[cfg(windows)] mod shell;`、`state/platform_state/shell.rs`)の `impl ImeStateHub` に置く。殻は「`foreground_scope()` を読む → `_in_scope` を呼ぶ」の 1 行だけ。殻のメソッド名は今と同じで、runtime/ の呼び出し元の差分は 0。**`architecture_guard.rs:1620` の `RECORDERS`(`.record_confirmed(`=5・`.record_optimistic(`=1。INV-A97-1/BUG-69)を、同じ PR で次のように直す: `state/platform_state/shell.rs` を走査から除外し(殻は委譲するだけ。同じ理由で `:3767-3775` の呼び出し元の固定からも、殻のファイルは対象外にする方針とする)、`.record_confirmed(` と `.record_confirmed_in_scope(` の合計を 5、`.record_optimistic(` と `.record_optimistic_in_scope(` の合計を 1 に保つ(殻の本体の呼び出しを数えて 6 に書き換えると、「記録の呼び出し元が 1 つ増えた」ことと区別できなくなる)** | P1。P3 と同じファイルなので **P2 → P3 の順(直列)** | windows-build の lib テスト(`platform_state` の 48 本)と `architecture_guard`(`:3767-3775` の呼び出し元の固定、`:1620` の `RECORDERS`)が green。runtime/ の差分 0 | 殻が 1 行で書けないメソッドが出る(スコープを 2 回読むなど) |
| **P3** | 時計の構築の分割とテストの付け替え | `ImeStateHub::new()` を `#[cfg(windows)]` にし(本体は `HubClock::wall(crate::hook::current_tick_ms)` のまま、`platform_state.rs` 内)、ungated な `with_clock(clock: HubClock)` を足す。テスト用に `PlatformState::for_test(clock: HubClock)` を用意し、`platform_state.rs` のテストの `PlatformState::new()` **36 か所**を付け替える(うち、`effective_open()` を**呼ぶ** 19 か所は、構築の 36 か所とは別の集合で、時間が意味を持つので `HubClock::manual(<Windows と同じ大きさの基準 tick>)`、それ以外は同じヘルパー)。**Linux 用の壁時計を `cfg` で切り替える案は採らない**(時間軸の意味が変わる) | P2 | windows-build で `platform_state` の全テスト(48 本)が従来どおり pass(まだ gated)。`architecture_guard.rs:1382`(`HubClock::wall(crate::hook::current_tick_ms)` の件数固定)が書き換えなしで green | `new()` を使う ungated のコードが見つかる(`tests/support/harness.rs` は `ImeStateHub` を使っておらず影響なしと確認済み) |
| **P4** | `platform_state` の ungate(段階 2 本体) | `state/mod.rs` の `#[cfg(windows)] pub mod platform_state;` と `#[cfg(windows)] pub use platform_state::PlatformState;`(develop では `:187-190`)の gate **だけ**を外す。**`ime_decision_view` は gated のまま**(`tsf::observer` の `TsfObservations`・`ActiveImeKind`・`candidate_was_seen()` に依存し、`platform_state` は使っていない。借用ビューの所有化の対象で、P4 には含めない。`ime_event_log` は #492 で済み)。テストの書き換え: 殻の名前のメソッドを呼ぶ **16 か所**(`follow_external_change` 9、`arm_external_change_watch` 3、`record_ime_apply_result` 3、`record_confirmed` 1)と、`crate::win32::foreground_scope()` を直接呼ぶ **9 か所**を、`_in_scope` + `test_foreground_scope()`(`:2561`、既存)へ。 | #494・#495・P1〜P3 のマージ | Linux で `state::platform_state::tests::*`(48 本の見込み)が回る。Linux のビルドで新しい警告が無い(`allow` を増やさない) | `allow` を足さないと消えない警告が 3 件を超える。**Linux で結果が変わるテストが出たら止めて、時間軸の違い(`HubClock::manual` の基準 tick)か本物のずれかを切り分ける**(切り分けは、同じテストを基準 tick を変えて 2 通り流して見る。本物のずれなら BUG として扱う) |
| **P5** | ハーネスの写しの置き換え(1 系統ずつ) | `tests/support/harness.rs` の手書きの写し 7 系統のうち、`platform_state` の ungate で本物の呼び出しになる 5 系統(`apply_key_effect_prediction`・`effective_open_at`・`warrant_context`/`issue_actuation_order`・`record_explicit_intent`/`write_*`・`arm/follow_external_change`)を、1 系統ずつ本物の `ImeStateHub` の呼び出しに置き換える。最初は `effective_open_at`(`HubClock::Manual` で仮想時計とそろえる)。ungate 自体では写しは減らない。残る 2 系統(`kp_stage_key_effect_track`/`kp_predict_key_effect`、`ir_apply_drift_correction` の前半)は runtime/ にあり F4 で扱う | P4 | `closed_loop_scenarios` の全シナリオの結果が不変。写しが 7 → 6 | 本物を呼ぶと結果が変わる。それは**写しのずれの発見**なので、ADR-224 の着手条件を満たす実例として `docs/known-bugs/` に記録して止める |
| **RW** | ReplayWriter(並行する別の線) | テストだけで本番の変更 0。`MechanismWriter`/`AsyncMechanismWriter` の実装 1 つで、記録済みの `(mechanism, outcome)` を返し、`run_chain(_async)` の走査と打ち切り位置を既存コーパス(`tests/journals/actuation_decision/`)で再生する(F-D1 の handler 例外の実証)。任意で `ActuationDecisionRecord` に `gate` を明示記録する(旧コーパスは逆算のフォールバックを残す) | なし(P1〜P3 とファイルが重ならない) | 既存コーパスの 37 レコードで、走査の再生が全件一致。`replay_record` の逆算・skip が新形式のレコードでは通らない(再生できない分岐 4 → 1) | `ReplayWriter` のために本番の `Actuation`/`ActuationOrder` に test 専用のコンストラクタが要り、warrant の型安全(ADR-090)を弱める。旧コーパスで 1 件でも不一致(それは本番の挙動差の発見なので BUG として扱う) |

### P0 の実装上の注意(Opus round3 S13)

1. **テキストの切り方**: 「`#[cfg(test)] mod tests` より前、コメント行を除く」だけでは不十分。行末のコメント(`foo(); // Instant::now()`)、文字列リテラル(ログの文言に `std::fs` など)、`mod tests` 以外の名前のテストモジュール(PR #493 の `plan_shell_tests`)、`mod tests` より前の `#[cfg(test)] impl`/`#[cfg(test)] fn`(例: `platform_state.rs:1610` の `#[cfg(test)] impl ImeStateHub`)を見落とす。**`layer_boundary_guard.rs` の `code_lines`/`test_block_mask`(`:47-112`。コメントを除き、`#[cfg(test)]` が付いた item の本体を丸ごと覆う)を使う**のが最も確実。P0 のテストを `layer_boundary_guard.rs` 側に置くか、同じヘルパーを移す。
2. **`#[cfg(windows)]` の検出**: 完全一致だけだと `#[cfg(all(windows, …))]`・`#[cfg(target_os = "windows")]`・`#[cfg_attr(windows, …)]` を見逃す。許可する 3 つの形(セミコロンで終わる `mod <名前>;` 宣言の直前(**インラインの `mod x { … }` は不可**。PR #498 レビュー S15)、`any(windows, test)`、`cfg_attr(not(windows), allow(...))`(S16。`allow(` に限る))を明示的に除いた上で、`cfg` の中に `windows` の語があれば違反とする。
3. **壁時計**: `Instant::now()`/`SystemTime::now()` に加えて、`quanta::Clock::new()`/`quanta::Instant::now()`(journal が使う)、`timed_fsm` の実時計も対象にする。
4. **FS/環境変数**: `std::fs`・`fs::` に加えて `File::open`、`Path::exists`/`metadata`。`env!`/`option_env!`/`include_str!` はコンパイル時の展開なので許可する。
5. **`static`**: 不変も含めて一律に違反とする(不変の表は `const` にするか、そのファイルを `CORE_MODULES` に載せない)。

## 3. 第 3 弾(F の分割、Tier-2)

**共通**: サンドイッチ(`observe` → core の `decide` → `execute`)。分けたファイルは P0 の `CORE_MODULES` に足す(同じ PR)。journal replay で、元の判断の入力が復元できるかを確かめる。いずれも、再発ファミリーのガード(`fix-requires-evidence.md` の表)と `decision3_…` の instrument 一覧を見直す。

| ID | 対象 | 注意 |
|---|---|---|
| F1 | `ir_decide_read_strategy`(36 行、`runtime/ime_refresh.rs`) | 最も易しい |
| F2 | `dispatch_ime_set_open` の `plan_set_open`(`runtime/executor.rs`) | actuation 合流点のファミリー(ADR-119)。gate を再検出する 3 関数の設計(ADR-180)に触れない |
| F3 | `execute_relay`/`drain_deferred` の計画/ガード状態機械 | **defer/replay キュー(ADR-156)。defer 側と drain 側の 2 窓口を、同じ PR で必ず対にする** |
| F4 | `ir_apply_drift_correction` の `DriftPlan`。ハーネスの写しの残り 2 系統(`kp_stage_key_effect_track`/`kp_predict_key_effect`、`ir_apply_drift_correction` の前半)を本物に | 写し 7 → 1 への道 |
| F5 | focus 系(`classify_focus`、`msaa_classify`、`resolve_focus_kind`、`learn_imm_capability_on_focus`)。「O が facts、core が判断」 | 条件付きの段階的な読み取りは F-D1 の handler 例外(MSAA までの同期の段階)。UIA は A 種の Cmd/Event。UIA 経路の削除(約 400 行)は別判断 |
| F6 | Output(`vk_send` F 623 行、`output/mod` 576 行)の `Vec<Cmd>` 化 | `Output` の状態を `OutputState` へ(`RAW_TSF_LITERAL`、`OutputActiveGuard`)。`OutputActiveGuard` は `GateAcquire`/`GateRelease` を対に(ADR-156) |

## 4. その後・保留

- **R7 コルーチン**: `tsf/probe.rs` の `LiteralDetector`/`TsfReadinessProbe` をスナップショット引数に → warmup コルーチン群(`tsf/warmup/`、F は `run_start` の 84 行だけ)。**baseline を `SendInput` の前に取る順序**(BUG-027/029/030/033、ADR-079)をテストで固定してから。スナップショット化で判定が最大 10ms 古くなる(epoch fence は 20ms)。
- **最後**: `open_chain` の 3 関数(INV-45・BUG-34・ADR-119/180。**書き換えない**。F-D1 の例外 1 と F-D5-4 の範囲)、`on_focus_process_changed`(307 行)、`monitor_loop`(202 行)、`handle_hook_key_event` の defer 判断。journal replay で回帰網を先に張る。
- **フックの薄型化(畳み込み)**: 前提確認(リングに載らない経路が `physical_key_state` を更新する、`RawKeyEvent` に拡張ビットと Alt なりすまし前の vk が無い、`KeyInput` が生入力を持たない、`held_modifiers` の鮮度要件)のあとに別段階で。
- **crate の物理分割**: 最後。`architecture_guard.rs` がファイルパスの文字列リテラルを異なりで 76 種持つので、機械的に付け替えられる状態にしてから。
- **所有者の判断(2026-10-06 に回答済み)**: ① ADR-224 の決定を改訂し、段階 2 を核と殻の分割で進める(承認。ADR-224 に追記済み)、② S2(`state/physical_disposition.rs` を `.githooks/pre-push`・`fix-requires-evidence.md` の表・`.cargo/mutants-awase-windows.toml` に足す)を別 PR で実施する(承認。実装中)、③ 1 turn の入口集約を将来の目標にする(承認)、④ フックの薄型化を将来の目標にする(承認。着手は前提確認のあと)。

## 5. 失われやすい注意(Opus round2 の §4 から)

1. Tier-1(ungate、コンパイラが守る)と Tier-2(`CORE_MODULES`、テキスト走査)は別の作業で、別のガード。「core」の語は Tier を添えて使う。
2. handler 例外は閉じた列挙(`run_chain(_async)` と、条件付きの同期の効果を個別に列挙)。追加は ADR の改訂。
3. F-D5 の 4 番目(非同期の handler は await の後に観測し直す)を、`open_chain` を書き換えない理由とともに残す。
4. P2 の殻は 13 個で、`RECORDERS` の数え方を同じ PR で直す。「11 個・差分 0」と書き写さない。
5. P3 はテストの構築の付け替え(36 か所、`effective_open()` を呼ぶ時間が意味を持つ 19 か所は `Manual`)を含む。P4 は殻の名前 16 か所・`foreground_scope()` 直接 9 か所のテストの書き換えを含む。
6. 順序: P1 は #495 のマージ後。P2 → P3 は直列。RW は並行可。
7. `HubClock` は Tier-2 の外(時計の実装そのもの)。`hub_clock.rs` 自身は `CORE_MODULES` に入らない。
8. `CORE_MODULES` は「違反 0 のファイルだけ」で、許可リストは持たない。違反のある 8 ファイルの内訳(付録 B)は「殻へ出す候補」。
9. ADR-224 の改訂は所有者の判断で、ADR-229 の改訂とは別に記録する。
10. 共通の後処理と、着手前の「テストが呼ぶ識別子は gated 側にないか」を、各 PR の成否の判定に入れる。
11. `allow(dead_code)` を増やさない。
12. 成否は CI のどのジョブのどのログで見るかを、PR の本文に具体的に書く(§0-6)。

## 付録 A: `CORE_MODULES` の初期候補 45 ファイル(`state/`、Opus round2、origin/develop d0d42be6 時点。#492〜#495 のマージ後に再実測してから確定する)

`actuation_chain`、`actuation_decision_record`、`alt_impersonation`、`app_ime_policy`、`app_suppression`、`belief`、`conv_after_open`、`conv_classify`、`conv_mode`、`drift_correction`、`eisu_recovery`、`event_origin`、`evidence`、`explicit_press`、`external_change_watch`、`focus_probe_plan`、`focus_resync_policy`、`force_guard`、`generation`、`gji_direct_mechanism`、`half_width_alnum`、`hook_state`、`hook_watchdog`、`ime_actuation`、`ime_actuation_decision`、`ime_kind`、`imm_evidence`、`injection_mode`、`input_barrier`、`intent_store`、`key_effect_table`、`key_sequence_policy`、`keymap_initial_hypothesis`、`keymap_latch`、`layout_language`、`mode_key_pass`、`observation_store`、`open_warrant`、`physical_disposition`、`post_bypass`、`press_ledger`、`scoped_latch`、`state_dependent_key_warning`、`transition`、`win_key_guard`(いずれも `crates/awase-windows/src/state/<名前>.rs`)。

### 付録 A-2: #492〜#495 のマージ後の予想(P0 の実装者向け。マージ後に再実測してから確定する)

P0 の初期対象は `state/` に限る(`state/` 以外を対象にするなら、そのとき「`state/` 以外も対象にする」と書く)。

| ファイル | マージ後の予想 | 扱い |
|---|---|---|
| `state/physical_disposition.rs` | #493 で `#[cfg(any(windows, test))]` が 1 件入る | 例外なので違反 0 のまま、付録 A に残る |
| `state/ime_event_log.rs` | #492 で ungated になるが `Instant::now()`(`:41`)がある | 違反あり(付録 B 側)。直すなら `record_at` を使う側へ |
| `focus/hwnd_cache.rs`・`tsf/tsf_gate.rs` | #494・#492 で ungated。違反 0 の見込み | `state/` の外なので初期対象に載せない |
| `journal.rs` | #495 で ungated。`#[cfg(windows)]` の dump 関数 2 件と `quanta::Clock` | 違反あり。記録係なので core に入れる必要は薄い |

## 付録 B: 違反のある 8 ファイル(最初は載せない。直したら足す)

| ファイル | 違反 | 殻へ出す候補 |
|---|---|---|
| `state/hub_clock.rs` | `Instant::now()` 2 件(`:41`、`:51`) | 時計の実装そのもの。**恒久的に Tier-2 の外** |
| `state/ime_event.rs` | `#[cfg(windows)] impl HwndId`・`impl From<HWND> for HwndId`(`:36`、`:46`) | HWND との変換は殻(拡張 trait 化) |
| `state/ime_model.rs` | `Instant::now()`(`:435` の `effective_open()`、`:504`) | `effective_open_at(now)` を呼ぶ側へ |
| `state/ime_profile_driver.rs` | `static` 3 つ(`IMM_CROSS_DRIVER`・`IMM32_UNAVAILABLE_DRIVER`・`TSF_NATIVE_DRIVER`、`:186-188`。不変だが `static` は一律に違反とする。可変か不変かをテキスト走査で判定するのは難しい) | ゼロサイズの構造体なので `const` による `&'static dyn` の昇格で置き換えられる見込み(**未確認**)。直すまで `CORE_MODULES` に載せない |
| `state/key_effect_predictor.rs` | `#[cfg(windows)]` の `get_gji`/`get_native`(`:570`、`:580`)。FS/レジストリを読む | 殻へ |
| `state/key_effect_runtime.rs` | `#[cfg(windows)]`(`:377`、`:385`、`:393`、`:408`、`:440`、`:816`)。学習済み表のパス解決・`fs::metadata`・読み込み | 殻へ(FS) |
| `state/probe_admission.rs` | 可変の `static REJECTION_COUNTERS`(`:71`)、`#[cfg(windows)]` の関数(`:325`) | カウンタは shell へ |
| `state/mod.rs` | `#[cfg(windows)]` 8 件。`mod` 宣言のほか、`pub(crate) use conv_mode::{…}`(`:32-35`)・`pub use platform_state::PlatformState`(`:189-190`)・`pub(crate) use ime_decision_view::{…}`(`:194-195`)など `use` の再公開を含む | **恒久的に `CORE_MODULES` に載せない**(モジュール宣言の集約ファイル。例外を `use` まで広げると、core のファイルに gated な再公開を置く抜け道になる) |
