# ADR-274 Opus レビュー round 2(2026-10-20)

対象: `/home/cuzic/rust-nicola-wt-adr274`(HEAD `58d57c94` = origin/develop `1cdc8778` + 草稿)。読み取りのみ、`cargo` は走らせていない。
`python3 -I tools/core-split/deps.py crates/awase-windows-core/src tools/core-split/assign.py --check` → `OK` を確認した。

**総評**: round 1 の Blocker 2 件は解消した。割り当ては本番・テストの両方で成り立つ。段階 1 の hook-core と plans は、
use 文で確かめた限り、ファイルの中身を変えずに移せる。残る指摘は Blocker 0・Major 3・Minor 6。
Major はすべて「分割の途中で、ガードが黙って検査を減らす」型で、段階 0 に 2 項目を足せば塞がる。

---

## 1. `--check` の割り当ては成り立つか、見落としは残っていないか

**成り立つ。** 確かめたこと:

- **本番の辺**(`--check` の出力の `===== prod`): `keyeffect→base`、`imecore→{base,keyeffect}`、`plans→{base,imecore}`、`hookinput→base`。
  計画どおり除外しているのは `focus::class_names → state::ime_event`(`class_names.rs:334`)だけ。
- **テストの辺も違反は無い**: `deps.py` の `--check` の `for kind in ("prod",)` を `("prod", "test")` に変えた写し
  (`scratchpad/deps_testcheck.py`)でも `OK`。ただし [Minor-1] のとおり、CI にするなら test も検査する。
- **新しく入った `indicator_glyph.rs`(1,481 行)・`indicator_evidence.rs`(285 行)・`indicator_menu.rs`**: 外への参照は次だけで、
  すべて imecore の中か base に収まる。
  - `indicator_glyph.rs:596-597`: `crate::journal::IndicatorGlyphRecord`
  - `indicator_evidence.rs:11,147,265`: `crate::journal::{IndicatorMenuRecord, IndicatorGlyphRecord}`
  - `indicator_menu.rs:371,382`: `crate::journal::IndicatorMenuRecord`
  - `indicator_menu.rs:433-436,819`: `crate::state::ime_kind`
  - `serde`
- **journal の依存**: journal も imecore にあるので、journal → imecore の辺はすべて同じクラスタの中。
  base・keyeffect のファイルから `crate::journal`・`indicator_*`・imecore の各モジュールを参照する箇所を全数 grep した。
  出てきたのは `class_names.rs:334,587`(計画どおり)だけだった。`converter_target.rs:13` と `late_keymap_plan.rs:130,134,288` は `ime_kind` で、これは base。
- **スキャナの残る近似**: インラインの非テストモジュールで `super::` を使うものは、`evidence.rs:36 mod sealed`・
  `actuation_decision_record.rs:74 mod nested_optional_bool`・`drift_plan.rs:938 mod edge`(テスト内)・`conv_classify.rs:810`(テスト内)の 4 つ。
  `super` の解決は 1 段ずれるが、どれも外のクラスタを指さないので、結果は変わらない。

### [Major-1] `deps.py --check` は、分割の途中から黙って検査範囲が縮む

- `deps.py` が走査するのは引数の 1 ディレクトリ(`crates/awase-windows-core/src`)だけ。
- `--check` は「走査したのに未割当のモジュール」は落とす。一方で「`assign.py` にあるのに走査で見つからないモジュール」は落とさない(`deps.py:226` の `unassigned` は一方向)。
- そのため、段階 1 で hook-core と plans を別の crate へ移すと、それらのファイルは走査から消える。`--check` は `OK` のまま、
  移した crate の中から上向きの参照(たとえば plans → hookinput)が増えても検出しない。
  cargo が防ぐのは crate 間の循環だけで、「hook-core が base 以外に依存しない」(ADR の主張)は cargo では守られない。
  hook-core は段階 1 では `awase-windows-core` 全体に依存するからだ(次の Major-2)。
- 対策(段階 0 の 3 に含める):
  - 走査対象を「core 系 crate の `src/` の和」にする(段階 0 の 5 と同じ crate の一覧を使う)。
  - `assign.py` にあって走査に無いモジュールも NG にする。

### [Major-2] `ALLOWED` と ADR の本文が食い違っている

