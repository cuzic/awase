# ADR-032: IME 状態モデルの 4 階層 reducer アーキテクチャ

## ステータス

採用済み（2026-05-28 完了、Phase 1〜3 + Phase 3b sync 補完）

## コンテキスト

awase の IME 状態管理は当初、以下のような「5 階層優先度 + サイドバンドガード」
構造で実装されていた:

- **Priority 1〜5**: `sync_key`, `physical_key`, `set_open_request`,
  `focus_probe`, `observer_poll` の 5 種類の「観測ソース」が優先度付きで
  `belief.ime_on` を書き込む
- **サイドバンドガード 7 個**: `ctrl_bypass_hold`, `SyncKeyGate`,
  `focus_transition_pending`, `force_on_until_ms`, `shadow_toggle_suppressed_vks`,
  `ImeRecoveryState`, `ime_detect_miss_count` 等の boolean / 期限付きフラグ

この設計は当初は機能していたが、以下の根本的な問題があった:

1. **責務の混在** — 「ユーザーが望む状態」「OS が報告した状態」「awase が
   適用中の状態」「一時的な例外」が同じ `belief.ime_on` フィールドに
   優先度順で混ざっており、reducer 内で各場面ごとに分岐が増え続けた
2. **observation が intent を破壊する** — `observer_poll` (Priority 5) は
   通常 `set_open_request` (Priority 3) で抑制されるが、focus_probe 後に
   set_open_request が「消費済み」状態になると stale な observe が
   `belief` を上書きし、Engine が誤った認識で動作する
3. **sideband guard の積み増し** — 新しい edge case が見つかるたびに
   boolean guard を追加し、結果として状態遷移の網羅性が把握不能になった
4. **時間軸の暗黙化** — 「直前 300ms 以内」「focus 変更後 50ms grace」等の
   タイミング判定が壁時計依存で散らばり、reproduce や test が困難
5. **`KANJI後 IME-ON Engine-OFF`** ([[project_kanji_imecross_spurious_vk3]])
   などの繰り返し回帰 — sync_key と shadow_toggle と suppress registry が
   3 段階で干渉し、正しい挙動を再構成するのに 1 サイクル数日かかった

### 触媒となった相談

ChatGPT に複雑度の整理を相談し（`IME-State-Model-Complexity-Consultation.md` に
記録）、「異なる責務に分解して別レイヤーに置く」という方針が固まった。

## 決定

### 4 カテゴリ + サポート構造への分解

5 階層優先度を以下の 4 カテゴリに分解し、それぞれ別の型/モジュールに配置する:

| 旧構造 | 新カテゴリ | 役割 |
|---|---|---|
| Priority 1-2 (sync_key, physical_key) | **Intent** | ユーザー/awase が望む状態 |
| Priority 3 (set_open_request) | **Transition** | OS に適用中の状態 (pending) |
| Priority 4-5 (focus_probe, observer_poll) | **Observation** | 外部がそう見えた状態 (per-source) |
| `ctrl_bypass_hold` / `SyncKeyGate` / `focus_transition_pending` | **Barrier** | 入力列・focus・chord 一時隔離 |
| `force_on_*` / `ime_detect_miss_count` | **ForceGuardSet + DriftMonitor** | 回復措置・乖離追跡 |

### モジュール構成

```
crates/awase-windows/src/state/
├ ime_event.rs          — ImeEvent enum (10 variants), EventTime, IntentSource, etc.
├ ime_event_log.rs      — ImeEventLog (512 entries ring buffer)
├ ime_model.rs          — ImeModel (SSOT), 全 event を reduce
├ app_ime_policy.rs     — AppImePolicy (アプリ別ポリシー隔離)
├ observation_store.rs  — ObservationStore (per-source + drift)
├ input_barrier.rs      — CtrlImeChord / FocusTransition
├ force_guard.rs        — ForceGuardSet + DriftMonitor
└ transition.rs         — ImeTransition (generation 付き)
```

### 設計原則（破ったら設計が壊れる）

これらは「コードレビュー時の絶対チェックポイント」であり、
新規 PR は全てこの 6 原則に対して照合する。

1. **UserIntent だけが `desired_open` を即時に変えられる**
   `ImeModel::reduce` の `UserImeSetIntent` / `UserImeToggleIntent` アーム
   以外から `desired_open = ...` を書く実装は禁止
2. **Observer は `desired_open` を直接壊さない**
   `observer/` モジュールから `desired_open` への代入を直接行わない。
   `ImeEvent::ObserverReported` を dispatch して reducer に判断させる
3. **Apply は pending transition として管理する（generation 照合必須）**
   非同期 IME apply の完了 event (`ImeApplySucceeded` / `ImeApplyFailed`)
   は必ず `pending.generation` と照合してから `applied_open` を更新する
4. **App 固有差分は AppImePolicy に閉じ込める**
   `AppKind::*` や `class_name == ...` のハードコード分岐を reducer に
   ベタ書きしない。`AppImePolicy::for_profile()` に集約する
5. **Boolean guard は transaction / barrier / force guard に置き換える**
   新しい edge case で boolean フラグを追加したくなったら、まず
   `InputBarrier` / `ForceGuardSet` で表現できないかを検討する
