---
id: ADR-163-companion-163-implementation-tasks
title: |-
  ADR-163 Part D（TH1d'）実装タスク一覧
type: companion-doc
related_adr:
  - "ADR-119"
  - "ADR-121"
  - "ADR-148"
  - "ADR-149"
  - "ADR-163"
  - "ADR-164"
---

# ADR-163 Part D（TH1d'）実装タスク一覧

[ADR-163](163-actuation-decision-io-separation-and-replay-harness.md)「Part D」節の決定D1〜D8を、
実装可能な単位に分割したタスクリスト。`docs/adr/158-implementation-tasks.md`と同じ形式
（内容・受け入れ基準・依存）を踏襲する。各タスクは個別のコミットにすること。

対象領域は`.claude/rules/fix-requires-evidence.md`の「IME actuation 合流点」ファミリーに
該当するため、各コミットは回帰テストを伴うこと。

**改訂履歴**: 初版（T0→T1→T2→T5→T6→T4→T3→T7、T8独立）をopus-adversarial-consultで
レビューした結果、Blocker6件（B1〜B6）・Should-fix10件（S1〜S10）が見つかり「このまま
Codexへ委任するのは不可」と判定された。以下は全指摘を反映した改訂版。

## 発覚した設計ミス（改訂の理由、必ず読むこと）

- **B1**: `DecisionSite`の新バリアントを`decide_attempt`まで伝搬させると、
  Standard×MS-IME（chain先頭がImmCross）のreassert/force-ON経路で`decide_attempt`が
  `None`を返し、`ime_controller.rs:352`の`debug_assert_eq!(mechanism, WriteMechanism::GjiDirect)`
  がdebugビルドでpanicし、releaseではImmCross writeが無音でスキップされる
  （ADR-121のBUG-37冪等再送・ADR-149/151/153系のforce-ON救済が壊れる）。
  → **解決方針: `decide_attempt`へ渡す`site`引数は常に`DecisionSite::Sync`のまま変えない。
  新バリアントは`ActuationDecisionRecord`に記録する側のラベルとしてのみ使い、
  呼び出し元（`runtime/mod.rs`側）が`apply()`の戻り値を受け取った後に記録のsite
  フィールドだけを上書きする。command計算には一切関与させない。**
- **B2**: `with_app`経由で記録しようとすると、Sync siteと`fallback_write`は
  呼び出し時点で既に`with_app`の内側（再入）にいるため記録が100%失敗し、コーパスが
  ゼロ本になる。
  → **解決方針: site群ごとに到達手段を分ける（下記「配線」タスク群a/b/c）。**
- **B3**: `journal.rs`は`#[cfg(windows)]`ゲートされているため、`cargo test -p
  awase-windows --lib`（Linux実行）では journal 側のテストは一覧にすら出ない。
  各タスクの受け入れ基準で「Linuxで実行可能」と「Windowsターゲットのコンパイル確認+
  windows-build CIでの実行」を必ず書き分ける。
- **B4**: `app_version`を`ActuationDecisionRecord`ごとの`String`にすると、T2で排除した
  はずのヒープ確保が復活する。バージョンはdumpのenvelope/headerに1回だけ持たせる。
- **B5**: T8（fixture投入）をスキーマ確定前に行うと、T0/T2/T4のスキーマ変更のたびに
  fixtureのJSONが`from_str`でpanicする。スキーマ確定後に回す。
- **B6**: T0時点では本番コードに`ActuationDecisionRecord`を組み立てる箇所が1つも
  存在しない（現状は`state/actuation_decision_record.rs`のfixture専用型）。
  「`fallback_write`に記録呼び出しを足す」はT0では書けない——配線タスクへ移す。

## 実装順序

**フェーズ1（スキーマ確定、`state/`の2ファイルに閉じる、いずれもLinuxで検証可能）**:
163-T0 → 163-T2 → 163-T5(record専用) → 163-T4(schema部分) → 163-T-schema-roundtrip

**フェーズ2（配線、journal相乗り、Windows-gated）**:
163-T1a（可視性+`JournalEntry` variant追加、本番記録なし）→
163-T1b（Sync site配線）→ 163-T1c（open_chainの2 site配線）→
163-T1d（`dispatch_ime_set_open`配線）

