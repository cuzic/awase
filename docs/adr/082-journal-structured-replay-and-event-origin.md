# ADR-082: `journal.rs` を「事後ログ」から「構造化リプレイ基盤」へ格上げし、出所・世代の規律を横断型 `EventOrigin` 1箇所に統合する

## ステータス

「第一歩」・Phase 0.5 実装済み（2026-07-25、Linux 検証済み、詳細は「第一歩
実施記録」「Phase 0.5 実施記録」節）。当初は Claude Fable 5 との壁打ちから
起票した提案（未実装）だったが、`EventOrigin`/`Generation`/`EventSource` の
最小実装（第一歩）と、`JournalEntry::ImeActuation` 構造化 variant +
`Actuation` への `EventOrigin` 配線（Phase 0.5、ADR-081 Phase 1 着手前の
先行実施）まで完了している。`tests/drift_correction_replay.rs`（BUG-43）が
新 variant 経由で green であることを確認済み。

本 ADR が最終的に狙う「journal 構造化リプレイの全面適用」（`decide_alt_impersonation`
= BUG-41 系への拡張など、「Phase 0.5 実施記録」節の「次の一歩（推奨）」参照）は
未着手。

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

## 第一歩 実施記録（2026-07-25）

「第一歩」節の 1.〜3. を実施した。以下は実施内容と結果。**上の「提案中」本文
（ステータス含む）は変更していない** — この節は追記のみ。

### 1. `EventOrigin` 最小実装

`crates/awase-windows/src/state/event_origin.rs` を新設し、`Generation`
（`u64` newtype、`Copy`、`next()`/`is_newer_than()`）と `EventSource`
（`Physical` / `Injected { reason }` / `SelfActuated { strategy }`）、両者を
束ねる `EventOrigin { source, epoch }` を実装した。指示通り**既存コードへの
配線は一切行っていない**（`RawKeyEvent::injected`・`InputModeApplyStrategy`・
`WarmEpoch`・`cold_seq`・`Actuation.attempts` はいずれも無変更）。12件の
ユニットテストを添えた（`cargo test -p awase-windows --lib`、Linux で実行可能）。

### 2./3. BUG-43 の journal リプレイ実証

`decide_actuation_action`（`state/ime_actuation.rs`、既存の純粋関数、ADR-080）
の実引数 `(FeedbackPolicy, attempts: u32)` に合わせて `DriftCorrectionFixture`
（`policy` + `ticks: Vec<{attempts, observed_at_ms, expected}>`）を設計し、
`docs/known-bugs.md` BUG-43 の実機ログ（675ms の間に `apply_ime_open(false)` を
16 回連続送信、observe tick 20ms とほぼ同期、`duration_ms` 84502ms→85176ms）を
`tests/journals/drift_correction/bug-43-drift-correction-tight-loop.json` として
固定化した。`tests/drift_correction_replay.rs` が 16 tick 分 `decide_actuation_action`
をリプレイし、以下を assert する:

- 16 回の連続 drift 検知のうち実際に `Send` になるのは `max_attempts`（5）回だけ
  （残り 11 回は `GiveUp`）。
- 一度 `GiveUp` に達したら、tick 列の最後まで `Send` に戻らない。

**結果: 通った**（`cargo test -p awase-windows --test drift_correction_replay`
2 tests ok）。ADR-080 Phase1 の `Blind` ポリシーが BUG-43 と同じ入力パターン
（16 回の高頻度連続検知）に対して型レベルで有界終端することを、実機ログ由来の
固定フィクスチャで確認できた。

**実証できたこと**: 「実機で観測済みの入力パターンをフィクスチャとして固定化し、
純粋関数のリプレイで回帰を防ぐ」という `ConvClassifyFixture` と同じ枠組みが、
`classify_conv_transition` 以外の純粋関数（`decide_actuation_action`）にも
そのまま展開できた。既存の `tests/journal_replay.rs` を変更せず、新規
`tests/drift_correction_replay.rs` + `tests/journals/drift_correction/`
サブディレクトリ（`journal_replay.rs` の非再帰 `read_dir` と衝突しない）を
追加するだけで済み、作業量は「第一歩」2. が想定した「1本書く」の範囲に収まった。

