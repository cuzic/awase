# ADR-081: IME 制御ロジックをプロファイル別 capability 駆動ドライバへ分離し、汎用ループの分岐面を止める

## ステータス

提案中（2026-07-25、Claude Fable 5 との壁打ちから起票、未実装）。

本 ADR は実装計画ではなく、着手前に**同意を取るための設計変針提案**。
Phase 0（下記「第一歩」）を実施してから Go/No-Go を判断する。

## コンテキスト

### 「共通化」を前提にしてきたことへの疑い

`crates/awase-windows` の IME 制御は、`AppImeProfile`（`focus/class_names.rs`:
`Standard` / `Imm32Unavailable` / `TsfNative`、ADR-033）で分類したアプリ群に
対して、
**単一の汎用ループ**（drift correction、cold-start probe、literal detect、
focus settle 等）を適用し、ループ内部で profile ごとに分岐する設計を
一貫して採ってきた。これは意図的な設計判断であり、「アプリ差分は
`AppImePolicy` に閉じ込める、reducer 本体に if-else を増やさない」という
原則が `state/app_ime_policy.rs` の module doc に明記されている。

しかし `docs/known-bugs.md`（43 件）を通観すると、この前提が繰り返し
コストを生んでいる実例が複数ある。

**1. IME OFF キー選択の 5 日間 6 回反転**（`docs/experiments.md` エントリ01）:
`VK_DBE_ALPHANUMERIC` / `VK_KANJI` / `VK_IME_OFF` / F22 経由のどれを送るかが、
TsfNative×GJI・TsfNative×MS-IME・Chrome×GJI のどの組み合わせにも同時に
効く「単一の正解」であるかのように扱われ、ある組み合わせを直すたびに
別の組み合わせで副作用が出て revert された（`534051a` → `098c663` →
`adb856c` → `b271aee` … 前史 `d4d9e27` から数えて6反転）。「アプリ×IMEで
キー選択が変わる。単一の正解キーは無い」がエントリ末尾の学びとして
明記されている。

**2. タイミング定数の家族的インフレ**（`.claude/rules/tuning-constants.md`）:
Chrome 向け probe 待機の最小値が `CHROME_PROBE_MIN_MS`(20ms) →
`CHROME_PROBE_LONG_IDLE_MIN_MS`(100ms→200ms) →
`CHROME_PROBE_F2_GJI_IDLE_MIN_MS`(350ms) と、5週間で4段階に釣り上がった。
根本原因は「Chrome が準備できるまで待つ」という同一目的の定数が、
`long_idle` / `f2_gji_long_idle` / `skip_f2_send` という条件フラグの
組み合わせが増えるたびに `probe_fsm.rs` 内の共有分岐へ追加され続けた
ことにある。プロファイル固有の待機ロジックが、プロファイル非依存の
共有関数のパラメータとして表現されている。

**3. 無関係な機能同士が同じ共有可変状態を取り合った regression**
（BUG-23、ADR-054）: VcXsrv 由来の stuck Ctrl 対策として `PHYSICAL_KEY_STATE`
に injected フィルタを追加した変更が、無関係な `panic_reset()` の
`send_all_modifier_key_ups()`（自己注入）を巻き込んで弾いてしまい、
paniced reset が意図した回復を機能させなくなっていた。単一の共有テーブルを
複数の目的（物理キー追跡・stuck Ctrl 対策・panic 回復）が無調整に読み書き
していたことが原因。

**4. `ImeOpenStrategy` の固定フォールバックチェーン**
（`ime_controller.rs`）: `ImmCrossProcessStrategy` → `GjiDirectStrategy` →
`MsImeDirectStrategy` → `KanjiToggleStrategy` という**全プロファイル共通の
順序**で最初に成功した戦略を採用する設計になっている。プロファイルごとに
「まずこれを試す」という所有権が無く、ある戦略の判定条件を変えると
理論上どのプロファイルにも波及しうる。`ime_controller.rs` の module doc
自身も「`GjiDirectStrategy` は GJI 検出済み時に全プロファイルで適用」
「`KanjiToggleStrategy` への到達は Standard×MS-IME×ImmCross 失敗後の
1組み合わせのみ」と、順序依存の適用条件をプロファイル横断で記述せざるを
得ていない（`docs/adr/034-gji-direct-strategy.md` の TsfNative 除外撤廃も
同種の横断変更の実例）。

### 既存の部分的な一歩は Step 1.5 で止まっている

`state/app_ime_policy.rs` の `AppImePolicy` は、まさにこの方向への
最初の一歩として作られた（module doc: 「Step 1.5 ... reducer 本格化の
ときに polymorphic な参照点として使う」）。だが実態は「profile ごとに
異なる**データ**（`focus_settle_ms` / `default_feedback` / `actuator_kind`）
を1つの struct に持たせ、それを読む側の**ロジックは依然として共有**」
という段階で止まっている。`ImeActuatorKind` の4分岐も、実際に呼ばれる
コードパス（`ir_apply_drift_correction` 等）の内部 if/match として残る。

