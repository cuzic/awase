---
id: ADR-210
title: |-
  学習表の状態に「変換中」フラグと「直前キーの文脈」を加え、隠れ状態で結果が割れるセルを決定的にする(草稿)
summary: |-
  学習(ADR-195/196)の表は (open, mode, composing, key) で引くが、同じ状態・同じキーで結果が割れるセルが GJI MS-IME プリセットで押下の約19%、ATOK で約8%、MS-IME 本体で約21%ある(診断ログ、run 36690186527/36690186642)。
  観測できる特徴量を押下ごとに全部記録し、割れを説明する特徴量を交差検証(leave-one-out)で調べた。結果は環境で違う: GJI ATOK は「変換中」(IMM の属性マスク/候補窓)で 0.977→0.994、GJI MS-IME プリセットは「直前キー」(F2/変換/Space/F0)で 0.950→0.978、MS-IME 本体はどちらでも変わらない(0.922、F0系の全角/半角英数の巡回記憶という別の隠れ状態)。
  本 ADR は、(1) `Status` に `converting` を加える、(2) 表のセルに直前キー(文脈キー)を加える、の2つを段階的に入れる案と、入れない/学習側だけで済ませる代案を比較する。永続化スキーマ(版上げ)と実行時の予測器(`key_effect_runtime`、前キー追跡が無い)の両方に関わる。
status: |-
  草稿(2026-09-30)。未レビュー・未着手。所有者は「変換中 + 直前キーの文脈」の方針を選択済み(2026-09-30)。Opus 敵対的レビューで収束させてから実装する。
related_adr:
  - "ADR-195"
  - "ADR-196"
  - "ADR-199"
---

# ADR-210: 学習表の状態に「変換中」と「直前キーの文脈」を加える(草稿)

## 背景と根拠

学習表は `(Status{open, mode, composing}, key)` をセルとする。同じセルで押下後の結果が割れる(=その状態だけでは結果が決まらない)と、`classify_robust` が非決定/履歴依存と判定し、`predict` は予測を出さない。学習の自己検証(独立ウォーク)の精度が基準 0.95 を割る主因も、観測数の少ない「見かけ上決定的なセル」に隠れ状態が潜んでいることだった(検証ログの誤答の大半が `n_obs=2` の det セル)。

### 実測(押下ごとの特徴量ログ、windows-latest、`--trace-features`)

各押下の前後で次を観測して記録した: 変換モード生値の全ビット、文モード、入力中文字数、カーソル位置、文節バイト数、属性マスク(`GCS_COMPATTR`)、確定文字数、候補窓の数、EDIT の文字数とキャレット、TSF の開閉と変換モード、直前2キー。

| 環境(プリセット) | 押下数 | 結果が割れるセル | その押下 | 現状の LOO 一致率 | +変換中 | +直前キー文脈 |
|---|---|---|---|---|---|---|
| GJI ATOK | 973 | 6/84 | 8% | 0.977 | **0.994** | 0.983(0x1C) |
| GJI MS-IME プリセット | 3646 | 29/171 | 19% | 0.950 | 0.959 | **0.978**(F2, 変換, Space, F0) |
| MS-IME 本体 | 3920 | 28/196 | 21% | 0.922 | 0.922 | 効果なし |

(LOO = 押下ごとの leave-one-out 一致率。文脈キーは貪欲に最大4つまで選んだ。)

- ATOK の割れは「入力中/変換中」(属性マスクと候補窓で観測できる)で説明できる。実行時予測器が既に持つ `Stage`(Typing/ConvSpace/ConvHenkan/ConvMuhenkan)に対応する。
- GJI MS-IME プリセットの割れは、閉状態からの F0〜F3/半角全角キーが「直前にどのキーを押したか」で開く/開かない・遷移先が変わるもの。
- MS-IME 本体の割れは、`(true,9,false)+F0 → 全角英数(0x08) / 閉(0x00)` のように、F0 系キーが状態を巡回する記憶に依存する。直前1〜2キーでも文字数でも説明できない。**本 ADR の対象外**(将来の課題)。

### 期待される改善の大きさ(正直な見積もり)

全セルで平均した LOO 一致率の改善は ATOK +1.7pt、GJI MS-IME +2.8pt、MS-IME 本体 0。自己検証精度は実行間で 0.94〜0.99 動くため、改善幅は判別しにくい。採否は複数回反復の平均で判断する(評価計画)。

## 既知の制約(調査結果、2026-09-30)

- `PersistedCell` は `{status, key, prediction}`。ctx(直前キー)は学習側のグラフと分類にだけ効き、**永続化されず**、`predict` も ctx を使わない(HistoryDep は予測なし)。
- 実行時(`awase-windows`)に前キーの追跡は無い。typing/converting は打鍵履歴の追跡(`ImeModel::key_track`)で決め、TSF の `composing` bool だけを観測に使う。IMM の属性マスク/候補窓は実行時にも学習時にも(診断以外では)使っていない。
- 実行時の予測は主に belief 更新(`KeyEffectPredicted`)だが、`toggle_contradiction`/`key_shadow_action` は学習表の矛盾で `shadow_action` を狭める方向に効くため、間接的に actuation の判断にも影響する。
- `CURRENT_SCHEMA_VERSION = 2`。`from_json` は不一致を `SchemaVersionMismatch` で棄却し、移行処理は無い(実行時は同梱表へフォールバック)。
- `Status{}` の直接構築が `awase-keymap-learn` 内に約20箇所、`awase-keymap-learn-win`/`awase-settings` のテストにある。`HashMap`/`HashSet` のキーと `Ord` 導出にも波及する。

## 決定案

### 決定1: `Status` に `converting: bool` を加える