**限界（重要）**: このフィクスチャは実機 journal ダンプからの機械的な転記では
**ない**。BUG-43 発生当時（ADR-080 Phase1 実装前）は actuation 呼び出しを
journal に記録する仕組みが存在しなかったため、`docs/known-bugs.md` の集計的な
記述（675ms/16回・平均間隔・`duration_ms` の始点終点）から `observed_at_ms` を
手で近似復元した。`ConvClassifyFixture` が実現している「実機ダンプの
`JournalEntry::ConvClassifyCall` をそのまま転記する」フローは、`Actuation`
呼び出しについてはまだ再現できていない（`journal.rs::JournalEntry` に
actuation 用の専用 variant が無いため）。

### 採用範囲を広げる推奨

**推奨: 進める。** ただし次の一歩は「別の純粋関数にもリプレイを広げる」前に、
**`journal.rs::JournalEntry` に actuation 呼び出しを記録する専用 variant を
追加する**ことを優先すべきと判断する。理由: 今回のフィクスチャが「known-bugs.md
の散文からの手作業復元」に留まったのは、まさに ADR 本文が課題として挙げている
「`ImeEvent { description: String }` が自由文字列で構造化されていない」ことの
別側面（actuation はそもそも journal に記録すらされていない）である。この専用
variant を追加すれば、次回同種のバグを実機で踏んだ際に `ConvClassifyFixture` と
同じ「ダンプ→転記→リプレイ」フローがそのまま使え、近似復元という弱点が解消される。

その上で、ADR 本文が示す優先順位（`decide_actuation_action` → 
`decide_alt_impersonation`（BUG-41） → `GjiFsm` 状態遷移関数（BUG-33 追補3・4））
通り、次に `decide_alt_impersonation` へのリプレイ基盤拡張に進むことを推奨する。

`EventOrigin`（1.）自体を既存コードへ配線するかどうかは、今回の検証範囲外
（意図的にスコープ外とした）であり、判断を保留する。`journal.rs` に actuation
の専用 variant を追加する際に、その variant が `EventOrigin` を必須フィールドと
して持つ設計にできるかを最初の配線候補として検討するのが自然な次点になる
（ADR 本文「決定 2.」が想定する経路そのもの）。

## Phase 0.5 実施記録（2026-07-25）

「第一歩 実施記録」の「採用範囲を広げる推奨」節が最優先とした
**「`journal.rs::JournalEntry` に actuation 呼び出しを記録する専用 variant を
追加する」** を実施した。**上の「提案中」本文および「第一歩 実施記録」節は変更して
いない** — この節は追記のみ。ADR-081 Phase 1（`ir_apply_drift_correction` の
ドライバ分離）が着手する前に、actuation 呼び出しの journal リプレイ回帰網を張る
ことが目的。

### 1. `JournalEntry::ImeActuation` 構造化 variant の追加

`journal.rs` に `ImeActuation { record: ActuationRecord }` を追加した。ペイロード
`ActuationRecord`（`state/ime_actuation.rs`）は出所・世代・目標値・方針・試行回数・
判定を型として持つ:

```rust
pub struct ActuationRecord {
    pub origin: EventOrigin,   // source は常に SelfActuated、epoch = 何回目の試行か
    pub target: bool,          // この試行が目指す IME open 状態
    pub policy: FeedbackPolicy,
    pub attempts: u32,
    pub action: ActuationAction, // decide_actuation_action で new() 内一意導出
}
```

`ImeEvent { description: String }`（ADR 本文が「決定 1.」で問題視した自由文字列）と
違い、`format!("{event:?}")` ではなく `source`/`epoch`/`action` を型として取り出せる。

**型定義を `state` 層に置いた**理由: `journal` モジュールは `#[cfg(windows)]` で
ゲートされているため、Linux のリプレイテストからは `JournalEntry` を直接参照できない。
ペイロード型を `state`（プラットフォーム非依存、ADR-065）に置くことで、Windows の
journal 記録と Linux のリプレイが**単一の型定義・単一の構築経路（`ActuationRecord::new`）
を共有**する。

### 2. `EventOrigin` を `Actuation` に配線

