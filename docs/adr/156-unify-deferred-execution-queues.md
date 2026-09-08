# ADR-156: `INPUT_DEFER`/`pending_deferred`/`deferred_engine_timers` を単一の遅延実行機構へ統合する（将来構想・未採用）

## ステータス

**将来構想として起票（未レビュー、opus-adversarial-consult 未実施、実装
未着手）。着手条件は「今後の議論」節を参照——現時点では着手判断を保留し、
同種の見落としが実例として積み上がるかを観察する。**

## 背景

### 動機: 4本の ADR に共通する構造パターンの発見

[ADR-121](121-explicit-physical-ime-key-idempotent-reassert.md)、
[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md)、
[ADR-128](128-escape-composition-collateral-deferred-loss.md)、
[ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)
を並べて読み直したところ、個別の症状（IME belief の no-op ガード、
`pending_deferred` の追い越し、drain-before-send の gate 見落とし、
親指タイムスタンプのライブ再取得）の背後に、共通する2つの構造パターンが
見つかった。

**パターン1: 遅延実行キューが複数独立に存在し、それぞれが「いつ安全に
flush/再送/replay してよいか」のガード条件を個別に再発明している。**

現状、少なくとも3つの独立した遅延キュー実装がある。

| キュー | 何を待避するか | 所有ファイル | 解放条件の判定箇所 |
|---|---|---|---|
| `INPUT_DEFER` | `OUTPUT_GATE` active 中のキーイベント全体 | `input_defer.rs`, `message_handlers.rs::handle_wm_drain_output_queue` | `OUTPUT_GATE` の depth |
| `pending_deferred` | TSF probe/recovery 中に確定できない VK | `output/tsf_warmup_coord.rs` | `has_pending_tsf()`/`raw_recovery_owns_deferred()`/`!pending_deferred.is_empty()`（[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md) 決定で3-way OR に拡張、[ADR-128](128-escape-composition-collateral-deferred-loss.md) 決定で `DeferGate::Enforced`/`Exempt` を追加） |
| `deferred_engine_timers` | `OUTPUT_GATE` active 中のタイマー | `message_handlers.rs:600-601` | `OUTPUT_GATE` の depth（`INPUT_DEFER` と同じ gate だが、別のキュー・別の replay 経路） |

[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md) が
`pending_deferred` に新しい解放条件（`gate: DeferGate`）を導入した際、
[ADR-128](128-escape-composition-collateral-deferred-loss.md) が確定させた
とおり、**別の関数（`drain_pending_deferred_before_send_if_queue_only`）が
その条件を引き継いでおらず、無条件発火のまま残っていた。** これは
「新しい防御条件を1箇所追加したら、全ての呼び出し元に配線しないと1箇所
だけ古いまま残る」という、`.claude/rules/fix-requires-evidence.md` の
「IME actuation 合流点」表が IME 書き込み経路について既に警告している
のと同型の問題が、**defer/replay キューの解放条件についても存在する**
ことを示している。ただし IME actuation 合流点と異なり、defer キューの
解放条件には合流点の一覧表が存在しない。

**パターン2: 「発生時点のコンテキスト」ではなく「処理実行時点のライブな
グローバル状態」を読んでしまう箇所が、直しても直しても別の場所に残る。**

[ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md) が
確定させた `phys.right_thumb_down`（`hook::thumb_down_timestamps()` の
ライブクエリ）の問題は、**同じクラスの問題が `RawKeyEvent::
modifier_snapshot`（Ctrl/Shift/Alt/Win）で既に一度解決されていた**にも
関わらず、親指タイムスタンプには適用されていなかった、という事実の上に
成り立っている。さらに ADR-129 自身の修正はキーイベント経路のみを閉じ、
タイマー経路（`deferred_engine_timers` の replay）は
[ADR-155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md)
へ切り出さざるを得なかった——**同じ設計判断（capture 時点でスナップ
ショットを取る）が、フィールドごと・経路ごとにバラバラに、しかも都度
「見つかったら直す」形でしか適用されていない。**