- ADR の D1 は「L1 hookinput(base だけに依存)」、要約も「hookinput は base だけに依存する」と書く。
- ところが `assign.py` の `ALLOWED['hookinput'] = ['base', 'keyeffect', 'imecore']`。これでは将来 hookinput → imecore の辺が足されても `--check` が通る。
- 段階 1 では hook-core は `awase-windows-core` 全体に依存する。cargo もこの辺を止めない。段階 2 で base を剥がしたとき、
  hook-core を base だけの依存に付け替えられなくなる。
- 対策: `ALLOWED['hookinput'] = ['base']` にする。本文の層の番号(L1)と `ALLOWED` を一致させる。
  plans も、本文は `plans → {base, imecore}` だが `ALLOWED` は keyeffect も許す。実際の辺に合わせて絞るか、本文に keyeffect を足す。

---

## 2. 段階 1 の hook-core・plans は準備なしで単独でコンパイルできるか

### hookinput(7 モジュール)の依存は base だけ。use 文で確認した

| ファイル | core への参照 |
|---|---|
| `hook_state.rs` | `awase::…` のみ(1-3, 110-111 行) |
| `hook_watchdog.rs`・`win_key_guard.rs` | `crate::`/`super::<名前>` の参照なし |
| `alt_impersonation.rs` | `crate::vk::…`(21-22, 39, 114, 205 行) |
| `foreign_modifier.rs` | `crate::vk::{VK_CONTROL, VK_LCONTROL, VK_RCONTROL}`(16 行) |
| `half_width_alnum.rs` | `crate::state::{TickMs, foreground_scope::ForegroundScope, ime_kind::ImeKindId}`、`crate::vk::VK_[LR]SHIFT`(4-5, 185-188, 474-478, 1103, 1138-1139 行) |
| `half_width_resume_hold.rs` | `crate::state::{TickMs, foreground_scope::ForegroundScope}`(14-15 行) |

- core の中に hookinput の利用元は無い。`alt_impersonation`・`half_width_alnum` の名前が出るのは `app_suppression.rs:5`・`win_key_guard.rs`・`tuning.rs:260` で、すべてコメント。
- `HookConfig`・`SyncKeyGate`(`pub use hook_state::*`)の core 内の利用もゼロ。
- `cfg(any(test, feature = "testing"))` の項目も、`compile_fail` doctest も、他 crate の型への inherent impl も無い。**準備は要らない。**

### plans(11 モジュール)の依存は base と imecore。use 文で確認した

| ファイル | core への参照 |
|---|---|
| `raw_recovery_plan.rs` | `super::focus_gen::FocusGen`・`super::foreground_scope::ForegroundScope`(12-13 行) |
| `relay_plan.rs` | `super::physical_disposition::PhysicalKeyDisposition`(21 行) |
| `focus_probe_plan.rs` | `super::observation_store::{FocusProbeOpenStatus, ObservedOpenValue}`(16 行)。テストで `crate::focus::class_names::AppImeProfile`(78 行) |
| `focus_classify_plan.rs`・`focus_resolve_plan.rs`・`msaa_role_plan.rs` | `crate::focus::FocusKind`(13・11・10 行) |
| `focus/hwnd_cache.rs` | `crate::state::TickMs`・`crate::tuning::HWND_CACHE_MAX_AGE_MS`(7-8 行) |
| `warm_send_plan.rs`・`deferred_gate_plan.rs`・`post_bypass.rs`・`focus_resync_policy.rs` | 自分の enum の `use` だけ |

- `deferred_gate_plan` は `warm_send_plan.rs:11` に出てくるが、コメント。両方とも plans なので、どちらにしても問題ない。
- `hwnd_cache` は `observation_store.rs:246…`(同名のフィールド)、`intent_store.rs:33`・`tuning.rs:425`(コメント)に出てくるだけ。
- **core の中に plans の利用元は無い。準備は要らない。**

### focus/ をまたぐ glob 再公開は成り立つ

- plans の `focus/mod.rs` を次のようにする。
  ```rust
  pub use awase_windows_core::focus::*;
  pub mod hwnd_cache;
  ```
- 次に、`awase-windows-core` の `focus/mod.rs` から `pub mod hwnd_cache;` を消す。
- これで plans の中の `crate::focus::FocusKind`(core の `focus/mod.rs` が `kinds::{…}` を `pub use` している。その glob の連鎖も glob で入る)と、
  `crate::focus::class_names::AppImeProfile`(テスト)が解決する。
- 殻は付け替えが要る。`focus/mod.rs:15` が明示列挙の `pub use awase_windows_core::focus::{class_names, hwnd_cache, kinds, …}` なので、
  `hwnd_cache` だけを `awase_windows_plans::focus::hwnd_cache` に替える。
  殻の `state/mod.rs:8` にも `pub use awase_windows_plans::state::*;`・`pub use awase_hook_core::state::*;` を足す。