- 観測(学習側): `ImmGetCompositionStringW(GCS_COMPATTR)` の属性に TARGET_CONVERTED/CONVERTED/TARGET_NOT_CONVERTED/FIXEDCONVERTED(いずれか)が含まれる、または `ImmGetCandidateListCountW > 0`。`composing == false` のとき常に `false`。
- 永続化: `PersistedCell.status.converting`(`#[serde(default)]`)。スキーマ版は 3 に上げる。**v2 の表は棄却せず、`converting=false` として読む**(v2 は typing と converting を畳んだ表で、実行時の現行動作=`composing` を一律 `Stage::Typing` に写像、と一致するため挙動は変わらない)。v3 の表を v2 相当の古いバイナリが読むと重複セルで `DuplicateCell` になり同梱表へ縮退する(安全側)。
- 実行時: `Stage::ConvSpace/ConvHenkan/ConvMuhenkan` → `converting=true`、`Stage::Typing` → `false` で検索する。学習表に該当セルが無ければ従来どおり `Typing` 行で代用(現行のフォールバック規則)。
- 縮退率(`coverage_slot_count`、`MIN_COVERAGE_RATIO=0.80`)の分母に converting の枠が加わる。分母が増えて基準を割らないか、実測で確認する。

### 決定2: セルに文脈キー(直前キー)を加える

- 文脈キーの集合は環境ごとにデータから決める(貪欲選択、利得 +0.5pt 以上、最大4キー)。学習が選んだ集合を表に保存する(`context_keys`)。
- セルのキーを `(Status, key, Option<KeyId> ctx)` に拡張する。`ctx` は「直前に押したキーが `context_keys` に含まれればそのキー、無ければ `None`」。
- `predict`/`classify_robust` を ctx 対応にする: ctx 付きで決定的なら予測を出す(現状は履歴依存で予測なし)。
- 実行時に前キーの追跡を新設する: 物理キーの押下を `ImeModel` 外(`key_pipeline`)で記録し、フォーカス変更・観測による状態訂正・一定時間経過(要実測)でリセットする。ctx が不明(`gap`)のときは `None` の行で代用する。
- 診断ログでは、直前キーが連続していない(間にリセット/セットアップ押下が挟まった)場合を `gap` として除いた。実行時も同じ扱い。

### 決定3: 段階分け

1. 段階1: `converting`(決定1)。学習・永続化・実行時のマッピングまで。
2. 段階2: 文脈キー(決定2)。段階1の実機結果を見てから着手する。

## 代案

- **A. 何もしない**: 精度基準の実行間ばらつきが改善幅より大きく、効果を確認しにくい。既存ユーザーの表への影響も無い。
- **B. 学習側だけ**: 特徴量で「隠れ状態に依存するセル」を検出して非決定と宣言し、予測を出さない。永続化・実行時の変更は不要だが、予測できるセルは増えない(精度の分母が変わるだけ)。
- **C. 段階1だけ**: 変換中のみ(ATOK に効く)。文脈キーの実行時追跡という最大のコストを避けられる。

## 評価計画(受け入れ基準)

- 診断ログ(5本)の LOO 一致率は「学習データ内」の評価。採用前に、独立ウォークの自己検証精度で GJI ATOK / GJI MS-IME プリセット / MS-IME 本体を**各5回以上**反復し、平均精度が +1.5pt 以上・rejected の頻度が増えない・MS-IME 本体が悪化しない、を確認する。
- 学習時間の増加(状態数・セル数が増える分)を測る。現状: GJI MS-IME プリセット約117秒、ATOK約44秒(PR #388/#389/#391 反映後の見込み)。
- 実行時: ジャーナルリプレイ(`journal_replay.rs`)で、`converting` を含む表・含まない v2 表の両方が予測を壊さないことを確認する。

## 未解決の論点(レビューで確認したい)

1. 学習時は属性マスク/候補窓で「変換中」を**観測**し、実行時は打鍵履歴の追跡で**推定**する。この不一致で、学習表の `converting=true` セルが実行時に誤って引かれる(または引かれない)危険はないか。
2. 文脈キーの実行時追跡のリセット条件(時間・フォーカス・観測訂正)は、実測なしで決められるか。誤ったリセットは誤った予測より悪いか。
3. スキーマ版 3 で v2 を受理する方針(`converting=false` として読む)は、旧表の semantics 変更を隠さないか。`fingerprint`/`env_version`/`staleness` との整合はどうか。
4. MS-IME 本体に効かない(0.922 のまま)のに複雑性を増やす価値があるか。複雑性予算(`complexity-budget.md`)の観点。
5. `key_shadow_action`/`toggle_contradiction` が学習表の矛盾で actuation を狭める経路を持つ。`converting`/ctx 次元の追加でこの矛盾判定の意味が変わらないか。
6. 状態数が増える(converting で composing セルが約2倍、ctx で最大 5 倍)ことで、セルあたりの観測が薄まり、`min_minority=2` の誤り耐性分類が壊れないか。

## 影響範囲(調査結果)

`awase-keymap-learn`(`model.rs`/`table.rs`/`persist.rs`/`verify.rs`/`graph.rs`/`strategy.rs`/`exec.rs`/`sample_models.rs`/`minimize.rs`/`mismatch_tag.rs`)、`awase-keymap-learn-win`(`driver.rs` の観測、`main.rs` の `build_persisted_cells`)、`awase-windows`(`state/key_effect_runtime.rs`、`key_effect_predictor.rs`、`runtime/key_pipeline.rs`、`bug_report.rs`)、`awase-settings`(`keymap_learn_status.rs`)。同梱表の生成器(`gen_key_effect_table.py`)は学習表と別形式のため対象外の見込み(要確認)。
