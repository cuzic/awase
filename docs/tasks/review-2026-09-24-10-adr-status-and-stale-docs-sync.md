---
title: ADR status/index の追随、撤去記録の欠落、撤去済み機構を現在形で書く文書・コメント、CLAUDE.md 一覧、pre-push 対象
status: 未着手
created: 2026-09-24
related_adr: ["ADR-191", "ADR-187", "ADR-189", "ADR-190", "ADR-195", "ADR-196", "ADR-176", "ADR-179", "ADR-185", "ADR-090", "ADR-121", "ADR-098", "ADR-153", "ADR-172", "ADR-173", "ADR-158"]
source_review: 俯瞰レビュー（2026-09-24）の A-4 / A-5 / A-6 / A-7 / B-6 / B-8
---

# ADR status・撤去記録・古い文書の同期（俯瞰レビュー A-4/A-5/A-6/A-7/B-6/B-8）

索引: [11](review-2026-09-24-11-low-priority-backlog.md)。裏取りは worktree HEAD `5877f982`（origin/develop、PR #296 マージ後）で行った。ADR・index の編集は `.claude/rules/docs-frontmatter-convention.md`（index に長文を書き戻さない、全文は frontmatter）に従う。**本タスクファイル作成時点では ADR/index/コードは編集していない。**

`related_adr` に ADR-178 を入れていないのは意図的。`docs/adr/178-*` は MSI アンインストールの ADR で、本タスクが扱う「ADR-178撤去プロジェクト」とは別物（A-5 参照）。

## 作業単位（PR を3つに分ける）

検証の手段とレビューの粒度が違うので、次の3つに分けて別 PR にする。

| 単位 | 範囲 | コード変更 | 検証 |
|---|---|---|---|
| **P1** docs のみ | A-4、A-5、A-6 の文書部分、A-7 | なし | Linux: CI `adr-index-consistency` と同じ grep、下記の grep 確認 |
| **P2** awase-windows のコメント・ログ文言・死蔵コード | A-6 のコード部分、B-6 | あり（`crates/awase-windows`） | Linux: `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`、`cargo nextest run -p awase-windows --test architecture_guard --test layer_boundary_guard`。windows-build CI |
| **P3** pre-push と rules | B-8 | なし（hook と rules） | Linux: `cargo run -p xtask-adr-evidence -- .`（CI `adr-evidence-consistency` と同じ）、hook の直接実行 |

P1〜P3 のあいだに順序の依存はない。3つとも保留にしない（B-6 の削除部分だけは保留可）。俯瞰レビュー D節は B-8 を「保留でよい」に入れていたが、B-8 は `.githooks` の正規表現と rules の表の数行だけの変更で、[01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md)・[05](review-2026-09-24-05-startup-desired-open-forced-on.md) の修正がルールの対象になるかどうかがこれに依存するため、先に入れる。

## A-4: ADR の status が実装に追いついていない

### 事実の訂正（他タスクを待たずにすぐ直せる）

以下は「未マージ」「撤去ブランチ」など、**現時点で事実に反する**記述。01/06/07 の判断を待たずに直す。各コミットが HEAD に含まれることは `git merge-base --is-ancestor <hash> HEAD` で確認済み。

