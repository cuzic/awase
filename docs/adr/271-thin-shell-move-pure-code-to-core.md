---
id: ADR-271
title: |-
  shell(awase-windows)を薄くする: 同じ crate の中で gate を外すのを先にし、tsf/warmup の FSM から R7 として再開する
status: |-
  段階 A は実装済み(2026-10-20、`refactor/thin-shell` ブランチ。PR #696・#699・#700・#701)。段階 B(R7 の再開)と段階 C は未着手。実装は未着手
related_adr:
  - "ADR-019"
  - "ADR-156"
  - "ADR-229"
---

# ADR-271: shell を薄くする

## 背景

ADR-229 の F1〜F6・crate の物理分割 E2(#574)は済み、F7 は PR-A(#552)だけ(PR-B・PR-C は未着手)。
タスク表(`docs/tasks/fcis-layering-tasks-2026-10-06.md`)は「着手できる FCIS の分割タスクは無い」とし、
次の対象は §4「その後・保留」の R7(:144、warmup コルーチン群)で「条件待ち」としている。
shell は約 69,100 行、core は約 53,100 行(2026-10-20、`find … -name '*.rs' | xargs wc -l`)。

利益として挙げるのは次の 3 つ。**回帰網に入るかどうかではない**(`windows-test` ジョブ、ci.yml:404 が
`#[cfg(windows)]` 配下のテストも PR ごとに実行している)。

1. cargo-mutants の対象になる(`.cargo/mutants-awase-windows.toml` は `#[cfg(windows)]` 配下を偽の MISSED
   になるため `examine_globs` から外している。現在 6 ファイル)。
2. Linux の `test` ジョブと pre-push で速く走る。
3. proptest・closed_loop・journal replay と同じ binary で偽物と組み合わせられる。

指標は「Linux の lib テストバイナリに含まれ、かつ `examine_globs` に入れられる関数・テストの数」と、
shell にある判断関数の数。行数は指標にしない(F5d の前例)。

測定(Opus レビューが gate をたどった値): shell の Windows 専用テストは約 248 本。内訳は tsf 83
(うち `tsf/warmup/` 48)、output 66、runtime 39、focus 16、observer 11、app 8。**本体は runtime ではなく tsf/output**。

## 測定の位置づけ

`scripts/shell_purity.py` の試作は子モジュール宣言の伝播で `gji_fsm` を BLOCKED と誤判定するなど根拠にならないため、**この PR には入れない**。
`super::`・`use crate::{a, b}`・lib 直下の項目への依存を見ず、`mod.rs` の子の扱いで LEAF の合計が
約 3,100〜10,100 行に動く(走査の細部で 3 倍変わる)。判定の拠り所は次の既存の関門:

- Tier-1: gate を外して Linux でビルドが通るか(コンパイラ)。
- Tier-2(crate を移せるか): `layer_boundary_guard.rs` の `core_violations` と `CORE_CANDIDATE_FILES`。
  static・FS 書き込み・Win32 を持つものは core に入れない(F-D3)。

## 決定

1. **対象外(決定済みの再確認)**: `state/platform_state.rs`・`sync_actuation.rs`・`ime_decision_view.rs` は
   移さない。`state/mod.rs` 冒頭と タスク表 :337 の所有者の決定(記録系の `pub(crate)`〈INV-A97-1〉を
   コンパイラが強制し続ける)による。E2(#574)で殻に残したもので、ADR-229 D4 そのものの規定ではない。
   覆す提案は本 ADR に含めない。
2. **対象外**: `scancode_*`(約 4.8k 行)。IME の判断ではなく設定 UI のモデルで、awase-windows 本体は使っていない
   (消費者は `awase-settings/src/scancode_map_admin.rs`)。移し先の `awase-settings` は lib target が無く
   Linux のテストが減り、`awase-vkmap` は外部リポジトリと共有で `unsafe_code = "deny"`。やるなら
   lib target を持つ専用 crate(`publish = false`)への別件で、`scancode-vm-verify.yml` の paths と
   `tests/scancode_vm_scenarios.rs` の置き場所を同じ PR で直す。
3. **対象外(段階 C で crate を移す対象から外す意味。段階 A の Tier-1 の ungate は妨げない)**: static・FS 書き込みを持つ `input_defer`・`send_health`・`focus_resync`・`probe_actuation_fence`・
   `conv_mutation`・`app::logging`(Tier-2 違反。F-D3 は「グローバルは shell が読んでスナップショットで渡す」)。
   すでに Linux でテストが走る `gji_fsm`・`tsf_gate` は行数のためだけには移さない(ただし `warmup_strategy` が `gji_fsm` に依存するため、段階 C で warmup を移すなら `gji_fsm` も一緒に移る)。
4. **段階 A(Tier-1、同じ crate の中。cfg で分けるだけ)**: `tsf/mod.rs` の `#[cfg(windows)] pub(crate) mod warmup;`
   を外す。ただし gate を外すだけでは成立しない(Opus 2 回目 N1):
   - **個別に gate を残す**: `cold_warmup.rs`(`with_app`・`ime`・`imm`)と `gji_warmup_coro.rs`
     (`win32_async::spawn_local`。この crate は Windows でしか依存に入らない)。
   - **一緒に ungate が要る**: `tsf/probe`・`tsf/observer`・`probe_bridge`・`input_defer`・`ime_mode_fsm`、
     `tsf/output.rs` から切り出す `ColdReason`。さらに 次の 5 関数は `#[cfg(windows)]` 版と非 Windows 版の 2 本に分ける:
     `hook::current_tick_ms`・`win32::post_to_main_thread`・`ime_diagnostic::log_composition_probe`・
     `observer::mozc_shm::read_compose`・`ime::window_thread_responds`。cfg の分割は要らないもの:
     `imm::IME_CMODE_*`(定数を移すだけ)、`output::resolve_ascii_to_vk`(`crate::vk::ascii_to_vk` に直接置き換える)。
   - **やらないこと**: 関数ポインタ化(F-D3 が採らない形)、`OutputActiveGuard` を値で返す action 化
     (段階 B (ii) の寿命変更を順序テストより前に持ち込む)。段階 A は cfg の振り分けだけにとどめる。
   - **時計**: 非 Windows 版の `current_tick_ms` は**起動からの単調時計**にする。テストが
     `current_tick_ms().saturating_sub(50)` で「epoch より前」の値を作るため、0 に張り付くとテストが壊れる/フレークする。
   - **`allow(dead_code)`**: ungate で Linux 未使用の項目が出る。ADR-229 タスク表 §0 の 5 の形
     (モジュール単位の `#[cfg_attr(not(windows), allow(dead_code))]` 1 行まで)を認める。それを超えたら止めて報告する。
   - 効果の見積り: Linux と mutants に入るテストは 48 本ではなく**約 80 本**(連れて ungate されるものを含む)。
5. **段階 B(R7 の再開、段階 A の後)**: 前提条件を先頭に置く。
   (i) baseline を `SendInput` の前に取る順序(BUG-027/029/030/033、ADR-079)の固定テストを**先に**入れる。
   (ii) `OutputActiveGuard` は F-D4 どおり `wants_output_gate: bool` を値で返し、trait にしない。trait にするなら
   F-D1 の閉じた列挙の改訂と、タスク表 :33 の再着手条件(解放漏れを型で防ぐ手段・ゲート故障の実機再現)が先。
   (iii) `probe_fsm` が 6 回直接呼ぶ `log_composition_probe` は、診断要求の action として返し殻が実行する。
   (iv) `LiteralDetector` のスナップショット化は判定が最大 10ms 古くなる(epoch fence は 20ms)ことを許容するか、
   (i) のテストで確かめてから。
6. **段階 C(crate を移す、任意)**: 段階 B の後、Tier-2 を満たしたファイルだけ core へ。次を守る。
   - **相対パスを保つ**(改名しない)。`EXTRA_SRC_CRATES` が同じ相対パスを吸収するため、`architecture_guard`・
     `layer_boundary_guard`・pre-push の正規表現の付け替えは 0。改名すると `RELOCATED`・`DECISION3_FILES`(:6644)の追記が要る。
   - core の公開 API を最小にする(FSM の型と `step` だけ `pub`、内部は core 内 `pub(crate)`)。殻のテストが内部を
     触る箇所は core の `testing` feature へ。`pub(crate)`→`pub` は `gji_fsm` 27・`probe_fsm` 27・`tsf_warmup_coord` 42 行(`grep -c`、`pub(crate) mod`/`use` を含む)。
   - 偽物は ADR-229 D5 の契約テストで本物に拘束し、Windows のジョブで同じテストを本物に流す(件数だけでは
     Drop の解放を落とした偽物を見逃す)。
   - core の `Cargo.toml` に `timed-fsm` を足す。
   - 共通の後処理(タスク表 §0 の 4): `examine_globs`、`.github/workflows` の paths トリガー、
     fix-requires-evidence の表、`CLAUDE.md` の core の列挙。着手前の確認(§0 の 3、PR #493 の教訓):
     一度もホストでコンパイルされていない gated コードをグレップする。
   - rename だけのコミットを分け、類似度 100% を保つ(`--follow`・`blame -M` のため)。merge commit で入れる
     (squash すると潰れる)。`git log -S` はパスに依存しないので影響しない。
7. **ガード「shell に純粋なファイルを足さない」**は入れない(偽陽性の見積りが先で、現状の測定では根拠が足りない)。

## 却下した案

- 行数の目標。段階 1(純粋な葉を丸ごと移す)も、Linux で増えるテストが約 8 本で候補の多くが Tier-2 違反のため削除した。
- trait ポートの新設(`OutputActiveGuard`・`LiteralDetector` は具体型。F-D4・F-D1 と衝突)。
- 測定スクリプトを根拠にした優先順位づけ。

## 検証

各 PR: `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows -p awase-windows-core`、
`cargo nextest run --workspace --lib`、`architecture_guard`・`layer_boundary_guard`。ローカルではビルドせず
GitHub Actions に任せる。段階 A は「Linux の `cargo nextest list` に `tsf::warmup::` の 48 本に加え、連れて ungate する `tsf::probe` 16・`tsf::observer` 7・`tsf::ime_mode_fsm` 5・`input_defer` 4 が出る(1 つでも漏れたら検証は通らない)」ことを PR 本文に書く。

## 未決

- 段階 B の (iv) の許容可否(実測が要る)。
- 段階 C を実際にやる価値(増えるのは行数の差だけかもしれない。段階 A の結果で判断)。

## 段階 A の結果(2026-10-20)

`refactor/thin-shell` に PR #696(A-1)・#699(A-2a)・#700(A-2b)・#701(A-2c)をマージした。
Linux の `cargo test -p awase-windows --lib` に加わったテストは次のとおり(各 PR の本文の実測):

| PR | Linux の lib テスト | 加わったもの |
|---|---|---|
| #696 A-1 | +5 | `ime_mode_fsm`(時計と `IME_CMODE_*` を gate なしの場所へ) |
| #699 A-2a | +23 (77→100 の `tsf::`) | `tsf/probe`(16)・`tsf/observer`(7)。`ColdReason` は `tsf/cold_reason.rs` へ |
| #700 A-2b | +4 (467→471) | `input_defer`(`post_drain_output_queue` だけ cfg) |
| #701 A-2c | +47 (471→518) | `tsf/warmup/` の FSM 群。`cold_warmup`・`gji_warmup_coro` は gate を残した |

ADR の見積り(約 80 本)に対し、A-1〜A-2c の合計は約 79 本。`cold_warmup`・`gji_warmup_coro` は
`with_app`・`ime`・`imm`・`win32_async` に依存するため、決定 4 のとおり Windows 専用のまま。

**所見**
- 壁時計に依存する `tsf::probe` のテストが、ホストの負荷が高いとき(load average 25 / 8 コア)に
  lib 全体の 15 回中 5 回落ちた(60ms の sleep が 354ms になる等)。単体では 30 回中 0 回。
  GitHub のランナーでは #699・#701 の `test` ジョブは通った。負荷による停止と見ているが、状態の漏れが
  無いと確かめたのは `TSF_OBS` への store の所在(Windows 専用の 2 ファイルのみ)を grep しただけ。
- `.cargo/mutants-awase-windows.toml` の `examine_globs` に 12 ファイルを足した。mutants の実行は未確認。