- 同じ `TickMs` などが 3 本の glob から入るが、どれも同じ項目なので曖昧エラーにならない。
- 各新しい crate の `state/mod.rs` には、core と同じ `#![allow(unreachable_pub)]`(core の `state/mod.rs:3`)を写す。
  `pub use …::*` は今の殻(`state/mod.rs:8`)と core(`pub use belief::*`)ですでに clippy の pedantic deny を通っているので、`wildcard_imports` の心配は無い。

### [Minor-2] hook-core を移すと、`state/mod.rs` の行も動く

`state/mod.rs` の `pub use hook_state::*;`(28 行)は hook-core の `state/mod.rs` へ移す。これは移動の PR の中の `mod.rs` 編集なので、`git mv` の類似度には影響しない。
ただし D3 の箇条書きは「base の mod.rs には置けない」としか書いていない。段階 1 で hook-core へ移すと明記する。

---

## 3. 段階 0 の 5 つで足りるか

段階 1 の前に、次を足すことを勧める。

### [Major-3] 段階 0 に「core 系 crate の一覧を 1 か所に置き、パスを使う検査をそこから作る」を足す

段階 1 の hook-core と plans は、**どちらも再発ファミリーのファイルを含む**。
- 半角英数トグル: `state/half_width_alnum.rs`
- 物理キー押下ラッチ: `half_width_resume_hold.rs` も pre-push の正規表現の対象
- warmup: `state/warm_send_plan.rs`
- defer/replay: `deferred_gate_plan.rs`・`relay_plan.rs`
- focus 遷移: `focus_classify_plan.rs`・`focus_resolve_plan.rs`・`msaa_role_plan.rs`

D5 の 1(`.githooks/pre-push:38,86`)は、段階 1 の最初の PR から効く。「各 PR の共通の後処理」で毎回正規表現を書き換える方式は、
足し忘れが BUG-116 と同じ穴になる。

段階 0 の 5 と同じ一覧(例: ワークスペースのメンバーのうち `awase-windows-base` か `awase-windows-core` に推移的に依存するもの)を
`tools/core-split/` か `xtask` に 1 つ置き、次の 5 つをそこから作るか、一覧と照合して落とす:
1. pre-push の正規表現: `crates/awase-[a-z-]+/src/` のように crate 名を問わない形にする、でもよい。
2. `xtask-adr-evidence/src/core_registry.rs:53-54`(パスの直書き)と `main.rs:243`(読む mutants の設定の 2 ファイル固定)。
3. `ci.yml` の `-p` 群(57・191・336・404 行)。
4. mutants の matrix(`ci.yml:658`、`mutants-windows.yml:50`)。
5. 段階 0 の 3(`deps.py` の走査対象)。

### [Minor-3] scancode と bug-report は「準備なし」ではない

- `scancode_map.rs` は純粋部と `#[cfg(windows)] mod registry`(82・227 行)が同じファイルにある。D1 は「registry は殻に残す」と言うので、
  移動の前に registry を別ファイルへ分ける PR が要る。そうしないと D3 の「`git mv`(類似度 100%)」が成り立たない。
- `bug_report.rs` の `BugReportKeymapLearnSummary::from_paths` が `key_effect_io::read_persisted_table`(殻)を呼ぶ箇所を、
  結果を受け取る形に変える準備も、移動の前の PR に分ける。
- 段階 1 の 3 には、この 2 つの準備 PR を明記する。

### [Minor-4] 移した crate の統合テストは黙って走らなくなる

- `ci_test_coverage_guard.rs` は `crates/awase-windows/tests/*.rs` しか見ない。
- `cargo nextest run --workspace --lib`(`ci.yml:54`)は統合テストを走らせない。
- scancode と一緒に `tests/scancode_vm_scenarios.rs` を新しい crate の `tests/` へ移すと、どこからも実行されなくなる。
- 対策: 統合テストは殻に残す(再公開で `awase_windows::scancode_*` が引ける)か、ガードの走査対象を core 系の crate の一覧に広げる。D5 に 10 番目として足す。

---

## 4. D5 の 9 か所の誤りと漏れ