**フェーズ3**: 163-T6（`with_app`再入カウンタ）

**フェーズ4（文書、スキーマ変更コミット自身が持たない残りのみ）**: 163-T3 → 163-T7

**フェーズ5**: 163-T8（fixture投入、スキーマ確定後）

各フェーズ内のタスクは前のタスクに依存する。T7のみ常に独立。

---

## フェーズ1: スキーマ確定

### 163-T0（決定D7、前提条件）: `AttemptRecord`にBUG-113上書き前の値を追加する（スキーマのみ）

**内容**: `AttemptRecord`（`state/actuation_decision_record.rs`）に
`shadow_on_before_bug113_override: Option<Option<bool>>`
（`post_failed_reobservation`と同じ「未取得」と「取得してfalse」を区別するパターン）を
追加する。**本番コードへの記録呼び出しの配線はここでは行わない**（B6——記録先がまだ
存在しない）。配線は163-T1cで行う。

- 手組みテストfixture（`replay_accepts_a_hand_built_sync_gji_direct_record`等）を
  新フィールド込みで更新する。
- 改ざん検出テストを1つ追加する: 上書き前の値だけを変えたレコードを再生させ、
  `replay_detects_a_tampered_command`と同じ手法で不一致を検出できることを示す。
- ADR本文のスキーマ掲載コード（`docs/adr/163-actuation-decision-io-separation-and-replay-harness.md`
  「Part B」節の`pub(crate) struct AttemptRecord { ... }`）を同じコミットで更新する（S9）。

**受け入れ基準（Linuxで実行可能）**: `cargo test -p awase-windows --lib`の
`state::actuation_decision_record::tests`配下が全green。

**依存**: なし。

---

### 163-T2（決定D2）: `chain`/`attempts`をVecから固定長配列にする

**内容**: `WriteMechanism::ALL`は`[Self; 4]`のため:

- `chain: Vec<WriteMechanism>` → `[Option<WriteMechanism>; 4]` + 使用数。
- `attempts: Vec<AttemptRecord>` → `[Option<AttemptRecord>; 4]` + 使用数。
- 再生ハーネス・既存fixtureを更新する。
- ADR本文のスキーマ掲載コードを同じコミットで更新する（S9）。

**受け入れ基準（機械的、目視確認は不可）**:
- `const _: () = assert!(std::mem::size_of::<ActuationDecisionRecord>() <= N);`
  （Nは変更前後の実測値をコミット本文に残してから決める、
  [tuning-constants](../../.claude/rules/tuning-constants.md)の精神）。
- `AttemptRecord`が`Copy`であることのコンパイル時境界チェックを追加する
  （既に`Copy`をderive済みなので確認のみ）。`ActuationDecisionRecord`自体を
  `Copy`にできるかも確認し、できなければ理由をコミット本文に残す。
- `cargo clippy --target x86_64-pc-windows-msvc -p awase-windows`がclean。
- **注意（S4）**: `size_of::<ActuationDecisionRecord>()`と`size_of::<JournalEntry>()`の
  変更前後の値を両方コミット本文に残す。`JournalEntry`の最大variantサイズが
  跳ね上がると、4 lane×512枠の事前確保量（`journal.rs`の`VecDeque::with_capacity`）が
  全体で増えるため、`Box`化で誤魔化さない（D2に反する）。

**依存**: 163-T0。

---

### 163-T5（決定D5、record専用ラベル）: `DecisionSite`に2バリアントを追加する（command計算には関与させない）

**内容**: `DecisionSite`に`ReassertExplicitPhysicalKey`/`ForceOnRomajiCorrection`を
追加する。**`ime_controller.rs:239`の`decide_attempt`呼び出しは、この2バリアントを
一切渡さない——引数は現状どおり常に`DecisionSite::Sync`のまま**（B1の解決）。
2バリアントは`ActuationDecisionRecord.site`（記録用フィールド）にのみ現れる。

- `runtime/mod.rs::reassert_explicit_physical_key`/`force_on_and_correct_romaji`
  （163-T1bで配線）が、`apply()`から受け取った`ActuationDecisionRecord`の`site`
  フィールドを、記録する直前に自分の呼び出し元であることを示すこの2バリアントへ
  差し替える（command自体は既に`Sync`前提で計算済みなので変更しない）。
