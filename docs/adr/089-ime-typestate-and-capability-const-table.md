# ADR-089: IME 状態制御を Rust の型システムでどう表現するか — 型状態パターンの局所適用と capability const 表（trait 静的分岐の却下）

## ステータス

**ドラフト。Fable（レビュアー）× Opus（設計者）による pre-mortem 往復
**4ラウンド**で収束。本 ADR 起票時点でプロダクションコードの変更は 0 行だった。**

**追記（2026-08-12）**: **Phase A（観測側、§6）は実装済み**（`state/evidence.rs`
新設、`ObservationStore` のプール分離、`architecture_guard.rs` の整理）。
**Phase B（actuation 側、§6 item 6〜10）も同日実装済み**（実装記録は §6
「Phase B 実施記録」節）。**Phase C（§6 item 11〜12）も同日実装済み**——
着手条件（ADR-088 トラック D の復旧）は**ユーザー判断で解除**され、方針は
「実装 → 実機での長期ソークで検証」に変更された（§6 Phase C「ゲート解除」節、
実装記録は §6「Phase C 実施記録」節、実機ソークの申し送りは §9-17）。
Phase A 実装時に本文へ入れた訂正は次の 3 点で、いずれも
「r5 までの記述が実コードと食い違っていた」ものである:

- **dylint 2 crate は撤去対象ではなかった**（§7・§1.4・§3 冒頭の表・§8.1・§10）。
  `observation_source_guard` は input_mode 軸、`ime_event_guard` は
  `PanicReset`/`HwndCacheRestored`/`EngineActivationSync` を見ており、
  Phase A が型化した open 軸とは重ならない。
- **`architecture_guard.rs` から削除できたテキスト検査は正味 0 件**（§7・§8.1）。
  5 件削除の計画に対し、削除→復活 1 件・維持 4 件・新設 1 件になった。
- **Phase A の型はまだ本番経路に効いていない**（§9-10）。`record` / `record_belief`
  の本番呼び出し元がゼロで、観測はすべて `AnyObservation` 経由で流れる。
  witness の強度も不均一である（§9-11）。

round4 は round3 の設計に対する追加裏取り（実コード照合）だけを行い、
**r3 の設計を全面採用**した（新規の設計変更なし、軽微な指摘2件が §9 に残った）。

**round5（Opus による起票後レビュー、2026-08-11）で 10 件の指摘を反映した。**
うち設計そのものを変えたのは次の 4 件で、いずれも **r3/r4 が実コードと
食い違っていた**ことによる（§1.3 冒頭の「3度、実コードと食い違った前提の上に
組まれていた」に続く 4 度目）:

1. **evidence 型は 11 個ではなく 9 個**——`ConvBitsInference` / `GjiIoInference` は
   `PerSourceObservations` にフィールドが無く、open 観測として構造的に記録
   されない（§1.3(h)・§2.1、INV-38）。
2. **`derive_actuating` / `derive_any` は `DeriveOutcome` を返す**——
   `Option<bool>` へ潰すと ADR-087 の `WarrantBasis` 3 variant が構築不能に
   なる（§2.1、INV-39）。
3. **`ActuationReceipt::settle` は `&mut GjiFsm` を取れない**——`GjiFsm::on_sync`
   は存在せず、同期には `&mut WindowsPlatform` 相当が要る。`GjiSyncSink` trait で
   受ける形に訂正（§1.3(f)・§2.4、INV-42/43）。
4. **`caps` の GJI/MsImeDirect 行の末尾に `KanjiToggle` を置かない**——現行の
   フォールスルー述語では到達不能で、到達させると Win キー押下中に非冪等な
   `VK_KANJI` を送る新経路になる（§2.3・§2.8・§4.9、INV-44）。

あわせて §4.1 却下理由5 の事実誤認（「ADR-081 Phase 1d が 1 年近く未配線」は誤り。
実際は約 2〜3 週間で、停滞理由は Windows 実機の不在）と、§8.1 の保証水準の誇大
（INV-43 は「型として再発不能」ではなく「debug ビルドの実行時検出」）を訂正した。
**結論（trait 静的分岐の却下、型状態を 3 箇所に絞る、`caps` は const 表）は
いずれも維持している。**
r1〜r2 で採っていた2つの方向——**capability の trait 静的分岐**と
**sealed trait 2分割による観測プールの排他**——はいずれも却下・撤回されており、
その経緯は §4 に却下記録として残す（`.claude/rules/experiment-logging.md` の
「なぜ前回それを捨てたのかを辿れるようにする」規約を、revert コミットではなく
ADR のレベルで適用する）。

**実機検証の状態**: 本 ADR は新規実測を一切含まない。§1 の事実確認はすべて
`ae64431d` 時点の実コード読解と既存の `docs/known-bugs.md` /
`docs/experiments.md` / ADR-080〜088 に由来する。実測が必要になる作業は
§6 Phase C にのみ現れる。

**ADR-088 との役割分担（重要）**:

| | ADR-088 | ADR-089（本 ADR） |
|---|---|---|
| 問い | **何が壊れているか** | **それを Rust の型でどう表現するか** |
| 内容 | IME 状態の4軸分解（open / charset / romaji / engine）、`AxisCapability`、`CharsetOwner`、修飾キー汚染ハザード（未収束）、VK 送信口 18 箇所の棚卸し、実機実測トラックの中断記録 | 型状態パターンの**局所適用**3箇所（観測プール分離 / Actuation チェーン / `GjiFsm` 同期義務のアフィン型化）と、capability を **const 表**に据える決定 |
| 成果物の性質 | **発見の記録**（軸モデルと所有権概念の発明） | **表現手段の決定**（どの規律をコンパイラへ移すか、どれを移さないか） |
| 番号空間 | INV-29〜37、P17〜18 | **INV-38〜46、P19〜21** |

**両者は独立に実装できる。** ADR-088 の `CharsetOwner`（charset 軸の所有権）は
本 ADR の Phase A/B と衝突せず、逆に本 ADR の型状態は charset 軸に依存しない。
ただし §2.7 のとおり、本 ADR は `CharsetSlot` の型状態化を**明示的に却下**して
おり、これは ADR-088 の `CharsetOwner` を private フィールド + witness 引数の形で
実装することを推奨する意味を持つ。

**invariant の採番**: ADR-084 が INV-1〜11、ADR-086 が INV-12〜19、ADR-087 が
INV-20〜28、ADR-088 が INV-29〜37 を使用済みのため、本 ADR は **INV-38 から**
採番する（同一の名前空間に属し、後日の grep で一意に辿れることが規約の実効性
そのものであるため。ADR-086/087/088 と同じ理由）。

**原則（P 番号）の採番**: ADR-084 が P1〜P5、ADR-086 が P6〜P10、ADR-087 が
P11〜P16、ADR-088 が P17〜P18 を使用済みのため、本 ADR は **P19 から**採番する。
**注意**: 本 ADR の設計セッション（Fable × Opus）は内部で r1/r2/r3 という
ラウンド番号を使っており、その中で §・§2.x といった節番号も独自に振られていた。
本 ADR の節番号はそれとは**別物**である。設計セッション内部の r2 §3 は本 ADR の
§4.1 に、r3 §2.1〜2.7 は本 ADR の §2.1〜2.7 に対応する。

---

## 1. コンテキスト

### 1.1 発端 — 「Rust の型システムを活かした非連続な再設計をしたい」

ADR-088 が収束したトラック A（軸モデル + `CharsetOwner`）を実装する段になって、
ユーザーから「型状態パターンと trait 静的分岐で、Rust の型システムを活かした
**非連続な**再設計をしたい」という要望が出た。「非連続」は「既存の分岐を少しずつ
整理する」のではなく「**規律をレビューとテストからコンパイラへ移す**」という意味で
ある。

この要望が出た背景には、本リポジトリの IME 制御まわりの規律が
**ほぼすべて人間のレビューと機械的テキスト検査で支えられている**という現状がある。

### 1.2 現状の規律の担い手（`ae64431d` 時点の実測）

| 担い手 | 実体 | 件数 |
|---|---|---|
| **機械的テキスト検査** | `crates/awase-windows/tests/architecture_guard.rs` の `#[test]` | **22 件**（ヘルパ関数を除く。`extract_fn_body` / `extract_all_balanced_blocks` / `production_code_only` によるソーステキスト走査） |
| **dylint（HIR）** | `lints/ime_event_guard`、`lints/observation_source_guard`、`lints/no_vk_as_scan` | **3 crate** |
| **golden / characterization** | `tests/ime_key_sequence_golden.rs`、`tests/golden_scenarios.rs`、`tests/golden/` | 2 テストファイル + golden ディレクトリ |
| **リプレイ** | `tests/journal_replay.rs`、`tests/drift_correction_replay.rs`、`tests/journals/` | 2 テストファイル |
| **コンパイラ** | `UserIntentSource` の型強制、`ForceGuardSet.guards` の private 化、`GjiDirectAccess` token（ADR-081 Phase 1c、**未配線**）等 | 散発的 |

**`architecture_guard.rs` の 22 件のうち、少なくとも 8 件は「本来なら型で防げるはず
のこと」をテキスト検査で代替している**:

- `user_intent_source_construction_is_limited_to_typed_writers`（`:628`）
- `heuristic_default_observation_is_limited_to_designated_methods`（`:337`）
- `focus_probe_observation_is_limited_to_real_probe_path`（`:523`）
- `conv_open_inference_source_is_limited_to_report_and_gate`（`:610`）
- `panic_reset_event_is_limited_to_apply_panic_reset`（`:260`）
- `hwnd_cache_restored_event_is_limited_to_apply_hwnd_cache_restore`（`:280`）
- `input_mode_observed_construction_sites_are_accounted_for`（`:300`）
- `input_mode_applied_construction_sites_are_accounted_for`（`:382`）

これらはいずれも「**この `ObservationSource` はこの関数からしか名乗れない**」という
形の制約であり、`ObservationSource` が単なる enum で `ImeObservation` の
`source: ObservationSource`（`state/observation_store.rs:28`）が `pub` である限り、
**コンパイラには何も伝わっていない**。テキスト検査は関数名を変えただけで穴が開くし、
`extract_fn_body` の needle が実装のリファクタでずれれば黙って通る。

### 1.3 棚卸し（実コードで裏取りした前提。ここが r2 の誤りの発生源だった）

本 ADR の設計は r1〜r2 で**3度、実コードと食い違った前提の上に組まれていた**。
r3 でそれらを訂正し、r4 で全行を再照合した。**それでもなお r5（起票後の Opus
レビュー）で 4 件の食い違いが見つかっている**（§ステータスの round5 節）——
うち (f)(h) は本節の記述そのものの訂正である。以下は `ae64431d` 時点で確認した
事実である。**この節の内容と食い違う設計案は、その時点で誤りである。**

#### (a) IME open 戦略は 4 本、`ime_key_for` がカバーするのは 2 機構だけ

`ime_controller.rs` の `ImeOpenStrategy` 実装は4本ある——
`ImmCrossProcessStrategy`（`:54`）/ `GjiDirectStrategy`（`:103`）/
`MsImeDirectStrategy`（`:156`）/ `KanjiToggleStrategy`（`:229`）。

一方、キー値の SSOT である `key_sequence_policy::ime_key_for`
（`state/key_sequence_policy.rs:106`、`pub(crate) const fn`）が受け取る
`KeyMechanism` は **`GjiDirect` と `MsImeDirect` の2値のみ**である。ImmCross は
VK を送らず `ImmSetOpenStatus` のクロスプロセス呼び出しを行い、KanjiToggle は
`VK_KANJI` を `ime::post_kanji_toggle_to_focused()` で直送する。
**したがって「chain の各要素にキーを引ける」わけではない**（§2.8 でこの非対称を
どう扱うかを決める）。

`KanjiToggleStrategy` の到達条件は doc コメント（`ime_controller.rs:218-228`）に
明記されている: **Standard プロファイル × MS-IME × ImmCross 非同期適用の失敗後
（`apply_skipping_imm`）という1組だけ**。`ActiveImeKind` は2値のため「IME 種別不明」
は存在しない。そして **`VK_KANJI` は非冪等なトグルキー**であり、
`already_matched` の判定を行わずに送信する。

#### (b) `AcceptedObservation::for_sync` は `pub`、呼び出し元は 3 箇所

`state/probe_admission.rs:113` に `pub fn for_sync(focus_epoch: FocusEpoch) -> Self`
がある。呼び出し元は `runtime/mod.rs:1019`、`runtime/ime_refresh.rs:157`、
`runtime/ime_refresh.rs:366` の3箇所（すべて `runtime` 層）。
`pub` である必要はなく、**`pub(crate)` へ縮小できる**（Phase A に含める）。

#### (c) `TickMs` は既存（`state/mod.rs:13`、`pub struct TickMs(pub u64)`）

r1 で「新設する」と書いていたのは誤り。

#### (d) `AppImePolicy` の 4 フィールドのうち、本番の読み手があるのは 2 つだけ

`state/app_ime_policy.rs:44-50` の `AppImePolicy` は
`owns_physical_kanji` / `actuator_kind` / `focus_settle_ms` / `default_feedback`
の 4 フィールドを持つ。読み手を全数調査した結果:

| フィールド | 本番コードの読み手 | テストの読み手 |
|---|---|---|
| `default_feedback` | **あり** — `state/open_warrant.rs:204`（ADR-087 Step 4c）、`state/platform_state.rs:493`、`runtime/ime_refresh.rs:600` | `app_ime_policy.rs` 自身、`tests/drift_correction_replay.rs:168` |
| `focus_settle_ms` | **あり** — `state/ime_model.rs:508`（settle_until 算出）、`state/platform_state.rs:370` / `:486`、`runtime/mod.rs:507` | — |
| `owns_physical_kanji` | **ゼロ** | `app_ime_policy.rs` 自身、`tests/golden_scenarios.rs:190` / `:340` |
| `actuator_kind` | **ゼロ** | `app_ime_policy.rs` 自身、`tests/golden_scenarios.rs:175` |

**r3 が「`actuator_kind` の `transport.rs` の参照を `caps(p,k).chain[0]` へ
差し替える」と書いていたのは誤りである（本 ADR で訂正）。** `runtime/transport.rs`
の `PhysicalKeyDisposition::plan`（`:134`）が読んでいるのは
`profile.can_use_imm32_cross_process()`（`AppImeProfile` のメソッド）と
`key_sequence_policy::gji_direct_applicable` / `ms_ime_direct_applicable`
（`:161-162`）+ `active_ime_kind` であって、**`owns_physical_kanji` でも
`actuator_kind` でもない**。

ただし r3 が `owns_physical_kanji` を `caps` に吸収しないと判断した**理由**
（BUG-46 の物理キー抑止は `ActiveImeKind` と組み合わせて判断する別軸であり、
静的 profile 軸の `caps` に入れると意味が壊れる）は、`plan` の実装を読む限り
**正しい**。訂正されるのは「どこが読んでいるか」であって「別軸である」という判断
ではない。

#### (e) `ImePolicyProfile::Plain` / `Unknown` は本番で一度も構築されない

これが r2 の中核的な誤りの発生源である。事実は次のとおり:

- `ImePolicyProfile`（`state/ime_event.rs:218`）は **5 値**
  （`ImmCross` / `Imm32Unavailable` / `TsfNative` / `Plain` / `Unknown`）で、
  `#[default]` は `Unknown`。
- 実行時に profile を供給するのは `impl From<AppImeProfile> for ImePolicyProfile`
  （`focus/class_names.rs:196`）だけで、`AppImeProfile` は **3 値**
  （`Standard` / `Imm32Unavailable` / `TsfNative`、`:120`）。
  `Standard → ImmCross` にマップされる。
- `ImeModel` の初期 `app_policy` は `AppImePolicy::standard()`（`ime_model.rs:205`）
  = `from_profile(ImePolicyProfile::ImmCross)`（`app_ime_policy.rs:122-124`）で
  あり、**`Unknown` ではない**。
- `ImePolicyProfile::Plain` / `Unknown` をリテラルとして書いている箇所は、
  `app_ime_policy.rs:82` と `ime_profile_driver.rs:378` の**match 腕**、および
  両ファイルの**テスト**（`app_ime_policy.rs:167` / `:192`、
  `ime_profile_driver.rs:516-517`）のみ。`ImePolicyProfile::default()` の呼び出しは
  リポジトリ全体で **0 件**。

**したがって `Plain` / `Unknown` 行は現時点で構造的に到達不能である**
（r3 は「`ImeModel` 初期値経由でのみ到達する」と書いていたが、これも誤りだった。
本 ADR で再訂正する）。`app_ime_policy.rs:82-90` が `Plain`/`Unknown` を
`ImmCross` と同じ腕に入れて `owns_physical_kanji: true` /
`actuator_kind: ImmCross` / `focus_settle_ms: 100` /
`default_feedback: Read{ImmGetOpenStatus, ...}` を与えているのは、
**到達したときのための安全デフォルト**である。

**この行を落とすと何が起きるか（r2 の実害）**: r2 の caps 表は `Plain`/`Unknown` を
別行にして `chain: [MsImeDirect, KanjiToggle]` 相当を与えていた。将来 `Plain` が
配線された場合、起動直後 × MS-IME で `KanjiToggle`（非冪等な `VK_KANJI`）へ
直行する経路が生まれる——現行が慎重に避けている shadow desync の新設である。
**caps 表を書くときは、既存の安全デフォルトを黙って落としていないかを
`app_ime_policy.rs` と1行ずつ突き合わせること。**

#### (f) `GjiFsm` 同期義務は outcome 軸だけで決まる

`platform.rs:879-891` の `on_ime_applied` 相当の分岐は、**戦略・profile・
`active_ime_kind` を一切問わず** `open` の値だけで `gji_on_ime_on(mode)` /
`gji_on_ime_off()` を呼ぶ。`mode` は `self.output.injection_mode`（**`:883`** で
読み、`:884` の `self.gji_on_ime_on(mode)` へ渡す）。

**同期は `&mut GjiFsm` では実行できない。** `GjiFsm` には `on_sync` は存在せず
（`tsf/gji_fsm.rs:529` は `timed_fsm` の `fn on_event`）、実体は
`output.warmup_coord.tsf_warmup`（`RefCell`）の中にある。legacy の
`gji_on_ime_on`（`platform.rs:484-493`）は `self.output.gji_on_event(GjiEvent::ImeOn{..})`
が返す `Response<GjiAction, GjiTimer>` を `self.dispatch_gji_response(&resp)`
（`platform.rs:259`）でタイマー・アクションへ展開する。つまり **1 回の同期には
`&mut WindowsPlatform` 相当が要る**（§2.4 の `settle` シグネチャはこの事実に
従って設計する）。

これを純粋関数として固定したのが
`state/gji_direct_mechanism.rs:159` の
`legacy_gji_sync_obligation(open, outcome) -> Option<GjiFsmSync>` であり、
**`outcome == UnsafeToToggle` のときだけ `None`、それ以外の全 outcome で
`Some(GjiFsmSync::for_open(open))`** を返す。

同ファイルの doc コメント（`:134-157`）は、この legacy と ADR-081 Phase 1c の
`GjiDirectMechanism::access_for`（`uses_gji_direct() == true` のドライバにしか
token を出さない）との**非対称**を「ADR-081 Phase 1e ブロッカー」として明記して
いる。`ImmCrossDriver`（LINE/Qt）は `uses_gji_direct() == false` を宣言するため、
機構経由では `GjiFsmSync` を得られない。しかし **LINE × Google 日本語入力は実在
する組み合わせ**であり、legacy はそこで今も同期している。ここを落とすと
BUG-18/22 型の同期欠落が再発する。

#### (g) `ActiveImeKind::MicrosoftIme` は観測値ではなく**推定値**

`tsf/observer.rs:493-503` の doc コメントに明記されている:

> `gji_monitor_ok` の状態から派生する（新たなアトミック不要）。
> **GJI が検出されていなければ MS-IME（または互換 IME）とみなす。**

つまり `MicrosoftIme` は「MS-IME を観測した」ではなく「**GJI を検出できなかった**」
である。GJI 起動直後・フォーカス直後の未検出ウィンドウでは、GJI 環境でも
`MicrosoftIme` を返しうる。

#### (h) `ObservationSource` は 11 値だが、open 観測として記録されるのは 9 値だけ

`state/ime_event.rs:181-195` の `authority()`:

- **`ObservationAuthority::Actuating`（5）**: `ImmGetOpenStatus` / `ImmCrossProbe` /
  `ObserverPoll` / `Gji` / `Tsf`
- **`ObservationAuthority::BeliefOnly`（6）**: `ConvOpenInference` /
  `HeuristicDefault` / `HwndCache` / `FocusProbe` / `ConvBitsInference` /
  `GjiIoInference`

**しかし `ConvBitsInference` / `GjiIoInference` の 2 値は open 観測プールに
構造的に入らない。** `PerSourceObservations` のフィールドは 9 個しかなく、
`get`（`state/observation_store.rs:79-94`）はこの 2 値に対して `None` を、
`set`（`:97-113`）は no-op を返す。`authority()` の doc コメント
（`state/ime_event.rs:176-179`）も「この関数の呼び出し元からは到達しない。
`ObservationSource` 全体で定義する都合上、網羅性のため `BeliefOnly`
（安全側デフォルト）を割り当てる」と明記している。実際この 2 値は
`InputModeObserved { source, confidence }`（input_mode 軸）の source としてのみ
dispatch される（`runtime/key_pipeline.rs:597` = `ConvBitsInference`、
`runtime/ime_refresh.rs:177` = `GjiIoInference`）。

したがって:

- **open 観測プールの分割は 5:4** — Actuating 5 値 : BeliefOnly 4 値
  （`ConvOpenInference` / `HeuristicDefault` / `HwndCache` / `FocusProbe`）。
  **この 5:4 の分割が §2.1 のプール分離と 1:1 対応する。**
- 残り 2 値は **open 軸の evidence 型を持たない**（§2.1 で明示的に除外する）。

`ObservationStore::derive_open`（`state/observation_store.rs:314`）は
`derive_open_filtered(now, |_| true)` の薄いラッパであり、`derive_open_filtered`
（`:331`）が述語で絞り込む。`derive_open_filtered` の戻り値は
**`Option<DeriveOutcome>`**（`Option<bool>` ではない）で、「どのソースが決定打
だったか」（`HighSingle { source, open }` / `MediumConsensus { first, second, open }`）
を保持する。`state/open_warrant.rs:160-180` がこれを `WarrantBasis::DirectRead` /
`Corroborated` / `SingleIndirect` の構築に使っている（ADR-087）。
**この診断情報を捨てる設計は ADR-087 を壊す**（§2.1 で戻り値型をそろえる）。

#### (i) 存在しないもの（新設が必要）

