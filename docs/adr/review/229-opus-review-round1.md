---
id: ADR-229-companion-229-opus-review-round1
title: |-
  ADR-229 opus-adversarial-consult round1(最初の着手単位の見定め)
type: companion-doc
related_adr:
  - "ADR-229"
  - "ADR-224"
---

# ADR-229 の最初の着手単位の見定め(1ラウンド)

- 対象: develop `4d287cb1`(依頼時の `964ee004` の 1 つ先。src の差分なし)。読み取りのみ。ビルド・テスト・git 操作はしていない。
- 「未確認」と書いたものはコンパイルしていない推測。

## 0. 結論

**最初の 1 本は `tsf/tsf_gate.rs` の gate 解除(候補 T2)。** `tsf/mod.rs` の 2 行を変えるだけ。

- 依存はゼロ: `crate::` 参照なし。使うのは std / `timed_fsm` / `awase::types` だけ。
- テストが 19 本、新たに Linux で回る。
- ADR-229「先に確かめること」1(ungate 後にコンパイルが通るか)を、最小の差分で実地に確かめられる。
- 前例として `tsf/gji_fsm.rs` が同じ形(ungated の親モジュールの下で、子モジュールを個別に gate する)で既に ungate されている(ADR-082)。

続く順番は次のとおり。

1. **T1**: `runtime/transport.rs::plan_tests`(30 本)を ungated の `state/physical_disposition.rs` に移す。本番コードは不変で、再発ファミリー(配送判断、BUG-116/131/ADR-166)の決定表が初めて Linux で回る。**価値は T1 が最大**。T2 を先にするのは、ADR-229 の前提確認として最小だからにすぎない。
2. **T3 + T5**: `ime_event_log` の gate 解除と、空の `tsf/send.rs` の削除。
3. **T4**: journal の 3 点修正と gate 解除。
4. **T7**: hwnd_cache に時刻を引数で渡すようにしてから gate を外す。
5. ここで段階 2(platform_state)を測り直す。

### 新しく分かった最重要の事実(ADR-224・ADR-229 の前提を変える)

**同じ場所で gate を外すだけなら、テキスト走査のガードは 1 件も壊れない。**

- `tests/architecture_guard.rs`・`layer_boundary_guard.rs`・`ci_test_coverage_guard.rs` に出てくる `cfg(windows)` は、コメントの 4 か所だけ(`layer_boundary_guard.rs:8`、`architecture_guard.rs:1293, 4856, 5868`)。
- ガードは `read_crate_file("src/…")`/`collect_rs` でファイルをテキストとして読む。gate の有無は結果に影響しない。
- dylint は `--target x86_64-pc-windows-msvc` で回る(`ci.yml:175-179`)ので、Windows 側のコンパイル内容が変わらない限り影響を受けない。
- 壊れるのは次の 2 つだけ。
  - **ファイルや関数を移したとき**(D4 の論旨は正しい)。
  - **ガードが固定している文字列そのものを変えたとき**。例: `architecture_guard.rs:1382` の `HubClock::wall(crate::hook::current_tick_ms)`。段階 2 で `ImeStateHub::new(tick_fn, ..)` にすると必ず落ちる。

---

## 1. 着手単位の候補と順位

| 順位 | ID | 内容 | 新たに Linux で回るテスト | 本番の挙動 | 規模 |
|---|---|---|---|---|---|
| 1 | **T2** | `tsf/tsf_gate.rs` の gate 解除 | 19 | 不変 | 1 ファイル・2〜6 行 |
| 2 | **T1** | `runtime/transport.rs` の `plan_tests` を `state/physical_disposition.rs` へ移し、`plan_core` で検査 | 30 | 不変(テストの移動のみ) | 2 ファイル・移動約 1,075 行 + 補助数行 |
| 3 | T3 | `state/ime_event_log.rs` の gate 解除 | 5 | 不変 | 1 ファイル・1 行 |
| 4 | T5 | 空の `tsf/send.rs` と `pub mod send;` の削除 | 0 | 不変 | 2 ファイル・−6 行 |
| 5 | T4 | `journal.rs` の gate 解除(`SentKeyEvent` を移す + dump の 2 関数を `#[cfg(windows)]`) | 22 | 不変 | 3〜4 ファイル・数十行 |
| 6 | T7 | `focus/hwnd_cache.rs` の `save`/`restore` に `now_ms` を渡す → gate 解除 → 期限切れのテストを追加 | 0 → 新規 3〜5 | 不変(時刻を読む位置が呼び出し元へ数 µs ずれるだけ) | 3 ファイル・数十行 |
| 7 | T6 | `tsf/probe.rs` の `evidence_is_fresh` の 3 重複(`:726`, `:805`, `:735` 相当の `evidence_now`)を private 関数 1 つに | 0(Windows CI の既存 16 本で検証) | 不変 | 1 ファイル・±10 行 |

