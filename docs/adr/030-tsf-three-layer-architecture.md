# ADR-030: TSF 状態管理の3層分離アーキテクチャ

## ステータス

採用済み

## コンテキスト

awase-windows の TSF（Text Services Framework）関連コードは長らく
`tsf_observations.rs` と `output.rs` に渾然一体で詰め込まれていた。
具体的には以下の責務がファイル境界を超えて混在していた:

- **observation（観測）**: GJI（Google Japanese Input）プロセスの
  I/O カウンタ監視、WinEvent 経由の候補ウィンドウ表示シグナル受信、
  OBJ_NAMECHANGE 等 Win32 シグナルからの atomic グローバル更新
- **judgement（判断）**: 観測値から「composition は warm か」「TSF は
  ready か」「直前の送信は raw literal だったか」を推論
- **action（実行）**: `SendInput` で VK_DBE_HIRAGANA や ローマ字を注入、
  バックスペース + 再送による raw literal リカバリ
- **メッセージループ統合**: プローブ実行中にキーフックを再入させないため
  キーを退避し、プローブ完了後に `WM_DRAIN_PROBE_QUEUE` 経由で再配送

この結果、以下の問題が顕在化していた:

1. ある atomic グローバル（例: `OBS_GJI_CANDIDATE_SHOW_SEQ`）の書き込み元
   と読み取り元がファイルをまたいで分散し、不変条件の追跡が困難
2. cold start 時の挙動を変えたいだけでも observation / judgement / action
   のどこを触ればいいか不明確
3. macOS / Linux に移植する際、どの層がプラットフォーム依存で、どの層が
   汎用ロジックなのか分離されていない

## 決定

### 3層 + 統合レイヤー構造

`crates/awase-windows/src/tsf/` 配下に以下の4ファイルで TSF サブシステムを
構成する:

```
crates/awase-windows/src/tsf/
├── mod.rs            ─ サブモジュール公開のみ
├── observer.rs       ─ Layer 1: observation
├── probe.rs          ─ Layer 2: judgement
├── output.rs         ─ Layer 3: action
└── probe_bridge.rs   ─ メッセージループ統合
```

#### Layer 1: `tsf/observer.rs`（observation）

OS から生のシグナルを受け取り atomic グローバルに記録するだけの層。
書き込み元は限定された 2 種類のみ:

- **`GjiMonitor` バックグラウンドスレッド**:
  `GetProcessIoCounters` で GJI Converter プロセスの I/O カウンタを
  10ms 間隔でサンプリング → `OBS_GJI_LAST_IO_MS`, `OBS_GJI_MONITOR_OK`
- **`observation_event_proc`（WinEvent コールバック）**:
  GJI candidate window の SHOW/HIDE、WezTerm 等の OBJ_NAMECHANGE を
  受信 → `OBS_GJI_CANDIDATE_VISIBLE`, `OBS_GJI_CANDIDATE_SHOW_SEQ`,
  `OBS_FOCUS_NAMECHANGE_SEQ`, `COMPOSITION_PROBE_SEQ`

この層は判断ロジックを持たない。観測値の解釈は次の層で行う。

#### Layer 2: `tsf/probe.rs`（judgement）

observation 層の atomic を読んで状態を推論する純粋ロジック層:

- **`TsfReadinessProbe`**: VK_DBE_HIRAGANA 送信後、`min_ms` の固定待機を
  経てから GJI I/O 静止（80ms）を監視し「composition 受付可能」を判定
- **`CompositionState`**: warm/cold epoch を管理し、フォーカス変更や
  確定キー入力で自動的に cold に遷移
- **`LiteralDetector`**: ローマ字送信前後で `OBS_GJI_CANDIDATE_*` を
  比較し、composition が成功したか raw TSF literal（ASCII 直接出力）
  だったかを判定

`win32_async::block_on` を内部で動かすが、自身は `SendInput` 等の副作用を
持たない。

#### Layer 3: `tsf/output.rs`（action）

judgement 結果を元に Win32 API を呼び出す副作用層:

- **`ColdReason`** enum: cold になった理由を分類し
  `eager_settle_ms()` / `probe_min_ms()` でタイミングパラメータを決定
- **`INJECTED_MARKER`, `TSF_MARKER`**: `SendInput.dwExtraInfo` の
  自己注入マーカー
- **`make_tsf_key_input`, `make_key_input_ex`**: TSF 専用 KEYBDINPUT
  ビルダー（KEYEVENTF_SCANCODE 等のフラグを正しく組み合わせる）

#### 統合レイヤー: `tsf/probe_bridge.rs`

3 層構造では収まらない、メッセージループとの境界処理を担う:

- **`PROBE_ACTIVE`**: `block_on` ネストループ中はキーフックが
  `APP.get_mut()` を呼ばないようにするフラグ
