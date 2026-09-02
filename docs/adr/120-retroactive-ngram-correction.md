# ADR-120: n-gram 事後訂正 — 後続文脈による曖昧決定の再評価と BACKSPACE 書き換え

## ステータス

**Phase 0a のみ採用（2026-09-02）。決定2〜8 は「判断点1 のゲートを通過した場合にのみ実装する」
条件付き設計であり、通過しなければ本 ADR は棄却クローズする。**

GitHub issue #140 の (b)（PR #141 のスコープから明示的に外した部分）に対応する。
Opus 敵対的 premortem 4 ラウンドで収束（経緯は末尾「Premortem の経緯」）。

いま実装してよいのは **Phase 0a（既存決定のカウンタのみ、新設計ゼロ、実運用の挙動は無変化）**
だけである。決定2〜8 は Phase 0a のデータが棄却ゲートを通過するまで 1 行も書かない。
これは 2026-08-30 の「計装を先に」という結論の履行であると同時に、
**この機能が実際には一度も発火しない可能性・実害が利得を上回る可能性を、実装コストを
払う前に潰すため**の順序である。

## コンテキスト

### 何を作りたいか

`TimingJudge::three_key_pairing`（`src/engine/timing.rs:113-172`）は char1→thumb→char2 の
3 鍵並びで thumb をどちらとペアリングするかを 2 段で決める。

1. **Phase 1**: d1(char1→thumb) と d2(thumb→char2) の差が `timing_margin_percent`（既定 30%）
   を超えるならタイミングだけで確定
2. **Phase 2**: タイミングが接近しているときだけ n-gram スコアで決着

Phase 2 が使える文脈は `recent_kana` ＝**その決定より前に確定したかな**だけである。
本 ADR は、曖昧決定の 1〜2 かなあとに同じ 2 択を再評価し、結論が覆るなら
BACKSPACE で消して打ち直す機構を設計する。

### (a) は済んでいる

issue #140 の (a) は PR #141（squash `1045a05e`）でマージ済み。BUG-105 として
`docs/known-bugs.md` に記録。本 ADR は Phase 1/Phase 2 の判定式そのものには手を触れず、
**その出力を後から書き換えるかどうか**だけを扱う。BUG-105 の「既知の限界」
（`three_key_pairing` は keydown の 3 タイムスタンプしか見ず release 時刻の概念を持たない）が
指す誤判定クラスが、本 ADR の想定顧客である。

### 既存の投機出力機構との関係

| | 既存 `retract_and_replace` | 本 ADR の事後訂正 |
|---|---|---|
| 訂正の契機 | 次の 1 イベント（親指キー or タイムアウト） | 後続 1（既定）〜2 かなの確定 |
| 窓の長さ | 数十 ms | **2 打鍵分**（k=1）／3 打鍵分（k=2） |
| 使う証拠 | タイミングのみ | 決定より**後**のかなを含む n-gram 文脈 |
| BS 数 | 常に 1 | 2（k=1）／3（k=2） |
| 有効な `ConfirmMode` | `Wait` 以外（＝既定では無効） | **`Wait`（既定）を含む全モード** |

3 鍵仲裁は char2 到着時点で即座に出力を確定・送出するので、既定 `Wait` でも
「既に出してしまった誤り」は存在する。BUG-105 はまさに既定 `Wait` で起きた。
したがって本機能は `ConfirmMode` と**直交する軸**である（決定7）。

### BACKSPACE 訂正の前科

- `RawTsfLiteralRecovery` の suffix 再送方式は PR #103 マージ後、premortem
  （Sonnet 1 体 + Opus 2 体、6 ラウンド）で **4 項目（うち致命度「高」2 件）＋後日判明 1 項目**が
  指摘され PR #104 で revert された。
- IME OFF キー選択は 5 日間で 6 回反転した（`docs/experiments.md` エントリ 01）。

引き継ぐ教訓は 2 つ。

1. **「放棄」が常に安全でなければならない**（決定4、INV-120-1）。本機能の最悪ケースは
   「今日と同じ」であって「今日より悪い」ではない、という性質を設計の中心に置く。
2. **未検証の前提・偶然の防御・実在しない機構を設計の土台にしない。**
   本 ADR の設計過程では、この失敗が 3 ラウンド連続で起きた（premortem の経緯を参照）。
   最終案は**実コードで確認した双条件と、既に存在する適用経路**だけを土台にしている。

### 2026-08-30 の結論との整合

2026-08-30 の Opus 敵対的議論の合意は (1) `NgramPredictive` は opt-in のまま、
(2) 計装を先に、(3) `confirm_mode`/`simultaneous_threshold_ms` の既定値は凍結、
(4) プラットフォーム固有の判断は platform 層に、(5) チャネル競合は未解決のまま先送り、であった。

- (1)(3) は遵守する。本 ADR は既定値を一切変更せず、新機能自体の既定も `off`。
- (4) は決定6。
- (5) は決定6 で回答する（新しい BS チャネルを増やす以上、解かずには進めない）。
- (2) について、**Phase 0 全体を「純粋な計装」と称するのは誤りである**——反転率と適格率の
  観測は決定2/決定8 の実装を要する。そこで

  - **Phase 0a** ＝ 既存決定のカウントのみ（新設計ゼロ）。ここまでが純粋な計装。
  - **Phase 0b** ＝ 決定2・決定8 の実装を伴う shadow 評価。ここは既に新機構の実装。

  と分割し、**棄却は Phase 0a のデータだけで下せる**ようゲートを設計した。
  判断点1 で棄却できる範囲は E1+E2+E7+E9 に及ぶ（決定0a 項目 2b/2c）。

## 用語

- **曖昧決定**: `three_key_pairing` の Phase 2 に到達し、スコア差が小さかった 3 鍵仲裁。
- **候補B**: 実際に採用された側。**候補A**: 採用されなかった側。
- **X / Y₂**: 候補A が出力したはずの 2 かな（X = char1+thumb、Y₂ = char2 の Normal 面）。
  `committed` には存在しない**仮想出力**である。
- **span**: 訂正で書き換える `committed` 末尾のエントリ列（＝候補B の実際の出力）。
- **訂正バッチ**: `[BS × m, 候補A の replay…, 通常の後続出力…]` を 1 回の `send_keys` で送るもの。

## 決定

### 決定0a（Phase 0a、いま実装してよい唯一の部分）: 既存決定のカウンタのみ

新しいスコア関数も適格判定も実装しない。既に走っている判定の結果を数えるだけ。

| # | 測る値 |
|---|---|
| 1 | 3 鍵仲裁の総数と Phase 2 到達率 |
| 2 | Phase 2 の `score_a`/`score_b` の生値と差の分布。加えて **`NEG_INFINITY` だった割合・`0.0`（未知センチネル）だった割合** |
| **2b** | **Phase 2 に到達した 3 鍵仲裁のうち、`lookup_kana_at(char2.pos, Face::Normal)` が `Some` かつひらがなだった割合**（E9 の Y₂ 側上界） |
| **2c** | **Phase 2 決定の直後、連続 2 打鍵（k=1 の窓）に親指キーの KeyDown が 1 つも無かった割合**（E7 充足率） |
| 4 | 曖昧決定から後続 1/2 かな確定までの経過 ms 分布と打鍵数 |
| 7 | **精度プロキシ（対照群つき）**: 決定から N ms 以内のユーザー訂正操作の頻度を、Phase 2 決定群 / Phase 1 決定群 / 曖昧でない打鍵群 で比較 |

**項目 2b/2c が「新設計ゼロ」を壊さない理由**:
2b は `lookup_kana_at(char2.pos, Face::Normal)` という既存テーブルへの 1 行 lookup、
2c は既存のイベント列に対する親指 KeyDown の有無の計数であり、
どちらもスコア関数でも適格述語でもない。

**項目 2b の根拠となる双条件**: `lookup_face`（`src/engine/nicola_fsm.rs:789-796`）と
`impl From<&YabValue> for KeyAction`（同 73-101）を突き合わせると、次が成り立つ。

> `lookup_face` が返す `kana.is_some()` ⟺ 同時に返る `KeyAction` が `Char(ch)` であり `ch == kana`

（`Romaji { kana: Some(ch) }` → `Char(ch)`／`Literal(s)` → 先頭 1 文字の `Char`。
それ以外は `kana: None` かつ `Char` 以外。）帰結:

1. E3′ は `action` の match を要さず **`entry.kana.is_some() && ひらがな`** で書ける。
2. **E9 の X 側は E2 の finite 要求とほぼ同値**（`score_a != NEG_INFINITY` ⟺
   `char1_thumb_kana.is_some()` ⟺ X が `Char`）。項目 2 が既に X 側の適格率を測っている。
3. 新規に要るのは **Y₂ 側だけ**＝項目 2b。

**項目 7 の計数対象**: NICOLA 使用者の訂正操作は 2 経路あり、**両方を数える**。

- **(a) 物理 BACKSPACE**: scan `0x0E` は `scan_to_pos_jis` に無いため `classify_key` の
  最終 else で `Passthrough` に落ち、bypass 経路で VK `0x08` として観測できる。