### T2: `tsf/tsf_gate.rs` の gate 解除

- **(a) Windows 依存**
  - `use` は `std::time::Duration`、`timed_fsm::{Response, TimedStateMachine, GateAction, HoldingGate}`、`awase::types::RawKeyEvent` だけ(`:50-55`)。
  - 本体に `crate::` 参照は 0。テストは `use super::*; use timed_fsm::TimerCommand;` だけ(`:372-373`)。
  - **推移的にも依存なし(確認済み)**。
  - 変更は `tsf/mod.rs:47` の `#[cfg(windows)] pub(crate) mod tsf_gate;` と、`:54-57` の `#[cfg(windows)] pub use tsf_gate::{…}` の 2 か所の gate を外すこと。
  - 同ファイル冒頭の doc(`:13`「gji_fsm 以外の全サブモジュールは windows crate に依存」)と `:20` の「唯一の ungated モジュール」は古くなるので直す。`literal_facts` は既に ungated なので、この 2 か所は今も既に不正確。
- **(b) 壊れるガード**: 0。`architecture_guard.rs:1044` の言及はコメントだけ。`.githooks/pre-push:36` の正規表現は `tsf/` に当たるので**警告が出る**(ブロックはしない)。`layer_boundary_guard` と dylint には影響なし。
- **(c) 新たに回るテスト**: 19 本(`scenario_a1_focus_change_to_pending_warmup` ほか)。中身は純粋な状態機械と `TimerCommand` の検査で、Windows 固有の前提は無い(読んだ範囲)。
- **(d) dead_code / clippy**
  - 公開されている項目は全部 `pub` で、`pub mod tsf` から `pub use` で再公開される。Linux でも crate の公開 API に乗るので、dead_code は出ない見込み(**未確認**)。private は `HELD_MAX` だけで、内部から使われている。
  - Linux の clippy ジョブは `cargo clippy --lib`(`ci.yml:127`)で、ワークスペースに `default-members` が無いのでルートの `awase` だけが対象。awase-windows の Linux 側の警告で CI は落ちない。
  - Windows の clippy(`ci.yml:302`、`-D warnings`)は、Windows でのコンパイル内容が変わらないので影響なし。
- **(e) 再発ファミリー**: warmup / cold-start(`tsf/`)の行に触れるが、gate を外すだけの純粋なリファクタで、ロジックは不変。テストを足す側の変更なので (a) の精神は満たす。pre-push の警告はそのまま push してよい。
- **(f) CI**
  - 成否は Linux の `test` ジョブ `cargo nextest run --workspace --lib`(`ci.yml:46`)で、`tsf::tsf_gate::tests::*` が 19 本、新たに列挙されて pass すること。
  - Windows 側では `windows-cross-check`(`cargo xwin build --tests`、`:111`)と `windows-build` の clippy・lib テスト(`:302`, `:349`)が green のまま。
- **(g) 取りやめと戻し方**: Linux でコンパイルエラーが出たら取りやめ。原因の推移的な依存を記録して `git revert`。2 行なので戻すのは容易。

### T1: `plan_tests` を ungated 側へ移す