| ADR | 現状の status（frontmatter、`5877f982`） | 実際 |
|---|---|---|
| ADR-191（本体 `191-ime-is-source-of-truth-observe-not-write.md`。`191-calibration-experiments.md`・`191-gji-state-scope-spec.md` は補助資料なので編集しない） | status 17〜18行「草案（2026-09-21）…撤去ブランチ`feat/adr191-remove-hardcoded-mode-keys`で実装済み（develop未マージ…）」。**summary にも同じ古い記述がある**: 15行 (6)「実装は`feat/adr191-remove-hardcoded-mode-keys`（develop未マージ）」、11行 (3)「BUG-151は撤去ブランチ（…）で扱う」 | PR #240（`d777bcfe`）で develop に入っている。status 17〜18行と summary 15行を直す。summary 11行 (3) と status 19行「PR #238 は取り下げ、撤去ブランチで扱う」は取り下げの経緯を述べた文で誤りではないので、「（撤去ブランチは PR #240 で develop にマージ済み）」を添えるだけにする。summary (2) と本文（決定1-1・RM3）の食い違いは [08](review-2026-09-24-08-open-close-fixed-set-vs-custom-keymap.md) が直す（10 は触らない） |
| ADR-187 | 「決定・実装済み(未マージ)」 | `c949ba33` が develop に入っている |
| ADR-189 | 「[ADR-191で範囲を拡張]…決定・実装済み(未マージ)・CI検証済み」 | `651cab8d` が develop に入っている |
| ADR-190 | 「実装済み(未マージ)・CI実機E2Eで検証済み」 | `feb49ffd` が develop に入っている |
| ADR-195 | frontmatter status 44行目に「ADR-196自体はまだ実装中（PR #259・#260、develop未マージ、別セッション担当）」。`docs/adr/index.md:202` の行にも「(ADR-196自体は未マージ)」 | ADR-196 関連 PR は develop にマージ済み（#259・#260・#263・#265・#269・#273・#275・#278・#279・#281・#283・#284・#285・#288・#293・#294・#296 など。ブランチ名に `adr196` を含むマージコミットで確認）。frontmatter と index の両方を直す |
| ADR-179 | frontmatter「収束済み（round1〜8）…」、index:186「収束・実装着手可」、本文44行「決定1・2は実装済み」 | 中核の `ModeKeyActuationOwner`（決定2）は `502c6673`（HEAD に含まれる）で撤去済み。コード中の `ModeKeyActuationOwner` は0件。本文に「191」は0件。frontmatter・index・本文44行の3箇所を直す |
| ADR-121 | 「D1実装済み・実機未検証（PR #188）」 | reassert（D1）は `f83084b3` で撤去済み。本文に撤去の記録なし（「撤去」の出現は代替案 A3 の節だけ） |
| ADR-098 | 「実装済み（…2026-08-21）。決定0/1-a/1-b/1-c/…」 | 決定1-c の force-on クールダウンは `621bf93c` で撤去済み。本文に撤去の記録なし |
| ADR-153 | 「決定1実装済み・developマージ済み（PR #185）…」 | GJI/MS-IME 設定からの自動採用は ADR-191 で撤去済み。記録は `src/config.rs:361` のコメントだけ。ADR 本文にはない |

### 他タスクの判断を待つもの（暫定注記に留める）

| ADR | 現状 | 待つ判断 |
|---|---|---|
| ADR-196 | 「草案rev5…実装着手可」 | 実装の大部分はマージ済みなので「草案」は事実として古い。「実装済み（一部未完）」への変更はすぐにできるが、採用の仕組みのずれは [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md) の結論を待って書く |
| ADR-195 | 段階8 | 配線が未反映（[06](review-2026-09-24-06-keymap-learn-staleness-wiring.md)）。上の「未マージ」の訂正はすぐ行い、段階8の記述だけ 06 を待つ |
| ADR-176 | 「2026-09-17: 実機A/B検証完了…」 | 適用側は ADR-191 の撤去で消えている。撤去の記録は `176-implementation-tasks.md:844-846` の追記だけ。**ADR-176 本体の status は [07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md)(2) が直す（07:120）。10 は編集しない**。index.md の 176 行の短縮ステータスだけ、07 の結果に合わせて 10 で追随させる |

## A-5: 「ADR-178撤去プロジェクト」の記録がない・番号衝突・ADR-172/173 が develop に無い

### 番号衝突の原因

- `docs/adr/index.md:186`（179の行）に「元178番、developマージ済みの別ADR-178(msi-uninstall)と衝突し179へ採番し直し」とある。つまり「ADR-178撤去プロジェクト」「ADR-178領域A」は、**現在の ADR-179 の旧番号**で呼ばれている。ところが ADR-179 本文には領域A/C の記述が無い（`grep -n 領域` で0件）。
- 撤去の根拠（drift correction だけ残した理由、warmup は読み取り専用ゲートだったので対象外という訂正）は、`f83084b3`・`621bf93c`・`f5338edc`（ADR-185）のコミット本文と `docs/adr/090-*.md:682` 付近に散らばっている。
- `docs/adr/090-*.md:682` は `[ADR-178](178-msi-uninstall-preserve-userdata.md)` と、**MSI の ADR にリンクしている**。リンク先が誤り。
- 撤去プロジェクト（＝現 ADR-179 の旧番号）を「ADR-178」と呼んでいる場所。`5877f982` で `grep -rn "ADR-178" docs .claude crates src CLAUDE.md ARCHITECTURE.md` を実行し、MSI 側（`178-msi-*`・`178-opus-review-*` と、コード中の「ADR-178 決定N」「v14」参照）を除いて手で判定した結果:
  - 「領域」「撤去」が続く表記: `.claude/rules/fix-requires-evidence.md:41`、`docs/adr/180-*.md:9`、`182-*.md:582`、`185-*.md:5`、`docs/adr/index.md:192`、`docs/known-bugs/BUG-135.md:13`、`BUG-136.md:13`、`docs/tasks/actuation-confluence-inventory.md:75`、`docs/tasks/review-2026-09-24-09-*.md`、`crates/awase-windows/src/ime_controller.rs:603`、`runtime/open_chain.rs:636`。
  - 「領域」「撤去」が直後に続かない本文: `docs/known-bugs/BUG-146.md:31`（「ADR-178がforce-ONを撤去した」）、`docs/adr/182-*.md:558`、`184-gji-atok-muhenkan-toggle-*.md:154`。`180-*.md:51` は「ADR-178（`docs/adr/179-*.md`=ADR-179…）」とすでに対応を書いているので対象外。
  - frontmatter の `related_adr: "ADR-178"` が MSI の ADR を指してしまっているもの: `docs/known-bugs/BUG-135.md:6`、`BUG-136.md:6`、`docs/adr/185-directinput-open-axis-write-teardown.md:23`。ADR-179 に置き換える。
  - コード: `crates/awase-windows/src/runtime/key_pipeline.rs:1521` の「ADR-178 round4/round8」（2026-09-17）。`git log -S"ADR-178 round4/round8"` の導入コミットは `e475b600`「docs(adr-178): 設計をround3〜8まで発展させModeKeyActuationOwner案に収束」で、MSI 側ではなく**現 ADR-179 のレビュー round** を指す。置き換え対象（P2、コメントのみ）。
  - opus レビュー記録（`184-opus-review-round3.md:33,304,311,478`、`round4.md:14,48`、`round6.md:304`）は当時の記録なので**書き換えない**。読み手は ADR-179 に追記する節の「旧称」の説明で対応を辿れる。