- **D5-2(compile_fail doctest)の時期**: 5 個(`ime_actuation.rs:155`・`gji_direct_mechanism.rs:182`・`observation_store.rs:80,177`・`actuation_chain.rs:93`)は
  すべて imecore にある。新しい順序では、**どれも移動しない**。危ないのは段階 2 の 3「`awase-windows-core` を `awase-ime-core` に改名」の時点だけ。
  - package 名を変えると `ci.yml:57` の `-p awase-windows-core` は「package が無い」で落ちる。これは大きな音で落ちる。
  - 一方、doctest の中の `use awase_windows_core::…` を直し忘れると、黙って空になる。
  - 時期をそう書き直す。
- **D5-9(凍結コーパス)の時期**: `actuation_decision_record.rs` は imecore に残るので、パスが変わるのはディレクトリ名を変える場合だけ。
  未決 3(改名するか)に「package 名だけ変えるか、ディレクトリ `crates/awase-windows-core/` も変えるか」を足す。
  ディレクトリを変えると、`complexity-budget.md`・`journal-replay-guide.md`・`fix-requires-evidence.md` の注記・メモリのパス、`git log -- <path>` が一斉に動く。
  package 名だけなら、ほぼ D5-2 だけで済む。
- **D5-4 の精度**: Linux の単体テストは `cargo nextest run --workspace --lib`(`ci.yml:54`)が新しい crate も自動で拾うので、外れない。
  外れるのは windows-test の `ci.yml:404` と clippy の `ci.yml:336` だけ。
- **D5-7 の漏れ**: `test_package` だけでなく、移したファイルの `examine_globs`(`.cargo/mutants-awase-windows-core.toml` に、plans の 10 本・hookinput の 5 本が並んでいる)も問題になる。
  移すと存在しないパスになり、その変異は誰も検査しなくなる。新しい `mutants-<crate>.toml` を作り、`ci.yml:658` の `pkg` に足さない限り、黙って外れる。
  `xtask core-registry` は「base 比で足された名前」しか見ないので、移動は検出しない。
- **D5 に足すもの**:
  - 10 番目: [Minor-4] の統合テスト。
  - 11 番目: [Major-1] の `deps.py` の走査対象。
- **D5-8(`config-verify.yml` の paths)**: 正しい。ただし段階 2 の 1(base)まで関係しない。

---

## 5. 判断点の置き方

段階 1 の後に置くこと自体は妥当。**ただし、測るものが段階 2 の判断に効かない。**

- **[Minor-5]「`pub(crate)` → `pub` の実際の個数」は判断材料にならない**:
  - 段階 1 の葉(hookinput・plans)には、クラスタをまたぐ `pub(crate)` が構造上ゼロ(どちらも core から使われない)。
  - 段階 2 の base・keyeffect も、round 1 で全数を調べた結果、`key_sequence_policy` の 4 項目を imecore へ移した今はゼロ。
    残りの `pub(crate)`(`app_suppression::normalize_process_name` は base 内、`relay_plan.rs:92,126` は plans 内、`mode_key_pass` は imecore 内)も同じクラスタで閉じる。
  - どの段階でも 0 なので、測っても判断が変わらない。
- **段階 2 の費用の性質は段階 1 と違う**: 段階 2 の base は全員が依存する土台で、変更の多いファイル(`tuning`・`vk`・`converter_target`・`class_names`)を含む。
  段階 1 の測定(葉 2 つの CI 時間)から、段階 2 の費用(base を変える PR が 5 crate の再ビルドと全 crate の glob 経由になる)は外挿できない。
- **[Minor-6] 判定の基準を事前に数字で書く**。後から「見通しが良くなった」で決めると、検証にならない。案:
  - (a) CI の `test`・`clippy`・`windows-test` の所要時間の増加が N 秒以内(段階前の 3 run の中央値と比べる)。
  - (b) 段階 1 の後の 2 週間に、hook-core か plans と core の両方を触った PR の割合(round 1 で数えた「クラスタをまたぐコミット 23%」の実測版)。
    これが高ければ、葉の分割すら同時変更を増やしている。
  - (c) 段階 1 の PR のレビュー・CI で見つかった「D5 の穴」の件数。0 件でなければ、段階 0 の自動化が足りない。
  - (d) 段階 2 に進む場合、先に `deps.py` の base クラスタへの変更頻度を数え、base を剥がしたときの再ビルドの広がりを見積もる。
- 「段階 2 に進まない」も正規の結論として書いておく。その場合、`awase-windows-core` には base + keyeffect + imecore が残る。
  その場合の扱いも決めておく: 名前を `awase-ime-core` に変えるか(未決 3)、description と `CLAUDE.md` だけ直すか。
