---
id: ADR-168
title: |-
  Clojure風protocol/transducerでの複雑性削減案(型システム全面置換・決定軸registry統一)の却下記録、
  およびADR-088の`post_*_direct`4関数保持決定の反転
summary: |-
  Clojureのprotocol/transducer/macroという発想をこのコードベースに応用できないかというブレストから、
  (A)actuation合流点dylint許可リストの型システム全面置換、(B)IME actuation決定17軸のtransducer的
  registry統一、の2案を検討し、2ラウンドのopus-adversarial-consultにかけたところ両方とも不成立
  （前提の事実誤認・既存ADRとの衝突・1-in-1-outの引き算原資なし）と判定された。レビュー中に見つかった
  実際に価値のある小さな作業3件は、この却下記録とは別に本ADR外（ADR-159追記・ADR-089 §9-15更新・
  コミット本文のみ）に振り分けた。本ADR自体の本体は却下記録と、その過程で発見した
  ADR-088:901「`post_ime_on_direct`等4関数を削除するな」という決定の反転（後継の回帰テストが
  ADR-158 D3で既に揃ったため）。
status: |-
  実装済み。opus-adversarial-consult 2ラウンドで収束（1ラウンド目で案A/B自体を却下、2ラウンド目で
  本ADR初稿の実装方針の誤り6件を検出・修正）。
related_adr:
  - "ADR-088"
  - "ADR-089"
  - "ADR-090"
  - "ADR-158"
  - "ADR-159"
  - "ADR-161"
  - "ADR-163"
---

# ADR-168: 案A/案B却下記録 + ADR-088反転

## ステータス

**実装済み。** opus-adversarial-consultを2ラウンド実施した。1ラウンド目は案A・案B
（下記）自体の是非を検証し両方とも不成立と判定、2ラウンド目はその結果を受けて書いた
本ADR初稿の実装方針を検証し、6件のBlocker（実施4の反転理由の誤り・付け替え先の
検出力ゼロ・実施3の可視性計画の実装不可能性・実施2の分割設計の不成立など）を検出、
反映した。

## 背景・経緯

ユーザーとのブレストで「Clojureのprotocol/transducer/macroという発想を、
`docs/adr/158-complexity-inventory-2026-09-10.md`が挙げるこのコードベースの複雑な箇所に
応用できないか」という相談があり、次の2案を検討した。

- **案A**: `lints/actuation_call_guard`（rustc-private・nightly依存のdylintクレート、
  actuation合流点の呼び出し元を許可リストで検査）を、既存の`state/actuation_chain.rs`の
  `Actuation<Requested/Warranted/Verified>` type-stateパターン（`run_chain`は`Verified`
  にしか生えない、`compile_fail` doctestで固定済み）を下層のFFI境界まで伸ばすことで
  型システムに置き換えられないか、という案。
- **案B**: IME actuationの「何を送るか」を決めるロジックが17軸・非対称な複数経路に
  分散している問題を、Clojureのtransducer（reduceするstep関数をsource/sinkから
  独立させる発想）を借りて、**統合はせず**共通シグネチャのregistryとして可視化する案。

両案をOpus（`opus-adversarial-consult`、読み取り専用の批判的レビュー）にかけた結果、
**どちらも当初の目的では不成立**と判定された。

## Opusレビューの結論（不成立の理由）

### 案A: 「dylint丸ごと削除」は不成立

`apply_ime_open_with_view`/`apply_ime_open_with_belief`は**既に**
`ActuationOrder`という値で締められている（`platform.rs:1665-1673`。`ActuationOrder`の
唯一の構築口`issue()`は必ず`issue_open_warrant()`を通る、`actuation_chain.rs:341-352`、
INV-47）。dylintの`RESTRICTED_CALLS`がこの2エントリに対して守っているのは「証跡の有無」
ではなく「呼び出し元が宣言した件数より増えていないか」であり、これは型では表現できない
性質である（`ActuationOrder::issue`は`pub`で、crate内のどこからでも起案できる）。