`runtime::ime_actuation::Actuation`（ADR-080 の実行時状態）に `origin: EventOrigin`
フィールドを追加。`actuation_for()` の新規構築時に
`actuation_origin(policy, Generation::INITIAL)` で初期化し（`target` 変化のたびに
`0` から振り直す）、実送信ごとに `attempts` と `origin.epoch` を同時に進める
`advance_epoch()` に集約した（別々に更新して片方を忘れる事故を構造的に防ぐ）。
`ir_apply_drift_correction` の **Send 側と Blind GiveUp 側の両方**で試行1回分を
`JournalEntry::ImeActuation` として記録する。これで BUG-43 の「16 回中 5 回だけ
`Send`・残り 11 回は `GiveUp`」が journal から型で追える。`GiveUp` 時に observations へ
書き込まない規約（ADR-080、BUG-33 型の収束偽装防止）は不変。

配線の核心（`strategy` 導出・`EventOrigin` 構築・`action` 導出）は `state` 層の純粋
関数（`actuation_strategy` / `actuation_origin` / `ActuationRecord::new`）に切り出し、
Linux でユニットテスト済み（`cargo test -p awase-windows --lib`）。`runtime/` 部分は
`#[cfg(windows)]` のため `cargo check --target x86_64-pc-windows-gnu --lib` の
コンパイル確認に留めた。

### 3. BUG-43 リプレイを新 variant 経由に更新

`tests/drift_correction_replay.rs` を `ActuationRecord`（= journal に積まれるのと
同一の構造化レコード）経由に更新した。fixture の各 tick に `epoch` フィールドを追加
（健全な系列では `epoch == attempts`）。従来の `action` 照合に加え、**(2) 出所が
常に `SelfActuated` であること・(3) `epoch` が `attempts` と歩調を合わせて積まれて
いること**（`EventOrigin` 配線の退行検知）を照合する。既存の 2 テスト構成と fixture を
最大限再利用した拡張であり、全面書き直しはしていない。

### 型の serde 方針（設計判断）

`EventSource` の `Injected { reason }` / `SelfActuated { strategy }` は `&'static str`
のため、`EventSource`・`EventOrigin`・`ActuationRecord` は **`Serialize` のみ**導出した
（任意入力から `&'static str` の借用を復元する `Deserialize` は型として表現できない）。
journal は書き出し専用なのでこれで足りる。リプレイ側（`DriftCorrectionFixture`）は
`Generation`（`u64` newtype、Ser/De 両対応）だけを保存し、`strategy` は
`actuation_strategy(policy)` で `policy` から一意に再構築するため、`EventSource` 自体の
`Deserialize` は不要。`event_origin.rs` の既存 12 テストと Copy・`&'static str` の
API は無変更のまま（ADR-081 が想定する `EventSource::SelfActuated` の Copy 前提を
壊さないため）。

### テスト結果

- `cargo test -p awase-windows --lib`: **169 件 green**（第一歩の 165 + Phase 0.5 で
  追加した 4 件: `actuation_strategy`/`actuation_origin` 系）。退行なし。
- `cargo test -p awase-windows --test drift_correction_replay --test journal_replay`:
  **3 件 green**（新 variant `ActuationRecord` 経由でも BUG-43 の有界終端が維持）。退行なし。
- `cargo check -p awase-windows --target x86_64-pc-windows-gnu --lib`: green
  （runtime 配線のコンパイル確認）。
- `cargo clippy -p awase-windows --lib --target x86_64-pc-windows-gnu -- -D warnings`
  / `cargo fmt --check`: clean。

### ADR-081 Phase 1d への申し送り

ADR-081 のドライバ `actuate()` から本 variant を積むには:

1. ドライバ内で actuation 試行を行う箇所で、`ActuationRecord::new(origin, target,
   policy, attempts)` を組み立て `JournalEntry::ImeActuation { record }` として
   journal に `record()` する。`origin` は `Actuation.origin`（本 Phase で配線済み）を
   そのまま渡せばよい。ドライバが独自に `EventOrigin` を作る場合も
   `actuation_origin(policy, epoch)` を通すこと（`strategy` 文字列の唯一の定義点。
   直書きしない）。