fable との壁打ちでの指摘: 「フレームワークは既にデータの中でフォークして
いる（プロファイル別定数・アプリ別キー選択）。コードだけ共有してデータが
分岐している現状は、統一と分離の悪いとこ取り」。上記4件はいずれも
「共有ロジック＋プロファイル分岐」という設計そのものが波及元になっている
ケースであり、`AppImePolicy` のデータ分離だけでは防げなかった。

## 決定

IME 制御を、**プロファイルごとに独立した「capability を宣言する
ドライバ」**へ分離する方向へ舵を切る。共有するのは「ドライバが満たす
べき契約（trait/型）」と「journal に残すイベント語彙」のみとし、
制御フローの共有を最小化する。

### 設計の骨子

1. **`ImeProfileDriver` trait を新設**し、`AppImeProfile` ごとに独立した
   実装（`ImmCrossDriver` / `Imm32UnavailableDriver` / `TsfNativeDriver`）
   を持つ。各ドライバが自分の cold-start probe 予算・drift correction
   の feedback 方針・IME OFF キー選択・focus settle 時間を**所有**する
   （現在のように共有関数へパラメータとして渡すのではなく、ドライバの
   実装内に埋め込む）。
2. **ドライバは capability を型で宣言する**: `CanReadOpenStatus`,
   `CanObserveCompose`, `PhysicalKanjiOwnership` 等。コアループ
   （`runtime/ime_refresh.rs` 等）はドライバの capability を見て
   分岐するのではなく、`ImeProfileDriver` trait のメソッド呼び出しだけで
   完結させる（if/match で `ImeActuatorKind` を分岐する現状コードを
   ドライバ内部に押し込む）。
3. **「ある変更がどのドライバに閉じるか」をコミット規約で強制する**:
   1つのドライバ実装ファイルのみを変更するコミットと、trait 定義や
   複数ドライバをまたぐコミットを区別し、後者には
   `.claude/rules/tuning-constants.md` 相当の「なぜ複数プロファイルに
   またがる必要があるか」の説明義務を課す（新規 rule ファイルとして
   `.claude/rules/` に追加する）。
4. **フォールバックチェーンではなくドライバの自己完結を優先する**:
   `ImeOpenStrategy` の4戦略順次試行は、各ドライバが「自分のプロファイル
   で有効な手段」を静的に1つ（必要なら明示的な自ドライバ内フォールバック
   として2つ）持つ形に置き換える。`GjiDirectStrategy` のような
   「全プロファイル共通で使う」戦略は、共通コードとして残してよいが、
   どのドライバがどの条件でそれを呼ぶかをドライバ側が明示的に選択する
   （暗黙の優先順位リストに依存しない）。

### 適用しない範囲（意図的なスコープ限定）

- `journal.rs` / belief の3層分離（`ime-belief-architecture.md`）は
  そのまま維持する。本 ADR はロジックの**所在**を変えるものであり、
  belief 更新の規律（Observe → Pure → Apply）を変えない。
- 完全な「コード共有ゼロ」は狙わない。fable の指摘通り「間違った抽象より
  重複の方が安い」可能性を検証するのが Phase 0 の目的であり、最初から
  全面分離を決め打ちしない。

## 第一歩（Phase 0、検証専用・コード変更を伴わない）

1. `docs/known-bugs.md` の43件を実際に読み、「ある profile 向けの修正が
   別 profile に影響した」件数と「単一 profile に閉じていた」件数を
   数える（本 ADR のコンテキスト節で4件を提示したが、悉皆調査ではない）。
   共有コストが定量的に高ければ Phase 1 へ進む根拠になる。
2. `ime_controller.rs` の4戦略・`app_ime_policy.rs` の3プロファイルを
   対象に、「もし完全に別ファイル・別 struct に分けたら重複する行数は
   何行か」を実際にコードを書かず机上で見積もる（構造体定義・
   trait 実装のシグネチャレベルで十分）。
3. 上記2点をもとに Go/No-Go を判断し、Go なら最小プロファイル1本
   （`ImmCross` — 依存が少なく、LINE/Qt 向けの独自原則
   ([[feedback_immcross_owns_kanji]]) が既にあるため境界が引きやすい）
   を試験的にドライバ化して実測する。

## 不変条件（Phase 1 着手時にテスト/型で強制する候補）

- コアループ（`runtime/ime_refresh.rs` 等）のソースに `ImeActuatorKind::`
  や `AppImeProfile::` へのパターンマッチが出現しないことを
  `architecture_guard.rs` 相当のテキスト走査で固定する（trait 越しの
  呼び出しのみを許可）。
- 1コミットが `driver/imm_cross.rs` と `driver/tsf_native.rs` の両方を
  変更する場合、コミット本文に「なぜ複数ドライバにまたがるか」の説明が
  あることをレビューで確認する（自動チェックは困難なため、
  `.claude/rules/` の pre-push 警告レベルで運用する）。

## 関連

- [[project_integration_unmerged_branches_2026_07_25]]（統合作業の文脈）
- ADR-033（AppImeProfile 分類の元 ADR）
- ADR-080（`AppImePolicy.default_feedback` を導入した直近の型強制、
  本 ADR はこれをドライバ全体に拡張する方向）
- `.claude/rules/experiment-logging.md`（IME OFF キー6反転の記録規約）
- `.claude/rules/tuning-constants.md`（タイミング定数エスカレーション規約）