- **(b) 配列セル由来の BACKSPACE**: `layout/nicola.yab` は `:` キー（`scan_to_pos_jis` の
  `0x28 => (2, 10)`）に `後` を割り当てており、`KeyClassification::Char` として FSM を通り
  **`KeyAction::SpecialKey(Backspace)`** として出力される。
  **ホームポジションから手を動かさずに訂正できるこちらが、NICOLA 使用者の自然な経路である。**
- **(c)** `逃`（`]` キー、`SpecialKey(Escape)`）は別カウントする（訂正以外の離脱と区別するため）。

項目 2 で `NEG_INFINITY`/`0.0` の割合を先に測るのは、`|score_a − score_b|` が連続的な信頼度では
なく「片方が `NEG_INFINITY`（かな未解決）」「両方 `0.0`（n-gram 未知）」という離散的な塊を含む
混合分布だからである。E2 の θ_amb を引く前に構成比を知る。

記録は**単純カウンタ集計**で足り、レコード列も pull API も要らない。

#### 判断点1 の棄却ゲート

- **(i) 母数ゲート**: 3 鍵仲裁のうち Phase 2 到達が X% 未満なら棄却クローズ。
- **(ii) 精度プロキシ相関ゲート**: Phase 2 決定後 N ms 以内のユーザー訂正操作（(a)+(b)）の率が、
  対照群のそれを統計的有意に上回らなければ棄却クローズ。対照群があるため
  「ユーザーは色々な理由で BS を押す」という交絡が相殺される。
- **(iii) 適格率上界ゲート**: **項目 2 の finite 率 × 項目 2b × 項目 2c** が Y% 未満なら
  棄却クローズ（E1+E2+E7+E9 だけで既に実用に足りない）。
- X・N・Y の具体値は Phase 0a のデータを見てから決める（実測前に定数を置かない）。

**(iii) の積は見積りであることの注記**: 3 条件は独立ではなく相関しうる
（例: 親指面を多用する打鍵列は濁音・半濁音を含みやすく、Y₂ の Normal 面かなの有無とも
相関する）。したがって積は上界の目安として扱い、**内訳のうち支配項がどれかを見て判断する**。

見立てとして、**E7（項目 2c）が支配項になる可能性が高い**。`layout/nicola.yab` の
かなセル分布は通常面 26／左親指面 27／右親指面 28 で、**濁音・半濁音はすべて親指面**にある。
したがって日本語文の打鍵における親指キー使用率はおおむね 4〜5 割に達し、
k=1 の窓（連続 2 打鍵）が親指なしである確率は概ね 0.55² ≈ 0.3 程度——
**E7 だけで適格率を 1/3 程度に落としうる**。この見立てが実測で裏付けられ、かつ
(iii) を通らないなら、決定2/決定8 を 1 行も書かずに棄却クローズするのが正しい結末である。

**反転率はゲートに使わない**（正解データが無く、決定8 の交絡と限界4 の力学的乖離があるため）。
反転率は Phase 0b の診断値として記録する。

### 決定0a-report: Phase 0a の集計を不具合報告（ADR-095）に含める

**なぜ必要か**: 決定0a は「何を数えるか」だけを定め、**数えた結果をどう取り出すか**を
決めていなかった（Phase 0a 単独では判断点1 のデータが開発者の手元に届く経路が無い）。
このリポジトリには既に、タスクトレイの「不具合を報告」機能
（[ADR-095](095-tray-bug-report-cloudflare-intake.md)）というユーザー環境からの
情報回収経路がある。判断点1 のソークをこの経路に相乗りさせる。

**実装（Phase 0a に含めて同時に行う。Phase 0a を「計装のみ」で終わらせず、
配信経路まで含めて初めて実測データが得られる）**:

1. **core**: `NicolaFsm` に決定0a の項目 1/2/2b/2c/4/7 を集計する
   `RetroEvalStats`（`src/engine/retro_eval_stats.rs` 新設）フィールドを持たせる。
   起動からの累積カウンタで、`three_key_pairing` を呼ぶ箇所（`compute_prefer_char1`）・
   `step_pending_char_thumb_3key`・ユーザー訂正操作（項目7、決定0a が定義する
   物理 BACKSPACE と配列セル由来 `SpecialKey(Backspace)` の両方）の観測点で加算する。
   スコア関数・適格判定は実装しない（決定0a の「新設計ゼロ」原則を維持）。
   公開経路は `NicolaFsm::retro_eval_stats(&self) -> &RetroEvalStats`（`pub`）→
   `FsmAdapter::retro_eval_stats(&self) -> &RetroEvalStats`（`pub(super)`、
   `fsm: NicolaFsm` は私有フィールドのため中継が要る）→
   `Engine::retro_eval_stats(&self) -> &RetroEvalStats`（`pub`）の3段委譲
   （いずれも `const fn` 化できる、`FsmAdapter::new` の前例と同様）。

   **項目4・項目7 は固定バケットのヒストグラムにする（単一の sum/count 対にしない）**。
   決定0a は「X・N・Y の具体値は Phase 0a のデータを見てから決める」
   「経過 ms **分布**」と明記しており、単一カウンタ対では事後に N（項目7 の
   相関窓）や `retro_window_ms`（項目4）のパーセンタイルを選べない——実測前に
   定数を固定してしまうことになり、決定0a 自身の原則および
   [tuning-constants](../../.claude/rules/tuning-constants.md) の実測義務
   （最大値+マージンの導出）を満たせない。

   ```rust
   /// 桁スケールの粗いバケット境界（ms）。粒度を細かくしないこと——
   /// 個人の打鍵ダイナミクス（バイオメトリクス的特徴）に近づけないため、
   /// 分布形状が分かる最小限の解像度に留める。
   pub const ELAPSED_MS_BUCKETS: [u64; 7] = [50, 100, 200, 400, 800, 1600, u64::MAX];

   #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
   pub struct RetroEvalStats {
       pub three_key_total: u64,
       pub phase2_reached: u64,
       pub phase1_reached: u64,
       pub no_ngram_count: u64,
       /// score_a/score_b は独立な値なので別々に3値分類する（実装レビューで
       /// 訂正: 初稿は `score_neg_infinity_count`/`score_both_zero_count`/
       /// `score_finite_count` という単一集計案だったが、`(score_a, score_b)`
       /// の組み合わせによっては「どちらのカテゴリか」を一意に決められない
       /// ケースがあり誤りだった）。
       pub score_a_neg_infinity_count: u64,
       pub score_a_zero_count: u64,
       pub score_a_finite_count: u64,
       pub score_b_neg_infinity_count: u64,
       pub score_b_zero_count: u64,
       pub score_b_finite_count: u64,
       pub char2_normal_hiragana_count: u64,        // 項目2b
       pub no_thumb_followup_count: u64,             // 項目2c
       pub followup_elapsed_ms_histogram: [u64; 7],  // 項目4（バケット定義は上記）
       /// 項目7: Phase2決定後の経過msバケットごとのユーザー訂正操作回数（分子）。
       /// 分母（その群で何回決定が起きたか）は1個の数なのでスカラー。
       /// 訂正率(N) = prefix_sum(ヒストグラム, Nまで) / 分母 で、
       /// N をゲート判定時に任意に選べる（バケット境界=候補Nの上限）。
       /// 対照群(Phase1決定後/非曖昧打鍵後)も同じ形で持つ。
       /// （2026-09-02実装レビューで訂正: 初稿は分母も`[u64;7]`だったが、
       /// 分母をバケット分けする対象は無いため誤りだった）
       pub phase2_decisions_total: u64,
       pub phase2_correction_histogram: [u64; 7],
       pub phase1_decisions_total: u64,              // 対照群1・分母
       pub phase1_correction_histogram: [u64; 7],
       pub baseline_decisions_total: u64,             // 対照群2（曖昧でない打鍵後）・分母
       pub baseline_correction_histogram: [u64; 7],
       /// 項目7(c): Escape出力回数（訂正カウントとは別、離脱と区別するため）
       pub escape_output_count: u64,
   }
   ```

   ヒストグラムも「単純カウンタ集計」の範囲内（固定長配列のインクリメントのみ、
   新しいスコア関数でも適格判定でもない）であり、決定0a の「新設計ゼロ」を壊さない。

   起動からの累積値のみで、タイムスタンプ付き系列は持たない。複数の不具合報告を
   `process_uptime_secs`（`BugReportStateSnapshot` に既存）と突き合わせれば、
   セッション間の差分から任意の期間の増分を後から復元できる。