### なぜ「共通パターン」として扱う価値があるか

ADR-121/123/128/129 はいずれも個別の実機不具合報告（BUG-37、issue #148、
BUG-109、report `01M1N36MGDDJ5HN8FWRE4ZHS3J`）から出発しており、修正自体は
それぞれ局所的で正しい。しかし、**この4件が同じ半年弱の期間に、同じ2つの
構造パターンの異なるインスタンスとして独立に発見・修正されている**という
事実そのものが、個別修正を積み重ねるだけでは同じクラスのバグが今後も
（5件目、6件目として）再発し続けることを示唆している。

本 ADR は「今すぐ大改造する」ことを提案するものではなく、**この観察を
将来の設計判断として記録し、着手すべきタイミングの条件を明記しておく**
ことを目的とする（[experiment-logging](../../.claude/rules/experiment-logging.md)
と同種の「早すぎる大改造より実例の蓄積を優先する」というこのリポジトリの
一貫した進め方に沿う）。

## 決定（構想レベル、詳細設計は着手時のレビュー対象）

### 決定1: 3つの遅延キューを単一の汎用機構に統合する方向性

`INPUT_DEFER`・`pending_deferred`・`deferred_engine_timers` を、「何を」
「なぜ待避しているか（所有者トークン）」「いつ解放してよいか（解放条件
の関数）」を持つ単一のジェネリックな待避機構（暫定名
`DeferredExecutionQueue<T>`）に統合する。各キューが個別に再発明している
解放条件の判定述語（`has_pending_tsf()` 等）を、キュー種別ごとの分岐では
なく「所有者トークン＋解放条件」という共通の型で表現できれば、
[ADR-128](128-escape-composition-collateral-deferred-loss.md) のような
「新しい条件を1関数だけ忘れる」という失敗が構造的に起きにくくなる
（コンパイラが「この解放条件を全ての解放経路に適用したか」をチェック
できる形にする、が具体的な型設計は着手時に詰める）。

### 決定2: capture 時点のコンテキストを型で保持する共通カプセル

`RawKeyEvent::modifier_snapshot` が確立した「capture 時点で1回読んで
運ぶ」パターンを、個別のフィールド追加（本 ADR 起票時点で
`modifier_snapshot`・`left/right_thumb_down_snapshot`
[ADR-155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md)、
将来増えうる belief 関連スナップショット等）としてではなく、1つの
capture-time コンテキストカプセル型にまとめる。遅延実行機構（決定1）が
扱う `T` は、このカプセルを内包することを型で強制する。

### 決定3: `architecture_guard.rs` に「defer/replay 経路からのライブ
グローバル参照」を横断的に禁止するガードを追加する

[ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md) が
`hook::thumb_down_timestamps()` 1関数について個別に追加したテキスト走査
ガードを一般化し、「決定1の遅延実行機構経由で呼ばれるコード（`execute`
相当の関数群）から、`OnceLock`/`AtomicU64` 等のグローバル状態を直接
読む関数を呼んではならない」という規則を、対象関数のホワイトリストでは
なくパターンとして表現する。次に別のグローバル状態で同じ穴が開いた
とき、実装前に CI で気づけるようにすることが狙い。

## 検討した代替案

### 代替案A（現状維持）: 個別修正を今後も都度積み重ねる

最も低コストだが、「背景」節で述べたとおり、同じ2つのパターンが既に4回
独立に発見・修正されている実績があり、5件目・6件目が起きる確率は
低くないと見積もる。ただし、今すぐ決定1〜3を実装する追加コスト（3つの
キューの型・呼び出し元の棚卸し、実機ソーク込みの移行）は決して小さくなく、
「大改造の価値がコストに見合うか」を判断できるだけの実例（現状4件）が
十分かどうかは本 ADR 単独では判断しない。