`send_input_safe`の19呼び出し元を実際に仕分けると、IME open actuationに属するのは3件
（`post_kanji_toggle_to_focused`/`send_ime_mode_key`/
`send_ime_mode_key_with_shift_release_prefix`）だけで、残り16件は「SendInputの唯一の
ラッパーを経由させる」というAPI表面の一本化が目的であり、capability tokenの対象では
ない。しかもその3件のうち`send_ime_mode_key`には`platform.rs:1274`の
`PlatformRuntime::send_engine_state_ime_key`という、`ActuationOrder`もwarrantも
持たない独立した経路（エンジンON/OFF時のユーザー設定IMEモードキー送信、`applied`/
profile/`uses_kanji_toggle()`で独自に3段のスキップ判定をしている）からも呼ばれて
おり、ここを型で締めようとすると偽のwarrantをでっち上げることになる
（`.claude/rules/ime-belief-architecture.md`が名指しで禁じている意味論的偽装と同型）。
`send_engine_state_ime_key`は`.claude/rules/fix-requires-evidence.md`の「IME
actuation合流点」表（6エントリ）とは別ファミリー（「force-write / actuation
ターゲット（ADR-084/086）」側）であり、合流点表への追加7件目という意味ではない。

型化が本当に成立する候補は`ime_controller.rs::apply_mechanism`だった
（ADR-089 §9-15として既に記録済みの残課題。ADR-168での再評価結果は
[ADR-089](089-ime-typestate-and-capability-const-table.md)側に追記した——
可視性計画が実装不可能・実際に削減できる対価が小さい・進行中のADR-163 TH1eと
同じトレイトを触るため順序衝突しうる、の3点から**今回は実装せず据え置き**）。

### 案B: 不採用

前提のうち3点が事実誤認と判明した。

1. 「合流点は5経路」ではなく実際は6（`.claude/rules/fix-requires-evidence.md`の
   「IME actuation合流点」表が明記）。ADR-163がスコープ外にしたのは
   `reassert_explicit_physical_key`と`force_on_and_correct_romaji`の2つ。
2. `shadow_on`の「4供給元」は、実際には`ControlLog.shadow_on`（唯一の構築点、
   `platform.rs:1641`）・`OpenBeliefInputs.shadow_on`（診断専用の別フィールド、
   `bool`、`executor.rs:1000-1004`が「BUG-113修正の対象外」と明記）・probe引数の
   `shadow_on`という**意味論の異なる3種類の別物**を、名前が同じという理由だけで
   1つに見ていた。案B自身が最大のリスクとして自戒していた「実在しない対称性を
   仮定する」誤りに、案B自身が陥っていた。
3. 「`None`が未知か意図的bypassか区別できない」問題は、2026-09-11のADR-163 Part D
   （`AttemptRecord.shadow_on_before_bug113_override`、`DecisionSite`列挙）で
   既に解決済みだった。

加えて、`decide_*`関数群は引数も戻り値も揃っておらず「同じ形の値を畳み込むstep関数」
というtransducer成立の前提自体が無い。1-in-1-outの引き算の原資として提案していた
「17軸の手書き表の自動生成」「合流点表の自動生成」は、どちらも既存ADR
（ADR-161 round4 TJ1 M1、`xtask-adr-evidence`のヘッダコメント）が「散文の注記は
生成せず引き続き人手で維持する」と既に決定済みの範囲だった。

`.claude/rules/complexity-budget.md`は「gate数（呼び出し前判定ロジック一般）は
対象に含めない」と明記しており、案Bはこの対象外領域に自分から新しい宣言機構
（trait/registry）を持ち込んで予算対象を拡張するという、ADR-158 RC4
（ガバナンスは足されるだけで引き算がない）の典型例になっていた。

案Bから唯一救う価値があったのは、`decide_needs_romaji_pre_write`
（`state/ime_actuation_decision.rs:189`）と`decide_dispatch_conv_after_open`
（`:208`）という、意図的に別の条件式である2つのROMAN判定の関係を、全数の含意
（`pre_write ⟹ conv_after_open == Write`が全入力で成り立つ）として固定するテストの
追加だった。新しい抽象は導入しないため、ADRではなく通常のコミットとして
`state/ime_actuation_decision.rs`の既存テストに追加した
（`needs_romaji_pre_write_condition_matches_the_pre_phase_c_strategies`と同じ
4重ループに相乗り）。