2. **platform**（`crates/awase-windows/src/bug_report.rs`）: `BugReportStateSnapshot` と
   並ぶ形で `BugReportRetroEvalStats`（フィールドは `RetroEvalStats` と同型、
   `Serialize`/`Deserialize` を持つ独立型として定義——`BugReportPayload` は内部型を
   直接 `Serialize` しない、という ADR-095 決定3/B-5 の allowlist 原則を踏襲する）を追加する。

   `attach_retro_eval_stats: bool` を `BugReportInput`（`:213-223`）と
   `BugReportPayload`（`:162-168`）に追加する。**`BugReportDiagnostics`
   （`:178-186`）にはデータのみを持たせる**（既存の `state_snapshot`/`config_toml`/
   `layout_yab` と同じく、`attach_*` は `BugReportInput`/`BugReportPayload` 側にしか
   無い）: `retro_eval_stats: Option<BugReportRetroEvalStats>` を追加し、
   `BugReportDiagnostics` の手書き `Default` 実装（`:188-200`）も更新する。
   `build_payload_with_log_budget`（`bug_report.rs:239`）で他の `attach_*` と
   同じ if 分岐を辿らせる。

   `current_bug_report_diagnostics`（`runtime/message_handlers.rs:1182`）で
   `app.engine.retro_eval_stats()` を読み、`BugReportStateSnapshot` の構築と同じ場所で
   `BugReportRetroEvalStats` へ変換する（新規の COM/TSF 呼び出しは発生しない
   ——決定8 の原則と同じ「既存のインメモリ状態の読み取りのみ」）。

   **既定は ON**（他の `attach_state_snapshot`/`attach_config`/`attach_layout` と同様、
   決定4/決定9 のパターンを踏襲）。項目1/2/2b/2c/4/7 はいずれも累積カウンタであり
   打鍵内容・個人を特定しうる情報を含まないため、`journal`/`app_log`（生打鍵列を含む、
   マスキングなし）と同じ慎重さは要らない。ただし ADR-095 決定4 の「送信前プレビューで
   全文表示・個別に外せる」という必須要件は他の添付と同様に適用する。

   **既定 ON でも実収集率は 100% にならない**ことに注意する。実報告
   `01M15R86FJW24278GGD3ETS9QX`（`docs/bug-reports-triage.md:42`）は
   `attach_log`/`attach_config`/`attach_state_snapshot` をいずれも `false` にして
   送信されている——ユーザーは実際に個別トグルを使って添付を外す。判断点1 の
   ゲート(ii)（対照群付き有意差判定）は、この分だけ想定より少ないサンプルで
   判断することになるため、統計的検出力の見積りにこの実収集率の低下を織り込むこと。

3. **schema_version は上げない**。新フィールド
   （`attach_retro_eval_stats`/`retro_eval_stats`）は Worker 側
   （`services/report-worker/src/index.ts::validatePayload`）で
   `optionalBoolean`/`optionalNullableRecord` 相当の検証にし、フィールド自体が
   存在しない旧クライアントの報告も受理する。

   **理由（棄却した代案の記録）**: 当初 schema_version を 3→4 に上げ、ADR-095
   決定7・決定8 の前例（v1→v2 で新規必須フィールドを追加）に倣う案を書いたが、
   premortem で否定された。`validatePayload`（`index.ts:179-180`）は
   `value.schema_version !== SCHEMA_VERSION` という**厳密等値**で弾く実装であり、
   版を上げた瞬間、**まだ更新していない全ユーザーの不具合報告が
   `unsupported_schema_version`（400）で拒否される**——retro 統計と無関係な通常の
   不具合報告も含めて全滅する。この ADR の目的は「ユーザー環境から実測データを
   回収すること」であり、報告してくれる層には「不具合に遭遇したが最新版にしていない」
   ユーザーが構造的に多く含まれるため、判断点1 のサンプルを増やすための変更が
   報告総数を減らすという逆効果になる。`index.ts` には既に同型の前例
   （`app_log_excerpt` を `optionalNullableString` で受理し「このフィールドを
   送らない旧クライアントの報告を拒否してはならない」とコメントされている、
   BUG-34 横展開）があり、今回もそれに倣う。ADR-095 決定7/8 が同じ副作用を
   持っていたことは、当時何件の報告を落としたか誰も測っていない以上、
   先例として踏襲する理由にならない。

**この決定が決定0a 自体を変えないことの正確な範囲**: `RetroEvalStats` は
カウンタの**保持**を core に追加するだけで、新しいスコア関数・適格判定・訂正出力を
一切追加しない。**IME に送る打鍵内容とタイミングは無変化**——決定0a が
「実運用の挙動は無変化」で保証している範囲はここまでである。一方で
**コードには軽微な変化がある**ことを正確に書く: 3 鍵仲裁が起きるたびに
テーブル引き（`lookup_kana_at(char2.pos, Face::Normal)`、項目2b）1 回とカウンタ
加算が増え、passthrough 打鍵・配列セル由来 BACKSPACE 出力のたびに項目7 用の
カウンタ加算が増える（実測上は既存の TSF/COM 呼び出しや `SendInput` と桁違いに
軽微だが、「挙動不変」をこのリポジトリでは強い保証として使ってきた
——ADR-112 決定0 の「挙動不変の純粋リファクタ」等——ため、語の水準を落とさない）。

**テスト方針への影響（必須）**: INV-120-1（`retro_ngram_correction = "off"` の
実行と「1 bit も違わない」ことをプロパティテストで固定する）は、`RetroEvalStats`
のカウンタ増分ぶんだけ厳密には破れる。**比較対象から `RetroEvalStats` を除外する**
とテスト方針に明記すること（除外しないとテストが書けないか、書いた瞬間に落ちる）。

**`retro_ngram_correction`（決定7）とは独立に、常時集計する**: 項目 1/2/2b/2c/4/7 は
既に無条件で起きている 3 鍵仲裁・ユーザー打鍵の観測にすぎず、新しい判定・出力を
一切伴わない。したがって `retro_ngram_correction = "off"`（既定）でも
`RetroEvalStats` は加算し続ける——判断点1 のデータを集めるためにユーザーへ
config 変更を求めるのは Phase 0a の趣旨（既定のまま自然に実測が貯まる）に反する。
`retro_ngram_correction` が実際にゲートするのは決定2/決定8（Phase 0b 以降、
新しいスコア関数と適格判定を伴う）のみである。カウンタ自体は個人を特定しうる
内容を含まない集計値なので、この無条件収集は決定4/決定9 が要求する
「送信前プレビューで確認できる」枠内で許容する（収集は常時・送信は
`attach_retro_eval_stats` トグルとプレビューでユーザーが制御する、という
既存の journal/state_snapshot と同じ二段構え）。

**棄却した代案**: 専用のログファイル・デバッグコマンド（`awase.exe --dump-retro-stats`
等）を新設する案。既存の不具合報告経路（ADR-095）は複数ユーザーからの複数セッション分の
スナップショットを`report_id`ごとに`process_uptime_secs`と一緒に既に集められる仕組みが
あり、専用の新規回収経路を作るコストに見合わない。

### 決定2: 適格条件

以下**すべて**を満たす場合のみ訂正窓を開き、訂正を実行する。1 つでも欠ければ
「窓を開かない／既に開いていれば破棄」であり、破棄は常に安全（決定4）。

- **E1（Phase 2 限定）**: `three_key_pairing` の Phase 2 で決着した決定であること。
  Phase 1 は対象外（BUG-105 で撤回したばかりの「弱い根拠でタイミングを無視する」方向へ
  逆走しないため）。
- **E2（低信頼のみ）**: `|score_a − score_b| < θ_amb` かつ両スコアが finite。
  θ_amb は Phase 0a 項目 2 の分布から決める。**初期値は本 ADR に書かない。**
- **E3′（span の述語）**: span の全エントリが **`entry.kana.is_some()` かつその kana が
  ひらがな**であること。

  双条件により、これは「全エントリが `KeyAction::Char(ch)` かつ `ch` がひらがな」と等価であり、
  `OutputEntry` が既に持つデータだけで判定できる。この述語の下では

  > 1 エントリ = 1 `Char` = 1 かな = 1 完全ローマ字チャンク

  が成り立つので、競合する 2 つの BS モデル（`src/engine/output_history.rs:89-91` の
  「完全なローマ字は IME で 1 composition unit なので BS は常に 1」と「1 かな = 1 BS」）は
  **一致し、本 ADR はどちらが正しいかを選ばずに済む**。

  ひらがな要求の理由: 記号セルは `KeySequence`/`Sequence` になりがちで `lookup_face` の
  kana 抽出が `None` を返し（`docs/known-bugs.md` の ADR-115 既知の限界）、IME 側の変換・
  確定挙動を誘発しうるが core からは観測できず、n-gram モデルはひらがな n-gram だからである。
  なお `Literal("、")` のように **`kana` が `Some` でもひらがなでない**ケースがあるため、
  ひらがな判定は `kana.is_some()` とは独立に必要である。
- **E5**: 欠番（設計過程で削除）。「span の物理キーが全て解放済みであること」という条件を
  検討したが、`RewriteTail` が `pending_releases` を触らない以上何も守らず、高速打鍵時の
  ロールオーバーで失格するため Phase 2 到達確率と失格確率が同一の潜在変数（打鍵速度）で
  駆動される構造的同源を作ってしまうため撤回した。必要な保証は E3′ が与える——`Char` は
  `drain_pending_releases_as_keyups` の doc が明言するとおり「Unicode 注入で完結済みで
  OS 側に押されっぱなしの VK が無い」ため KeyUp 対発行の義務を持たない。