- ADR本文のスキーマ掲載コードを同じコミットで更新する（S9）。

**受け入れ基準（挙動不変であることを確認、S6）**:
- `state/ime_actuation_decision.rs`の`(mechanism, site)`全組合せ固定テスト2本
  （ImmCross×Sync→`Some(SetOpenCrossProcessSync)`、ImmCross×非Sync→`None`）が、
  新バリアント追加後も**変更なしで**greenであることを確認する（＝
  `decide_attempt`のmatch自体は触っていないことの証拠）。
- `ime_controller.rs:343`の`unreachable!()`・`:352`の
  `debug_assert_eq!(mechanism, WriteMechanism::GjiDirect)`が前提とする
  「`apply_mechanism`は常に`DecisionSite::Sync`で呼ばれる」が変更後も真であることを
  コミット本文に明記する。
- `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`が通る。
  実際の記録・挙動確認は163-T1bで行う（windows-build CI、B3）。

**依存**: 163-T2。

---

### 163-T4（決定D4、スキーマ部分）: バージョン情報はdump headerへ、per-recordの`String`は不採用

**内容**: `ActuationDecisionRecord`に`app_version: String`は追加しない（B4——TF2再開条件に
反する）。dump時の`truncation_header_json`相当の場所（`journal.rs`のdumpヘッダ生成箇所）に、
dump全体で1回だけバージョン情報を持たせる。

- `site ∈ {ImmCrossWrite, RunOpenChainAsync, DispatchImeSetOpen}`のImmCross attemptは
  TH1e完了まで自動差分証明の対象外であることを、`state/actuation_decision_record.rs`の
  モジュールdocに明記する（この部分はT3と重複しないよう、スキーマに関する記述はここで
  行い、運用に関する記述はT3で行う）。

**受け入れ基準**: dumpヘッダにバージョンフィールドが1つ追加され、
`ActuationDecisionRecord`自体のサイズが変わらないこと（163-T2のsize_ofチェックが
引き続きパスすることで確認する）。

**依存**: 163-T2。

---

### 163-T-schema-roundtrip（S10、新設）: JSON往復一致テスト

**内容**: `state/actuation_decision_record.rs`の`#[cfg(test)] mod tests`に、
手組みの`ActuationDecisionRecord`（163-T0/T2/T4後の最終スキーマ）を`serde_json`で
シリアライズ→デシリアライズして元の値と一致することを確認するテストを1本追加する
（`event_origin_record_round_trips_via_json`と同型）。`&'static str`を含む
フィールドを将来足した瞬間にコンパイルは通るが転記だけ静かに壊れる、という
`EventSourceKind`が既に回避した罠の再発を防ぐ。

**受け入れ基準（Linuxで実行可能）**: `cargo test -p awase-windows --lib`でgreen。

**依存**: 163-T0, 163-T2, 163-T4（最終スキーマ確定後）。

---

## フェーズ2: 配線（journal相乗り、B2の解決）

### 163-T1a: 可視性変更 + `JournalEntry` variant追加（本番記録なし）

**内容**:

1. `state/mod.rs:98`の`#[cfg(test)] pub(crate) mod actuation_decision_record;`を
   `pub mod actuation_decision_record;`に変更し、モジュール内の型
   （`ActuationDecisionRecord`/`AttemptRecord`/`ActuationOrderRecord`/
   `EventOriginRecord`/`EventSourceKind`）を`pub`にする。
2. `state/mod.rs:93-94`の`ime_actuation_decision`モジュールと、
   `DecisionInputs`/`DecisionSite`/`MechanismCommand`も`pub`にする。
3. 再生ハーネス本体（`#[cfg(test)] mod tests`）はcfg(test)のまま変更しない。
4. `journal.rs::JournalEntry`に`ActuationDecision { record: ActuationDecisionRecord }`
   を追加する。
5. **lane配置は既存`LaneKind::Actuation`への相乗りに決め打ちする**（S1——専用lane新設は
   別タスクとし、ここでは選択肢として残さない）。`lane_kind()`に1アームを追加するのみ。