- **(a) Windows 依存**
  - `plan_tests`(`runtime/transport.rs:138-1211`、30 本)が使うのは `crate::vk::*`(ungated)、`PhysicalKeyDisposition::plan`、`ActiveImeKind`(`tsf/observer.rs:710`、gated)だけ。
  - `plan` は `Self::plan_core(event, profile, shadow_toggled, active_ime_kind.into())` 1 行の殻(`:128-136`)。`plan_core` は ungated の `state/physical_disposition.rs:106`。
  - テストの中の `plan(.., ActiveImeKind::GoogleJapaneseInput/MicrosoftIme)` を `plan_core(.., ImeKindId::Gji/MsIme)` に置き換えれば(写像は `tsf/observer.rs:720-727` の `From` と同じ)、gated への依存は 0 になる。
  - `transport.rs` には殻の配線を見る Windows 専用のテストを 1 本残す。
  - runtime/ は `lib.rs:83` でモジュールごと gate されているので、transport.rs 自体の gate は外せない。だからファイルを ungate するのではなく、テストを移す。
- **(b) 壊れるガード: 0(確認済み)**
  - `input_relay_profile_wiring_occurrence_counts_are_pinned`(`architecture_guard.rs:4214-4259`)は `strip_any_test_module` を使い、`physical_disposition.rs` の期待値は 2。`#[cfg(test)] mod …` の後ろは数えないので、テストを足しても変わらない。
  - `bug116_shift_katakana_guards_are_present_in_production_code`(`:4858-`)も `strip_any_test_module` で、トークンの有無を見るだけ。
  - `:892` は `production_code_only` + `extract_fn_body` で、関数の本体しか見ない。
  - **移す先のモジュール名は `mod tests` にする**(`production_code_only` が切り落とすのは `mod tests` だけ。`:130-145`)。
  - コメントで古くなる箇所: `architecture_guard.rs:4175-4180, 4855-4857`、CLAUDE.md の「`runtime/transport.rs::plan_tests` は Linux に存在しない」の例示。
- **(c) 新たに回るテスト**: 30 本。ADR-166 の全数決定表、M-2 非対称の固定、BUG-116 の Shift+かな を含む。`plan_core` 内の `debug_assert!`(ADR-166 の status にある、injected なら shadow_toggled の件)は Linux の debug ビルドでも同じに効く。
- **(d)**: 新しい dead_code は無い(`plan_core` は ungated の `explicit_press` からも使われている)。
- **(e) 再発ファミリー**: 「物理 IME キーの Suppress/Allow 配送判断」の行に当たる。本番コードは不変で、テストが Linux CI でも回るようになるので回帰網は強くなる。pre-push は `runtime/transport` で警告を出す(`crates/awase-windows/tests/` に差分が無いため)。そのまま push してよい。
- **(f) CI**: Linux `test` ジョブで `state::physical_disposition::tests::*` が 30 本 pass。`windows-build` の lib テストでは同じ 30 本が場所を変えて pass。`git diff --color-moved` で、移動以外の差分が `plan`→`plan_core` の置き換えと補助関数だけであることを確認できる。
- **(g) 取りやめ条件**: 1 本でも Linux で落ちたら止める。殻と core が等価でない、つまり本番の差の発見なので、BUG として扱う。戻すときは revert。
- **後続(この PR ではやらない)**
  - `bug116_shift_katakana_guards…` の doc は「Linux CI 側の唯一の防波堤」を理由に挙げている(`:4855-4857`)。T1 の後はこの理由が消えるので、削除か縮小を検討できる(ガードの撤去 = 複雑性の減少)。
  - `.cargo/mutants-awase-windows.toml` の `examine_globs` に `state/physical_disposition.rs` が無い(grep で確認)。足せば変異テストの対象になる。

### T3: `state/ime_event_log.rs` の gate 解除

- (a) `use` は `VecDeque`、`Instant`、`super::ime_event`、`super::TickMs` だけ(`:6-10`)。依存なし(確認済み)。変更は `state/mod.rs:202` の 1 行。
- (b) ガードの参照は 0(grep で確認)。
- (c) 5 本。`Instant` を使うが、テストは容量やリングの挙動の検査(**未確認**だが、壁時計の値を assert していない見込み)。
- (d) `pub mod` の `pub` 項目なので dead_code は出ない見込み(未確認)。
- (e) 該当なし。
- (f)(g) は T2 と同じ。
- **ADR-224 との関係**: ADR-224 は案A の対象を「`ImeStateHub`/`journal`/`ime_event_log` の ungate」と名指ししている。T3 と T4 は**形式上は案A の一部**になるので、ADR-224 に一言追記して矛盾を消す(§4-9)。

