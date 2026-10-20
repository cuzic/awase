# ADR-274 草稿 Opus レビュー(2026-10-20)

対象: `/home/cuzic/rust-nicola-wt-adr274`(HEAD `274701e1` = origin/develop `5f916c69` + 草稿)。
読み取りのみ。`cargo` は走らせていない。再現用の改良版スキャナ:
`/tmp/claude-1001/-home-cuzic-rust-nicola/c8f2fa6a-d88c-413f-8fb7-ac5b33cd241e/scratchpad/deps2.py`
(`python3 -I deps2.py crates/awase-windows-core/src tools/core-split/assign.py`。コメントと文字列を除く、
`#[cfg(test)]` の付いた項目だけを外す、`a::{b, c}` を展開する、`super::super::` を解決する、`state/mod.rs`・`focus/mod.rs` の
`pub use` を解決する)。出力: 同じディレクトリの `deps2.out`、モジュール単位の辺は `modedges.json`。

結論を先に書く。**「測定」の節の主要な主張(クラスタ間は 1 か所の移動で DAG になる)は誤り**。
本番コードにクラスタをまたぐ循環がもう 1 つある(imecore ↔ diagnostics、`mozc_shm` 経由)。
deps.py は本番コードの一部を読み落とし、同時に doc コメントを依存として数えている。
D4 の順序にもコンパイルが通らない段階がある。分割そのものについては、**今は 7 分割をやらず、
層の向きをガードテストで固定し、小さなコードの移動を 2 つだけ先に入れる**ことを勧める(§5)。

---

## 1. 「測定」の裏取り

### 1-a. [Blocker] 見落としている循環: imecore → diagnostics(`evidence.rs:265` → `mozc_shm`)

- `state/evidence.rs:259-270` は本番コード(`impl Observed<MozcShm>` の `from_shm`)で、引数の型が `super::mozc_shm::ShmState`。
- `state/mozc_shm.rs:13` は `use crate::journal::MozcShmRecord;`(`to_record`、`mozc_shm.rs:432`)。
- `journal` は imecore の 7 モジュールを使う(`ime_event`・`ime_actuation`・`ime_actuation_decision`・`drift_plan`・`drift_correction`・`conv_classify`・`actuation_decision_record`)。
- よって imecore(evidence)→ diagnostics(mozc_shm → journal)→ imecore の循環。D4 の段階 5(imecore を抜く)はこの状態ではコンパイルできない。
  diagnostics はまだ旧 crate に残っていて、旧 crate は imecore に依存するので、imecore 側から `mozc_shm` を参照できない。
- 改良版スキャナで両方の辺(`class_names → ime_event` と `evidence → mozc_shm`)を除くと、
  モジュール単位の SCC は `{key_effect_predictor, key_effect_table}`・`{evidence, ime_event, probe_admission}`・
  `{actuation_chain, app_ime_policy, open_warrant}` の 3 組に戻り、クラスタ間は DAG になる。
  除かないと、`focus::class_names`・`journal`・`mozc_shm`・`key_sequence_policy` を含む **20 モジュールの SCC** になる。
- 直し方の案(小さい順):
  (i) `mozc_shm::to_record`(と `RecordContext`)を journal 側へ移し、`mozc_shm`(共有メモリの解釈と `ShmState`)を imecore に置く。
  (ii) `MozcShmRecord` の定義を `mozc_shm.rs` へ移し、journal が使う。
  ADR-268 で共有メモリは belief の証拠(High)になったので、`mozc_shm` は「診断」ではなく「観測源」として imecore に置くのが実態に合う。
  草稿の割り当ての根拠(「mozc_shm が journal を使う」)は ADR-265 段階 1 時点の話で、もう古い。

### 1-b. [Major] deps.py は本番コードを読み落とす(最初の `#[cfg(test)]` で切っている)

`production()` は最初の行頭 `#[cfg(test)]` 以降を捨てる。この行が `mod tests` ではなく 1 項目だけのテスト用 `use`/`const fn` のファイルがある:

| ファイル | 最初の `#[cfg(test)]` | 読まれない本番コード |
|---|---|---|
| `state/evidence.rs` | 32 行(`use super::probe_admission::FocusFence`) | 33〜394 行(上の `mozc_shm` 参照もここにある) |
| `state/conv_mode.rs` | 11 行(`use super::ime_event::HwndId`) | 12〜254 行(`super::probe_admission`・`super::TickMs`) |
| `state/key_effect_rules.rs` | 90 行(`const fn all_keys_is_exhaustive`) | 91〜198 行 |

1-a の見落としの直接の原因はこれ。

### 1-c. [Major] deps.py は doc コメントを依存として数える(偽の辺)

`crate::...` の正規表現がコメントにも当たる。このため次の辺は実在しない:
- 草稿の SCC `{key_effect_predictor, key_effect_runtime, key_effect_table}` のうち predictor → runtime は doc だけ
  (`key_effect_predictor.rs:240,270,300,498,1173,1414` の `` [`crate::state::key_effect_runtime`] ``)。本当の SCC は `{predictor, table}`。
- `hookinput → tsf`(`state/hook_state.rs:32,35` の doc。指しているのは殻の `crate::tsf::TsfGate`)。
- `diagnostics → tsf`(`tsf/literal_facts.rs:93` の doc。殻の `crate::tsf::observer`)。

D1 の矢印 `diagnostics → {…, tsf}` と `hookinput → {…, tsf}` は doc 由来で、実際には無い。tsf クラスタに依存する core のモジュールは無い
(使うのは殻だけ)。focus も同じ。実際の向きは次のとおり:
`keyeffect→base`、`imecore→{base,keyeffect}`、`tsf→{base,keyeffect}`、`focus→{base,imecore}`、
`diagnostics→{base,imecore}`(テストは +keyeffect)、`hookinput→{base,diagnostics}`。

### 1-d. 読み落としを確かめた項目(問題なし、または小さい)

- **`state/mod.rs` の glob 再公開**(`pub use belief::*;`・`pub use hook_state::*;`・`conv_mode::{…}`・`ime_model::AppliedImeState`・`generation::{…}`):
  core の中で `crate::state::<型名>`/`super::<型名>` の形で使っているのは `TickMs`・`ApplyGeneration`(どちらも base)と
  `focus::{AppKind, FocusChangedAxes, FocusKind}`(base)だけ。隠れた辺は無い。
  ただし、これらの `pub use` 行は base の `state/mod.rs` に置けない(imecore・hookinput の型を指す)。
  imecore の `state/mod.rs` と hookinput の `state/mod.rs` へ移す必要がある(D3 に書かれていない)。
- **`state` = `mod.rs` の中身**: `TickMs`(+ `saturating_sub`)と `pub mod` 宣言・再公開だけ。base に置いてよい。
- **`pub(crate)`/`pub(super)`/`pub(in)` のクラスタ越え**: 実際にまたぐのは `state/key_sequence_policy.rs:65,75,88,127`
  (`ImeOperation`・`from_open`・`KeyMechanism`・`ime_key_for`)を `ime_actuation_decision.rs:22,383-390`(imecore)が使う 1 組だけ。
  `belief.rs:33-40` の `pub(in crate::state)` は belief.rs の中でしか触っていない。`observation_store::set` の `pub(crate)`
  (INV-49)の呼び出し元は `open_warrant`・`drift_correction`・`ime_model` で、すべて imecore。`unreachable_pub` の心配は小さい。
- **inherent impl(E0116)**: `state/explicit_press.rs:1751` の `#[cfg(test)] mod tests` の中にある
  `impl PhysicalKeyDisposition { fn legacy_… }`。`PhysicalKeyDisposition` は keyeffect の型なので、imecore へ移すとテストがコンパイルできない。
  これは ADR-208 L0 の「逐語コピー」で、コメントに「L1 以降で削除してよい」とある。削除するか自由関数に書き換える準備が要る。
