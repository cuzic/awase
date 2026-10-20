---
id: ADR-274
title: |-
  awase-windows-core を役割ごとの複数の core に分け、名前で責務が分かるようにする(葉から剥がす)
summary: |-
  awase-windows-core(55,061 行・91 モジュール)は「IME の状態判断の core」として作ったが、キー効果の予測、TSF とフォーカスの判断の計画、フックの状態まで同居し、名前が責務を表さない。所有者の方針(2026-10-20): 複数の core に分け、名前を適切に与える。複数の core があってよく、その方が見通しが良い。
  依存を機械的に測った(tools/core-split/deps.py。Opus レビューで改良)。クラスタは base ← keyeffect ← imecore(journal を含む)、plans と hookinput は葉。クラスタ間の本番コードの依存は、`class_names.rs` の `impl From` 1 個を imecore 側へ移せば一方向(DAG)になる。当初案の diagnostics(journal)は、imecore の観測源と相互に依存するため分けず imecore に含める。belief と actuation も辺が 8/10 本で分けない。
  進め方は葉から剥がす: (0) 準備のコード移動と、層の向きを CI で固定する `deps.py --check`、(1) 葉の crate(hookinput → `awase-hook-core`、plans → `awase-windows-plans`、殻から scancode・bug_report)、(2) 下へ剥がす(base → `awase-windows-base`、keyeffect → `awase-key-effect`)と、残りの `awase-windows-core` を `awase-ime-core` に改名。各 crate は分割前と同じ相対パスを保ち、git mv(類似度 100%)で、merge commit で入れる。
  Opus レビュー(round 1・2)の Blocker 2 件(見落とした循環、D4 の順序)と、黙って検査が外れる 9 か所(pre-push の正規表現・compile_fail doctest・dylint の -p ほか)を反映した。Opus は「今は 7 分割をやらずガードだけ」を勧めたが、所有者の方針に従い、費用を抑える葉から剥がす順にした。
status: |-
  起案・Opus レビュー round 1・2 反映済み(2026-10-20)。設計のみ。実装は未着手(段階 0 から)。
related_adr:
  - "ADR-019"
  - "ADR-229"
  - "ADR-272"
---

# ADR-274: awase-windows-core を役割ごとの複数の core に分ける

## 背景

`awase-windows-core`(ADR-229 D4、2026-10-10 に分離)は、`CLAUDE.md` の説明では「IME-state decisions/types/tables」。
実際の中身は(origin/develop `1cdc8778` 時点、本体+テストの行数):

| クラスタ(案) | モジュール数 | 行数 | 内容 |
|---|---|---|---|
| base | 19 | 6,101 | 定数・仮想キー・キーマップ表・世代・時計・IME 種別・アプリ分類の型 |
| keyeffect | 8 | 8,505 | 学習済みのキー効果の表と予測 |
| imecore | 46 | 34,126 | IME の状態(belief)・観測・書き込み(actuation)の判断、journal |
| plans | 11 | 2,425 | TSF の送信・退避・回収と、フォーカスの分類・解決の「計画」(Facts → Plan) |
| hookinput | 7 | 3,904 | フック・修飾キー・半角英数トグルの状態 |
| 計 | 91 | 55,061 | |

名前は「IME の状態判断」を指すが、キー効果の予測、TSF とフォーカスの計画、フックの状態も同居する。
さらに、OS 非依存なのに殻に残っているコードがある(ECRS 分析、`docs/tasks/ecrs-awase-windows-2026-10-20/INDEX.md`):
`scancode_*`(約 5,000 行。参照元は `awase-settings` とテスト 1 本だけ)と `bug_report.rs`(約 3,100 行)。
本番コードの推定行数は、分割直後 `awase-windows` 43,352 / core 21,087 から、現在 48,720 / 25,024 で、殻は減っていない。

**所有者の方針(2026-10-20)**: awase-windows-core を複数の core に分け、名前を適切に与える。複数の core があってよく、その方が見通しが良い。

## 測定(分割が成り立つか)

