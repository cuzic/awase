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