`DriftEpisode` / `WriteMechanism` / `ActuationTarget` 型（`ime::ActuationTarget` は
`runtime/key_pipeline.rs:642` で `capture(focus_gen).await` として使われており
**存在する**） / `ReadBack` / `ConvergedReceipt` / `Observed<E>` / `AnyObservation` /
`OpenEvidence` / `IntentWitness` / `ActuationReceipt` / `ImeKindId` はいずれも
**現存しない**。`trybuild` はどの `Cargo.toml` にも入っていない。
`state/ime_kind.rs` も存在しない。

### 1.4 既存 ADR との関係

本 ADR は**既存 ADR を1件も廃止しない**。表現手段の決定であり、決定内容の変更では
ない。ただし ADR-081 については1件、**明示的な凍結提案**を含む（§6）。

| 既存 | 何を定めたか | 本 ADR との関係 |
|---|---|---|
| [ADR-078](078-ime-mode-belief-desired-effective-constraint.md) | conv/mode belief の3分割（Phase 1a のみ実装） | **無関係**（本 ADR は belief の分割ではなく、既存 belief の**構築経路**を型で閉じる） |
| [ADR-080](080-ime-actuation-lifecycle-and-epoch-fenced-drift-correction.md) | actuation ライフサイクルと epoch fencing。不変条件6「`ReadBack` の産物を観測として記録しない」 | **一部を型化**。§2.5 の `ConvergedReceipt` が「`Observed<E>` / `AnyObservation` へ変換不能」であることでコンパイラ強制になる（**Phase B 時点では receipt が制御フローに配線されておらず、この強制は空回りしている**——§8.1 の訂正 / §9-16）。ADR-080 の決定は変更しない |
| [ADR-081](081-per-profile-capability-driver-decomposition.md) | プロファイル別 capability 駆動ドライバへの分離（Phase 1a/1b/1c 試験実装・**未配線**、1d/1e 未着手） | **一部を廃止提案**。§2.8 の `caps` const 表は ADR-081 の「capability 表」部分を**別の形（trait ではなく const 表）で実現する**。さらに §2.4 が同期義務を outcome 軸へ移すため、ADR-081 Phase 1c の**不変条件4・5 と `GjiDirectAccess` token は根拠を失う**（§6「ADR-081 Phase 1d 凍結」） |
| [ADR-082](082-journal-structured-replay-and-event-origin.md) | journal 構造化リプレイと `EventOrigin` | **制約として受ける**。§2.1 の `record_replayed(AnyObservation)` は journal / fixture 復元専用の口であり、ここだけ実行時 match が残る |
| [ADR-084](084-conv-mode-single-ownership-and-width-ssot.md) | conv 単一 actuator（P1/INV-1）、書き込みと belief 無効化の不可分性（INV-2） | **維持**。§2.3 の Actuation チェーンは INV-1 の「低レベル API を actuator の外から呼ばない」を型で表現する試み |
| [ADR-085](085-conv-mode-force-policy.md) | `ConvModePolicy{Observe, Force}` | **無関係**（本 ADR は force の可否を変えない） |
| [ADR-086](086-force-write-trigger-and-target-identity.md) | force-write のトリガー軸（INV-15）・空間軸（INV-14 = `ActuationTarget`）、INV-12〜19 | **一部を型化**。§2.3 の `Actuation<Verified>` は INV-14 の「capture したターゲットを最後まで保持する」を型状態で表現する。**INV-14 の残タスク（ImmCross の同期 IMC write の `ActuationTarget` 化）は §6 Phase C へ送る**（ADR-086 Phase 3 で「スコープ超過」と判断された作業を再吸収しないため） |
| [ADR-087](087-open-belief-actuation-warrant-separation.md) | `OpenWarrant` / `WarrantBasis` / `issue_open_warrant()`、INV-20〜28 | **維持し、その入力を型で守る**。`issue_open_warrant()` は `ObservationStore` を読むが、そのストアに何が入るかは今テキスト検査でしか守られていない。§2.1〜2.2 がそこを閉じる。`WarrantBasis` の7 variant は増やさない |
| [ADR-088](088-ime-axis-capability-and-charset-owner.md) | 4軸モデル、`AxisCapability`、`CharsetOwner`、修飾キー汚染（未収束）、VK 送信口 18 箇所 | **姉妹編。役割分担は §ステータスの表**。ADR-088 の `AxisCapability` と本 ADR の `caps` は**別物**（前者は「軸 × 読み書き可否」、後者は「(profile, IME種別) × 戦略チェーン」）。両立するが、実装時は同じ `state/app_ime_policy.rs` に同居することになるため命名で区別すること |
| `.claude/rules/ime-belief-architecture.md` | Observe → Pure → Apply の三層分離、3段構えの強制 | **段1（コンパイラ）への移動を進める ADR である**。同ルールの「dylint は型で防げない意味論的偽装にのみ投資する」判断基準に従い、本 ADR は dylint を**増やさない**。r5 まで書いていた「2 crate 減らす」は誤りで、既存 3 crate はいずれも Phase A の型化範囲外だった（§7 の訂正） |

---

## 2. 決定

### 2.0 全体方針 — trait 静的分岐は却下、型状態は3箇所に絞る

**capability の表現は const 表（`caps(p, k)`）のままとする。trait 静的分岐
（`trait ProfileCaps` + `impl for ImmCross` + ジェネリック呼び出し側）は採らない。**
却下の経緯は §4.1 に詳述する。

**型状態パターンは、実際に失敗モードを踏んだ 3 箇所にだけ投下する:**

| # | 投下先 | 守る不変条件 | 現状の担い手 | 到達する保証水準 |
|---|---|---|---|---|
| 1 | `ObservationStore` のプール分離（§2.1）+ `Observed<E>` のデータ witness 構築子（§2.2） | 「この観測ソースはこの経路からしか名乗れない」「この観測は actuation の根拠にしてよい／belief 専用」 | `architecture_guard.rs` の 5 テスト（+1 件は期待値縮小）。**dylint 2 crate はここに含まれない**——`observation_source_guard` は input_mode 軸、`ime_event_guard` は `PanicReset` 系であり、いずれも Phase A の型化範囲外（§7 の訂正） | **コンパイル時**（関連型のコヒーレンス + 構築子の引数型） |
| 2 | `Actuation` の型状態チェーン（§2.3） | 「warrant なしに write しない」「capture したターゲットを最後まで保持する」「1 値 = 高々 1 回の成功 write」 | ADR-086 INV-14 の `actuation_target_capture_is_first_await_in_spawn_local_block` 等のテキスト検査 | **コンパイル時**（`run_chain` が `Actuation<Verified>` にしか生えない） |
| 3 | `ActuationReceipt` による `GjiFsm` 同期義務のアフィン型化（§2.4） | 「actuate したら必ず `GjiFsm` を同期する」 | **何も守っていない**（`platform.rs` の分岐が正しく書かれていることに依存） | **debug ビルドの実行時検出**まで（`debug_assert` は release で消え、`#[must_use]` は束縛されると発火しない。§8.1 の保証水準の注記） |

**「型状態にしない」と明示的に決めたもの**: `CharsetSlot`（§2.7）、
capability 分岐（§2.8）、`ImeKindId` によるゲート（§2.9）。

### 2.1 Evidence — 排他を**関連型（コヒーレンス）**に担がせる

#### 問題

「ある `ObservationSource` は actuation の根拠にしてよい（Actuating）／belief に
しか使えない（BeliefOnly）」という 5:4 の分割（§1.3(h)）は、現在 `authority()` の
実行時 match と `derive_open_filtered` の述語で表現されている。呼び出し側が
述語を渡し忘れれば黙って BeliefOnly 観測が actuation 根拠になる。

#### r2 の案とその撤回

r2 は sealed trait を 2 本（`ActuatingEvidence` / `BeliefEvidence`）に分け、
「両方に impl されていないこと」を golden で固定する案だった。**r3 でこれを
撤回する。**

- sealed trait が防ぐのは**外部 crate の impl** だけであり、**crate 内の二重 impl
  は合法**である。
- 「golden で impl 一覧を固定する」は、まさに本 ADR が置き換えようとしている
  テキスト検査の規律そのものであり、**自己矛盾**である。

#### 採用する形 — 単一トレイト + 関連型

```rust
// state/observation_store.rs（または state/evidence.rs へ新設）

mod sealed { pub trait Sealed {} }

pub trait PoolKind: sealed::Sealed {}
pub struct ActuatingPool;
pub struct BeliefPool;
impl PoolKind for ActuatingPool {}
impl PoolKind for BeliefPool {}

/// 観測の「根拠としての種別」。1 型につき impl は 1 つしか書けない。
pub trait OpenEvidence: sealed::Sealed {
    /// この evidence がどちらのプールへ入るか。
    type Pool: PoolKind;
    /// journal / fixture 復元時の実行時タグ（§2.1 末尾）。
    const SOURCE: ObservationSource;
}

impl ObservationStore {
    pub fn record<E: OpenEvidence<Pool = ActuatingPool>>(&mut self, o: Observed<E>);
    pub fn record_belief<E: OpenEvidence<Pool = BeliefPool>>(&mut self, o: Observed<E>);
}
```

`OpenEvidence` を impl するのは **`PerSourceObservations` に実フィールドを持つ
9 値だけ**である（§1.3(h)）。この 9 個が `record` / `record_belief` の全入力を
なす。

**なぜこれで足りるか**: Rust のコヒーレンス規則により、1 つの型に
`OpenEvidence` の impl は 1 つしか書けない。`type Pool` は impl ごとに一意に
決まるので、**二重所属が構造的に表現不能**になり、golden による impl 一覧の固定が
不要になる。これは「排他をレビューで守る」から「排他を型検査器が守る」への移行で
あり、本 ADR が目指す非連続性そのものである。

#### 分類は `authority()` と 1:1（ただし evidence 型は 9 個）

| `type Pool` | evidence 型（現 `ObservationSource`） | 個数 |
|---|---|---|
| `ActuatingPool` | `ImmGetOpenStatus` / `ImmCrossProbe` / `ObserverPoll` / `Gji` / `Tsf` | 5 |
| `BeliefPool` | `ConvOpenInference` / `HeuristicDefault` / `HwndCache` / `FocusProbe` | 4 |
| **（evidence 型を作らない）** | `ConvBitsInference` / `GjiIoInference` | 2 |

#### `ConvBitsInference` / `GjiIoInference` に `OpenEvidence` を impl してはならない

**この 2 値は open 観測プールに構造的に入らない**（§1.3(h)）。`authority()` が
これらを `BeliefOnly` と答えるのは「網羅性のための安全側デフォルト」であって、
「BeliefOnly プールに入る」という意味ではない。`OpenEvidence` を impl すると
次のどちらかになり、**どちらも設計の退行である**:

1. `PerSourceObservations::set` が no-op のままなら、`record_belief` が**黙って
   何もしない**（型は通るのに観測が消える。テキスト検査より悪い偽陰性）。
2. 記録できるようにフィールドを増やすと、**conv ビット由来の間接推測が open の
   多数決に参加する経路を新設する**——`ConvOpenInference` を
   `report_conv_open_inference()` 専用の別ソースに切り出して confidence 上限を
   Medium に縛った BUG-19 対策（`state/ime_event.rs:115-130` の doc）を、
   別の名前でやり直すことになる。

したがって **この 2 値は open 軸の evidence 型を持たない**。両者が運ぶ情報は
`InputModeObserved { source, confidence }`（input_mode 軸、
`runtime/key_pipeline.rs:597` / `runtime/ime_refresh.rs:177`）のままとし、
本 ADR は input_mode 軸の型化には踏み込まない（ADR-078 の領域）。

**`authority()` の実行時 match（`state/ime_event.rs:181`）は 11 値のまま残す。**
journal 復元と診断ログが `ObservationSource` を値として扱う以上、対応表は両方に
必要である。両者が食い違わないことは **9 件の全数テスト**（`OpenEvidence::SOURCE`
× `authority()`）+ **2 件の除外テスト**（`ConvBitsInference` / `GjiIoInference` に
対し `PerSourceObservations::get` が `None` を返し続けること）で固定する（§7）。

#### `derive_any` / `derive_actuating` は `DeriveOutcome` を返す

`derive_open()` に相当する「全観測から導く」経路は
`derive_any(now) -> Option<DeriveOutcome>` とし、**両プールをマージした後に
High単独 → Medium無競合多数決の判定を 1 回だけ行う**。

**戻り値を `Option<bool>` に縮めてはならない。** 現行
`derive_open_filtered`（`state/observation_store.rs:331`）は
`Option<DeriveOutcome>` を返し、`state/open_warrant.rs:160-180` がそこから
`WarrantBasis::DirectRead(source)` / `Corroborated { a, b }` /
`SingleIndirect(first)` を構築している（ADR-087）。**`bool` へ潰すと
`WarrantBasis` の 7 variant のうち 3 つが構築不能になり、§1.4 の「ADR-087 を
維持する」と両立しない。** したがって:

```rust
impl ObservationStore {
    /// ActuatingPool のみ（ADR-087 Step 3 の入力）。
    pub fn derive_actuating(&self, now: Instant) -> Option<DeriveOutcome>;
    /// 両プールをマージしてから 1 回判定（旧 derive_open 相当）。
    pub fn derive_any(&self, now: Instant) -> Option<DeriveOutcome>;
}
```

`bool` だけが欲しい呼び出し元は既存の `DeriveOutcome::value()`
（`observation_store.rs:181`）を使う。**旧 `derive_open()` が
`Option<bool>` を返していたのは `derive_open_filtered` の `.map(|o| o.value())`
ラッパだったからであり、判定本体は最初から `DeriveOutcome` を持っている**——
新 API はラッパを剥がすだけで、情報を落とす変更ではない。

**プール毎に判定してから合成する形は採らない。** BUG-19 は「conv 由来の間接推測が
`desired_open` を直接書き換えた」ことで再発したバグであり、プール毎判定 → 合成は
「BeliefOnly プールだけで結論が出て、それが Actuating 側の High を上書きする」
経路を作りうる——BUG-19 の再発条件と同型である。
`derive_actuating(now)` は `ActuatingPool` のみ、`derive_any(now)` はマージ後判定、
の 2 本だけを提供する。

**pinned test（Phase A の最初にやること）**:
`derive_actuating ≡ 旧 derive_open_filtered(now, |s| s.authority() == Actuating)`、
`derive_any ≡ 旧 derive_open_filtered(now, |_| true)` を、リファクタ**前**に
固定する。**比較は `DeriveOutcome` の等値で行う**（`.value()` の `bool` だけを
比較すると、`WarrantBasis` の構築に使う `source` / `first` / `second` が
変わったことを検出できない）。

#### journal / fixture 復元だけは実行時 match が残る

`record_replayed(AnyObservation)` を唯一の口として集約する。ここでは
`ObservationSource` の値から `authority()` を引いてプールを選ぶ実行時 match が
必要になる。**これは型で消せない残余であり、隠さず1箇所に集める**
（ADR-082 の journal リプレイ基盤がこの口を使う）。

この match の腕は **11 値すべてを網羅する**が、`ConvBitsInference` /
`GjiIoInference` の 2 腕は**記録せず捨てる**（現行 `PerSourceObservations::set`
の no-op と同一の挙動。ここだけ挙動を変えると journal リプレイが本番と別の
状態を再現する）。捨てたことが分かるよう `log::debug!` を残すこと。

### 2.2 Witness — プール分割では守れない不変条件を**データ witness** で補う

プール分割が守るのは「どちらのプールに入るか」だけである。
「**`Observed<FocusProbe>` を probe 経路以外が構築できない**」は依然として
crate 内で破れる（`Observed { source: FocusProbe, .. }` と書けてしまう）。

そこで `Observed<E>` のフィールドを private にし、**構築子がソースごとに固有の
データ witness を要求する**形にする:

```rust
impl Observed<FocusProbe> {
    /// probe 経路でしか作れない。`AcceptedObservation` は
    /// `state/probe_admission.rs` でしか構築できない（Phase A で `for_sync` を
    /// pub(crate) へ縮小、§1.3(b)）。
    pub fn from_probe(accepted: &AcceptedObservation, open: bool, hwnd: HwndId, at: Instant) -> Self;
}

impl Observed<HeuristicDefault> {
    /// 引数が起点を限定する。profile なしには作れない。
    pub fn at_startup(profile: ImePolicyProfile, open: bool, hwnd: HwndId, at: Instant) -> Self;
}

impl Observed<ConvOpenInference> {
    /// conv ビットを読んだ事実そのものを引数に要求する。
    pub fn from_conv(bits: ConvBits, hwnd: HwndId, at: Instant) -> Self;
}

impl IntentWitness {
    /// 物理 IME キー（VK_F3/F4 等）由来の明示意図。`UserIntentSource::PhysicalImeKey`。
    /// `injected == true`（外部注入 / 自分の注入）は `None`。
    pub fn from_physical(e: &RawKeyEvent) -> Option<Self>;

    /// 設定された同期キー（Shift+Space 等）由来。`UserIntentSource::SyncKey`。
    /// 同じく `injected == true` は `None`。
    pub fn from_sync_key(e: &RawKeyEvent) -> Option<Self>;
}
```

#### `UserIntentSource` は 3 値 — witness は 2 本、`Command` はガードで残す

`state/ime_event.rs:73-80` の `UserIntentSource` は
**`SyncKey` / `PhysicalImeKey` / `Command`** の 3 値で、typed writer も 3 本ある
（`state/platform_state.rs:857` `write_sync_key` / `:867` `write_physical_key` /
`:877` `write_set_open_request`）。`from_physical` だけを用意して
`user_intent_source_construction_is_limited_to_typed_writers`
（`tests/architecture_guard.rs:628`、この 3 箇所を数えている）を削除すると、
**残り 2 値の保護が丸ごと消える**。

- **`SyncKey`**: 発行元は `runtime/key_pipeline.rs:821`（`IntentKind::SyncKey`）で、
  `from_physical` と同じ `&RawKeyEvent`（`event`）がスコープにある。
  **`from_sync_key` を追加する**（`injected` チェックは同一。BUG-14 の型化を
  両方に効かせる）。
- **`Command`**: 発行元は `state/platform_state.rs:253`
  （`handle_engine_set_open` → `write_set_open_request`）で、**エンジン内部の判断
  であり `RawKeyEvent` は存在しない**。データ witness に載せられる外部事実が無く、
  「引数の型が起点を限定する」という §2.2 の原理が働かない。
  **したがって `Command` は Phase A の witness 化・ガード撤去の対象から明示的に
  除外する。** `write_set_open_request` を `pub(in crate::state)` に留めたうえで、
  `architecture_guard.rs:628` の count guard は **削除せず、期待値 3 → 1
  （`Command` の 1 箇所のみ）に縮小して残す**。§9-8 に未解決事項として記録する。

**なぜ `pub(in path)` の可視性制御ではなくデータ witness か**:

1. `hook.rs` は `#[cfg(windows)]` の下にあり、Linux のテストからは到達できない。
   可視性で縛ると「Windows では正規経路、Linux テストでは裏口」という二経路が
   でき、テストが本番と別のコードを検査することになる。
2. 可視性の階層は、モジュールを移動しただけで意図せず緩む。データ witness は
   **引数の型**なので、モジュール構成から独立している。
3. `IntentWitness::from_physical` が `injected == false` を条件に `Option` を返す
   形は、BUG-14（外部注入された IME モードキーが意図に昇格する）の根治で
   `RawKeyEvent.injected` を伝搬させた既存の設計と**同じ判断を型に固定する**もので
   ある。

**この §2.2 が実配線されて初めて**、§1.2 に挙げた `architecture_guard.rs` の
テキスト検査 **5 件**を削除し、1 件（`user_intent_source_...`）の期待値を
3 → 1 に縮小できる（削除の条件と時期は §7）。

### 2.3 Actuation — 型状態チェーンとフォールバック連鎖

```rust
pub struct Actuation<S> { /* private */ }

pub struct Requested;   // 要求はあるが warrant なし
pub struct Warranted;   // OpenWarrant 発行済み（ADR-087）
pub struct Verified;    // ActuationTarget capture 済み（ADR-086 INV-14）

impl Actuation<Requested> { pub fn warrant(self, w: OpenWarrant) -> Actuation<Warranted>; }
impl Actuation<Warranted> { pub async fn capture(self) -> Option<Actuation<Verified>>; }
impl Actuation<Verified>  { pub async fn run_chain(self, chain: &[WriteMechanism]) -> ImeOpenOutcome; }

pub enum WriteErr {
    /// 次の機構へフォールバックしてよい。`Actuation<Verified>` を返して
    /// 連鎖を保存する（値を落とさない）。
    Retryable(Actuation<Verified>, ImeOpenOutcome),
    /// 連鎖を打ち切る。
    Fatal(ImeOpenOutcome),
}
```

#### フォールスルー述語は現行 `apply_iter` と**同値**にする（勝手に広げない）

現行 `ImeController::apply_iter`（`ime_controller.rs:302-321`）は

```rust
let outcome = strategy.apply(open, view);
if outcome != ImeOpenOutcome::Failed { return outcome; }   // ← Failed 以外は即 return
```

であり、**次の戦略へ進むのは `Failed` のときだけ**である。したがって
`WriteErr::Retryable` を構築してよいのも `Failed` のときだけ:

| `ImeOpenOutcome` | `run_chain` の扱い |
|---|---|
| `Failed` | `WriteErr::Retryable` → 次の機構へ |
| `Applied` / `FallbackSent` / `AlreadyMatched` / `UnsafeToToggle` | 即 return（`Fatal` 相当、連鎖を打ち切る） |

**特に `UnsafeToToggle` を `Retryable` に入れてはならない。**
`UnsafeToToggle` は「Win キー押下中で `send_ime_mode_key` が未送信」を意味する
（`ime_controller.rs:123-126` / `:194-201` のコメント）。ここでフォールスルー
させると、**Win キー押下中に非冪等な `VK_KANJI`
（`KanjiToggleStrategy` → `post_kanji_toggle_to_focused`）を送る新経路**が
生まれる——現行が構造的に到達しない shadow desync 経路の新設であり、§4.5 で
却下したのと同型の誤りである。

実際に `Failed` を返す戦略は **`ImmCrossProcessStrategy` ただ 1 つ**
（`ime_controller.rs:80-84`、`set_ime_open_cross_process` の失敗）である。
`GjiDirectStrategy::apply`（`:110-127`）は `AlreadyMatched` / `Applied` /
`UnsafeToToggle` しか返さず、`MsImeDirectStrategy::apply`（`:166-214`）は
`Applied` / `UnsafeToToggle` しか返さない。**この事実が §2.8 の caps 表の
チェーン長を決める**（そこで到達不能な末尾要素を足さないこと）。

#### 不変条件の正確な文言（r2 の誤りを訂正）