### 方針

新しい ADR は起こさない。**ADR-179 に「領域A・C の撤去（旧称: ADR-178撤去プロジェクト）」節を追記**し、`f83084b3`・`621bf93c`・`f5338edc` の根拠と、warmup を対象外にした理由をまとめる。番号の由来とも合い、新しい番号を増やさずに済む。そのうえで、上に挙げたファイル（opus レビュー記録を除く）の「ADR-178」を「ADR-179（旧178）」に置き換え、`related_adr` の3件を ADR-179 にし、`090-*.md:682` のリンクを ADR-179 に直す。コード3箇所（`ime_controller.rs:603`、`open_chain.rs:636`、`key_pipeline.rs:1521`、P2）はコメントだけの変更。`docs/tasks/review-2026-09-24-09-*.md` は置き換えない: 09 の frontmatter `related_adr` にはすでに ADR-178 が無く、残る 09:15 の「ADR-178撤去プロジェクト領域A」は番号衝突そのものを説明する文なので、直す必要がない。

ADR-179 への追記は2段階に分ける。**P1 では撤去の事実と根拠（コミット3本・warmup を対象外にした理由）だけを書く**。[09](review-2026-09-24-09-remaining-active-writes-inventory.md) の残存書き込みの A/B 結果は、09 が終わってから同じ節に追記する。こうすれば P1 は 09 を待たずに出せる。

### ADR-172/173

- 両 ADR の docs コミットは origin のアーカイブタグで保存されている: `refs/tags/archive/adr172-tsfnative-rescue-consolidation`（`f0868bcb`）、`refs/tags/archive/adr173-solo-tap-ime-action-by-process-name`（`cbb412cf`）。`git ls-remote --tags origin 'archive/adr17*'` で確認済み。**救出の作業は不要**。develop に本文ファイルが無いだけ。
- BUG-142、ADR-174、ADR-175 は ADR-172 を参照している。`docs/known-bugs/BUG-142.md:51` は `../adr/173-scope-solo-tap-ime-action-by-process-name.md` にリンクしているが、develop にこのファイルは無く、リンク切れになっている。
- `docs/adr/index.md` には 172・173 の行が無い（171の次が174）。
- **CI の制約**: `adr-index-consistency`（`.github/workflows/ci.yml:215`）は、index 中の `| [NNN](file)` 形式の行について、本文ファイルが存在するかを検証する。172/173 の行を**リンク形式で**足すと CI が落ちる。
- 方針: index の 171 と 174 の間に、リンク無しの `| 172 | … | アーカイブ（タグ archive/adr172-…、develop 未収録） |` 形式で1行ずつ足す（CI の grep `^\| \[[0-9A-Za-z]+\]\(` に掛からない）。BUG-142:51 のリンクと ADR-174/175 の ADR-172 への参照には、同じ注記を付ける。本文をタグから develop に取り込む案は、ADR-172 が「コード変更なし」で収束したものであり、取り込む利点が小さいので採らない。

## A-6: 撤去済みの機構を現在形で説明する文書・コメント

### 文書（P1）