### T5: 空の `tsf/send.rs` の削除

- 中身は doc コメント 4 行だけ(確認済み)。`tsf/mod.rs:38-39` の `#[cfg(windows)] pub mod send;` も消す。
- ガードと pre-push への影響: pre-push は `tsf/` で警告を出すだけ。
- docs の参照(`docs/tasks/actuation-inventory-2026-09-30.md:34` の `tsf/send.rs:27 send_eager_warmup_vk_pair`、known-bugs の過去の記述)は**既に古い**ので、直す必要は無い。
- CI: `windows-cross-check` / `windows-build`。T3 と同じ PR にまとめてよい。

### T4: journal.rs の gate 解除

- **(a) Windows 依存**: 推移的に追うと、gated なのは次の 3 点だけ(確認済み)。README の 3 点修正は正しい。
  - `crate::win32::SentKeyEvent`(`:216-217`、テスト `:2062`)
  - `crate::hook::current_tick_ms()`(`:1512`, `:1532`。dump のファイル名に使う tick)

  その他の参照(`journal_policy`、`state::ime_event`/`ime_actuation`/`ime_actuation_decision`/`event_origin`/`conv_classify`/`actuation_decision_record`、`tsf::literal_facts`、`focus::current`、`vk`)は全部 ungated。`quanta` は無条件の依存(`Cargo.toml`)。
- **やり方(挙動不変)**
  - `SentKeyEvent`(`win32.rs:262`、`u16`/`bool`/`usize` だけの POD)を ungated のモジュールへ移し、`win32.rs` で `pub use` して既存のパスを保つ。
  - `dump_to_file`/`dump_to_file_for_report` には `#[cfg(windows)]` を付ける。使うのは Windows 側だけ(`runtime/message_handlers.rs:1225`)。時刻の出所を変えると dump のファイル名の系が変わるので、注入はしない。
- **(b) 壊れるガード**: 0。`architecture_guard.rs:5165-5221` は `journal.rs` のテキストを読むだけ。`drift_correction_replay.rs`/`journal_replay.rs` の doc に「journal は `#[cfg(windows)]`」という記述があり、これは古くなる。
- **(c) 新たに回るテスト**: 22 本(`DumpTriggerTracker` は `quanta::Clock::mock()`、`SentKeyEventSummary` の変換など)。テストは dump 関数を呼ばない(確認済み)。
- **(d)**: dump 関数だけが使う private な補助関数があれば、Linux で dead_code が出る(**未確認**)。出たら同じく `#[cfg(windows)]` を付ける。
- **(e)**: 該当なし。
- **(f)**: Linux `test` ジョブで 22 本。
- **(g)**: dead_code の対処に `allow` を足す必要が 3 か所を超えたら止める。指標(`allow(dead_code)` の減少)と逆行するため。
- **効果**: replay テストが、写しの型ではなく実際の `JournalEntry` を Linux で読めるようになる。前回の私のレビューの A′/B′(journal から判断を再生する)の土台。段階 2 の前提の 1 つも満たす。

### T7: hwnd_cache に時刻を渡してから gate を外す

- (a) `crate::hook::current_tick_ms()` を 2 か所で読む(`:59`, `:85`)。**README と ADR-229 の「Win トークンの無い gated ファイル」に入っているが、推移的には `GetTickCount64`(`hook.rs:801-805`)に依存する**。`save(.., now_ms)`/`restore(.., now_ms)` にして、呼び出し元の `focus/tracker.rs` で読めば gate を外せる。呼び出し元は tracker の 1 ファイル(確認済み)。
- (b) ガードの参照は `hwnd_cache` という文字列のみで、`architecture_guard.rs:2517` は `PerSourceObservations` のフィールド名で別物。壊れない。
- (c) 現在のテストは 0。期限切れ・retain・hwnd 一致のテストを新しく足せる(Linux)。
- (e) **focus 遷移ファミリー**に入り、BUG-128 / ADR-165(キャッシュ復元)の領域。ロジックは不変だが、テストの追加を同じ PR に含めることを条件にする。
- (f) Linux `test`、`windows-build`。
- (g) `tracker.rs` 以外の呼び出し元が見つかったら止める。
- **効果**: 段階 2 の前提(`HwndImeSnapshot`、`platform_state.rs:1247, 3201`)を 1 つ片付ける。