r2 は「1 つの warrant = 高々 1 回の成功 write」と書いていた。**これは誤りである。**
`FeedbackPolicy::Blind { max_attempts, backoff }`（`state/ime_actuation.rs:26-29`）の
下では、`decide_actuation_action(policy, attempts)`（`:58`）が `Send` を返す限り
**同一の warrant で最大 `max_attempts` 回の成功 write が起こりうる**。

正しい文言は:

> **1 つの `Actuation` 値 = 高々 1 回の成功 write（アフィン性）。
> warrant の有効性は episode 単位。**

**型が保証するのは値のアフィン性だけであり、回数制限は
`decide_actuation_action(policy, attempts)` の責務である。** この境界を曖昧に
書くと、「型で回数が守られている」と誤読して `decide_actuation_action` の
呼び出しを省く実装が出る——ADR-080/BUG-43 の give-up が効かなくなる経路である。

#### チェーンの「入口」を数えるだけでは足りない（Phase B 追随、2026-08-12）

Phase B 実装への Opus レビューが、**新設した件数ガード 3 件がどれも
`ime_controller::apply_mechanism(mechanism, open, view)` の呼び出し元を数えて
いない**ことを指摘した。`legacy_unwarranted_actuation_sites_are_accounted_for` は
`Actuation` の**起案数**を、`async_imm_cross_actuation_goes_through_the_single_chain_entry`
は**非同期入口数**を数えているだけで、`apply_mechanism` はチェーンを 1 つも
構築せずに 1 機構分の実 write（`SendInput` / `post_kanji_toggle_to_focused` /
`ImmSetOpenStatus`）を起こせる `pub(crate)` 関数のまま残っていた。

実コードで確認した呼び出し元は 2 箇所で、どちらも
`MechanismWriter` / `AsyncMechanismWriter` の `write` 実装——すなわち
`run_chain` / `run_chain_async` が駆動する **write ステップそのもの**である
（`ime_controller.rs::SyncChainWriter::write` と
`runtime/open_chain.rs::fallback_write`）。したがって「チェーン経由へ書き換える」
ことは定義上できない（実装の中でチェーンを再度張ると再帰する）し、別モジュール
から呼ぶ以上 `pub(crate)` 未満にも絞れない。**件数ガードで固定する**のが
Phase B での結論である:

- `raw_mechanism_write_sites_are_confined_to_chain_writers`（新設）——
  `apply_mechanism(` の本番呼び出し元を 2 ファイル各 1 件に固定し、さらに
  それぞれが上記 writer 実装の中にあることまで確認する。

**並行する裏口も同時に塞いだ**: `ImeOpenStrategy` トレイトと 4 戦略構造体は
`pub(crate)` だったため、crate 内のどこからでも
`GjiDirectStrategy.apply(open, &view)` と書けば `apply_mechanism` すら経由せずに
同じ実 write を起こせた。`ime_controller.rs` の外に参照が無いことを確認して
**モジュール private へ縮小した**（可視性はコンパイラが強制するため、ガードは
「宣言を再び `pub` へ広げないこと」だけを見る）。

型で完全に閉じる案（`run_chain` だけが構築できる authorization トークンを
`MechanismWriter::write` の引数に通し、`apply_mechanism` がそれを要求する）は
**採らなかった**——writer トレイトのシグネチャ変更は §7 の `compile_fail`
doctest（ケース1 とその「通る双子」）まで波及し、`caps(p, k).chain` を導入する
Phase C（§2.8）が同じ場所をもう一度触る。§9-15 に残す。

### 2.4 `GjiFsm` 同期義務 — legacy 等価（outcome 軸のみ）に戻す

#### r2 の K 軸ゲートを全面撤回する

r2 は同期義務のゲートを `(k == ImeKindId::Gji && outcome != UnsafeToToggle)` と
していた。**r3 でこれを全面撤回する。**

理由: §1.3(g) のとおり `ImeKindId` は `gji_monitor_ok` 由来の**推定値**である。
GJI 起動直後・フォーカス直後の未検出ウィンドウでは GJI 環境でも `MsIme` と
誤判定し、その結果 `gji_sync = None` となって `GjiFsm` が `ImeOn` を見落とす。
これは `state/gji_direct_mechanism.rs:146-148` が「**belief を actuate 抜きで ON に
する高速パスが `GjiFsm` 同期を踏み抜く BUG-18/22 型の再発条件そのもの**」と警告
している失敗モードを、profile 軸ではなく K 軸で再生産するものである。

**無条件同期が無害である根拠**: `GjiEvent::ImeOn` は実測 `gji_idle_ms` と
`injection_mode` を取り、`GjiFsm` 側で自己ゲートする。MS-IME 環境で `ImeOn` を
渡してもコストはゼロで、副作用もない。**推測値でゲートして落とすリスクのほうが
一方的に大きい。**

#### 採用する形

**`&mut GjiFsm` は受け取れない。** §1.3(f) のとおり `GjiFsm::on_sync` は存在せず、
`GjiFsm` 本体は `output.warmup_coord.tsf_warmup`（`RefCell`）の中にあり、1 回の
同期は `output.gji_on_event(..)` の `Response<GjiAction, GjiTimer>` を
`dispatch_gji_response` へ流すところまでを含む。実装可能なシグネチャは
**「同期の実行口を trait で受ける」**形である:

```rust
// state/gji_direct_mechanism.rs（ungated。ADR-065 に従い GjiFsm へ依存しない）

/// `GjiFsm` 同期の実行口。Windows 側の実装だけが実 FSM に触れる。
pub trait GjiSyncSink {
    fn sync_gji(&mut self, sync: GjiFsmSync);
}

#[must_use]
pub struct ActuationReceipt {
    outcome: ImeOpenOutcome,
    want: bool,
    settled: bool,
}

impl ActuationReceipt {
    /// 同期義務の導出は `legacy_gji_sync_obligation` に委ねる（式を二重に書かない）。
    pub fn settle<S: GjiSyncSink + ?Sized>(&mut self, sink: &mut S) {
        if let Some(sync) = legacy_gji_sync_obligation(self.want, self.outcome) {
            sink.sync_gji(sync);
        }
        self.settled = true;
    }
}

impl Drop for ActuationReceipt {
    fn drop(&mut self) {
        debug_assert!(self.settled, "ActuationReceipt が settle されずに drop された");
    }
}
```

```rust
// platform.rs（windows-gated）。既存の :879-891 の分岐がそのまま impl になる。
impl GjiSyncSink for WindowsPlatform {
    fn sync_gji(&mut self, sync: GjiFsmSync) {
        match sync {
            GjiFsmSync::OnImeOn => {
                let mode = self.output.injection_mode;   // ← settle 時点の値（現 :883）
                self.gji_on_ime_on(mode);                // 現 :884
            }
            GjiFsmSync::OnImeOff => self.gji_on_ime_off(), // 現 :890
        }
    }
}
```

**設計上の細目（実装時に迷わないよう ADR に明記する）**:

1. **導出式は legacy と同一**: `outcome != UnsafeToToggle`。
   `state/gji_direct_mechanism.rs:159` の `legacy_gji_sync_obligation` を
   **直接呼ぶ形**で保証する（式を二重に書かない）。なお
   `GjiFsmSync::for_open`（`:62`）はモジュール private な `const fn` であり、
   receipt 側から直接は呼べない——`legacy_gji_sync_obligation` が唯一の入口で
   あることは、現行コードの可視性ですでに担保されている。
2. **`injection_mode` は receipt にも `settle` の引数にも積まない。**
   `sink.sync_gji` の実装内で `self.output.injection_mode`（`platform.rs:883`）を
   読む。これで **settle 時点の値が正** という要件を満たしつつ、ungated 側
   （`state/`）が `InjectionMode`（windows 側の型）に依存せずに済む。
   receipt 生成時に積むと、actuation 中に mode が変わった場合に古い値で同期する。
3. **receipt は `WindowsPlatform` のフィールドに持たせない。**
   `receipt.settle(&mut platform)` は receipt と platform の 2 つの可変借用を
   同時に取る。receipt を platform 内に格納すると `&mut self` から
   `&mut self.receipt` を切り出す形になり借用検査に落ちる。**receipt は
   actuation を起動した呼び出しフレームのローカル値として持ち、同じフレームで
   settle する**（`on_ime_applied` 相当の呼び出し元）。これは §2.3 のアフィン性
   （1 値 = 高々 1 回の write）とも整合する。
4. **`settle(self)` による consume 型は採れない。** `Drop` を実装した型は
   フィールドを move できず、`self` を consume するメソッドで
   `ManuallyDrop`/`mem::forget` を使う形になる。**`settled: bool` フラグ +
   `Drop` での `debug_assert` パターン**を採る（`ManuallyDrop` 不要）。
   この選択理由を書き残さないと、次の担当者が「`settle(self)` のほうが綺麗だ」と
   書き換えて `Drop` と衝突する。
5. **`ImeProfileDriver::uses_gji_direct` は撤去する**（`state/ime_profile_driver.rs:118`
   の trait メソッドと 2 つの impl）。同期義務は profile 軸でも K 軸でもなく
   **outcome 軸**であることが確定した以上、`uses_gji_direct` は
   ADR-081 Phase 1e ブロッカー（§1.3(f)）を宣言するためだけに存在する概念になる。
   撤去は §6 の「ADR-081 Phase 1d 凍結」とセットで行う。

### 2.5 `FeedbackPolicy` と `AppImePolicy` 残余の行き先

- **`feedback` の値を決める場所は `caps(p, k).feedback` ただ 1 つにする**
  （現行の `AppImePolicy::from_profile`（`app_ime_policy.rs:78-118`）が唯一の
  決定点であるのと同じ性質を保つ）。ただし現時点で K（`Gji` / `MsIme`）は
  すべての交差で同値であり、K で分岐しない（§2.9 の原則 P20 に従い、推測値で
  分けない）。

  **「決定点が 1 つ」＝「読み手が 1 つ」ではない。** 実際の読み手は複数ある
  （§1.3(d)、`ae64431d` 時点で全数確認済み）:

  | 値 | 本番の読み手 |
  |---|---|
  | `default_feedback` | `state/open_warrant.rs:204`（Step 4c の `OwnSsot` 分岐）/ `state/platform_state.rs:492-493`（アクセサ）/ `runtime/ime_refresh.rs:600`（アクセサ経由） |
  | `focus_settle_ms` | `state/ime_model.rs:508`（`settle_until` 算出）/ `state/platform_state.rs:370`・`:485-486`（アクセサ）/ `runtime/mod.rs:507`（アクセサ経由、`+50ms` で refresh を再スケジュール） |

  **Phase C の作業には、これらの読み手を `caps` 経由へ寄せる（または
  `AppImePolicy` を `caps` の薄いファサードに退化させる）ことを含める。**
  寄せずに `caps` を新設すると、`AppImePolicy` と `caps` の二重 SSOT に
  なる——ADR-081 の `ImeProfileDriver` が `AppImePolicy` と parity テストで
  同期を取り続けている（`ime_profile_driver.rs:467-486`）のと同じ負債を、
  3 本目として増やすことになる。

- **`caps` を K 軸で引くと、`AppImePolicy` の意味論が変わる（見落としやすい）。**
  現行 `ImeModel::app_policy` は `FocusChanged` の腕（`state/ime_model.rs:489`
  `self.app_policy = AppImePolicy::from_profile(profile)`）でのみ更新される
  **profile 由来のスナップショット**であり、フォーカスが変わるまで不変である。
  一方 `ImeKindId` は `gji_monitor_ok` 由来の実行時観測（§1.3(g)）で、
  **同一フォーカス中に反転しうる**。`caps(p, k)` を導入すると
  `focus_settle_ms` / `default_feedback` が「フォーカス中に変わりうる動的値」に
  なり、`settle_until`（既に計算済みの絶対時刻）や `Blind { max_attempts }` の
  試行カウントと組み合わさったときの挙動が現行と変わる。
  **Phase C ではまず K 非依存（今日の値のまま）で `caps` を導入し、
  K で分岐させる変更は別コミット・別ソークに分けること。**
- **`owns_physical_kanji` は `caps` に吸収せず `AppImePolicy` に残す。**
  §1.3(d) のとおり本番の読み手は現在ゼロだが、この軸が想定しているのは BUG-46 の
  物理キー抑止判断であり、`runtime/transport.rs` の
  `PhysicalKeyDisposition::plan`（`:134`）が `active_ime_kind`（実行時観測）と
  合わせて判断する**動的軸**である。静的 profile 軸の `caps` に入れると
  「`caps` は静的 profile 軸の表である」という意味が壊れる。
- **`actuator_kind` は廃止する。** `caps(p, k).chain` の先頭要素と情報が完全に
  重複する。本番の読み手はゼロで、唯一の読み手は `tests/golden_scenarios.rs:175`
  である（§1.3(d)）——**このテストの期待値を `caps(p, k).chain[0]` へ書き換える
  のが、`actuator_kind` 廃止に必要な作業のすべてである**（r3 が書いていた
  「`transport.rs` の参照を差し替える」は誤りだった。§1.3(d) で訂正済み）。
- **`ReadBack` の産物は観測に化けない。** 読み戻しの結果は
  `ConvergedReceipt { converged: bool, attempts: u32 }` という専用型で返し、
  `Observed<E>` にも `AnyObservation` にも**変換手段を提供しない**。
  これは ADR-080 不変条件6（「`ReadBack` の産物を観測として記録しない」）を
  型で表現したものであり、BUG-33 型の「収束偽装」——give-up したのに観測を書いて
  収束したように見せる——を構造的に不可能にする。

### 2.6 再試行ループ

`DriftEpisode`（**新設**、§1.3(i)）が attempt ごとに `Actuation<Warranted>` を
**新規生成**し、`capture → verify → run_chain` を再実行する。

```rust
impl DriftEpisode {
    /// attempt ごとに新しい Actuation を作る。warrant は episode 単位で有効。
    pub fn next_attempt(&mut self) -> Option<Actuation<Warranted>>;
}
```

`Actuation` 値のアフィン性（§2.3）と、`decide_actuation_action` による回数制限が
ここで組み合わさる。**`Actuation` 値を使い回さない**ことがアフィン性の実効条件で
ある。

### 2.7 Charset — 型状態化しない

`CharsetSlot`（ADR-088 の `CharsetOwner` に対応する保持側）は**型状態にしない**。
private フィールド + `reclaim(&IntentWitness)` という**メソッド境界**で守る。

理由: charset の所有権は「ユーザーが掌握しているか」という**動的に反転する状態**
であり、型パラメータで表現すると `CharsetSlot<UserOwned>` と
`CharsetSlot<AwaseOwned>` の間を実行時に行き来するために毎回 move が必要になる。
これは所有権を持つ側（`ConvModeMgr`）の構造を型パラメータで汚染する割に、
`reclaim` が `&IntentWitness` を要求するだけで同等の保護が得られる。
**型状態は「一方向に進む段階」に効く（§2.3 の Requested → Warranted → Verified）
のであって、「反転する状態」には効かない。**

### 2.8 capability 分岐 — **const 表**（trait 静的分岐は却下）

```rust
// state/app_ime_policy.rs（ADR-088 の AxisCapability と同居する。命名で区別すること）

pub struct Caps {
    pub chain: &'static [WriteMechanism],
    pub feedback: FeedbackPolicy,
    pub focus_settle_ms: u64,
}

pub const fn caps(p: ImePolicyProfile, k: ImeKindId) -> Caps { /* match */ }
```

#### 表の内容（3 フィールド全部を埋める。`Plain`/`Unknown` 行は `ImmCross` と同一）

`feedback` / `focus_settle_ms` の実値は
`AppImePolicy::from_profile`（`state/app_ime_policy.rs:78-118`）から 1 行ずつ
転記したものである（`ae64431d` 時点で照合済み）。`BLIND` は
`FeedbackPolicy::Blind { max_attempts: 5, backoff: DRIFT_CORRECTION_THRESHOLD_MS }`
（`IME_ACTUATION_BLIND_MAX_ATTEMPTS = 5`、`app_ime_policy.rs:24`）、
`READ` は `FeedbackPolicy::Read { source: ImmGetOpenStatus, deadline: DRIFT_CORRECTION_THRESHOLD_MS }`
の略。**`feedback` / `focus_settle_ms` は現時点で K に依存しない**（§2.5）。

| profile | K | `chain` | `feedback` | `focus_settle_ms` |
|---|---|---|---|---|
| `ImmCross` / `Plain` / `Unknown` | `Gji` | `[ImmCross, GjiDirect]` | `READ` | 100 |
| `ImmCross` / `Plain` / `Unknown` | `MsIme` | `[ImmCross, KanjiToggle]` | `READ` | 100 |
| `Imm32Unavailable` | `Gji` | `[GjiDirect]` | `BLIND` | 500 |
| `Imm32Unavailable` | `MsIme` | `[MsImeDirect]` | `BLIND` | 500 |
| `TsfNative` | `Gji` | `[GjiDirect]` | `BLIND` | 200 |
| `TsfNative` | `MsIme` | `[MsImeDirect]` | `BLIND` | 200 |

**`ImmCross × MsIme` に `MsImeDirect` を入れない理由**:
`MsImeDirectStrategy::is_applicable` の条件は
`active_ime_kind == MicrosoftIme && !can_use_imm32_cross_process()` である
（`ime_controller.rs:159-164`、`tests/ime_key_sequence_golden.rs` の `KEY_DOC`
にも明記）。`ImmCross` プロファイルでは `can_use_imm32_cross_process()` が真
なので、`MsImeDirect` は適用されない。
**この行に `MsImeDirect` を足すと、現行が到達しない経路を新設することになる。**

**`GjiDirect` / `MsImeDirect` の後ろに `KanjiToggle` を置かない理由
（r3 案の訂正）**:
§2.3 のとおり現行 `apply_iter`（`ime_controller.rs:302-321`）は **`Failed` の
ときだけ**次の戦略へ進み、`Failed` を返す戦略は `ImmCrossProcessStrategy`
（`:80-84`）だけである。`GjiDirectStrategy::apply`（`:110-127`）と
`MsImeDirectStrategy::apply`（`:166-214`）は `Failed` を返さない
（`Applied` / `AlreadyMatched` / `UnsafeToToggle` のみ）。したがって
これらの後ろに置いた `KanjiToggle` は **現行では到達不能**であり、

- そのまま書くと「表に載っているのに永久に使われない行」という誤読の種になり
  （§4.1 却下理由3 で「一覧できる const 表」を採った利点を自ら潰す）、
- `WriteErr::Retryable` の対応表を広げて到達させると、**Win キー押下中
  （`UnsafeToToggle`）に非冪等な `VK_KANJI` を送る新経路**になる（§2.3）。

`KanjiToggle` が実際に到達するのは **`ImmCross × MsIme`** の 1 組だけであり
（`ime_controller.rs:218-228` の doc「Standard プロファイル × MS-IME ×
`apply_skipping_imm`」と一致）、表もそのとおり 1 箇所にだけ置く。
**将来 `GjiDirectStrategy` / `MsImeDirectStrategy` が `Failed` を返すように
変わったときに初めて、`KanjiToggle` を末尾に足すかどうかを実機ソーク付きで
判断する**（`.claude/rules/fix-requires-evidence.md` の「キー選択」ファミリー）。

#### 表に必ず添える注記（この注記が無かったことが r2 の誤りを生んだ）

> **`Plain` / `Unknown` 行は現時点で構造的に到達不能である**（`ae64431d` 時点、
> §1.3(e)）。実行時に profile を供給するのは
> `impl From<AppImeProfile> for ImePolicyProfile`（`focus/class_names.rs:196`）
> だけで、`AppImeProfile` は 3 値（`Standard` → `ImmCross`）。
> `ImeModel` の初期値も `AppImePolicy::standard()` =
> `from_profile(ImmCross)`（`state/ime_model.rs:205`、`app_ime_policy.rs:122`）で
> ある。`Plain`/`Unknown` の 4 交差（2 profile × 2 K）は「将来 `Plain` が配線
> されたときのための安全デフォルト」であり、
> `app_ime_policy.rs:82-90` の既存の腕と**同一の内容でなければならない**。
> `ImmCross` と別扱いにすると、起動直後 × MS-IME で非冪等な `VK_KANJI`
> （`KanjiToggle`）へ直行する shadow desync 経路が生まれる。

#### キー値は `caps` に持たせない

`key_sequence_policy::ime_key_for`（`state/key_sequence_policy.rs:106`）が
キー値の SSOT のままとする。§1.3(a) のとおり `KeyMechanism` は 2 値
（`GjiDirect` / `MsImeDirect`）しかカバーせず、`ImmCross`（VK を送らない）と
`KanjiToggle`（`VK_KANJI` 直送）は含まれない。**`caps` にキーを持たせようとすると
この非対称を `Option<VkCode>` で表現することになり、`docs/experiments.md`
エントリ01（IME OFF キーが 5 日間で 6 回反転した）の回帰検知点が
`ime_key_for` と `caps` の 2 箇所に分裂する。**

#### K 軸の型

`state::ime_kind::ImeKindId { Gji, MsIme }` を **ungated（`#[cfg(windows)]` なし）**
で新設する。`tsf::observer::ActiveImeKind`（`pub(crate)`、windows-gated、
`tsf/observer.rs:498`）は Windows 専用なので、Linux で `caps` を全数テストする
ためには ungated な対応型が要る。両者の変換は runtime 境界で 1 箇所に置く
（`focus/class_names.rs:196` の `From<AppImeProfile>` と同じ形）。

### 2.9 型で保証できる範囲との境界

§2.4 の K 軸ゲート撤回を、**一般則**として書き残す:

> **推測値に、安全側でないゲートを掛けない。**
> `ImeKindId::MsIme` は「MS-IME を観測した」ではなく「GJI を検出できなかった」で
> ある（§1.3(g)）。K で分岐してよいのは「**誤っても被害が対称な選択**」だけで
> ある。

**今日 `GjiDirect` と `MsImeDirect` が同一 VK（`VK_IME_ON` / `VK_IME_OFF`）を
送るのは偶然である**（`key_sequence_policy.rs:110-116`）。キーが再び分岐したら、
K の誤判定は非対称な被害になる——そしてそれを検出できるのは
`tests/ime_key_sequence_golden.rs` の golden と実機ソークだけであり、型では
検出できない（`docs/experiments.md` エントリ01 が実証済み）。

**型の外に残るもの（本 ADR のスコープ外）**:

- **修飾キー汚染**（ADR-088 トラック B）。「Ctrl が押されている最中に VK を送ると
  OS が何をするか」は純粋関数に切り出せない。
- **SendInput の到達性**（ADR-088 トラック D）。「API が成功を返したが入力が
  届かない」は型でも純粋関数でも表現できない。

---

## 3. 原則