- **孤児規則**: `class_names.rs:334` は、そもそも孤児規則の問題ではない
  (`impl From<Local> for Foreign` は RFC 2451 で許される)。問題は base から `ImePolicyProfile` を名前で参照できないこと。
  ime_event 側へ移せばこの辺は消える(§1-e)。ほかの `impl Trait for Type` で、クラスタをまたぐのは
  `actuation_decision_record.rs:656,665`(ローカル型 `ReplayWriter` に imecore のトレイトを実装)だけ。これは許される。
- **マクロ**: `vk_table!`(`#[macro_export]`、`vk.rs:65`)は `$crate` を使わず、表を渡すだけ。殻の呼び出し
  `vk_windows.rs:24` の `awase_windows_core::vk_table!` は crate 名の付け替えだけで足りる。`declare_evidence!`(evidence.rs:98)は imecore の内部で閉じている。
- **`#[cfg(test)]` の中の依存**: 上向きの参照(下の層のテストが上の層を使う)は 2 つだけ。
  `class_names.rs:584-603` のテスト(`ImePolicyProfile`)は、impl と一緒に移す必要がある。
  `observation_store.rs:1498-1510` のテスト(`mozc_shm::ShmState`)は 1-a の直し方で解消する。
- **`cfg(any(test, feature = "testing"))`**: `FocusGen::new`(`state/focus_gen.rs:18`、base)を
  `tsf/literal_facts.rs:341-452` のテスト(diagnostics)が使う。依存先の crate では `cfg(test)` が立たないので、
  diagnostics の dev-dependency で base の `testing` を有効にする必要がある。
  殻の `[dev-dependencies] awase-windows-core = { features = ["testing"] }`(`crates/awase-windows/Cargo.toml:61`)は、
  薄い再エクスポートの crate では `testing = ["awase-windows-base/testing", "awase-ime-core/testing", …]` と各 crate へ転送する必要がある
  (`ImeModel::set_desired_open_for_test`、`ime_model.rs:400`、の利用元は殻の `platform_state.rs`)。草稿に書かれていない。

### 1-e. `class_names.rs:334` の移動だけでその辺が消えるか

本番コードの辺は消える。ただし、次も一緒に要る:
- `class_names.rs:584-603` のテスト `From<AppImeProfile> for ImePolicyProfile の3分岐` も imecore へ移す。
- impl の doc(`class_names.rs:329-333`「state 層が focus 層に直接依存しない設計を維持する」)は、移動先では逆の意味になる
  (ime_event → focus::class_names の依存ができる)。imecore は `explicit_press`・`app_ime_policy`・`ime_actuation_decision` で
  すでに `AppImeProfile` を使っているので、層の問題は無い。doc を書き直す。
- `ime_event.rs` は `architecture_guard.rs:4637,4682` の出現数ガードの対象(`InitialFocusScopeEstablished`・`ModeKeyPassedThrough`)。
  移す impl はどちらの文字列も含まないので影響しない。

ほかに隠れた辺は、1-a(`evidence → mozc_shm`)だけ。

---

## 2. D3「パスを変えない」は Rust のモジュール解決として成り立つか

**成り立つ。ただし草稿の 1 行の説明では足りない。** 具体的には次のとおり。

- `crate::state::foo::Bar` は、`state` が複数の crate に分かれていても解決できる。
  各 crate が自分の `src/state/mod.rs` を持ち、下位の crate の `state` を `pub use lower::state::*;` で glob 再公開すれば、
  `pub mod foo` も glob で入る(モジュールは型の名前空間の項目)。複数の glob が同じ項目を別の経路で持ち込んでも
  曖昧エラーにならない(同じ Res)。ローカルの `pub mod` は glob より優先される。
  `super::foo`(`state/bar.rs` から)と `super::super::foo`(`state/bar.rs` の `mod tests` から)も、
  `super` = その crate の `state` なので解決できる。
