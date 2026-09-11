# ADR-163 Part D（TH1d'）実装タスク一覧

[ADR-163](163-actuation-decision-io-separation-and-replay-harness.md)「Part D」節の決定D1〜D8を、
実装可能な単位に分割したタスクリスト。`docs/adr/158-implementation-tasks.md`と同じ形式
（内容・受け入れ基準・依存）を踏襲する。各タスクは個別のコミットにすること
（[granular-commits](../../.claude/rules/../../../.claude/CLAUDE.md)の精神、
このリポジトリの慣習）。

対象領域は`.claude/rules/fix-requires-evidence.md`の「IME actuation 合流点」
「defer/replayキューの解放条件」ファミリーに該当するため、各コミットは回帰テスト
（golden/journalリプレイ/characterization）を伴うこと。

## 実装順序

T8（TH1d、既知バグfixture投入）は他タスクと独立でいつ実施してもよい。それ以外は
T0 → T1 → T2 → T5 → T6 → T4 → T3 → T7 の順に依存する。

---

### 163-T0（決定D7、前提条件）: `AttemptRecord`にBUG-113上書き前の値を追加する

**内容**: [ADR-163](163-actuation-decision-io-separation-and-replay-harness.md) Part B
「attempt単位の決定点ジャーナルと凍結コーパス」節が「`view.control.shadow_on`への
BUG-113追補上書き（`runtime/open_chain.rs`の`fallback_write`、決定入力の改変であり
決定そのものではない——記録は上書き後の値でよいが、上書きが発生した事実自体は別
フィールドで残す）」と約束していたのに、現行`AttemptRecord`
（`crates/awase-windows/src/state/actuation_decision_record.rs`）に該当フィールドが
無い（TH1cの実装漏れ）。

- `AttemptRecord`に`shadow_on_before_bug113_override: Option<Option<bool>>`
  （`post_failed_reobservation`と同じ`Option<Option<bool>>`パターン——「未取得」と
  「取得してfalse」を区別する）を追加する。
- `fallback_write`が`view.control.shadow_on = None;`で上書きする直前の値をここに
  記録する呼び出しを追加する。
- `state/actuation_decision_record.rs`内の手組みテストfixture
  （`replay_accepts_a_hand_built_sync_gji_direct_record`等）を新フィールド込みで
  更新する。

**受け入れ基準**: `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`
が通る。`cargo test -p awase-windows --lib`（ホストターゲット、Linux実行可）の
`state::actuation_decision_record::tests`配下が全green。BUG-113型の回帰
（上書き前後の値の混同）を検出できることを示す新規テストケースを1つ追加する
（`replay_detects_a_tampered_command`と同じ手法で、上書き前の値を改ざんしたら
再生が検出することを確認する）。

**依存**: なし。

---

### 163-T1（決定D1）: journal相乗り——ungatedなpub型+`JournalEntry` variant

**内容**:

1. `crates/awase-windows/src/state/mod.rs:98`の
   `#[cfg(test)] pub(crate) mod actuation_decision_record;`を
   `pub mod actuation_decision_record;`（ungated・pub）に変更する。
2. 同モジュール内の型（`ActuationDecisionRecord`/`AttemptRecord`/
   `ActuationOrderRecord`/`EventOriginRecord`/`EventSourceKind`）を`pub`にする。
3. `crates/awase-windows/src/state/ime_actuation_decision.rs`の
   `DecisionInputs`/`DecisionSite`/`MechanismCommand`も`pub`にする
   （`:93-94`の`#[allow(dead_code)] pub(crate) mod ime_actuation_decision;`も
   `pub mod`へ）。
4. 再生ハーネス本体（`state/actuation_decision_record.rs`内の`#[cfg(test)] mod tests`）
   は変更しない——crate内`#[cfg(test)]`のまま維持する（Part B M9方針の継続）。
5. `crates/awase-windows/src/journal.rs::JournalEntry`に新variant
   `ActuationDecision { record: ActuationDecisionRecord }`を追加する。