2. `ir_apply_drift_correction` を書き換える際、本 Phase で入れた 2 箇所の
   `record()`（Send 側・GiveUp 側）を**ドライバ側に移設**する形にすれば、リプレイ
   テストは `ActuationRecord` 経由のままなので**変更不要で回帰網として機能する**
   （テストは journal モジュールに依存せず `state::ActuationRecord` を見ているため、
   ドライバ移設で壊れない）。
3. `advance_epoch()` は `attempts` と `epoch` を必ず一対で進める。ドライバが試行回数を
   独自管理する場合も、この不変条件（`epoch == その actuation 系列の試行回数`）を
   保つこと。リプレイテスト (3) がこの一致を検証しているため、崩すと落ちる。

### 次の一歩（推奨）

ADR 本文の優先順位通り、次は `decide_alt_impersonation`（BUG-41）へリプレイ基盤を
広げる。ただし BUG-41 は「非同期確認の世代誤帰属」ではなく「KeyUp でのローカル状態
クリア漏れ」という別形状（ADR 本文コンテキスト参照）なので、`ActuationRecord` を
そのまま流用するのではなく、`decide_alt_impersonation` 専用の fixture 型を新設する
（`DriftCorrectionFixture` と同じ「実機観測を固定化する」枠組みは共有）。

## 決定1 実施記録（2026-08-01）

「決定 1.」（`journal.rs::JournalEntry::ImeEvent { description: String }` の廃止、
実 `ImeEvent` をそのまま記録する）を実施した。**上の「提案中」本文・既存の実施記録
節は変更していない** — この節は追記のみ。ADR-082 Phase 0.5 の「採用範囲を広げる
推奨」節が次点として挙げていた項目であり、BUG-41/BUG-33 へのリプレイ拡張に着手する
前段として先行実施した（Linux サンドボックスでの Opus 2周レビューにより、当初の
優先順位案から繰り上げと判断された）。

### 実施内容

- `state/ime_event.rs::ImeEvent` および全サブ型（`HwndId`/`UserIntentSource`/
  `ObservationConfidence`/`ImePolicyProfile`/`ChordKind`/`ApplyError`/
  `InputModeApplyStrategy`/`InputModeApplyResult`）に `serde::Serialize` を derive。
  `ObservationSource`/`InputModeState` は既存で対応済み。`state/mod.rs::TickMs` にも
  同様に追加。書き出し専用のため `Deserialize` は導出しない（`ActuationRecord` と
  同じ方針。全フィールドがプレーンな値のみで構成されるため機械的に導出可能）。
- `journal.rs::JournalEntry::ImeEvent` を `{ description: String }` から
  `{ event: crate::state::ime_event::ImeEvent }` に置換。
- `state/platform_state.rs::dispatch_event()` の `format!("{event:?}")` 呼び出しを削除し、
  `event.clone()` をそのまま journal へ渡すよう変更。

### テスト結果

- `cargo test -p awase-windows --lib`: **218 passed / 0 failed**（Phase 0.5 の 169 から、
  本 ADR とは無関係な他タスクの追加分を含め増加、退行なし）。
- `cargo check -p awase-windows --target x86_64-pc-windows-gnu --lib` / `cargo clippy`
  （Linux・windows-gnu 両方 `-D warnings`）/ `cargo fmt --check`: いずれも green。

### 次の一歩

`decide_alt_impersonation`（BUG-41）・`GjiFsm` 状態遷移関数（BUG-33 追補3・4）への
拡張は、本 ADR 本文の優先順位通り次点。ただし Opus レビューにより、BUG-41/BUG-33 の
両方とも「実機でしか観測できない事象の journal 固定化」に該当しないと判定された
（BUG-41 は既存の決定論的ユニットテストの回帰であり、BUG-33 追補3・4 も同様に既存
テストで固定化済み）ため、journal リプレイ fixture ではなく「Linux で実行可能にする
（cfg(windows) ゲートの外に出す）+ 網羅的な不変条件テストを追加する」方針に変更して
実施した。詳細は `.claude/rules/` 配下ではなく、当該コミットの本文および
`docs/known-bugs.md` の該当 BUG 節を参照。