- 個別に `pub use awase_key_effect::state::key_effect_predictor;` と並べる書き方(草稿の例)は、
  下位の crate のモジュールを 1 つ足すたびに上位の crate すべての `state/mod.rs` を直すことになる。glob に統一するほうがよい。
- **`focus/` と `tsf/` も分かれる**(草稿は `state/mod.rs` しか書いていない)。base が `focus/{mod,kinds,class_names}.rs`、
  focus クラスタが `focus/hwnd_cache.rs`、base が `tsf/converter_target.rs`、diagnostics が `tsf/literal_facts.rs` を持つ。
  `focus/mod.rs`・`tsf/mod.rs`・`lib.rs` が最大 3〜4 個の crate にでき、各 crate の `lib.rs` に
  `pub use awase_windows_base::{vk, tuning, keymap};`、diagnostics を使う crate には `pub use awase_diagnostics::journal;` などが要る。
  殻の `focus/mod.rs:15` の明示 `pub use awase_windows_core::focus::{class_names, hwnd_cache, kinds, …}` と、
  `tsf/mod.rs:31,33` も付け替える。
- **`pub(crate)`**: 越えるのは §1-d の `key_sequence_policy` の 4 項目だけ。§4 の割り当て変更をすれば 0 になる。
- **`pub(super)`/`pub(in crate::state)`**: `key_effect_table.rs:13,130,332`(`ATOK`・`MSIME`・`MSIME_NATIVE`)と
  `key_effect_predictor.rs:208-214,712,717` は同じクラスタの中で閉じる。`belief.rs:33-40` は belief.rs の中で閉じる。壊れない。
- **E0116**: `explicit_press.rs:1751`(§1-d)。
- **doctest(サイレントに失敗する、最も危ない)**: `compile_fail` の doctest が 5 個ある
  (`ime_actuation.rs:155`・`gji_direct_mechanism.rs:182`・`observation_store.rs:80,177`・`actuation_chain.rs:93`)。
  これらは `use awase_windows_core::state::…` と書いている(`actuation_chain.rs:69-98`、`ime_actuation.rs:139-157`、
  `gji_direct_mechanism.rs:166,185`、`observation_store.rs` に 8 か所)。imecore へ移すと:
  (a) CI は `cargo test --doc -p awase-windows-core`(`ci.yml:57`)しか走らせないので、imecore の doctest は実行されなくなる。
  (b) `-p awase-ime-core` に直しても、`awase_windows_core` は awase-ime-core の依存ではないので `use` の解決に失敗する。
  その結果 **`compile_fail` は「間違った理由で」コンパイルに失敗して合格し続ける**。型状態の証明(`ci.yml:55-56` のコメント
  「`compile_fail` は型状態の証明そのもの」)が黙って空になる。
  対策: 移動の PR で doctest の crate 名を書き換え、`ci.yml:57` の `-p` を足す。さらに、各 `compile_fail` と対になる
  「同じ `use` で正しい形はコンパイルできる」doctest(`rust` の通常ブロック)を 1 つずつ置き、`use` の失敗では合格しないようにする。
- **doc の intra-doc link**: core の doc はすでに殻のパス(`crate::tsf::TsfGate`・`crate::ime::send_ime_mode_key`・
  `crate::msime_key_assignment` など)を指していて、解決できていない。CI は rustdoc を `-D warnings` で回していないので、
  分割しても状況は変わらない。対処は不要。
- **dylint**: `ime_event_guard`・`observation_source_guard` は `def_path_str(..).ends_with("ime_event::ImeEvent")` と関数名で照合している。
  crate 名には依存しない。ただし、適用対象は `ci.yml:191` の `-p awase-windows -p awase-windows-core` だけ。
  `ImeEvent` を組み立てる reducer(`ime_model.rs`)が awase-ime-core へ移ると、`-p` を足さない限り **lint が黙って外れる**。

---

## 3. D4 の順序と、各段階が単独でコンパイルできるか

### 3-a. [Blocker] 段階 3(base)は段階 5 の「先に impl From を移す」より前なので通らない

