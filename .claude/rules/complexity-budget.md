# 複雑性予算制（actuation合流点・tuning定数）

**現状: 起草済み・未発効。** 本ルールは[ADR-162](../../docs/adr/162-governance-reversal.md)
E1が定める規約を文書化したものだが、発効条件（下記「発効条件」参照）をまだ満たしていない
ため、現時点ではまだ強制されない。TH1タスク（`docs/adr/158-implementation-tasks.md`）が
この文書を用意した段階で、発効条件が満たされ次第このルールを有効化する。

## ルール

以下の対象について、宣言（許可リスト・定数）へ新しいエントリを追加するコミットは、
同種1件の削除を伴わない限り許可しない（1-in-1-outの複雑性予算制）。

- **actuation合流点数**: `lints/actuation_call_guard/src/lib.rs::RESTRICTED_CALLS`が
  宣言する各チョークポイント（`set_ime_open`・`send_input_safe`・`send_ime_control`・
  `apply_ime_open_with_view`・`apply_ime_open_with_belief`）の許可呼び出し元リスト。
- **tuning定数数**: `crates/awase-windows/src/tuning.rs`の`pub const`定数
  （`#[measured_macro::measured(...)]`が必須、[tuning-constants](./tuning-constants.md)
  参照）。

「gate数」（`runtime/executor.rs`等の呼び出し前判定ロジック一般）は対象に**含めない**
——現時点で対応する宣言機構が存在しないため（[ADR-161](../../docs/adr/161-single-source-spec-generation.md)
の「gate」と「actuation合流点」の語法衝突をADR-162 round4 TJ4 M1で整理した結果）。

### 棚卸しによる宣言追加と新規複雑性の追加の区別

[ADR-159](../../docs/adr/159-existing-io-boundary-inventory.md)段階0は本質的に
「既存の未宣言呼び出し元を許可リストに載せていく」棚卸し作業であり、初期許可リストは
必ず不完全で、後から「実は既にあった呼び出し元」を追加するコミットが継続的に発生する
（`set_ime_open`で実例あり）。これと「新しい呼び出し元を作った」場合は、許可リストの
diffとしては同一の見た目になる。

**区別の判定方法**: 対象コミットが`git log -S<関数名>`で該当関数の初出コミットより後で
あれば「新規複雑性の追加」として1-in-1-outを適用し、初出コミット以前から存在した呼び出しの
棚卸し追加であれば適用しない（コミット本文に「棚卸し中」である旨を明記すること）。
`ADR-159`段階0の各対象の棚卸しが完了するまでは、この猶予を適用する。

### 例外条項

ユーザーが実際に困っている症状の修正を「削除相手が見つからない」という理由で止めては
ならない。**不具合修正コミットで、削除相手が見つからない場合は、超過を明記した短いADR
（既存ADRへの追記でもよい）を起票した上で許可し、次回のE3定例棚卸し（下記「関連」参照）で
削除の返済を検討する。**

## 発効条件

[ADR-162](../../docs/adr/162-governance-reversal.md) round4 TJ4 M5の訂正により、
本ルールの発効条件は「配線確認」ではなく**能力ベース**である: [ADR-159](../../docs/adr/159-existing-io-boundary-inventory.md)
の記録・再生基盤で、**実際の削除・統合を1件、N本の記録トレースの再生で送信列差分ゼロと
検証できたこと**。

2026-09-11時点でこの条件は**未達成**——ADR-159段階2（TF2、`send_input_safe`/
`send_ime_control`の実際の送信内容を記録するシャドー実行、`shadow_send_trace.rs`）は
2026-09-10にPR#193で実装・実機検証済みだが、これは「記録」側の充足に過ぎない。
[ADR-163](../../docs/adr/163-actuation-decision-io-separation-and-replay-harness.md)が
定める「決定点への再投入（再生）」側もTH1a〜TH1c（`decide_gate`/`decide_chain`/
`decide_attempt`+crate内再生ハーネス、PR#195/#196）まで実装済みだが、再生ハーネスが
読むのは現時点では手組みfixtureのみで、実機ダンプからの凍結コーパス投入（TH1d）と、
そのコーパスを使った実際の削除・統合+差分ゼロ再生証明（TH1e、本ルールの発効条件そのもの）
はまだ着手していない。この条件が満たされるまで、本ルールは参考文書のまま強制しない。

## なぜこのルールが必要か（背景）

[ADR-158](../../docs/adr/158-complexity-reduction-north-star.md)のRC4（ガバナンスが
加算のみを義務化し、減算に報酬がない）への直接対策。`.claude/rules/fix-requires-evidence.md`
は「fixにはテストか記録を**添えろ**」であり削除を促す条項がなく、
[ADR-156](../../docs/adr/156-unify-deferred-execution-queues.md)の最終結論が
「唯一コストゼロで価値を出せる対策として、規約の表に1行足す」だったことが、この非対称性の
実例として記録されている。

## 関連

- [ADR-162](../../docs/adr/162-governance-reversal.md) E1（本ルールの起点）・
  E3（四半期定例棚卸し、超過の返済先）。
- [tuning-constants](./tuning-constants.md)（tuning定数の実測義務、本ルールの対象定数の
  変更規約）。
- [fix-requires-evidence.md](./fix-requires-evidence.md)（削除ではなく追記を促す既存規約、
  本ルールが反転しようとしている非対称性そのもの）。