### T6: `evidence_is_fresh` の重複を 1 つにまとめる

- `tsf/probe.rs` は gated のまま(`crate::hook`、`crate::tsf::observer::TSF_OBS`、`crate::output` に依存)。`!fencing_active || last_write_ms >= self.epoch_send_ms` という同じ式が 3 か所にある(`:726`、`:745` 付近の `evidence_now` の `evidence_fresh:`、`:805`)。
- warmup ファミリーで、BUG-75 と Opus 指摘3の経緯がある式。純粋な抽出で、既存の Windows テスト(`check_now_confirms_when_write_evidence_is_fresh` ほか、計 16 本)で検証できる。
- 価値は小さい。手が空いたときでよい。

---

## 2. 今は着手しないもの(理由つき)

| 対象 | 理由 |
|---|---|
| 段階 1(時計の注入を一括で) | `hook::current_tick_ms()` の直接呼び出しが **151 か所・34 ファイル**、`Instant::now()` が **158 か所**、`now_timestamp_us()` が 12 か所。「機械的」ではあるが小さくない。読む位置を 1 ターンの入口へ寄せると鮮度が変わり(ADR-229 自身が 10ms と 20ms の epoch fence を挙げている)、tuning-constants の領域に触れる。必要な所だけ個別にやる(T7 の形) |
| 段階 2(`platform_state.rs` の ungate) | 前提がまだ揃っていない(§4-2)。`foreground_scope()` の 11 か所(`platform_state.rs:308` ほか)を注入に変えると、ガード `architecture_guard.rs:1382` が必ず落ちる。IME belief ファミリーで、ADR-224 の「案C が先」という順番とも衝突する。T4・T7 の後に、壊れるガードを実測してから決める |
| S-A の型をまとめて切り出す(`ColdReason`・`DetectionResult`・`ImeModeState`・`ImeUpdate`・`InjectionHint`・マーカー定数など) | 型だけを移しても ungated 側に使い手が無く、Linux のテストは 1 本も増えない。使い手(journal → `SentKeyEvent`、platform_state → `ForegroundScope`/`ImeUpdate`/`HwndImeSnapshot`)を ungate する PR で、必要になった時点で移す |
| `focus/tracker.rs`・`observer/gji_observer.rs` の gate 解除 | 推移的に gated へ依存する。tracker は `focus::classifier`(gated)・`focus::uia::SendableHwnd`(`:12-17`)、gji_observer は `crate::hook::current_tick_ms`・`crate::tsf::observer::tsf_obs`(`:32-37`)。gji_observer は gated な `observer/` モジュールの子でもある |
| `focus/uia.rs` の UIA 経路の削除 | 挙動が変わる(COM ワーカーとプロセス間の UIA 呼び出しが止まる)。CI だけでは成否が決まらない。focus ファミリー。ガード `uia_async_focus_kind_handler_does_not_write_belief`(`architecture_guard.rs:1795`)と `:5085` の一覧、`SendableHwnd` を使う 5 か所(`platform.rs:1359`、`tracker.rs:17`、`focus_tracking.rs:104, 868`、`runtime/mod.rs:2071`、`bootstrap.rs:1317`)に波及する。ADR-229 の未決定5(所有者判断)のまま |
| `MechanismCommand` の `unreachable!` variant を部分型に分ける、`decide_attempt` の二重呼び出し、`ConvAfterOpen` の重複型 | `MechanismCommand` は `serde::Serialize/Deserialize` で(`ime_actuation_decision.rs:107-110`)、凍結コーパス `tests/journals/actuation_decision/*.json` に 35 件の `"command"` が入っている。型を分けると replay の互換性に響き、「小さな撤去」ではない。actuation 合流点ファミリーでもある |
| crate の物理分割(段階 6)、フックの薄型化(H) | ADR-229 のとおり最後。 |

---

## 3. 着手単位ごとに壊れるガード(まとめ、ADR-224 の未実測点への回答)