（P1〜P5 は ADR-084、P6〜P10 は ADR-086、P11〜P16 は ADR-087、P17〜P18 は
ADR-088 が使用済み。本 ADR は P19 から採番する。）

### P19: 排他制約はコヒーレンス（1 型 1 impl）に担がせる。リストを golden で固定するのは最後の手段である

「A にも B にも属さない／両方に属さない」という排他は、**関連型**で表現すれば
型検査器が保証する（§2.1）。sealed trait を 2 本に分けて「両方に impl されて
いないこと」を golden で固定する形は、**置換対象の規律（テキスト検査）を別の場所に
移しただけ**であり、非連続な改善にならない。

**適用の限界**: この手法が効くのは「1 つの型がどちらか一方に属する」形の排他だけ
である。「1 つの値が状態を行き来する」形（§2.7 の charset 所有権）には効かない。

### P20: 推測値に、安全側でないゲートを掛けない

§2.9 のとおり。**この原則の実効的な判定手順**: あるゲートを足そうとしたとき、
「そのゲートが**誤って閉じた**ときに何が起きるか」を先に書く。答えが「同期が
落ちる」「復旧操作が効かなくなる」なら、そのゲートは推測値で駆動してはならない。

ADR-088 P18（SafetyValve と fresh intent は所有権ゲートより先に評価する）と同じ
形の原則である——**ゲートは「閉じ損ねる」より「開き損ねる」ほうが害が大きい**。

### P21: 型状態は「規律の総量」ではなく「実際に踏んだ失敗モード」に投下する

trait 静的分岐を却下した理由の一般形（§4.1）。「型で表現できる」は
「型で表現すべき」を意味しない。投下の判断基準は次の 2 つを**両方**満たすこと:

1. その規律が**現在テキスト検査 / レビュー / 実機ソークでしか守られていない**こと。
2. その規律を**実際に破ったバグが存在する**こと（`docs/known-bugs.md` /
   `docs/experiments.md` に記録があること）。

`caps` の分岐は 1 は満たすが 2 を満たさない（profile 分岐そのものを取り違えた
バグは記録に無い）。§2.0 の表の 3 箇所はいずれも 2 を満たす
（BUG-19 / BUG-33 / BUG-18・22）。

---

## 4. 検討した代替案と却下記録

`.claude/rules/experiment-logging.md` の教訓（「良いアイデアに見えるか」ではなく
「過去にどの条件で壊れたか」で評価する）に従う。

### 4.1 却下: capability の trait 静的分岐（**再提案禁止**）

**r1〜r2 の設計はこれを採っていた。r2 の終盤にユーザーとの認識合わせで方向転換し、
r3 で正式に却下した。**

#### 却下された案

```rust
// 却下された形
trait ProfileCaps {
    const CHAIN: &'static [WriteMechanism];
    const FEEDBACK: FeedbackPolicy;
    fn actuate(...) -> ImeOpenOutcome;  // 静的ディスパッチ
}
struct ImmCrossProfile;
struct Imm32UnavailableProfile;
struct TsfNativeProfile;
impl ProfileCaps for ImmCrossProfile { ... }
// 呼び出し側は fn apply<P: ProfileCaps>(...) でジェネリック化
```

#### 却下の理由

1. **profile は実行時に決まる。** `AppImeProfile` はフォーカス変更のたびに
   ウィンドウクラス名から分類される（`focus/class_names.rs`）。型パラメータに
   するには、結局どこかで `match profile { ... => apply::<ImmCrossProfile>(..) }`
   という**実行時 dispatch を書かざるを得ない**。分岐は消えず、
   **場所が移動して 1 段増えるだけ**である。
2. **ジェネリックが呼び出し側へ伝染する。** `apply<P>` を呼ぶ側もジェネリックに
   なるか `Box<dyn>` に落ちる。`Box<dyn>` にするなら静的分岐の利点（インライン化・
   const 展開）が消え、trait object の分岐と const 表の分岐で**何も変わらない**。
3. **「この profile ではこの機構」という知識が impl ブロックへ散る。**
   現在 `app_ime_policy.rs:79-120` の 1 つの `match` で一覧できるものが、
   3〜5 個の impl ブロックに分かれる。§1.3(e) で判明したような
   「`Plain`/`Unknown` 行が既存の安全デフォルトを落としている」という誤りは、
   **一覧できる const 表なら 1 行ずつ突き合わせて検出できる**が、impl が散って
   いると気づけない。**r2 の誤りは、まさに表が散っていく方向の設計をしている
   最中に起きた。**
4. **P21 を満たさない。** profile 分岐そのものを取り違えたバグは
   `docs/known-bugs.md` に記録が無い。実際に何度も踏んでいるのは
   「observation の出自の偽装」（BUG-19）「give-up 後の観測書き込み」（BUG-33）
   「`GjiFsm` 同期の欠落」（BUG-18/22）であって、profile 分岐ではない。
5. **ADR-081 が同じ方向で一度止まっており、その過程で trait 設計の前提の誤りが
   判明している。**（この理由は r3 まで**事実誤認を含んでいた**ので、`git log` と
   ADR-081 のステータス節で裏取りして書き直した。）

   正しい経緯:

   - ADR-081 は `ImeProfileDriver` trait + 3 ドライバという形で trait 分岐を
     試験実装した。Phase 0 が `4af265f7`（2026-07-25）、Phase 1a/1b/1c が
     `bc3d13e8`〜`57a5a908`（同 2026-07-25）、以後の追記が `ceb99d18`
     （2026-08-01）と `2f317552`（2026-08-03）。**本 ADR 起票時点で経過は
     約 2〜3 週間であり、「1 年近く未配線」は誤りである。**
   - **未配線が続いている理由は trait 設計の失敗ではない。** ADR-081 の
     ステータス節は「Phase 1d（実機ソーク必須の strangler-fig 配線）・1e は
     未着手 —— **このサンドボックスには Windows 実機（wine）が無く実行できない**
     ため、次に Windows 実機セッションが取れたタイミングで着手すること」と
     明記している。停滞理由は**実機の不在**という環境要因である
     （同じ制約が本 ADR §6 Phase C の着手条件にもかかっている）。
   - **一方で、Phase 1d 検討（ADR-081「Phase 1d 検討・実施記録（2026-08-02）」
     節、コミット `2f317552`）が発見した設計欠陥は本物である**:
     「同期義務を profile 軸（`uses_gji_direct()`）で宣言する」という trait
     設計の前提が誤りで、実際の同期条件は outcome 軸だった（§1.3(f)、
     `state/gji_direct_mechanism.rs:134-157` が非対称としてテストで固定済み）。

   **却下理由として有効なのは後者だけである**——「trait にしたら止まった」では
   なく「**trait の宣言軸として選んだ profile が、守るべき不変条件の軸と
   一致していなかった**」。これは理由 1〜4（実行時 dispatch は消えない／
   ジェネリックが伝染する／表が散る／P21 を満たさない）とは独立の、
   **本リポジトリで実証済みの失敗**である。理由 1〜4 は事実関係の訂正を受けず、
   **結論（trait 静的分岐の却下）は維持する。**

#### ユーザーとの認識合わせ（そのまま記録する）

> **trait 静的分岐は今回の問題の本質的な解ではなく、局所的に効果があるところ
> だけ使えばいい。**

本 ADR の §2.0 の 3 箇所（プール分離 / Actuation チェーン / 同期義務の
アフィン型化）は、この「局所的に効果があるところ」の具体化である。
`caps` は const 表のままにする。

#### 再提案の歯止め

**「capability を trait にすれば分岐が消える」という直感は、profile が実行時に
決まる以上、必ず失敗する。** 再提案する場合は、少なくとも次の 2 つに答えること:

- 実行時の profile 値から型パラメータへ落とす dispatch は**どこに 1 箇所だけ**
  置くのか。それは現在の `match` より一覧性が高いのか。
- ADR-081 Phase 1d 検討が発見した「同期義務の宣言軸として profile を選んだのが
  誤りだった」（`state/gji_direct_mechanism.rs:134-157`）に対して、その trait は
  **どの軸で capability を宣言するのか**。その軸が実行時観測（`ImeKindId` 等）で
  ある場合、P20（推測値に安全側でないゲートを掛けない）をどう満たすのか。

### 4.2 却下: sealed trait 2 分割 + golden による impl 一覧の固定（r2 → r3 で撤回）

§2.1 に記載のとおり。**却下の核心は「sealed は crate 内の二重 impl を防がない」
という Rust の言語仕様上の事実**であり、設計の好みではない。
「golden で impl 一覧を固定する」案は、本 ADR が置き換えようとしている
テキスト検査の規律を別の場所に移すだけで自己矛盾である（P19）。

### 4.3 却下: `GjiFsm` 同期義務を K 軸（`ImeKindId`）でゲートする（r2 → r3 で全面撤回）

§2.4 に記載のとおり。**`ImeKindId` は推測値であり、誤って `MsIme` になった
瞬間に `GjiFsm` 同期が落ちる**（§1.3(g)）。これは
`state/gji_direct_mechanism.rs:146-148` が「ADR-081 Phase 1e ブロッカー」として
警告している BUG-18/22 型の再発条件を、profile 軸ではなく K 軸で再生産する。

**教訓の一般形は P20**。ADR-081 Phase 1d が profile 軸で同じ失敗をしており、
本件はその 2 例目である——**「同期義務をどの軸で宣言するか」を静的な分類軸
（profile / IME 種別）に求める方向は 2 回連続で失敗した。答えは outcome 軸で
ある。**

### 4.4 却下: `ActuationReceipt::settle(self)` による consume 型

`Drop` を実装した型はフィールドを move できないため、`self` を consume する
`settle` は `ManuallyDrop` / `mem::forget` を要する。`settled: bool` +
`Drop` での `debug_assert` のほうが単純で、未 settle の検出という目的を同等に
達成する（§2.4 の細目4）。

### 4.5 却下: `caps` の `Plain` / `Unknown` 行を `ImmCross` と別扱いにする（r2 の誤り）

§1.3(e) と §2.8 に記載のとおり。**既存の安全デフォルトを黙って落とし、
起動直後 × MS-IME で非冪等な `VK_KANJI` へ直行する shadow desync 経路を新設する
変更だった。** 表を書き換えるときは `app_ime_policy.rs:79-120` と 1 行ずつ
突き合わせること。

### 4.6 却下: プール毎に `derive` してから結果を合成する

§2.1 に記載のとおり。BeliefOnly プール単独の結論が Actuating 側の High を
上書きしうる経路が生まれ、**BUG-19 の再発条件と同型**になる。
マージ後に 1 回だけ判定する。

### 4.7 却下: `ImeProfileDriver::uses_gji_direct` の維持

§2.4 の細目5。同期義務が outcome 軸に移った以上、`uses_gji_direct` は
「ADR-081 Phase 1e ブロッカーを宣言するためだけの概念」になる。撤去する。

### 4.8 却下: `CharsetSlot` の型状態化

§2.7 に記載のとおり。**型状態は「一方向に進む段階」に効き、「反転する状態」には
効かない。**

### 4.9 却下: `caps` の GJI/MsImeDirect 行の末尾に `KanjiToggle` を置く（r3 の誤り）

r3 の caps 表は `Imm32Unavailable` / `TsfNative` の全行と `ImmCross × Gji` 行の
末尾に `KanjiToggle` を置いていた。**現行のフォールスルー述語（`Failed` の
ときだけ次へ、`ime_controller.rs:310`）では到達不能**であり、到達させるには
`UnsafeToToggle` をフォールスルー対象に含めるしかない——それは **Win キー押下中に
非冪等な `VK_KANJI` を送る新経路**である（§2.3・§2.8）。
`GjiDirectStrategy::apply` は `Failed` を返さない（`:110-127`）という事実を
確認せずに「フォールバックは長いほど安全」という直感で書いた行であり、
§4.5（`Plain`/`Unknown` 行）と**同じ種類の誤り**——既存が到達しない経路を
表の見た目の対称性のために新設する——である。

### 4.10 比較表

| 案 | 分岐の一覧性 | 実行時 dispatch | 呼び出し側への伝染 | 実バグとの対応（P21） | 判定 |
|---|---|---|---|---|---|
| **const 表 `caps(p, k)`（採用）** | 1 箇所の `match` で 10 行 | 1 段（`caps` 呼び出しのみ） | なし | — | **採用** |
| trait 静的分岐 | 3〜5 個の impl に分散 | 1 段（型パラメータへの落とし込み）+ 既存の match | ジェネリック伝染 or `Box<dyn>` | なし | **却下**（§4.1、再提案禁止） |
| `ImeProfileDriver`（ADR-081 現状） | trait + 3 impl | 未配線のため不明 | 未検証 | 同期義務の軸を誤った（§1.3(f)） | **凍結提案**（§6） |
| 現状（散在する `if profile.uses_*()`） | 一覧できない | 各所 | — | — | 置換対象 |

---

## 5. 不変条件（invariant）

ADR-084 の INV-1〜11、ADR-086 の INV-12〜19、ADR-087 の INV-20〜28、
ADR-088 の INV-29〜37 を継承し、**INV-38 から**採番する。

- **INV-38（観測プールの所属はコンパイラが保証する）**: ある evidence 型が
  Actuating プールと BeliefOnly プールの両方に属することは、
  `OpenEvidence::Pool` 関連型のコヒーレンスにより**型として表現できない**。
  この排他を sealed trait の 2 分割やテスト側の impl 一覧固定で代替しては
  ならない（§2.1、P19）。
  **`OpenEvidence` を impl するのは 9 個**（Actuating 5 + BeliefOnly 4）で
  あり、`ConvBitsInference` / `GjiIoInference` には impl しない——これらは
  `PerSourceObservations` にフィールドを持たず、`get`/`set` が None/no-op を
  返す input_mode 専用ソースである（§1.3(h)・§2.1）。impl すると
  `record_belief` が黙って no-op になるか、conv 由来の間接推測を open の
  多数決へ入れる BUG-19 型の経路を新設するかのどちらかになる。

- **INV-39（`derive_any` はマージ後に 1 回だけ判定する）**: 「全観測から open を
  導く」経路は、両プールをマージした後に High単独 → Medium無競合多数決の判定を
  **1 回だけ**行う。プール毎に判定してから結果を合成してはならない
  （BUG-19 の再発条件と同型、§2.1・§4.6）。
  提供する導出は `derive_actuating`（Actuating のみ）と `derive_any`（マージ後）の
  **2 本のみ**とする。
  **両者の戻り値は `Option<DeriveOutcome>` とし、`Option<bool>` へ縮めない**
  ——`state/open_warrant.rs:160-180` が `WarrantBasis::DirectRead` /
  `Corroborated` / `SingleIndirect` の構築に「どのソースが決定打だったか」を
  必要とする（ADR-087、§1.3(h)・§2.1）。

- **INV-40（`Observed<E>` はデータ witness 経由でしか構築できない）**:
  `Observed<E>` のフィールドは private とし、構築子はソースごとに固有の
  データ witness（`&AcceptedObservation` / `ImePolicyProfile` / `ConvBits` /
  `&RawKeyEvent`）を引数に要求する。可視性（`pub(in ...)`）だけに頼っては
  ならない——Windows-gated な `hook.rs` と Linux テストが別経路になる（§2.2）。
  `IntentWitness` は `from_physical`（`UserIntentSource::PhysicalImeKey`）と
  `from_sync_key`（同 `SyncKey`）の **2 本**を持ち、どちらも
  `injected == true` のイベントに対して `None` を返す（BUG-14 の型化）。
  **`UserIntentSource::Command` は witness 化の対象外**であり、
  `user_intent_source_construction_is_limited_to_typed_writers`
  （`tests/architecture_guard.rs:628`）を期待値 1 に縮小して残すこと
  （§2.2・§7・§9-8）。**ガードを削除してはならない。**

- **INV-41（`Actuation` 値のアフィン性と、回数制限の帰属）**:
  **1 つの `Actuation` 値からは高々 1 回しか成功 write が起きない**（アフィン性）。
  warrant の有効性は episode 単位であり、**同一 warrant での複数回 write は
  `FeedbackPolicy::Blind { max_attempts }` の下で正常に起こりうる**。
  **回数制限は型ではなく `decide_actuation_action(policy, attempts)`
  （`state/ime_actuation.rs:58`）の責務である。**「型が回数を守っている」と
  読み替えて `decide_actuation_action` の呼び出しを省いてはならない
  （ADR-080 / BUG-43 の give-up が無効化される、§2.3）。

- **INV-42（`GjiFsm` 同期義務は outcome 軸のみで決まる）**: 同期義務の導出式は
  `outcome != ImeOpenOutcome::UnsafeToToggle` であり、
  `state/gji_direct_mechanism.rs:159` の `legacy_gji_sync_obligation` と
  **同一である**（式を二重に書かず、その関数を呼ぶ）。
  **profile 軸（`uses_gji_direct()`）でも K 軸（`ImeKindId`）でもゲートしては
  ならない**（§2.4・§4.3。ADR-081 Phase 1d が profile 軸で、本設計の r2 が
  K 軸で、同じ失敗を 2 回している）。
  **`settle` は `&mut GjiFsm` を取らない**——`GjiFsm::on_sync` は存在せず、
  1 回の同期は `output.gji_on_event(..)` → `dispatch_gji_response(..)` まで
  含むため `&mut WindowsPlatform` 相当が要る（§1.3(f)）。ungated 側は
  `GjiSyncSink` trait で受け、`injection_mode` は sink 実装内で
  `output.injection_mode`（`platform.rs:883`）から **settle 時点に**読む
  （receipt にも `settle` の引数にも積まない、§2.4 細目2）。

- **INV-43（`ActuationReceipt` は settle されずに drop されない）**:
  `ActuationReceipt` は `#[must_use]` とし、`Drop` で `settled` を
  `debug_assert` する。`settle(self)` の consume 形は採らない（§2.4 細目4・§4.4）。
  **この不変条件の強制力は「debug ビルドでの実行時検出」+「未束縛の receipt に
  対する `#[must_use]`」までである**（release では `debug_assert` が消え、
  `let r = ...` では `#[must_use]` が発火しない、§8.1 の保証水準の注記）。
  **「型で守られている」を根拠に `platform.rs:879-891` の legacy 同期を撤去
  しないこと**（ADR-081 Phase 1e が踏みかけた BUG-18/22 型の再発条件、§1.3(f)）。
  receipt は `WindowsPlatform` のフィールドに保持せず、actuation を起動した
  呼び出しフレームのローカル値として settle する（借用衝突の回避、§2.4 細目3）。

- **INV-44（capability は const 表 1 箇所、キー値は含めない）**:
  `(profile, ime_kind)` → 戦略チェーン / feedback / settle の対応は
  `caps(p, k)` という **1 つの const fn の match** で宣言する。
  trait 静的分岐へ展開してはならない（§4.1、再提案禁止）。
  **キー値（VK）は `caps` に持たせない**——
  `key_sequence_policy::ime_key_for`（`state/key_sequence_policy.rs:106`）が
  SSOT のままであり、`docs/experiments.md` エントリ01 の回帰検知点を分裂させない
  （§2.8）。
  `Plain` / `Unknown` 行は `ImmCross` 行と**同一内容**でなければならない
  （§1.3(e)・§4.5）。
  **`Caps` の 3 フィールド（`chain` / `feedback` / `focus_settle_ms`）はすべて
  埋める**（§2.8 の表。`feedback` / `focus_settle_ms` の実値は
  `state/app_ime_policy.rs:78-118` からの転記であり、K に依存しない）。
  **`chain` に、現行 `apply_iter` のフォールスルー述語（`Failed` のときだけ
  次へ）では到達しない要素を並べてはならない**——特に `GjiDirect` /
  `MsImeDirect` の後ろに `KanjiToggle` を置かないこと。到達させるには
  `UnsafeToToggle` をフォールスルー対象に含める必要があり、それは Win キー
  押下中に非冪等な `VK_KANJI` を送る新経路の新設である（§2.3・§2.8）。

- **INV-45（`ImeKindId` は推測値であり、非対称な選択に使わない）**:
  `ImeKindId::MsIme` は「GJI を検出できなかった」の意である
  （`tsf/observer.rs:498-502`）。K で分岐してよいのは**誤っても被害が対称な選択**
  だけとする。K で分岐を足すときは、「そのゲートが誤って閉じたときに何が
  起きるか」をコード コメントまたはコミット本文に書くこと（P20）。

- **INV-46（`ReadBack` の産物は観測に変換できない）**: 読み戻しの結果は
  `ConvergedReceipt { converged, attempts }` で返し、`Observed<E>` および
  `AnyObservation` への変換手段（`From` / `Into` / コンストラクタ引数）を
  **提供しない**。ADR-080 不変条件6 の型化であり、BUG-33 型の収束偽装を
  構造的に不可能にする（§2.5）。

**明示的に却下する方向（再提案禁止）**:

- capability を trait 静的分岐へ展開すること（INV-44、§4.1）
- sealed trait の 2 分割 + golden による impl 一覧の固定（INV-38、§4.2）
- `GjiFsm` 同期義務を profile 軸 / K 軸でゲートすること（INV-42、§4.3）
- `caps` の `Plain` / `Unknown` 行を `ImmCross` と別扱いにすること（INV-44、§4.5）
- プール毎に derive してから合成すること（INV-39、§4.6）
- `CharsetSlot` の型状態化（§2.7・§4.8）

---

## 6. 移行計画

各 Phase は独立してリリース可能で、後の Phase が中止されても前の Phase は残る。

### Phase A（観測側。**Linux で全テスト可能**）

**先にやること（pinned test）**: リファクタの**前**に、
`derive_actuating ≡ 旧 derive_open_filtered(now, |s| s.authority() == Actuating)`
と `derive_any ≡ 旧 derive_open_filtered(now, |_| true)` を固定するテストを
入れる（比較は `DeriveOutcome` の等値で、§2.1）。これが無いと、プール分離が
導出結果を変えたかどうかを判定できない。

1. `state::ime_kind::ImeKindId { Gji, MsIme }` を ungated で新設し、
   `tsf::observer::ActiveImeKind` との変換を runtime 境界 1 箇所に置く（§2.8）。
2. `OpenEvidence` トレイト + `PoolKind` 関連型 + **9 個**の evidence 型
   （Actuating 5 + BeliefOnly 4）を新設し、`ObservationStore::record` /
   `record_belief` / `record_replayed` の 3 口に分ける（§2.1、INV-38/39）。
   **`ConvBitsInference` / `GjiIoInference` には evidence 型を作らない**
   （§1.3(h)・§2.1。これらは `PerSourceObservations` にフィールドを持たず、
   `InputModeObserved` 専用のソースである）。
   `derive_actuating` / `derive_any` の戻り値は `Option<DeriveOutcome>` とする
   （§2.1、ADR-087 の `WarrantBasis` 構築に必要）。
