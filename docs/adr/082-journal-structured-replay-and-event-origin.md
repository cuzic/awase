# ADR-082: `journal.rs` を「事後ログ」から「構造化リプレイ基盤」へ格上げし、出所・世代の規律を横断型 `EventOrigin` 1箇所に統合する

## ステータス

提案中（2026-07-25、Claude Fable 5 との壁打ちから起票、未実装）。

fable の評価: 「どの案を選んでも検証コストを下げ、選ばなくても現行
アーキテクチャの延命に効く。唯一の後悔しない手」。本 ADR は
ADR-081（プロファイル別ドライバ分離）の採否に関わらず独立に進める
価値がある。

## コンテキスト

### 現状の `journal.rs` は「観測ログ」であって「唯一の書き込み経路」ではない

`crates/awase-windows/src/journal.rs`（`UnifiedJournal`）は既に実装済みで、
ホットキー（Alt+変換→Alt+無変換 を2回連続）で直近 `DEFAULT_CAPACITY=2048`
件のイベントを `%TEMP%/awase_journal_<tick_ms>.json` にダンプできる。
`docs/journal-replay-guide.md` により `ConvClassifyCall` エントリを
`tests/journal_replay.rs` でリプレイする回帰基盤も既にある。

しかし現状は2つの点で「本物の event-sourcing」になっていない。

1. **状態変化は journal と独立に起きる**。`reduce()` / `dispatch_event()`
   が実際の belief 更新を行い、`journal.record(...)` はその**後から**
   別途呼ばれるだけの副次的なログ出力にすぎない。journal を再生しても
   状態が再構成される保証は無く、単に「何が起きたかを見返せる」もの。
2. **`ImeEvent` エントリが構造化されていない**（`journal.rs:110-111`）:

   ```rust
   /// IME 状態変更イベント（dispatch_event 経由の全 ImeEvent）
   ImeEvent { description: String },
   ```

   `format!("{event:?}")` 相当の自由文字列であり、`source`/`epoch`/
   `confidence` を型として取り出せない。リプレイ対象は
   `classify_conv_transition` 単体（`ConvClassifyCall`、純粋関数1つ）に
   限定されている（`journal-replay-guide.md` 「現在のスコープ（MVP）」に
   明記）。

### 「出所・世代の規律」は境界1点ではなく、発見のたびに個別追加されてきた

`.claude/rules/ime-belief-architecture.md` の「`ImeModel` 以外の belief 的
状態への適用範囲（2026-07-23 追記）」節が、この問題を自ら文書化している:
`GjiFsm` の warm/cold 判定が「弱い代理指標（`AtomicBool` の素読み）だけで
無条件に belief を書き換えていた」ことが実機バグ2件（BUG-33 追補3・4）の
根本原因だった。同じ監査で `ForceGuardSet.guards` の直接フィールド操作
（private 化で対処）、UIA 非同期結果ハンドラの no-op 保証欠如
（`architecture_guard.rs` のテキスト走査で対処）も見つかっている。

同種の「規律の後付け」は他にも複数回起きている:

- **BUG-06 → ADR-069**: `focus_epoch: u32` の wraparound リスクを
  `WarmEpoch` 構造体の再設計で解消。
- **BUG-35 → ADR-079**: per-VK confirm が世代をまたいだ stale な confirm
  根拠を現世代の証拠として誤用。epoch fencing + replay で対処。
- **BUG-39 → `cold_seq` 世代管理**: `literal_session_confirmed` が
  「次にたまたま epoch 付きで HIDE がドレインされるまで」true であり
  続ける欠陥を、明示的な `cold_seq` 不一致検知で修正。
- **BUG-43 → ADR-080 Phase 1**: actuation の completion イベントが
  `generation: None` のため observation store へフィードバックされず
  無限ループ。`Actuation`/`FeedbackPolicy` で型強制。
- **BUG-41**（関連だが少し異なる形状）: `decide_alt_impersonation` の
  KeyUp 後の判定が stuck true のまま持ち越され、無関係な後続 Alt 押下まで
  補正対象にした。こちらは非同期確認の世代誤帰属ではなく、KeyDown 時に
  記録したローカル状態を KeyUp で正しくクリアし損ねたという別形状の
  欠陥だが、「出所・世代を明示的に持たない派生状態が stale なまま
  参照され続ける」という根はBUG-06/35/39/43と共通する。

BUG-06/35/39/43 はいずれも「非同期に届く確認情報が、どの世代の要求に
対する応答か」という**同一形状の欠陥**の個別インスタンスである。
`RawKeyEvent` は
`injected: bool` フィールド（`src/types.rs:201`、BUG-14 を教訓に追加済み）
で「出所」は境界で型付けされているが、そこから派生する各 FSM・キャッシュ・
フラグ（`GjiFsm` の warm epoch、`ALT_L_IMPERSONATING`、
`literal_session_confirmed`、drift correction の `Actuation`）は、
それぞれ独立に「出所・世代を追跡する仕組み」を再発明してきた。統一された
型がないため、新しい belief 的状態を追加するたびに、この規律を実装し
忘れる／気づかず省略するリスクが構造的に残る。

## 決定

### 1. `ImeEvent` エントリを構造化し、`source`/`epoch` を必須フィールド化する

`journal.rs::JournalEntry::ImeEvent { description: String }` を廃止し、
実際の `ImeEvent`（`state/ime_event.rs`）を `Serialize` 対応にした上で
そのまま記録する。これにより journal が「読める」だけでなく「そのまま
再生できる」形式になる。