`tools/core-split/deps.py` が、モジュール単位の依存グラフを作る(`python3 -I tools/core-split/deps.py crates/awase-windows-core/src tools/core-split/assign.py [--check]`)。
Opus レビュー(`review/274-opus-split-round1.md`)で、当初の版の誤りを直した改良版である:
コメントと文字列を除く(doc コメントの `crate::...` は依存ではない)、`#[cfg(test)]` の付いた**項目だけ**を外す(当初は最初の行頭 `#[cfg(test)]` 以降を丸ごと捨て、
`evidence.rs`・`conv_mode.rs`・`key_effect_rules.rs` の本番コードを読み落としていた)、`a::{b, c}` を展開する、`super::super` を解決する、`state/mod.rs`・`focus/mod.rs` の `pub use` を解決する。
それでもモジュール単位の近似で、型・関数の単位ではなく、マクロ展開は追わない。

結果(本番コード):
- クラスタ間の依存は `keyeffect → base`、`imecore → {base, keyeffect}`、`plans → {base, imecore}`、`hookinput → base`。**`class_names.rs:334` の `impl From<AppImeProfile> for ImePolicyProfile`
  (とそのテスト 584-603 行)を imecore 側の `ime_event.rs` へ移せば、双方向の依存が無くなる(DAG)**。孤児規則の問題ではない(`impl From<Local> for Foreign` は許される)。
  base から imecore の型名を参照できないだけ。
- モジュール単位の循環(強連結成分)は 3 組(`{key_effect_predictor, key_effect_table}`、`{evidence, ime_event, probe_admission}`、`{actuation_chain, app_ime_policy, open_warrant}`)で、どれも同じクラスタの中に入る。
- **当初案から変えた点(Opus の指摘)**:
  - 当初は journal・診断を独立のクラスタ(diagnostics)にしたが、`evidence.rs` → `mozc_shm` → `journal` → imecore の循環があった。観測源(`mozc_shm`・`indicator_menu`・`indicator_glyph`・`indicator_evidence`)は
    `*Record`(journal の型)を返し、journal は imecore のイベント型(`DriftEdge`・`ConvTransition`・`ObservationSource` ほか)を記録する。依存が相互なので、**journal は imecore の crate に含める**。
    このリポジトリの観測の段階の進め方(「観測のみ・journal → belief へ接続」、ADR-265→268、ADR-264→271→273、ADR-261)は、診断に置いたものが後で belief の依存へ移るのが定型で、層を分けるとその都度ぶつかる。
  - `key_sequence_policy`・`physical_disposition`(キー選択・物理キーの配送判断)は、core の利用元が imecore と plans だけなので imecore に置く。`mode_key_pass` も imecore。keyeffect は「学習済みのキー効果の表と予測」だけになる。
  - tsf と focus の「計画」は、どちらも core のどこからも使われない葉なので、小さな crate を 2 つ作らず `plans` にまとめる(1 crate の固定費は行数に比例しない)。
  - `hookinput` は base だけに依存する(`indicator_menu` を imecore へ移したため)。
  - `scoped_latch`(依存なし)は base。
- **belief と actuation は分けない**: 辺が 8 本(actuation → belief)と 10 本(belief → actuation)で、実質 1 つの塊。

## 実測した費用と利益(Opus による)