3. `Observed<E>` のデータ witness 構築子と `IntentWitness`
   （`from_physical` / `from_sync_key` の 2 本）を新設する（§2.2、INV-40）。
   `UserIntentSource::Command` は witness 化の対象外
   （§2.2「`UserIntentSource` は 3 値 — witness は 2 本」、§9-8）。
   `architecture_guard.rs:628` は削除せず期待値 3 → 1 に縮小する（§7）。
4. `AcceptedObservation::for_sync`（`state/probe_admission.rs:113`）を
   `pub(crate)` へ縮小する（§1.3(b)。呼び出し元 3 箇所はすべて `runtime` 層）。
5. `OpenEvidence::SOURCE` と `ObservationSource::authority()`
   （`state/ime_event.rs:181`）が **evidence 型 9 個の全数で**一致することを
   テストで固定する。あわせて `ConvBitsInference` / `GjiIoInference` に対して
   `PerSourceObservations::get` が `None` を返し続けることを固定する
   （既存テスト `observation_store.rs:481-487` の拡張）。

**この Phase は純粋ロジックのみで、Windows API を一切呼ばない**（ADR-087 §8 が
`issue_open_warrant()` で採った方針と同じ。ADR-088 §7.4 のとおり、実機自動テストが
使えない以上これが最も強い防御である）。

### Phase B（actuation 側）

6. `Actuation<Requested/Warranted/Verified>` チェーンと `run_chain` を新設し、
   4 戦略を async 化して二重経路（`apply` / `apply_skipping_imm`）を解消する
   （§2.3、INV-41）。
7. `ActuationReceipt` + `GjiSyncSink` trait を新設し、`platform.rs:879-891` の
   `gji_on_ime_on` / `gji_on_ime_off` 直接呼び出しを `impl GjiSyncSink for
   WindowsPlatform` へ移す（§2.4、INV-42/43）。導出は
   `legacy_gji_sync_obligation` を呼ぶ形にする。**receipt は platform の
   フィールドに持たせない**（借用衝突、§2.4 細目3）。
8. `ImeProfileDriver::uses_gji_direct`（`state/ime_profile_driver.rs:118`）を
   撤去する（§2.4 細目5、§4.7）。
9. `DriftEpisode::next_attempt` を新設する（§2.6）。
10. `ConvergedReceipt` を新設し、`ReadBack` の戻り値型にする（§2.5、INV-46）。

**Phase B に含めない分割線（意図的なスコープ限定）**:
**ImmCross の `set_ime_romaji_mode` 同期 IMC write（ADR-086 INV-14 の未移行分、
`ime_controller.rs:75` / `:182` のコメントが「`ActuationTarget` 化には
`ImeOpenStrategy::apply` 自体の非同期化が要る」と明記している）を
`ActuationTarget` 化することは Phase B に含めない。** したがって
`write_conv_then_open` のような複合操作も Phase B では導入せず、Phase C へ送る。
**理由**: ADR-086 Phase 3 が同じ作業を「スコープ超過」と判断して item 0 を
未移行のまま残しており、それを型状態リファクタに便乗して再吸収すると、
Phase B の失敗時に切り分けられなくなる。

### Phase B 実施記録（2026-08-12）

**item 6〜10 すべて実装した。** 新設 2 ファイル
（`state/actuation_chain.rs` / `runtime/open_chain.rs`）、改修 8 ファイル。
`cargo build` / `cargo test -p awase-windows`（Linux）と
`cargo xwin check --target x86_64-pc-windows-msvc`（windows-gated コードの
型検査）がグリーン。clippy の警告件数は HEAD と同数（`git stash` で比較）。

#### ADR の記述と実コードが食い違っていた点（実装時の判断）

| # | ADR の記述 | 実コード | 採った判断 |
|---|---|---|---|
| B-1 | §2.3 `async fn run_chain(self, chain) -> ImeOpenOutcome` | 実 write は Win32 FFI であり `state/` は `#[cfg(windows)]` に依存できない（ADR-065） | **writer を引数に取る形にした**（`MechanismWriter` / `AsyncMechanismWriter`）。走査・フォールスルー判定・アフィン性だけを ungated 側が持ち、Linux で全数テストする |
| B-2 | §2.3 `run_chain` は async 1 本 | GJI/MS-IME/KanjiToggle は `SendInput` のみで非ブロッキング。これらを await 越しにすると打鍵ホットパスのレイテンシが変わる（§8.2 が実機ソーク必須と書いている軸） | **同期版・非同期版の 2 本にした。判定（`Actuation::classify`）は 1 箇所に集約**。二重化しているのは「future を駆動する殻」であって、フォールスルー述語でも戦略選択でもない |
| B-3 | §2.3 `Actuation<Requested>::warrant(w: OpenWarrant)` | **`issue_open_warrant()` の本番呼び出し元はゼロ**（`src/` 全体を grep、2026-08-12。ADR-087 Phase 3 の配線が未了で、呼び出しは同ファイルのテストのみ） | 既存経路に warrant を要求すると ADR-087 Phase 3 を Phase B に巻き込む。**`warrant_pending_adr087()` という名前付きの暫定入口を分け**、件数を `legacy_unwarranted_actuation_sites_are_accounted_for`（期待値 **2**）で固定した。ADR-087 Phase 3 が進むたびにこの期待値は減るのが正しい方向 |
| B-4 | §2.3 `Actuation<Verified>` は「ADR-086 INV-14 の capture 済みターゲットを保持する」 | `crate::ime::ActuationTarget` は**フィールドを private にして「`verify_still_current` を経由せずに hwnd を取り出せない」ことを型で保証している**（ADR-086 §6 段1） | hwnd を取り出すアクセサを生やすとその保証を壊す。**`VerifiedTarget::Captured` を payload 無しにした**。VK 送信機構向けの `FocusImplicit` は INV-14 未移行分であることを型の doc に明記（Phase C で潰す） |
| B-5 | §2.4 細目5「`ImeProfileDriver::uses_gji_direct` を撤去する」 | 撤去すると `GjiDirectMechanism::access_for`（token の唯一の発行口）→ `GjiDirectAccess` → `GjiDirectMechanism::actuate` → `GjiActuation` が芋づるで根拠を失う | **連鎖して撤去した**（§4.7 が「維持」を明示的に却下しているため）。ADR-081 側にステータス追記済み。**ただし ADR-081 の凍結そのもの（§9-4）は未決定のまま**——`ImeProfileDriver` 本体と `driver_for` レジストリは残している |
| B-6 | §6 item 10「`ConvergedReceipt` を `ReadBack` の戻り値型にする」 | **`ReadBack` という型・関数は存在しない**（§1.3(i) の記述どおり） | 読み戻しの帰結が実際に確定するのは `ir_apply_drift_correction` の `Read` 収束判定と `Blind` give-up 判定の 2 箇所。**そこで `ConvergedReceipt` を構築するようにした**（INV-46 の「観測へ変換できない」は型として成立している）。**ただし receipt は制御フローに使われず `log::debug!` に渡るだけで、収束判定は従来どおり `most_recent_trusted_after` の返す `ImeObservation` が担っている**（§9-16、§8.1 の訂正） |
| B-7 | §7「新設するもの — `trybuild`（compile-fail テスト、4 ケース）」 | `trybuild` は未導入。§7 自身が「rustc 更新で `stderr` が変わり CI が赤くなる」保守負担を明記している | **`compile_fail` doctest で代替した**（stderr を照合しないため rustc バージョンに依存せず、dev-dependency も増えない）。`compile_fail` は「何らかの理由で落ちれば通る」ため、**1 行だけ違う「通る双子」を必ず併記**して、落ちている理由が目的の型エラーであることを示す形にした。ケース 1（`Warranted` から `run_chain`）・3（`ConvergedReceipt` → `AnyObservation`）・4（未束縛の `ActuationReceipt`）を実装。**ケース2（`record_belief` に `ActuatingPool` の evidence）は Phase A の範囲**であり本 Phase では追加していない |
| B-8 | §9-1「`Drop` の `debug_assert` が panic unwind 中に double panic を起こしうる。Phase B 実装時に決める」 | — | **`std::thread::panicking()` で unwind 中を除外する形を採った**。double panic → abort になると**本来の panic の原因が失われる**（`panic_detect.rs` のクラッシュ報告も元の payload を拾えなくなる）ため |
| B-9 | §9-9「receipt を持つ呼び出しフレームの特定が済んでいない」 | `platform.rs::on_ime_applied`（`&mut self` の中）が唯一の同期点 | receipt を**同メソッドのローカル値**として作り、同じフレームで `settle(self)` する形で解決した。async 境界をまたがない（async 経路も完了 outcome を WM 経由で `on_ime_apply_complete` へ返してから同期するため）ので、§9-1 の double panic と await 中断の相互作用は生じない |

#### 実装したもの

1. **`state/actuation_chain.rs`（新設、ungated）** — `WriteMechanism`（4 値、キー値は
   持たない）、`falls_through`（**フォールスルー述語の SSOT**、`Failed` のときだけ真）、
   `Actuation<Requested/Warranted/Verified>`、`WriteErr`、`VerifiedTarget`、
   `Authorization`、`MechanismWriter` / `AsyncMechanismWriter`、
   `run_chain` / `run_chain_async`、`DriftEpisode`（item 9）。
   Linux ユニットテスト 12 本（`UnsafeToToggle` が `KanjiToggle` へ落ちないこと、
   同期版と非同期版が全 outcome 組み合わせで一致すること、`DriftEpisode` が
   `Blind` の `max_attempts` で払い出しを止めること等）。
2. **`state/gji_direct_mechanism.rs`（改修）** — `GjiSyncSink` trait と
   `ActuationReceipt` を新設し、`GjiDirectAccess` / `GjiDirectMechanism` /
   `GjiActuation` を撤去（B-5）。`settle` が全 `ImeOpenOutcome` × `open` で
   `legacy_gji_sync_obligation` と一致することを全数テスト（INV-42）。
3. **`platform.rs`（改修）** — `impl GjiSyncSink for WindowsPlatform` を追加し、
   `on_ime_applied` の `gji_on_ime_on` / `gji_on_ime_off` 直接呼び出しを
   `receipt.settle(self)` へ置換（item 7）。`injection_mode` は sink 実装内で
   settle 時点に読む（§2.4 細目2）。
4. **`ime_controller.rs`（改修）** — `apply_iter` を撤去し `run_chain` へ委譲。
   `ImeController` から `strategies` フィールドが消え（`WriteMechanism::ALL` +
   `strategy_for` の写像に置換）、**`apply_skipping_imm` を撤去**。
5. **`runtime/open_chain.rs`（新設）** — **ImmCross を機構チェーンの要素にした**。
   `executor.rs` / `key_pipeline.rs` の `spawn_local` に inline されていた
   ImmCross 書き込みをここへ移し、`Failed` 後のフォールスルーを
   `run_chain_async` に任せた。これが `apply_skipping_imm`（2 本目の走査入口）が
   不要になった理由である（item 6 の「二重経路解消」）。
6. **`state/ime_actuation.rs`（改修）** — `ConvergedReceipt`（item 10、INV-46）。
   `runtime/ime_refresh.rs` の `Read` 収束 / `Blind` give-up の 2 箇所で構築。
   **構築するだけで、制御フローには配線していない**（判定は従来どおり
   `most_recent_trusted_after` の返す `ImeObservation` を見る）。§9-16。
7. **`state/ime_profile_driver.rs`（改修）** — `uses_gji_direct` 撤去（item 8）。
   contract test の不変条件4・5 を「ドライバの静的宣言が同期義務をゲートしない
   こと」の確認へ置き換え。
8. **`state/observation_store.rs`（改修、§9-11）** — `PerSourceObservations::set`
   を `pub(crate)` へ縮小し、**crate 外からの観測注入という裏口を塞いだ**。
   crate 内の本番呼び出し元 1 箇所（`record_replayed`）を
   `per_source_set_is_confined_to_the_store` が固定する。

#### 新設した architecture_guard（4 件）

- `legacy_unwarranted_actuation_sites_are_accounted_for`（期待値 2、B-3）
- `async_imm_cross_actuation_goes_through_the_single_chain_entry` —
  `apply_skipping_imm` がゼロであること、ImmCross の実書き込み API を
  チェーン外から呼ぶ箇所の固定、非同期入口 `run_open_chain_async` が 2 呼び出し
- `per_source_set_is_confined_to_the_store`（§9-11）
- `raw_mechanism_write_sites_are_confined_to_chain_writers`（**Phase B 追随、
  2026-08-12**）—— `apply_mechanism(` の本番呼び出し元 2 件（`SyncChainWriter::write` /
  `fallback_write`）の固定と、`ImeOpenStrategy` トレイト・4 戦略構造体が
  `ime_controller.rs` の外へ出ていないことの固定（§2.3「チェーンの『入口』を
  数えるだけでは足りない」）。**上 3 件はどれもチェーンの入口しか数えておらず、
  1 機構分の生 write の口が無ガードで残っていた**という Opus レビュー指摘への対応

`ime_open_actuation_entry_points_are_accounted_for` からは
`.apply_skipping_imm(` の行（期待値 2）を削除した。

#### Phase B でも消えなかったもの（Phase C / 別 ADR 送り）

- **`ImmCrossOp::Untargeted`**（`key_pipeline.rs` の shadow-toggle OFF 経路）は
  宛先を捕獲しないまま。ADR-086 INV-14 の未移行分であり、`Targeted` へ寄せると
  挙動が変わる（実機ソーク必須）ため Phase C（item 12）。
  → **Phase C でも移行しなかった**（§9-19/§9-20）。Phase C item 12 の実作業は
  `set_ime_romaji_mode()` の是正に絞り、`Untargeted` は非同期チェーンの
  chain 固定問題（§9-20）と同じ段で判断することにした。
- **`set_ime_romaji_mode()` の同期 IMC write**（`ime_controller.rs` の
  ImmCross / MsImeDirect 内）は手つかず。§6 の分割線どおり Phase C。
  → **Phase C で是正済み**（§6 Phase C 実施記録の item 12）。2 戦略から撤去し、
  `apply_mechanism` の ROMAN 補完ステップ 1 箇所へ統合したうえで
  `ActuationTarget::capture_blocking` → `set_ime_romaji_mode_for_target_blocking`
  経由にした。低レベル API（`set_ime_romaji_mode` / `_async`）は削除した。
- **`VerifiedTarget::FocusImplicit`** が `Verified` に入っていること自体が
  INV-14 の未達を表す。Phase C で潰す。
  → **訂正: 潰せない。「未達」ではなく VK 送信機構の性質である**（§9-19）。
  `SendInput` は宛先引数を取らないため、捕獲すべき hwnd が構造的に存在しない。

#### 実機検証の状態

**未実施。** 本 Phase の変更は `cargo xwin check` による型検査までしか通って
おらず、Windows 実機での動作確認はしていない（サンドボックスに実機が無い、
§6 Phase C 着手条件と同じ制約）。§8.2 が求める**レイテンシ実測も未実施**——
ただし B-2 のとおり**同期経路を async 化していない**ため、打鍵ホットパスの
await 点は Phase B 以前と同数である（ImmCross 経路は元から `spawn_local`）。

### Phase C（**ゲートあり**。実機ソーク必須）

11. `caps` 一本化 + `ime_controller.rs` の書き換え + `actuator_kind` 廃止
    （`tests/golden_scenarios.rs:175` の期待値を `caps(p,k).chain[0]` へ書き換える、
    §2.5・§2.8、INV-44）。**`default_feedback` の読み手 3 箇所
    （`open_warrant.rs:204` / `platform_state.rs:492-493` / `ime_refresh.rs:600`）と
    `focus_settle_ms` の読み手 4 箇所（`ime_model.rs:508` /
    `platform_state.rs:370`・`:485-486` / `runtime/mod.rs:507`）を `caps` 経由へ
    寄せる作業を含める**（寄せないと `AppImePolicy` と `caps` の二重 SSOT に
    なる、§2.5）。**この段階では `caps` を K 非依存のまま導入する**——K で
    分岐させる変更は別コミット・別ソークに分ける（§2.5）。
12. ADR-086 INV-14 の是正（ImmCross 同期 IMC write の `ActuationTarget` 化）。

**旧・着手条件（2026-08-12 にユーザー判断で解除）**: **ADR-088 トラック D
（実機 SendInput 検証）の復旧。** Phase C は実際に送るキーと順序を変えうるため、
実機で検証できない状態で入れてはならない（`docs/experiments.md` エントリ01 の
5 日間 6 回反転は、まさにキー選択を実機検証なしに動かした結果である）——
というのが起票時のゲートだった。

#### ゲート解除（2026-08-12、ユーザー判断）

**トラック D は原因を特定できないまま棚上げになった。** 検証した仮説は
少なくとも次の 5 つで、いずれも `SendInput` が「成功を返すのに入力が届かない」
という現象を説明できなかった:

1. `clipd`（clipwire のリモート実行デーモン）の実行コンテキストの権限
2. awase 自身の低レベルフック（`WH_KEYBOARD_LL`）による干渉
3. ウィンドウステーション / デスクトップの不一致
4. フォアグラウンド・フォーカスの不在
5. `DESKTOP_JOURNALPLAYBACK` アクセス権の欠如

**決定打は、ユーザー自身が対話的に操作し、実際にマウスでクリックして対象
ウィンドウにフォーカスを与えた状態でも `SendInput` が効かなかったこと**である。
これは 1〜4 が想定していた「`clipd` の実行コンテキスト固有の問題」という説明を
否定する——ユーザーの対話セッションは通常の対話ウィンドウステーション・
対話デスクトップ・実フォーカスを持っており、そこでも再現した以上、
原因は実行コンテキストの外側にある。**現時点で原因は不明のまま棚上げする。**

**したがって Phase C の着手条件はユーザー判断で解除する。** 方針を
「**Phase C を実装し、実機での長期ソーク（soak）テストで検証する**」に変更する。
自動化された実機 `SendInput` 検証が復旧する見込みが立たない以上、
「ゲートが満たされるまで待つ」は「永久に着手しない」と同義であり
（§9-5 が既にその可能性を指摘していた）、実装 → 長期ソークのほうが
情報を得られるという判断である。

**この方針変更が課す義務**:

- Phase C 実装のうち **どの部分が Linux 上のビルド/テストで検証済みか** と、
  **どの部分が実機ソークでしか検証できないか** を、実装完了後に本 ADR へ
  明確に区別して記録すること（§6「Phase C 実施記録」・§9-17）。
- 実機ソークで異常が出た場合の revert は
  `.claude/rules/experiment-logging.md` の義務（アプリ / IME / 再現手順）を
  必ず満たすこと（§6「revert する場合の義務」）。

**トラック D が復旧しなくても Phase B で止めれば主目的は達成する。**
BUG-19（観測の出自偽装）/ BUG-33（give-up 後の観測書き込み）/ ADR-086 INV-14
（ターゲット同一性）型の再発防止は Phase A/B で完了する。**Phase C は
「一覧性の改善」であって「再発防止」ではない**——この評価はゲート解除後も
変わらない。したがって Phase C は「実装しないと危険」ではなく
「実装しても安全であることをソークで確かめる」種類の作業である。

### Phase C 実施記録（2026-08-12）

**item 11・12 とも実装した。** 改修 9 ファイル（新設ファイルは無し）。
Linux の `cargo build` / `cargo test -p awase-windows` と、windows-gated コードの
`cargo xwin check --target x86_64-pc-windows-msvc --all-targets` がグリーン。
clippy の警告・エラー行は Linux / xwin の両方で HEAD と**完全一致**
（`git stash` 比較で diff 無し）。`cargo fmt -- --check` はグリーン
（HEAD に残っていた既存の 1 件も解消した）。

**`tests/ime_key_sequence_golden.rs` と `tests/golden/ime_key_sequences.txt` は
1 バイトも変更していない。** これは目標ではなく Phase C の必須条件として
扱った（実機検証ができない状態で「送るキーと順序」を動かさないため）。

#### ADR の記述と実コードが食い違っていた点（実装時の判断）

