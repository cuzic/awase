# ADR-069: 凝集性リファクタ（H-1〜M-5）— 循環依存・God Object・Reducer 不変条件の一括改善

## ステータス

採用済み（2026-06-30〜07-01 実装、21 タスク完了）

## コンテキスト

### 問題群

凝集性レビューの結果、以下の問題群が判明した。

#### H-1: 型定義の循環依存

- `ModifierState` が `awase-windows`（プラットフォーム依存クレート）に置かれており、
  `engine` が OS 依存の型を直接 import していた
- `engine::decision` が `platform::EffectOrigin` を参照し、下位層が上位層に依存する逆転が生じていた
- `fsm_types.rs` と `nicola_fsm.rs` が相互 import していた

#### H-2: 状態層の OS グローバル直接呼び出し

- `ObservedState::capture_now()` が `hook::current_tick_ms()` を直接呼ぶため、
  状態層がキャプチャのタイミングを自律的に決定できなかった（テスト困難）
- 状態層が OS のグローバル関数に直接触れており、テスト時のモック差し込みが不可能

#### H-3: Reducer 不変条件の未強制

- `ImeBelief.input_mode` が `pub` で、Reducer を経由せずに直接書き換えられていた
- `conv_mode` の所有権（awase/ユーザー）が散在した enum ではなく `bool` で表現され、
  意味が曖昧だった

#### H-4: Output → Runtime への逆依存

- `output/vk_send.rs` の `spawn_local` 内で `with_app()` を呼んでおり、
  Output モジュールが Runtime の内部構造に直接依存していた

#### H-5: God Object 三連発

- `PlatformState`（〜900 行）: IME 状態・フォーカス状態・ゲート・キーマップを全て保持
- `Output`（〜2000 行）: キー注入・TSF warmup・IME apply 計画・Runtime 通知を全て実装
- `Runtime`（〜800 行）: フォーカス追跡・リフレッシュスケジューリング・IME 協調を一括管理

## 決定

21 タスクを優先度順（H: 高、M: 中）に実施した。

### H-1: 型定義循環依存解消

| タスク | 内容 |
|--------|------|
| H-1-a | `ModifierState` を `awase/src/types.rs` へ移動（OS 非依存クレートへ）|
| H-1-b | `engine::DecisionOrigin` を追加し `platform::EffectOrigin` への依存を除去 |
| H-1-c | `TIMER_PENDING`/`SPECULATIVE` を `fsm_types.rs` へ移動し相互 import を解消 |

### H-2: 状態層 OS 依存除去

| タスク | 内容 |
|--------|------|
| H-2-a | `TickMs` ニュータイプを導入し、`current_tick_ms()` を状態層から隠蔽 |
| H-2-b | `ObservedState::capture_now()` を廃止し `from_snapshot(tick: TickMs)` 経由に |

### H-3: Reducer 不変条件強化

| タスク | 内容 |
|--------|------|
| H-3-a〜d | `ImeBelief.input_mode` を `private` 化し、`ImeEvent::InputMode*` 系バリアント経由でのみ更新 |
| H-3-e | `conv_mode_authority` を `ConvModeAuthority` enum に型付け（`AwaseOwned` / `UserOwned`） |

### H-4: Output → Runtime 逆依存解消

| タスク | 内容 |
|--------|------|
| H-4-a | `RuntimeOutbox` / `RuntimeRequest` を導入し、Output は post するだけ、Runtime が drain する設計に |
| H-4-b | `Output.take_pending_requests()` を event loop に配線し `with_app()` を段階的に置換 |

### H-5: God Object 三連発の分割

| タスク | 内容 |
|--------|------|
| H-5-a | `PlatformState` → `ImeStore` / `FocusStore` / `GateStore` / `KeymapStore` の 4 Facade に分割 |
| H-5-b | `Output` → `KeyInjector`（`output/key_injector.rs`）を抽出 |
| H-5-c | `Output` → `TsfWarmupCoordinator`（`output/tsf_warmup_coord.rs`）を抽出 |
| H-5-d | `Output` → `ImeApplyPlanner`（`output/ime_apply_planner.rs`）を抽出 |
| H-5-e | `Runtime` → `FocusTracker`（`runtime/focus_tracker.rs`）を抽出。後続で `RefreshScheduler` / `ImeCoordinator` も分割 |

### M タスク（優先度中）

| タスク | 内容 |
|--------|------|
| M-1 | `is_engine_processing()` 追加（`TIMER_PENDING` を focus 層から隠蔽） |
| M-3 | `ImePolicyProfile` 追加（`state` → `focus::AppImeProfile` 逆依存を解消） |
| M-4 | `tsf/observer.rs` → `gji_monitor.rs` + `win_event_obs.rs` に分割 |
| M-5 | `engine/decision.rs` → `mode_state.rs` + `idle_check.rs` に分割 |

## 検討した代替案

### 段階的リファクタを先延ばし

→ 採用しなかった。循環依存は放置するほど絡まり、一度解消しないと次のリファクタのコストが指数的に増える。
  21 タスクをまとめて実施することで PR が複雑になるが、各タスクは独立しておりコンフリクトは最小。

### God Object を「整理」だけして分割しない

→ 採用しなかった。`Output` の 2000 行は責務の混在が根本原因であり、コメント整理では解決しない。
  抽出した sub-struct は独立してテスト・変更できる利点がある。

## 新設ファイル一覧

| ファイル | 内容 |
|---------|------|
| `awase/src/types.rs` | `ModifierState`（OS 非依存クレートへ移動） |
| `awase-windows/src/engine/mode_state.rs` | `InputModeState` / `AssumedReason` |
| `awase-windows/src/engine/idle_check.rs` | `should_run_idle_conv_check`（純粋関数） |
| `awase-windows/src/output/key_injector.rs` | `KeyInjector`（VK 送信・VkMarker 管理） |
| `awase-windows/src/output/tsf_warmup_coord.rs` | `TsfWarmupCoordinator` |
| `awase-windows/src/output/ime_apply_planner.rs` | `ImeApplyPlan` / `ImeApplyResult` / `ImeApplyPlanner` |
| `awase-windows/src/runtime/outbox.rs` | `RuntimeRequest` / `RuntimeOutbox` |
| `awase-windows/src/runtime/focus_tracker.rs` | `FocusTracker` |
| `awase-windows/src/tsf/gji_monitor.rs` | GJI CLSID 監視（`tsf/observer.rs` から分割） |
| `awase-windows/src/tsf/win_event_obs.rs` | WinEvent 購読（`tsf/observer.rs` から分割） |

## 関連 ADR

- ADR-032: IME 状態 Reducer 4 層モデル（H-3 の基盤）
- ADR-036: Runtime 境界 API（H-4 の動機）
- ADR-070: `reduce_open_belief` 純粋関数（H-5-d の ImeApplyPlanner に依存）
- ADR-071: deferred VK キュー → TsfWarmupCoordinator 移管（H-5-c の直接的帰結）