| 単位 | `architecture_guard` | `layer_boundary_guard` | `ci_test_coverage_guard` | dylint | `.githooks/pre-push` | mutants 設定 |
|---|---|---|---|---|---|---|
| T2 | 0(`:1044` はコメント) | 0 | 0 | 0(Windows ターゲット) | 警告(`tsf/`) | 0(足せば対象を拡大) |
| T1 | 0(`strip_any_test_module`/`production_code_only` の挙動を確認済み。`mod tests` という名前が条件) | 0 | 0 | 0 | 警告(`runtime/transport`) | 0(`physical_disposition.rs` を足すと良い) |
| T3 | 0 | 0 | 0 | 0 | 0 | 0 |
| T4 | 0(`:5165-5221` はテキストを読むだけ) | 0 | 0 | 0 | 0(`journal.rs` は正規表現に無い) | 0 |
| T7 | 0 | 0 | 0 | 0 | 警告(`focus/`) | 0 |
| 段階 2(参考) | **少なくとも 1**(`:1382` `HubClock::wall(crate::hook::current_tick_ms)` の件数固定)。`:1293` 付近は「gated だから」を理由にしたガードで、ungate 後は本物のテストへの置き換え候補(撤去の機会) | 0(`c6_single_reduce_call_site` は `platform_state.rs` の名前を見るだけ) | 0 | 0 | 警告(`state/platform_state`) | `examine_globs` の見直し |

---

## 4. ADR-229 本文の事実の誤り・書きすぎ

1. **段階 0 の「Win トークンの無い gated ファイル(`tsf_gate`・`hwnd_cache`・`tracker`・`gji_observer`・`ime_event_log`)の gate 解除」**: 単独で外せるのは `tsf_gate` と `ime_event_log` だけ。`hwnd_cache` は `hook::current_tick_ms` に、`tracker` は `focus::classifier`/`focus::uia`/`hwnd_cache` に、`gji_observer` は `hook`/`tsf::observer` に推移的に依存する。「`windows::` の字面が無い」と「gate を外せる」は別物。
2. **「`platform_state.rs` の本体 1,813 行に Win32 API 呼び出しも `unsafe` も `with_app` も無い」**: 字面としては正しい。ただし `crate::win32::foreground_scope()`(内部で `GetForegroundWindow`、`win32.rs:63-77`)を 11 回、`HubClock::wall(crate::hook::current_tick_ms)`(`GetTickCount64`)を 1 回呼び、gated な型 `ForegroundScope`/`ImeUpdate`/`HwndImeSnapshot`/`journal` を使う。README §4 の「前提 5 つ」には **`focus::hwnd_cache::HwndImeSnapshot`(`:1247`)が抜けている**。
3. **段階 2 の指標「ハーネスの写し 7 → 1」(README は「7 系統のうち 6 が本物の呼び出しになる」)**: 写し 7 系統(`harness.rs:12-26`)のうち `kp_stage_key_effect_track`/`kp_predict_key_effect`(`runtime/key_pipeline.rs`)と `ir_apply_drift_correction` の前半(`runtime/ime_refresh.rs`)は runtime/ にあり、platform_state を ungate しても残る。**現実的には 7 → 2**。
4. **「gate 内のテストは数えられた分だけで約 180 本」**: gated なファイルの `#[test]` をざっと数えると **約 354 本**(platform_state 48、transport 30、journal 22、tsf_gate 19、warmup 39、tsf/probe 16 ほか。Windows 専用のものを含む粗い数)。README は「数えられた分だけ」と断っているが、ADR の基準値として使うなら過小。
5. **「`cfg(windows)` が src に 190 か所」**: 私の数え方では、`#[cfg(windows)]`/`#![cfg(windows)]` の属性が 172、`cfg(…windows…)` を含む行が 184。数え方を本文に書くこと。`not(windows)` の `allow(dead_code)` は 45 + その他の `cfg_attr(not(windows))` が 1 = 46(11 ファイル)で一致。
6. **ガードの書き換え量**(ADR-224 の「`cfg(windows)` 前提のテキスト走査をしており、広く壊れる」、ADR-229 の「未実測」): §0 と §3 のとおり、その場で gate を外すだけならガードは壊れない。ADR-224 の案A の費用見積もりのうち、ガードの部分は根拠が無い(`foreground_scope` を stub にすると挙動が隠れる、という懸念は別で、そちらは有効)。費用が出るのはファイルの移動と固定文字列の変更(`:1382`)だけ。
7. **「`architecture_guard.rs` はファイルパスを約 70 件直書き」・「約 6,000 行」**
   - パスの文字列リテラルは異なりで 76(うち `"src/…rs"` 形式が 45)、出現数は 193 以上。
   - ファイルは 5,993 行、`#[test]` は 117 本。
   - 「ガード約 70 件」と読める言い方は誤り(テストは 117 本、`layer_boundary_guard` が 8 本)。