- `ARCHITECTURE.md:43`（3層構成の「3. **SSOT フォールバック**」）から `:47`（「未知の IMM-broken アプリへの初回フォーカス時」）までの段落全体。`try_force_on_bootstrap` は `621bf93c` で撤去済み。`:47` の `imm_cache.toml` を、`:80` と同じ `cache.toml`（旧 `imm_cache.toml`）表記にそろえる。
- `docs/ime-control-overview.md`: `:319`（`ForceOnReason` の列挙に `BrokenAppBootstrap`）、`:328`、`:359`（`miss_count ≥ 3` → `force_on_broken_app_bootstrap`）、`:469`（drift_monitor の閾値3 → force_on 発動）。`:276` の `drift_monitor` 自体は観測失敗の追跡として残っているかどうかを、書き換えのときに確認する（未確認）。
- `.claude/rules/fix-requires-evidence.md` の合流点行は、撤去済みと明記済みなので対象外（「ADR-178」表記の置き換えは A-5 で行う）。

### コードコメント（P2）

`try_force_on_bootstrap` と `apply_force_on_for_imm_broken` の**関数定義は現存しない**。`grep -rn "fn try_force_on_bootstrap\|fn apply_force_on_for_imm_broken" crates/awase-windows/src` の結果は0件。参照は約30件あり、行番号で列挙すると漏れるので、**次の grep の全ヒットを判定する**ことを作業の定義にする。

```sh
grep -rn "try_force_on_bootstrap\|apply_force_on_for_imm_broken" crates/awase-windows/src
```

各ヒットを「過去形（撤去済み・削除した）」「撤去コミットへの参照付き」「現在形」に分け、現在形を0件にする。`5877f982` での現在形の例: `force_guard.rs:206`（「閾値到達で `Runtime::try_force_on_bootstrap()` が…追加する」）、`output/conv_actuation.rs:56-71`、`ime_controller.rs:222, 405, 550`、`platform.rs:1530`、`ime.rs:1039`、`runtime/executor.rs:152`、`runtime/ime_refresh.rs:532, 559, 619, 663, 939`、`runtime/mod.rs:645, 848, 979`、`runtime/key_pipeline.rs:443`、`runtime/message_handlers.rs:783`、`state/ime_event.rs:344, 347`、`state/platform_state.rs:1134, 2482`、`state/ime_actuation.rs:367`、`state/eisu_recovery.rs:26`、`state/actuation_chain.rs:250, 389`、`state/open_warrant.rs:1166, 1187`。`ime_controller.rs:604` と `runtime/mod.rs:887-889` はすでに過去形なので、「ADR-178」表記の置き換え（A-5）以外は不要。

コメントだけの変更は挙動が変わらないので、`experiment-logging` の対象外。

### 本番のログ文言（P2、コメントとは分けて扱う）

`state/platform_state.rs:1341` の `tracing::warn!("IME detection failed {miss} consecutive times, will force IME ON")` は本番コードにある（`mod tests` は1936行から）。force-ON は `621bf93c` で撤去済みなので、このログは実際には起きない挙動を宣言しており、不具合報告のログを読む人を誤らせる。文言を「force-ON は撤去済み。観測失敗の回数だけ記録する」旨に直す。ログ出力の変更なので、コミット本文にその旨を書く。

## A-7: CLAUDE.md の workspace 一覧（P1）

- workspace の `members`（`Cargo.toml:2`）と `crates/` には `awase-keymap-learn-win`、`xtask-adr-evidence`、`measured-macro`、`actuation-choke-point-macro` があるが、CLAUDE.md（`:96` 付近）の一覧に無い。
- `awase-keymap-learn` の説明は「traversal planner + offline simulator (… ADR-191)」のまま。`5877f982` の実モジュールは anomaly, cost, exec, external_write, graph, judgement, metrics, minimize, mismatch_tag, model, persist, remeasure, revalidation, rng, sample_models, sim, staleness, strategy, table, verify, walk_trace で、学習・自己検証・永続化・再測定・陳腐化検出・再検証まで含む。製品化の ADR（ADR-195/196）も併記する。
- `/home/cuzic/rust-nicola`（メインの作業ツリー）には、別セッションの `crates/awase-keymap-learn/src/{staleness,verify}.rs` の未コミットの差分がある。一覧は develop 上のモジュール構成を基準に書く。

## B-6: 撤去後に使われなくなったコード（P2、保留可）

- `ForceOnReason::BrokenAppBootstrap`（`state/force_guard.rs:25`）を**追加する本番コードは存在しない**。使用箇所は、enum 定義を除いて全て `mod tests` の中にある（`force_guard.rs` の tests は239行から、`open_warrant.rs` は223行から、`ime_model.rs` は1104行から）。本番に残っているのは次の2つだけ。
  - `state/platform_state.rs:1344-1347` の `if update.clear_force_on_broken_app_bootstrap { … remove(ForceOnReason::BrokenAppBootstrap) }`（空集合に対する remove）
  - その入力の `observer/ime_observer.rs:40, 53, 72, 82, 92, 105, 115, 122, 249` の `clear_force_on_broken_app_bootstrap` フィールド（テストは `:397, 445, 469`）