6. **Event は immutable record + seq による全順序**
   `ImeEvent` は `EventTime { seq: u64, monotonic: Instant, tick_ms: u64 }`
   と一緒に `event_log` に積まれ、reducer の判断は壁時計依存ではなく seq
   依存にする。リプレイ可能性を保証

### 移行コミット履歴

| Phase | Commit | 内容 |
|---|---|---|
| 0 | `50dca39` | ImeEvent + EventLog (足場、behavior 不変) |
| 1 | `c6bbfd9` | Shadow Reducer 並走 + diff log |
| 1.5 | `2ef6bed` | AppImePolicy 導入 (アプリ分岐の隔離) |
| 2A | `15bc804` | last_explicit_intent compat 化 |
| 2B | `80f17fc` | last_explicit_intent フィールド削除 |
| 3 | `02962a2` | ObservationStore (per-source + drift) 統合 |
| test | `783cf8b` | Golden Scenario テスト 12 ケース |
| 4 | `4426a4a` | ctrl_bypass_hold → InputBarrier::CtrlImeChord |
| 5 | `f2f25a7` | focus_transition_pending → InputBarrier::FocusTransition |
| 6 | `1fa0c43` | ForceGuardSet + DriftMonitor 分離 |
| 7 | `bb291b1` | ImeTransition + generation 照合 |
| 3a | `066b70c` | ImeRecoveryState 撤去 → shadow_model.force_guards / drift_monitor |
| 3b | `cd952da` | ImeApplyRequested/Succeeded/Failed event 実コード接続 |
| 3c | `bdb2585` | last_applied_ime_on を model.applied_open に mirror |
| 3d-1 | `bade255` | PlatformState::ime_on() を shadow_model.effective_open() に切替 |
| 3d-2 | `75719b0` | build_input_context() を ime_on 引数化、Engine 判断を shadow SSOT 化 |
| 3b-sync | `fffb522` | sync IME apply path に ImeApplySucceeded/Failed event dispatch 補完 |

## 結果

### メリット

- **責務の明確化** — Intent / Observation / Transition / Barrier が異なる型
  に分かれており、新規 edge case の追加場所が即座に決まる
- **observation が intent を壊さない構造的保証** — `observer/` から
  `desired_open` への直接書き込みパスが存在しない（reducer のみが書ける）
- **generation 照合による stale apply 排除** — 非同期 apply の完了 event が
  別の transition を壊さない（[[feedback_generation_check_for_async_apply]]）
- **時間軸の構造化** — `EventTime` で seq / monotonic / tick_ms を分離し、
  reducer は seq で順序判断、壁時計は表示用のみ
- **AppImePolicy による app 分岐の集約** — LINE/Qt (ImmCross) と
  Chrome/Edge (Imm32Unavailable) と WezTerm (TsfNative) の差分が
  `AppImePolicy` 一箇所で完結

### デメリット

- **学習コスト** — 6 原則を理解しないと PR が原則違反になる
- **diagnostic 用フィールドの一時残存** — `belief.ime_on` /
  `ime_observations` / `log_shadow_diff_if_any` は Phase 3e で撤去予定だが、
  検証期間中は残置（[[project_ime_state_reducer_refactor]] 残タスク 2）
- **observer_poll の挙動変化** — 旧 belief は OS 観測でも更新されたが、
  新 desired_open は UserIntent のみ更新する。OS 直接 IME 操作で乖離する
  可能性は DriftMonitor で検出する設計（実機検証中）

### 影響を受けるファイル

| ファイル | 役割 |
|---------|------|
| `crates/awase-windows/src/state/ime_event.rs` | ImeEvent enum 定義（10 variants） |
| `crates/awase-windows/src/state/ime_event_log.rs` | リングバッファ |
| `crates/awase-windows/src/state/ime_model.rs` | ImeModel reducer (SSOT) |
| `crates/awase-windows/src/state/app_ime_policy.rs` | AppImePolicy |
| `crates/awase-windows/src/state/observation_store.rs` | ObservationStore |
| `crates/awase-windows/src/state/input_barrier.rs` | CtrlImeChord / FocusTransition |
| `crates/awase-windows/src/state/force_guard.rs` | ForceGuardSet + DriftMonitor |
| `crates/awase-windows/src/state/transition.rs` | ImeTransition |
| `crates/awase-windows/src/state/platform_state.rs` | PlatformState の SSOT 切替 |
| `crates/awase-windows/src/executor.rs` | dispatch_ime_set_open の async/sync 分岐 |
| `crates/awase-windows/src/runtime/mod.rs` | flush_sync_apply_events helper |

## 関連 ADR / ドキュメント

- [ADR-026](026-preconditions-and-key-routing.md) — Preconditions と key routing
- [ADR-027](027-ime-state-refresh-and-control.md) — IME 状態 refresh と制御
- [ADR-029](029-ime-detection-resilience.md) — IME 検出の耐障害性と SSOT
- [ADR-030](030-tsf-three-layer-architecture.md) — TSF 3 層分離
- [docs/layer-boundaries.md](../layer-boundaries.md) — 現行レイヤー境界ルール集
- `IME-State-Refactor-Plan.md`（リポジトリルート、移行計画書）
- `IME-State-Model-Complexity-Consultation.md`（ChatGPT 相談記録）