`class_names.rs` は base。`impl From<AppImeProfile> for crate::state::ime_event::ImePolicyProfile`(334 行)と、
そのテスト(584-603 行)が残ったまま base を抜くと、base から `ime_event` を参照できない。impl とテストの移動は段階 3 より前に置く。

### 3-b. [Blocker] 段階 5(imecore)は `evidence → mozc_shm`(§1-a)を先に直さないと通らない

### 3-c. 各段階で必要な事前作業(移動を 100% 類似度の `git mv` にするため、ファイルの中身の変更は先の PR に分ける)

| 段階 | ファイルの中身の事前変更 |
|---|---|
| base | `class_names.rs:334` の impl と `:584-603` のテストを `ime_event.rs` へ。`state/mod.rs` の `pub use belief::*`・`hook_state::*`・`conv_mode::…`・`ime_model::AppliedImeState` を上位の crate へ |
| keyeffect | `key_sequence_policy` の `pub(crate)` → `pub`(または §4 で imecore へ) |
| imecore | `mozc_shm::to_record` を journal へ(§1-a)。`explicit_press.rs:1751` の inherent impl を削除または書き換え。`compile_fail` doctest の crate 名 |
| diagnostics | `crates/awase-windows-core/tests/journals/actuation_decision/` も移す(`actuation_decision_record.rs:1270` が `env!("CARGO_MANIFEST_DIR")` から読む。0 件なら `assert_ok` が落ちるので、黙っては壊れない)。`docs/journal-replay-guide.md:66`・`.claude/rules/complexity-budget.md` の凍結コーパスのパス |

base を抜いたとき、残りが base の何を `crate::` で参照するかは問題にならない(残りの crate は base に依存し、再公開で解決する)。
問題は逆向き(base → 残り)で、本番もテストも、上の impl From の 1 組だけだった(改良版スキャナで確認)。

### 3-d. 分割で壊れる、または黙って穴があく箇所(ファイル:行)

**黙って穴があく(CI は緑のまま検査が外れる)もの。ここが最も危ない:**
1. `.githooks/pre-push:38`: 再発ファミリーの正規表現が `crates/awase-windows(-core)?/src/…`。新しい crate 名には当たらない。
   `ime_model.rs`・`observation_store.rs`・`physical_disposition.rs`・`*_plan.rs`・`tuning.rs` などが **fix-requires-evidence の警告の対象から外れる**
   (BUG-116 と同じ型の穴)。`:86` の `grep -qE '^crates/awase-windows(-core)?/'` も同じ。
2. `ci.yml:57`: doctest の `-p awase-windows-core`(§2 の compile_fail)。
3. `ci.yml:191`: dylint の `-p`。
4. `ci.yml:336`: clippy の `-p awase-windows -p awase-windows-core`。依存として build はされるが、clippy の lint は `-p` で名指しした package にしか効かない。
5. `ci.yml:404`(windows の nextest `--lib -p awase-windows -p awase-windows-core`): 新しい crate の単体テストが Windows で走らなくなる。
6. `tests/support/src_roots.rs:11` の `EXTRA_SRC_CRATES`: 足し忘れると、`architecture_guard.rs` の「列挙されないファイルは 0 件」型の
   出現数ガード(例: `:4623-4660`、`:4675-4690`)が、その crate のファイルを走査しないまま合格する。
   草稿は「足せば足りる」と書くが、**足し忘れを検出する仕組みが無い**。`cargo metadata` で、`awase-windows-base` に推移的に依存する
   ワークスペースのメンバーがすべて `EXTRA_SRC_CRATES` にあることを確かめるガードを、最初の段で足す。
7. `layer_boundary_guard.rs:811-845` の `crate_does_not_depend_on_windows`: `../awase-windows-core/Cargo.toml` の 1 つだけを見る。
   新しい crate は対象外(草稿の D5 は認識している)。6 と同じ仕組みで、crate の一覧から作るようにする。
