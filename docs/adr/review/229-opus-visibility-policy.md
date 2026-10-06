---
id: ADR-229-companion-229-opus-visibility-policy
title: |-
  ADR-229 P4・P5 の可視性の方針(Opus 確定)
type: companion-doc
related_adr:
  - "ADR-229"
---

# P4・P5 の前に決める「可視性の方針」(PR #500 S19 の確定)

- 実測の基準: `origin/develop`(#496〜#500 マージ済み。P3 は未マージ)。読み取りのみ。
- 行番号は `state/platform_state.rs`(develop)と `tests/support/harness.rs`(develop)のもの。

## 0. 結論(推奨を 1 つ)

**方針案 (a) を、次の 3 つの規則つきで採る。**

1. **閉ループのハーネスが呼ぶものだけを `pub` にする**。`#[doc(hidden)]` は付けない。
2. **殻からしか呼ばれない `_in_scope`(とその専用の下位関数)は `#[cfg(any(windows, test))]`**。記録系(`record_*_in_scope`、INV-A97-1)は**絶対に `pub` にしない**。
3. **`pub` にした集合を `architecture_guard` で固定**し、`pub` がなし崩しに増えないようにする。

代案 (b)(ハーネスを `src/` の `#[cfg(test)]` へ移す)は採らない。理由は §3。

あわせて、**P4 のタスク表に書かれていない前提を 2 つ見つけた**(Must。§4)。

---

## 1. P5 の 5 系統が呼ぶ必要のある型・メソッドと、今の可視性

ハーネスは今、`ImeModel` と `IntentStore` を別々に持ち(`harness.rs:194-217` の `model`・`intents`)、`ImeStateHub` の配線を写している。本物の `ImeStateHub` に置き換えると、ハーネスは `ImeStateHub` を 1 つ持ち、次を呼ぶ。

| 系統(写しの場所) | 呼ぶもの(`platform_state.rs`) | 今の可視性 |
|---|---|---|
| 構築・時計 | `ImeStateHub`(型、`:36`)、`with_clock(HubClock)`(P3 で新設)、時計を進める口(`clock` フィールド `:42` は `pub(crate)`) | `pub(crate)` / 未作成 |
| 共通(写しではないが、置き換え後に必要) | `dispatch_event`(`:165`。ハーネスの `reduce` が流す `InitialFocusFenceEstablished`・`InitialFocusHwndEstablished`・`InitialAppPolicyEstablished`・`InputModeObserved`・`FocusChanged`。`harness.rs:260-262, 363, 379`)。観測の書き込み口 `write_imm_cross_probe`(`:1542`)・`write_observer_poll`(`:1403`)・`report_conv_open_inference`(`:1573`)(`harness.rs:354` の `ObserverReported` の 3 ソース)。読み取り `model()`(`:965`。ハーネスの `pub const fn model`(`harness.rs:483`)と `closed_loop_scenarios.rs` の assert が使う)、`desired_open()`(`:894`)、`input_mode()`(`:902`)、`explicit_intent()`(`:266`)、`set_is_japanese_ime`(`:1381`) | すべて `pub(crate)` |
| ① `apply_key_effect_prediction`(`harness.rs:597-612`) | `apply_key_effect_prediction`(`:274`) | `pub(crate)` |
| ② `effective_open_at`(`harness.rs:560-567`) | `effective_open_at`(`:836`) | `pub(crate)` |
| ③ `warrant_context`/`issue_actuation_order`(`harness.rs:614-650`) | `warrant_context`(`:990`)、`issue_actuation_order`(`:1016`) | `pub(crate)` |
| ④ `record_explicit_intent`/`write_*`(`harness.rs:394-406` の `user_set_open`) | `write_set_open_request`(`:1494`)、`record_explicit_intent`(`:1461`) | `pub(crate)` |
| ⑤ `arm/follow_external_change`(`harness.rs:426-475`) | `arm_external_change_watch_in_scope`(`:415`)、`follow_external_change_in_scope`(`:453`) | **private**(P2 で殻を分けた核) |
| (settle 内、runtime の写し。P5 の対象外だが近い) | `desired_is_placeholder`(`:500`)、`align_placeholder_desired`(`:511`)、`check_drift_correction`(`:1038`) | `pub(crate)` |

- シグネチャに現れる型は、どれも既に `pub` の `state::*`(`TickMs`・`ImeEvent`・`ImeModel`・`Prediction`・`WarrantContext`・`ActuationOrder`・`AcceptedObservation`・`HubClock`・`ForegroundScope`(P1))。
- `IntentWitness`(`write_sync_key` の引数)はハーネスで使わないので対象外。
- 付随する改善: 置き換えた後は、ハーネスが制限付きの `ImeEvent` の variant(`ModeKeyPassedThrough`・`KeyEffectPredicted`。`harness.rs:465, 602, 680`)を**自分で組み立てなくなる**。本物の designated 関数(`follow_external_change_in_scope`・`apply_key_effect_prediction`)を通るので、ime_event_guard の意図にも近づく。

---

## 2. `pub` にした場合の増加量と、他の仕組みとの相互作用

### 増加量

- **型 1 つ**(`ImeStateHub`)。
- **メソッド約 20**: 構築と時計で 2(`with_clock`、時計を進める口)、共通で 9(`dispatch_event`、観測の書き込み 3、読み取り 4、`set_is_japanese_ime`)、5 系統で 7、settle の近くで 3(任意)。
- `PlatformState`(`:1770` 付近、既に `pub struct`)は、ハーネスが `ImeStateHub` だけを持てば不要。

### `#[doc(hidden)]`

**付けない**。awase-windows は crates.io に公開しておらず、rustdoc を CI で見ていない。付けても実効は無く、「テストのための口」という意図は doc の 1 行(「閉ループのハーネス `tests/support/harness.rs` からも呼ぶ」)のほうが明確に伝わる。

### 相互作用

| 仕組み | 影響 |
|---|---|
| `docs/layer-boundaries.md` | 可視性の規則は無い(B-1 `with_app`、C-6 の reduce の呼び出しは 1 か所、など)。`dispatch_event` を `pub` にしても、C-6(本番の `.reduce(` は `platform_state.rs` の 1 か所)は変わらない |
| `architecture_guard.rs` | 構築・呼び出しの件数のガードは `src/` だけを走査するので、`tests/` の呼び出しは数えない(今と同じ)。可視性を前提にしたガードは無い(`pub(crate) fn deliver_key_event(` の本体を名前で探すもの `:3256, :4018` は別ファイル)。**ただし `extract_fn_body(production, "pub(crate) fn …")` のように、可視性つきの文字列で `platform_state.rs` の関数を探すガードがあれば、`pub` にした瞬間に空振りする**。P5 の各 PR で、`platform_state.rs` を名前と可視性で探すガードを grep で確認する(今回の実測では見当たらない。**未確認の残り**として成否の判定に入れる) |
| dylint | `ime_event_guard` は「designated 関数の外での構築」を関数名で見る。`actuation_call_guard` は呼び出しを関数名で照合する。どちらも可視性には依存しない。`--tests` を含めずに走るので、ハーネスは対象外(今と同じ) |
| `layer_boundary_guard.rs`(#498) | P4 で `platform_state` が ungated になると、網羅検査が `CORE_MODULES` か `NOT_CORE_MODULES` への分類を要求する(§4 の M13) |
| clippy(windows-build、`-D warnings`、pedantic deny) | `pub` にすると `must_use_candidate` などが効き始める(値を返す `pub fn` に `#[must_use]` が無いと警告。対象は `effective_open_at`・`model`・`desired_open`・`input_mode`・`explicit_intent`・`warrant_context`・`check_drift_correction` など)。P5 の PR で `#[must_use]` を足す必要がある |
| `unreachable_pub` | `state` と `platform_state` はどちらも `pub mod` なので、`pub` の項目は外から到達でき、発火しない |

---

## 3. 代案 (b)(ハーネスを `src/` の `#[cfg(test)]` へ移す)の費用

- **移動量**: `tests/closed_loop_scenarios.rs` 580 行 + `tests/support/{harness 749, pseudo_ime 509, invariants 221, mod 5}` で、約 2,060 行。パスも `awase_windows::state::…` → `crate::state::…` に書き換える。
- **CI**
  - `ci.yml:51` の `--test closed_loop_scenarios` を消す。消さないと、nextest がテストのターゲットを見つけられずに落ちる。
  - `closed-loop-ignored.yml:31`(`cargo test -p awase-windows --test closed_loop_scenarios -- --ignored`)を `--lib` の名前の絞り込みに書き換える。
  - `ci_test_coverage_guard.rs` は `tests/*.rs` が ci.yml に載っているかを見るだけなので、ファイルを消す分には壊れない。
- **ガードとの摩擦(大)**
  - `layer_boundary_guard.rs` の `collect_rs` は、**ファイル名に `test` を含むものだけ**を除外する。`src/…/harness.rs` などの名前のままだと本番として走査され、D-1(VK の 16 進の直書き。シナリオの `0x1D` など)に当たる。
  - `architecture_guard.rs` の `production_code_only` は `#[cfg(test)] mod tests` で切るだけなので、`#[cfg(test)] mod closed_loop;` で宣言した**別ファイルの中身は本番として数えられる**。ハーネスが今は自分で組み立てている `ImeEvent::ModeKeyPassedThrough`/`KeyEffectPredicted` などが、構築の件数のガード(designated 関数に限る)を落とす。
  - P5 で写しを消すまでの間、除外の設定が増える(#500 の M11 で、殻の除外が抜け道になった前例と同じ形)。
- **利点**: 可視性を一切変えずに済む(`dispatch_event` も `pub(crate)` のまま)。
- **判定**: 移動とガードの除外の費用が、`pub` の増加(型 1・メソッド約 20)より大きい。さらに、統合テストという「crate の外から見た契約」の性格が失われる。**採らない**。
- (参考)機能フラグ(`test-support`)+ 自分自身を dev-dependency にして、その時だけ `pub` にする案も検討した。dev-dependency の feature はテストのビルドでしか有効にならないので、Linux の本番ビルド(awase-settings などからの依存)では `_in_scope` が未使用のまま残り、§4 の cfg の問題が解決しない。Cargo の設定が複雑になる割に得るものが少ないので、採らない。

---

## 4. P4 で `_in_scope` が `never used` になる件と、P4 の書かれていない前提

### `_in_scope` の扱い(Q4)

P4 の後、Linux の本番(テストでない)ビルドで未使用になるのは、**殻 `shell.rs`(gated)からしか呼ばれない核**と、その専用の下位関数。

- 殻からだけ呼ばれる根: `arm_mode_key_pass_mark_in_scope`(`:310`)、`mode_key_pass_expiry_wait_ms_in_scope`(`:322`)、`mode_key_pass_window_remaining_ms_in_scope`(`:353`)、`mode_key_pass_mark_live_in_scope`(`:342`)、`expire_mode_key_pass_mark_in_scope`(`:374`)、`invalidate_intents_if_mode_key_pass_live_in_scope`(`:365`)、`external_change_watch_remaining_ms_in_scope`(`:430`)、`align_after_expired_mode_key_pass_in_scope`(`:528`)、`record_optimistic_in_scope`(`:568`)、`record_ime_apply_result_in_scope`(`:1056`)。
- その結果として未使用になる下位関数: `drop_intents_for_mode_key_pass_in_scope`(`:384`、expire と invalidate からだけ呼ばれる)、`record_confirmed_in_scope`(`:579`)、`note_awase_write_for_mode_key_pass_in_scope`(`:335`)。
- 合計で約 13。正確な集合は、P4 の PR の Linux の `test` ログ(`never used` の一覧)で確定する。

**推奨**

- **ハーネスが呼ぶ 2 つ**(`arm_external_change_watch_in_scope`・`follow_external_change_in_scope`)→ `pub`(§1)。
- **残りの約 13**→ `#[cfg(any(windows, test))]`。`platform_state` の中の単体テストが呼ぶもの(`invalidate_…`・`drop_intents_…`・`align_after_expired_…` など)があるので、`cfg(windows)` だけでは足りない。`any(windows, test)` は #498 の規則 3 の例外なので、`platform_state` を `CORE_MODULES` に入れる妨げにもならない。
- **`record_*_in_scope` は `pub` にしない**。`pub` にすると、`RECORDERS`(INV-A97-1/BUG-69)が見ない crate の外から、`applied` を書けるようになる。
- 「全部 `pub` にすれば cfg が要らない」案は採らない。記録系を公開することになり、可視性の目的(書いてよい入口を絞る)に反する。

### P4 のタスク表に無い前提(新しい指摘)

- **M12(Must)**: `platform_state.rs` には `crate::win32::ForegroundScope` の**型のパスが 22 か所**(フィールド `:90`・`:95` 付近、`_in_scope` の引数、`post_bypass` の `ScopedOneShot<…>` ほか)と、`crate::observer::ime_observer::ImeUpdate`(`apply_ime_update` の引数、`:1175` 付近)が残っている。`crate::win32` と `crate::observer` は Linux では存在しない(gated)ので、**P4 で gate を外すとこれらがコンパイルエラーになる**。P1 で型は `crate::state::foreground_scope::ForegroundScope`・`crate::state::ime_update::ImeUpdate` に移したので、P4 の PR(または直前の小さな PR)でパスを書き換える。殻 `shell.rs` は gated なので、`crate::win32::foreground_scope()` のままでよい。テストの `crate::win32::foreground_scope()` の直接呼び出し 9 か所は、P4 のタスク表に既に書かれている。
- **M13(Must)**: P4 で `platform_state` が `state/mod.rs` の ungated な `mod` になると、#498 の `core_modules_classify_every_ungated_state_module` が、`CORE_MODULES` か `NOT_CORE_MODULES` への分類を要求する(しないと落ちる)。今の `platform_state.rs` の本番コードで Tier-2 の規則に当たりうるのは、`#[cfg(windows)] mod shell;`(例外。通る)と、P3 で入る **`#[cfg(windows)] fn new()`(規則 3 に当たる)**。
  - **推奨**: P3 で `new()` を `platform_state.rs` ではなく殻 `shell.rs` の側に置く。そうすれば `platform_state.rs` は違反 0 になり、P4 で `CORE_MODULES` に入れられる。この場合、`architecture_guard.rs:1382` が `HubClock::wall(crate::hook::current_tick_ms)` を探すファイルを `platform_state.rs` から `platform_state/shell.rs` へ付け替える(1 行。ガードの意味は不変)。
  - `new()` を `platform_state.rs` に置くなら、P4 で `NOT_CORE_MODULES` に理由つきで入れる。
  - どちらにするかを、P3 のタスク表に書く。

---

## 5. タスク表に書き込む形

### P3 の行への追記

- `ImeStateHub::new()`(`HubClock::wall(crate::hook::current_tick_ms)`)は、**殻 `state/platform_state/shell.rs` に置く**(`#[cfg(windows)]` の項目を `platform_state.rs` に残さない。M13)。`architecture_guard.rs:1382` の対象ファイルを `shell.rs` に付け替える。
- ungated な `with_clock(clock: HubClock)` を `platform_state.rs` に置く(P5 で `pub` にする前提)。

### P4 の行への追記

- **前提の追加**: `platform_state.rs` の `crate::win32::ForegroundScope`(22 か所)→ `crate::state::foreground_scope::ForegroundScope`、`crate::observer::ime_observer::ImeUpdate` → `crate::state::ime_update::ImeUpdate`(M12)。
- **可視性**: 殻からしか呼ばれない `_in_scope` とその専用の下位関数(約 13。`record_*_in_scope` を含む)に `#[cfg(any(windows, test))]`。`allow(dead_code)` は使わない。`pub` にはまだしない(P5 で必要なものだけ)。
- **#498 の分類**: `platform_state` を `CORE_MODULES` に足す(P3 で `new()` を殻に置いた場合)。置かなかった場合は `NOT_CORE_MODULES` に理由つきで足す(M13)。
- **成否の判定**(既存に追加): Linux の `test` ログの `never used`/`never read` が 0。`layer_boundary_guard` の網羅検査が green。
- **取りやめ条件**(既存に追加): `#[cfg(any(windows, test))]` で消えない未使用が残り、`allow` を足さないと消えない警告が 3 件を超える。

### P5 の行への追記

- **前提**: 可視性の方針は (a)。代案 (b) は採らない(§3)。
- **`pub` にするもの**(写しを 1 系統置き換えるたびに、その系統が要るものだけを `pub` にする)
  - 型 `ImeStateHub`
  - `with_clock`、時計を進める口
  - `dispatch_event`
  - `write_imm_cross_probe`・`write_observer_poll`・`report_conv_open_inference`
  - `model`・`desired_open`・`input_mode`・`explicit_intent`・`set_is_japanese_ime`
  - 各系統: ① `apply_key_effect_prediction`、② `effective_open_at`、③ `warrant_context`・`issue_actuation_order`、④ `write_set_open_request`・`record_explicit_intent`、⑤ `arm_external_change_watch_in_scope`・`follow_external_change_in_scope`
  - 各メソッドの doc に「閉ループのハーネス(`tests/support/harness.rs`)からも呼ぶ。本番の呼び出し元は crate 内だけ」と 1 行書く。`#[doc(hidden)]` は付けない。値を返すものには `#[must_use]` を付ける(clippy pedantic)。
- **`pub(crate)`/private のまま**: `record_optimistic_in_scope`・`record_confirmed_in_scope`・`record_ime_apply_result_in_scope`(INV-A97-1)、殻からしか呼ばれない `_in_scope`、その他のメソッド。
- **`pub` の集合の固定(S20)**: `architecture_guard` に、`platform_state.rs` の `pub fn`(`pub(crate)` を除く)の名前の集合が期待の一覧と一致することを確かめるテストを 1 本足す。P5 の各 PR で一覧に足す。記録系の名前が `pub fn` に現れたら失敗する。
- **成否の判定**: `closed_loop_scenarios` の全シナリオの結果が不変(既存)。windows-build の clippy `-D warnings` が green(`must_use_candidate` など)。`platform_state.rs` を可視性つきの文字列(`"pub(crate) fn …"`)で探すガードが空振りしていないことを、PR ごとに grep で確認する。
- **取りやめ条件**: 1 系統の置き換えのために、記録系(`record_*_in_scope`)を `pub` にする必要が出た場合。そのときは止めて、その系統を写しのまま残すか、(b) を再検討する。

---

## 6. 指摘の一覧

| ID | 重大度 | 内容 |
|---|---|---|
| M12 | Must | P4 で `platform_state.rs` の `crate::win32::ForegroundScope`(22 か所)と `crate::observer::ime_observer::ImeUpdate` のパスを書き換える前提が、タスク表に無い(Linux でコンパイルエラーになる) |
| M13 | Must | P4 で `platform_state` が ungated になると、#498 の網羅検査が分類を要求する。P3 の `#[cfg(windows)] new()` の置き場所(殻に置けば `CORE_MODULES`、`platform_state.rs` に置けば `NOT_CORE_MODULES`)を P3 で決める |
| S20 | Should | `pub` の集合を `architecture_guard` で固定し、`pub` がなし崩しに増えることと、記録系の公開を防ぐ |
| S21 | Should | P5 で `pub` にするメソッドに `#[must_use]` などを足す(windows-build の clippy pedantic) |
| N25 | Nit | タスク表の方針案の「`_in_scope` 版・読み取りメソッドを `pub` にする」は、「ハーネスが呼ぶ 2 つの `_in_scope` だけを `pub` に、残りは `#[cfg(any(windows, test))]`」に直す(§4) |