`git log --since=2026-10-06 -- crates/awase-windows-core/src`(97 コミット)を当初の割り当てで数えた:
- 複数のクラスタにまたがるコミットは 22/97(23%)。分割後は複数の crate の同時変更になる。
- 1 コミットで再ビルドされる crate 数(下流を含む)は平均 4.7/7。変更の多いファイル(`key_effect_predictor`・`tuning`・`vk`・`journal`)が下の層にあるため、「並列ビルド」と「変更の影響範囲が crate で分かる」の利益は小さい。
- 「依存の向きを cargo が強制する」「名前で責務が分かる」は本物の利益。前者はガードテストでもほぼ得られるが、cargo の強制は迂回できない。
- 費用: PR 8〜10 本と準備の PR、open PR との競合(#707 は merge 済みだが `state/mod.rs`・`journal.rs` などは常に動く)、`pub(crate)` → `pub`、ガードの更新、黙って検査が外れる 9 か所(下)。

Opus の推奨は「今は 7 分割をやらず、層の向きをガードで固定する」だった。所有者は「複数の core があってよく、見通しが良くなる」と判断した。**この ADR は、費用を抑える順(葉から)で分割する**。
段階 1 の後に CI の所要時間と、実際の `pub(crate)` → `pub` の個数を測り、段階 2 に進むかを再判断する。

## 決定

### D1: クラスタと層

```
L0  base       (6,101 行)   awase-windows-base  定数・仮想キー・キーマップ表・世代・時計・IME 種別・アプリ分類の型
L1  keyeffect  (8,505)      awase-key-effect    学習済みのキー効果の表と予測、キー配置の仮説
L2  imecore    (34,126)     awase-ime-core      IME の状態(belief)・観測・書き込み(actuation)の判断、journal
L3  plans      (2,425)      awase-windows-plans TSF とフォーカスの「計画」(Facts → Plan の純粋な判断)
L1  hookinput  (3,904)      awase-hook-core     フック・修飾キー・半角英数トグルの状態(base だけに依存)
```

割り当ての全文と許可する依存の向き(`ALLOWED`)は `tools/core-split/assign.py`。加えて、殻から外す OS 非依存の葉の crate:
- `awase-scancode`: `scancode_pairs/editor/diagram/apply.rs` と `scancode_map.rs` の純粋部(`#[cfg(windows)] mod registry` は殻に残す)。
- `awase-bug-report`: `bug_report.rs`。殻に依存する 1 箇所(`BugReportKeymapLearnSummary::from_paths` が `key_effect_io::read_persisted_table` を呼ぶ)は、読み取りの結果を受け取る関数に変えて、殻が読む。
  依存は core(`key_effect_*`・`indicator_evidence`)の下位 crate と `awase-keymap-learn`・`serde`・`flate2`・`base64`。

### D2: 名前(案。所有者が最終決定)

`awase-windows-base`(Windows の世界の語彙の土台。OS 非依存だが語彙は Windows のもの)/ `awase-key-effect`(学習側の `awase-keymap-learn` と対)/
`awase-ime-core`(ADR-191「IME が状態の正」を実装する判断の核。最後に `awase-windows-core` を改名してこれにする)/ `awase-windows-plans`(Facts → Plan の純粋な計画)/ `awase-hook-core` / `awase-scancode` / `awase-bug-report`。
`awase-tsf-core`・`awase-focus-core` は作らない(計画は 1,200 行ずつで固定費に見合わない。中身は `tsf/` ではなく `state/*_plan.rs`)。

### D3: パスを変えない(ADR-229 D4 と同じ手法。Opus の指摘で具体化)

- 各 crate は分割前と同じ `src/` 内の相対パス(`state/foo.rs` など)を保ち、移動は `git mv`(類似度 100%)だけのコミットを分け、merge commit で入れる(squash すると `--follow`・`blame -M` が潰れる)。
  ファイルの中身の変更が要る準備は、移動の PR より前の別の PR に分ける。
- 上位の crate は下位の crate の `state`・`focus`・`tsf` を **glob で再公開**する: 各 crate の `src/state/mod.rs` に `pub use <下位 crate>::state::*;`(モジュールも glob で入る。同じ項目が複数の経路で入っても曖昧エラーにならない。ローカルの `pub mod` が優先)。
  個別の `pub use` を並べる書き方は、下位に 1 つ足すたびに上位すべてを直すことになるので採らない。`focus/`・`tsf/` も複数の crate にまたがるので、`focus/mod.rs`・`tsf/mod.rs`・`lib.rs` も同様に書く
  (`lib.rs` には `pub use awase_windows_base::{vk, tuning, keymap};` など)。
- `state/mod.rs` の `pub use belief::*;`・`hook_state::*;`・`conv_mode::{…}`・`ime_model::AppliedImeState` は、base の `mod.rs` には置けない(上位の型を指す)。imecore・hookinput の `mod.rs` へ移す。
  段階 1 で hook-core を抜くとき、`pub use hook_state::*;`(28 行)は hook-core の `state/mod.rs` へ移る(移動の PR の中の `mod.rs` 編集で、`git mv` の類似度には影響しない)。
- plans の `focus/hwnd_cache.rs` は `focus/` が複数の crate にまたがる例になる: plans の `focus/mod.rs` を `pub use awase_windows_core::focus::*; pub mod hwnd_cache;` とし、core の `focus/mod.rs` から `pub mod hwnd_cache;` を消す。
  殻は `focus/mod.rs:15` の明示列挙のうち `hwnd_cache` だけを `awase_windows_plans::focus::hwnd_cache` に替え、`state/mod.rs:8` に `pub use awase_windows_plans::state::*;`・`pub use awase_hook_core::state::*;` を足す。
  各新 crate の `state/mod.rs` に、core と同じ `#![allow(unreachable_pub)]` を写す。
- `#[macro_export]` の `vk_table!` は `$crate` を使わないので、殻の呼び出し `awase_windows_core::vk_table!` の crate 名の付け替えで足りる。

### D4: 順序

**段階 0(準備。どれも挙動を変えない小さな PR)**
1. `class_names.rs:334` の `impl From<AppImeProfile> for ImePolicyProfile` と、そのテスト(584-603 行)を `ime_event.rs` へ移す。doc(「state 層が focus 層に直接依存しない設計を維持する」)は書き直す。
2. `explicit_press.rs:1751` の `#[cfg(test)]` 内の `impl PhysicalKeyDisposition { fn legacy_… }`(ADR-208 L0 の逐語コピー、コメントに「L1 以降で削除してよい」)を削除するか自由関数に書き換える(inherent impl は別 crate の型に書けない)。
3. `tools/core-split/deps.py --check` を CI のジョブにする。走査対象は引数の 1 ディレクトリではなく、**core 系 crate の `src/` の和**(コロン区切りで複数渡す。段階 0 の 5 の一覧から作る)。許可されていない依存(本番とテストの両方)、未割当のモジュール、
   `assign.py` にあるのに走査で見つからないモジュール(移した crate が走査から漏れた)で落とす。走査が 1 ディレクトリのままだと、段階 1 で葉を別の crate へ移したとたんに、それらのファイルの検査が黙って消える。
4. `[workspace.lints]` へ lints を寄せる(`[lints]` の 25 行が crate の数だけ複製されるのを避ける)。
5. **core 系 crate の一覧を 1 か所に置く**(`tools/core-split/` か `xtask`。例: ワークスペースのメンバーのうち core に推移的に依存するもの)。次をそこから作るか、一覧と照合して落とす(足し忘れが BUG-116 と同じ穴になるのを防ぐ):
   - `EXTRA_SRC_CRATES`(`tests/support/src_roots.rs`)と、`layer_boundary_guard.rs` の `crate_does_not_depend_on_windows`。
   - `.githooks/pre-push:38,86` の正規表現(`crates/awase-[a-z-]+/src/` のように crate 名を問わない形でもよい)。段階 1 の hook-core と plans は再発ファミリーのファイルを含む(半角英数トグル・物理キー押下ラッチ・warmup・defer/replay・focus 遷移)ので、最初の PR から効く。
   - `xtask-adr-evidence/src/core_registry.rs:53-54`(パスの直書き)と `main.rs:243`(読む mutants の設定が 2 ファイル固定)。
   - `ci.yml` の `-p`(57・191・336・404 行)と mutants の matrix(`ci.yml:658`、`mutants-windows.yml:50`)。
   - `deps.py` の走査対象(下の 3)。
6. **統合テストの扱い**: `ci_test_coverage_guard.rs` は `crates/awase-windows/tests/*.rs` しか見ず、`cargo nextest run --workspace --lib` は統合テストを走らせない。移した crate の `tests/` は黙って走らなくなるので、統合テストは殻に残す(再公開で `awase_windows::scancode_*` が引ける)か、ガードの走査対象を core 系 crate の一覧に広げる。

**段階 1(葉から剥がす。`awase-windows-core` の名前とパスは変えない)**
1. `awase-hook-core`(hookinput)。依存は base だけで、準備の変更が要らない。再公開の glob と段階 0 の 5 の仕組みの検証になる。
2. `awase-windows-plans`(plans)。同様に準備の変更が要らない。
3. `awase-scancode`・`awase-bug-report`(殻から)。**準備の PR が先に要る**: `scancode_map.rs` は純粋部と `#[cfg(windows)] mod registry`(82・227 行)が同じファイルにあるので、registry を別ファイルへ分ける。
   `bug_report.rs` は `BugReportKeymapLearnSummary::from_paths` が殻の `key_effect_io::read_persisted_table` を呼ぶ箇所を、読み取りの結果を受け取る形に変える。そうしないと `git mv`(類似度 100%)で移せない。

**判断点(段階 1 の後)**。後から「見通しが良くなった」で決めると検証にならないので、基準を事前に決める:
- (a) CI の `test`・`clippy`・`windows-test` の所要時間の増加が N 秒以内(段階前の 3 run の中央値と比べる。N は段階 1 の最初の PR の前に所有者が決める)。
- (b) 段階 1 の後の 2 週間に、hook-core か plans と core の両方を触った PR の割合(「クラスタをまたぐコミット 23%」の実測版)。高ければ、葉の分割すら同時変更を増やしている。
- (c) 段階 1 の PR のレビュー・CI で見つかった「D5 の穴」の件数。0 件でなければ段階 0 の自動化が足りない。
- (d) 段階 2 に進む場合、先に `deps.py` で base クラスタの変更頻度を数え、base を剥がしたときの再ビルドの広がりを見積もる(base は全員が依存する土台で、`tuning`・`vk`・`converter_target`・`class_names` を含む。葉 2 つの測定からは外挿できない)。
- 「`pub(crate)` → `pub` の個数」は判断材料にしない(クラスタをまたぐ `pub(crate)` は構造上どの段階でも 0)。
- **「段階 2 に進まない」も正規の結論**。その場合 `awase-windows-core` には base + keyeffect + imecore が残る。名前を `awase-ime-core` に改名するか、description と `CLAUDE.md` だけ直すかは、未決 3 で決める。

**段階 2(下へ剥がす)**
1. `awase-windows-base`(base)。`windows-core` は base を再公開する。
2. `awase-key-effect`(keyeffect)。
3. 残った `awase-windows-core`(= imecore)を `awase-ime-core` に改名し、`awase-windows` が各 crate を直接使う形にする。

**各 PR の共通の後処理**(ADR-229 のタスク表 §0 の 4 に加え、下の「黙って検査が外れる箇所」):
`EXTRA_SRC_CRATES`、`.github/workflows/ci.yml` の対象 crate と paths トリガー、`.cargo/mutants-<crate>.toml`、`.githooks/pre-push` の正規表現、
`fix-requires-evidence.md` の表のパス注記、`CLAUDE.md` の crate 一覧、`xtask-adr-evidence` の `core_registry`、`mise.toml` の `-p`。

### D5: 黙って検査が外れる箇所(Opus の指摘。新しい crate を足す PR ごとに確認する)

CI が緑のまま検査が外れる:
1. `.githooks/pre-push:38,86`: 再発ファミリーの正規表現が `crates/awase-windows(-core)?/src/…`。新しい crate 名に当たらず、`ime_model.rs`・`observation_store.rs` などが fix-requires-evidence の警告の対象から外れる(BUG-116 と同型)。
2. `ci.yml:57`: `compile_fail` doctest が `-p awase-windows-core` だけ(**危ないのは段階 2 の 3 の改名の時点だけ**。5 個はすべて imecore にあり、段階 1・2 の 1・2 では移動しない)。`ime_actuation.rs:155`・`gji_direct_mechanism.rs:182`・`observation_store.rs:80,177`・`actuation_chain.rs:93` の 5 個は `use awase_windows_core::state::…` と書いており、
   package 名を変えると `-p awase-windows-core` は「package が無い」で大きな音を立てて落ちるが、doctest の中の `use awase_windows_core::…` を直し忘れると `use` の解決失敗で「間違った理由で」コンパイルに失敗し、合格し続ける(型状態の証明が黙って空になる)。
   改名の PR で crate 名を書き換え、各 `compile_fail` と対になる「同じ `use` で正しい形はコンパイルできる」doctest を置く。
3. `ci.yml:191`: dylint の `-p`。`ImeEvent` を組み立てる reducer(`ime_model.rs`)の crate を足さないと lint が外れる。
4. `ci.yml:336`(clippy)・`ci.yml:404`(windows の nextest)の `-p`: 新しい crate の lint と Windows のテストが走らなくなる(Linux の単体テストは `cargo nextest run --workspace --lib`、`ci.yml:54` が新しい crate も自動で拾うので外れない)。
5. `tests/support/src_roots.rs` の `EXTRA_SRC_CRATES`: 足し忘れると `architecture_guard.rs` の「列挙されないファイルは 0 件」型のガードがその crate を走査しないまま合格する(段階 0 の 5 で検出する)。
6. `layer_boundary_guard.rs` の `crate_does_not_depend_on_windows`: `../awase-windows-core/Cargo.toml` の 1 つだけを見る(段階 0 の 5)。
7. mutants: `.cargo/mutants-awase-windows-core.toml` の `test_package` と **`examine_globs`**(plans の 10 本・hookinput の 5 本が並んでいる。移すと存在しないパスになり、その変異は誰も検査しなくなる。`xtask core-registry` は「base 比で足された名前」しか見ないので移動は検出しない)。
   新しい `mutants-<crate>.toml` を作り、`ci.yml:658` の `pkg` に足す。分割後の各設定の `test_package` には、その crate より上の crate をすべて入れる(上位でしか捕まらない変異が false MISSED になる)。`pkg × 8 shard` なので crate が増えるとジョブ数が増える。
8. `config-verify.yml` の `paths:`(`crates/awase-windows-core/src/vk.rs`・`keymap.rs`)。base へ移すと起動しなくなる(段階 2 の 1 まで関係しない)。
9. 凍結コーパス `crates/awase-windows-core/tests/journals/actuation_decision/`(`actuation_decision_record.rs:1270` が `CARGO_MANIFEST_DIR` から読む)。`actuation_decision_record.rs` は imecore に残るので、パスが変わるのは**ディレクトリ名を変える場合だけ**(未決 3)。変えるなら `docs/journal-replay-guide.md`・`complexity-budget.md`・`fix-requires-evidence.md` の注記・メモリのパス、`git log -- <path>` が一斉に動く。package 名だけなら、ほぼ D5-2 だけで済む。
10. **移した crate の統合テスト**: 段階 0 の 6 のとおり、どこからも実行されなくなる。
11. **`deps.py` の走査対象**: 段階 0 の 3 のとおり、1 ディレクトリのままだと分割の途中から検査範囲が縮む。

音を立てて落ちる(直せば済む): `xtask-adr-evidence/src/core_registry.rs`(直書き)、`main.rs` が読む mutants 設定、`mise.toml`、ルート `Cargo.toml` の `members`、
`testing` feature(殻の `[dev-dependencies] awase-windows-core = { features = ["testing"] }` を、各 crate の `testing` へ転送する)、`FocusGen::new`(`cfg(any(test, feature = "testing"))`)を使うテストの dev-dependency。

### D6: 検証

ローカルではビルド・テストを走らせず、CI(`windows-cross-check`・`test`・`clippy`・`dylint`・`windows-build`・`windows-test`)で確かめる。
各 crate が `windows` crate に依存しないことをガードで固定し、層の向きは `Cargo.toml` の依存(循環は cargo が拒否する)と `deps.py --check` で固定する。

## 影響と注意

- 公開範囲: 層をまたぐ `pub(crate)` は `key_sequence_policy` の 4 項目だけ(imecore に置くので 0 になる)。`unreachable_pub` は、再公開の glob で足りる想定。実際の個数は段階 1 で測る。
- `ImeStateHub` の記録系 `record_*` の `pub(crate)`(INV-A97-1)は `platform_state.rs` が殻に残るので影響しない。`platform_state.rs`・`sync_actuation.rs` の core 移動(約 5,800 行、ECRS R1)は別の判断で、本 ADR の範囲外。
- `imecore` は 34,126 行で、分割後も 6 割を占める。belief と actuation の結合を弱めない限り分けない。分けるなら別の ADR。
- 履歴: `git log -- <path>` で辿る運用(`tuning-constants.md`・`experiment-logging.md`)は `--follow` が要る。移動と同時に中身を変えたコミットが混ざると履歴が切れるので、中身の変更は先の PR に分ける。

## 未決(所有者または実装時に決める)

1. D2 の名前(特に `awase-windows-base`・`awase-windows-plans`)。
2. `awase-scancode` の移動先。`awase-settings` の中に置く案もある(参照元が `awase-settings` だけのため)。
3. `awase-windows-core` を最後に `awase-ime-core` へ改名するか、そのまま残すか。改名する場合は、package 名だけか、ディレクトリ `crates/awase-windows-core/` も変えるか(後者は D5-9 の影響が大きい)。
4. 段階 2 に進むかの判断(段階 1 の後)。