## 決定: ADR-088の`post_*_direct`4関数保持決定を反転する

### ADR-088の元の決定と、それが今は成り立たない理由

`docs/adr/088-ime-axis-capability-and-charset-owner.md:901`は、`ime.rs`の
`post_ime_on_direct`/`post_ime_off_direct`/`post_gji_ime_on`/`post_gji_ime_off`
（`ime_controller.rs`が`send_ime_mode_key`を直接呼ぶようになった後、本番呼び出し元が
ゼロになっていた）について、**「それでも削除してはならない」**と決定していた。
理由: `tests/architecture_guard.rs`の`ime_open_close_functions_send_expected_vk_codes()`
がこれら4関数の本体テキストを検査対象にしており、削除すると
`docs/experiments.md`エントリ01（IME OFFキー選択が5日間に6回反転した記録）に対する
**唯一の**回帰検知が消えるため。この判断はADR-088執筆時点では正しかった。

### 何が変わったか

ADR-158 D3で`state/key_sequence_policy.rs::{gji_direct_keys, ms_ime_direct_keys}`
という後継テストが追加された。これは真のSSOTである`ime_key_for`（`:135`）の
GjiDirect/MsImeDirectの4アーム（Open/Close × 2機構）すべてをピン留めしており、
Close側のassertメッセージには「ADR-158 D3: IME OFFキーにVK_DBE_ALPHANUMERIC等を
再導入していないか確認せよ。docs/experiments.mdエントリ01（5日間に6回反転した記録）
を読むこと」という、まさにエントリ01を指す警告が既に書かれている。`ime_key_for`の
docコメント自身も「検出は`gji_direct_keys`/`ms_ime_direct_keys`テストが既に4アーム
すべてをピン留めしており担っている」と明言している。

つまり、ADR-088が「削除するな」の根拠にした**唯一の回帰検知**は、ADR-088執筆後に
`key_sequence_policy.rs`側へ完全に引き継がれていた。反転の根拠はこの事実であり、
「見落とし」ではない。

### やったこと

1. `ime.rs`から`post_ime_on_direct`/`post_ime_off_direct`/`post_gji_ime_on`/
   `post_gji_ime_off`の4関数を削除した。
2. `tests/architecture_guard.rs`の`ime_open_close_functions_send_expected_vk_codes()`
   のうち、この4関数の本体テキストを検査していた部分を削除し、生きている
   `post_kanji_toggle_to_focused`（VK_KANJIトグルのdown/up検査）だけを残した
   テストを`kanji_toggle_fallback_sends_expected_vk_codes()`に改名した
   （**「付け替え」ではなく削除**——当初案は検査を`ime_controller.rs`の
   `write`実装へ付け替えるつもりだったが、その関数にはVK_IME_ON/OFFという
   識別子が一度も出現せず、VKは`state/key_sequence_policy.rs::ime_key_for`が
   決めるため、付け替えても検出力ゼロの恒真ガードになるとレビューで判明した）。
3. `state/key_sequence_policy.rs`のコメント、`tests/ime_key_sequence_golden.rs`の
   `KEY_DOC`（および対応する`tests/golden/ime_key_sequences.txt`）、
   `docs/ime-control-overview.md`の該当箇所を、実際の呼び出し経路
   （`ime_controller.rs`の`MechanismCommand::SendVk(vk)`分岐が
   `send_ime_mode_key(vk)`を直接呼ぶ）を指すよう更新した。
4. ADR-088自身に、この決定が反転されたことを示す追記を残した（frontmatterの
   `status`と該当する表の行の両方）。

### 回帰検知の引き継ぎ確認

`fix-requires-evidence.md`の「キー選択」ファミリーが要求する(a)(b)のうち、
(a)回帰テストとして`state/key_sequence_policy.rs::{gji_direct_keys, ms_ime_direct_keys}`
（既存、新規作成不要）を後継として明記する。エントリ01の回帰検知が途切れていない
ことは、この2テストと`ime_key_for`のdocコメントの相互参照で確認できる。

