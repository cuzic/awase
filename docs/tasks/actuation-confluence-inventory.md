# IME actuation 合流点4件の棚卸し（統合候補/構造上必要/ロジック共有候補の分類）

状態: **完了（2026-09-23）**
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること
（`docs/actuation-confluence-inventory` ブランチ、
`rust-nicola-worktrees/actuation-confluence-inventory` で実施）。

## 結果サマリー（2026-09-23）

現存5エントリ（`ImeController::apply`/`run_open_chain_async`/`fallback_write`/
`imm_cross_write`/`dispatch_ime_set_open`）は**全て構造上必要なエントリ点**と
判明した。**本物の統合候補（エントリ点そのものを削れるもの）はゼロ**——
complexity-budget.md の TH1e 証明材料はこの棚卸しでは得られなかった。

想定していた「エントリ点は残すが中の判断ロジックを共通pure関数へ切り出す」
（ロジック共有候補）作業も、着手前に**既に完了済み**と判明した:

- InputRelayゲート判定（旧: 4箇所が個別に同じ`matches!`を書いていた）は
  `state/ime_actuation_decision.rs::decide_gate`/`is_input_relay`へ
  集約済み（[ADR-180](../adr/180-actuation-gate-recheck-deduplication.md)
  決定1、develop コミット`c8bc1adc`で実装済み）。
- 機構dispatchロジック（`SetOpenCrossProcessSync`/`SendVk`の実Win32呼び出し）
  は`ime_controller.rs::apply_mechanism`へ集約済み（ADR-163 TH1b-2b）——
  sync経路（`ImeController::apply`の`SyncChainWriter`）とasync経路
  （`open_chain.rs::fallback_write`）の2箇所から共有されている。

さらに、より野心的な統合（`ActuationDecisionRecord`3実装の統一、`with_app`を
内包する共有ゲートヘルパー化）は、**このタスクの着手前にADR-180で既に
3ラウンドのopus-adversarial-consultを経て検討・見送り済み**だった
（decision2: 本番−40行に対しテスト+ガード+80〜125行が必要で費用対効果が負、
`caller`の構築時引数化が実装不能。decision1のヘルパー化: `fallback_write`
からの再入でInputRelayゲートが恒久的に無効化される危険、issue #136/BUG-90型
の回帰）。この見送り理由がコード側（`open_chain.rs`の該当`with_app`
クロージャ直前）に無かったため、ADR-180が約束していた「付随作業」として
2箇所に説明コメントを追加した（本タスクで唯一の直接コード変更）。

各エントリの分類の詳細は
[fix-requires-evidence.md「IME actuation合流点」行](../../.claude/rules/fix-requires-evidence.md)
に反映した。

### このタスクの当初想定との差分

起票時点（上記「背景・動機」節）では「この棚卸し自体がTH1eの証明材料に
なりうる」と見込んでいたが、実際には「証明すべき統合候補がそもそも存在しない
（既に先行ADRで分類・実施・却下済み）」という結果になった。これは
complexity-budget.mdの発効を遅らせる新たな障害ではない——単に、今回の
探索範囲（IME actuation合流点5箇所）では統合可能な重複が既に払底していた
ということ。TH1eの証明材料は引き続き別の領域（例: 姉妹タスク
[actuation-confluence-already-matched-gap.md](actuation-confluence-already-matched-gap.md)
のような個別の未テスト分岐ではなく、まだADR-180/163の対象になっていない
箇所）から探す必要がある。

## 背景・動機

「新しい欠陥が見つかるたびに、既存の判断点を直すのではなく別の合流点（gate/
precondition）を新設して症状を隠す」という積み重ねが問題になっている
（[ADR-158](../adr/158-complexity-reduction-north-star.md) RC4: ガバナンスが
「足す」ことしか義務化しておらず「減らす」ことを評価していない）。
[complexity-budget.md](../../.claude/rules/complexity-budget.md)は対策として
1-in-1-out制を起草済みだが、発効条件（[ADR-159](../adr/159-existing-io-boundary-inventory.md)
のTH1e: 凍結コーパスで実際の削除・統合+差分ゼロ再生を1件証明すること）が
未達のためまだ強制されていない。

**この棚卸し自体がTH1eの証明材料になりうる**——本物の冗長合流点を1件見つけて
[ADR-163](../adr/163-actuation-decision-io-separation-and-replay-harness.md)の
リプレイ基盤で安全に統合できれば、それがそのまま complexity-budget.md 発効の
最後のピースになる。

## これまでに分かっていること（2026-09-22時点）

[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)の
「IME actuation合流点」表は元々6エントリを挙げていたが、`git log -S`で調査した
ところ、うち2つ（`runtime/mod.rs::reassert_explicit_physical_key`〈ADR-121 D1〉、
`runtime/mod.rs::force_on_and_correct_romaji`〈ADR-158 TB2〉）は
ADR-179（旧178）領域A撤去（コミット`f83084b3`/`621bf93c`、2026-09-18）で**既に削除済み**
だった。表は追随できておらず、2026-09-22（コミット`d8076516`）で修正済み。