6. `emit_tracing`にアームを追加する。`tests/architecture_guard.rs`の
   `journal_emit_tracing_has_no_debug_display_sigils_or_wildcards`が`?`/`%`/
   `_ =>`/`.. =>`を禁止しているため、`DecisionSite`/`WriteMechanism`/
   `MechanismCommand`/`AppImeProfile`/`ImeKindId`/`InputModeState`/`ImeOpenOutcome`
   それぞれについて、既存の`feedback_policy_kind_str`と同型の`*_str`ヘルパーを
   新規に書く（S3）。attempts（最大4件）をtracingフィールドへどう平坦化するか
   （件数+先頭1件のみ、等）もここで決める。
7. **本番からの記録呼び出しはまだ追加しない**（配線は163-T1b〜T1dで行う）。

**受け入れ基準**:
- `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib`
  が通る（Windowsターゲットのコンパイル確認のみ、B3）。
- `tests/architecture_guard.rs`の関連ガードを確認し、想定外の増加が無ければそのまま、
  変わっていれば期待値を更新する。
- **バイト予算への影響（S2）**: `journal_policy.rs::RESERVED_PERCENT`は変更しない
  （相乗りのため）。ただし実際の記録開始後（163-T1b〜d完了後）に、同一操作列で
  dumpを取り、`truncation_header_json`が出すlane別emitted/dropped件数を
  変更前後で比較する回帰テストを163-T1dの受け入れ基準として置く（他lane
  ——特にBUG-02型のTiming lane——を圧迫していないことの確認）。

**依存**: フェーズ1完了。

---

### 163-T1b: Sync site配線（`ImeController::apply`/`runtime/mod.rs`）

**内容**: `ImeController::apply`（`ime_controller.rs:538`）・`apply_mechanism`
（`:239`の`decide_attempt`呼び出し）の結果から`ActuationDecisionRecord`を組み立て、
**戻り値として呼び出し元へ返す**（B2の解決——`apply_ime_open_with_view`は`&self`で
journalへ到達できないため、記録は呼び出し元に委ねる）。

- `runtime/mod.rs::reassert_explicit_physical_key`（`:1028`付近）・
  `force_on_and_correct_romaji`（`:1195`付近）・`runtime/executor.rs`（`:990`付近の
  通常Sync呼び出し）が、受け取ったrecordを`self.platform_state.ime.journal.record(...)`
  で記録する（前例: `runtime/ime_refresh.rs:675,940`、`runtime/focus_tracking.rs:550`）。
- `reassert_explicit_physical_key`/`force_on_and_correct_romaji`は、記録直前に
  163-T5で追加した`ReassertExplicitPhysicalKey`/`ForceOnRomajiCorrection`へ
  `record.site`を差し替える（command自体には触れない）。
- `apply_ime_open_with_view`を`&mut self`化する必要は無いことを確認する
  （戻り値経由のため。`&mut self`化すると`runtime/mod.rs:1019`の`view`保持中に
  `&mut self.platform`を取ることになり借用エラーになる——この案は選ばない）。

**受け入れ基準**:
- `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`が通る。
- windows-build CIで、reassert/force-on経路の既存回帰テスト（ADR-121/149/151/153系）が
  引き続きgreenであることを確認する（B1で壊れかけた経路そのもの）。
- `tests/ime_key_sequence_golden.rs`/`tests/golden/ime_key_sequences.txt`の期待値が
  不変であることをwindows-build CIで確認する。

**依存**: 163-T1a。

---

### 163-T1c: open_chainの2 site配線（`fallback_write`/`run_open_chain_async`/`imm_cross_write`）

**内容**: `open_chain.rs`の`run_open_chain_async`（`:154-161`）・`fallback_write`
（`:300`付近）・`imm_cross_write`（`:379-386`）は、自身が`with_app`クロージャの
内側にいる（B2該当なし）。そのクロージャ内で`app.platform_state.ime.journal.record(...)`
まで完結させる。

- `fallback_write`が`view.control.shadow_on = None;`で上書きする直前の値を、
  163-T0で追加した`shadow_on_before_bug113_override`に実際に埋める（ここで初めて
  スキーマに値が入る）。
- fail-open（`with_app(...).unwrap_or(false)`が`None`側に落ちるケース）では記録できない
  ことを確認し、163-T6のカウンタ対象として明記する。

**受け入れ基準**: windows-build CIで、`fallback_write`実行時にjournalへ
`ActuationDecision`エントリが実際に積まれることを確認する回帰テストを追加する
（`journal.rs`の既存単体テストパターンに倣う）。`shadow_on_before_bug113_override`が
正しい値（上書き前の値）を持つことをアサートする。