- **E6（窓が有効）**: 決定4 の無効化イベントが 1 つも起きていないこと。
- **E7（反実仮想が決定論的に定まること）**: **曖昧決定から訂正発火までの間に親指キーの
  KeyDown が 1 つも無いこと**。

  理由: 候補B世界では char2 は `Reduce` で即座に確定し FSM は Idle になるが、
  候補A世界（`reduce_char_thumb_and_continue`）では **char2 は
  `ReduceAndContinue{remaining: char2}` で FSM に再投入され、既定 `Wait` では
  `enter_pending_char(char2)`＝未確定**である。その後に親指キーが来れば char2 は
  親指面かなに化けうる。後続に親指 KeyDown が無いことを確認できて初めて、
  `step_pending_char_char` 経由で `resolve_pending_char_as_single`（Normal 面）に
  落ちることが決定論的に言える。
- **E8（履歴更新が `record` に一本化されている経路にのみ相乗り）**:
  訂正を相乗りさせる先が、出力履歴を **`ParseAction` の `record` フィールドだけで更新する**
  `Reduce`/`ReduceAndContinue` であること。

  **これは「遅延適用の経路」という意味ではない。** `record` は
  `crates/timed-fsm/src/parser.rs:381-397` の parse ループ内で `self.on_reduce(record)`
  （→ `NicolaFsm::on_reduce` → `update_history`）により**即座に**適用される——`Response` が
  組み上がる前、`send_keys` よりはるかに前である。E8 が選別しているのは「遅い経路」ではなく
  **「履歴への書き込み口が `record` 1 つに揃っている経路」**である。

  E8 が排除するもの:
  - `commit_char1_output` を通る経路（3 鍵仲裁の各分岐）——`update_history` を eager に
    呼んでから `record: OutputUpdate::None` を返すため、`RewriteTail` が数える
    `committed` 末尾が分岐に依存してしまう。
  - **タイムアウト経路**（`timeout_pending_char`、`src/engine/nicola_fsm.rs:2125-2129`）——
    `update_history` を eager に呼んでから `Resp` を直接組む。これを排除していることが
    決定3 の「非同期 BS が飛ばない」主張を支えている（load-bearing、決定3 参照）。
- **E9（候補A の仮想出力にも同じ述語）**: **X と Y₂ も `kana` が `Some` かつひらがな**であること。

  E3′ が見るのは `committed` の span＝**候補B（実際に起きた方）の出力**であり、
  訂正バッチが送る X・Y₂ は `committed` に存在しない仮想出力なので E3′ では検証されない。
  双条件により、X 側は E2 の finite 要求と実質同値、**Y₂ 側だけが新規**である。

  なお `layout/nicola.yab` のホーム段末尾（`:`→`後`、`]`→`逃`）は
  `KeyClassification::Char` の配列キーであり 3 鍵仲裁の char1/char2 になりうるが、
  `lookup_face` が `kana: None` を返すため E2 の finite 要求で弾かれる。
  これは双条件から導かれる構造的な性質であって偶然ではないが、
  **Y₂ 側は E2 が守っていない**ため E9 の明示は依然として必要である。

### 決定3: 訂正は「後続打鍵と同一バッチ」でしか送らない（非同期訂正の禁止）

訂正専用のタイマーを作らない。訂正は、適格性を満たした後続かなを出力する
`ParseAction::Reduce`/`ReduceAndContinue` に相乗りする形でのみ送出する。

```
通常:   [Char(s₁)]
訂正時: [BS × m, Char(X), Char(Y₂), Char(s₁)]              （k=1: m=2）
        [BS × m, Char(X), Char(Y₂), Char(s₁), Char(s₂)]   （k=2: m=3）
```

**発火タイミング**: 既定 `ConfirmMode::Wait` では s₁ が押された時点では出力されない
（`idle_wait` → `ParseAction::Shift`、actions 無し）。s₁ が Reduce されるのは
**s₂ が押されたとき**（`step_pending_char_char` → `into_reduce_and_continue`）である。したがって

- **k=1 の訂正は曖昧決定の 2 打鍵後**（s₂ の KeyDown 時）に着弾する。
- **k=2 は 3 打鍵後**。

**「非同期 BS が飛ばない」は E8 に支えられている（load-bearing）**:
決定3 単独では成り立たない——既存の pending タイマーが s₁ を非同期に flush しうるからである。
実際にこの経路を塞いでいるのは **E8** である（`timeout_pending_char` は eager 経路）。
**E8 を緩める提案が将来出たときの歯止めとしてここに明記する。**
テスト方針にも「タイムアウト flush には訂正が相乗りしないこと」を入れる。

**同一 `send_keys` バッチ性**: `timed-fsm` の `parse`（`crates/timed-fsm/src/parser.rs:376-401`）は
`ReduceAndContinue` のたびに `actions.extend(output)` し、終端で 1 本の `Response` を返す。
したがって訂正バッチと継続処理の actions は**同一の `send_keys` 呼び出しに入る**。帰結:

- BS が先頭に集まる形は保たれる。
- ただし継続処理が Char の**後**に非 defer 対象（`SpecialKey`/`Key`/`CtrlChord`）を積むと、
  `crates/awase-windows/src/platform.rs:1023-1043` の `needs_unicode_cold_warmup` 判定が
  false になり、**GJI long-cold 時の warmup 保護がそのバッチ全体で外れる**
  （BUG-02 系の再発ファミリー）。逆にパターンが保たれた場合は Char が defer され、
  flush 失敗時に m かなを失う（限界1）。どちらも決定6b の送信時ゲートが
  「訂正を丸ごと落とす」ことで回避すべきケースである。

**「BS 方式は既存の出力経路を一切変えない」は成り立たない**: PR #141 の `commit_char1_output` は
罠を踏んだのではなく、遅延適用では `append_key_up_for` が依存する `remove_by_scan` が
対象を見つけられないため**意図的に eager を選んだ**（同関数の doc に明記）。結果として
`step_pending_char_thumb_3key` は eager push と `record` が同居する関数である。
本 ADR の回答は「触れない」ではなく**「混ぜない」**（E8）である。

履歴の更新には `OutputUpdate` に 1 バリアントを追加する:

```rust
OutputUpdate::RewriteTail {
    retract: usize,                       // committed から捨てる件数（= m）
    replay: SmallVec<[OutputEntry; 4]>,   // committed にだけ積み直す
    record: Option<OutputEntry>,          // 新しいキーの分。従来どおり両方に積む
}
```

`replay` が `pending_releases` を触らないことは E3′ が保証する。
**適用は他の `OutputUpdate` と完全に同じ**（`on_reduce` → `update_history`）であり、
特別扱いも保留もしない。`committed` の変更経路は
`push` / `retract_and_record` / `RewriteTail` の 3 つのままである。

### 決定4: 無効化と 2 つの不変条件

**INV-120-1（放棄は常に安全）**: 訂正窓が破棄された場合の出力・履歴・FSM 状態は、
本 ADR 導入前と完全に同一である。訂正窓は `NicolaFsm` の `Option<RetroWindow>`
フィールド 1 つに閉じており、破棄は `= None` だけで完了する。

**INV-120-2（訂正が発火したときの範囲）**: 訂正が書き換えるのは
`OutputHistory.committed` **だけ**である。FSM のそれ以外の状態
（`right/left_thumb_consumed`、solo カウンタ、`state`、`pending_releases`）は
**候補B が起きた世界のまま**であり、巻き戻さない。
E7 の下では `consume_thumb` は両世界で同じ side・同じ物理押下に対して呼ばれるため
`is_thumb_consumed` の結果は一致し、親指消費状態の差は後続の面選択に影響しない。

窓を破棄するイベント（core 側で判定できるもの）:

- **ユーザー自身の BACKSPACE / Delete**（物理 VK・配列セル由来の `SpecialKey(Backspace)` の
  **両方**）。決定3 により訂正は後続かなと同一バッチでしか出ないので、
  「ユーザーが BS を押した直後に我々の BS が飛ぶ」順序は発生しえない。
  **ただし `committed` はユーザーの BS では縮まない**（`handle_bypass` は `remove_by_scan` で
  `pending_releases` しか触らない）ため、窓を破棄する以外の救済手段は無い（限界3）。
- レイアウトのかな出力以外のあらゆる `KeyDown`（Space・Enter・変換・矢印・Tab・Esc・
  修飾キー付き打鍵・親指キーの単独タップ passthrough・`post_bypass` 経路を含む）。
- **親指キーの KeyDown**（E7）。
- 記号・非ひらがな・`kana` が `None` の出力（E3′/E9）。
- `flush(ContextChange::*)`・`toggle_enabled`・`swap_layout`・フォーカス変更。
- `SpeculativeChar` への遷移、`retract_and_replace` の発火（決定5）。
- `retro_window_ms` 超過。値は Phase 0a 項目 4 の実測分布から決める。**初期値は書かない。**
- 決定6a の bool が false になったこと。

### 決定5: 訂正義務スロットは 1 つだけ（既定構成では空振りする）

FSM が保持できる未完了の訂正義務は常に高々 1 つ。訂正窓が開いている間に
`SpeculativeChar` に入る／`retract_and_replace` が走る場合は訂正窓を破棄する。
曖昧決定が窓の内側で再発した場合は入れ子にせず、古い窓を訂正せずに閉じる。