現存する4エントリ:

1. `ime_controller.rs::ImeController::apply`（同期経路唯一の合流点）
2. `runtime/open_chain.rs::run_open_chain_async`
3. `runtime/open_chain.rs::fallback_write`
4. `runtime/open_chain.rs::imm_cross_write`
5. `runtime/executor.rs::DecisionExecutor::dispatch_ime_set_open`
   （早期exit最適化、1〜4と重複するが単独では不十分、と表に明記されている）

（表記上「4関数」と呼んでいるが、`dispatch_ime_set_open`を含めると実質5関数。
`lints/actuation_call_guard/src/lib.rs::RESTRICTED_CALLS`の`apply_ime_open_with_view`
エントリは現在「`dispatch_ime_set_open`と`apply_ime_open_with_belief`の2箇所のみ」と
コメントされている——つまり`ImeController::apply`や`open_chain.rs`の3関数は
`apply_ime_open_with_view`を直接は呼んでおらず、別の合流点〈`set_ime_open`/
`send_input_safe`/`send_ime_control`/`apply_ime_open_with_belief`のどれか〉を
経由している可能性が高い。最初のステップでここを確定させること。）

参考: 姉妹タスク
[docs/tasks/actuation-confluence-already-matched-gap.md](actuation-confluence-already-matched-gap.md)
で、`imm_cross_write`のAlreadyMatched判定（356行目）に未テストの分岐があることが
cargo-mutantsで判明している。今回の棚卸しで「この判定は本当にここでしか行えないのか」
も併せて検討する材料になる。

## やること

各エントリについて、以下を調べて分類する。

### 1. `git log -S<関数名> -- <ファイル>` で追加経緯を確認する

- いつ・どのコミット/ADRで追加されたか
- 追加時のコミットメッセージが「新規に独立ロジックを作った」と言っているか、
  「既存の未宣言呼び出し元を棚卸しで登録しただけ」と言っているか
  （[complexity-budget.md](../../.claude/rules/complexity-budget.md)の
  「棚卸しによる宣言追加と新規複雑性の追加の区別」節の判定方法を流用できる）
- `docs/experiments.md`に、この合流点を統合しようとして失敗・撤回した記録が
  ないか確認する（[experiment-logging.md](../../.claude/rules/experiment-logging.md)）

### 2. `lints/actuation_call_guard/src/lib.rs::RESTRICTED_CALLS` を突き合わせる

各合流点が実際にどのチョークポイント（`set_ime_open`/`send_input_safe`/
`send_ime_control`/`apply_ime_open_with_view`/`apply_ime_open_with_belief`）を
呼んでいるかを確認する。同じチョークポイントを呼んでいる合流点同士は統合候補の
第一候補になる。

### 3. 各合流点を「エントリ点」と「中の判断ロジック」に分けて考える

- **エントリ点自体が冗長**（同じ判断ロジックを独立発見で別の場所に再実装した）
  → 統合候補
- **エントリ点は構造的に必要**（sync executor と async open_chain のように
  呼ばれる文脈自体が違う）→ エントリは残すが、中の判断ロジックだけ共通の
  pure関数へ切り出せないか検討する（ロジック共有候補、リプレイ証明不要で
  すぐ着手できる低リスクな改善）

`open_chain.rs`の3関数は表に「3関数**すべて**が独立に再検出する設計、1箇所だけ
では足りない」と明記されているが、**この記述を鵜呑みにせず現在のコードで
再確認すること**（今回2エントリが既に撤去済みだったのに表が追随していなかった
のと同じ轍を踏まないため）。

## 出したもの・出し方

- 分類結果を[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)
  の「IME actuation合流点」行に反映する（各エントリに分類ラベルを付記する形が
  想定される）。
- **この段階では何も削除しない。** 統合候補が見つかっても、過去に「1箇所だけ
  gateして二重actuationを自己回帰させた」（issue #136）「冗長に見えた経路が
  実は別の呼び出しコンテキスト用だった」という実例がある
  （[fix-requires-evidence.mdの「なぜこのルールが必要か」節](../../.claude/rules/fix-requires-evidence.md#なぜこのルールが必要か背景)参照）。
  本物の統合候補が見つかったら、[ADR-163](../adr/163-actuation-decision-io-separation-and-replay-harness.md)
  のリプレイ基盤（TH1a〜TH1dは完了済み、凍結コーパスあり）で削除+差分ゼロ再生を
  証明してから実施する、別タスクとして切り出す。
- ロジック共有候補（エントリ点は残すが中身を共通化）は、リプレイ証明が要らない
  低リスク改善なので、このタスク内で直接着手してよい。

## 関連

- [docs/tasks/actuation-confluence-already-matched-gap.md](actuation-confluence-already-matched-gap.md) —
  同じ調査から派生したもう一方のタスク
- [complexity-budget.md](../../.claude/rules/complexity-budget.md)、
  [fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)
- ワークフロー/設定: `.github/workflows/mutants-actuation-confluence-windows.yml`、
  `.cargo/mutants-actuation-confluence-scope.toml`