### 代替案B（棄却）: [ADR-152](152-keystroke-step-source-sink-pipeline.md)
の `KeyStrokeStep`/`StepOwnership` に統合する

ADR-152 は「誰が IME actuation を所有するか」という**所有権**の軸を
1つの型に統合する構想であり、本 ADR が扱う「いつ・どの文脈で遅延実行
してよいか」という**タイミング**の軸とは直交する別の関心事である。
両者が最終的に同じ `KeyStrokeStep` 的な型の上に実装される可能性は
否定しないが、ADR-152 自体が [ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)
の force-ON 救済 Blocker が解決するまで着手できないブロック状態にある
ため、本 ADR をそれに従属させると本 ADR も同じブロッカーを（不必要に）
背負うことになる。**本 ADR は ADR-151/152 とは独立に検討・着手できる
別軸の構想として扱う。**

## 今後の議論（着手条件）

**本 ADR は次のいずれかが満たされるまで着手しない:**

1. 同種の「defer キューの解放条件を1箇所だけ見落とす」バグが、
   ADR-121/123/128/129 とは別のキュー・別の条件でさらに1〜2件発見される
   （＝パターン1が「4件の偶然の一致」ではなく「構造的に繰り返す」ことが
   より強く裏付けられる）。
2. または、`.claude/rules/tuning-constants.md`/`fix-requires-evidence.md`
   の運用と同様、pre-push フックや `architecture_guard.rs` による軽量な
   「合流点一覧表」の維持（決定3の縮小版、統合そのものは行わない）だけで
   十分に再発を抑止できるかを、まず試す。

**着手する場合の前提として詰めるべき論点（opus-adversarial-consult 対象）:**

1. **実装コスト・影響範囲**: `INPUT_DEFER`/`pending_deferred`/
   `deferred_engine_timers` それぞれの型・呼び出し元・テストの棚卸しが
   必要。3つのうち最も呼び出し元が少ないものから段階的に統合するか、
   一括で置き換えるか。
2. **`DeferredExecutionQueue<T>` の生成コスト**: 全キーイベント/タイマーで
   都度アロケーションすると、`hook.rs`/`key_pipeline.rs` の低レイテンシ
   要求（ミリ秒単位）に影響しないか。[ADR-152](152-keystroke-step-source-sink-pipeline.md)
   が `KeyStrokeStep` について挙げた同種の懸念（論点2）と同じ検証が要る。
3. **既存の個別最適化との衝突**: `pending_deferred` の件数上限32
   （[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md)
   決定の暫定値）、`OUTPUT_GATE` の depth カウンタ等、キュー種別ごとに
   個別にチューニングされている値・挙動が、汎用機構への統合でどこまで
   保存できるか。
4. **段階的移行の順序**: 3つのキューを同時に置き換えるか、最も直近に
   見落としが発覚した `pending_deferred` から先行するか。
5. **決定3（ガードの一般化）単独での先行実装可否**: 決定1〜2（統合本体）
   に着手しなくても、決定3（横断的なライブ参照禁止ガード）だけを先に
   実装し、再発の早期検知策として単独で価値を出せるか。

## 関連

[ADR-121](121-explicit-physical-ime-key-idempotent-reassert.md)、
[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md)、
[ADR-128](128-escape-composition-collateral-deferred-loss.md)、
[ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)
（本 ADR の動機となった4本、パターン1・2の実例）、
[ADR-155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md)
（パターン2の5件目のインスタンス、局所修正として先行実装）、
[ADR-152](152-keystroke-step-source-sink-pipeline.md)（所有権の軸を統合
する隣接構想、本 ADR とは直交・独立）、
`.claude/rules/fix-requires-evidence.md`
の「IME actuation 合流点」表（同型の合流点管理の先例）、
`.claude/rules/experiment-logging.md`（大改造より実例蓄積を優先する
運用方針の先例）。
