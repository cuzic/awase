---
title: FCIS による層の引き直し — 実装タスク表(ADR-229)
status: FCIS 設計は Opus round3 で収束(2026-10-06)。マージ済み: T1〜T7(#492〜#495)、P0(#498)・P1(#496)・RW(#499)・S2(#497)・P2(#500)・P2 のガード(#502)・P3(#503)・W-c(#505、#508)・P4a(#507)・F1(#506)・`with_clock` のガード(#509)・F2(#504)・**P4(#510。`platform_state` が Linux でテストできる。48 本)**。実装中・レビュー中: F4(#514)・F5a(#512)・F1 の追加(#511)・`with_clock` のガードの追加(#513)・死んだ variant の削除(#515)・F3・P5 の最初の 1 系統・W-c の S22。BUG-098 は案 D で記録の訂正のみ(コードは変えない)
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
5. **`allow(dead_code)` を増やさない**。Linux で未使用の `pub(crate)` 項目: テストも使うなら `#[cfg(any(windows, test))]`、使わないなら `#[cfg(windows)]`。外から到達できる `pub` 項目には何も付けない(PR #494 の S4)。モジュール単位の 1 行(`#[cfg_attr(not(windows), allow(dead_code))]`、呼び出し元が Windows 側にあるモジュール)を超えて必要になったら止めて報告する(P4 で、件数ではなくこの形に書き直した。ADR-229 参照)。
6. **CI の見方**(ローカルで見ない代わりに、ログで確かめる): `gh pr checks <N>` で全ジョブ。Linux の `test` ジョブのログ(`gh api repos/cuzic/awase/actions/jobs/<job id>/logs`。job id は `gh pr checks` の URL の末尾)で、①追加・移動したテスト名が列挙されて PASS、②`^warning` と `never used`/`never read` が 0 件。`windows-build`(clippy `-D warnings` と lib テスト)と `windows-cross-check` が green。run 全体が完了するまで `gh run view --log` は使えないが、`gh api .../jobs/<id>/logs` は完了済みのジョブなら読める。
7. 実装後は Opus の PR レビュー(読み取り専用、形態 b: 指摘 → 修正 → 同じレビュアーに再確認)を受けてからマージする。マージは所有者の許可を得てから。
8. **1 つの PR で ADR・ガード・実装の論点を混ぜない**(特に `CORE_MODULES` のガードは P0 として単独で入れる)。

## 1. 済(Tier-1 の最初の弾。PR #492〜#495 としてマージ済み、`42bc25ea`、2026-10-06)

第 1 弾の続き(PR #496〜#499。2026-10-06 にマージ済み、develop `c225f8d6`。いずれも Opus の PR レビュー round1・round2 を経た): **P1**(#496)`ForegroundScope` を `state/foreground_scope.rs` へ、`ImeUpdate`/`ImeObs` を `state/ime_update.rs` へ移動、**S2**(#497)`state/physical_disposition.rs` を pre-push・表・mutants に追加、**RW**(#499)ReplayWriter(再生できない分岐 4 → 3。非 Sync の手組み 3 本と chain の照合を追加。`replay_record` の「信用・推定している分岐」の残りは (a) gate の逆算、(c) 非 Sync の ImmCross の command 再計算の skip、(d') 非 Sync の ImmCross の `is_applicable` の代用)、**P0**(#498)`CORE_MODULES` のガード(47 件。`layer_boundary_guard.rs` に実装、`NOT_CORE_MODULES` 8 件と網羅検査つき)。

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
| **P0** | Tier-2 のガード | **済(#498)**: `layer_boundary_guard.rs` に定数 `CORE_MODULES`(47 ファイル、付録 A)・`NOT_CORE_MODULES`(8 件)・テストを足した(元の計画: `architecture_guard.rs` に初期 45 ファイル)。各ファイルの本番コード(`#[cfg(test)] mod tests` より前、コメント行を除く)が、Tier-2 の 4 規則(壁時計 `Instant::now()`/`SystemTime::now()`、`static`(不変も含む)と `thread_local!`、ファイル内の `#[cfg(windows)]` 項目(`mod` 宣言と `#[cfg(any(windows, test))]` は対象外)、FS/環境変数/レジストリ)に違反しないことを確かめる。実装上の注意は §2 の直後「P0 の実装上の注意」 | なし(単独で先に入れる。#492〜#495 のマージ後に再実測してから載せる) | Linux の `test`(`architecture_guard`)が green。`CORE_MODULES` の全ファイルで違反 0 | 偽陽性が多く、規則の書き分けが複雑になる(例: 5 規則以上、または許可リストが必要になる)。そのときは止めて報告 |
| **P1** | 型移動(R4・R5 の準備) | `ForegroundScope`(`win32.rs:46-60`。**済: P1 で `state/foreground_scope.rs` へ移動**)を ungated(例: `state/foreground_scope.rs`)へ、`ImeUpdate`/`ImeObs`(`observer/ime_observer.rs:22-45`)を ungated(例: `state/ime_update.rs`)へ移し、元の場所で `pub use` する | **#495 のマージ後**(`win32.rs` が重なる) | Linux と Windows でビルドが通る。新しい警告 0。呼び出し元の変更 0。test・windows-cross-check・windows-build が green | 型に gated な依存が見つかる(`ImeUpdate` は `InputModeState` などだけのはずだが**未確認**) |
| **P2** | 核と殻の分割(まだ gated、R5) | `platform_state.rs` の **13 メソッド**を `_in_scope(scope)` 版(5 つは既存)と殻に分ける。内訳: 10 メソッド(`arm_mode_key_pass_mark`・`mode_key_pass_expiry_wait_ms`・`mode_key_pass_window_remaining_ms`・`mode_key_pass_mark_live`・`expire_mode_key_pass_mark`・`arm_external_change_watch`・`external_change_watch_remaining_ms`・`follow_external_change`・`align_after_expired_mode_key_pass`・`invalidate_intents_if_mode_key_pass_live`)+ 連鎖の 3 メソッド(`note_awase_write_for_mode_key_pass` を呼ぶ core の `record_optimistic`・`record_confirmed`、それを呼ぶ `record_ime_apply_result`)。殻は **`platform_state.rs` の子モジュール**(`#[cfg(windows)] mod shell;`、`state/platform_state/shell.rs`)の `impl ImeStateHub` に置く。殻は「`foreground_scope()` を読む → `_in_scope` を呼ぶ」の 1 行だけ。殻のメソッド名は今と同じで、runtime/ の呼び出し元の差分は 0。**`architecture_guard.rs:1620` の `RECORDERS`(`.record_confirmed(`=5・`.record_optimistic(`=1。INV-A97-1/BUG-69)を、同じ PR で次のように直す: `state/platform_state/shell.rs` を走査から除外し(殻は委譲するだけ。同じ理由で `:3767-3775` の呼び出し元の固定からも、殻のファイルは対象外にする方針とする)、`.record_confirmed(` と `.record_confirmed_in_scope(` の合計を 5、`.record_optimistic(` と `.record_optimistic_in_scope(` の合計を 1 に保つ(殻の本体の呼び出しを数えて 6 に書き換えると、「記録の呼び出し元が 1 つ増えた」ことと区別できなくなる)** | P1。P3 と同じファイルなので **P2 → P3 の順(直列)** | windows-build の lib テスト(`platform_state` の 48 本)と `architecture_guard`(`:3767-3775` の呼び出し元の固定、`:1620` の `RECORDERS`)が green。runtime/ の差分 0 | 殻が 1 行で書けないメソッドが出る(スコープを 2 回読むなど) |
| **P3** | 時計の構築の分割とテストの付け替え | `ImeStateHub::new()` を `#[cfg(windows)]` にし(本体は `HubClock::wall(crate::hook::current_tick_ms)` のまま、`platform_state.rs` 内)、ungated な `with_clock(clock: HubClock)` を足す。テスト用に `PlatformState::for_test(clock: HubClock)` を用意し、`platform_state.rs` のテストの `PlatformState::new()` **36 か所**を付け替える(うち、`effective_open()` を**呼ぶ** 19 か所は、構築の 36 か所とは別の集合で、時間が意味を持つので `HubClock::manual(<Windows と同じ大きさの基準 tick>)`、それ以外は同じヘルパー)。**Linux 用の壁時計を `cfg` で切り替える案は採らない**(時間軸の意味が変わる) | P2 | windows-build で `platform_state` の全テスト(48 本)が従来どおり pass(まだ gated)。`architecture_guard.rs:1382`(`HubClock::wall(crate::hook::current_tick_ms)` の件数固定)が書き換えなしで green | `new()` を使う ungated のコードが見つかる(`tests/support/harness.rs` は `ImeStateHub` を使っておらず影響なしと確認済み) |
| **P4a** | P4 の前提(まだ gated) | (1) `ImeStateHub::new()`(`HubClock::wall(crate::hook::current_tick_ms)`)・`PlatformState::new()`・`impl Default for PlatformState` を殻 `state/platform_state/shell.rs` へ移す(`platform_state.rs` に `#[cfg(windows)]` の項目を残さない。M13)。`architecture_guard.rs:1382` の対象ファイルを `shell.rs` へ付け替える(固定する文字列は変えない)。#502 の殻の形のガードが `new()` で落ちないよう、ガードを弱めずに除外できる形を確かめる。(2) `platform_state.rs` の `crate::win32::ForegroundScope`(22 か所)→ `crate::state::foreground_scope::ForegroundScope`、`crate::observer::ime_observer::ImeUpdate` → `crate::state::ime_update::ImeUpdate`(M12)。(3) P4 で gate を外すと Linux で解決できない参照(`crate::win32::`・`crate::observer::`・`crate::hook::`・`crate::tsf::observer::` ほか)の一覧を PR 本文に書く | P3(マージ済み) | windows-build(lib テスト 48 本・clippy)・windows-cross-check・Linux の `test`(`:1382`・`RECORDERS`・殻の形のガード・網羅検査)が green。呼び出し元の差分 0 | 殻の形のガードを弱めないと `new()` を殻に置けない |
| **P4** | `platform_state` の ungate(段階 2 本体) | `state/mod.rs` の `#[cfg(windows)] pub mod platform_state;` と `#[cfg(windows)] pub use platform_state::PlatformState;`(develop では `:187-190`)の gate **だけ**を外す。**`ime_decision_view` は gated のまま**(`tsf::observer` の `TsfObservations`・`ActiveImeKind`・`candidate_was_seen()` に依存し、`platform_state` は使っていない。借用ビューの所有化の対象で、P4 には含めない。`ime_event_log` は #492 で済み)。テストの書き換え: 殻の名前のメソッドを呼ぶ **16 か所**(`follow_external_change` 9、`arm_external_change_watch` 3、`record_ime_apply_result` 3、`record_confirmed` 1)と、`crate::win32::foreground_scope()` を直接呼ぶ **9 か所**を、`_in_scope` + `test_foreground_scope()`(`:2561`、既存)へ。 | #494・#495・P1〜P3 のマージ | Linux で `state::platform_state::tests::*`(48 本の見込み)が回る。Linux のビルドで新しい警告が無い(`allow` を増やさない) | `allow` を足さないと消えない警告が 3 件を超える。**Linux で結果が変わるテストが出たら止めて、時間軸の違い(`HubClock::manual` の基準 tick)か本物のずれかを切り分ける**(切り分けは、同じテストを基準 tick を変えて 2 通り流して見る。本物のずれなら BUG として扱う) **【Opus 確定の追記】前提: P4a(M12・M13)が済んでいること。可視性: 殻からしか呼ばれない `_in_scope` とその専用の下位関数(約 13。`record_*_in_scope` を含む)に `#[cfg(any(windows, test))]`(`allow(dead_code)` は使わない)。`pub` にはまだしない(P5 で必要なものだけ)。`platform_state` を `CORE_MODULES` に足す(P4a で `new()` を殻へ移した場合)。成否の判定に「Linux の `test` ログの `never used`/`never read` が 0」と「`layer_boundary_guard` の網羅検査が green」を追加。取りやめ条件に「`#[cfg(any(windows, test))]` で消えない未使用が残り、`allow` を足さないと消えない警告が 3 件を超える」を追加** **【済(#510、2026-10-06)】テストの書き換えは殻の名前を呼ぶ 16 か所(「`foreground_scope()` の直接呼び出し 9 か所」は実在しない誤りだった)。可視性は、方針の前提(未使用になるのは `_in_scope` 約 13 だけ)が誤りで、`ImeStateHub` の全体が Windows 側の呼び出し元だけだったため、`_in_scope` への `cfg(any(windows, test))` ではなく、モジュール単位の `#[cfg_attr(not(windows), allow(dead_code))]` 1 行になった(Opus が受け入れた逸脱。`not(windows)` の `allow(dead_code)` の指標は 45 → 46。S23)。本当の未使用は windows-build の clippy `-D warnings` で引き続き検出される** |
| **P5** | ハーネスの写しの置き換え(1 系統ずつ) | `tests/support/harness.rs` の手書きの写し 7 系統のうち、`platform_state` の ungate で本物の呼び出しになる 5 系統(`apply_key_effect_prediction`・`effective_open_at`・`warrant_context`/`issue_actuation_order`・`record_explicit_intent`/`write_*`・`arm/follow_external_change`)を、1 系統ずつ本物の `ImeStateHub` の呼び出しに置き換える。最初は `effective_open_at`(`HubClock::Manual` で仮想時計とそろえる)。ungate 自体では写しは減らない。残る 2 系統(`kp_stage_key_effect_track`/`kp_predict_key_effect`、`ir_apply_drift_correction` の前半)は runtime/ にあり F4 で扱う | P4 | `closed_loop_scenarios` の全シナリオの結果が不変。写しが 7 → 6 | 本物を呼ぶと結果が変わる。それは**写しのずれの発見**なので、ADR-224 の着手条件を満たす実例として `docs/known-bugs/` に記録して止める **【Opus 確定の追記】可視性の方針は上の「P4・P5 の前に決めた」の (a)。写しを 1 系統置き換えるたびに、その系統が要るものだけを `pub` にする(型 `ImeStateHub`、`with_clock`、時計を進める口、`dispatch_event`、`write_imm_cross_probe`・`write_observer_poll`・`report_conv_open_inference`、`model`・`desired_open`・`input_mode`・`explicit_intent`・`set_is_japanese_ime`、①`apply_key_effect_prediction`、②`effective_open_at`、③`warrant_context`・`issue_actuation_order`、④`write_set_open_request`・`record_explicit_intent`、⑤`arm_external_change_watch_in_scope`・`follow_external_change_in_scope`)。`pub` の集合を `architecture_guard` で固定。取りやめ条件に「1 系統の置き換えのために `record_*_in_scope` を `pub` にする必要が出た(止めて、その系統を写しのまま残すか、(b) を再検討)」を追加**  **【P5a-1 済(#518、2026-10-06)・計画からの逸脱】** 「1 系統ずつ・最初は `effective_open_at` だけ・写し 7 → 6」ではなく、**4 系統と `warrant_context` をまとめて置き換えた**(`effective_open_at`・`apply_key_effect_prediction`・`record_explicit_intent`・`arm/follow_external_change` の 4 系統。`warrant_context/issue_actuation_order` の系統は `warrant_context` までが本物で、`issue_actuation_order` は写しのまま)。理由: ハブの `shadow_model`・`intent_store` は private で、`effective_open_at` だけを本物にするとハーネスが同じ状態を別に持つ写しになる。本物にするには `ImeModel`・`IntentStore`・監視窓・仮想時計をまとめてハブへ移す必要があった(`reduce` → `dispatch_event` を含む)。写しはヘッダの 7 系統から 3 系統(`issue_actuation_order`/`write_*`・`kp_*`・`ir_apply_drift_correction`)に減り、`settle` 内の `align_placeholder_desired` の写しが 1 つ残る(ヘッダに追記)。`pub` にした集合は方針の想定(約 20)より小さい(新たに `pub` にしたのは型 `ImeStateHub` とメソッド 12 個。固定した集合の 13 には、以前から `pub` の `PlatformState::new` を含む。観測の書き込み口と `desired_open`/`input_mode`/`explicit_intent` は `model()` 経由の読みで足りた)。`pub` の面は `platform_state_pub_fns_are_fixed_and_exclude_recorders`、本番のハブへの到達不能性は `production_hub_is_unreachable_from_outside_the_crate` で固定。`closed_loop_scenarios` は 24 PASS(シナリオ 19 + `pseudo_ime` 5)・`#[ignore]` 1(`conversion_esc_does_not_leave_stale_conv_stage`、未起票。擬似 IME の仮定を実機で確かめるまで外さない)で、結果は不変。**ignore 中のシナリオは本物のハブでは未確認**。`follow_external_change_in_scope` に `#[must_use]` を付けない理由: 副作用が本体で戻り値(追随した値)は付帯情報、clippy の `must_use_candidate` は `&mut self` に効かず、本番の殻 `follow_external_change` にも付いていない(そろえる)。 **【P5b 済(#564)】** 写しを 2 つ本物にした: `key()` の判断を F4b の `plan_key_effect_track` に、`settle` 内の `align_placeholder_desired` の写しを `ImeStateHub::align_placeholder_desired`(`pub` 化、`PLATFORM_STATE_PUB_FNS` に追加)の呼び出しに。残る写し: `issue_actuation_order`/`write_*`(`warrant_context` + `issue_open_warrant` の 2 行。`ActuationOrder` の warrant が private で、本物にすると `pub` 面が増える割に得るものが小さい)、`kp_predict_key_effect`(分けない)、`ir_apply_drift_correction` の前半。 |
| **RW** | ReplayWriter(並行する別の線) | テストだけで本番の変更 0。`MechanismWriter`/`AsyncMechanismWriter` の実装 1 つで、記録済みの `(mechanism, outcome)` を返し、`run_chain(_async)` の走査と打ち切り位置を既存コーパス(`tests/journals/actuation_decision/`)で再生する(F-D1 の handler 例外の実証)。任意で `ActuationDecisionRecord` に `gate` を明示記録する(旧コーパスは逆算のフォールバックを残す) | なし(P1〜P3 とファイルが重ならない) | 既存コーパスの 37 レコードで、走査の再生が全件一致。`replay_record` の逆算・skip が新形式のレコードでは通らない(再生できない分岐 4 → 1) | `ReplayWriter` のために本番の `Actuation`/`ActuationOrder` に test 専用のコンストラクタが要り、warrant の型安全(ADR-090)を弱める。旧コーパスで 1 件でも不一致(それは本番の挙動差の発見なので BUG として扱う) |

### P0 の実装上の注意(Opus round3 S13)

1. **テキストの切り方**: 「`#[cfg(test)] mod tests` より前、コメント行を除く」だけでは不十分。行末のコメント(`foo(); // Instant::now()`)、文字列リテラル(ログの文言に `std::fs` など)、`mod tests` 以外の名前のテストモジュール(PR #493 の `plan_shell_tests`)、`mod tests` より前の `#[cfg(test)] impl`/`#[cfg(test)] fn`(例: `platform_state.rs:1610` の `#[cfg(test)] impl ImeStateHub`)を見落とす。**`layer_boundary_guard.rs` の `code_lines`/`test_block_mask`(`:47-112`。コメントを除き、`#[cfg(test)]` が付いた item の本体を丸ごと覆う)を使う**のが最も確実。P0 のテストを `layer_boundary_guard.rs` 側に置くか、同じヘルパーを移す。
2. **`#[cfg(windows)]` の検出**: 完全一致だけだと `#[cfg(all(windows, …))]`・`#[cfg(target_os = "windows")]`・`#[cfg_attr(windows, …)]` を見逃す。許可する 3 つの形(セミコロンで終わる `mod <名前>;` 宣言の直前(**インラインの `mod x { … }` は不可**。PR #498 レビュー S15)、`any(windows, test)`、`cfg_attr(not(windows), allow(...))`(S16。`allow(` に限る))を明示的に除いた上で、`cfg` の中に `windows` の語があれば違反とする。
3. **壁時計**: `Instant::now()`/`SystemTime::now()` に加えて、`quanta::Clock::new()`/`quanta::Instant::now()`(journal が使う)、`timed_fsm` の実時計も対象にする。
4. **FS/環境変数**: `std::fs`・`fs::` に加えて `File::open`、`Path::exists`/`metadata`。`env!`/`option_env!`/`include_str!` はコンパイル時の展開なので許可する。
5. **`static`**: 不変も含めて一律に違反とする(不変の表は `const` にするか、そのファイルを `CORE_MODULES` に載せない)。

### P4・P5 の前に決めた: 可視性の方針(Opus 確定、2026-10-06、`docs/adr/review/229-opus-visibility-policy.md`)

**方針案 (a) を、次の 3 つの規則つきで採る**(代案 (b)=ハーネスを `src/` の `#[cfg(test)]` へ移す、は採らない。約 2,060 行の移動と CI 2 か所の書き換えに加え、`layer_boundary_guard`・`architecture_guard` が移した別ファイルを本番コードとして走査して除外が増えるため)。

1. **閉ループのハーネスが呼ぶものだけを `pub` にする**(型 `ImeStateHub` と、メソッド約 20)。`#[doc(hidden)]` は付けない(awase-windows は非公開で rustdoc を CI で見ていない。意図は各メソッドの doc の 1 行「閉ループのハーネス `tests/support/harness.rs` からも呼ぶ。本番の呼び出し元は crate 内だけ」で伝える)。値を返す `pub fn` には `#[must_use]`(windows-build の clippy pedantic `must_use_candidate`)。
2. **殻からしか呼ばれない `_in_scope`(とその専用の下位関数。約 13)は `#[cfg(any(windows, test))]`**(`allow(dead_code)` は使わない)。**記録系(`record_optimistic_in_scope`・`record_confirmed_in_scope`・`record_ime_apply_result_in_scope`。INV-A97-1)は絶対に `pub` にしない**(`RECORDERS` が見ない crate の外から `applied` を書けてしまうため)。
3. **`pub` にした集合を `architecture_guard` で固定する**(`platform_state.rs` の `pub fn` の名前の集合が期待の一覧と一致する。P5 の各 PR で一覧に足す。記録系の名前が `pub fn` に現れたら失敗)。

P4 に書かれていなかった前提 2 つ(Opus の Must): **M12** `platform_state.rs` に `crate::win32::ForegroundScope`(22 か所)と `crate::observer::ime_observer::ImeUpdate` のパスが残っており、gate を外すと Linux で解決できない → P1 で移した先のパスに書き換える。**M13** P3 で入れた `#[cfg(windows)] fn new()` が、#498 の Tier-2 の規則 3 に当たり、`platform_state` を `CORE_MODULES` に入れられない → `new()`・`PlatformState::new()`・`Default` を殻 `shell.rs` に移し、`architecture_guard.rs:1382` の対象ファイルを `platform_state/shell.rs` に付け替える。**これらは P4a(P4 の前提の PR、まだ gated)として、P4 本体の前に行う。**

## 3. 第 3 弾(F の分割、Tier-2)

**共通**: サンドイッチ(`observe` → core の `decide` → `execute`)。分けたファイルは P0 の `CORE_MODULES` に足す(同じ PR)。journal replay で、元の判断の入力が復元できるかを確かめる。いずれも、再発ファミリーのガード(`fix-requires-evidence.md` の表)と `decision3_…` の instrument 一覧を見直す。

| ID | 対象 | 注意 |
|---|---|---|
| F1 | `ir_decide_read_strategy`(36 行、`runtime/ime_refresh.rs`) | 最も易しい。**同じ PR で、F1 の `decide` の再生 fixture を足すと同時に、dev-only crate `crates/awase-replay`(`publish = false`、依存は `serde`・`serde_json`、`awase-windows` の dev-dependency)を作り、再生ハーネス `replay_dir<T>(dir, check) -> ReplayReport` を取り出す**(ADR-229「FCIS の汎用部品の判断」)。既存の 4 か所(`journal_replay.rs`・`drift_correction_replay.rs`・`actuation_decision_record.rs` の fixture 読み込み)を移し、`journal_replay.rs` が直下を全部読む件は、直下の JSON を `tests/journals/conv_classify/` へ移して解消する(1 ディレクトリ = 1 形式)。`cargo machete` の dev-dep 誤検出に注意(最初の使い手と同じ PR で入れる) |
| F2 | `dispatch_ime_set_open` の `plan_set_open`(`runtime/executor.rs`) | actuation 合流点のファミリー(ADR-119)。gate を再検出する 3 関数の設計(ADR-180)に触れない |
| F3 | `execute_relay`/`drain_deferred` の計画/ガード状態機械 | **defer/replay キュー(ADR-156)。defer 側と drain 側の 2 窓口を、同じ PR で必ず対にする**。**E0 の暗黙の順序: `execute_relay`(`executor.rs:430-445`)は Consume のとき Timer だけが即時に実行されてキューを追い越す。この順序を保つ**(`docs/tasks/effect-signature-inventory-2026-10-06/inventory-e0.md` §11) |
| F4 | `ir_apply_drift_correction` の `DriftPlan`。ハーネスの写しの残り 2 系統(`kp_stage_key_effect_track`/`kp_predict_key_effect`、`ir_apply_drift_correction` の前半)を本物に **【実施状況 PR #514】`ir_apply_drift_correction` を observe → `decide_drift_plan`(`state/drift_plan.rs`、`CORE_MODULES`)→ execute に分割済み。ただし harness の写し 7 → 1 は未達: harness は変えておらず、`ir_apply_drift_correction` の前半を `decide_drift_plan`(と `check_drift_correction`)の本物の呼び出しに置き換えるのは P5 の作業。`kp_stage_key_effect_track` の判断は F4b で `state/key_effect_predictor.rs::plan_key_effect_track`(純粋、Linux 単体テスト)に切り出した。ハーネスの `key()` の `passive_rule_eligible` 決め打ちは P5b(#564)で `plan_key_effect_track` の呼び出しに置き換えた。`kp_predict_key_effect` は観測(keymap・TSF)と実行が大半で、純粋な `predict_with_override` は既に分離済みのため分割しない(F5d と同じ理由)** | 写し 7 → 1 への道 |
| F5 | focus 系(`classify_focus`、`msaa_classify`、`resolve_focus_kind`、`learn_imm_capability_on_focus`)。「O が facts、core が判断」 | 条件付きの段階的な読み取りは F-D1 の handler 例外(MSAA までの同期の段階)。UIA は A 種の Cmd/Event。UIA 経路の削除(約 400 行)は別判断 追記: `msaa_classify`(#512)・`classify_focus`(#527)は handler 例外なしで分割済み(`Option` を返す純粋関数+殻)。残りは `resolve_focus_kind`・`learn_imm_capability_on_focus` |
| F6 | Output(`vk_send` F 623 行、`output/mod` 576 行)の `Vec<Cmd>` 化 | `Output` の状態を `OutputState` へ(`RAW_TSF_LITERAL`、`OutputActiveGuard`)。`OutputActiveGuard` は `GateAcquire`/`GateRelease` を対に(ADR-156)。**【実施状況 PR #523】退避 gate の判断だけを `state/deferred_gate_plan.rs`(`plan_blocking`/`plan_defer`/`plan_drain_before_send`、`CORE_MODULES`)に切り出した。見送り: 送信パイプライン(`assess_warmth` と経過時間が混ざり `TickMs` 化が先に要る)〈**訂正(F6b、PR #528)**: `output/`・`tsf/` の本番コードに `Instant` は無く、時刻は `current_tick_ms` の `u64` で既に持たれていたので、`TickMs` 化は先に要らなかった。全数表で固定できるので V4 は適用しなかった〉、`ms_ime_gate_defer`(判断の途中でタイマーと coro を作る)。`INPUT_DEFER`(c21)は別のキューで F6 の対象外。**【F6b、PR #528】** 送信パイプラインのうち `assess_warmth` と `send_romaji_as_tsf_warm` の 4 述語(warm/cold・期限切れ・GJI 応答待ち・長期静止・LiteralDetect 設置)だけを `state/warm_send_plan.rs` に切り出した(読む位置と OS 送信の順序は殻のまま)。**【F6c】** 解放側の `finish_probe_stage` を `probe_or_recovery_block_reason(true)`(`plan_blocking`)経由にした(`raw_recovery_owns_deferred` の呼び出し 2 → 1)。`take_pending_deferred_if_probe_idle`・`flush_stale_deferred_vks_after_recovery` は意図して通さない(`deferred_gate_plan.rs` 冒頭に理由)。**【F6 の残りは分けない、調べて決定】** `Vec<Cmd>` 化・`OutputState`・`OutputActiveGuard` の対は、判断の移動ではなく寿命の設計変更で、得るものが小さく、実害の再発リスクが大きい。数え方: 各ファイルの先頭から最初の `#[cfg(test)] mod tests` の手前までを本番とし、`//` で始まる行を除いた(Opus 再確認で、最初の数え方がテスト・コメント込みで過大だったのを訂正)。`OutputActiveGuard::begin()` は本番に 11 箇所・9 ファイル(`tsf/warmup/` 4〔`gji_warmup_coro.rs` 2、`literal_detect_fsm.rs`・`ms_ime_ready_coro.rs` 各 1〕、`runtime/executor.rs` 2、`vk_send.rs`・`sender.rs`・`tsf_warmup_coord.rs`・`runtime/mod.rs`・`key_pipeline.rs` 各 1。`output/mod.rs` の 6 箇所は全てテスト)あり、`ChromeProbe`・coroutine のフィールド・`RefCell<Option<_>>` へ move されて await をまたぐ。Drop の RAII が、キャンセルされた future でも深度 1→0 の解放(`OUTPUT_GATE.active=false` + `WM_DRAIN_OUTPUT_QUEUE` の post)を保証している(ガードを `mem::forget` する本番コードは無い)。`GateAcquire`/`GateRelease` の明示対にすると、解放漏れ=ゲートが開かないままキーが INPUT_DEFER に溜まり続ける故障が、全終了経路(キャンセル・早期 return・`clear_pending_tsf` による上書き)で起こりうる。純粋に出せるのは深度の 0→1・1→0 の 2 つの比較だけで、F5d と同じく移る判断が少ない(BUG-65 のラップアラウンドは `real` フラグで済み)。`RAW_TSF_LITERAL`(`RawTsfLiteralPending`)の `OutputState` 化は、本番の参照が 3 ファイル 16 箇所(`lib.rs` 1〔定義〕、`output/mod.rs` 13、`tsf/output.rs` 2)で、規模は小さいが判断は変わらないので同じく分けない(ADR-164 のグローバル static の集約として扱う場合は別)。再着手する条件: 解放漏れを型で防ぐ手段(ガードを `must_use` の線形な型にする等)が先に決まり、ゲートの故障を実機で再現できること。** |
| F7 | 同期経路の actuation の判断(ADR-241 段階 1 の決定2): `ImeController::apply` の本体・`SyncChainWriter::write`・`dispatch_ime_set_open` の判断(D1 → gate → claim → plan → 同期チェーン)・`kp_stage_post_decision` の SetOpen の部分 | **【実施状況】PR-A**: 核 `state/sync_actuation.rs`(`apply_sync`・`CoreSyncWriter`・`dispatch_set_open`、`CORE_MODULES`・mutants・pre-push・fix-requires の表に登録)とハブの `on_engine_set_open_request` に移した。差し替え口は `CommandSink` 1 つ(殻は `ShellCommandSink`)。同期の合流点の宣言(ADR-119・`RESTRICTED_CALLS`・`architecture_guard`)を更新。ハーネスへの配線(決定3)と既存の再生一式の撤去は PR-B・PR-C |

### F の分割の各 PR に含める「汎用でない」改善(ADR-229「FCIS の汎用部品の判断」)

- 生の `u32` の `focus_gen`(22 か所)の newtype 化(取り違えを型で防ぐ。個別の PR)。
- `decide` を出すときの入力型のうち、replay に使うものにだけ `serde` を足す(共通の trait は作らない)。

## 3.5 世界モデルと Effect 列の検討結果から(2026-10-06、ADR-229 の同名の節、`docs/adr/review/229-opus-effect-plan-round1.md`)

| ID | 内容 | 判定 | 根拠・注意 |
|---|---|---|---|
| E0 | Effect の署名の棚卸し(`MechanismCommand`・`ProbeAction`・`UiEffect`・`OutEffect`・`TimerCommand` などの全 variant。各々の順序の制約・冪等性・副作用) | **済(2026-10-06)**。`docs/tasks/effect-signature-inventory-2026-10-06/`。54 enum・variant 244・命令約 65、暗黙の順序 22 件、吸収が危険な実例 7 件 | 書き込みの吸収が危険な理由(BUG-141・ADR-208)の根拠表になる **所有者の方針(2026-10-06): 順序契約のテストは全体には作らない(ほとんどの処理は順序不要)。順序の制約は少数の依存辺(DAG)で、`docs/tasks/effect-signature-inventory-2026-10-06/dependency-edges.md` が正。F の分割の PR は、触る辺を本文に書き、その辺だけを固定するテストを足す** |
| E1 | 決定関数が省略の理由を enum で返して journal に載せる(新しい解釈器は作らない) | 実際の調査で理由が足りなかった決定から 1 つずつ | `GateResult`・`suppress_reason`・`Delivery`・`FeedbackPolicy` の action が既にある |
| BUG-098 | 世代の無い非同期の shadow toggle OFF の完了を、既存の世代(F-D5-2)で直す | 独立した修正タスク(挙動を変えうる。所有者の判断) | Plan の有無に関わらず効く唯一の実害対応 |
| W-a | 失効判定のカウンタ 8 種類・生の `u32` の `focus_gen`(22 か所)の newtype 化 | F の分割の各 PR の中で | 取り違えを型で防ぐ。toolkit round1 でも指摘 |
| W-b | ~~死んだフィールド `Runtime.state_dependent_key_warning_dialog` の削除~~ | **取りやめ(W0-b の誤検出)** | 実装担当が grep で確認したところ、`runtime/mod.rs:1609-1611` の `select(...)` で使われている。削除しない |
| W-c | journal の `ImeEvent` の記録に時刻(`tick_ms`・`Instant`)と `seq` を足す | 再生の入力として足りるようにする。ADR 起草の前に所有者の判断 | W0-a。`event_log` は本番で読み手がいない |
| E2〜E6 | `Try`・正規化と法則・`Bracket`・capability の網羅テスト・観測の購読 | **着手しない** | 待つ条件は `229-opus-effect-plan-round1.md` の §4(F の分割が 3 本進み、`run_chain` 以外に有限の機構を順に試す連鎖が 2 つ以上現れたら、など) |

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
5. P3 はテストの構築の付け替え(36 か所。うち時計を読むのは引数なしの `effective_open()` を呼ぶ 2 本だけ。「19 か所」は `effective_open()` という文字列の出現数で、`effective_open_at(..)` の呼び出し数ではない=実数は 33。Opus の #503 レビュー Should 3)を含む。P4 は殻の名前 16 か所・`foreground_scope()` 直接 9 か所のテストの書き換えを含む。
6. 順序: P1 は #495 のマージ後。P2 → P3 は直列。RW は並行可。
7. ~~`HubClock` は Tier-2 の外~~ **訂正(段階 B-2、PR #568)**: 実時計を構築側から注入する形にしたので `hub_clock.rs` は `CORE_MODULES` に入った。
8. `CORE_MODULES` は「違反 0 のファイルだけ」で、許可リストは持たない。違反のある 8 ファイルの内訳(付録 B)は「殻へ出す候補」。
9. ADR-224 の改訂は所有者の判断で、ADR-229 の改訂とは別に記録する。
10. 共通の後処理と、着手前の「テストが呼ぶ識別子は gated 側にないか」を、各 PR の成否の判定に入れる。
11. `allow(dead_code)` を増やさない。
12. 成否は CI のどのジョブのどのログで見るかを、PR の本文に具体的に書く(§0-6)。

## 付録 A: `CORE_MODULES` の初期候補 45 ファイル(実装では #496 の `foreground_scope`・`ime_update` を足した 47 ファイル。実装の `CORE_MODULES` が正)(`state/`、Opus round2、origin/develop d0d42be6 時点。#492〜#495 のマージ後に再実測してから確定する)

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
| `state/hub_clock.rs` | `Instant::now()` 2 件(`:41`、`:51`) | ~~時計の実装そのもの。恒久的に Tier-2 の外~~ → 段階 B-2(PR #568)で実時計を注入にして `CORE_MODULES` へ |
| `state/ime_event.rs` | `#[cfg(windows)] impl HwndId`・`impl From<HWND> for HwndId`(`:36`、`:46`) | HWND との変換は殻(拡張 trait 化) |
| `state/ime_model.rs` | `Instant::now()`(`:435` の `effective_open()`、`:504`) | `effective_open_at(now)` を呼ぶ側へ |
| `state/ime_profile_driver.rs` | `static` 3 つ(`IMM_CROSS_DRIVER`・`IMM32_UNAVAILABLE_DRIVER`・`TSF_NATIVE_DRIVER`、`:186-188`。不変だが `static` は一律に違反とする。可変か不変かをテキスト走査で判定するのは難しい) | ゼロサイズの構造体なので `const` による `&'static dyn` の昇格で置き換えられる見込み(**未確認**)。直すまで `CORE_MODULES` に載せない |
| `state/key_effect_predictor.rs` | `#[cfg(windows)]` の `get_gji`/`get_native`(`:570`、`:580`)。FS/レジストリを読む | 殻へ |
| `state/key_effect_runtime.rs` | `#[cfg(windows)]`(`:377`、`:385`、`:393`、`:408`、`:440`、`:816`)。学習済み表のパス解決・`fs::metadata`・読み込み | 殻へ(FS) |
| `state/probe_admission.rs` | 可変の `static REJECTION_COUNTERS`(`:71`)、`#[cfg(windows)]` の関数(`:325`) | カウンタは shell へ |
| `state/mod.rs` | `#[cfg(windows)]` 8 件。`mod` 宣言のほか、`pub(crate) use conv_mode::{…}`(`:32-35`)・`pub use platform_state::PlatformState`(`:189-190`)・`pub(crate) use ime_decision_view::{…}`(`:194-195`)など `use` の再公開を含む | **恒久的に `CORE_MODULES` に載せない**(モジュール宣言の集約ファイル。例外を `use` まで広げると、core のファイルに gated な再公開を置く抜け道になる) |

## 進め方の見直し(2026-10-06、所有者承認)

F の分割と P4・P5a-1 の実装で分かったことから、進め方を 5 点見直す。既存の行と節は書き換えない(並行 PR との衝突を避けるため)。事実は develop `bbb70f29` 時点のコミット本文と PR 本文で確かめた。新しいタスクの ID は `V1`〜(`R1`〜`R7` は ADR-229 の移行のレシピが使っている)。ADR-229 の同名の節には、決定の変更(D4 の順序)と要約だけを置く。

### 根拠の事実

1. **機械的な後処理の漏れ**: 新しい純粋モジュールを自動チェックに載せる作業が、Opus の PR レビューを受けてから足されたものが 3 件ある。F2(#504、`d04c3843`: 表と mutants の 2 か所)、F3(#517、未マージ、`783b6fa3`: pre-push・表・mutants の 3 か所)、F4(#514、マージ済み、`fbd1bea6`: 3 か所。レビューの主題は別の指摘で、3 か所は「併せて」の対応)。
2. **走査ガードのすり抜け**: #513 の再確認で、`/*` を含むコメントで本番コードが消える穴が見つかり、字句走査 `strip_comments` に置き換えた(`754811cd`。生文字列の接頭辞は `ec1aa703`)。#518 は `pub` の固定ガードの走査範囲を全ファイルの `impl` へ広げた(`f16ffb10`)。#517 は e12・e13 の走査テストを違反例を検出できる形に作り直し、その後も検出器の panic を直した(`7aafb794`)。P5 の `pub` の集合の固定そのものも、crate 境界が無いことの穴埋め。
3. **理由つきの Plan の形**: 「事実 → 理由つきの Plan の enum → 殻が実行」が F1・F2・F3・F4 の 4 回できた(F5a はロール値の表引き)。汎用の Effect/Handler の部品は、どの F でも必要にならなかった。
4. **再生 fixture**: F の分割で新しく再生 fixture を作れたのは F1 の `read_strategy` だけ。F4 は事実の型が `Instant` を持つので fixture を作らず全数表にした(#514 本文)。F2・F3・F5a は入力空間が小さく、全数表で足りた。
5. **P5 の写し**: P5a-1(#518)で 5 系統を同時に本物の `ImeStateHub` に置き換えても、`closed_loop_scenarios` は 24 PASS・失敗 0・`#[ignore]` 1 のままで、写しのずれは見つからなかった(ignore 中の 1 本は本物のハブで未確認)。

### 書き方の明文化(根拠 3)

ADR-229 の代案 A と E1 を、F の分割の書き方として明文化する。新しい決定ではない: `decide_*` は理由を持つ Plan の enum を返す、実行は殻の 1 関数、E1(省略の根拠)は Plan の理由の一部。4 回できたので書き方として固定する。汎用の部品は作らない(toolkit の「作る条件」は満たしていない)。

### 新しいタスク

| ID | 内容 | 成否の判定 | 撤去先・取りやめ条件 |
|---|---|---|---|
| **V1** | 差分で検査する(前提: 今の `adr-evidence-consistency` ジョブは `actions/checkout@v4` の既定の fetch-depth 1 で、xtask も差分を扱わない。`pull_request` のときだけ base を取得し、差分を読む処理を xtask に足す。develop への直接 push は V1 の対象外): PR の差分で `CORE_MODULES` に**足された**名前を検出し、その PR の差分で `.cargo/mutants-awase-windows.toml` の `examine_globs` にも足されていることを CI で要求する(`crates/xtask-adr-evidence` の延長)。pre-push の正規表現と `fix-requires-evidence.md` の表は「再発ファミリー」の一覧で、純粋なモジュール全部が載るものではないので、V1 では見ない(載せるかは V2 で人が判断)。除外リストは作らない(ADR-218〜220 と同じ型になるため) | `CORE_MODULES` に名前を足し mutants に足さない PR で CI が落ちる。名前を足さない PR では何もしない | 撤去: なし(develop への直接 push を拾えないので、V2 の 1 項目目の人手のチェックは残す)。取りやめ: 差分の検出が CI の base の取り方で不安定になる |
| **V1b** | 棚卸し(事実の記録のみ。今回は着手しない): develop `bbb70f29` で `CORE_MODULES` 52 件のうち mutants の `examine_globs` に載るのは 15 件、`fix-requires-evidence.md` の表に載るのは 9 件(数え方: mutants は `state/<名前>.rs` の完全一致、表は `<名前>.rs` の前が英小文字・`_` でない一致。basename の部分一致だと `belief`・`drift_correction`・`explicit_press` が別のファイル名に誤一致する)。穴埋めするか、するならどれかは別に判断する | — | — |
| **V2** | 完了前チェック(下記)。docs のみ | **済(本節)** | 撤去: 1 は残す(V1 は develop への直接 push を見ない)。4 は V4 を取りやめたら消す |
| **V3** | `strip_comments`(#513 で `architecture_guard.rs` に入った字句走査)を、テストの共通部品にし、各ガードが使う | 既存のガードの期待値が変わらない(変わったら、すり抜けの発見として PR 本文に列挙)。**独自のコメント除去が 0 になる**: `architecture_guard.rs:276` の `non_comment_lines`、`trim_start().starts_with("//")` の行内フィルタ(`architecture_guard.rs` 10 件・`layer_boundary_guard.rs` 2 件) | 撤去: 上の独自のコメント除去。取りやめ: 置き換えでガードの意味が変わり、1 PR で収まらない |
| **V4** | 分割の前提チェック 1 項目: 再生 fixture を作りたい分割では、事実の型の時刻を `Instant` ではなく `TickMs`(`HubClock` から読んだ tick)で持つ。根拠は F4 の 1 件だけ | 該当する分割の PR に、fixture か、作らない理由がある | 適用しない: 入力空間が小さく全数表で足りる、または時刻の持ち方を直す PR が分割の PR より大きくなる。取りやめ: 次の 2 本の分割で 1 度も当てはまらない |
| **V5** | e13(`OutputActiveGuard::begin()` を `spawn_local` の前に取る)を、ガードをタスクに所有権ごと渡す形などの型で表す。e12(Consume は Timer だけ即時)は型にしにくいので、走査テストに残る | 順序を入れ替えるとコンパイルが通らない。e13 の走査テストを削除 | 前提: #517 のマージ。撤去: e13 の走査テスト。取りやめ: 型にすると `OutputActiveGuard` の寿命が変わる(ADR-156 の defer/drain 2 窓口) |
| **V6** | crate 分割の前倒し(**方針のみ。着手しない**)。下の着手条件を満たしたら、着手するかを所有者に諮る | — | — |

### 完了前チェック(V2。Opus のレビューに出す前に実装エージェントが確かめる)

1. 新しく足した純粋モジュールを `CORE_MODULES` と mutants の `examine_globs` に載せたか。再発ファミリーに属するなら、pre-push の正規表現と `fix-requires-evidence.md` の表にも載せたか(§0 の 4 の共通の後処理)。
2. `.claude/rules/agent-handoff.md` の 1・3 に従う(ADR-242・ADR-243)。
3. ソース走査のテストを足したなら、違反例を 1 つ作って検出できることを確かめたか。
4. V4 に当たる分割なら、事実の型の時刻を `TickMs` で持っているか。

### crate 分割の着手条件(V6)

ADR-229 は crate の物理分割を「最後」としていた(D4、F-D6、段階表の 6)。理由は「ファイル移動で、ファイルパスを文字列で持つガードが空振りする」。前倒しの条件は、この理由に答えるもの:

1. ガード修正の PR 群がマージされている(#513・#518 は済み、#517 は未マージ)。
2. V3 が済み、`architecture_guard.rs` のファイルパスの文字列リテラルを機械的に付け替えられる。
3. core crate に移す候補が、付録 B の 8 件の扱いを含めて決まっている。

期待する置き換え: windows に依存しない core crate を切り、`CORE_MODULES` の `#[cfg(windows)]` の検出は crate の依存関係に、P5 の `pub` の集合の固定は crate の可視性に置き換える。壁時計・`static`・FS の規則は crate を分けても走査で残る。

### 優先順位の更新

1. **起動時のフォーカス経路**(BUG-102・114・148、起源は BUG-081)。ADR-232 D1(起草、develop 収録、コードは未着手)が承認されたら最優先(実害の記録があるため)。
2. V1・V3。
3. V4・V5 と書き方の明文化を、F3(#517、未マージ)のマージ後の分割に適用する。
4. F6・E1 は、書き方の明文化と V4 を済ませてから。
5. P5 の残りの写しの置き換えは優先度を下げる(根拠 5)。

## 進捗と所有者の判断(2026-10-06 時点、develop `e5567ddb`)

上の各行は書き換えない。現在の状態はこの節が正(上の行の「実装中」「未マージ」などの注記は、この節で読み替える)。

### マージ済み(上の節以降)

| 対象 | PR |
|---|---|
| F1 の追加 | #511 |
| F2 | #504 |
| F3(`relay_plan`) | #517 |
| F4(`drift_plan`) | #514(F4 行の注記〈ハーネスの写しの置き換えは未達〉は、そのまま有効) |
| F5a・F5b | #512・#527 |
| F6(`deferred_gate_plan`) | #523 |
| P4 | #510 |
| `with_clock` のガード・`pub` の固定ガード・ハブの到達不能性ガード | #513・#518・#521 |
| 死んだ variant の削除・凍結コーパスの caller の書き換え | #515・#520 |
| W-c の S22 | #516 |
| ADR-232 S1・S2 | #522・#525 |
| `shadow_send_trace` の撤去 | #524 |
| ログの重複の整理(第 1 段階) | #526 |
| V1 | #530 |
| `plan_relay` の action と reason の統合(案 (ii)) | #531 |

レビュー・修正対応中: F6b(#528)、W-a(#519)、E1(#532)、B5 の試作(#533)、BUG-114 の Windows Terminal 構成(#534)、F5c(#535、`resolve_focus_kind` の前段)。閉じた: F5d(#536、下記)、実 Chrome の測定器(#529)。

### F5 の行への追記: F5d は分けない

**F5d(`learn_imm_capability_on_focus`)は分けない**(#536 で試し、マージせず閉じた)。挙動不変と、BUG-56/107 の汚染条件への影響が無いことは確かめられたが、純増 +127 行で、移った判断は述語 4 つだけだった。評価順(非 Win32 のアプリで `process_name` を評価しない契約)を守るために殻が Win32 の判定を 3 回書き、判断が殻と純粋側に重複して、元の早期 return 4 つより読みにくくなった。BUG-107 を実際に止めているのは殻の 1 行で、Linux のテストでは固定できない。`plan_probe_result` は bool から enum への恒等写像だった。

可読性メモ(`docs/tasks/readability-ideas-study-2026-10-06/`)の「判断を F の分割で分ける」の一般的な効果に反する実例: 述語が少なく、評価順の契約が殻に残る関数は、分ける価値が小さい。

### V1 の実態(#530)

V1 は、PR で `CORE_MODULES` に足した名前が `.cargo/mutants-awase-windows.toml` に載っているかだけを見る(`xtask-adr-evidence core-registry`、CI の `core-registry-consistency` ジョブ、base が develop の `pull_request` のときだけ)。pre-push の正規表現と `fix-requires-evidence.md` の表は V1 では見ない(V2 で人が判断する)。上の V1 の行と同じ。#530 は squash マージで、PR のタイトルは初版の「3 か所」のまま残っている(本文の 2 つ目のコミットで mutants だけに絞った)。V1 を必須チェックにするかは所有者の判断(未決)。

#524 のブランチのコミット `cb581901` の件名「消費者0」は不正確。正しくは `99569f7a` と #524 本文(ADR-242)。

### V4 の根拠の訂正

「F の分割の前に `TickMs` 化が要る」は誤りだった。F6b(#528、レビュー中)の実測で、`output/`・`tsf/` の本番コードに `Instant` は無く(テスト内の 3 か所だけ)、時刻は `current_tick_ms()` の `u64` で持たれていた。V4 の根拠は F4 の 1 件のまま。#528 のマージ後に確定する。

### 所有者の判断(2026-10-06)

1. 凍結コーパスと古い journal は捨ててよい。
2. B4 + B5 を最小形で進める。
3. 10 分の窓を外す。
4. 決定関数の DSL は「小さく試作して読み比べる」を選び、結果は**不採用**(monad-proto、実験ブランチ `experiment/monad-style-decision`)。
5. `plan_relay` は案 (ii) を承認(#531 でマージ済み)。
6. E1・F5・F6 の次の分割を優先する。
7. ADR-232 は S0・S1・S2 を実施する(S1・S2 はマージ済み)。S3 は未着手・未承認。

### 未決・実施中

- コーパスの影響のメモ: 所有者が判断済み(`e5567ddb`。`complexity-budget.md` は未発効のまま据え置く、撤去は新しい再生基盤と同時)。
- 可読性メモ(`docs/tasks/readability-ideas-study-2026-10-06/`)とコーパスの影響のメモ(`docs/tasks/corpus-discard-impact-2026-10-06/`)は develop に入っている。
- B5 の試作: #533 でレビュー・修正対応中。
- BUG-114 の Windows Terminal 構成(#534): CI で修正あり 3/3 PASS・修正なし 3/3 FAIL が出たが、Opus のレビューで「drift 補正を起こしていたのは打鍵ではなく終了処理の `WM_CLOSE` の後の観測」「基準 5(再武装が 0 件)は give-up から停止までがクールダウンより短く、構造上 FAIL になり得ない」と指摘され、修正対応中。**BUG-114 が CI で確かめられたとは、まだ言えない**。

### crate の物理分割: 段階 A の実施(2026-10-10、ブランチ `refactor/fcis-crate-split-prep`)

付録 B のうち 6 件を `CORE_MODULES` へ移した(`ime_event`・`ime_model`・`probe_admission`・`ime_profile_driver`・`key_effect_predictor`・`key_effect_runtime`、`hub_clock` は段階 B-2(PR #568)で実時計を注入にして核へ)。切り出した殻は `win32.rs`(`HwndIdExt`)・`probe_rejection_stats.rs`・`key_effect_io.rs`(crate 直下)。`architecture_guard.rs` に付け替え表 `RELOCATED` を足した(分割前は空)。

段階 B(crate を切る)の前に残る課題:
- ~~`platform_state`(`CORE_MODULES`)が殻の `hub_clock::HubClock`(`Instant::now()`)に依存する。`HubClock` を核へ移すか、時計を trait で受けるかを決める。~~ **済み**(段階 B-2、PR #568。実時計を構築側から注入)。
- `vk`・`tuning` の 5 定数・`focus::class_names`・`FocusKind`・`ime`・`tsf::TsfGate`・`with_app` への依存の扱い。
- ~~ディレクトリを丸ごと走査するガード(`join("src")` 11 + 5 か所)を、走査する場所を複数指定できる形に~~ **済み**(段階 B-1、PR #567。`EXTRA_SRC_CRATES`)。
- doc コメントの `awase_windows::state::...` の例(doctest)の付け替え。

### crate の物理分割: 段階 B-1(ガードの複数ルート化、PR #567)で出た「実際に切る PR の前提条件」(Opus レビュー)

- **P2(必須)**: `lib.rs`・`state/mod.rs` など同じ相対パスのファイルが両 crate にできる。`list_src_files()`（呼び出し 17 か所）→ `read_crate_file` は自 crate 側だけを読むので、核側を見逃すか殻側を二重に数える。`list_rs_files_under` を絶対パス（または `(crate, 相対パス)`）で返し、読む側もそのパスを直接読む形にする。`src_relative_paths_are_unique_across_crates`（B-1 で追加、`lib.rs`/`state/mod.rs` を除く）が重複を検出する。`focus/mod.rs`（`focus/{kinds,class_names,hwnd_cache}` を移すと両 crate にできる）や `vk.rs` も重複しうる。**除外リストに足して黙らせず、先に `list_rs_files_under` の根本修正をする。**
- **P3**: `crate::state::...` を含む needle（例 `architecture_guard.rs` の `"applied: crate::state::AppliedImeState::"`）は、殻のコードが `awase_windows_core::state::...` や再公開名で書くと当たらない。`grep -n '"crate::' crates/awase-windows/tests/*.rs` で洗い出し、新 crate 名の版も数える。
- **P4**: `ungated_state_modules` は殻の `state/mod.rs` しか読まない。核の `mod.rs` も読み、「核にある ungated モジュールが全部 `CORE_MODULES` にあるか」を検査する。
- **ガードの外のパス直書き**: `crates/xtask-adr-evidence/src/core_registry.rs:50`（CI `core-registry-consistency`）、`.cargo/mutants-awase-windows.toml`（`examine_globs`・`-p awase-windows`）、`.githooks/pre-push` の正規表現、`.claude/rules/fix-requires-evidence.md` の表。
- **段階 B で核へ移す範囲（所有者の決定、2026-10-10）**: `tuning.rs` は丸ごと、`vk.rs` は windows 定数との突き合わせ assert と `parse_hotkey` を殻に残して核へ、`journal.rs`+`journal_policy.rs` は核へ（`cfg(windows)` 5 か所を先に調べる）、`focus/{kinds,class_names,hwnd_cache}`・`keymap.rs` も核へ、`hub_clock` は核へ移し実時計は注入。

### crate の物理分割: 段階 B-3(核へ移す候補の準備、2026-10-10)

`state/` の外の移す候補 9 ファイル(`vk.rs`・`tuning.rs`・`keymap.rs`・`journal.rs`・`journal_policy.rs`・`focus/{kinds,class_names,hwnd_cache}.rs`・`tsf/literal_facts.rs`)を、Tier-2 の 4 規則に違反しない状態にした。`layer_boundary_guard.rs` の `CORE_CANDIDATE_FILES`(`core_candidate_files_have_no_tier2_violations`)が移す前から検査する。

- `vk.rs`: 表を `#[macro_export] vk_table!` にし、windows crate の定数との突き合わせと `parse_hotkey`(と `#[cfg(windows)]` のテスト)を殻の `vk_windows.rs` へ。`is_role_candidate` の `OnceLock` は殻の `vk_windows::is_role_candidate_cached` へ(核は素朴版 `is_role_candidate`/`role_candidates`)。
- `journal.rs`: ファイル書き出し(`dump_to_file`・`dump_to_file_for_report`)を殻の `journal_dump.rs`(`JournalDumpExt`)へ、核に `report_json`。`UnifiedJournal::new`/`Default`・`DumpTriggerTracker::new`/`Default`(`quanta::Clock::new()` を読む)を撤去し、時計は構築側(`ImeStateHub::with_clock(clock, journal_clock)`・`bootstrap.rs`)が渡す。`FocusChangedAxes` は `focus/kinds.rs` へ(`focus::FocusChangedAxes`)。
- wall-clock 規則: `quanta::` 全体の禁止を、実時計を作る `quanta::Clock::new`/`Clock::default` の禁止に絞った(注入された `quanta::Clock` 型は許す)。
- `with_clock(` の呼び出し元を固定するガードは `DumpTriggerTracker::with_clock(` を数えない(別の時計)。
- 残り: `ungated_state_modules`、P2(`list_src_files` の根本修正)、実際に crate を切る PR。
- **B1(Opus、PR #569)**: `focus/` と `tsf/` の一部だけを核へ移すと `focus/mod.rs` と `tsf/mod.rs` が両 crate にできる(`vk.rs` も `vk_table!` の `#[macro_export]` で crate 直下に出る)。#567 の P2 が必ず発火する。**P2(`list_src_files` の根本修正)を先に行う**。`#[macro_export]` の `vk_table!` は移動時に呼び出しパス(`crate::vk_table!`)の書き換えか再公開が要る。
- wall-clock 規則は `use quanta` と `::recent(` も禁止する(`use quanta::Clock; Clock::new()` や別名の素通りを防ぐ。核に `use quanta` は無い)。

### crate の物理分割: E2 の方針(所有者の決定、2026-10-10、Opus 案 (d))

crate を切るとき、`state/platform_state.rs`(`ImeStateHub`/`PlatformState`、4,124 行)と `state/sync_actuation.rs`(核のモジュールのうち `ImeStateHub` に依存する唯一のもの、758 行)は**殻の crate(`awase-windows`)に残し**、それ以外の純粋な部品(`state/` の残り・`vk`・`tuning`・`keymap`・`journal`・`journal_policy`・`focus/{kinds,class_names,hwnd_cache}`・`tsf/literal_facts`)だけを `awase-windows-core` へ移す。理由: 記録系(`record_optimistic` など)の `pub(crate)`(INV-A97-1)は、`ImeStateHub` が殻の crate にある限りコンパイラが強制し続ける。(a)(`ImeStateHub` も核へ移して記録系を `pub`+`#[doc(hidden)]`+走査で固定)は、保証がコンパイラから走査に弱まるので採らない。(b)(c) は殻と統合テストがどちらも核の下流の別 crate になるのでコンパイラの保証が増えない。

- 帰結: `platform_state` は `CORE_MODULES`(Tier-2 のファイル単位の検査)に入ったまま、crate 境界とは別に純粋さを検査する。`platform_state` が殻の crate にあるので、`crate::win32::foreground_scope()` を直接読める(PR #571 のフォアグラウンド取得関数の注入と `platform_ctor.rs` は不要になり、#571 は閉じた)。
- 残る準備: `PhysicalKeyDisposition::plan` の拡張トレイト(核の型へ殻の inherent impl を置けないため。別 PR)。
- E2 で核から殻の `platform_state`/`sync_actuation` を参照する向きは無い(`platform_state` が核の下流)。核の `state/mod.rs` は `platform_state`/`sync_actuation` を宣言しない。殻の `state/mod.rs` が核を `pub use` で再公開し、`platform_state`/`sync_actuation`/`ime_decision_view` を足す。

### crate の物理分割: E2(実施、2026-10-10、PR は本ブランチ)

`crates/awase-windows-core`(`awase-windows-core`)を作り、79 ファイルを `git mv` した(`src/` 以下の相対パスは同じ)。`awase-windows` は `pub use awase_windows_core::...` で再公開するので `crate::state::...` のパスは変わらない。`state/platform_state.rs`・`sync_actuation.rs`・`ime_decision_view.rs` は殻に残した(所有者の決定、Opus 案 (d)。記録系の `pub(crate)` をコンパイラが強制し続けるため)。

- 可視性: 殻から使われる `pub(crate)`/`pub(in crate::state)` を `pub` にした(自動修正スクリプトで約 130 か所。`ImeBelief` のフィールドは直接書かず、核の更新口〔`reset_for_panic`・`set_japanese_ime`・`set_prev_conversion_mode`・`apply_eisu_candidate_update`〕に変えた)。
- 孤児規則: `From<(InjectionHint, AppKind)> for InjectionMode` → 関数 `injection_mode_for`。`FocusChangedAxes::any` は核へ。
- テスト専用の口: 核の feature `test-support`(`set_desired_open_for_test`・`FocusGen::new/get`)。殻の dev-dependency で有効にする。核の `ime_set_open_plan` など `cfg(any(windows, test))` のモジュールは常時コンパイルにした。
- ガード: `EXTRA_SRC_CRATES` に核を追加。`ungated_state_modules`(P4)は両 crate の `state/mod.rs` を読む。`core_crate_does_not_depend_on_windows`(核の `Cargo.toml`)。xtask `core-registry` は両 crate の mutants 設定を見る。
- CI: mutants は `pkg × shard` のマトリクス(`.cargo/mutants-awase-windows{,-core}.toml`)。clippy・dylint・Windows の lib テストは `-p awase-windows-core` も対象。`gen_key_effect_table.py` の出力先は核。pre-push の正規表現は両 crate。
- doctest: 核の `awase_windows::...` の例は `awase_windows_core::...` に書き換え、doctest のまま残した。
- 核の unit テスト(1004)は核 crate で、殻のテスト(353)は殻で走る(合計は分割前の 1357 と同じ)。`tests/journals/actuation_decision` は核の `tests/` へ移した。