| # | ADR の記述 | 実コード | 採った判断 |
|---|---|---|---|
| C-1 | §7「維持するもの」: `tests/ime_key_sequence_golden.rs` に `caps().chain` の (P, K) **10 行を golden に追加する**」 | golden への行追加は golden ファイルの更新を伴い、「キー選択の回帰検知点を無変更で通す」という Phase C の必須条件と両立しない。加えて同ファイルは `#![cfg(windows)]` で **Linux では 0 テスト**であり、caps の全数検査をそこへ置いても Linux CI では実行されない | **golden には追加しなかった。** caps の全数テストは `state/app_ime_policy.rs` の `#[cfg(test)]`（**Linux で実行される**）と `ime_controller.rs` の `#[cfg(test)]`（windows-gated、`cargo xwin check --all-targets` で型検査）へ分けて置いた |
| C-2 | §2.5「`default_feedback` の読み手 3 箇所と `focus_settle_ms` の読み手 4 箇所を `caps` 経由へ寄せる」 | 読み手は `open_warrant.rs` / `platform_state.rs` のアクセサ / `ime_refresh.rs` / `ime_model.rs` / `runtime/mod.rs` に散っており、すべて `ImeModel::app_policy`（`AppImePolicy` 値）を経由する。読み手側を `caps(p, k)` の直接呼び出しへ書き換えると、**`AppImePolicy` が持っていない K を各読み手が自前で調達する**ことになり、§2.5 が警告している「`focus_settle_ms` がフォーカス中に変わりうる動的値になる」変更を読み手の数だけ作り込む | §2.5 が併記していた**もう一方の選択肢「`AppImePolicy` を `caps` の薄いファサードに退化させる」を採った**。リテラルは `caps` 側にしか無いので二重 SSOT は解消し、読み手は 1 行も触らないので挙動リスクがゼロになる。K 分岐を入れるときに改めて読み手を見直す |
| C-3 | §6 item 11「`tests/golden_scenarios.rs:175` の期待値を `caps(p,k).chain[0]` へ書き換える」 | `chain[0]` は **K 依存**（`Imm32Unavailable` は `GjiDirect`/`MsImeDirect`）だが、`ImeModel::app_policy` は `FocusChanged` 時点の **profile スナップショット**で K を持たない（§2.5）。そのまま照合できない。なお実際の行は `:175` ではなく `:186` で、`actuator_kind` を 3 variant の否定形で見るシナリオ 4 の assert だった | **`focus_settle_ms` での識別に置き換えた**（profile ごとに一意: 100/500/200）。このテストが見たいのは「reducer が FocusChanged で profile 由来のポリシーへ切り替えたか」であり、K を持ち込む必要がない。値そのものは `caps_settle_values_match_the_pre_phase_c_literals` が固定する |
| C-4 | §2.8 は `caps` を「機構チェーンの唯一の宣言」とする | `runtime/open_chain.rs::run_open_chain_async` は ImmCross の書き込みを **await した後**にフォールバックへ進み、`fallback_write` が機構ごとに view を作り直して `is_applicable` を再評価する（旧 `apply_skipping_imm` と同じ「完了時点の状態で残りを選ぶ」意味論） | **非同期チェーンだけは `WriteMechanism::ALL` のまま**にした。起案時点の `(p, k)` で chain を固定すると、await 中にフォーカスが動いた場合に「完了時点では適用可能な機構が chain に載っていない」新しい取りこぼしが生まれる（起案時 Standard×MS-IME の chain は `[ImmCross, KanjiToggle]`。await 中に TsfNative へ移ると旧実装は `MsImeDirect` を選ぶが、固定 chain では `KanjiToggle` を送る）。`ImeKindId` は推測値であり（INV-45）、await をまたいで K を固定するのは P20 が禁じる「安全側でないゲート」に当たる。§9-20 に残余論点として記録 |
| C-5 | §6「Phase B でも消えなかったもの」: 「`VerifiedTarget::FocusImplicit` が `Verified` に入っていること自体が INV-14 の未達を表す。**Phase C で潰す**」 | VK 送信機構（`GjiDirect` / `MsImeDirect` / `KanjiToggle`）の実 write は `SendInput` であり、**宛先引数を取らない**（フォアグラウンドのキュー宛）。捕獲すべき hwnd が構造的に存在しない | **潰せない——`FocusImplicit` は「未達」ではなく機構固有の性質である**と訂正した（§9-19）。同期経路で hwnd を持つ唯一の write は ROMAN 補完であり、そちらを `ActuationTarget` 化した。`ImmCrossOp::Untargeted`（`key_pipeline.rs` の shadow-toggle OFF）は依然として未移行（§9-20） |
| C-6 | §6 item 12「ADR-086 INV-14 の是正（ImmCross 同期 IMC write の `ActuationTarget` 化）」/ ADR-086 と `output/conv_actuation.rs` は「`ImeOpenStrategy::apply` 自体の非同期化が要る」としていた | `ActuationTarget::capture` が実際にやっているのは `get_focused_hwnd()` **1 回**であり、旧 `set_ime_romaji_mode()` が内部で行っていたライブクエリと同一である。**「捕獲を write の外へ出す」だけなら同期のままできる** | `capture_blocking` / `verify_gen_only` / `set_ime_romaji_mode_for_target_blocking` を新設し、**`apply` を非同期化せずに** ROMAN 補完を `ActuationTarget` 経由へ移した。「非同期化が要る」という ADR-086 Phase 3 の判断は、`verify_still_current` の hwnd 再クエリまで必須と読んだ場合にのみ正しい |
| C-7 | — | 同期経路の `ImmCrossProcessStrategy::apply` の到達可能性を呼び出し元で追跡した（`executor.rs` / `key_pipeline.rs` は `imm_cross_is_first_applicable` で async 分岐、`apply_force_on_for_imm_broken` と `arm_force_open_pending` は `!can_use_imm32_cross_process()` を要求、`ir_apply_drift_correction` は `can_use_imm32_cross_process()` なら `set_ime_open` を使う分岐、`ime_refresh.rs:499` と `key_pipeline.rs:742` は TsfNative 限定） | **【2026-08-12 訂正】当初は「同期経路からは到達しない」と結論したが、これは棚卸し漏れによる誤りだった**（Opus レビューで指摘、詳細は §9-21）。`runtime/mod.rs::try_force_on_bootstrap`（`:892`）が `apply_ime_open_with_belief(true, None, belief)` を呼び、そのガードは `detect_miss_count() >= IME_DETECT_MISS_THRESHOLD` / `is_user_enabled()` / `is_eligible_for_ime_force_on()`（= `is_japanese_ime() && effective_open()`）/ `!is_force_on_guard_active()` だけで、**同種の他経路が持つ `!can_use_imm32_cross_process()` プロファイルガードを持たない**。したがって Standard（= `ImmCross` プロファイル、LINE / Qt 等）では `caps` chain の先頭 `ImmCross` が `is_applicable` を満たし、同期経路で `ImmCrossProcessStrategy::apply` に到達する。**Phase C 以前から同じ挙動であり、Phase C が作り込んだ回帰ではない**（Phase C は chain の作り方を `ALL` 走査から `caps` へ変えただけで、`try_force_on_bootstrap` のガードにも ImmCross の適用条件にも触れていない） |

#### 実装したもの

1. **`state/app_ime_policy.rs`（改修）** — `Caps { chain, feedback, focus_settle_ms }` と
   `caps(p, k)`（10 行の const match）を新設。`AppImePolicy::from_profile` を
   その薄いファサードへ退化させ、`actuator_kind` / `ImeActuatorKind` を廃止した。
   Linux ユニットテスト 8 本（表のリテラル照合 / `Plain`・`Unknown` 行の同一性 /
   K 非依存 / ファサード parity / 旧リテラルとの一致 / 到達不能な末尾要素の禁止）。
2. **`state/actuation_chain.rs`（改修）** — `WriteMechanism::may_return_failed()`
   （`ImmCross` のみ真。caps 表の末尾規則の根拠）と `needs_romaji_pre_write`
   （ROMAN 補完の発火条件、ungated）を新設。ユニットテスト 6 本追加。
3. **`ime_controller.rs`（改修）** — `apply` の chain を `caps_chain_for(view)` へ。
   `imm_cross_is_first_applicable` も caps ベース（`chain[0] == ImmCross` の
   同一性チェック付き）。`apply_mechanism` に ROMAN 補完ステップ
   （`romaji_pre_write`）を追加し、2 戦略から `set_ime_romaji_mode()` 呼び出しを
   撤去。windows-gated テスト 3 本を新設（caps chain と旧 ALL 走査の同値性 /
   chain 全要素の適用可能性 / async 分岐判定の不変性）。
4. **`ime.rs`（改修）** — `ActuationTarget::capture_blocking` / `verify_gen_only` /
   `set_ime_romaji_mode_for_target_blocking` を新設し、
   **`set_ime_romaji_mode()` と `set_ime_romaji_mode_async()` を削除**した
   （後者は移行前から呼び出し元ゼロ）。
5. **`state/ime_decision_view.rs` / `platform.rs`（改修）** — `FocusFacts` に
   `focus_gen` を追加。値は `Output::ime_mode_focus_gen`（executor の async 経路が
   `ActuationTarget::capture(focus_gen)` に渡すのと同じカウンタ）。
6. **`runtime/open_chain.rs`（改修）** — chain を `ALL` のまま維持する理由を
   モジュール doc に明記（C-4）。
7. **`tests/architecture_guard.rs`（改修）** —
   `sync_romaji_write_goes_through_a_captured_target` を新設（1 件）。
8. **`tests/golden_scenarios.rs`（改修）** — `actuator_kind` 期待値の置き換え（C-3）。

#### 新設した architecture_guard（1 件）

- `sync_romaji_write_goes_through_a_captured_target` ——
  (1) 削除したライブクエリ版（`set_ime_romaji_mode()` / `_async()`）が本番コードに
  復活していないこと、(2) `ActuationTarget::capture_blocking(` と
  `set_ime_romaji_mode_for_target_blocking(` の本番呼び出し元が
  `src/ime_controller.rs` の 1 件ずつであること、(3) その 1 件が
  `romaji_pre_write` の中にある（= `needs_romaji_pre_write` の条件判定を
  迂回しない）こと、を固定する。

#### Phase C で **Linux 上のビルド/テストで検証済み**の範囲

| 検証したこと | 手段 |
|---|---|
| `caps` 表の 10 行が ADR §2.8 の表どおりであること | `caps_chains_match_the_adr089_table`（Linux 実行） |
| `Plain`/`Unknown` 行が `ImmCross` 行と同一であること（INV-44） | `plain_and_unknown_caps_are_identical_to_imm_cross`（同上） |
| `feedback` / `focus_settle_ms` が K 非依存であること（§2.5） | `caps_feedback_and_settle_are_k_independent`（同上） |
| `caps` の値が Phase C 以前の `AppImePolicy` リテラルと一致すること | `caps_settle_values_match_the_pre_phase_c_literals` / `app_ime_policy_is_a_facade_over_caps`（同上） |
| chain に到達不能な末尾要素が無いこと（INV-44・§4.9） | `caps_chains_have_no_unreachable_trailing_element`（同上） |
| ROMAN 補完の発火条件が Phase C 以前の 2 戦略と同値であること | `romaji_pre_write_condition_matches_the_pre_phase_c_strategies`（同上、機構 4 × open 2 × K 2 × input_mode 5 の全数） |
| 書き込み口の本数（生 write / 同期 ROMAN write / チェーン入口） | `architecture_guard.rs` の件数ガード群（同上） |
| reducer が profile 由来のポリシーへ切り替えること | `golden_scenarios.rs` シナリオ 4/5（同上） |
| **caps chain と Phase B までの `ALL` 走査が同じ機構列になること** | `ime_controller.rs::caps_chain_matches_legacy_all_scan`（**windows-gated**。`cargo xwin check --all-targets` で**型検査のみ**。実行は Windows 実機/CI が要る） |
| windows-gated コード全体の型整合 | `cargo xwin check -p awase-windows --target x86_64-pc-windows-msvc --all-targets` |

#### Phase C で **実機ソークでしか検証できない**残余リスク

§9-17 に列挙した。**次にこのリポジトリを Windows 実機で確認する人は、まず §9-17
を読むこと。**

### ADR-081 Phase 1d の凍結（提案）

**`caps` へ寄せるなら、配線前の今が低コストなタイミングである。**
ADR-081 Phase 1a/1b/1c は試験実装済み・未配線であり、今なら撤去コストが
テストの削除だけで済む。配線後に撤去すると実機ソークをやり直すことになる。

**凍結を提案する根拠は「ADR-081 が長期間放置されているから」ではない**
（§4.1 却下理由5 の訂正のとおり、Phase 1a/1b/1c は 2026-07-25、最終追記は
`2f317552`（2026-08-03）で、経過は約 2〜3 週間にすぎない）。根拠は次の 2 点に
限られる:

1. **capability の表現手段が重複する。** ADR-081 の trait と本 ADR の `caps` は
   同じ (profile → 挙動) の対応を 2 通りに書くことになる。ADR-081 側は既に
   `AppImePolicy` との parity テスト（`ime_profile_driver.rs:467-486`）で
   drift を防いでいる状態であり、`caps` を足すと SSOT が 3 本になる（§2.5）。
2. **`uses_gji_direct()` の根拠が消える。** 同期義務が outcome 軸であることが
   確定した以上（§2.4、INV-42）、profile 軸での宣言は不要になる。

**ADR-081 が未配線である理由（Windows 実機の不在）は、凍結の根拠ではない。**
同じ制約は本 ADR の Phase C にもかかっており（§6 Phase C 着手条件）、
「実機が無いから止まっている設計は捨ててよい」という判断をすると、本 ADR
自身の Phase C も同じ論法で捨てられることになる。

**凍結する場合、成果物に ADR-081 のステータス更新を含めること。** 具体的には
次の 2 点の廃止理由を ADR-081 側に明記する:

- **Phase 1c の `GjiDirectAccess` token**（`state/gji_direct_mechanism.rs`）:
  同期義務が §2.4 で outcome 軸へ移り、token でアクセスを絞る前提が消えた。
- **Phase 1c の contract test 不変条件 4・5**（ADR-081「不変条件（Phase 1c で
  実装する contract test、5件）」の 4「belief を actuate 抜きで ON にする高速
  パスは必ず `GjiFsm` を同期させる」/ 5「GJI 機構の状態遷移はどのドライバ経由
  でも同一の `GjiFsm` 同期を通る」）: **どちらも INV-42 + INV-43 が引き取るため、
  profile 軸の宣言（`uses_gji_direct()`）が不要になる。**
  ただし INV-43 の強制力は「debug ビルドの実行時検出」止まりである（§8.1 の
  保証水準の注記を参照）。**不変条件 4 を落とす前に、`ActuationReceipt` を
  settle せずに drop する compile-fail ケース（§7 の trybuild ケース4）が
  実際に赤くなることを確認すること。**
  不変条件 1（IME-ON 経路と stale `ObservedEisu` 救済の対）と 3（`Blind`
  give-up 後に observation を書かない）は本 ADR が引き取らないので、
  **ADR-081 側に残すか `architecture_guard.rs` へ移すかを凍結時に決めること。**
  不変条件 2（`owns_physical_kanji==true` のドライバは物理 KANJI を漏らさない）は
  §2.5 のとおり `AppImePolicy` 側に残る軸なので、凍結対象外。

**凍結を選ばない場合**: `caps` と `ImeProfileDriver` の二重定義期間が生じる。
その場合は「どちらが SSOT か」を ADR-081 か本 ADR のどちらかに明記し、
二重定義の解消期限を切ること（ADR-088 §10-3 が `AxisCapability` について
同じ条件を課している）。

### revert する場合の義務

`.claude/rules/experiment-logging.md` に従い、本 ADR 由来の変更を revert する
コミットは本文に **アプリ / IME（種別と状態）/ 再現手順と症状** を必ず記載する。
本 ADR が対象とする領域（`ime_controller.rs` / `platform.rs` /
`state/observation_store.rs` / `state/ime_event.rs` / `output/` 配下）は
同ルールの適用範囲に明示的に含まれている。

**特に Phase B/C は `ime_controller.rs`（キー選択）に触れる**ため、
`.claude/rules/fix-requires-evidence.md` の「キー選択（IME ON/OFF に送る VK）」
ファミリーに該当する。`tests/ime_key_sequence_golden.rs` の期待値更新か
`docs/known-bugs.md` の追記のどちらかを必ず添えること。

---

## 7. 既存テスト資産の位置づけ

### 維持するもの

| 資産 | 本 ADR での扱い |
|---|---|
| `tests/ime_key_sequence_golden.rs` | **維持。1 バイトも変更していない**（Phase C の必須条件）。r5 までは「Phase C で `caps().chain` の 10 行を golden に追加する」としていたが、**追加しなかった**（§6 Phase C 実施記録 C-1）——行の追加は golden ファイルの更新を伴い「キー選択の回帰検知点を無変更で通す」と両立しない。加えて同ファイルは `#![cfg(windows)]` で **Linux では 0 テスト**であり、caps の全数検査をそこへ置いても Linux CI では実行されない。caps の全数テストは `state/app_ime_policy.rs`（Linux 実行）と `ime_controller.rs`（windows-gated）へ分けた。**なお同ファイルの `KEY_DOC` は「直前に `set_ime_romaji_mode()`」という、Phase C で削除した関数名を今も含んでいる**——挙動の記述（ROMAN ビットを先に立てる）は今も正確だが関数名は古い。更新には golden の再生成が要るため、**次に実機で golden を回すときにまとめて直すこと** |
| `tests/golden_scenarios.rs` | **維持。** `actuator_kind` 期待値は Phase C で `focus_settle_ms` での profile 識別へ書き換えた（ADR が指示していた `caps(p,k).chain[0]` は K 依存で `ImeModel::app_policy` が K を持たないため使えない、§6 Phase C 実施記録 C-3）。`owns_physical_kanji` の 2 件はそのまま |
| `tests/journal_replay.rs` / `tests/drift_correction_replay.rs` / `tests/journals/` | **維持。** §2.1 の `record_replayed(AnyObservation)` がこれらの入口になる |
| `lints/no_vk_as_scan` | **維持**（本 ADR と直交する。VK を scan code として使わせない） |
| `tests/architecture_guard.rs` の維持対象 | `drift_correction_giveup_and_confirmed_do_not_write_observations`（`:808`）、`ime_open_actuation_entry_points_are_accounted_for` の `ENTRY_POINTS`（`:670`）、`actuation_target_capture_is_first_await_in_spawn_local_block`（`:1045`）、`force_write_is_not_triggered_by_raw_focus_change`（`:1131`） |

### 削除するもの（**削除の時期に条件がある**）

r5 は「§2.2 のデータ witness 構築子が実配線された後に限り、次の 5 件を Phase A
完了時点で削除する」と書いていた。**Phase A 実装時に 1 件ずつ照合した結果、
削除できたものは正味 0 件だった**（2026-08-12 の訂正）:

| 旧「削除するもの」 | Phase A 実装後の実際 |
|---|---|
| `heuristic_default_observation_is_limited_to_designated_methods`（`:337`） | **維持**。witness（`ImePolicyProfile`）は「起点を限定する」効果しかなく「起動時に限定する」効果は無い（§9-2）。needle を `evidence::HeuristicDefault` へ付け替えた |
| `focus_probe_observation_is_limited_to_real_probe_path`（`:523`） | **維持**。witness `AcceptedObservation` は `for_sync(epoch)` で runtime 層のどこからでも作れるため、ce45b82（probe を実行していないコードが `write_focus_probe(false)` を注入 → BUG-07）と同型の経路を型では止められない |
| `conv_open_inference_source_is_limited_to_report_and_gate`（`:610`） | 一度削除したが**復活させた**。witness `ConvSyncReason` は普通の public enum で誰でも構築できる（§9-11）。needle を `evidence::ConvOpenInference`、期待値を 2 → 1 に更新して維持 |
| `panic_reset_event_is_limited_to_apply_panic_reset`（`:260`） | **維持**。`PanicReset` は観測でも意図でもなく `Observed<E>` / `IntentWitness` のどちらにも載らない（後述の dylint の表と同じ理由） |
| `hwnd_cache_restored_event_is_limited_to_apply_hwnd_cache_restore`（`:280`） | **維持**。同上 |

代わりに **1 件を新設**した:
`any_observation_replay_door_is_not_used_in_production` —
`AnyObservation::restored_from_journal`（journal / fixture 復元専用の残余の口）が
本番コードから呼ばれていないことを `src/` 全体で固定する。これは削除した 1 件より
広い範囲（witness 構築子の総本数）を 1 本で守る。

**訂正（Phase C 実装時、2026-08-12）**: `conv_write_call_sites_are_target_explicit`
は「Phase C（ADR-086 INV-14 の是正完了後）に削除する」としていたが、
**削除しなかった**。同テストが見ているのは削除済み API
（`set_ime_romaji_mode_with_target_async(`）の**復活検知**であって、
INV-14 の達成度ではない。INV-14 が是正されたからこそ「ライブクエリ版を
再実装しないこと」を守り続ける必要がある（Phase A の教訓「型を書いたから
削除してよい、ではない」と同じ理由）。Phase C は同じ役割のガードを 1 件
**増やした**（`sync_romaji_write_goes_through_a_captured_target`。今回削除した
`set_ime_romaji_mode()` / `_async()` の復活検知を含む）。
**Phase A・B に続き、Phase C でも削除できたテキスト検査は正味 0 件である。**

**削除せず「期待値を縮小して残す」もの**:

- `user_intent_source_construction_is_limited_to_typed_writers`（`:628`）—
  現在 `src/state/platform_state.rs` 内の `source: UserIntentSource::` を
  **3 件**（`write_sync_key` / `write_physical_key` / `write_set_open_request`）
  数えている。§2.2 の witness は `SyncKey` / `PhysicalImeKey` の 2 本しか
  カバーせず、**`Command` は engine 内部判断で witness に載せられる外部事実が
  無い**。したがってこのガードは削除せず、**期待値を 3 → 1
  （`write_set_open_request` の 1 箇所のみ）へ縮小して残す**。
  ゼロにできる条件（`Command` 用の witness 型を作れるか）は §9-8。

**「型を書いたから削除してよい」ではなく「型を通る経路に全呼び出し元を
移してから削除する」**。中間状態（新しい構築子を足したが古い経路も残っている）で
ガードを消すと、両方の防御が同時に消える。

### 撤去するもの — **無し**（r5 までの「dylint 2 crate を撤去する」は誤り）

**訂正（Phase A 実装時の実コード照合、2026-08-12）**: r2〜r5 は
`lints/ime_event_guard` と `lints/observation_source_guard` を「Phase A の型化で
置き換え可能」としていたが、**2 crate とも実際に見ているものが Phase A の
型化範囲（open 軸の観測プール + `Observed<E>` の witness）と重なっていない**。
どちらも**撤去しない**。dylint は 3 crate のまま維持する。

| dylint crate | 実際に flag しているもの（実コード） | Phase A との関係 |
|---|---|---|
| `lints/observation_source_guard` | `ImeEvent::InputModeObserved { source: .. }` の source 偽装（`ImmGetOpenStatus` を名乗る等）。すなわち **input_mode 軸** | **無関係。** Phase A が型化したのは `ObserverReported`（**open 軸**）であり、本 ADR は input_mode 軸の型化に踏み込まない（§2.1）。`InputModeObserved` は `Observed<E>` を通らない |
| `lints/ime_event_guard` | `ImeEvent::PanicReset` / `HwndCacheRestored` / `EngineActivationSync` の designated 関数外での構築 | **無関係。** この 3 variant は**観測でも意図でもない**（`desired_open` の直接書き込み口）ため、`Observed<E>`（観測）にも `IntentWitness`（意図）にも載らない。§2.2 は代替 witness を設計していない |

したがって、`architecture_guard.rs` の
`panic_reset_event_is_limited_to_apply_panic_reset` /
`hwnd_cache_restored_event_is_limited_to_apply_hwnd_cache_restore` も
**削除しない**（前掲「削除するもの」の 5 件のうち、この 2 件は同じ理由で
Phase A では対象外）。

`.claude/rules/ime-belief-architecture.md` の「dylint は型で防げない意味論的
偽装にのみ投資する」という判断基準そのものは維持する——本 ADR が示したのは、
**残る 3 crate はいずれも「型で防げない意味論的偽装」を見ている**という
裏取りである。降ろせる dylint があるとすれば、それは
`InputModeObserved` / `PanicReset` 系を型化する**別の ADR**の成果になる。

### 新設するもの — `trybuild`（compile-fail テスト、4 ケース）

`trybuild` は現在どの `Cargo.toml` にも入っていない（§1.3(i)）。dev-dependency と
して追加し、次の 4 ケースを compile-fail で固定する:

1. `Actuation<Warranted>` から直接 write する（`run_chain` が
   `Actuation<Verified>` にしか生えていないこと）。
2. `record_belief` に `Pool = ActuatingPool` の evidence を渡す
   （**§2.1 の関連型不一致として自然にエラーになる**——これが r2 の
   sealed trait 2 分割案に対する §2.1 の優位性の実証でもある）。
3. `ConvergedReceipt` を `record` へ渡す（INV-46）。
4. `ActuationReceipt` を settle せずに drop する（INV-43。
   `#[must_use]` 由来の warning を `#![deny(unused_must_use)]` 下で
   エラー化する形）。**このケースが固定できるのは「receipt を束縛せずに捨てた」
   形だけである**——`let r = make_receipt();` は `#[must_use]` を発火させないため
   compile-fail にできない（§8.1 の保証水準の注記）。