6. `decide_gate`/`decide_chain`/`decide_attempt`の呼び出し箇所
   （`ime_controller.rs:546,602,241`、`runtime/executor.rs:829`、
   `runtime/open_chain.rs:157,339,388`）で、呼び出し結果から
   `ActuationDecisionRecord`を組み立て、`UnifiedJournal`（`with_app`経由）へ
   `JournalEntry::ActuationDecision`として記録する呼び出しを追加する。
7. lane配置（既存`LaneKind::Actuation`への相乗り、または専用の新規`LaneKind`）を
   決める。数値（capacity）は決め打ちせず、既存`LaneKind::Actuation`
   （capacity=512、`journal_policy.rs`）と共有した場合の既存エントリへの影響を
   実測してから記す（[tuning-constants](../../.claude/rules/tuning-constants.md)
   の精神）。

**受け入れ基準**: `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows`
が通る。`crates/awase-windows/tests/architecture_guard.rs`の関連する件数ガード
（`raw_mechanism_write_sites_are_confined_to_chain_writers`等）を確認し、想定外の
増加が無ければそのまま、変わっていれば期待値を更新する。journalダンプに
`ActuationDecision`エントリが実際に出現することを示す回帰テストを
（`journal.rs`の既存単体テストパターンに倣って）追加する。

**依存**: 163-T0。

---

### 163-T2（決定D2）: `chain`/`attempts`をVecから固定長配列にする

**内容**: `WriteMechanism::ALL`は`[Self; 4]`
（`state/actuation_chain.rs:162`）で最大4機構固定のため:

- `ActuationDecisionRecord.chain: Vec<WriteMechanism>` →
  `[Option<WriteMechanism>; 4]` + 使用数（または同等の固定長表現）。
- `ActuationDecisionRecord.attempts: Vec<AttemptRecord>` →
  `[Option<AttemptRecord>; 4]` + 使用数。
- `AttemptRecord`自体も可能な限り`Copy`にできる構成を優先する（現状の
  フィールド構成なら到達可能——`Vec`/`String`を含まない）。
- 再生ハーネス（`replay_record`等）・既存テストfixtureを新しい表現に合わせて
  更新する。

**受け入れ基準**: `cargo clippy --target x86_64-pc-windows-msvc -p awase-windows`
がclean。`cargo test -p awase-windows --lib`のreplayテスト全green。実際に
actuationのホットパス（`ime_controller.rs::apply_mechanism`等、163-T1で記録呼び出しを
追加した箇所）にヒープ確保（`Vec::push`/`Box::new`等）が無いことを目視確認し、
コミット本文に明記する（TF2再開条件、ADR-163「TF2との突合せ」節参照）。

**依存**: 163-T1。

---

### 163-T5（決定D5）: `DecisionSite`に2バリアントを追加する

**内容**: `fix-requires-evidence.md`表が5番目・6番目の独立入口として挙げる
`runtime/mod.rs::reassert_explicit_physical_key`（`:1028`付近）・
`force_on_and_correct_romaji`（`:1195`付近）は、現状`platform.apply_ime_open_with_view`
→`ImeController::apply`経由で`site: Sync`として記録され、他のSyncレコードと
区別がつかない。

- `DecisionSite`に`ReassertExplicitPhysicalKey`/`ForceOnRomajiCorrection`を追加する。
- 上記2箇所の呼び出し経路で、`ImeController::apply`にこの2つのsiteを伝搬できるよう
  シグネチャを拡張する（既存の`Sync`固定呼び出しとは別扱いにする）。
- ADR-163「検出できる回帰・できない回帰」節が「本ハーネスでは検出できない」と
  明記しているこの2経路のレコードが、TH1eの差分ゼロ検証の母数に無自覚に混入しない
  ことを確認する（再生ハーネス側でこの2 siteを明示的に除外またはラベル付けする）。

**受け入れ基準**: `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`
が通る。`tests/architecture_guard.rs`の`.apply_ime_open_with_view(`件数ガード
（現状4）を確認し、シグネチャ変更で数が変わっていないか検証する。

**依存**: 163-T1。

---

### 163-T6（決定D6）: `with_app`再入時の記録漏れをカウンタで可視化する