**既定 `ConfirmMode::Wait` では `idle_wait` が `SpeculativeChar` へ遷移しないため、
この節の主要部分は既定構成では到達不能である。** 既定構成で実際に競合しうるのは
platform 側の `RawTsfLiteralRecovery` だけであり、2026-08-30 が積み残した
「チャネル競合」の本丸はそちら＝決定6 である。

### 決定6: ADR-019 境界

#### 6a: platform → core は bool 1 個（適格判定用）

`InputContext` に `retro_correction_allowed: bool` を追加する。core はこの値の理由を知らない。
platform 側で false にする条件（少なくとも）: composition が cold／warmup・probe が
in-flight／`RawTsfLiteralRecovery` が直近 X ms 以内に BS を注入した、または in-flight／
IME open/close の apply が in-flight／conv mode 変化直後・belief が不確か／`InjectionMode` 未確定。

**「Unicode 注入モードかつ GJI が long-cold」は 6a に入れない。**
6b の送信時ゲートがある以上 6a にも入れるのは二重ゲートであり、long-cold の間ずっと訂正を
不適格にして適格率を落とす（限界8 に直接効く）。判定は**送信直前の 1 箇所だけ**にする。

**`InputContext` の doc コメント（`src/engine/decision.rs:272-278`）が課すゲートの通過論証**:
同 doc は「OS 由来の瞬間値のみを含む」「このフィールドを増やす前に、Engine 内部状態で
代替できないか検討すること」と定めている。

- **Engine 内部状態では代替できない**: この bool は IME の composition 状態・warmup 進行・
  platform 側 BS チャネルの占有状況の関数であり、core はこれらを観測する手段を持たない
  （持たせることが ADR-019 違反になる）。`ime_on` が同じ性質の先例である。
- 「瞬間値」性は満たすが、複数の非同期条件の論理和であるため陳腐化が速い。
  そのまま使うと TOCTOU を生むので 6b を併設する。
- 網羅性を担保するのは、`InputContext` が `Default` を derive せず全フィールド必須で、
  `build_input_context` が本番唯一の構築点であるという**コンパイラの保証**である
  （`layer_boundary_guard.rs` に `InputContext` のフィールドを見るルールは無く、
  `thumb_context_guard.rs` は thumb の timestamp/shift 保存だけを見る）。

#### 6b: 送信時ゲート — eager 適用 + 例外時の有界な乖離

**問題**: core の適格判定は `InputContext` のスナップショットで行われるが、
`send_keys` が `gji_is_next_key_long_cold()` を評価するのはその後である。判定と送信の間に
GJI が long-cold に落ちると、バッチ中の `Char` が全部 defer され、
「BS が m 発着弾 → flush が失敗・遅延・欠落 → m かなが消えたまま何も戻らない」
という事故が起こりうる（`retract_and_replace` の被害 1 かなの m 倍）。

**採用案: eager 適用 + 保守的な 6a + 例外時の有界な乖離**

1. **`RewriteTail` は他の `OutputUpdate` と完全に同じタイミングで適用する**（決定3）。
   保留状態を持たないので、`committed` の第 4 の変更経路も、保留 `RewriteTail` の寿命問題も
   発生しない。
2. **core は訂正が乗るバッチを、`KeyAction` の新バリアントではなく `InputEffect` の
   新バリアントとして送る**:

   ```rust
   InputEffect::SendKeysWithRetract {
       retract: usize,            // m 発の BACKSPACE
       replay: Vec<KeyAction>,    // 候補A の再送分（先頭に来ることが保証される）
       rest:   Vec<KeyAction>,    // 訂正が無ければ送っていたはずの actions
   }
   ```

3. **platform は送信直前に自分の条件を再検査する**（`unicode_cold_defer_active()`、後述）。
   - 安全 → `[Backspace × retract] ++ replay ++ rest` を 1 回の `send_keys` で送る。
   - 危険 → **`rest` だけを送る**（訂正を丸ごと落とす。画面は候補B のまま＝今日と同じ）。
4. **落とされた場合、core は知らないままでよい。** `committed` は候補A に書き換わっているが
   画面は候補B なので乖離する。**この乖離は有界である**——`recent_kana(3)` は末尾 3 件しか
   読まないため、以後 3 かなの打鍵で自然に解消する（ただし「有界」であって「無害」ではない。
   限界4・限界5 を参照）。6a を保守的にしてあるのでこの例外自体が稀である。

**なぜ「送ってから適用を決める」方式を採らないか**: `record` は parse ループ内で
`on_reduce` により**即座に**適用される（`crates/timed-fsm/src/parser.rs:381-397`）ので、
「送信結果を見てから `committed` を書き換える」余地が無い。実現するには `update_history` が
`RewriteTail` だけを特別扱いして stash する必要があり、**`committed` の第 4 の変更経路**が
生まれる。さらに `record: Option<OutputEntry>`（s_k の分）を一緒に stash すれば
`recent_kana` が痩せ、別々に適用すれば「末尾の下に差し込む」操作になる。

**なぜ「破棄を core に報告する」方式を採らないか**: platform → core の逆流チャネルは
実コードに存在しない——`PlatformRuntime::send_keys` の戻り値は `()`（`src/platform.rs:244`）、
`InputEffect`（`src/engine/decision.rs:26-31`）は core → platform 片方向である。

**乖離の比較（頻度 × 向き）**: 「1 打鍵遅延 commit」（訂正の適用を次の入力まで保留する案）と
比べた場合、比較軸は「乖離の量」ではなく**「頻度 × 向き」**である。

| | 1 打鍵遅延 commit 案 | 採用案（eager + 例外時のみ乖離） |
|---|---|---|
| 乖離の頻度 | **訂正が成功した全ケース** | platform が落とした稀なケースのみ |
| 乖離の向き | 「誤りと判断した候補B」を `recent_kana` が返す → `adjusted_threshold` → `is_simultaneous` に**有害方向へ決定論的に**効く（限界4 のループを毎回自分で回す） | 「訂正が落ちた＝画面は候補B のまま」なのに文脈は候補A を返す。頻度が桁で低い |
| 実現可能性 | **不能**（上記の理由） | 可能 |

**`unicode_cold_defer_active()` の抽出（二重実装の回避）**:
`needs_unicode_cold_warmup`（`crates/awase-windows/src/platform.rs:1023-1043`）は 3 項の論理積
——(1) `injection_mode == Unicode`、(2) バッチ形状スキャン、(3) `gji_is_next_key_long_cold()`。
**(1)(3) を `fn unicode_cold_defer_active(&self) -> bool` として 1 関数に括り出し、
6b と `send_keys` の両方がそれを呼べば再実装はゼロになる。** 形状スキャン (2) は
`send_keys` に残す（6b の時点でもバッチ形状は分かるので、必要なら (2) も共有できる）。

**`PlatformRuntime` への追加と、`send_keys` の戻り値を変える案との差**:
`InputEffect::SendKeysWithRetract` を platform に届けるには `PlatformRuntime` に
新メソッドが要る。`PlatformRuntime`／`TsfComposition` には**デフォルト実装の前例が 7 件ある**
（`apply_ime_open`/`send_engine_state_ime_key`/`composition_output`/`output_in_flight_ms`/
`is_composition_warm`/`is_tsf_mode`/`on_ime_applied`、`src/platform.rs:279,308,323,328,333,338,346`）。
`fn send_keys_with_retract(&mut self, retract, replay, rest)` に
「`[Backspace × retract] ++ replay ++ rest` を組み立てて `self.send_keys()` を呼ぶ」という
デフォルト実装を付ければ、**awase-macos は無改修**で済み、Windows だけが送信時ゲート付きに
オーバーライドする。

**awase-linux は 1 arm の追加が必要**: `crates/awase-linux/src/output.rs:238` の
`execute_effects` は `PlatformRuntime` を経由せず `InputEffect` を直接 match しているため、
新バリアントに対する arm を 1 つ足す必要がある（デフォルト実装では吸収されない）。
それでも、`send_keys` の戻り値を変える案が**既存の全呼び出し側の型を変える**のに対し、
こちらは**新バリアントを追加した箇所だけ**で済む——これが両者の実質的な差である。

**棄却した代案**: 訂正処理そのものを platform 層に閉じる案——n-gram 再評価は core の責務であり
ADR-019 に反する。

### 決定7: config は `ConfirmMode` を拡張せず、直交フラグにする

```toml
[general]
retro_ngram_correction = "off"   # "off"（既定） | "shadow" | "on"
retro_lookahead_chars  = 1       # 既定 1（訂正は曖昧決定の2打鍵後に着弾）。最大 2（3打鍵後）
retro_window_ms        = <Phase 0a の実測後に決定>
```

`ConfirmMode` は「最初の出力をいつ出すか」の軸、事後訂正は「出した後どうするか」の軸で
直交する。バリアントとして足すと `Wait` × 事後訂正が表現できず、
**BUG-105 が実際に起きた既定構成で本機能が使えない**。

既定 `retro_lookahead_chars` を 1 にするのは、m を 3→2 に縮め、訂正着弾までの窓を
3 打鍵→**2 打鍵**に縮めるためである。kill switch は config のこの 1 項目とし、env var は置かない
（ADR-112 と同じ理由——各コミットが独立に revert 可能）。