8. mutants: `.cargo/mutants-awase-windows-core.toml:14` の `test_package = ["awase-windows-core", "awase-windows"]`。
   分割後の各設定には、**その crate より上の crate をすべて**入れる。そうしないと、上位の crate のテストでしか捕まらない変異が
   false MISSED になる(同ファイル 11-13 行のコメントにある Opus PR #574 C4 と同じ)。base なら 8 crate すべて。
   `ci.yml:658` の matrix は `pkg × 8 shard` で、2 → 9 pkg にすると 72 ジョブになる。`mutants-windows.yml:50` も同じ。
9. `config-verify.yml:20-21` の `paths:` トリガー(`crates/awase-windows-core/src/vk.rs`・`keymap.rs`)。base へ移すと、vk.rs を変えてもこの workflow が起動しない。

**大きな音で落ちる(直せば済む)もの:**
- `crates/xtask-adr-evidence/src/core_registry.rs:53-54,69` は `"crates/awase-windows-core/src/state/{name}.rs"` を直書きしている。
  `main.rs:243` は `.cargo/mutants-awase-windows{,-core}.toml` の 2 つだけを読む。
- `mise.toml:12`(check の `-p`)。
- ルートの `Cargo.toml:2` の `members`。各 crate の `[lints]` の 25 行(`crates/awase-windows-core/Cargo.toml` の lints 節)。
  `[workspace.lints]` が無いので 9 個に複製される。ずれを防ぐため、先に `[workspace.lints]` へ寄せる。

---

## 4. 名前(D2)とクラスタの切り方(D1)

- **belief と actuation を imecore にまとめる**: 賛成。`{evidence, ime_event, probe_admission}`・`{actuation_chain, app_ime_policy, open_warrant}` の
  SCC と `ime_model` の結合から、分けると循環になる。
- **keyeffect に入れた 4 つは、辺を消すための置き場で、名前と合わない**:
  - `key_sequence_policy`(202 行、IME ON/OFF に送る VK。再発ファミリーでは「キー選択」)は、core の利用元がすべて imecore
    (`ime_actuation_decision`・`ime_profile_driver`・`gji_direct_mechanism`・`app_ime_policy`・`actuation_chain`)と `physical_disposition`。
  - `physical_disposition`(1,324 行、物理キーの配送判断)の利用元は `explicit_press`(imecore)と `relay_plan`(tsf)。
  - 案: この 2 つを imecore へ移す。`pub(crate)` → `pub` は 0 件になり、`tsf → imecore` の辺ができる
    (imecore → tsf は無いので循環しない)。keyeffect は「学習済みのキー効果の表と予測」だけになり、名前と一致する。
  - `scoped_latch`(102 行、依存なし)は汎用の部品なので base へ。`mode_key_pass`(721 行)は core に利用元が無い葉。
    再発ファミリーは IME belief(ADR-187、desired_open の揃え)なので imecore へ。
- **diagnostics は「観測源」と「記録」を混ぜている**: `mozc_shm` は ADR-268 で belief の証拠になった(§1-a)。
  `indicator_menu`(hookinput に割り当て)と `indicator_evidence`(diagnostics)は、**open PR #705(ADR-273)が belief へ配線する入口条件を
  決めているもの**そのもの(`resolve_open_at` に分岐を足す)。#707・#709(ADR-261 段階 1)も `state/indicator_glyph.rs` と
  `indicator_evidence.rs` を足す・変える。このリポジトリの段階の進め方は「観測のみ・journal → belief へ接続」
  (ADR-265→268、ADR-264→271→273、ADR-261 shadow)で、**diagnostics に置いたモジュールが後で imecore の依存へ移るのが定型**になっている。
  diagnostics を imecore より上に置く層は、この流れと毎回ぶつかる(`mozc_shm` はすでにぶつかって、測定が見落とした)。
  - 規則にするなら「観測源(読み取りの解釈と witness 型)は imecore、journal の記録型とその変換(`to_record`)は diagnostics」。
    `mozc_shm`・`indicator_menu`・`indicator_evidence`・(#707 の)`indicator_glyph` の解釈部分は imecore、`*Record` と変換は journal。
  - これで hookinput → diagnostics の唯一の辺(`indicator_menu → journal::IndicatorMenuRecord`)も消える。hookinput は「フック・修飾キー・
    半角英数トグル」と名前どおりになる(`indicator_menu` はフックとは無関係)。
- **tsf(1,232 行)と focus(1,186 行)は crate にするには小さすぎる**。実体は `state/*_plan.rs` で、awase-tsf-core には `tsf/` の
  ファイルが 1 つも無い(`tsf/converter_target.rs` は base、`tsf/literal_facts.rs` は diagnostics)。名前とディレクトリが食い違う。
  tsf の中身(`warm_send_plan`・`deferred_gate_plan`・`raw_recovery_plan`・`relay_plan`・`post_bypass`)は殻の `output/` の送信の判断なので、
  分けるとしても `output` の語のほうが合う。どちらも core に利用元が無い葉なので、imecore と同じ crate、あるいは 1 つの
  「`awase-windows-plans`」にまとめても循環は起きない。1 crate あたりの固定費(Cargo.toml・lints・mutants 設定・CI の 6 か所の `-p`・
  ガードの一覧)は行数に比例しないので、1,200 行の crate は割に合わない。
- **base に `tuning.rs` と `tsf/converter_target.rs` を置くと、変更頻度の高いファイルが最下層に来る**(下の §5 の数字)。
  base に置くのは依存の都合で正しいが、「変更の影響範囲が crate で分かる」という利点は、これでほぼ相殺される。
- **名前**: `awase-windows-base` は中身(OS 非依存だが語彙は Windows)と合っている。`awase-ime-core` は、上の移動の後なら妥当。
  `awase-tsf-core`・`awase-focus-core` は、上の理由で作らないことを勧める。

---

## 5. やる価値があるか

### 得るものの実測(草稿の主張と照らして)

`git log --since=2026-10-06 -- crates/awase-windows-core/src`(97 コミット)を草稿の割り当てで数えた。
`mod.rs` は除いた(分割後は各 crate の `mod.rs` になるため)。

- 複数のクラスタにまたがるコミット: **22/97(23%)**。これらは分割後、複数の crate の同時変更になる。
- 1 コミットで再ビルドされる crate の数(下流を含む): **平均 4.7 / 7**。6 以上は **51/97**。
  変更の多いファイルが下の層にある(`key_effect_predictor` 19 コミットは keyeffect、`tuning` 9・`vk` 5・`converter_target` 5 は base、
  `journal` 16 は diagnostics)。草稿の言う「変更の影響範囲が crate で分かる」と「並列ビルド」の利点は小さい。
  さらに、最長の鎖は base → keyeffect → imecore → diagnostics → hookinput → 殻の 6 段で、22k 行の imecore がその途中にある。
  並列度が上がるとは言えない(未実測。主張するなら CI の所要時間を比べてから)。
- 「依存の向きが cargo で強制される」は本物の利点。ただし、これは分割しなくても、**ガードテストでほぼ同じものが得られる**(下)。
- 「名前で責務が分かる」は本物の利点。ただし、§4 のとおり草稿の割り当ては名前と中身がずれている所が多い
  (keyeffect の 4 モジュール、hookinput の indicator_menu、diagnostics の mozc_shm、tsf-core に tsf/ が無い)。

### 払うもの

- PR 8〜9 本と、その前の準備の PR(impl From・mozc_shm・explicit_press・key_sequence_policy・`[workspace.lints]`・EXTRA_SRC_CRATES の自動化)。
- open PR の競合: #707・#709 は `state/mod.rs`・`indicator_evidence.rs`・`journal.rs`・`.cargo/mutants-awase-windows-core.toml` を触る。
  core は 2 週間で 121 コミット・`state/mod.rs` 10 回の変更があり、移動の PR は常に何かと衝突する。
- §3-d の黙って穴があく 9 か所。そのうち 3 つ(pre-push の正規表現、compile_fail doctest、dylint の `-p`)は、
  このリポジトリが過去に実害を出した型(BUG-116 の正規表現漏れ、Opus PR #574 C4/C7)と同じ。
- 再発ファミリーのファイル(`ime_model`・`observation_store`・`physical_disposition`・`*_plan` など)の物理パスが変わる。
  `fix-requires-evidence.md` の表は相対パスの注記で済むが、`git log -- <path>` で履歴を辿る運用(tuning-constants.md・experiment-logging.md が
  前提にしている)は `--follow` が必要になる。ADR-229 D4 で 1 回目の移動はすでに入っている。2 回目で、`--follow` の類似度検出が効かない
  コミット(移動と同時に中身を変えたもの)が混ざると、履歴が切れる。

### 推奨

**今は 7 分割をやらない。** 代わりに、次の 4 つを(この順で)やる。どれも小さく、後で分割するときにもそのまま前提になる。

1. **コードの移動を 2 つ(どちらも振る舞いは変えない)**: `class_names.rs:334` の impl とテストを `ime_event.rs` へ。
   `mozc_shm::to_record` を journal 側へ。どちらも、循環を消すことそのものに価値がある。
2. **層をガードテストで固定する**: `crates/awase-windows/tests/layer_boundary_guard.rs` に「モジュール → 層」の表
   (`tools/core-split/assign.py` を Rust の定数にしたもの)を置く。本番とテストの `crate::`/`super::` の参照が上向きでないことを、
   改良版スキャナ相当の走査(コメントと文字列を除く、cfg(test) の項目を外す、`{}` を展開する、`super::super`、mod.rs の再公開を解決する)で固定する。
   新しいモジュールは表に載っていなければ落とす(載せ忘れ防止)。cargo の強制とほぼ同じ効果が、ファイルを 1 つも動かさずに得られる。
   open PR とも衝突しない。
3. **名前の不一致は文書で直す**: `crates/awase-windows-core/Cargo.toml` の description、`lib.rs` の doc、`CLAUDE.md` の crate 一覧を
   「IME の状態判断」から実態(層の表)に合わせる。
4. **ガードを数週間〜1 か月運用する**。違反がほとんど出なければ、層の切り方は安定している証拠になり、その時点で分割を再検討する。
   違反が頻繁に出るなら(ADR-273 の配線などで)、層の切り方が間違っているので、分割していたら移動をやり直す羽目になっていた。

### それでも分割するなら、最初の 1 段

- **base からではなく、上から(葉から)剥がす。** base を先に抜くと、すべてのクラスタがその再エクスポートを経由することになり、
  移行中だけの「薄い再エクスポートの crate」(D2・D4 の段階 8)が必要になる。上の葉(core のどこからも使われていないクラスタ)を
  1 つずつ別の crate にすると、残りの `awase-windows-core` は名前もパスも変わらない。新しい crate は `awase-windows-core` に依存し、殻が直接使う。
  最後に残る `awase-windows-core`(base + keyeffect + imecore)は、名前どおりの「IME の状態判断の核」になる。
- 葉の候補(core に利用元が無いもの): tsf の plans、focus の plans、hookinput。そのうち **tsf の plans(`warm_send_plan`・`deferred_gate_plan`・
  `raw_recovery_plan`・`relay_plan`・`post_bypass`)** が最も安全。理由: 準備の変更が要らない(上向きの参照・`pub(crate)`・inherent impl・
  `compile_fail` doctest・`testing` 機能・fixture のどれも無い)。再公開の glob と EXTRA_SRC_CRATES の自動化(§3-d の 6)を
  この段で作れば、仕組みの検証になる。
  hookinput は `indicator_menu`(ADR-273 で動く)を含むので、§4 の割り当て直しの後にする。diagnostics は §1-a の修正の後にする。
  imecore と base は最後(あるいは分けない)。