**内容**: `UnifiedJournal`は`PlatformState`（`with_app`経由でのみ到達可能）の中に
あるため、`run_open_chain_async`/`imm_cross_write`のfail-open再入時
（`with_app(...).unwrap_or(false)`が`None`側に落ちるケース、`open_chain.rs:154-161,300,379-386`）
は記録できない。再入自体は解決しない。

- 既存の`dropped_by_lane`と同じパターンで、「`with_app`再入により
  `ActuationDecisionRecord`の記録をスキップした回数」を数える`AtomicU64`カウンタを
  追加する。
- このカウンタを診断ログ（`tracing::debug!`）に定期的に出す、または
  bug reportの既存カウンタ系フィールド（`retro_eval_stats`と同様の扱い）に
  相乗りさせるかを実装時に判断する。

**受け入れ基準**: `with_app`が`None`を返すケースを模したユニットテストで、
カウンタが増加することを確認する。

**依存**: 163-T1。

---

### 163-T4（決定D4）: `app_version`スタンプ + 「TH1e完了まで死蔵」の明記

**内容**:

- 163-T1で`JournalEntry`に相乗りさせた場合、`JournalEnvelope`が`seq`/時刻を
  既に付与するかを確認する（付与していれば重複フィールドを避ける）。
  ビルドのバージョン識別（`bug_report.rs::BugReportInput::app_version`相当）が
  `JournalEnvelope`に含まれていなければ、`ActuationDecisionRecord`に
  `app_version: String`を追加する。
- `site ∈ {ImmCrossWrite, RunOpenChainAsync, DispatchImeSetOpen}`のImmCross
  attemptは、TH1e完了（`decide_attempt`がImmCrossを扱うようになる）まで自動差分
  証明の対象外であることを、`state/actuation_decision_record.rs`のモジュールdocと
  `docs/journal-replay-guide.md`に明記する。

**受け入れ基準**: fixture round-trip（記録→シリアライズ→デシリアライズ→再生）の
テストに`app_version`フィールドが含まれることを確認する。

**依存**: 163-T1。

---

### 163-T3（決定D3）: characterization corpusであることの文書化

**内容**: `state/actuation_decision_record.rs`のモジュールdoc（TH1cが書いた
「本タスク時点では未投入」等の記述の近く）に、Part Dで集まるレコードは
correctness corpusではなくcharacterization corpusである旨、および2つの用途
（①人間による根本原因特定、②TH1e以降の差分ゼロ証明）を明記する。
`docs/journal-replay-guide.md`にも同内容の追記を行う。

**受け入れ基準**: ドキュメントのみ。レビューで内容の正確性を確認する。

**依存**: 163-T1〜T2の実装内容を正しく反映できること。

---

### 163-T7（決定D8）: `DecisionInputs`にドリフト防止の警告を追記する

**内容**: `DecisionInputs`（`state/ime_actuation_decision.rs`、既に`class_name`
除外理由が書かれている箇所）のdoc commentに、ADR-148が
`BugReportGjiKeymapSummary`のdocに残した警告
（「将来『診断のため未対応のキートークンも載せよう』という変更を加えると、
allowlistという唯一の防壁を素通りして任意文字列を送信するチャネルに変質する」）
と同型の注意書きを追加する。

**受け入れ基準**: ドキュメントのみ。

**依存**: なし（いつでも実施可）。

---

### 163-T8（TH1d、既存タスク・維持）: 既知バグ由来fixtureの初回投入

**内容**: `tests/journals/actuation_decision/`に既知バグ
（例: BUG-113系）由来のfixtureを最低1本、手で投入する。
`state/actuation_decision_record.rs::tests::replay_all_actuation_decision_fixtures`の
「ディレクトリが存在しない間は黙って通す」ガードを、fixtureが1本以上存在する
ことを要求する`assert!(total > 0)`相当に強化する。

**受け入れ基準**: `cargo test -p awase-windows --lib`が新fixture込みでgreen。
fixtureディレクトリを空にすると意図的にテストが失敗することを確認する
（ガードが機能している証拠として、一時的に空にして失敗を確認してから元に戻す）。

**依存**: なし（163-T0〜T7と並行して実施可）。