**保守負担についての注記**: compile-fail テストは rustc の更新でエラー
メッセージが変わり、CI（stable 追従）で赤くなる。**`stderr` の照合は緩める**
（`trybuild` の `normalize` を併用し、行番号やノートの差分を吸収する）。
「型で守っている」ことの証明としては 4 ケースで十分であり、網羅性を追わない。

### 新設するもの — 全数テスト（Linux で完結）

- `OpenEvidence::SOURCE` × `authority()` の **9 値**一致 + 除外 2 値の
  `PerSourceObservations::get(..) == None`（Phase A item 5、§2.1）。
  **実装時の強化（2026-08-12）**: この一致は `PoolKind` の関連定数
  `AUTHORITY` を介して取る——テストは `<E::Pool as PoolKind>::AUTHORITY` を
  **実際に参照**し、`E::SOURCE.authority()` および手書き期待値と 3 者比較する。
  `E::SOURCE.authority()` と手書き期待値だけを比べる形だと、`type Pool` の
  割り当てを取り違えてもテストが素通りしてしまう（`record` / `record_belief`
  の本番呼び出し元がまだ無い Phase A では、型境界がどこにも効かないため。§9-10）。
  同じ一致は `declare_evidence!` 内の `const _: () = assert!(..)` が
  **コンパイル時**にも検査する（`ObservationSource::authority()` が `const fn`
  であるため可能）。したがってプール取り違えは「ビルドが通らない」か
  「全数テストが落ちる」かのどちらかになる。
- `caps(p, k)` の 10 行が `app_ime_policy.rs` の既存 `from_profile` と
  矛盾しないこと（Phase C。**`Plain`/`Unknown` 行が `ImmCross` と同一である
  ことを明示的に assert する**、INV-44）。照合対象は
  `caps(p, k).feedback == AppImePolicy::from_profile(p).default_feedback` と
  `caps(p, k).focus_settle_ms == AppImePolicy::from_profile(p).focus_settle_ms`
  の 2 本（K 非依存であることも同時に assert する、§2.5）。
- `caps(p, k).chain` の各要素が、現行 `ImeController` の
  `is_applicable` × フォールスルー述語（`Failed` のときだけ次へ）で
  **実際に到達する**こと（Phase C、INV-44）。到達不能な末尾要素を足した瞬間に
  落ちるテストにする。既存の `characterize_strategy`
  （`ime_controller.rs:375`、`is_applicable` のみ評価する副作用なしのシーム）を
  再利用できる。
- `legacy_gji_sync_obligation` と `ActuationReceipt::settle` の同期判定が
  全 `ImeOpenOutcome` × `open` で一致すること（INV-42）。

---

## 8. 影響

### 8.1 良くなること

- **`architecture_guard.rs` のテキスト検査が減り、1 件
  （`user_intent_source_construction_is_limited_to_typed_writers`）は期待値
  3 → 1 に縮小できる。** 規律の担い手がテキスト検査からコンパイラへ移る。
  **訂正（Phase A 実装時の実測）**: r5 まで「6 件削除 + dylint 2 crate 不要」と
  書いていたが、実際に Phase A で削除できたテキスト検査は
  **正味 0 件**である——1 件（`conv_open_inference_source_is_limited_to_report_and_gate`）
  を削除したうえで復活させ（needle を witness 構築子へ付け替えて維持）、
  代わりに広い範囲を守る 1 件（`any_observation_replay_door_is_not_used_in_production`）
  を新設した。dylint 2 crate も撤去対象ではなかった（§7 の訂正）。
  **型化の成果は「ガードの本数が減ること」ではなく「同じ本数のガードが、
  より広い範囲（witness 構築子の総本数）を守る形に置き換わること」だった。**
- **BUG-19 / BUG-33 の 2 ファミリーは、型として再発不能に「なる」——
  ただし配線が済んだ後の話である。**
  どちらの保証も「その API が存在しない」形であり——プール毎 derive を
  提供しない（INV-39）、`ConvergedReceipt` から `Observed<E>` /
  `AnyObservation` への変換を提供しない（INV-46）——**release ビルドでも
  有効で、`cfg` にも依存しない。**
  **訂正（Phase B 実装後の実測、2026-08-12）**: r5 まで本項は
  「BUG-19 / BUG-33 の 2 ファミリーは型として再発不能になる」と**完了形で**
  書いていた。**Phase B 時点でこれは成立していない。**
  - **BUG-33（INV-46）**: `ConvergedReceipt` は
    `runtime/ime_refresh.rs` の 2 箇所（`Read` 収束判定 / `Blind` give-up
    判定）で構築されているが、**その値は `log::debug!` の引数にしかなって
    いない**（`receipt.converged()` / `receipt.attempts()`）。実際の読み戻しは
    従来どおり `ObservationStore::most_recent_trusted_after(now, since)` が
    `Option<&ImeObservation>` を直接返し、収束判定
    （`.is_some_and(|o| o.open == desired)`）も give-up 後の復旧判定
    （`.is_some()`）もその `ImeObservation` を見て行う。**`ConvergedReceipt` を
    削除しても制御フローは 1 ビットも変わらない。** したがって
    BUG-33（読み戻しの産物を観測として記録し、収束を偽装する）を今も止めて
    いるのは、`ime_refresh.rs` の「observations には一切書き込まない」という
    コードとコメントの規約、およびそれを見張るテキスト検査
    `drift_correction_giveup_and_confirmed_do_not_write_observations` である。
    型が効くのは「`ConvergedReceipt` を持っている人が、それを観測に変換
    しようとしたとき」だけであり、**その状況が本番コードに存在しない**以上、
    現状の INV-46 は**空回りしている**（§9-16）。
  - **BUG-19（INV-39）**: §9-10 が Phase A の既知の限界として既に書いている
    とおり、`record` / `record_belief` の本番呼び出し元がゼロで、本番の観測は
    すべて `AnyObservation`（`Pool` の型情報を落とした値）を経由する。
  **どちらも「型を置いた」段階であって「型で守られている」段階ではない。**
  配線は Phase C 以降（§9-10 / §9-16）。
- **BUG-18・22（`GjiFsm` 同期欠落）は「型として再発不能」にはならない。
  実効は debug ビルドでの実行時検出にとどまる**（下記の保証水準の注記）。
- **`ime_controller.rs` の 2 経路（`apply` / `apply_skipping_imm`）が
  `run_chain` に一本化される**（Phase B）。**実装済み（2026-08-12）**——
  `apply_skipping_imm` は撤去し、ImmCross を機構チェーンの要素にしたことで
  `Failed` 後のフォールスルーは `run_chain_async` が行うようになった。
- **`AppImePolicy` の死んだフィールド 2 つのうち 1 つ（`actuator_kind`）が消える**
  （Phase C）。**実装済み（2026-08-12）** —— `ImeActuatorKind` ごと撤去した。
  残る `owns_physical_kanji` は本番の読み手ゼロのままだが、BUG-46 の物理キー
  抑止という別軸の概念なので `caps` へは吸収していない（§2.5）。
- **capability の宣言が `caps(p, k)` の 1 つの match に集約された**（Phase C、
  INV-44）。**ただし同期経路のみ**——非同期チェーン（`runtime/open_chain.rs`）は
  `WriteMechanism::ALL` のままであり、そこでの一意性は「`ALL` は全 `caps`
  チェーンの和集合である」という論証に依存している（§9-20）。
- **`AppImePolicy` と `caps` の二重 SSOT は生じていない**（Phase C）。
  `AppImePolicy::from_profile` を `caps` の薄いファサードへ退化させたため、
  `focus_settle_ms` / `default_feedback` のリテラルは `caps` 側にしかない。
  ADR-081 の `ImeProfileDriver` が `AppImePolicy` と parity テストで同期を
  取っている構造はそのまま残る（SSOT は 2 本のままで、3 本には増えていない）。

#### 保証水準の注記 — INV-43 は「コンパイル時保証」ではない

`ActuationReceipt`（§2.4）が使う 2 つの仕組みには、それぞれ穴がある:

| 仕組み | 効くとき | 効かないとき |
|---|---|---|
| `Drop` の `debug_assert!(self.settled)` | debug ビルドで、実際にその経路を**通ったとき** | **release ビルドでは消える**（`debug_assert` は `cfg(debug_assertions)`）。通らなかった経路は永久に検出されない |
| `#[must_use]` | receipt を**式の値として捨てた**とき（`make_receipt();`） | `let r = make_receipt();` / `let _ = make_receipt();` / 構造体フィールドへの格納では**発火しない** |

したがって INV-43 が与えるのは「**settle 忘れを debug 実行で気づける**」で
あって、「settle 忘れがコンパイルを通らない」ではない。§7 の trybuild ケース4
（`#![deny(unused_must_use)]` 下で未束縛の receipt をエラー化する）が固定
できるのも**未束縛のケースだけ**である。

**なぜそれでも現状より良いか**: 現状の担い手は「`platform.rs:879-891` の分岐が
正しく書かれていること」だけで、**何も検査していない**（§2.0 の表）。
debug 実行での検出と `#[must_use]` の一部カバーは、ゼロからの純増である。
**ただし「型で守られているから `platform.rs` の同期を消してよい」とは読めない**
——ADR-081 Phase 1e が legacy の直接呼び出しを撤去しようとして
BUG-18/22 型の再発条件を作りかけた（§1.3(f)）のと同じ轍を踏む。

### 8.2 悪くなる / 変わらないこと（正直に書く）

- **規律はゼロにならない。** 次の 3 つは依然としてレビュー対象である:
  - `caps` の match に腕を足すこと（`Plain`/`Unknown` 行の同一性は
    全数テストで守れるが、新しい profile を足すときの内容は人間が決める）。
  - `WriteMechanism` の新しい impl を書くこと。
  - `OpenEvidence` の新しい impl で `type Pool` を選ぶこと
    （**コヒーレンスは「2 つ書けない」を保証するが「正しいほうを選んだ」は
    保証しない**）。
- **`ObservationSource` の実行時 match は消えない。** journal 復元
  （ADR-082）と診断ログが値として扱う以上、対応表は型と値の両方に必要である
  （§2.1 末尾）。**「型にしたから enum を消せる」と読まないこと。**
- **型の外に残る領域は変わらない**: 修飾キー汚染（ADR-088 トラック B）、
  SendInput の到達性（同トラック D）、実機のタイミング（`tuning.rs` の定数群）。
  本 ADR はこれらに何も寄与しない。
- **Phase B は 4 戦略を async 化する**ため、打鍵ホットパスのレイテンシに影響
  しうる。ADR-086 §ステータスが記録している「item 0 の同期 IMC write が
  force-ON 発火のたびに打鍵ホットパスに乗る（実測最大 ~100ms）」と同じ軸の
  リスクがある。**Phase B の実機ソークではレイテンシを測ること**
  （`.claude/rules/tuning-constants.md` の実測義務）。

### 8.3 実装しない場合に起きること

現状維持でも動作は壊れない。ただし §1.2 の 8 件のテキスト検査は、
**関数名のリファクタや `extract_fn_body` の needle ずれで黙って無効化されうる**。
実際、ADR-087 §5 Phase 3 item14 で `EXPECTED_TOTAL=5` が doc コメント中の同名
文字列を 1 件誤カウントしていた前例がある（ADR-088 §5.5 が引用）。
**テキスト検査は「偽陰性が静かである」という性質を構造的に持つ。**

---

## 9. 未解決の論点

1. **`ActuationReceipt::Drop` の `debug_assert` が panic unwind 中に
   double panic を起こしうる。** actuation 中に panic した場合、
   receipt は settle されないまま drop される。`std::thread::panicking()` を
   チェックして skip する形にするか、`debug_assert` ではなく
   `log::error!` にするかは **Phase B 実装時に決める**。
   （round4 の Fable レビューで残った軽微な指摘その1。）

2. **`heuristic_default_observation_is_limited_to_designated_methods` を削除した
   後、`Observed<HeuristicDefault>::at_startup` が起動時以外から呼ばれても
   検出できない。** §2.2 の witness（`profile: ImePolicyProfile` を引数に取る）は
   「起点を限定する」効果はあるが、「起動時に限定する」効果は無い。
   **Phase A 完了時点での `architecture_guard` 置き換えの範囲として判断する**
   （witness を `StartupWitness` のような専用型にするか、ガードを 1 件だけ残すか）。
   （round4 の Fable レビューで残った軽微な指摘その2。）

3. **`caps` と ADR-088 の `AxisCapability` の同居。** 両者とも
   `state/app_ime_policy.rs` に置く計画であり、名前が似ている
   （前者は `(profile, ime_kind) → 戦略チェーン`、後者は `(profile, 軸) → 読み書き可否`）。
   **どちらか一方を別ファイルへ分けるべきか**は、両方を実装する段で決める。
   混同すると「capability 表を見たのに戦略チェーンの表だった」という読み違いが
   起きる。

4. **ADR-081 Phase 1d を凍結するかどうか**（§6）。凍結が最も低コストだが、
   ADR-081 の Phase 1a/1b/1c は 3 ファイル分の実装であり、撤去そのものの
   レビューコストがゼロではない。**凍結しない場合の二重定義期間の管理方法**を
   決めていない。

5. **~~Phase C の着手条件（ADR-088 トラック D の復旧）が満たされる見込みが無い。~~**
   **解消（2026-08-12、ユーザー判断でゲート解除）。** ADR-088 §7 のとおり、
   合成キー入力が原因不明で無効化される問題は未解決のまま棚上げになった
   （検証した 5 仮説と、ユーザーの対話操作＋実マウスクリックによる実フォーカス下
   でも再現したという決定打は §6 Phase C「ゲート解除」節に記録した）。
   **「ゲートが満たされるまで待つ」は「永久に着手しない」と同義**という本項の
   懸念がそのまま解除理由になった。方針は「Phase C 実装 → 実機での長期ソーク」
   に変更済み。残る論点は §9-17（何が Linux で検証済みで、何が実機ソークでしか
   検証できないか）へ引き継ぐ。

6. **`derive_any` のマージ後判定が、旧 `derive_open` と厳密に一致するか。**
   Phase A の pinned test で固定する計画だが、`ImeObservation` の
   `expires_at` / `focus_epoch` によるフィルタが両プールで非対称に効く可能性が
   ある。**epoch 照合を行うのは `derive_open_filtered` 内のクロージャ
   `is_epoch_ok`（`observation_store.rs:342-347`）であり、`ImmCrossProbe` /
   `FocusProbe` の 2 ソースだけを `current_focus_epoch` と突き合わせる**
   （r3 は `clear_on_focus_change` がこのフィルタを行うと書いていたが誤り。
   `clear_on_focus_change`（`:219-223`）は `per_source.clear_all()` で
   **全ソースを無条件に消し**、`current_focus_epoch` を更新するだけである）。
   問題は、この 2 ソースが **異なるプールに属する**点にある——`ImmCrossProbe` は
   Actuating、`FocusProbe` は BeliefOnly（`ime_event.rs:183-193`）。
   `derive_actuating` では epoch フィルタが `ImmCrossProbe` にだけ、
   `derive_any` では両方に効く。**プール分離が `is_epoch_ok` の意味を変えて
   いないかを、Phase A の最初に確認すること。**

7. **本 ADR は ADR-088 トラック A（`AxisCapability` + `CharsetOwner`）の実装
   計画と統合されていない。** 両者は独立に実装できると §ステータスに書いたが、
   **どちらを先に実装するか**は決めていない。ADR-088 Phase 1 と本 ADR Phase A は
   どちらも Linux で完結する純粋ロジックであり、同時に走らせると
   `state/` 配下でコンフリクトする。

8. **`UserIntentSource::Command` を witness 化する手段が無い。** §2.2 のとおり
   `SyncKey` / `PhysicalImeKey` は `&RawKeyEvent` を witness にできるが、
   `Command`（`state/platform_state.rs:253` の `handle_engine_set_open` →
   `write_set_open_request`）は engine 内部判断であり、引数の型で起点を限定
   できる外部事実が無い。**`architecture_guard.rs:628` を期待値 1 で残す**
   （§7）のが Phase A の結論だが、恒久解としては
   (a) engine 境界でのみ構築できる `EngineCommandWitness`（`generation` や
   `ImeApplyRequested` の発行権と束ねる）を作る、
   (b) `write_set_open_request` の可視性を `pub(in crate::state)` に絞って
   ガードを可視性へ置き換える、の 2 案がある。**どちらを採るかは Phase A の
   実装時に決める。** BUG-19 の再発条件（間接推測が `Command` を名乗って
   `desired_open` を書き換える）に直接関係する箇所なので、**ガードをゼロに
   する変更は単独では行わないこと。**

9. **`ActuationReceipt` を持つ呼び出しフレームの特定が済んでいない。**
   §2.4 細目3 のとおり receipt は `WindowsPlatform` のフィールドに置けず、
   actuation を起動したフレームのローカル値として settle まで持つ必要がある。
   現行の `on_ime_applied` 相当（`platform.rs:879-891`）は
   `&mut self` の中にいるため、receipt を作る場所（`run_chain` の戻り値として
   受け取る呼び出し元）が `&mut WindowsPlatform` を別途取れるかは、
   **Phase B で `ImeOpenStrategy::apply` を async 化した後の呼び出し構造を
   見ないと確定できない**。async 境界をまたいで receipt を保持する場合、
   `Drop` の `debug_assert` が await 中断時にも走る点も併せて設計すること
   （§9-1 の double panic 問題と同じ箇所）。

10. **【Phase A の既知の限界】型による保護が、まだ本番コードに配線されていない。**
    §2.1 のプール分離を強制する新設の入口——`ObservationStore::record`
    （`E::Pool = ActuatingPool` を要求）と `record_belief`
    （`E::Pool = BeliefPool` を要求）——の**呼び出し元が本番コードにゼロである**
    （`crates/awase-windows/src/` を grep して確認、2026-08-12）。本番の観測経路は
    すべて `ImeEvent::ObserverReported(AnyObservation)` を経由し、
    reduce 側では `record_replayed(AnyObservation)`（`state/ime_model.rs:463`）
    1 本に集約される。**`AnyObservation` は `Pool` の型情報を落とした値**なので、
    この経路では関連型による排他は一切効かない。
    Phase A が実際に効かせている型の力は
    (a) `Observed<E>` の witness 構築子が `AnyObservation` を作る**唯一の**
    通常経路であること（残余の口 `restored_from_journal` は
    `any_observation_replay_door_is_not_used_in_production` が塞ぐ）と、
    (b) evidence 型ごとの `SOURCE` / `CONFIDENCE` 固定、の 2 点にとどまる。
    **プール分離の実効的な型保護は Phase B 以降**——actuation 側がストアへ
    直接書く経路（`derive_actuating` の入力を作る側）ができ、その呼び出し元が
    `record` / `record_belief` を使うようになって初めて機能する。
    それまでは §7 の「削除するもの」を追加で削ってはならない。

11. **【Phase A の既知の限界】witness の強度が不均一である。**
    §2.2 は「構築子が固有のデータ witness を要求する」と書いたが、
    その witness の偽造難度は 3 段に分かれている:
    - **偽造不能**: `AcceptedObservation`（`state/probe_admission.rs` でしか
      構築できない）。ただし `for_sync(epoch)` があるため「probe を実行した」
      ことまでは保証しない（§7 の `focus_probe_...` を残す理由）。
    - **偽造容易**: `ImePolicyProfile` / `ConvSyncReason` は普通の public enum で、
      誰でも `ConvSyncReason::KatakanaShadowOff` と書ける。これらの witness は
      「起点を宣言させる」効果しかなく、「その事実が実在した」ことは保証しない。
      **だから `heuristic_default_...` / `conv_open_inference_...` の
      テキスト検査を残している**（§7）。
    - **裏口が残っている**: `ObservationStore::per_source`、
      `PerSourceObservations::set`、`ImeObservation` の全フィールドが
      `pub` のままである。`store.per_source.set(ImeObservation { source: .., .. })`
      と書けば witness も `record`/`record_belief` も経由せずに観測を注入できる。
    **可視性を絞る（`pub(in crate::state)` 化など）のは Phase B の作業**——
    Phase A で絞ると `runtime/` 側の既存呼び出し元が一斉に壊れ、
    型化そのものとは無関係な差分でレビューが埋まる。
    §9-8 の `write_set_open_request` の可視性案（b）と同じ段でまとめて検討する。
    **Phase B での前進（2026-08-12）**: `PerSourceObservations::set` を
    `pub(crate)` へ縮小し、crate 外からの注入は塞いだ。crate 内の呼び出し元は
    `per_source_set_is_confined_to_the_store` が 1 件に固定している。
    `ObservationStore::per_source` フィールドと `ImeObservation` の各フィールドは
    `pub` のままである（`tests/golden_scenarios.rs` が読んでいるため）。

12. **【Phase B の既知の限界】`Actuation` は warrant を持っていない。**
    §2.3 は `Actuation<Requested>::warrant(w: OpenWarrant)` を正規経路として
    いるが、**`issue_open_warrant()`（ADR-087）の本番呼び出し元は依然ゼロ**で
    あり、Phase B が配線した 2 経路（`ime_controller.rs` の同期チェーンと
    `runtime/open_chain.rs` の非同期チェーン）はどちらも
    `warrant_pending_adr087()` を通る。したがって
    **「warrant なしに write しない」は現時点で型としては効いていない**——
    効いているのは「`run_chain` は `Actuation<Verified>` にしか生えない」
    （段階の順序）と「1 値 = 高々 1 回の成功 write」（アフィン性）の 2 点である。
    これを閉じるのは ADR-087 Phase 3（`issue_open_warrant()` の実配線）であり、
    本 ADR の Phase C ではない。件数ガード
    （`legacy_unwarranted_actuation_sites_are_accounted_for`、期待値 2）が
    増加を検出する。

13. **【Phase B の既知の限界】非同期チェーンのフォールバックが機構ごとに
    `ImeControlView` を作り直す。**
    旧 `apply_skipping_imm` は `with_app` の中で 1 つの view を作って残り戦略を
    すべて評価していたが、`runtime/open_chain.rs` の `fallback_write` は
    機構ごとに `shadow_ime_control_view()` を作る。実害が無いと判断した根拠は
    「`Failed` を返す戦略が `ImmCrossProcessStrategy` だけ」（§2.3）であり、
    ImmCross 以降で 2 回以上 write が走ることが構造的に無いためである。
    **`GjiDirectStrategy` / `MsImeDirectStrategy` が将来 `Failed` を返すように
    変わったら、この前提は崩れる**（§2.8 の「`KanjiToggle` を末尾に足すか」の
    判断と同じタイミングで見直すこと）。