### 2. `EventOrigin { source: EventSource, epoch: Generation }` を横断型として導入する

`EventSource`（`Physical` / `Injected { reason }` / `SelfActuated { strategy }`
等、`RawKeyEvent::injected` や `InputModeApplyStrategy` が個別に表現して
いた概念を統合）と `Generation`（`WarmEpoch`/`cold_seq`/`Actuation` が
個別実装していた世代カウンタを統合、`u64` の newtype）を1箇所
（`state/event_origin.rs` 新設）で定義する。journal に記録される全ての
「非同期に届く確認・観測・完了通知」系エントリは `EventOrigin` を必須
フィールドとして持つ（コンパイラ強制）。

このステップは新規発明ではなく、`ime-belief-architecture.md` が既に
実践している「private フィールド化 → dylint → architecture_guard の
テキスト走査」という3段防御のうち、**最初の段（型でそもそも表現できない
ようにする）を、これまで対象ごとに個別実装してきた epoch/source 追跡の
全てに対して1回だけ行う**という位置づけ。

### 3. リプレイ対象を段階的に広げる

`journal-replay-guide.md` が既に示している拡張路線（「将来的に他の
純粋関数（`classify_idle`, `classify_fetched_snapshot` 等）にも同様の
仕組みを広げられる」）を実行する。優先順位は「BUGとして踏んだ実績が
ある純粋関数」を先にする: `decide_actuation_action`（ADR-080）、
`decide_alt_impersonation`（BUG-41）、`GjiFsm` の状態遷移関数
（BUG-33 追補3・4）の順。

### 4. `fix-requires-evidence.md` の (a) 回帰テストの主経路として journal リプレイを格上げする

現行の pre-push チェックは「対象ファミリーの変更に対して
`crates/awase-windows/tests/` 配下（golden・journal リプレイ・
architecture_guard 等を区別しない）か `docs/known-bugs.md` のいずれかに
差分があるか」を機械的に見ているだけで、journal リプレイは既に (a) の
テストとして認められている。本 ADR が変えるのは仕組みではなく**運用上の
優先順位**: 対象ファミリー（warmup/focus/belief/conv/キー選択）の fix で
journal ダンプが取得可能な場合、golden をゼロから書くより先に journal
リプレイ回帰テストの追加を第一選択として `fix-requires-evidence.md`
本文で案内する（golden はシナリオ全体を人手で設計、journal リプレイは
実機で偶然踏んだ入力の組合せをそのまま固定化、という役割分担を
明文化する）。

### 意図的に「していない」こと

- **状態の唯一の書き込み経路を journal.record() 自体にする**（真の
  event-sourcing、reducer が journal を再生することで状態を作る）とは
  しない。既存の `reduce()` / `dispatch_event()` を置き換える大改修は
  リスクに対してリターンが不確実なため、まず「型と記録の統合」に留める。
  将来的にここまで踏み込むかは Phase 2 以降で判断する。
- belief 更新の規律（Observe → Pure → Apply）自体は変更しない。
  `ime-belief-architecture.md` の3層防御パターンとは競合しない、その
  上位に位置する横断的インフラという位置づけ。

## 第一歩

BUG-43（drift correction 無限再送、`Runtime.active_actuation` /
`Actuation`/`FeedbackPolicy`）を題材に検証する。理由: 実機ログが
既に手元にある（675ms 間に16回連続送信、`docs/known-bugs.md` BUG-43）、
かつ ADR-080 Phase 1 で型強制を先行実装済みのため、「journal リプレイで
このバグが再現できるか」を検証する対象として最も情報が揃っている。

1. `EventOrigin` を最小実装（`Generation` newtype + `EventSource` enum の
   み、既存コードへの配線はしない）。
2. `ir_apply_drift_correction` の入出力を `ConvClassifyFixture` と同様の
   専用 `DriftCorrectionFixture` として手で書き起こし（BUG-43 の実機ログ
   から）、`decide_actuation_action`（既に純粋関数）をリプレイして
   `GaveUp`/`Resolution` が期待通りかを検証するテストを1本書く。
3. 通れば「journal 経由で実機発見済みバグを機械的に固定化する」流れが
   実証されたことになり、対象を広げる。通らない・手間が見合わない場合は
   Phase 1 をここで止め、`journal-replay-guide.md` の現状（MVP スコープ）
   を維持する。

## 不変条件（Phase 1 完了後にテスト/型で強制する候補）

- `EventOrigin` を持つべき型（`Actuation`, `GjiFsm` の内部状態,
  `AltImpersonation` フラグ, `literal_session_confirmed`）が
  `EventOrigin` フィールドを持たずに世代依存の判定を行っていないかを
  `architecture_guard.rs` のテキスト走査で検知する（`ime-belief-
  architecture.md` の UIA ハンドラガードと同型）。
- `journal.rs::JournalEntry` の新規 variant 追加時、`EventOrigin` を
  含む型は `Serialize`/`Deserialize` 両対応であることをテストで固定する
  （リプレイ対象からの脱落を防ぐ）。

## 関連

- [[project_integration_unmerged_branches_2026_07_25]]
- `docs/journal-replay-guide.md`、`crates/awase-windows/src/journal.rs`
- ADR-069（`WarmEpoch`）、ADR-079（epoch-fenced literal recovery）、
  ADR-080（`Actuation`/`FeedbackPolicy`） — 本 ADR が統合を狙う個別実装群
- `.claude/rules/ime-belief-architecture.md`（2026-07-23 追記節が
  同型の問題を先行して文書化している）
- `.claude/rules/fix-requires-evidence.md`