- 削除対象: enum variant、`clear_force_on_broken_app_bootstrap` フィールドとその配線、関連テスト、および `open_warrant.rs:28, 100, 330` と `ime_model.rs:420` の説明コメント。
- `state/open_warrant.rs:1127` の `old_is_eligible_for_ime_force_on`: テスト内の旧実装比較用で、現存を確認した。上と一緒に整理するか残すかは削除時に判断する。
- `ime.rs:1733` の `pub unsafe fn set_ime_mode`（[09](review-2026-09-24-09-remaining-active-writes-inventory.md):67・107 から受け取った項目）: `grep -rn "set_ime_mode(" crates src` のヒットは定義行だけで、呼び出し元は0件（`5877f982` で確認）。本体は `set_ime_mode_for_target`（`ime.rs:1757`、トレイのリセット `runtime/message_handlers.rs:1291` が使う）へ委譲しているだけ。`pub` なので dead_code 警告は出ない。`set_ime_mode` だけを削除対象に加え、`set_ime_mode_for_target` の doc（`ime.rs:1751`「[`set_ime_mode`] のターゲット指定版」）の intra-doc リンクも直す（`set_ime_mode` 自身の doc〈`:1727` 付近〉は一緒に消える）。`ime.rs` は pre-push の対象なので、純粋な削除である旨をコミット本文に書く。
- `ime_controller.rs:220-222` の `MechanismCommand::SetOpenCrossProcessSync` の分岐はコメントで「`try_force_on_bootstrap` 等の完全同期呼び出しのみ到達」と書いている。bootstrap が消えたいま、この分岐に本番で到達する経路が残っているかは**未確認**。`state/ime_actuation_decision.rs:269-287` がこの値を返す条件から追って、残っていなければ死蔵コードの候補に加える。
- 削除は `state/`（IME belief ファミリー）に掛かり、pre-push が警告を出す。挙動を変えない純粋な削除である旨をコミット本文に書き、警告は無視してよい。
- ADR-176 一式は [07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) の担当。
- `speculative_delay_ms` と `muhenkan_solo_tap_dedicated_fn_key` が設定画面に無いのは、隠し設定として意図的なもので、問題ない。

## B-8: pre-push と rules が新しい予測の仕組みを見ていない（P3）

- `.githooks/pre-push:36` の `target` 正規表現の `state/(ime|conv_mode|observation_store)` は、`state/key_effect_predictor.rs`・`key_effect_runtime.rs`・`key_effect_table.rs` にマッチしない。`KeyEffectPredicted` は belief を直接動かす。`.claude/rules/fix-requires-evidence.md` の表にも `key_effect` の記述が無い（grep 0件）。
- **実行される hook**: `git config --show-origin core.hooksPath` の結果は `file:/home/cuzic/rust-nicola/.git/config  /home/cuzic/rust-nicola/.githooks`。実行されるのは**追跡下の `.githooks/pre-push`** で、`.git/hooks/pre-push` は実行されない（中身は `:28` に古い正規表現が残っているが、使われていない）。`fix-requires-evidence.md:95` の「実行されるのは`.git/hooks/pre-push`側」は古い記述なので直す。
- **注意**: `hooksPath` はメインの作業ツリーの `.githooks` を絶対パスで指している。worktree から push しても、実行されるのはメインの作業ツリーでチェックアウト中のブランチの版（現在は develop）。worktree のブランチで hook を直しても、その push では新しい hook は動かない。
- **CI との依存**: CI `adr-evidence-consistency`（`.github/workflows/ci.yml:202-208`、`cargo run -p xtask-adr-evidence -- .`）は、`fix-requires-evidence.md` の表に書かれたバッククォート付きのパスを、`.githooks/pre-push` の `target` が全てカバーしているかを検証する。表に `state/key_effect_*.rs` を足すなら、`target` も**同じコミットで**更新しないと CI が落ちる。
- **05 から渡された判断**（05:84-86「`state/platform_state.rs` を対象に加えるかは 10 で判断」）: **加える**。`platform_state.rs` は `ImeStateHub` と `check_drift_correction`（`:1091`）を持ち、belief と desired を直接動かす。`state/mode_key_pass.rs` も `ImeStateHub` から切り出した通過マークの状態機械（ADR-187、BUG-157/158）で、`desired_open` の揃えを判断する。どちらも現在の `state/(ime|conv_mode|observation_store)` にマッチしない。`target` を `state/(ime|conv_mode|observation_store|platform_state|mode_key_pass|key_effect_)` にし、`fix-requires-evidence.md` の IME belief 行にも同じ3種を**同じコミットで**足す。
- 05:85・05:115 は「実行される `.git/hooks/pre-push:28`」と書いており、上の事実（実行されるのは `.githooks`）と食い違う。05 の担当なので本タスクでは直さず、05 側に訂正を依頼する（「他ファイルとの依存」参照）。

