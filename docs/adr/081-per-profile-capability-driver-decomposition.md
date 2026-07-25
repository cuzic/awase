# ADR-081: IME 制御ロジックをプロファイル別 capability 駆動ドライバへ分離し、汎用ループの分岐面を止める

## ステータス

提案中 → Phase 1 計画確定（2026-07-25、Claude Fable 5 との壁打ちから起票。
Phase 0 は PR [#31](https://github.com/cuzic/awase/pull/31) で Limited Go、
Phase 1 計画は Opus による立案 + Fable との壁打ちで未確定点を解消し確定。
Phase 0.5〜1c 実装は未着手）。

Phase 0（第一歩）は実施済み（PR #31）。以降は「Phase 1 計画（確定）」節を参照。

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

## Phase 1 計画（確定、2026-07-25）

Phase 0 の実施内容・定量調査（known-bugs.md 43 件の分類、`ImmCrossDriver` 試験実装）
は PR [#31](https://github.com/cuzic/awase/pull/31)（本ファイルの Phase 0 実施記録節、
未マージ）にある。本節は、Opus モデルによる Phase 1 実装計画の立案と、
Claude Fable 5 との壁打ちによる未確定点の解消を経て確定した全面移行計画。

### 置換対象（現行の共有コード）

- `state/app_ime_policy.rs::AppImePolicy::from_profile` の4データ分岐
- `ime_controller.rs` の `ImeOpenStrategy` 4戦略・共有フォールバックチェーン
- `runtime/ime_refresh.rs::ir_apply_drift_correction` の Blind/Read 分岐 +
  `can_use_imm32_cross_process()` 分岐
- `tuning.rs` の `CHROME_PROBE_*` 系定数 + `probe_fsm.rs` の条件分岐

### GJI 横断性の設計（要判断事項として保留していたが確定）

`active_ime_kind`（GJI/MS-IME）は `AppImeProfile` に対して静的でなく実行時観測であり、
「ドライバが capability を静的に宣言する」という本 ADR の中核モデルと素朴には衝突する。
2案を検討した:

- (A) `ime_open_mechanism(open, active_ime_kind)` のようにメソッドへ観測値を渡し、
  ドライバ内部に動的分岐を持ち込む。
- (B) GJI 直接制御（現行 `GjiDirectStrategy` 相当）を**全プロファイル横断で適用される
  共有機構**として1箇所に残し、各ドライバは `uses_gji_direct() -> bool` を
  **静的に宣言するだけ**にする。実行時の合成（driver + GJI機構）はランタイム側が行う。

**(B) を採用する。** 理由: profile 軸（アプリの IME 受容能力、静的）と IME 軸
（GJI/MS-IME、動的）は本質的に直交する2次元。(A) はこれを各ドライバのメソッド内
分岐として1次元に押し込む案であり、結果として3ドライバがそれぞれ GJI 分岐を
再実装することになる——これは Phase 0 で見つかった反証データ6件
（BUG-21,29,30,31,32,35、「既に部分分離されていた実装が同期を怠ってバグを生んだ」）
と同じ失敗モードの再生産である。(B) なら動的軸の実装は1箇所に留まり、ドライバは
固定値宣言のみを保てる。

**適用条件**（Phase 1c の不変条件5に反映）:
1. 共有 GJI 機構のインタフェースは狭く保つ（放置すると第二の「共有ループの分岐面」
   になり、本 ADR が解体しようとしている対象そのものが復活する）。
2. `uses_gji_direct()` を宣言しないドライバから GJI 機構を呼べないことを
   contract test で縛る。
3. GJI 機構の状態遷移は、どのドライバ経由で呼ばれても同一の `GjiFsm` 同期を
   通ること（BUG-18/22 型——belief を actuate 抜きで ON にする高速パスが `GjiFsm`
   同期を踏み抜く——の再発防止の核心。反証データ6件のうち最も実害が大きい型）。

### 65% を占める「プロファイル分岐と無関係な汎用インフラ欠陥」の扱い

known-bugs.md 43件中28件（65%）はドライバ分離で解決しない
（スレッド配送・タイマー順序・UIA キャッシュ粒度・hook 状態等）。**Phase 1 の
スコープから明示的に除外する。** ただし成功指標を曖昧にしないため、Phase 1 完了判定は
以下で測ることをここに明記する: **cross-profile 波及11件 + 同期漏れ型6件
（計 ~40%、上記2カテゴリの合算）の再発率**。インフラ側28件は非目標であり、
「081をやったのにバグ総数が減らない」という誤った失敗判定を防ぐための注記。
インフラ側の一部（スレッド配送・タイマー順序の可視化）は ADR-082 の
journal/`EventOrigin` が効く領域であり、放置ではなく「082系の将来課題」と位置づける。

### `Standard`/`Plain`/`Unknown` の `ImmCrossDriver` への集約

3値を1ドライバに集約する現行 Phase 0 実装を妥当と判断する。ドライバ数を増やすこと
自体が「同期すべき箇所」を増やす——反証データの失敗モードを増やす方向に働く。
挙動が同じ間は1ドライバが正しく、将来分岐が必要になれば trait 境界が既にあるため
分割コストは低い。**ただし分類そのもの（`ImePolicyProfile` の enum 値）は統合しない**
（driver へのマッピングだけを collapse する）。分類情報が ADR-082 の journal に
残っていれば、将来「`Unknown` だけ挙動が違う」というバグが出たときにデータ駆動で
分割判断ができる。

### フェーズ分割

- **Phase 0.5（ADR-082 側、先行実施）**: `JournalEntry::ImeActuation { origin }`
  を新設し、`Actuation` に `EventOrigin` を配線する。完了条件: BUG-43 リプレイ
  （`tests/drift_correction_replay.rs`）が新 variant 経由で green（Linux）。
  ADR-082 を先行させる理由: 081 が `ir_apply_drift_correction` を書き換える前に
  journal リプレイ回帰網を張っておける。081 のドライバ `actuate()` は最初から
  `EventSource::SelfActuated` を journal に積む形で実装し、二度手間を避ける。
- **Phase 1a**: `Imm32UnavailableDriver` を実装（`AppImePolicy` との parity テスト
  付き、未配線）。上記「共有GJI機構」設計を前提に、GJI 固有ロジックを
  ドライバ内に複製しない。
- **Phase 1b**: `TsfNativeDriver` を同様に実装。
- **Phase 1c**: ドライバレジストリ（`profile -> &'static dyn ImeProfileDriver`）+
  contract test スイート（不変条件5件、下記）を実装。完了条件: contract test
  green（Linux）+ cross-compile 通過（`cargo check --target x86_64-pc-windows-gnu`）。
- **Phase 1d（実機ソーク必須、この Linux サンドボックスでは実行不可）**:
  `ime_refresh`/`ime_controller` の `AppImePolicy` 参照を **1プロファイルずつ**
  ドライバ呼び出しへ置換する（strangler fig）。並走方式:
  - 並走中に actuate するのは**常に片方だけ**。もう片方は計算と parity 比較のみの
    read-only shadow とし、両経路が同時に実状態へ書き込む期間をゼロにする
    （反証データの失敗モード「両経路が実状態を持つことによる同期漏れ」を
    構造的に排除する）。
  - 1プロファイルのソーク合格 → 即座に旧経路を撤去 → 次のプロファイルへ、と
    1d の各ステップに撤去を畳み込む（一括撤去を最後にまとめない）。
  - 並走期間の上限: ソーク行列合格 + 実使用3〜5日のハード基準。
  - ソーク行列: Chrome×GJI / LINE×MS-IME / Edge×GJI / WezTerm(TsfNative) / Teams
    （cross-profile 11件が実際に指す組み合わせ）。各切替後に cold-start・
    focus 往復・drift correction を目視確認する。
- **Phase 1e**: 全プロファイルのソーク通過後、`AppImePolicy`・戦略チェーン・
  `CHROME_PROBE_*` 分岐・parity ガードを削除。完了条件: 旧経路参照ゼロを
  `architecture_guard.rs` のテキスト走査で固定する。

### 不変条件（Phase 1c で実装する contract test、5件）

1. IME-ON 経路を持つドライバは stale `ObservedEisu` 救済を対で持つ
   （`.claude/rules/ime-belief-architecture.md` の既存対称性テストの一般化）。
2. `owns_physical_kanji()==true` のドライバは物理 KANJI を漏らさない。
3. `Blind` give-up 後に observation を書かない（BUG-33 型の観測偽装防止）。
4. belief を actuate 抜きで ON にする高速パスは、必ず `GjiFsm` を同期させる
   （BUG-18/22 型の核心、反証データのうち最も実害が大きい失敗モード）。
5. **GJI 機構の状態遷移は、どのドライバ経由で呼ばれても同一の `GjiFsm` 同期を
   通る**（「GJI 横断性の設計」節、共有 GJI 機構の適用条件3を実装で保証する）。

### 実機検証について

Phase 1d・1e は Windows 実機での複数アプリ×複数 IME のソークが必須であり、
このサンドボックス（wine 未導入）では実行できない。Phase 0.5〜1c は
`crates/awase-windows/src/state/` 配下の platform-independent パターン
（ADR-065）に従って実装すれば Linux 上で `cargo test -p awase-windows --lib`
から検証可能。