**依存**: 163-T1a。

---

### 163-T1d: `dispatch_ime_set_open`配線（独立5番目の入口）

**内容**: `runtime/executor.rs::dispatch_ime_set_open`の`DecisionSite::DispatchImeSetOpen`
経路を記録する。ADR-119が「独立した5つ目の入口」と位置づけた早期gateであり他2群と
条件が異なるため独立コミットにする。

**受け入れ基準**: windows-build CIでの回帰テスト追加。加えて163-T1aで保留した
バイト予算の実測（S2、lane別emitted/dropped件数の変更前後比較）をここで完了させる。

**依存**: 163-T1a。

---

## フェーズ3

### 163-T6（決定D6）: `with_app`再入時の記録漏れをカウンタで可視化する

**内容**: `run_open_chain_async`/`imm_cross_write`のfail-open再入時は記録できない
（構造的な限界、解決はしない）。「再入により記録をスキップした回数」を数える
カウンタを追加する。

- **置き場所はADR-164（裸のグローバルstatic集約）の方針に従う**（S5——新規の
  裸`static AtomicU64`を安易に追加しない）。`with_app`が取れる文脈
  （`PlatformState`内）に置ければ裸staticは不要。記録漏れが起きるのは
  `with_app`が`None`のとき（＝`PlatformState`に到達できないとき）なので、
  `PlatformState`の外に置かざるを得ない可能性がある——その場合は
  `probe_actuation_fence.rs`が使うロックフリーパターン（ADR-164が是とする
  「正当な可変シングルトン」の実例）に倣う。実装前にADR-164の該当箇所を再読し、
  新規static追加がADR-164のレビュー対象に入らないか確認する。
- 出力先は`tracing::debug!`のみ（awase.logは既にbug reportへ`app_log_excerpt`として
  添付済み）。**bug reportの新規カウンタ系フィールドには相乗りしない**（S5——
  `attach_retro_eval_stats`と同じ4層ミラー構成を要求することになり、決定D1が
  避けたB2/B3を連れ戻すため、この選択肢は採らない）。

**受け入れ基準**: `with_app`が`None`を返すケースを模したテストでカウンタが
増加することを確認する（`runtime/`配下は`#[cfg(windows)]`のため windows-build CI）。

**依存**: 163-T1c。

---

## フェーズ4: 残りの文書化

### 163-T3（決定D3）: characterization corpusであることの運用文書化

**内容**: `docs/journal-replay-guide.md`（相当のガイド）に、Part Dで集まるレコードが
correctness corpusではなくcharacterization corpusである旨、2つの用途
（①人間による根本原因特定、②TH1e以降の差分ゼロ証明）を明記する。スキーマ自体の
ADR本文同期は各スキーマ変更コミット（T0/T2/T4/T5）が既に行っているため、ここでは
運用面のみ扱う。

**受け入れ基準**: ドキュメントのみ。

**依存**: フェーズ1〜3完了（実装内容を正確に反映できること）。

---

### 163-T7（決定D8）: `DecisionInputs`にドリフト防止の警告を追記する

**内容**: `DecisionInputs`のdoc commentに、ADR-148の`BugReportGjiKeymapSummary`と
同型の警告（allowlist原則を素通りする変更への注意）を追加する。

**受け入れ基準**: ドキュメントのみ。

**依存**: なし（いつでも実施可）。

---

## フェーズ5

### 163-T8（TH1d）: 既知バグ由来fixtureの初回投入 — **完了（2026-09-12）**

**内容**: `tests/journals/actuation_decision/`に既知バグ由来のfixtureを最低1本、
**実機ダンプから**手で投入する。