## タスク

- [ ] **P1** A-4「事実の訂正」表: 各 ADR の frontmatter status と `docs/adr/index.md` の短縮ステータスを直す。ADR-179 は本文44行も直す。ADR-121/098/153 には撤去の追記（コミットハッシュ付き）を1段落ずつ足す。
- [ ] **P1** A-4「他タスクの判断を待つもの」: ADR-196 の「草案」表記と ADR-195 の「未マージ」表記はすぐ直す。採用のずれ・段階8 の記述は 01/06 が終わってから確定する。ADR-176 本体の status は 07 が直し、10 は index の 176 行だけ追随させる。
- [ ] **P1** A-5: ADR-179 に「領域A・C の撤去」節を追記する（事実と根拠のみ。09 の A/B 結果は後から追記）。A-5 の一覧の「ADR-178」を「ADR-179（旧178）」に置き換える（opus レビュー記録と 09 のタスク文書を除く）。`related_adr: "ADR-178"` の3件を ADR-179 にする。`090-*.md:682` のリンクを直す。index に 172/173 のリンク無しの行を足す。BUG-142:51、ADR-174/175 の参照にアーカイブタグの注記を付ける。
- [ ] **P1** A-6 文書部分: `ARCHITECTURE.md:43-47` と `docs/ime-control-overview.md` の該当箇所を過去形に直すか削除する。`imm_cache.toml` 表記をそろえる。
- [ ] **P1** A-7: CLAUDE.md の一覧に4 crate を足し、`awase-keymap-learn` の説明と ADR 番号を直す。
- [ ] **P2** A-6 コード部分: 上の grep の全ヒットを判定し、現在形を0件にする。`platform_state.rs:1341` のログ文言を直す。A-5 のコードコメント3箇所（`ime_controller.rs:603`、`open_chain.rs:636`、`key_pipeline.rs:1521`）の「ADR-178」を置き換える。
- [x] **P2** B-6（実装済み）: `BrokenAppBootstrap` 一式を削除した（variant・`clear_force_on_broken_app_bootstrap` とその配線・専用テスト・説明コメント。`ime.rs::set_ime_mode` は develop で既に無い）。
  - `open_warrant.rs` の parity テストは、ヒューリスティック guard の次元を落とした（`EXPECTED_OLD_ONLY_COUNT` 8→4。消えた4件は旧 `BrokenAppBootstrap` の分）。
  - **残り（後続候補）**: `overrides_explicit_intent`（全 reason が true になった）、`active_heuristic_reason`（常に None）、`issue_open_warrant` の Step 4b、`HeuristicGuessSource::Guard` は、到達する reason が無くなった。ADR-087 の設計に紐づくので、削除は別途判断する。
  - **`SetOpenCrossProcessSync` の到達経路**: 消えてはいない。同期 `ImeController::apply` の呼び出し元は `platform.rs:1635`（`apply_ime_open_with_view`）と `key_pipeline.rs:1585`（shadow toggle OFF の同期分岐）で、後者は `imm_cross_is_first_applicable` が偽のときだけ通る（真なら非同期チェーン）。ImmCross が先頭でなければ同期チェーンに ImmCross が入らないはずだが、チェーン構成の全網羅は未確認。死蔵と断定するには、`decide_attempt(.., Sync, ImmCross, ..)` に到達する入力の有無をテストか網羅で確かめる必要がある。今回は削除しない。
- [ ] **P3** B-8（保留しない）: `.githooks/pre-push` の `target` に `state/(platform_state|mode_key_pass|key_effect_)` を足し、`fix-requires-evidence.md` の IME belief 行に同じファイルを**同じコミットで**足す。`fix-requires-evidence.md:95` の「実行されるのは `.git/hooks/pre-push` 側」を訂正する。未使用の `.git/hooks/pre-push`（未追跡、リポジトリ外）は削除してよいが、メインの作業ツリーを使っているユーザーに確認してから行う。

## 受け入れ条件

すべて Linux で実行できる（P2 のコンパイル確認を除き Windows ターゲット不要）。