### 決定8: 再評価スコア関数

#### 現行 Phase 2 が実際に計算しているもの

`src/engine/timing.rs:142-153` と `compute_prefer_char1` が渡す 3 引数から:

- `score_a = frequency_score(recent, char1_thumb_kana)` = **P(X | 直近文脈)**
  — 候補A の 2 文字目は**まったく見ていない**。`char2_single_kana` はそもそも計算されていない。
- `score_b = frequency_score(recent + [char1_single_kana], char2_thumb_kana)` = **P(Z | 文脈, Y)**
  — 候補B の 1 文字目 Y 自体の尤度は見ておらず、条件付けの深さだけが 1 段深い。

**同長比較でも対称比較でもない。** 事後訂正が使うスコアはこれとは**別物**である。

#### 事後訂正が使う新スコア

E7 が満たされているとき、両世界のかな列は同じ長さになる（E7 が課した制約の帰結であって
偶然の性質ではない）:

- 候補A: `[X, Y₂, s₁ … s_{k-1}]`
- 候補B: `[Y, Z,  s₁ … s_{k-1}]`

```
score(C) = Σ_i log P(c_i | ctx + c_1 … c_{i-1})
```

**未知項の扱い**: `NgramModel::frequency_score`（`src/ngram.rs:247-265`）は
trigram → bigram → `0.0`（未知＝中立センチネル）の 3 段フォールバック。異なるスケールの項を
足すと、項数が増えるほど「モデルが一度も見たことのない候補」が「見たことがあって稀と判定された
候補」より系統的に有利になる。最も保守的な回答として、**両候補のすべての項が trigram または
bigram にヒットすること（`0.0` センチネルを 1 つも含まないこと）を適格条件にする**。
放棄は常に安全なので、この厳しさは安全側にしか働かない。
（`NGRAM_CONTEXT_SIZE = 3` だが `frequency_score` は `recent` の末尾 2 要素しか読まないため
3 要素目は現状使われていない。本 ADR はこの挙動を変更しない。）

**ヒステリシス**: `score_retro(候補A) − score_retro(候補B) > δ_flip` のときだけ覆す。
`δ_flip > 0` は必須（同点で覆さない／振動を作らない）。値は Phase 0b の分布から決める。

**k=0 基準の併記**: shadow は必ず 3 値を記録する——元の決定 / k=0 の新スコアによる再評価 /
k=1(2) の新スコアによる再評価。`flip(k=0)` は**スコア関数を差し替えたことだけ**による不一致、
`flip(k) − flip(k=0)` が**後続文脈の情報利得**である。この分解なしに反転率を見ても、
「測りたいもの」と「測りたくないもの」が混ざったままになる。

### 決定0b（判断点1 通過後のみ）: shadow 評価

決定2 の適格判定（E1〜E9）と決定8 の再評価器を実装し、訂正は行わずに記録する。

| # | 測る値 | 用途 |
|---|---|---|
| 3a | k=0 再評価と元の決定の不一致率 | スコア関数差し替え由来（測りたくない量） |
| 3b | k=1 / k=2 再評価の不一致率 | `3b − 3a` が後続文脈の情報利得（測りたい量） |
| 5 | 適格性を失った理由の内訳（E1〜E9 のどれで落ちたか） | 実用適格率（上界は判断点1 で既に判明している） |
| 6 | span の m 分布と、訂正着弾までの打鍵数 | m 上限・窓の妥当性確認 |

**判断点2 の棄却ゲート**: 適格率が Y% 未満なら棄却クローズ。`3b − 3a` が実質ゼロ
（反転はスコア関数の差し替えで説明でき、後続文脈は寄与していない）なら本 ADR の前提そのものが
否定されたことになるので棄却クローズ。

### 棄却した代案

- **遅延確定（lookahead buffering、BS を使わない案）**: 曖昧決定の出力を送出せず、後続かなが
  揃うまで内部バッファに保持してから確定する。**棄却理由は体感レイテンシ単独**——曖昧決定の
  後続 1〜2 かなが確定するまで画面に何も出ないため、速く打っているときだけ画面が固まるという、
  体感上もっとも目立つ場所にレイテンシを置くことになる。
  （設計過程では「BS 方式は既存経路を一切変えないから安全」という理由も挙げたが、
  決定3 のとおり成り立たないので撤回した。留保: 本 ADR の BS 方式も適格条件次第では
  体感されうる。Phase 0b のデータ次第で再浮上する余地は残す。）
- **`KeyAction::RetractGroup(m)` の新設**: `KeyAction::` の参照は src 243 / awase-windows 19 /
  awase-linux 11 / awase-macos 9 の計 282 箇所あり、`KeyAction::romaji()` の非網羅性
  （ADR-115 決定6）・`drain_pending_releases_as_keyups` の match・`flatten_actions`・
  `Sequence` への入れ子可否（ADR-115 決定4）に波及する。加えて `SpecialKey::Backspace` という
  既存の BS 抽象と意味論が二重化し、`.yab` の `後` セルとも並立してしまう。
  `InputEffect` に 1 バリアント足す方（本番 match は 6 箇所）が波及が小さい。
- **`send_keys` の戻り値変更 / 新規逆流 `Effect`**: 決定6b の理由により棄却。
- **1 打鍵遅延 commit**: 実現不能（`record` は即時適用）かつ乖離の頻度 × 向きで劣る（決定6b）。
- **`ConfirmMode` への新バリアント追加**: 決定7 の理由により棄却。
- **訂正専用タイマーによる非同期訂正**: 決定3 の理由により棄却。
- **重なり時間ベースの適格判定**: `min_overlap_margin_percent = 0` が ADR-112 決定3 で
  恒久固定されているため no-op に退化する（BUG-105 で同じ理由により棄却済み）。
- **2 鍵ケース（`confirms_char_thumb_chord`）への適用**: 既定 `min_overlap_margin_percent = 0`
  の下では `overlap_only_verdict` が常に `Some(true)` を即 return し n-gram タイブレークに
  到達しないため「曖昧決定」が定義上発生しない。スコープ外とし、
  `min_overlap_margin_percent` を将来引き上げる別 ADR とセットで再検討する。

## 段階投入

| Phase | 内容 | 進む条件 |
|---|---|---|
| **0a** | 決定0a（カウンタのみ、新設計ゼロ、実運用は無変化）。項目 2b/2c を含む。決定0a-report（不具合報告=ADR-095への統合、schema_versionは据え置き新フィールドはoptional受理）を同時に実装しなければ判断点1 のデータが手元に届かない | **本 ADR で承認済み** |
| 0a ソーク | 実環境で数日〜数週。ユーザーからの不具合報告（`attach_retro_eval_stats`、既定ON）経由でデータを回収する | — |
| **判断点1** | (i) 母数ゲート、(ii) 対照群付きユーザー訂正相関ゲート、(iii) E1+E2+E7+E9 の適格率上界ゲート | 通らなければ**棄却クローズ** |
| **0b** | 決定2（E1〜E9）+ 決定8 の再評価器を実装し shadow 記録（訂正はしない） | 判断点1 通過 |
| **判断点2** | 適格率ゲート、`3b − 3a` が実質ゼロでないこと | 通らなければ**棄却クローズ** |
| 1 | 決定3〜6 の実装（`RewriteTail`・`InputEffect::SendKeysWithRetract`・`unicode_cold_defer_active()`）。既定 `off`、`on` は手動 opt-in | 判断点2 通過 + **実機検証** |
| 1 ソーク | 作者自身が `on` で日常利用（下記の撤退条件つき） | — |
| 2 | 既定値の変更は行わない（必要になったら別 ADR） | — |

**Phase 1 の実機検証ゲート**: 「1 `Char` = 1 BS」が GJI / MS-IME × composition 中 / 確定後 ×
`InjectionMode`（Romaji / Unicode）で成立することを実機で確認する。E3′ により
「1 エントリ = 1 かな = 1 完全ローマ字チャンク」に限定してあるので競合する 2 モデルは
ここで一致するはずだが、**IME 側の BS 意味論そのものは未検証**である。

**Phase 1 ソークの撤退条件**（`.claude/rules/experiment-logging.md` と整合）:
以下のいずれかを 1 件でも観測したら、**即座に `on` を撤回し、本 ADR を棄却クローズする**。
撤回コミットの本文には実験ログ規約どおり **アプリ・IME・再現手順**を書き、
`docs/experiments.md` にも 1 行追記する。

- **意図しないテキスト削除**: 訂正の BS が span 外の文字（確定済みテキスト、別の入力欄、
  別の位置）を削った事象。
- **訂正の振動**: 同一入力中に訂正が連鎖して打鍵が不安定になる事象（限界4 のフィードバックが
  実在した証拠）。
- **文字消失**: 訂正バッチの一部（BS だけ着弾して replay が来ない等）が観測された事象。

「頻度が低いから様子を見る」は取らない——限界1 の性質上、1 件観測されたということは
再現条件が実在するということであり、`retract_and_replace` の既知の実害を
より大きい規模で再生産している。

各 Phase は独立に `git revert` 可能。

## テスト方針（`.claude/rules/fix-requires-evidence.md` 対応）