**実施結果**: `bug-report-latest`スキルで直近10件の実機bug reportを取得し、
`ActuationDecision`エントリを含む唯一の報告（`01M29KDNZ22KNY1FPXSKBGMW7V`、
BUG-131/ADR-166の原因調査対象そのもの）から37レコードを抽出。抽出元は
N-1（ワイヤ圧縮）適用前のビルドが記録した旧形式（`chain`/`attempts`が
`null`パディング固定長配列、`shadow_on_before_bug113_override`等が
`{"recorded":bool,"value":..}`展開形）だったため、現行の
`ActuationDecisionRecordWire`が読める形へ変換（`null`除去・3値圧縮表現化）
してから`tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`
として投入した。抽出・変換手順は`docs/journal-replay-guide.md`
「ActuationDecisionコーパスの扱い」節に追記（S10）。
`replay_all_actuation_decision_fixtures`の「ディレクトリ不在／フィクスチャ0件は
黙って通す」ガードを`assert!`（ディレクトリ不在拒否・`paths.is_empty()`拒否・
`total > 0`拒否の3段）に強化した。

**受け入れ基準**: `cargo test -p awase-windows --lib`が新fixture込みで37件の
リプレイ含め green（669 passed）。fixtureディレクトリを一時的に空にしてテストが
意図どおり失敗することを確認済み。`cargo check --target x86_64-pc-windows-msvc
-p awase-windows`確認済み。`cargo fmt`適用済み。

**依存**: フェーズ1（最終スキーマ確定）+ フェーズ2（実機ダンプが取れる状態）
——いずれも充足済みだった。

---

## PR #201レビュー結果（opus-adversarial-consult round2、2026-09-11）での未達事項

以下はPR #201のマージ時点で受け入れ基準が未達のまま残った項目。実機/windows-build
CIが必要でこのセッション（Linuxサンドボックス）では実施できなかったもの。

- **163-T1c**: `fallback_write`実行時にjournalへ`ActuationDecision`エントリが
  実際に積まれることを確認する回帰テスト、`shadow_on_before_bug113_override`が
  正しい値を持つことのアサートは未実施（windows-build CI必須）。
- **163-T1d**: 同上の回帰テスト、およびlane別emitted/dropped件数の
  変更前後比較（実機/windows-build CI必須。ただしJSONバイト数の実測は
  `actuation_decision_record_json_byte_size_is_measured`テストでLinux上で
  完了済み——既存`ImeActuation`と合わせ同一laneを消費する点に注意）。
- **163-T6**: `with_app`が`None`を返す状況を実際に模したテストは未実施
  （`runtime/`配下は`#[cfg(windows)]`のためLinuxのテストバイナリに存在しない）。
- **S-8（対応済み、2026-09-11）**: `DecisionSite::RunOpenChainAsync`が
  `key_pipeline.rs`のshadow-toggle OFF経路・`runtime/mod.rs`のforce-on
  bootstrap経路の2つの異なる呼び出し元に共有されており、診断粒度としては
  区別できなかった（`executor.rs`の`dispatch_ime_set_open`経路は元々
  `site=DispatchImeSetOpen`で区別済み）。`run_open_chain_async`に
  `caller: Option<DecisionSite>`引数を追加し、`ActuationDecisionRecord::caller`
  （B-2で新設、`ReassertExplicitPhysicalKey`/`ForceOnRomajiCorrection`と同じ
  事後ラベル付けパターン）へそのまま転記する形で解決した。新設した
  `DecisionSite::ShadowToggleOff`/`ForceOnBootstrap`はcommand再計算には
  使わない記録専用ラベル（`state/ime_actuation_decision.rs`のdoc参照）。
  `journal.rs`のActuationDecisionトレースログにも`caller`を追加した。
- **N-1（対応済み、2026-09-11）**: `ActuationDecisionRecord`のJSON表現が
  1エントリ約631バイトあり、Actuation lane（journal.rsの20%予約）で既存
  `ImeActuation`/`DriftGiveUpDiagnostic`/`ConvClassifyCall`を押し出すペースを
  悪化させる懸念があった。`chain`/`attempts`を`null`パディング済み固定長
  配列のまま出さず埋まっている分だけの可変長配列として直列化し
  `chain_len`/`attempts_len`フィールドを廃止（`ActuationDecisionRecordWire`
  経由の手書き`Serialize`/`Deserialize`、メモリ上の固定長`Copy`表現はホット
  パスのヒープ確保回避のためそのまま維持）、`nested_optional_bool`の
  `{"recorded":bool,"value":Option<bool>}`オブジェクト展開を`null`／
  `"unknown"`／素の`bool`へ圧縮し、523バイトへ縮小した
  （`actuation_decision_record_json_byte_size_is_measured`参照）。