- **P1**
  - `for c in c949ba33 651cab8d feb49ffd d777bcfe f83084b3 621bf93c 502c6673; do git merge-base --is-ancestor $c HEAD || echo NG $c; done` が何も出力しない。
  - `grep -n "未マージ\|撤去ブランチ" docs/adr/{187,189,190,191-ime-is-source-of-truth-observe-not-write}*.md` の frontmatter の範囲（summary と status の両方）に、PR #238 取り下げの経緯を述べた2文（ADR-191 summary (3)・status の「単独先行（PR #238）は取り下げ」）以外のヒットが無い。`grep -n "ADR-196自体は未マージ\|develop未マージ、別セッション担当" docs/adr/195-*.md docs/adr/index.md` が0件。
  - `grep -rn "ADR-178" docs .claude crates src CLAUDE.md ARCHITECTURE.md | grep -v "docs/adr/178-\|docs/tasks/review-2026-09-24-\|opus-review\|ADR-178 \?決定[0-9]\|ADR-178 v14\|ADR-178（MSI\|旧178"` の残りヒットを手で判定し、撤去プロジェクト（現 ADR-179）を指すものが0件。`5877f982` では22件ヒットし、うち MSI 側で残してよいのは `crates/awase-windows/src/app/mod.rs:232`・`src/paths.rs:48`、番号の付け替えを説明済みで残してよいのは `docs/adr/index.md:186`・`180-*.md:51` の4件（除外を `決定[0-9]` 単独にすると `fix-requires-evidence.md:41` の「ADR-180決定1」で対象行ごと落ちるので、`ADR-178 決定N` の形に限る）。狭い grep（`ADR-178領域\|ADR-178撤去`）だけでは、`related_adr` や「ADR-178がforce-ONを撤去した」を拾えない。
  - CI `adr-index-consistency` と同じ grep（`.github/workflows/ci.yml:215-240`）をローカルで実行して、欠落0件。
- **P2**
  - `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib` が通る。
  - `cargo nextest run -p awase-windows --test architecture_guard --test layer_boundary_guard` が通る。このガードは `try_force_on_bootstrap` 等をスキャンしない。ただし `.apply_ime_open_with_view(` のような件数ガードがコメント中の出現も数える可能性があるので、コメントを書き換えたら実行する。
  - 上の grep の全ヒットが過去形、または撤去コミットへの参照付きになっている（レビューで確認）。
  - windows-build CI が通る（`#[cfg(windows)]` のテストはここでしか走らない）。
- **P3**
  - `cargo run -p xtask-adr-evidence -- .` が exit 0。
  - hook の直接実行: `printf 'refs/heads/x <key_effect を変えたコミット> refs/heads/x <その親>\n' | bash .githooks/pre-push origin <url>` で `[pre-push][warn]` が出る。hook は警告のあとに awase-windows の xwin キャッシュを消して再チェックする（重い）ので、警告が出たら中断してよい。worktree からの実際の push では、メインの作業ツリーの版が動くので確認にならない。

## 他ファイルとの依存

- 本タスク → [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md)・[06](review-2026-09-24-06-keymap-learn-staleness-wiring.md)・[07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md): ADR-196 の採用のずれ、ADR-195 段階8、ADR-176 の存廃は、それぞれの判断を待つ。「未マージ」などの事実訂正は待たない。
- [09](review-2026-09-24-09-remaining-active-writes-inventory.md) ↔ 本タスク（双方向、どちらも相手を待たない）: 10 の P1 で ADR-179 に撤去の事実と根拠を書き、09 はそれを参照する。09 の A/B 結果は、09 が終わってから同じ節に追記する。09 → 10 の B-6: `set_ime_mode` の死蔵コード（受け取った）。
- [08](review-2026-09-24-08-open-close-fixed-set-vs-custom-keymap.md): ADR-191 frontmatter の編集が重なる。10 は summary 11・15行と status の「未マージ」系の事実訂正だけ、08 は summary (2) と本文の食い違いを直す。同じ frontmatter を触るので、先にマージした側に後の側が rebase する。
- 本タスク → [05](review-2026-09-24-05-startup-desired-open-forced-on.md): B-8 で `platform_state.rs` を対象に加える（05:84-86 への回答）。05:85・05:115 の「実行される `.git/hooks/pre-push:28`」は誤り（実行されるのは `.githooks/pre-push`）なので、05 側で訂正してもらう。
- 本タスク → [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md): B-8 で `key_effect_*` が対象に入る（01:171）。
- 本タスク → [11](review-2026-09-24-11-low-priority-backlog.md): 11:32 は B-8 を「保留」としているが、本タスクは B-8 を保留にしない（上記「作業単位」）。11 の表の B-8 の記述を直してもらう。

## 未確認点

- `docs/ime-control-overview.md:276` の `drift_monitor`（観測失敗の追跡）自体が現存するか。
- `MechanismCommand::SetOpenCrossProcessSync` に本番で到達する経路が残っているか（B-6）。
- `docs/adr/176-implementation-tasks.md:844-846` 以外に、ADR-176 の撤去を記録した場所があるか。

## レビュー反映メモ（2026-09-24、Opus レビュー指摘 1〜16）