- **golden**: BUG-105 の report `01M1GDQVBET5DBX3MY4BRGQFW1` の実測タイムスタンプ列に
  後続かなを足したシナリオを `tests/scenarios.rs`（`Engine::on_input` 経由・実レイアウト
  `layout/nicola.yab`）に追加し、`on` で訂正バッチが出ること／`off` で PR #141 後の出力と
  1 bit も変わらないことの両方を固定する。
- **INV-120-1（最重要）**: 決定4 の全破棄トリガについて、破棄後の
  `committed` / `pending_releases` / FSM 状態 / 出力列が `retro_ngram_correction = "off"` の
  実行と一致することをプロパティテスト（`src/engine/proptest_tests.rs`）で固定する。
- **INV-120-2**: 訂正発火後も `thumb_consumed`・solo カウンタ・`state` が候補B のままである
  ことを固定する。
- **E8 の不変条件**: (a) 訂正が `commit_char1_output` を通る経路に相乗りしないこと、
  (b) **`timeout_pending_char` 等のタイムアウト flush に訂正が相乗りしないこと**
  （決定3 の「非同期 BS が飛ばない」主張はこれに支えられている）、
  (c) `RewriteTail.retract` が数える `committed` 末尾が常に `[char1, char2, …]` の順であること。
- **E3′/E9**: `kana` が `None` のセル、および `kana` は `Some` だがひらがなでないセル
  （`Literal("、")` 型）が span または X・Y₂ に現れたら訂正しないことを固定する。
  **`layout/nicola.yab` の `後`/`逃` セルを使ったケース**を明示的に含める。
- **双条件**: `lookup_face` の `kana.is_some()` と返り値 `KeyAction` が `Char` であることが
  一致することを、全 `YabValue` バリアントに対して固定する。E3′/E9 と Phase 0a 項目 2b が
  この双条件に依存しているため、将来 `From<&YabValue>` が変わったら気づけるようにする。
- **バッチ性**: `ReduceAndContinue` の継続処理を含めた最終 `Response.actions` の並びが
  `[BS…, Char…, （継続分）]` になること、継続分が Char の後に非 defer 対象を積むケースを
  検出できることを固定する。
- **決定8**: 未知項（`0.0`）を含む候補が適格外になること、`k=0` と `k=1` の評価が別々に
  記録されることを固定する。
- **journal replay**: 既存のリプレイ資産は `crates/awase-windows/tests/journals/` にあり、
  MVP の対象は `state/conv_classify.rs::classify_conv_transition` のみ、フィクスチャ型は
  `ConvClassifyFixture` である。**core（`src/`）のタイミング/n-gram 判定用のフィクスチャ形式は
  存在しない**ため、本 ADR のリプレイテストは既存資産の流用ではなく**新規のフレームワーク拡張**で
  あり、そのコストを Phase 0b に計上する。
- Linux で `cargo test` から動くことを必須とする（core のみの機能なので可能）。

## 既知の限界 / 承知のうえで受け入れる残存リスク

重い順。

1. **【既知の実害】BS が確定済み文字を余計に消す。** 「未検証の前提」ではなく既に観測・
   文書化されている: `docs/known-bugs.md:12498-12504`（ADR-115 の既知の限界）
   「閾値内に親指キーが来ると `retract_and_replace` が BACKSPACE 1 発で取り消そうとするが、
   **IME 確定は BS で戻らないため確定済み文字を 1 つ余分に消す**」。
   本 ADR の訂正窓は 2 打鍵分（k=1）で `retract_and_replace` より一桁長く、その間に IME が
   確定する機会は桁で大きい。span 途中で確定が起きていた場合、m 発の BS のうち何発が
   composition を戻し何発が確定テキストを削るかは事前に分からない。決定3 が BS を先頭に
   まとめる構成は送信順序の安全性の話であって、BS の意味論的ハザードを減らさない——
   1 発目で composition が崩壊すれば残り m−1 発は確定テキストに落ちる。
   **緩和は m の上限 3（既定 2）・手動 opt-in・Phase 1 の撤退条件だけであり、ゼロにはできない。**
   加えて、訂正バッチが GJI long-cold 時に defer 経路へ落ちると **m かな全部を失う**
   可能性がある（決定6b の送信時ゲートで落とすべきケース）。
2. **【検出不能】マウスによるキャレット移動を観測できない。** マウスフックは存在せず、
   同一ウィンドウ内のクリックはフォーカス変更でもないため決定6a の bool でも捕まらない。
   訂正窓（k=1 で曖昧決定から 2 打鍵分）が開いている間にユーザーがクリックしてキャレットを
   移動し次の文字を打つと、m 発の BS が**まったく別の場所のテキストを削る**。
   `retract_and_replace` も同じ露出を持つが窓が数十 ms・BS=1 である。
   低レベルマウスフックの追加は理論上の緩和策だが、スコープ・コスト・別種の不安定性
   （フック追加は本リポジトリで繰り返し事故源になっている）から採らない。
   **「頻度が下がる」ではなく「検出不能な残存リスク」である。**
3. **`committed` はユーザーの BACKSPACE で縮まない。** `handle_bypass` は `remove_by_scan`
   （`pending_releases` のみ）を呼ぶだけ。決定4 が「ユーザーの BS は窓を破棄する」ことでしか
   回避できず、キーボード以外の経路（限界2）には効かない。
4. **【力学的限界】shadow の軌道と `on` の軌道は一致しない。** 訂正が発火すると `committed` が
   書き換わり、`recent_kana(3)` として `TimingJudge` に渡り、Phase 2 スコアだけでなく
   **`adjusted_threshold` → `is_simultaneous`（同時打鍵の閾値そのもの）**に影響する。
   1 回の訂正が以後の打鍵の「同時打鍵と見なすか」を変える。shadow は訂正しないので、
   shadow が測る反転率は「2 回目以降の訂正が起きる世界」の反転率と一致しない。
   Phase 1 のソーク（撤退条件つき）で振動を監視する以外に事前検証の方法がない。
   **同じ理由で、限界5 の乖離は「有界」であっても「無害」ではない**——解消までの 3 かなの
   あいだに新しい曖昧決定が起きれば、その決定は画面に無い候補A の文脈で採点される。
5. **送信時ゲートが訂正を落とした場合、`committed` が画面より進む。** 画面は候補B、
   `committed` は候補A になる。この乖離は有界で、`recent_kana(3)` が末尾 3 件しか読まないため
   以後 3 かなの打鍵で自然に解消する。6a を保守的にしてあるので例外自体が稀である
   （「1 打鍵遅延 commit」案は訂正成功のたびに乖離していたため、頻度 × 向きの両面で劣る）。
   ただし限界4 のとおり、解消までの間に起きた曖昧決定は誤った文脈で採点される。
6. **【統計的限界】shadow は「不一致率」しか測れず「正解率」を測れない。** 正解データが
   存在しない。だからこそ棄却ゲートを反転率ではなく、対照群を持つユーザー訂正相関
   （Phase 0a 項目 7）と母数・適格率上界に置いた。
7. **訂正自体が誤りうる。** 正しかった出力を BS で壊す可能性はゼロにならない。
   δ_flip・未知項ゼロ要求・E1〜E9 は頻度を下げるだけである。既定 `off` と kill switch が
   唯一の実効的な備え。
8. **適格率が実用に足りない可能性。** E7・E9・未知項ゼロ要求はいずれも適格率を下げる方向に
   働き、特に **E7 は適格率を 1/3 程度に落とす支配項になりうる**（決定0a の見立て）。
   **その上界は判断点1 で分かる**（決定2/決定8 を 1 行も書かずに棄却できる）。
   低ければ**棄却クローズが妥当な結末**であり、条件を緩める余地は今から書かない。
9. **訂正は出力履歴だけを直し、FSM 状態は候補B のまま**（INV-120-2）。
   `consume_thumb` の記録も solo カウンタも巻き戻らない。
10. **同長比較は E7 が課した制約の帰結であり、一般には成立しない。** 2 鍵ケースや将来の
    他の曖昧クラスへ拡張するときは長さの異なる系列比較が復活し、長さ正規化という
    別の設計問題が発生する。本 ADR の枠組みをそのまま流用しないこと。
11. **視覚的なちらつき。** 2〜3 かなが消えて打ち直される瞬間はユーザーに見える。
12. **IME の学習・予測変換への副作用は未検証。**
13. **`kana` が `None` のセル（拗音の多かな `Romaji`、記号の `KeySequence`、ADR-115 の打鍵列、
    `後`/`逃` のような `SpecialKey` セル）と、ひらがなでない `Literal` は恒久的に対象外**
    （E3′/E9）。独自 `.yab` で拗音を 1 セルに割り当てた構成は本機能の対象にならない。
14. **本 ADR は Phase 1 の限界（BUG-105 の「既知の限界」＝ release 時刻を見ない問題）を
    解かない。** E1 で Phase 1 を対象外にしているため、「重なりは乏しいが d1 はたまたま
    短い」ケースは事後訂正でも救われない。その解決は重なり時間の実測を伴う別 ADR の仕事。

## 別途起票すべきもの（本 ADR のスコープ外）