- **`PROBE_KEY_QUEUE`**: プローブ中に到着したキーイベントの退避先
- **`ProbeGuard`**: `PROBE_ACTIVE` を RAII で false に戻すガード
- **`WM_DRAIN_PROBE_QUEUE` (= `WM_APP + 18`)**: プローブ完了後、退避キューを
  順序保証付きで再配送するためのカスタムウィンドウメッセージ
- **`post_drain_probe_queue()`**: バッチ送信 + `mark_composition_warm`
  完了後に呼び、退避キーを NICOLA に再配送する

### OS 拡張点: `CompositionOutput` trait

OS 非依存レイヤ（`awase` クレート）に `CompositionOutput` trait を定義し、
Windows 実装が `tsf::output` を呼び出す形にする:

```
// src/platform.rs
pub trait CompositionOutput {
    fn send_romaji(&self, romaji: &str);
    fn send_kana_char(&self, ch: char);
    fn is_composition_warm(&self) -> bool;
    fn mark_cold_focus_change(&self);
    fn mark_cold_confirm_key(&self);
    fn mark_cold_ime_toggle(&self);
    fn notify_ime_open(&self, open: bool);
    fn on_focus_changed(&self);
}
```

エンジン側は `PlatformRuntime::composition_output() -> Option<&dyn CompositionOutput>`
経由でアクセスし、OS の composition 機構の有無を実行時に判定できる。

### 移行手順（背景となるコミット）