裏取りの基準は `5877f982`。レビューは `cbae84ff` 基準だったが、両者の差分（PR #293〜#296）は keymap-learn と tuning の変更だけで、本タスクの対象ファイルは変わっていない。

- **反映（事実を確認済み）**
  - 1: `core.hooksPath` が `/home/cuzic/rust-nicola/.githooks`、実行されるのは `.githooks`。
  - 2: `xtask-adr-evidence` と CI `adr-evidence-consistency` の依存。
  - 3: ADR-172/173 はアーカイブタグで origin に保存済み。BUG-142:51 のリンク切れ。
  - 4: ADR-179 に追記する方針。`090:682` の誤リンク。
  - 5: ADR-195 の status 44行と index:202。
  - 6: `502c6673` は HEAD に含まれる。ADR-179 本文の「191」は0件。本文44行も対象。
  - 8: 未確認だった箇所は全て現存。grep で全ヒットを判定する方式に変更。`ARCHITECTURE.md:43` も対象。
  - 9: `platform_state.rs:1341` のログは本番コード。
  - 10: `BrokenAppBootstrap` の本番での使用は remove だけ。
  - 11: keymap-learn のモジュール一覧（`walk_trace` も追加されていた）。
  - 12: 受け入れ条件を具体的なコマンドに変更。
  - 13: 事実訂正と判断待ちの2段階に分割。
  - 14: architecture_guard の意味づけを訂正。
  - 15: P1/P2/P3 に分割。
  - 16: `related_adr` から ADR-178 を外し、ADR-090/173/185 を追加。ADR-158 も追加（B-8 の xtask は ADR-158 TC3）。
- **一部修正して反映**
  - 7: 範囲は「#259〜#291」でも正確でない（#261 は ADR-197、#267・#270 は別件）。ブランチ名に `adr196` を含むマージコミットの列挙に置き換えた。
  - 3: レビューの修正案「index に172・173の1行を追加」は、そのままだと CI `adr-index-consistency` が落ちる（リンク形式の行は本文ファイルの存在を検証する）。リンク無しの形式にした。レビューに無い点。
  - B-8 の hook 直接実行: レビュー案どおり合成の stdin で動くが、警告のあとに重い xwin チェックが続くことを確認したので、受け入れ条件に注記した。
- **反映しなかった点**
  - 1 の「memory `project_pre_push_hook_path_divergence_2026_09_06` を訂正する」はリポジトリ外のファイルなので、本タスクには含めない（呼び出し側に委ねる）。
  - 1 の「`.git/hooks/pre-push` を削除して構わない」は、未追跡のためリポジトリの PR では扱えない。ユーザー確認を条件にした。

## レビュー反映メモ（2026-09-24、Opus 再確認レビューの追加指摘 A〜F）

裏取りの基準は `5877f982`。

- **反映（事実を確認済み）**
  - A: `grep -rn "ADR-178"` で、狭い grep が拾わない `related_adr` 3件（BUG-135/136・ADR-185）と本文（BUG-146:31、182:558、184 本体:154）を確認して A-5 に加えた。`key_pipeline.rs:1521` は、`git log -S` の導入コミット `e475b600`（「docs(adr-178): …ModeKeyActuationOwner案に収束」）から現 ADR-179 のレビュー round を指すと確定し、P2 の対象に加えた。opus レビュー記録は書き換えない方針にした。受け入れ条件の grep を除外付きの広い形に変えた。
  - B: B-8 の保留を外した（01:171・05:84 が依存し、変更は数行）。11:32 の訂正を 11 に依頼する旨を依存節に書いた。
  - C: `platform_state.rs`（`ImeStateHub`・`check_drift_correction`）と `mode_key_pass.rs`（通過マークと `desired_open` の揃え）を B-8 の対象に加えると判断した。05:85/115 の `.git/hooks` の記述は 05 に訂正を依頼する。
  - D: `set_ime_mode`（`ime.rs:1733`）の呼び出し元0件を確認し、B-6 に加えた。
  - E: 09 の frontmatter に ADR-178 が無いことを確認し、09 への依頼を削除した。ADR-179 への追記を2段階（事実と根拠→09 の A/B 結果）に分けた。
  - F: ADR-191 の summary 11・15行を表に加え、08 を依存節に加えた。08:87 が「summary (2) の訂正は 08 で行う」としているので、担当を分けて書いた。
  - 補足: ADR-176 本体の status は 07 が直す（07:120）と明記した。07 は index.md の 176 行に触れないので、index の追随は 10 に残した。
- **一部修正して反映**
  - A の受け入れ grep 案（`grep -v "…\|決定[0-9]\|v14"`）は、`決定[0-9]` の除外で `fix-requires-evidence.md:41`（「ADR-180決定1」を含む）が対象行ごと落ちることを実行して確かめた。`ADR-178 \?決定[0-9]` に絞った。
