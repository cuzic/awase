# ADR-081: IME 制御ロジックをプロファイル別 capability 駆動ドライバへ分離し、汎用ループの分岐面を止める

## ステータス

Phase 1a/1b/1c 試験実装済み（2026-07-25、未配線・Linux 検証済み、詳細は
「Phase 1a/1b/1c 実施記録」節）。Phase 1 計画自体は Claude Fable 5 との壁打ちから
起票・確定（2026-07-25、Phase 0 は PR [#31](https://github.com/cuzic/awase/pull/31)
で Limited Go、Phase 1 計画は Opus による立案 + Fable との壁打ちで未確定点を解消
し確定）。

Phase 0（第一歩）は実施済み（PR #31）。Phase 0.5 は ADR-082 側で先行実施済み
（`JournalEntry::ImeActuation` + `EventOrigin` 配線、ADR-082「Phase 0.5 実施記録」
節参照）。Phase 1a（`Imm32UnavailableDriver`）/1b（`TsfNativeDriver`）/1c
（ドライバレジストリ + contract test 5件）は試験実装済みだが**ランタイムには
未配線**（`AppImePolicy`/`ime_controller.rs` の既存経路が引き続き使われている）。
Phase 1d（実機ソーク必須の strangler-fig 配線）・1e（旧経路撤去）は未着手 ——
このサンドボックスには Windows 実機（wine）が無く実行できないため、次に Windows
実機セッションが取れたタイミングで着手すること。詳細は「Phase 1a/1b/1c 実施記録」
節の「Phase 1d への申し送り」を参照。

### 追記（2026-08-12）: Phase 1c の一部を ADR-089 Phase B が引き取った

[ADR-089](089-ime-typestate-and-capability-const-table.md) §2.4（INV-42/43）が
**`GjiFsm` 同期義務の宣言軸を profile 軸から outcome 軸へ移した**ことにより、
Phase 1c の次の 3 つが根拠を失い、ADR-089 Phase B（§6 item 8、§4.7）で
**撤去**された:

| 撤去したもの | 引き取り先 | 理由 |
|---|---|---|
| `ImeProfileDriver::uses_gji_direct()`（trait メソッド + 3 impl） | — | 同期義務が profile 軸で決まらないことが確定したため、宣言する対象が無くなった |
| `GjiDirectAccess` token / `GjiDirectMechanism::access_for` / `GjiDirectMechanism::actuate` / `GjiActuation`（不変条件5） | ADR-089 INV-42（`legacy_gji_sync_obligation` が唯一の導出式） | token でアクセスを絞る前提（「`uses_gji_direct()` を宣言したドライバだけが同期義務を得る」）が消えた |
| contract test 不変条件4（belief を actuate 抜きで ON にする高速パスは必ず `GjiFsm` を同期させる） | ADR-089 INV-43（`ActuationReceipt` の `#[must_use]` + `Drop` の `debug_assert`） | **保証水準は「debug ビルドでの実行時検出」までに下がる**（ADR-089 §8.1）。落とす前提として ADR-089 §7 の compile-fail ケース4 が実際に赤くなることを確認済み（`state/gji_direct_mechanism.rs` の `compile_fail` doctest） |

**Phase 1c 不変条件1・2・3 は撤去していない**（ADR-089 は引き取らない）:

- 不変条件1（IME-ON 経路と stale `ObservedEisu` 救済の対）— `has_ime_on_path` /
  `stale_eisu_recovery_paired` として `ImeProfileDriver` に残っている。
- 不変条件2（`owns_physical_kanji==true` のドライバは物理 KANJI を漏らさない）—
  ADR-089 §2.5 のとおり `AppImePolicy` 側に残る軸であり、対象外。
- 不変条件3（`Blind` give-up 後に observation を書かない）— `default_feedback` /
  `decide_actuation_action` として残っている。

**Phase 1d/1e の位置づけ**: ADR-089 §6 は Phase 1d の**凍結を提案**しているが、
その採否は未決定（ADR-089 §9-4）。本追記は「同期義務の宣言軸」という 1 点に
ついてのみ、実装が ADR-089 側へ移ったことを記録するものである。
`ImeProfileDriver` 自体（`owns_physical_kanji` / `has_ime_on_path` /
`stale_eisu_recovery_paired` / `default_feedback` / `ime_open_mechanism` /
`probe_budget_ms`）と `driver_for` レジストリは**残っている**。

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

---

## Phase 0 実施記録（2026-07-25、検証専用・コード変更なしの調査 + 限定的な Go 判断後の試験実装）

本節は上記「決定」「第一歩」で定義された Phase 0 の実施結果を追記するものであり、
上記の提案本文（「ステータス」〜「不変条件」まで）は一切変更していない。

### 1. 定量調査: `docs/known-bugs.md` 43件の分類

対象は本ブランチ（`worktree-agent-a39c002e32c77fc87`、`main` の祖先コミット
`317d8cf` から分岐）に存在する BUG-01〜BUG-41（41件）に加え、`main`/
`integration/unmerged-branches-verify` 側にのみ存在する BUG-42・BUG-43
（`git show 18b8bc7:docs/known-bugs.md` で読み取り専用に参照、本ブランチへは
未マージ）を合わせた43件全件。分類基準は本 ADR 冒頭の3分類:

- **(a) cross-profile spillover**: ある profile/app 向けの修正・最適化が、
  別の profile/app に副作用を与えた（または与えうる構造を持っていた）事例。
- **(b) single-profile**: 単一の profile/app に症状・原因・修正がすべて閉じていた事例。
- **(c) cross-cutting infra**: `AppImeProfile`/`ImePolicyProfile` の分岐そのものとは
  無関係な、汎用インフラ（スレッド配送・タイマー・hook 状態・検出ヒューリスティック等）の欠陥。

| BUG | 分類 | 根拠（本文からの一文引用） |
|---|---|---|
| 01 | b | 「WezTerm は TSF native app。F2 (VK_DBE_HIRAGANA) 受信後、TSF composition context の初期化に実測 ~300–936ms かかることがある。」 |
| 02 | b | 「Chrome は F2 受信後に composition context を非同期初期化する。」 |
| 03 | c | 「Chrome 以外のアプリでも同様の GJI SHOW タイミング問題が起きる可能性がある。」（`LiteralDetector` は全 profile 共有） |
| 04 | c | 「GJI モニタースレッドが切断（gji_monitor_ok=false）している場合、probe は GJI 観測を行わず…フォールバックに移行する。」（profile 非依存の GJI 監視インフラ） |
| 05 | c | 「composition context が時間経過でいつ無効化されるか Windows API から通知されないため、保守的な固定値 2000ms を閾値として設定している。」 |
| 06 | c | u32 オーバーフローという型設計上の一般論。ADR-069 リファクタで解消済み。 |
| 07 | **a** | 「実観測経路を持たない Imm32Unavailable でのみ Low が belief を支配する」— Win+X 対策（`ce45b82`、非TSF全般向けの一般修正）が Imm32Unavailable だけに実害を出した。 |
| 08 | **a** | 「復元書き込みが conv を 0x19⇄0x09 で往復させ…直接入力中の spurious Engine ON + IME ON を実機で引き起こした」— GJI×TsfNative 向けの復元ロジックが steady-state まで広がり MS-IME×TsfNative を壊した（撤回）。 |
| 09 | c | 「hwnd=NULL の `PostMessageW` は…呼び出しスレッド自身への `PostThreadMessage` と等価」— profile 非依存のスレッド配送バグ。 |
| 10 | **a** | 「MsImeStrategy は `needs_f2_probe()=false` で F2 warmup を送らない…『消すが代わりを送らない』食い逃げになり」— GJI 戦略専用の Suppress 契約が MS-IME 戦略の入力を食い潰した。 |
| 11 | c | 「(pid, class) 粒度でキャッシュした瞬間にウィンドウ全体へ固着する」— UIA focus 分類インフラの構造的欠陥（profile 分岐と無関係）。 |
| 12 | c | 「配送修正の副作用として発症」— BUG-09（インフラ）修正が露出させた別のインフラ潜在バグ。 |
| 13 | b | MS-IME/TSF 戦略専用の cold-start 保護追加、他戦略へは波及しない設計。 |
| 14 | **a** | 「導入直後から Windows Terminal × MS-IME で一切入力できなくなり撤回…hook 層で遮断すると IME の状態機械が壊れる」— BUG-08 の VK_KANA 限定 swallow を IME モードキー全般へ一般化した結果 MS-IME が機能停止。 |
| 15 | b | MS-IME/TSF・Shift/Eisu 専用。9回の追補すべて MS-IME 側に閉じる（GJI 対応は BUG-25 側で別展開）。 |
| 16 | c | 「Win キー押下中の IME キー注入スキップが Applied 扱いになり」— `send_ime_mode_key` という GjiDirect/MsImeDirect 共有関数のバグ、profile 分岐ロジック自体には起因しない。 |
| 17 | c | GJI CLSID ポーリングデバウンス。GJI を使う任意の profile に共通のインフラ欠陥。 |
| 18 | **a** | 「hard pre-sync はまさに『実 apply をスキップして belief だけ ON にする』ための経路なので、`gji_on_ime_on` が一度も呼ばれず…`GjiFsm` は `OffCold` のまま残留する」— Imm32Unavailable 専用の高速パスが全 profile 共有の `GjiFsm` を同期し忘れた。 |
| 19 | c | `ConvModeMgr`/`classify_conv_transition` という profile 非依存の共有 conv 分類ロジックの欠陥（トリガーは Chrome 特有のポップアップ flicker）。 |
| 20 | **a** | 「GJI/TsfNative（Windows Terminal・Chrome 等）では常に no-op になる…ON 方向には対称の実装が…あり…OFF 方向の対称実装が存在しなかった」— `can_use_imm32_cross_process()` で分岐する共有関数の片側だけ実装漏れ。 |
| 21 | c* | 「Chrome 側の復帰処理…が重症度情報を捨てて毎回 Long cold 相当の最重量パスを踏んでいた」— WezTerm 側は既に独立実装で対応済みだった＝**per-target に分離済みの実装が同期されず drift した**事例（*重複コスト側の証拠として2節で再言及）。 |
| 22 | **a** | 「`apply_hwnd_cache_restore`…が…鮮度・confidence チェックなしに…無条件適用していた」— profile 非依存の汎用キャッシュ復元が Imm32Unavailable 固有の `ObservedEisu` の脆さを踏み抜いた。 |
| 23 | c | ロック画面中の modifier KeyUp 消失。`hook.rs`/`PHYSICAL_KEY_STATE` は profile と無関係な低レベルキー状態管理。 |
| 24 | c | `is_partial_literal()` は全 TSF-native cold-start 経路が共有する検出ヒューリスティックの構造的欠陥。 |
| 25 | **a** | 「GJI という IME 種別そのものが、この単発 F0 注入を認識しない…無関係な standalone トグル用途へ転用しない」— MS-IME 向け warmup ヘルパーを GJI へ転用する試み（追補1〜3）が3回連続で実機失敗。 |
| 26 | c | `classify_conv_transition` の steady-state 分岐の非対称漏れ（profile 分岐ではなく conv mode 判定ロジック自体の欠落）。 |
| 27 | **a** | 「追補2…msedge で入力を全面破壊した」— Chrome 実機観測1件から作った backspace リカバリが msedge（同じ Imm32Unavailable、別アプリインスタンス）で無限バックスペースを誘発し撤回。 |
| 28 | c | `pending_gji_key_responses` の drain 漏れ。TSF probe インフラのバースト処理欠陥。 |
| 29 | c* | Chrome の per-VK confirm 検出漏れ。「TSF/WezTerm 側（`gji_coro_body` Phase 5b）に同型の検出漏れがあるか未確認」— Chrome/TSF が別実装のため**片方だけ**修正された（*重複コスト証拠）。 |
| 30 | c* | 「TSF の gji io 閾値無しがおかしいと思います。Chrome のバイト量の閾値にする、方向で統一してください」— **TSF 用と Chrome 用に分岐していた検出ロジックを1本化したことが直接の修正**（*重複が生んだドリフトを統一で解消した実例）。 |
| 31 | c* | 「`ConfirmKeyDown`…と同種の『warm を無条件に cold 化してはいけない』ガードが欠けていたまま」— 同じ原則が別のイベントハンドラ（`NativeF2Down`）に伝播しておらず再発（*同一原則の複数箇所実装漏れ）。 |
| 32 | c* | 「これは…`send_ime_mode_key` が BUG-16 追補で修正した欠陥と全く同型で…本関数には同種の修正が入っていなかった」— 同じ「スキップ≠Applied」原則が別関数に伝播していなかった（*BUG-16/31 と同型の反復）。 |
| 33 | c | Imm32Unavailable の drift correction 構造的不発火。共有 `check_drift_correction`/観測ストアの設計欠陥。 |
| 34 | c | `SendMessageTimeoutW(SMTO_ABORTIFHUNG)` の誤解。呼び出し箇所5箇所以上に及ぶ汎用同期呼び出しパターンの欠陥。 |
| 35 | c* | 「`await_vk_detection`…は `check_now` を経由せず独自に同じ epoch 比較を inline で再実装しており…猶予ロジックが移植されていなかった」— 同じ fencing ロジックが2箇所に分裂し片方だけ更新漏れ（*重複コスト証拠）。 |
| 36 | c | Chrome GJI reinit と backspace flush の実行順序未保証。インフラのタイミング欠陥。 |
| 37 | **a** | 「`Imm32Unavailable` hard pre-sync…が…`applied` を即座に再ロックしてしまい」— BUG-18 と同根、Imm32Unavailable 専用ショートカットが共有の訂正経路を握り潰す。 |
| 38 | c | `pending_deferred` flush 漏れ。TSF probe コーディネーターのインフラ欠陥。 |
| 39 | c | `literal_session_confirmed` のリセット漏れ。observer インフラの蓄積状態管理欠陥。 |
| 40 | c | `nc_for_plan` の dead-code 削除に伴うリグレッション。純関数のクリーンアップミス。 |
| 41 | c | Alt なりすましの KeyUp 状態持ち越し。IME profile と無関係な hook/modifier 状態バグ。 |
| 42 | c | EngineOn コンボの context-inactive 未対応、トレイの `SetForegroundWindow` 誤対象化。いずれも IME profile 分岐と無関係な engine/tray インフラ。 |
| 43 | **a** | 「同じ `ir_apply_drift_correction`（Blacklist/TsfNative パス）が observation store を更新しないため…無限再送する」— BUG-20/33 と同型、共有 drift-correction 関数の non-ImmCross 分岐の欠陥が3回目の再発。ADR-080 の `Actuation`/`FeedbackPolicy`（プロファイルごとに `Blind`/`Read` を型で強制）で根治され、本 ADR が拡張しようとしている方向性の直近の実例。 |

**集計:** (a) cross-profile spillover = **11件**（07, 08, 10, 14, 18, 20, 22, 25, 27, 37, 43）
／ (b) single-profile = **4件**（01, 02, 13, 15）／ (c) cross-cutting infra = **28件**
（残り。うち `*` を付した **6件**（21, 29, 30, 31, 32, 35）は、既に per-target/
per-strategy に分離されている実装同士が同期されず drift した「重複コストが実際に
バグを生んだ」事例）。

**解釈:** 「共有ループ＋プロファイル分岐」が実際に他プロファイルへ副作用を
波及させた事例は 43件中 11件（26%）で、本 ADR コンテキスト節が提示した4件
（07/10/... 相当）の存在は裏付けられた。一方で、43件中 65%（28件）は
プロファイル分岐そのものとは無関係な汎用インフラ欠陥であり、これらは
ドライバへ分離しても解決しない（trait 分離後もスレッド配送・タイマー・
検出ヒューリスティックのバグは同じ形で残る）。さらに重要な点として、
**既に部分的に分離されている実装（Chrome 用 vs TSF 用の probe コルーチン、
literal 検出の per-target 分岐等）が「重複コストの現実」として既に6件の
バグを生んでいる** — これは本 ADR が提案する「プロファイルごとに独立した
ドライバ」を進める際に、同種の drift が `ImeProfileDriver` 実装間でも
起こりうることを示す実証的な警告である。

### 2. 重複コストの見積り（型シグネチャレベル、実装は書かない）

#### 現状（共有）

`ImeOpenStrategy`（`ime_controller.rs`）は既に4戦略が個別 struct + `impl` に
分離されている（`ImmCrossProcessStrategy`/`GjiDirectStrategy`/
`MsImeDirectStrategy`/`KanjiToggleStrategy`、各30〜80行）。共有なのは
trait 定義（12行）と `ImeController::apply_iter`（優先順位ループ、約25行）
のみ。つまり ADR コンテキストが問題視する「フォールバックチェーンの
暗黙の優先順位」は主に `apply_iter` の走査順（配列の並び）に宿っており、
各戦略の実装自体は既に分離済みで重複コストは小さい。

`AppImePolicy`（`state/app_ime_policy.rs`）は対照的に、3プロファイル
（実質 `ImmCross`/`Imm32Unavailable`/`TsfNative`、`Plain`/`Unknown` は
`ImmCross` にフォールバック）を **1つの struct + `from_profile` の match 式**
（本体35行）で表現しており、こちらが ADR の主眼（「データだけ分岐、
ロジックは共有」）に一致する。

#### 仮に `ImeProfileDriver` trait + 3構造体（`ImmCrossDriver`/
`Imm32UnavailableDriver`/`TsfNativeDriver`）へ完全分離した場合の見積り

ADR 本文が各ドライバに持たせたいとする責務（cold-start probe 予算、
drift correction feedback 方針、IME OFF キー選択、focus settle 時間）を
trait メソッドとして書き出す。**注記:** `ColdReason`（`tsf/output.rs`）・
`ImeControlView`（`state/ime_decision_view.rs`）は本ブランチでは
`#[cfg(windows)]` 限定型であり、`FeedbackPolicy`/`Actuation`（ADR-080）は
本ブランチに未マージのため存在しない。`state/app_ime_policy.rs` が Linux で
テスト可能なのは、まさにこれら windows 限定型に依存せず `ImePolicyProfile`
（`state/ime_event.rs`、ungated）のような純粋な値だけを扱っているためである。
本タスクの制約（新規コードは `state/` に置き Linux でテスト可能にする）を
満たすには、trait のシグネチャも同様に windows 限定型を避ける必要がある:

```rust
pub trait ImeProfileDriver: Sync {
    /// 物理 KANJI を awase が所有するか（旧 AppImePolicy::owns_physical_kanji）
    fn owns_physical_kanji(&self) -> bool;
    /// フォーカス変更後の settle 待ち時間 (ms)
    fn focus_settle_ms(&self) -> u64;
    /// cold-start probe の探索予算 (ms)。ColdReason 実体の代わりに
    /// 「確定キー起因か」「long idle か」という2つの bool パラメータへ
    /// 分解することで windows 限定型への依存を避ける。
    fn probe_budget_ms(&self, is_confirm_key: bool, long_idle: bool) -> u64;
    /// IME を開く/閉じるときに送る VK（`awase::types::VkCode` は windows 非依存）
    fn ime_open_key(&self, open: bool) -> awase::types::VkCode;
}
```

`is_applicable`/`apply`（実際の Win32 API 呼び出しを伴う）は
`ImeControlView`/`ImeOpenOutcome` 経由の実行時配線が要るため Phase 1
（ランタイム配線）のスコープとし、Phase 0 の trait には含めない
（後述4節の試験実装もこの4メソッドのみ）。

上記 trait 定義自体は約20行（1箇所のみ、重複しない）。3構造体それぞれに
必要な「型としての枠」を見積もる:

| 要素 | 1ドライバあたり行数（概算） | 3ドライバ合計 |
|---|---|---|
| struct 定義 + module doc（現状 `from_profile` の match アーム内コメントを移設） | ~12行 | 36行 |
| `impl ImeProfileDriver for XxxDriver` ヘッダ + 全面移行時 7メソッド（Phase 0 の上記4メソッド + Phase 1 で追加する `is_applicable`/`apply` + `AppImePolicy::default_feedback` 相当の feedback 方針メソッド）のシグネチャ+ワンライナー本体 | ~7メソッド×3行=21行 | 63行 |
| `probe_budget_ms` の中身（現状 BUG-01 の表: ColdReason×long_idle → ms、4行程度のmatch） | ~4行 | 12行（実質は表の値そのものなので新規重複ではなく移設） |
| `#[cfg(test)]` ユニットテスト（各ドライバ最低2〜3件、struct 構築+ assert） | ~15行 | 45行 |
| **小計（3ドライバ分の「型の骨組み」）** | | **約156行** |

対して現状の `AppImePolicy`（struct 17行 + `from_profile` 35行 +
テスト6件・約40行 = 約92行）を置き換えることになるため、**純増分は
約60〜100行**（3ドライバ化した場合の骨組みのみ。ADR が明言する通り
`ImeOpenStrategy` の4戦略は既に分離済みのため実質的な追加重複はほぼ
発生しない）。

**この見積りの前提と限界:** 上記は「判断ロジックの型」のみで、cold-start
probe・warmup コルーチン本体（`tsf/warmup/*.rs`、数百〜千行規模）を
ドライバごとに複製することは意図していない（ADR 本文が明示するスコープ外）。
仮にそこまで複製すると、1節の `*` 印6件が示す通り、**per-target 実装が
静かに drift する具体的リスクが既に実証されている**ため、Phase 1 で
probe/warmup 本体まで複製する場合は本 ADR の「不変条件」に加えて
「ドライバ間の contract テスト（各ドライバが同じ入力パターンに対し
一貫した契約を満たすことを検証する共有テストスイート）」を必須にすべきである。

### 3. Go/No-Go 判断

**判断: 限定的 Go（ADR 本文が提案する「最小プロファイル1本の試験実装」の
範囲に限定）。**

**Go の理由:**
- cross-profile spillover は43件中11件（26%）と無視できない比率で実在し、
  うち BUG-20→BUG-43 の系譜（`ir_apply_drift_correction` の non-ImmCross
  分岐）は同一の共有関数が3世代にわたって再発しており、ADR-080 が
  Actuation/FeedbackPolicy という「型でプロファイルごとの方針を強制する」
  設計に踏み切って初めて根治した。これは本 ADR の狙い（capability を
  型で宣言し、共有ループの if/match を無くす）が実際に効いた直近の実例。
- `ImeOpenStrategy` の4戦略は既にほぼ分離済み（2節）であり、
  `AppImePolicy` → `ImeProfileDriver` への移行コストは見積り上60〜100行と
  小さい。低コストで検証できる。
- ADR 本文の第一歩（3節）自体が「Go なら ImmCross 1本を試験実装」と
  スコープを限定しており、全面移行を決め打ちしていない。

**No-Go 側に倒すべきでない理由（＝全面 Go でもない理由）:**
- 43件中28件（65%）はプロファイル分岐と無関係な汎用インフラ欠陥であり、
  ドライバ分離では解決しない。「IME 制御の不具合の主因はプロファイル
  分岐である」という前提そのものが、悉皆調査では過半数を占めない。
- `*` を付した6件（21, 29, 30, 31, 32, 35）は、**既に部分分離されている
  実装が同期を怠ってバグを生んだ**実例であり、「間違った抽象より重複の
  方が安い」という ADR 本文の仮説（適用しない範囲節）に対する直接の
  反証データでもある。BUG-30 の追補1は逆に「重複していた検出ロジックを
  統一したことで直った」実例であり、Phase 1 で probe/warmup 本体まで
  ドライバごとに複製する場合は、この drift リスクを契約テストで
  相殺する設計が伴わない限り、新しい類の regression を生産しかねない。

**結論として:** `AppImePolicy` → `ImeProfileDriver` という「判断ロジックの
型」レベルの分離は妥当性の見積りがついた（Go）。Phase 1 で cold-start
probe・warmup 本体・`ImeOpenStrategy` の呼び出し順序まで踏み込む場合は、
上記6件の drift 事例を踏まえ、ドライバ間の contract テストを不変条件に
追加することを Phase 1 提案時の必須検討事項とする。

### 4. 試験実装（Go 判定に基づく最小実装）

`ImeProfileDriver` trait と、`ImmCross` プロファイル向けの `ImmCrossDriver`
実装を `crates/awase-windows/src/state/ime_profile_driver.rs` に新規追加した
（既存ランタイムへの配線はしていない。ADR-080 Phase 1 の `state/ime_actuation.rs`
と同じ位置づけ）。scope は 2節の見積りに沿い、`focus_settle_ms`/
`owns_physical_kanji`/cold-start probe 予算（`ColdReason`/idle 種別からの
ms 決定）/IME OFF キー選択の4点に限定し、cold-start probe・warmup
コルーチン本体は複製していない。ユニットテストを Linux 上の
`cargo test -p awase-windows --lib` で実行可能な形で追加した。

---

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

---

## Phase 1a/1b/1c 実施記録（2026-07-25、未配線・Linux 検証済み）

上記「Phase 1 計画（確定）」の 1a/1b/1c を実装した。1d（実機ソーク必須のランタイム
配線）・1e（旧経路撤去）は未着手（このサンドボックスでは実行不可のため）。既存内容は
変更せず本節を追記した。

### 実装した内容

- **Phase 1a — `Imm32UnavailableDriver`**（`crates/awase-windows/src/state/ime_profile_driver.rs`）:
  Chrome/Edge/UWP 向け。`owns_physical_kanji=true` / `focus_settle_ms=500` /
  `default_feedback=Blind{max_attempts:5, backoff:DRIFT_CORRECTION_THRESHOLD_MS}` を
  `AppImePolicy::from_profile(Imm32Unavailable)` と parity。`ime_open_mechanism` は
  `SharedImeKeyDispatch`（具体 VK 選択はランタイム合成に委譲）。
- **Phase 1b — `TsfNativeDriver`**（同ファイル）: WezTerm/Windows Terminal 向け。
  `owns_physical_kanji=false`（TSF が KANJI を処理するため通す）/ `focus_settle_ms=200` /
  `default_feedback=Blind`。`probe_budget_ms` は既存 tuning SSOT
  （`MEDIUM_IDLE_PROBE_MS` / `WARMUP_GRACE_MS`、`Imm32Unavailable` は
  `CHROME_GJI_REINIT_CONFIRM_MS`）を参照し**新規タイミング定数を導入しない**
  （`.claude/rules/tuning-constants.md` の「実測なしのエスカレーション禁止」を尊重。
  `ColdReason` 解像度別のエスカレーションは windows-gated メソッドとして Phase 1d へ）。
- **Phase 1c — レジストリ**: `driver_for(ImePolicyProfile) -> &'static dyn ImeProfileDriver`
  と `ALL_DRIVERS`。`ImmCross`/`Plain`/`Unknown` は `ImmCrossDriver` へ集約
  （分類 enum `ImePolicyProfile` 自体は統合せず、driver へのマッピングのみ collapse）。
- **Phase 1c — contract test スイート（不変条件5件）**: `ime_profile_driver.rs` の
  `tests` に実装。1=IME-ON 経路ドライバの stale `ObservedEisu` 救済ペア宣言、
  2=`owns_physical_kanji` ドライバの非 KANJI 機構宣言、3=`Blind` give-up の有界終端
  （`decide_actuation_action` SSOT を各ドライバの `default_feedback` で駆動）、
  4=GJI IME-ON の `GjiFsmSync` 分離不能性、5=`uses_gji_direct` によるアクセスゲート。

### GJI 横断性設計（design B）をどう反映したか

「GJI 横断性の設計」節で確定した (B)（GJI 直接制御を全プロファイル横断の共有機構として
1箇所に集約し、ドライバは `uses_gji_direct()` を静的宣言するだけ）を型で表現した:

- 新設 `crates/awase-windows/src/state/gji_direct_mechanism.rs` に共有機構
  `GjiDirectMechanism` を1箇所だけ実装。各ドライバは GJI/MS-IME の**動的分岐を持たない**
  （`ImeOpenMechanism` は cross-process API か「共有キー委譲」かの2択のみを宣言）。
- **アクセスの排他性（適用条件2・不変条件5）**: 共有機構を呼ぶ capability token
  `GjiDirectAccess` はフィールド非公開で、唯一の公開コンストラクタ
  `GjiDirectMechanism::access_for` が `uses_gji_direct()==true` のドライバにのみ `Some` を
  返す。宣言しないドライバ（`ImmCrossDriver`）からは構造的に到達不可能。
- **同期義務の分離不能性（適用条件3・不変条件4）**: 作動要求の帰結 `GjiActuation` が
  `GjiFsmSync`（`OnImeOn`/`OnImeOff`）を内包し、同期義務を伴わずに GJI で IME を ON にする
  経路を型として提供しない（BUG-18/22 型の「actuate 抜き belief-ON 高速パス」を防ぐ）。
- **狭いインタフェース（適用条件1）**: 共有機構は状態を持たず、公開 API は
  `access_for` / `actuate` の2つのみ。送信 VK は既存 SSOT `ime_key_for`
  （windows-gated）が握り、機構側で複製しない（SSOT 二重化を回避）。

### テスト結果（Linux / cross-compile）

- `cargo test -p awase-windows --lib`: **172 passed / 0 failed**（Phase 0 の 158 から
  contract/parity/機構テスト計 14 件増、退行なし）。
- `cargo check -p awase-windows --target x86_64-pc-windows-gnu --lib`: green。
- `cargo clippy -p awase-windows --lib --target x86_64-pc-windows-gnu -- -D warnings`: green。
- `cargo fmt --check`: green。

### Phase 1d（実機配線）への申し送り

- **VK 解決の合流点**: `GjiActuation.open` を `ime_key_for(KeyMechanism::GjiDirect, ..)`
  へ渡して具体 VK を解決し、`GjiActuation.fsm_sync` を実 `GjiFsm`（`gji_on_ime_on` /
  `gji_on_ime_off`）へ写像するのが 1d の中心作業。MS-IME 経路（`MsImeDirectStrategy`
  相当）は GJI 機構とは別に、`ime_open_mechanism==SharedImeKeyDispatch` かつ
  `active_ime_kind==MicrosoftIme` の合成としてランタイムが選ぶ。
- **並走方式**: 「Phase 1d」節の read-only shadow 方式（actuate は常に片方のみ、もう片方は
  parity 比較のみ）を厳守すること。`assert_policy_parity` 相当の実行時 parity ガードを
  ソーク中の shadow 比較に流用できる。
- **probe_budget_ms の精緻化**: 現状は `is_confirm_key` 軸を未使用（`TsfNative` は
  `long_idle` のみ、`Imm32Unavailable` は両軸未使用で単一定数）。`ColdReason` を
  windows-gated メソッドとして追加し、BUG-01/BUG-21 の重症度別予算を復元するのが 1d の残作業。
- **不変条件の実行時化**: contract test 5 件は Phase 1 では「型/静的宣言レベル」の検証に
  留まる。1d で実際に belief/`GjiFsm` を書く経路が入ったら、不変条件1・3・4 を journal
  リプレイ（ADR-082 の `EventOrigin`）で実行時にも固定すること。

## Phase 1d 準備状況・ソークチェックリスト（2026-08-01 追記）

Linux サンドボックス（wine 未導入、Windows 実機アクセスなし）で ADR-082 の残作業
（BUG-41/BUG-33 拡張）と並行して Phase 1d への着手可否を検討した結果を記録する。

### 「実機なし準備コード」を書かなかった判断

Phase 1d の本体は「`GjiActuation.open` を具体 VK へ解決し `GjiActuation.fsm_sync` を
実 `GjiFsm` へ写像する」ランタイム配線と、「並走中に actuate するのは常に片方だけ、
もう片方は read-only shadow として parity 比較のみ」という方式の実装である
（上記「申し送り」節）。この parity 検証が実際に意味を持つのは、Chrome×GJI /
LINE×MS-IME / Edge×GJI / WezTerm(TsfNative) / Teams という組み合わせで実機ソークを
回し、新旧経路の判断が実際に一致し続けることを確認して初めてであり、Linux で書ける
「型としての配線」自体は Phase 1c（`state/ime_profile_driver.rs`・
`state/gji_direct_mechanism.rs`）で既に完成している。この状態でさらに実行されない
グルーコードを積み増すことは、ADR-081 Phase 0 が定量化した失敗モード
（「既に部分分離されている実装が同期を怠ってバグを生んだ」6件、コンテキスト節参照）を
未配線コードの形で再生産するリスクの方が、準備コードの価値より大きいと判断した。
`is_confirm_key`/`ColdReason` 軸の精緻化（`probe_budget_ms`）も、`.claude/rules/
tuning-constants.md` の「実測なしのエスカレーション禁止」に照らし実機計測なしには
着手できない。**結論: 型は書けるものが既に書かれている。1d は実機ソークからしか
着手できない。**

### ソーク行列チェックリスト

次に Windows 実機セッションが取れたとき、このチェックリストをそのまま上から
実行すればよい状態を目指して整理した。各セルの「不合格」は即座に
`docs/known-bugs.md` へ症状を記録し、`.claude/rules/experiment-logging.md` の
記録規約（アプリ・IME・再現手順の3点）に従うこと。

| 組み合わせ | cold-start | focus 往復 | drift correction |
|---|---|---|---|
| Chrome × GJI（Imm32Unavailable） | 長 idle 後の1文字目がリテラル化しない（BUG-02/BUG-21 系） | Alt+Tab 復帰直後に spurious `apply_ime_open` が発火しない | `Blind` give-up 後、乖離が続いても再送が暴走しない（BUG-43 型の再発なし） |
| LINE/Qt × MS-IME（ImmCross） | 物理 KANJI 押下直後の初回入力が化けない | ウィンドウ切替後、IME ON/OFF 表示と実状態が一致 | 意図しない IME トグルが繰り返し発火しない |
| Edge × GJI（Imm32Unavailable） | Chrome と同様（別プロセスでの再現性確認） | フォーカス往復で `ObservedEisu` が stale なまま残らない（BUG-07/22/37 系） | 同上 |
| WezTerm（TsfNative） | F2 送信後、TSF composition context 初期化待ちで最初の文字が欠落・部分リテラル化しない（BUG-01/BUG-18 系） | フォーカス変更直後の `GjiFsm` cold 判定が過剰でない（BUG-33 追補3・4、本セッションで境界値テスト済み） | drift 補正が確定済み文字を誤って backspace しない |
| Windows Terminal（TsfNative） | 同上 | 同上 | 同上 |
| Teams | 長時間タイピング中に文字消失しない（BUG-31 系） | Ctrl+Shift+変換 等の Engine ON コンボが no-op にならない（BUG-42 系） | 同上 |

**運用条件（ADR 本文「Phase 1d」節から転記）**:
- 並走中に実状態へ actuate するのは常に片方のみ。もう片方は計算と parity 比較のみの
  read-only shadow とする。
- 1プロファイルのソーク合格 → 即座に旧経路を撤去 → 次のプロファイルへ（一括撤去を
  最後にまとめない）。
- 並走期間の上限: 上記ソーク行列合格 + 実使用3〜5日のハード基準。

**不合格時にやること**: 直す前に `%TEMP%/awase_journal_<tick_ms>.json` をダンプする
（ホットキー: Alt+変換→Alt+無変換 を2回連続、`docs/journal-replay-guide.md` 手順1）。
ADR-082 の `JournalEntry::ImeActuation`（Phase 0.5 実施済み）・`ImeEvent`（決定1、
本セッションで実施済み）が構造化されているため、ダンプから直接 `tests/journals/`
フィクスチャへ転記できる。

## Phase 1d 検討・実施記録（2026-08-02、ランタイム配線は見送り・設計欠陥1件を発見）

ユーザーから「配線コード自体は実機なしで書ける、実機ソークが要るのは検証だけ」との
指摘を受け、read-only shadow 方式のランタイム配線（新ドライバ経路は計算のみで実状態に
書き込まず、旧経路との判断の一致をログ比較する）に着手する検討を行った。Opus 2周
（立案→批判的レビュー）を経て、**ランタイム配線は実施せず**、代わりに検討過程で
発見した実際の設計欠陥をテストで固定した。**上の「Phase 1d 準備状況・ソークチェック
リスト」節を含む既存本文は変更していない** — 本節は追記のみ。

### ランタイム配線を見送った理由

当初計画は「shadow は `&self`（`Runtime`/`WindowsPlatform` の可変借用を取らない）+
`#![forbid(unsafe_code)]` により、実 IME 制御へ actuate できないことを構造的に保証する」
というものだった。しかしレビューでこの保証が**このリポジトリでは成立しない**ことが
判明した:

- 実 actuate 経路そのものが `&self` で宣言されている（`platform.rs::
  apply_ime_open_with_view`/`apply_ime_open_with_belief` はいずれも `&self`）。
  `&mut` を取らないことは何の保証にもならない。
- `WindowsPlatform.output` は `pub` フィールドで、内部に `RefCell`/`Cell`/`AtomicBool`
  を多数抱える（`output/tsf_warmup_coord.rs`・`output/key_injector.rs`・
  `state/conv_mode.rs` 等）。これらを書き換える `&self` メソッド（`Output::send_keys`
  等）が実在し、`&self` 経由でも実際の IME 送信ができてしまう。
- `#![forbid(unsafe_code)]` が塞ぐのは「Win32 を直接叩くこと」のみで、
  `ImeController::apply` のような safe ラッパー経由の actuate は素通しする。

**このリポジトリで IME OFF キー選択が5日間に6回反転した経緯
（`.claude/rules/experiment-logging.md`）を踏まえ、「規律で守る」設計のまま実機なしで
本番経路に近いコードを書き足すことは避けるべき**と判断した。ゼロにする方法（`state/`
の ungated モジュールに置き、import をスカラー値のみに絞って actuate 手段を構文的に
到達不能にする）はあるが、次の理由によりそのコストにも見合わないと判断した:

**ランタイム配線が検証しようとしていた内容は、既に本番コード0行で証明済みだった。**
`tests/ime_key_sequence_golden.rs::driver_shadow_parity_matches_characterize_strategy_primary_path`
（Phase 0/1c で実装済み）が `ImeProfileDriver` ベースの戦略選択と現行
`characterize_strategy` の一致を全 `(active_ime_kind, profile)` 組み合わせで
コンパイル時に固定しており、`state/ime_profile_driver.rs` の
`imm32_unavailable_driver_matches_app_ime_policy`/`tsf_native_driver_matches_app_ime_policy`
が feedback policy の一致を同様に固定している。ランタイムでこれを再検証しても
検出力は増えない。

### 発見した設計欠陥: GjiFsm 同期義務の非対称（Phase 1e ブロッカー）

検討の過程で、`legacy`（`platform.rs::on_ime_applied`）と `GjiDirectMechanism`
（Phase 1c 実装）の間に実際の非対称があることが判明した。

- legacy は **profile を問わず** `open` の値だけで無条件に `GjiFsm` を同期する
  （`gji_on_ime_on`/`gji_on_ime_off` を `outcome != UnsafeToToggle` なら常に呼ぶ）。
- `GjiDirectMechanism::access_for` は `uses_gji_direct() == true` のドライバ
  （`Imm32UnavailableDriver`/`TsfNativeDriver`）にしか token を発行しない。
  `ImmCrossDriver`（LINE/Qt 等）は `false` を宣言するため到達不能。

LINE × Google 日本語入力（ImmCross プロファイル × GJI 有効）は実在する組み合わせであり、
Phase 1e で legacy（`on_ime_applied` の直接呼び出し）を撤去すると、この組み合わせで
だけ `GjiFsm` 同期が失われる。これは「belief を actuate 抜きで ON にする高速パスが
`GjiFsm` 同期を踏み抜く」BUG-18/22 型の再発条件そのものであり、Phase 1c 不変条件4が
まさに防ごうとしていた失敗モードである。**`KnownGap` として流してよい差分ではない。**

`state/gji_direct_mechanism.rs` に `legacy_gji_sync_obligation(open, outcome) ->
Option<GjiFsmSync>` という純粋関数と、この非対称を直接示すテスト
`imm_cross_driver_cannot_obtain_sync_that_legacy_still_requires` を追加して固定した。

**Phase 1e 着手前の必須対応**: 同期義務の宣言軸を `uses_gji_direct()`（静的・profile 軸）
から `active_ime_kind == GJI`（動的軸、ADR 本文「GJI 横断性の設計」節が既に
「profile 軸と直交する」と明記している軸）へ改める設計変更が必要。`ImmCrossDriver` に
`uses_gji_direct` を条件付きで true にする、または `GjiDirectMechanism::access_for` の
ゲート条件自体を profile ではなく実行時観測に委ねる、のいずれかの方向で Phase 1e 起票時に
確定させること。

### Phase 1d の残スコープ（未着手のまま持ち越し）

- 上記 GjiFsm 同期義務の非対称の解消（Phase 1e ブロッカーとして最優先）。
- `skip_imm=true`（ImmCross 失敗後の GJI/MsImeDirect/KanjiToggle フォールバック合成）を
  `ImeProfileDriver` でどう表現するか（`tests/ime_key_sequence_golden.rs` の該当コメント
  参照、未解決）。
- `probe_budget_ms` の `ColdReason` 軸精緻化（実機計測が必要、引き続き着手不可）。
- 実機ソーク自体（「Phase 1d ソークチェックリスト」節、変更なし）。

### テスト結果

- `cargo test -p awase-windows --lib`: **268 passed / 0 failed**（既存268件から
  `gji_direct_mechanism` に5件追加、退行なし）。
- `cargo test --test architecture_guard --test layer_boundary_guard --test
  golden_scenarios --test ime_key_sequence_golden`: 全 green。
- `cargo check`/`cargo clippy`（Linux・windows-gnu 両方、`--lib`/`--tests`、
  `-D warnings`）/ `cargo fmt --check`: いずれも green。