14. **【Phase B の既知の限界】compile-fail を `trybuild` ではなく doctest で
    固定した。**
    §7 は `trybuild` を指定していたが、同節自身が挙げる保守負担（rustc 更新で
    `stderr` が変わり CI が赤くなる）を避けて `compile_fail` doctest にした
    （§6「Phase B 実施記録」B-7）。**`compile_fail` は「何らかの理由で
    コンパイルが落ちれば通る」ため、双子の passing doctest を必ず併記する規約に
    している**が、この規約自体は機械的に強制されていない——双子を消しても
    テストは緑のまま通る。`trybuild` へ戻すかどうかは、CI が実際に rustc 更新で
    赤くなった実績が出てから判断する。

15. **【Phase B の既知の限界】1 機構分の生 write の口は、型ではなく件数ガードで
    塞いでいる。**
    `ime_controller::apply_mechanism(mechanism, open, view)` は
    `Actuation` 型状態チェーンを 1 つも構築せずに実 write を起こせる
    `pub(crate)` 関数のままである（§2.3「チェーンの『入口』を数えるだけでは
    足りない」）。現在の呼び出し元 2 箇所はどちらもチェーンの writer 実装
    そのもの（`SyncChainWriter::write` / `fallback_write`）であり、
    可視性の縮小でも「チェーン経由へ書き換え」でも解けないため、
    `raw_mechanism_write_sites_are_confined_to_chain_writers` が件数で固定して
    いる。**恒久策は `run_chain` / `run_chain_async` だけが構築できる
    authorization トークン**（`Actuation<Verified>` の中でのみ生成できる
    private フィールド持ちの型）を `MechanismWriter::write` /
    `AsyncMechanismWriter::write` の引数に通し、`apply_mechanism` がそれを
    要求する形にすること。そうすればチェーン外からの呼び出しはコンパイルを
    通らなくなる。**Phase B では採らなかった**——writer トレイトのシグネチャ
    変更は §7 の `compile_fail` doctest（ケース1 とその「通る双子」）まで波及し、
    `caps(p, k).chain` を導入する Phase C（§2.8）が同じ場所をもう一度触るため。
    **Phase C で `caps` 表に合わせて writer を書き換えるときに同時に入れること。**

16. **【Phase B の既知の限界】`ConvergedReceipt` が制御フローに配線されて
    いない（INV-46 が空回りしている）。**
    §8.1 の訂正のとおり、`ConvergedReceipt` は `runtime/ime_refresh.rs` の
    2 箇所で構築されるが**値は `log::debug!` にしか渡っていない**。実際の
    読み戻しは `ObservationStore::most_recent_trusted_after(now, since)` が
    `Option<&ImeObservation>` を直接返し、収束判定も give-up 後の復旧判定も
    その `ImeObservation` を見て行う。**receipt を削除しても制御フローは
    変わらない。** INV-46（`ConvergedReceipt` → `Observed<E>` /
    `AnyObservation` の変換が存在しない）は型としては成立しているが、
    **変換したくなる人が本番コードに居ない**ため、BUG-33 を今止めているのは
    依然としてコード規約と
    `drift_correction_giveup_and_confirmed_do_not_write_observations`
    （テキスト検査）である。
    **Phase C で配線する形**: `most_recent_trusted_after` の**戻り値を
    そのまま返さない**読み戻し API（例
    `ObservationStore::read_back(now, since, desired) -> ConvergedReceipt`）を
    作り、`ir_apply_drift_correction` がそれだけを見て収束/復旧を判定する。
    そうして初めて「読み戻しの産物は `ImeObservation` として手に入らない」
    ——ADR-080 不変条件6 のコンパイラ強制——が成立する。
    **その時点まで §7 の「削除するもの」から
    `drift_correction_giveup_and_confirmed_do_not_write_observations` を
    削ってはならない**（§9-10 の Phase A の限界と同じ理由）。
    なお `give-up` 側は復旧判定に「値は不問、鮮度だけを見る」という別の
    述語を使っている（`most_recent_trusted_after(now, gave_up_at).is_some()`）
    ため、receipt に載せる情報は `converged` / `attempts` の 2 つでは足りない
    可能性がある。**API の形は配線時に決めること。**

17. **【Phase C の申し送り】実機ソークでのみ検証できる残余リスク（2026-08-12）。**
    Phase C は「実装 → 長期ソーク」という方針変更（§6 Phase C「ゲート解除」）の
    下で入れた。Linux で固定できた範囲は §6「Phase C 実施記録」の表のとおりで、
    **次の項目は実機でしか確かめられない**。ソーク中に異常が出たら、
    `.claude/rules/experiment-logging.md` の義務（アプリ / IME 種別と状態 /
    再現手順と症状）を満たした revert コミットを書くこと。

    | # | 確認すること | 対象アプリ × IME | 異常の見え方 |
    |---|---|---|---|
    | 17-a | **`caps` chain 切り替えで送るキー・順序が変わっていないこと**。`caps_chain_matches_legacy_all_scan` は windows-gated で、Linux では型検査しか通っていない | 全 6 組み合わせ（Standard / Imm32Unavailable / TsfNative × GJI / MS-IME） | IME ON/OFF が効かない、または別のキーが飛ぶ |
    | 17-b | **`ImmCross × MS-IME` で ImmCross が `Failed` を返したときに `KanjiToggle` へ落ちること**。caps chain が `[ImmCross, KanjiToggle]` になった唯一のフォールスルー経路 | LINE / Qt アプリ × MS-IME、`SendMessageTimeout` が実際にタイムアウトする状況 | IME が切り替わらないまま無反応 |
    | 17-c | **ROMAN 補完（`set_ime_romaji_mode_for_target_blocking`）が Phase C 以前と同じ hwnd へ着弾していること**。hwnd 解決の関数・タイムアウト・フォールバックは変えていないが、**捕獲を write の外へ出したこと自体**は実機で初めて確かめられる | Windows Terminal / Edge × MS-IME（BUG-55 の子ウィンドウ問題が出るアプリ）、かなモードから IME ON | 最初の打鍵が JIS かな入力になる（`aiueo` → `ちいすいえの` 等） |
    | 17-d | **`Aborted(GenStale)` が実際には発火しないこと**（同期経路には await 点が無いため常に一致するはずという設計前提）。`[imm-romaji] Aborted(GenStale)` のログが出たら前提が崩れている | 全アプリ、フォーカス切り替えの多い操作（Alt+Tab 連打） | ROMAN 補完が黙ってスキップされ、17-c と同じ症状 |
    | 17-e | **ROMAN 補完のレイテンシが変わっていないこと**。Win32 往復は 1 回のままだが、`.claude/rules/tuning-constants.md` の実測義務に従い force-ON 経路の実測を取ること | Chrome / Edge × MS-IME、force-ON がホットパスに乗る打鍵 | 打鍵の取りこぼし・体感の引っかかり |
    | 17-f | **`focus_settle_ms` / `default_feedback` が `caps` 経由になっても同じ値で使われていること**。ファサード化なので値は同じはずだが、`settle_until` と drift correction のタイミングに効く | Chrome（500ms）/ WezTerm（200ms）/ LINE（100ms） | フォーカス直後の spurious apply、または drift correction の過剰/過少発火 |
    | 17-g | **`imm_cross_is_first_applicable` の caps 化で async/sync 分岐が変わっていないこと**。`chain[0] == ImmCross` の同一性チェックを外すと GJI 経路が誤って async 分岐へ流れる（実装時に踏みかけた） | Chrome / WezTerm × GJI | `with_app` 再入、または IME 適用の二重発火 |
    | 17-h | **bootstrap force-ON（`try_force_on_bootstrap`）が同期 ImmCross 経路に入ったときの着弾先**。この経路は `!can_use_imm32_cross_process()` ガードを持たず、Standard プロファイルでも `ImmCrossProcessStrategy::apply` に到達する（§9-18・§9-21）。ROMAN 補完は `get_focused_hwnd()`（30ms + `GetForegroundWindow` フォールバック）で捕獲した hwnd、open write は `get_gui_thread_info_with_timeout(150ms)`（フォールバック無し）で、**別ウィンドウへ着弾しうる**。ログ `IME detection failed N times, forcing OS ime_on=true` と `force-on bootstrap: apply_ime_open(true) → ...` の有無で発火を確認する。**Phase C 以前から同じ挙動なので、異常が出ても Phase C の revert では直らない**（§9-18 の恒久策が要る） | LINE / Qt（Standard）× MS-IME または GJI、IME 検出が `IME_DETECT_MISS_THRESHOLD` 回連続で失敗する状況（`ir_poll_and_learn` の `OsPoll` 経由） | IME は ON になるが最初の打鍵が JIS かな入力になる（17-c と同じ症状）、または force-ON 自体が無反応 |

    **ソークの最低期間の目安**: `docs/experiments.md` エントリ01 が示すとおり、
    IME キー選択の不具合は「特定アプリ × 特定 idle 時間」でしか出ないことが
    ある。1 日の通常利用で 17-a/17-c/17-f をひととおり踏むこと、
    17-b と 17-h は再現条件が稀なのでログ（17-b は
    `[apply-ime] ImmCrossProcess failed, trying next fallback`、17-h は
    `force-on bootstrap: apply_ime_open(true)`）の有無で事後確認すること。

18. **【Phase C の既知の限界】同期 ImmCross 経路の open write は依然として
    自分でライブクエリする。**
    `ImmCrossProcessStrategy::apply` の `set_ime_open_cross_process(open)` は
    `get_gui_thread_info_with_timeout(150ms).focused_hwnd`（フォールバック無し）
    で宛先を write 時点に決める。一方 Phase C で `ActuationTarget` 化した
    ROMAN 補完は `get_focused_hwnd()`（30ms + `GetForegroundWindow`
    フォールバック）で捕獲する。**したがって同期 ImmCross 経路では
    ROMAN と open が別ウィンドウへ着弾しうる**（ADR-086 §1.2 欠陥1 と同型）。
    捕獲を共有させるには 2 つの hwnd 解決のどちらかへ寄せる必要があり、
    どちらへ寄せても **タイムアウト（30ms ↔ 150ms）とフォールバックの有無**
    という意味論が変わる——前者へ寄せると hung なフォアグラウンドで
    `GetForegroundWindow` にフォールバックし BUG-55 の「無関係な互換ウィンドウ
    へ書く」を再現しうるし、後者へ寄せると MS-IME 経路の最悪レイテンシが
    +120ms 変わる。**実機ソーク（特に 17-e の実測）が取れてから判断すること。**

    **【2026-08-12 訂正】この穴は「潜在的」ではなく、現に到達しうる。**
    初出時は「同期 ImmCross 経路は現時点で到達しない」と書いていたが、
    それは `runtime/mod.rs::try_force_on_bootstrap`（`:892`）を数え落とした
    誤りである（§9-21、§6 Phase C 実施記録 C-7 の訂正）。同関数は
    `apply_ime_open_with_belief(true, None, belief)` を同期で呼び、
    `!can_use_imm32_cross_process()` ガードを持たないため、**Standard
    （= `ImmCross` プロファイル、LINE / Qt 等）で IME 検出ミスが
    `IME_DETECT_MISS_THRESHOLD` 回連続したときの bootstrap force-ON**
    がこの経路に入る。したがって「ROMAN 補完と open write が別ウィンドウへ
    着弾しうる」ことは、その条件下では実際に起こりうる。
    **ただし Phase C 以前から同じ挙動である**——旧実装でも同じ呼び出し元から
    `ImmCrossProcessStrategy::apply` に入り、その中で
    `set_ime_romaji_mode()`（ライブクエリ）と `set_ime_open_cross_process()`
    （別のライブクエリ）が別々に宛先を決めていた。Phase C は前者を捕獲済み
    `ActuationTarget` へ移しただけで、**2 つの hwnd 解決が別物である点は
    Phase C 以前から変わっていない**（新規の回帰ではない）。実機ソーク項目
    17-h を参照。

19. **【訂正】`VerifiedTarget::FocusImplicit` は「INV-14 の未達」ではない。**
    §6「Phase B でも消えなかったもの」は「`FocusImplicit` が `Verified` に
    入っていること自体が INV-14 の未達を表す。Phase C で潰す」と書いていたが、
    **潰せない**。VK 送信機構（`GjiDirect` / `MsImeDirect` / `KanjiToggle`）の
    実 write は `SendInput` であり、**宛先引数を取らない**（フォアグラウンドの
    入力キュー宛に配送される）。捕獲すべき hwnd が構造的に存在しない以上、
    `FocusImplicit` はこれらの機構の**性質**であって未移行の印ではない。
    INV-14 が意味を持つのは hwnd を引数に取る write（IMC 系）だけであり、
    同期経路でそれに当たるのは ROMAN 補完のみ——それは Phase C で
    `ActuationTarget` 化した。**残る `FocusImplicit` の「本当の未移行分」は
    `ImmCrossOp::Untargeted`（`key_pipeline.rs` の shadow-toggle OFF 経路）
    1 件だけ**である（§9-20）。

20. **【Phase C の既知の限界】非同期チェーンは `caps` を使っていない。**
    `runtime/open_chain.rs::run_open_chain_async` の chain は
    `WriteMechanism::ALL` のままである（§6 Phase C 実施記録 C-4）。理由は
    「ImmCross の await をまたいでフォーカス（したがって profile と K）が
    動きうるため、起案時点の `caps(p, k).chain` を固定すると完了時点で
    適用可能な機構を取りこぼす」——`ImeKindId` が推測値である以上
    （INV-45）、await をまたいで K を固定するのは P20 が禁じる形のゲートである。
    したがって **INV-44 の「capability は const 表 1 箇所」は、同期経路では
    型どおり成立し、非同期経路では『`ALL` は全 `caps` チェーンの和集合であり、
    `is_applicable` + `falls_through` で絞れば同値』という**論証**に依存して
    いる**（同値性は `caps_chain_matches_legacy_all_scan` が固定するが、
    それは「(p, k) が変わらない場合」の同値である）。
    恒久策の候補は 2 つ:
    (a) `run_chain_async` が各ステップで chain を引き直せるようにする
    （`&[WriteMechanism]` ではなく `FnMut() -> &'static [WriteMechanism]` を取る）、
    (b) `ImmCrossOp::Untargeted` を `Targeted` へ寄せ（§9-19）、
    ImmCross 完了後にフォーカスが動いていたら `Aborted` にしてチェーン自体を
    打ち切る。**どちらも実機ソーク必須**であり、Phase C では採らなかった。

21. **【訂正】「`ImmCrossProcessStrategy::apply` は同期経路から到達しない」は
    誤りだった（2026-08-12 Opus レビュー）。**
    §6 Phase C 実施記録 C-7 と §9-18 初出、および
    `output/conv_actuation.rs` のモジュール doc は「全呼び出し元を追跡した
    結果、同期経路からは到達しない」と書いていたが、**棚卸しから
    `runtime/mod.rs::try_force_on_bootstrap`（`:892`）が漏れていた**。

    実際の経路:

    ```
    ir_poll_and_learn（OsPoll、ime_refresh.rs:383）
      → try_force_on_bootstrap（runtime/mod.rs:871）
        → platform.apply_ime_open_with_belief(true, None, belief)  // :892
          → apply_ime_open_with_view → CONTROLLER.apply(open, view)  // 同期
            → run_chain(caps_chain_for(view), SyncChainWriter)
              → caps(ImmCross, MsIme).chain = [ImmCross, KanjiToggle]
              → ImmCrossProcessStrategy::apply                      // ← 到達する
    ```

    **なぜ漏れたか**: `try_force_on_bootstrap` の doc コメントが
    「未知 Imm32Unavailable アプリで…」と書いており、名前と説明から
    Imm32Unavailable 限定だと読めてしまう。しかし**実際のガードは
    プロファイルを一切見ない**——

    | ガード | 内容 | プロファイルを見るか |
    |---|---|---|
    | `detect_miss_count() >= IME_DETECT_MISS_THRESHOLD` | IME 検出の連続失敗回数 | 見ない |
    | `engine.is_user_enabled()` | エンジン有効 | 見ない |
    | `is_eligible_for_ime_force_on()` | `is_japanese_ime() && effective_open()` | 見ない |
    | `!is_force_on_guard_active()` | force-ON guard 未発動 | 見ない |

    同種の force-ON 経路である `apply_force_on_for_imm_broken`（`:652`）と
    `arm_force_open_pending`（`:780`）は**どちらも
    `!can_use_imm32_cross_process()` を要求する**（前者は満たさなければ即
    `return`、後者は武装条件そのもの）。`try_force_on_bootstrap` だけが
    このプロファイルガードを持たない。したがって Standard
    （`can_use_imm32_cross_process() == true`、LINE / Qt 等）でも到達する。

    この非対称は `state/open_warrant.rs` のテストコメント（`:1166`・`:1187`）が
    既に明示していた——「`try_force_on_bootstrap` 呼び出し元はこちら側
    （`ImmCross` プロファイル）で到達する（`ir_poll_and_learn`／`OsPoll` 経由）」
    「`policy=ImmCross`・観測/意図/guard 一切無し・`desired_open=true`
    （`try_force_on_bootstrap` 相当）」。C-7 の棚卸しはこの既存記述と
    突き合わせていなかった。

    **Phase C の回帰ではない。** Phase C は chain の作り方を
    `WriteMechanism::ALL` の走査から `caps(p, k).chain` へ変えただけで、
    `try_force_on_bootstrap` のガードにも `ImmCrossProcessStrategy` の
    `is_applicable`（`profile.can_use_imm32_cross_process()`）にも触れていない。
    Phase C 以前も同じ呼び出し元から同じ戦略に入っていた。**したがって
    コードの修正は要らない**（誤っていたのは ADR とコード doc の記述だけ）。

    **この訂正が波及する記述**: §6 C-7、§9-18（「潜在的な穴」→「到達しうる」）、
    §9-17 の新項目 17-h、`output/conv_actuation.rs` のモジュール doc、
    `ime_controller.rs::romaji_pre_write` の doc。

    **次にこの領域を触る人へ**: `try_force_on_bootstrap` に
    `!can_use_imm32_cross_process()` を足すのは**挙動変更**であり
    （Standard での bootstrap force-ON が丸ごと止まる）、
    `open_warrant.rs` の差分テストが「Phase 3 実配線で ImmCross の bootstrap
    force-ON 経路が丸ごと無効化される、判明した中で最大の挙動変化」と
    記録している論点そのものである。ADR-087 Phase 3 の配線と一緒に
    判断すること。ここで単独に足してはならない。

---

## 10. 関連

- [ADR-080](080-ime-actuation-lifecycle-and-epoch-fenced-drift-correction.md):
  actuation ライフサイクルと epoch fencing。**不変条件6（`ReadBack` の産物を
  観測として記録しない）を INV-46 で型化する**
- [ADR-081](081-per-profile-capability-driver-decomposition.md): プロファイル別
  capability ドライバ（`ImeProfileDriver`、Phase 1a/1b/1c 試験実装済み・未配線）。
  **本 ADR は capability を trait ではなく const 表で表現することを決めたため、
  §6 で Phase 1d の凍結を提案する。** ADR-081 Phase 1d 検討（2026-08-02）が
  発見した「`GjiFsm` 同期義務の非対称」（`state/gji_direct_mechanism.rs:134-157`）は
  **本 ADR §2.4 / INV-42 が outcome 軸への移行という形で解決する**
- [ADR-082](082-journal-structured-replay-and-event-origin.md): journal 構造化
  リプレイ。§2.1 の `record_replayed(AnyObservation)` がその入口であり、
  **型で消せない実行時 match が 1 箇所だけ残る**
- [ADR-084](084-conv-mode-single-ownership-and-width-ssot.md): conv 単一 actuator
  （P1/INV-1）。§2.3 の Actuation チェーンは「低レベル API を actuator の外から
  呼ばない」を型で表現する試み
- [ADR-086](086-force-write-trigger-and-target-identity.md): force-write の
  トリガー軸・空間軸。**INV-14（`ActuationTarget` によるターゲット同一性）を
  §2.3 の `Actuation<Verified>` で型状態化する。INV-14 の未移行分
  （ImmCross 同期 IMC write、`ime_controller.rs:75` / `:182`）は Phase C へ送る**
- [ADR-087](087-open-belief-actuation-warrant-separation.md):
  `OpenWarrant` / `WarrantBasis` / `issue_open_warrant()`。**維持し、その入力
  （`ObservationStore` に何が入るか）を §2.1〜2.2 が型で守る。`WarrantBasis` の
  7 variant は増やさない。** ADR-087 §8 の「純粋関数として切り出して Linux で
  全数テストする」方針を Phase A/B が継承する
- [ADR-088](088-ime-axis-capability-and-charset-owner.md): **姉妹編。**
  ADR-088 = 「何が壊れているか」（4軸モデル + `CharsetOwner` の発見の記録）、
  本 ADR = 「それを Rust の型でどう表現するか」。
  ADR-088 の `AxisCapability`（軸 × 読み書き可否）と本 ADR の `caps`
  （(profile, IME種別) × 戦略チェーン）は**別物**（§9-3）
- `docs/known-bugs.md`: **BUG-18 / BUG-22**（`GjiFsm` 同期欠落。INV-42/43 が
  型化する失敗モード）、**BUG-19**（conv 由来の間接推測が `desired_open` を
  書き換えた。INV-39 がプール毎判定を却下する根拠）、**BUG-33**（give-up 後の
  観測書き込みによる収束偽装。INV-46 の根拠）、**BUG-14**（外部注入された IME
  モードキーの意図昇格。`IntentWitness::from_physical` の `injected` チェックの
  根拠）、**BUG-43**（`Blind` give-up。INV-41 が「回数制限は型ではない」と
  書く根拠）、**BUG-46**（物理キー抑止。§2.5 で `owns_physical_kanji` を
  `caps` に入れない根拠）
- `docs/experiments.md`: **エントリ01**（IME OFF キーが 5 日間で 6 回反転。
  §2.8 でキー値を `caps` に持たせない根拠、§6 Phase C のゲートの根拠、
  §2.9 で「同一 VK は偶然である」と書く根拠）
- `.claude/rules/ime-belief-architecture.md`: 3 段構えの強制。
  **本 ADR は段3（テキスト検査）から段1（コンパイラ）へ規律を移す。
  段2（dylint）の 3 crate はいずれも移動対象ではなかった**（§7 の訂正）
- `.claude/rules/fix-requires-evidence.md`: Phase B/C は「キー選択」ファミリーに
  該当する（§6「revert する場合の義務」）
- `.claude/rules/experiment-logging.md`: §4 の却下記録はこの規約の ADR レベルでの
  適用
- `.claude/rules/tuning-constants.md`: §8.2 の Phase B レイテンシ実測義務