## やらないこと

- `lints/actuation_call_guard`クレート自体の削除。
- `apply_ime_open_with_view`/`apply_ime_open_with_belief`への追加変更
  （既に`ActuationOrder`で型的に締まっている）。
- `send_input_safe`の19呼び出し元・`send_ime_mode_key`を含む3件への型capability導入
  （`send_engine_state_ime_key`という独立経路があり、型で締めると偽warrantの
  偽装になる）。
- IME actuation決定17軸を共通traitやregistryへまとめる案B全体。
- `apply_mechanism`へのwrite token導入（ADR-089 §9-15、今回は据え置き。
  理由は同ADRへの追記を参照）。

## 他の項目の記録先（本ADRに含めない）

- 2つのROMAN判定の全数差分テスト: 新しい抽象を導入しないためADRを起票せず、
  `state/ime_actuation_decision.rs`へのコミットのみ。
- `send_ime_control`のactuate/probe分割（`imm.rs`内でraw関数をmodule-private化し
  `probe_ime_control`/`actuate_ime_control`という型付きラッパーに割る）:
  ADR-159 round4 TJ2 MF2が受容していた負債の返済として、
  [ADR-159](159-existing-io-boundary-inventory.md)へ追記した。
- `apply_mechanism`へのwrite token導入: [ADR-089](089-ime-typestate-and-capability-const-table.md)
  §9-15へ、対価とリスクを再評価した結果として追記した（据え置き）。

## 複雑性予算（`.claude/rules/complexity-budget.md`、未発効だが方針として意識）

- ADR-088反転（4関数+空洞化した検査4ブロックの削除）: 純粋な引き算。
- `send_ime_control`分割: `RESTRICTED_CALLS`の宣言スロットは1→2に増えるが、
  probe専用6件（`modify_conv_mode`は両エントリに現れる）がactuation用の許可リストを
  希釈していた状態を解消するため実質は精度向上。`cmd: usize`という無型ペアを
  `ProbeCmd`/`ActuateCmd`という列挙型に置き換えたことで、dylintの許可リストが
  実際のactuate/probe区別と一致するようになった。
- 2つのROMAN判定の全数差分テスト: 新規追加はテストのみ、宣言・定数の増減なし。
- `apply_mechanism`のwrite token: 実装せず据え置き（対象外）。

## TH1発効条件との関係

`.claude/rules/complexity-budget.md`のTH1発効条件（実際の削除・統合1件を決定レコード
再生で差分ゼロと検証できたこと）の「最初の1件」は、ADR-163が既に
`AsyncChainWriter::is_applicable`の統合に確定させている
（`docs/adr/163-actuation-decision-io-separation-and-replay-harness.md`）。
本ADRで実施した作業はいずれも実行時の決定ロジックを変えない構造的クリーンアップ、
または純粋な引き算であり、ADR-163 round2 R5が「差分ゼロは定義上自明に成立してしまい、
ハーネスが実際に安全性を判定した証拠にならない（空証明）」として却下したパターンに
該当する。**本ADRの実施はTH1発効条件の代替にはならない**——引き続きADR-163の
既定路線（`AsyncChainWriter::is_applicable`統合）がTH1の本命である。

## 検証計画

- `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows`
- `cargo test --lib`（ホストターゲット、ROMAN判定差分テスト）
- `cargo nextest run -p awase-windows --test architecture_guard --test golden_scenarios --test layer_boundary_guard`
- `cargo clippy --target x86_64-pc-windows-msvc -p awase -- -A clippy::cargo`
- `DYLINT_RUSTFLAGS="-D warnings" cargo dylint --all -p awase-windows -- --target x86_64-pc-windows-msvc`
  （`RESTRICTED_CALLS`宣言変更後、新関数名の一意性確認込み）
- `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests`
  （`ime_key_sequence_golden.rs`は`#![cfg(windows)]`のためLinuxではコンパイル確認のみ、
  `KEY_DOC`とgoldenファイルの一致は手で揃えた上で`windows-build` CIで最終確認）
