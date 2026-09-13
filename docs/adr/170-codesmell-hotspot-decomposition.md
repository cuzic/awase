---
id: ADR-170
title: |-
  コードスメル解消: belief reduce()分割(決定1のみ実施、決定2・3は調査のみで見送り)
status: |-
  opus-adversarial-consult round1で決定2・3を見送りに縮小、決定1を実施しround2確認待ち
related_adr:
  - "ADR-090"
  - "ADR-098"
  - "ADR-108"
  - "ADR-121"
  - "ADR-158"
  - "ADR-159"
  - "ADR-161"
  - "ADR-164"
---

# ADR-170: コードスメル解消: belief reduce()分割(決定1のみ実施、決定2・3は調査のみで見送り)

## 背景

2026-09-13、`crates/awase-windows/src/`全体(observer/state/runtime/output/focus/tsf/app
+ ルートファイル、約8万行)を対象にコードスメル(バグではなく保守性上の指摘)を
フォークエージェント5並列で走査した。機械的な修正(タイミング定数のtuning.rs移設・
lint suppression統合・テストモジュール配置・小さな重複解消4件)は別PR
([#214](https://github.com/cuzic/awase/pull/214)、developマージ済み)で対応済み。

残った3件の設計判断を伴う指摘(reduce()分割・runtime/mod.rs重複ブロック統合・
output/mod.rs責務分割)を起草時点でADR-170としてまとめ、opus-adversarial-consult
round1を実施したところ、**決定2・3は事実誤認や見落としが多く実装するとテストを
壊す/検知能力を落とすことが判明**したため見送り、**決定1のみスコープを絞って実施**
した(round1の指摘全文: 実施セッションのログ参照。以下は反映後の内容)。

**重要な事前確認(レビュー候補からの除外)**: レビュー候補に挙がった
`tsf/gji_fsm.rs::on_event`と`journal.rs::emit_tracing`の巨大match分割は、
着手前にコードを読んだ結果**見送った**。両者とも「分割は挙動変更リスクが高い」
「variantごとに分割するとmatchの網羅性チェック(唯一の安全装置)が複数関数に
分散しかえって見通しが悪くなる」という明示的な意図コメントが既に付いており、
フォークの指摘はこのコメントを見落としていた。

## 決定1(実施済み): `state/ime_model.rs::ImeModel::reduce()` の大きい分岐を private ヘルパーへ抽出

### 問題

developブランチ基準(PR #214マージ後)で`reduce()`(555-703行、
`#[expect(clippy::cognitive_complexity)]`付き)は17分岐約150行のmatch文。
[ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md)が
要求する「belief更新はreduce()という単一書き込み口を通す」という制約自体は
妥当だが、各分岐の**中身**まで1つの関数に押し込む必然性はない。

17分岐のうち本体が20行を超えるのは以下の4件のみで、残り13分岐は3〜12行
(大半が1〜3行の単純代入)だった(opus-adversarial-consult round1 F5の実測):

| arm | 本体行数(概算) |
| --- | --- |
| `FocusChanged` | ~54行 |
| `ImeApplyRequested` | ~50行 |
| `ImeApplyFailed` | ~25行 |
| `ImeApplySucceeded` | ~23行 |

**先行事例**: PR #214が既に`UserImeToggleIntent`/`UserImeSetIntent`共通の
`RecordedIntent`構築を`record_intent()`private ヘルパーに切り出しており、
「reduce()からのみ呼ばれるprivateヘルパーを追加する」という手法自体は
develop上で実証済みだった。

### 決定・実施内容

上記4分岐の本体を、同じ`impl ImeModel`内のprivateヘルパーメソッド
(`reduce_focus_changed`/`reduce_ime_apply_requested`/
`reduce_ime_apply_succeeded`/`reduce_ime_apply_failed`)に切り出した。
`reduce()`本体は該当4分岐について「ヘルパーを1回呼ぶ」形に縮小し、
残り13分岐(本体20行以下)はそのまま残した(round1 F5: 過剰な抽出は
「呼び出し先を1段追わないと1行の代入が読めなくなる」だけで可読性が
下がるため)。match**直後**の pending purge ブロック(ADR-108決定4、
期限切れtransitionの後始末)は`reduce()`本体にそのまま残した。

ヘルパーは`impl ImeModel`の同一ファイル内privateメソッドとし、`pub`にしない。
belief系フィールドの可視性(`state/ime_model.rs`外からprivate)は変更していない。

### 「reduce()以外から呼ばれない」ことを保証する仕組み(round1 F2で発覚した見落とし)

round1で、ADRが根拠にしていた前提2つがいずれも誤りだと判明した:

1. `layer_boundary_guard.rs::c6_single_reduce_call_site`は「`model.reduce(`という
   リテラル文字列が本番コードで1回だけ出ること」を見ているだけで、
   「beliefを書くのはreduce()だけ」という性質は検証していない
   (private ヘルパー追加でこのテストは壊れないが、壊れない理由が
   ADR起草時の想定と違っていた)。
2. `.claude/rules/ime-belief-architecture.md`が主張していた
   「`reduce()`以外からの直接代入はコンパイルエラーになる」は、
   Rustのprivateがモジュールスコープである以上厳密には正しくなく、
   ヘルパーが`reduce()`以外から呼ばれないことを強制する言語機構は無い。

このため以下2点を追加で実施した:

- `.claude/rules/ime-belief-architecture.md`の「belief の書き込み点」節の
  記述を、実態(モジュールスコープのprivateであり、ヘルパーの呼び出し元は
  count guardで担保する)に修正した。
- `tests/layer_boundary_guard.rs::c6b_reduce_helpers_called_only_once_from_reduce`
  を新設し、4ヘルパーそれぞれの呼び出し箇所(`self.<helper>(`)が
  `ime_model.rs`内にちょうど1件であることを固定した。新しいヘルパーを
  追加・改名する場合はこのテストの`HELPERS`リストも更新すること。

### `#[expect(clippy::cognitive_complexity)]`の除去

分割後、`reduce()`本体はcognitive complexityの閾値を超えなくなったため
`#[expect(clippy::cognitive_complexity)]`を削除した(round1 F4: 残すと
複雑度警告が発火しなくなった時点で`unfulfilled_lint_expectations`が
`-D warnings`環境で発火しCIが赤くなるため、除去は「見込み」ではなく必須)。
`cargo clippy --target x86_64-pc-windows-msvc -p awase-windows -- -A clippy::cargo_common_metadata -D warnings -W clippy::cognitive_complexity`
(CI: `.github/workflows/ci.yml`と同じフラグ)で警告0件を確認済み。

なお`state/`は`#[cfg(windows)]`配下のため、この検証はWindowsターゲットで
しか行えない(Linuxローカルの素の`cargo clippy`ではそもそも評価されない、
round1 F4)。

### テスト結果

- `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows`
- `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`
- `cargo clippy --target x86_64-pc-windows-msvc -p awase-windows -- -A clippy::cargo_common_metadata -D warnings -W clippy::cognitive_complexity`(警告0件)
- `cargo fmt -- --check`
- `cargo test --lib`(ルート`awase`、1016 passed)
- `cargo nextest run -p awase-windows --test architecture_guard --test golden_scenarios --test layer_boundary_guard`
  (124 passed、新設`c6b_reduce_helpers_called_only_once_from_reduce`含む)

いずれもpass。`state/ime_model.rs`内の`#[cfg(test)] mod tests`(cfg(windows)配下、
Linuxではリンクできずローカル実行不可)はwindows-build CIでの確認に委ねる。

## 決定2(見送り): `runtime/mod.rs`の重複actuationブロック統合

### 検討した内容

`force_on_and_correct_romaji`と`reassert_explicit_physical_key`
(develop基準でそれぞれ`runtime/mod.rs`999-1066行、1184-1228行)が、
view構築→actuation実行→journal記録という約7行をほぼ逐語コピーしている。
これを共通privateヘルパー`apply_actuation_and_record`に切り出す案を
検討した。

### 見送った理由(opus-adversarial-consult round1)

1. **既存テストが確実に壊れる**: `architecture_guard.rs`の
   `force_write_paths_bypass_gji_shadow_on_via_none_applied`は
   `force_on_and_correct_romaji`の関数本体から`build_ime_control_view(None)`の
   呼び出しをちょうど1回として固定しており、このテストのdocコメント自身が
   「`force_on_and_correct_romaji`だけがADR-087 INV-28
   (force-writeが`applied=None`で`GjiDirectStrategy`のno-op skipを
   bypassする、崩れるとBUG-16再発)の**唯一のenforcement拠点**」と明記している。
   共通ヘルパーへ切り出すとこのテストの対象を張り替える必要があり、
   張り替え後は「2経路を1つのassertionで覆う」ことになって、
   第3の呼び出し元が増えても検知できなくなる意味論変化を伴う。
2. **統合すると新しい合流点の増加を検知できなくなる**:
   `architecture_guard.rs::ime_open_actuation_entry_points_are_accounted_for`と
   `lints/actuation_call_guard::RESTRICTED_CALLS`はどちらも
   「`apply_ime_open_with_view`への呼び出し元が想定外に増えていないか」を
   個別に監視している。2つの独立した合流点を1つのヘルパーに統合すると、
   監視対象が`apply_actuation_and_record`1点に縮退し、
   `.claude/rules/fix-requires-evidence.md`の「IME actuation合流点」表が
   守ろうとしている検知粒度そのものが下がる(issue #136/ADR-119が扱った
   事故「新しいgateを1箇所に置いて満足しない」の逆方向)。
3. **CIで走る3つ目の照合器(`xtask-adr-evidence`)への言及漏れ**: 上記2つの
   宣言(許可リストの件数・count guardの期待値)が一致しているかを
   `cargo run -p xtask-adr-evidence -- .`がCIで検証しており、
   どちらか一方だけ更新すると別のCIジョブだけが落ちる罠がある。
4. **費用対効果**: 上記の更新作業(既存テストの対象張り替え・
   lint許可リストの補償エントリ追加・xtask再確認・実行順入れ替えの正当化)
   に対して、削減できるのは重複7行のみであり、この重複が過去に実害
   (known-bugs入りするような事故)を起こした記録も無い。

### 今後この統合を再検討する場合

- 対応表: `architecture_guard.rs::force_write_paths_bypass_gji_shadow_on_via_none_applied`
  の対象関数名の張り替え、`ime_open_actuation_entry_points_are_accounted_for`の
  `.apply_ime_open_with_view(`期待値(4→3)、`lints/actuation_call_guard::RESTRICTED_CALLS`
  の許可リスト更新(2エントリ削除+新ヘルパー追加+補償エントリ
  `("apply_actuation_and_record", &["force_on_and_correct_romaji", "reassert_explicit_physical_key"])`)、
  `.claude/rules/experiment-logging.md`の適用範囲一覧への`runtime/mod.rs`追加
  (現状漏れている、`.githooks/pre-push`の正規表現とはズレている)。
- `force_on_and_correct_romaji`は`issue_actuation_order`をview構築より後で
  呼んでいるが、共通ヘルパーにするなら構築より前に繰り上げる必要がある
  (両者とも`&self`で状態を変えないため等価だが、その根拠をADRに明記すること)。
- `record.caller`を上書きしない(`site`は`Sync`のまま維持する、PR #201由来)
  という制約と、`issue_actuation_order`に渡すstrategy文字列(実機ログに
  そのまま出る)は呼び出し元が渡し続ける設計を維持すること。

## 決定3(見送り): `output/mod.rs`の`Output`構造体責務分割

### 検討した内容(起草時点、事実誤認あり)

`output/mod.rs`の`impl Output`を、GJI reinitスケジューリング・
confirm-gate override・Unicode cold-defer・GJI FSM橋渡し・
TSF gate/warmupオーケストレーションの5クラスタに分割する案を検討した。

### 見送った理由(opus-adversarial-consult round1)

起草時点の実地調査(1ブロック・約60メソッド・約1,340行という記述)が
不正確だった:

- `impl Output`は実際には**2ブロック**(444-1786行・1824-2093行)、
  合計**1,611行・80メソッド**であり、起草時点は2つ目のブロックの存在に
  一切触れていなかった。
- 80メソッドのうち**34メソッド(43%)**は、本体2文以下で
  `self.<field>.<method>(...)`を呼ぶだけの委譲であり、実体は既に
  `KeyInjector`/`TsfWarmupCoordinator`へ抽出済みだった。これらを
  新しいサブ構造体に移すと、`runtime/ → Output(facade) → 新サブ構造体
  (facade) → TsfWarmupCoordinator(実体)`という**削るべき中間層を
  1段増やす**だけになる。
- 起草時点で「最も設計判断の余地が大きい」と名指しした
  `ime_mode_fsm`↔GJI reinitの結合は、実際には`step_probe`が
  `TsfEnvSnapshot`という値型スナップショットに詰めて渡す形で**既に
  疎結合**になっており、論点として成立していなかった。
- 本当の結合は`output/`外(`platform.rs`・`runtime/key_pipeline.rs`・
  `tsf/warmup/cold_warmup.rs`等)と`output/`内の別ファイル
  (`probe_io.rs`・`vk_send.rs`・`conv_actuation.rs`)からの、クラスタ表が
  挙げるフィールドへの`pub(crate)`直接アクセス(25箇所以上)であり、
  これは起草時点で完全に見落としていた。フィールドをサブ構造体へ移すと
  これら全てにパス変更またはアクセサ追加が必要になり、「呼び出し元の
  変更を最小化する」という前提は成立しない。
- パス固定のガードテストが最低1件確実に壊れ(`raw_recovery_owns_deferred_call_sites_are_accounted_for`)、
  少なくとも1件は**failせずに検知能力だけを失う**
  (`deferred_origin_recovery_resend_construction_is_limited_to_gate_bypass`が
  列挙済みファイルパスだけを見るため、新規ファイルに同種の構築が
  生えても気づけない)。

### 今後の代替案(次に検討する場合の出発点)

`mod.rs`に実ロジックとして残っているのは実測で以下のみ:

| 実体 | 概算行数 |
| --- | --- |
| GJI reinit スケジューリング/retry | ~210行 |
| ImeModeFsm 所有 + IMC 反映 | ~100行 |
| confirm-gate override / shift-conv-guard | ~40行 |
| probe step/finish + defer 判定 | ~340行 |
| `send_keys` 本体 | ~230行 |

次に着手する場合は「5クラスタ分割」ではなく、(i) GJI reinitブロックのみを
1サブ構造体に切り出す(対象フィールドは全て`Cell`/`RefCell`なので`&self`
メソッドのまま移せる)、(ii) 1行委譲メソッドは移さない、(iii) むしろ
`pub(crate) fn warmup_coord(&self) -> &TsfWarmupCoordinator`等のアクセサを
出して委譲メソッドを**削除する**方向を別途検討する、という縮小版から
始めること。パス固定ガードの洗い出し(移動先ファイルの追加)を手順に
必ず含めること。

## 手続き上の記録

- 決定1・2・3とも`fix`ではなくrefactorのため、
  `.claude/rules/fix-requires-evidence.md`の(a)回帰テスト/(b)known-bugs追記の
  形式上の義務は発生しない。ただし決定1は`c6b_reduce_helpers_called_only_once_from_reduce`
  という新規回帰テストを実際に追加した(このリファクタ自身のevidence)。
- `.claude/rules/complexity-budget.md`の1-in-1-out規約はADR-162 TH1e未達成のため
  現時点では未発効。決定2を将来実施する場合の`RESTRICTED_CALLS`補償エントリ追加は
  現時点では許容される。
- 本ADRは当初developの先端ではなく古いコミット(main相当)から切ったworktreeで
  起草され、PR #214の内容を含んでいなかった(行番号がstaleだった)。
  round1で指摘を受け、実装前に`git reset --hard origin/develop`でbaseを
  修正した(`.claude/rules/main-develop-branch-flow.md`違反の是正)。