| コミット | 内容 |
|---------|------|
| `9d9e278` | `tsf` モジュール骨格を新設 |
| `9d527af` (#18) | `OBS_*` グローバルと `GjiMonitor` を `tsf/observer.rs` に集約 |
| `20b391a` (#24) | `observation_event_proc` を `tsf/observer.rs` に移動 |
| `fb5bfd4` (#19) | `TsfReadinessProbe` と `CompositionState` を `tsf/probe.rs` に集約 |
| `a1e4e07` (#20) | `ColdReason` と TSF 専用ロジックを `tsf/output.rs` に切り出し |
| `c71e029` (#21) | `LiteralDetector` を `tsf/probe.rs` に切り出し |
| `f18387c` (#22) | `PROBE_ACTIVE` / `PROBE_KEY_QUEUE` を `tsf/probe_bridge.rs` に移動 |
| `59bff56` (#23) | `awase` に `CompositionOutput` trait を追加 |
| `69088b7` (#25) | `tsf_observations.rs` を撤去、`tsf` サブモジュール直接参照に統一 |

## 結果

### メリット

- 各 atomic グローバルの書き込み元と読み取り元がファイル境界で明確化された
  （observer.rs 先頭コメント参照）
- cold start 動作を調整するときに、observation / judgement / action の
  どこを変えるべきかが即座に判断できる
- `LiteralDetector` や `TsfReadinessProbe` を judgement 層の純粋ロジックとして
  単体テストできる土台ができた
- macOS / Linux 対応時は `crates/awase-windows/src/tsf/` 全体の代替実装を
  作るだけで済む（`CompositionOutput` trait に従えば awase コアは変更不要）
- メッセージループ統合（`probe_bridge.rs`）が独立しているため、
  プローブ実行中の re-entrancy バグを集中的にレビューできる

### デメリット

- ファイル数が増えたため、初見の開発者は最初に `mod.rs` の階層解説を
  読む必要がある
- `tsf/observer.rs` の atomic グローバルは「定義は observer、利用は probe」
  という cross-module 参照になるため、Rust の可視性管理が冗長になる箇所がある

### 影響を受けるファイル

| ファイル | 役割 |
|---------|------|
| `crates/awase-windows/src/tsf/mod.rs` | サブモジュール公開・階層ドキュメント |
| `crates/awase-windows/src/tsf/observer.rs` | observation 層（GJI I/O, WinEvent） |
| `crates/awase-windows/src/tsf/probe.rs` | judgement 層（Readiness, State, Literal） |
| `crates/awase-windows/src/tsf/output.rs` | action 層（SendInput, ColdReason） |
| `crates/awase-windows/src/tsf/probe_bridge.rs` | メッセージループ統合 |
| `src/platform.rs` | `CompositionOutput` trait 定義 |
| `crates/awase-windows/src/output.rs` | `CompositionOutput` の Windows 実装 |

---

## 改訂: 第4層 = warmup オーケストレーション（P4-3, 2026-07-06）

> 本節は既存 3 層（observer / probe / output）+ 統合層の定義を変更しない**追記**である。
> cold-start warmup 実装の成長に伴い、どの層にも属していなかった warmup FSM/coro 群を
> 第 4 層として明示分離した記録。

### 背景

ADR-030 制定時 `tsf/` は 4〜5 ファイルだったが、cold-start warmup 実装が成長し
`tsf/` は 20 ファイル超に膨張。warmup の「多段フェーズを時系列に駆動する」責務を担う
FSM/coro 群が `tsf/` 直下に平置きされ、observer/probe/output のどの層にも属さない
「暗黙の第 4 層」になっていた。バグ調査時にどのファイルを見るべきかの地図が壊れており、
warmup 系の調査コスト・修正漏れの一因となっていた。

### Layer 4: `tsf/warmup/`（warmup オーケストレーション）

**責務を「時系列オーケストレーション + 副作用なしの `ProbeAction` emit」に限定する。**
10ms タイマー (`TIMER_TSF_PROBE`) で駆動される [`TickableFsm`](../../crates/awase-windows/src/tsf/warmup/tickable_fsm.rs)
実装群が、probe → FreshF2 → NameChangeWait → transmit → LiteralDetect → recovery の
シーケンスを進め、副作用を持たない `ProbeAction` を emit する。実際の副作用実行
（`SendInput`・timer 操作）は Layer 3（output）と `output/probe_io.rs` の
`dispatch_probe_actions` が担う。

```
crates/awase-windows/src/tsf/
├── observer.rs         ─ Layer 1: observation（gji_monitor / win_event_obs / tip_detector）
├── probe.rs            ─ Layer 2: judgement（Readiness, CompositionState, LiteralDetector）
│   ├ gji_fsm.rs        ─ Layer 2: warm/cold 判定 SSOT（ADR-046）
│   └ composition_fsm.rs─ Layer 2: warmup タイミング FSM
├── output.rs           ─ Layer 3: action（SendInput, ColdReason）
├── warmup/             ─ Layer 4: warmup オーケストレーション（本改訂で新設）
│   ├ mod.rs
│   ├ tickable_fsm.rs           ─ TickableFsm トレイト（family 共通 IF）
│   ├ probe_fsm.rs              ─ ProbeAction 定義 + TsfProbeCoro + decide_transmit_plan
│   ├ gji_warmup_coro.rs        ─ GjiWarmupCoro（GJI cold-start, StepCoro）
│   ├ sacr_warmup_coro.rs       ─ SacrificialWarmupCoro
│   ├ ime_offon_warmup_fsm.rs   ─ ImeOffOnWarmupFsm（カウンタ FSM）
│   ├ literal_detect_fsm.rs     ─ LiteralDetectCore/Fsm（literal 検出 単一所在地, P4-2）
│   ├ unicode_cold_warmup_fsm.rs
│   ├ unicode_literal_observer.rs
│   ├ chrome_probe.rs
│   ├ cold_warmup.rs            ─ ColdWarmupSequence
│   └ warmup_strategy.rs        ─ ImeWarmupStrategy トレイト, MsImeStrategy
└── probe_bridge.rs     ─ メッセージループ統合
```

### 明示的に第 4 層に **含めない** もの

- **`platform.rs` の FSM ディスパッチャ**（`advance_tsf_probe` / `dispatch_gji_response` /
  `dispatch_composition_response` / `feed_composition_event` / `drain_pending_composition_events` 等、
  約 400 行）は **`WindowsPlatform` に据え置く**。これらは `output` / `timer` / `focus` /
  `composition_fsm` という `WindowsPlatform` 所有の 4 サブシステムを繋ぐグルーであり、
  warmup ロジックではなく `WindowsPlatform` の正当なオーケストレーション責務である。
  warmup 層へ移すと「warmup 層がプラットフォーム全体を触れる」上方依存を新設するだけで
  疎結合は達成されない。加えて `gji_on_focus_change` は `spawn_local`+`with_app` を含み
  warmup 層へ持ち込めない（B-1）ことも、ディスパッチャが platform に属する裏付けである。
- **warm/cold の判定**（`gji_fsm.rs`）と **warmup タイミング FSM**（`composition_fsm.rs`）は
  `ProbeAction` を emit しない判断寄り状態機械のため **Layer 2（`tsf/` 直下）に残す**。
- **`output/tsf_warmup_coord.rs`**（`TsfWarmupCoordinator`）は `Output` が `pub(super)` フィールドを
  直接借用する密結合のため `output/` に据え置く（Layer 4 の中核だが物理配置は output 配下）。

### P4-3 で実際に移動したファイル

上記 `tsf/warmup/` 配下 11 ファイル（tickable_fsm / probe_fsm / gji_warmup_coro /
sacr_warmup_coro / ime_offon_warmup_fsm / literal_detect_fsm / unicode_cold_warmup_fsm /
unicode_literal_observer / chrome_probe / cold_warmup / warmup_strategy）。

## 関連 ADR

- ADR-046: GjiFsm（warm/cold 判定 SSOT、Layer 2 残置の根拠）
- ADR-047: TickableFsm / ImeWarmupStrategy（Layer 4 family の trait）
- ADR-053: StepCoro（GjiWarmupCoro / SacrificialWarmupCoro / TsfProbeCoro の基盤）
- ADR-049: TSF mode LiteralDetect（LiteralDetectCore の対象問題）