- **独自 `.yab` で拗音 romaji を 1 セルに書き、かつ投機モードを使うと、既存の
  `retract_and_replace` が BS 1 発で 2 かなを消し切れない**（`kana: None` →
  `KeyAction::Romaji` 経路）。出荷 5 レイアウト（nicola / nicola_f / nicola_kb232 /
  nicola_keytop / nicola_us）には拗音 romaji が存在しないため実運用の実害ではないが、
  `docs/known-bugs.md` に別エントリとして起票するのが筋である。

## Premortem の経緯

Opus 敵対的 premortem を 4 ラウンド回して収束した。**各ラウンドで、設計の中核前提が
実コードと食い違っていることが 1 つずつ見つかった**——この記録自体が、
本 ADR の限界一覧をどれだけ信用してよいかの目安になる。

**ラウンド1（初稿）→ 24 指摘**: 中核前提 2 つが破綻。(a) 「3 鍵仲裁の 2 択は同長のかな列
比較に落ちるので正規化問題が起きない（幸運な性質）」は誤りで、現行 Phase 2 のスコアは
非対称（`score_a` は 1 かなのみ、`score_b` は `char2_single_kana` を計算すらしない）。
(b) 「BS 数はかな数から数えるべき」という前提は、既存コードが明文で採用する
「1 完全ローマ字 = 1 BS」モデルと衝突し、どちらも未検証だった。加えて
「Phase 0 は計装だけ」という主張が、実際には決定2/決定8 の全実装を含んでいた。
→ k=0 基準の導入、Phase 0a/0b の分割、E5 削除、E7 強化、E8 新設で対応。

**ラウンド2 → 8 指摘**: **showstopper**——ラウンド1 で導入した適格述語
（`kana_len_for_romaji(entry.romaji)` が `Some(1)`）が実装不能で、しかも**逆を向いていた**。
出荷レイアウトの全かなセルは `KeyAction::Char` になり `OutputEntry.romaji` は `""` なので、
この述語は通常のかな出力を全部弾き、拗音（多かな）だけを通す——**訂正が一度も発火しない**
設計だった。さらに shadow の適格率観測が「0%」を返し、設計の当否と無関係な理由で
棄却クローズを出すところだった。候補A の仮想出力 X・Y₂ がどの適格条件も通っていないこと、
platform → core の逆流チャネルが存在しないことも判明。
→ E3′ への統合（`KeyAction::Char` かつひらがな）、E9 新設、決定6b の書き直しで対応。

**ラウンド3 → 7 指摘**: ラウンド2 の代替として導入した「1 打鍵遅延 commit」が、
`record` は `on_reduce` により parse ループ内で**即座に**適用されるという実装事実と矛盾していた
（`send_keys` より遥かに前に適用される）。また「乖離の量は同じ」という自己弁護に対し、
比較軸は**頻度 × 向き**であり遅延 commit 案が劣ると指摘された。
→ eager 適用へ戻し、E8 の「deferred」という誤った語を正し、双条件による E3′/E9 の簡約と
Phase 0a 項目 2b の追加で対応。

**ラウンド4 → 3 指摘（軽微）**: E7 充足率も Phase 0a で測れる（項目 2c）・
awase-linux は `execute_effects` が `InputEffect` を直接 match しているため 1 arm 追加が
必要・限界5 の乖離は「有界」だが「無害」ではない、の 3 点を反映して収束。

**決定0a-report（2026-09-02 追記、2026-09-02 premortem 1ラウンドで修正済み）**:
「Phase 0a のカウンタをどう取り出すか」が未規定だったとユーザー指摘を受け追記し、
その後ユーザーの依頼で単独 premortem を実施した。

- **【重大】schema_version を 3→4 に上げる初稿案は、この決定の目的そのものを
  損なう欠陥だった**: Worker（`services/report-worker/src/index.ts:179-180`）は
  `schema_version` を厳密等値で検証するため、版を上げた瞬間まだ更新していない
  全ユーザーの不具合報告（retro 統計と無関係なものも含む）が拒否される。
  「ユーザー環境から実測データを回収する」という目的と正反対の効果になるため、
  schema_version は据え置き、新フィールドを optional 受理に変更した。
- 項目4・項目7 を単一の sum/count 対で持つ初稿案も、決定0a 自身が明記した
  「N・retro_window_ms は実測後に決める」を事後に実現できない設計だったため、
  固定バケットのヒストグラムに変更した。
- そのほか `BugReportDiagnostics` への `attach_*` 追加の誤り、`Engine` への
  委譲が実際には `NicolaFsm`→`FsmAdapter`→`Engine` の3段であること、
  「実運用の挙動は無変化」の主張範囲（IME 出力は無変化／コードはホットパスに
  軽微な増分あり）を精緻化し、実収集率が100%でないこと（実報告に添付falseの
  実例あり）を明記した。
- 「`retro_ngram_correction` の設定値に関わらず常時集計する」という設計判断
  自体は premortem で妥当と確認された（既に無条件で起きている観測にすぎず、
  観測をゲートすると Phase 0a の趣旨が崩れるため）。

## 参照

- GitHub issue #140、`docs/known-bugs.md` BUG-105、PR #141（squash `1045a05e`）
- `docs/adr/112-keyup-lifecycle-fsm-delivery.md`、`docs/adr/019-platform-independence.md`、
  `docs/layer-boundaries.md`
- `docs/adr/095-tray-bug-report-cloudflare-intake.md`（決定0a-report の統合先。
  決定3/B-5 の allowlist 原則、決定4/決定9 の attach トグル+送信前プレビュー原則）、
  `crates/awase-windows/src/bug_report.rs:28,111,151,162-168,178-200,203,213-223,239`
  （`SCHEMA_VERSION`、`BugReportStateSnapshot`、`BugReportPayload`、
  `BugReportDiagnostics`とその手書き`Default`、`BugReportInput`、
  `build_payload_with_log_budget`）、
  `crates/awase-windows/src/runtime/message_handlers.rs:1182`
  （`current_bug_report_diagnostics`、既存フィールドの構築パターン）、
  `services/report-worker/src/index.ts:6,179-180,199-223`
  （`SCHEMA_VERSION`の厳密等値検証、`optionalNullableString`等による
  旧クライアント互換の既存前例、`state_snapshot_requires_attach_state_snapshot`
  等のクロスフィールド整合ガード）、
  `src/engine/engine.rs:52-53`・`src/engine/fsm_adapter.rs:19-28`
  （`Engine`→`FsmAdapter`→`NicolaFsm`の3段構造）、
  `docs/bug-reports-triage.md:42`（`attach_*`をfalseにして送信された実例、
  実収集率が100%でないことの根拠）
- `docs/adr/115-yab-keystroke-sequence.md`（`Sequence`/`CtrlChord`。
  `docs/known-bugs.md:12498-12504` の「BS が確定済み文字を 1 つ余分に消す」記録もここ）
- 実装の根拠となる位置:
  `src/engine/timing.rs:113-172`（Phase 1/2 の実際のスコア）、
  `src/engine/nicola_fsm.rs:73-101`（`impl From<&YabValue> for KeyAction`）、
  `src/engine/nicola_fsm.rs:789-796`（`lookup_face`。73-101 と合わせて kana ⟺ `Char` の双条件）、
  `src/engine/nicola_fsm.rs:977-979`（`on_reduce` → `update_history`）、
  `src/engine/nicola_fsm.rs:2125-2129`（タイムアウトが eager）、
  `src/types.rs:286-292`（`KeyAction::romaji()` の非網羅）、
  `src/engine/fsm_types.rs:301-308`（`OutputUpdate::record` の romaji 充填）、
  `src/engine/output_history.rs:89-99`（1 romaji = 1 BS の既存モデル）、
  `src/platform.rs:244`（`send_keys` の戻り値）、
  `src/platform.rs:279,308,323,328,333,338,346`（`PlatformRuntime`/`TsfComposition` の
  デフォルト実装前例 7 件）、
  `src/engine/decision.rs:26-31, 87-92, 272-278`（`InputEffect` 片方向、`InputContext` 設計ルール）、
  `crates/timed-fsm/src/parser.rs:376-401`（`ReduceAndContinue` の actions 集約、
  `on_reduce` の即時適用）、
  `crates/awase-windows/src/runtime/executor.rs:737`（`InputEffect` → `send_keys` の
  ディスパッチ点）、
  `crates/awase-linux/src/output.rs:238`（`execute_effects` が `InputEffect` を直接 match）、
  `crates/awase-windows/src/platform.rs:1023-1043`（`needs_unicode_cold_warmup` の 3 項）、
  `crates/awase-windows/src/scanmap.rs:70-71`・`hook.rs:36-57`（`後`/`逃` が Char クラス配列キー）、
  `src/ngram.rs:247-265`（3 段フォールバック）
- `docs/experiments.md` エントリ 01、BUG-74 / BUG-75（PR #103 → #104 の revert）
- `.claude/rules/tuning-constants.md`、`.claude/rules/fix-requires-evidence.md`、
  `.claude/rules/experiment-logging.md`
- 2026-08-30 Opus 敵対的議論（n-gram 既定化・適応しきい値の先送り結論）