8. **D2 の B「IMM32/MSAA/UIA のプロセス間呼び出し。今は `run_with_timeout`」**
   - IMM32 の `imm::send_ime_control`(`SendMessageTimeoutW`)は、同期経路では**エンジンスレッドでそのままブロックしうる**。最初の約 5 秒のブロックは防げず、再発だけを止めるサーキットブレーカがある(`send_health.rs` の doc、BUG-34)。B を「タイムアウトを値で返す」形にすると、これらの経路では挙動が変わる(型を変えるだけでは済まない)。
   - S の「`SendInput` は即時に返る」は**未確認**。LL フックの連鎖の影響を実測した記録が見当たらない。
9. **ADR-224 との整合**: T3/T4(`ime_event_log`/`journal`)と段階 2 は、ADR-224 が名指しした案A そのもの。ADR-224 は「実例が出るまで着手しない、出たら案C が先」と決めている。ADR-229 の status は「所有者の判断が要る」としているが、**段階 0 にも案A の一部が入っていることは書いていない**。ADR-224 に「ガードの費用見積もりは根拠が無かった(§3)」と「journal・ime_event_log の ungate は ADR-229 段階 0 で扱う」を追記して、矛盾を解いておくこと。
10. **「`tsf/probe.rs` は本体 818 行に `windows::` も `unsafe` も無い」を gate の粒度が粗い根拠に使っている点**: probe.rs は `crate::hook::current_tick_ms`・`crate::tsf::observer`(`TSF_OBS` のグローバル)・`crate::output` に依存し、S-B/S-E を済ませないと外せない。根拠としては書きすぎ。
11. 撤去候補「`MechanismCommand` の `unreachable!` 2 variant(部分型に分ける)」を「小」としている点: コーパスとの互換性があるので小さくない(§2)。

**Blocker**: 無し。T2/T1/T3/T5 は ADR-229 の承認を待たずに単独で入れても、ADR-229 が取りやめになった場合に戻す必要が無い種類の変更。テストの追加と gate の縮小だけなので、「コードに手を付けない」という status の制約をどう扱うかだけ、所有者に確認すること。

---

## 5. 推奨(1 つに絞る)

**最初の 1 本は T2(`tsf/tsf_gate.rs` の gate 解除)。**

1. 変更: `tsf/mod.rs:47` と `:54` の `#[cfg(windows)]` を外し、`:13` と `:20` の古い doc を直す。必要なら `.cargo/mutants-awase-windows.toml` の `examine_globs` に `tsf/tsf_gate.rs` を追加する(別 PR でもよい)。
2. 成功の判定
   - Linux の `test` ジョブ(`cargo nextest run --workspace --lib`)のログに `tsf::tsf_gate::tests::` が 19 本出て、すべて pass。
   - `windows-cross-check` と `windows-build`(clippy `-D warnings`、lib テスト)が green。
   - Linux のビルドログに `tsf_gate.rs` 由来の新しい警告が無い。
3. 取りやめの判定: Linux でコンパイルが通らない、または `allow(dead_code)` を足さないと警告が消えない。この場合は revert し、ADR-229 の「先に確かめること 1」に結果を書く。
4. 次に T1(`plan_tests` の移動、+30 本)→ T3+T5 → T4 → T7 の順。T4 と T7 が済んだら、段階 2 のガードの破損(少なくとも `:1382`)と、写し 7→2 という現実的な効果を測り直してから、ADR-224 と合わせて所有者が判断する。
