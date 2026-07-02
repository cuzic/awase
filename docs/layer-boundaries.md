# レイヤー境界ルール集

awase のコードベースには複数のレイヤー境界ルールが存在する。
これらは ADR で記録された設計判断 ([ADR-019](adr/019-platform-independence.md) /
[ADR-030](adr/030-tsf-three-layer-architecture.md) /
[ADR-032](adr/032-ime-state-reducer-4-layer-model.md)) を実コードに落とす際の
**実装上の禁則事項** をまとめたもの。

PR レビューと定期的な audit のチェックリストとして使う。

---

## カテゴリ A: クレート境界（プラットフォーム独立性）

### A-1: lib (`awase`) は OS 非依存

**ルール**: `src/` は Windows / macOS / Linux のいずれにも依存しない。

**Why**: ADR-019。macOS/Linux 対応のため lib を OS API 非依存に保つ。

**禁則**:
- `windows-rs` / `core-foundation` 等の OS crate を依存に追加する
- `VkCode.0 == 0xF3` のような OS 固有 magic 値の検査
- `unsafe { Win32API(...) }` の直接呼出

**検出**:
```sh
grep -rn "windows::\|core_foundation\|libc::" src/
```
期待: ゼロ件（cfg(target_os) 経由でも禁止）

### A-2: Engine は事前分類のみ参照

**ルール**: `src/engine/` は `RawKeyEvent` のフィールドのうち
`KeyClassification` / `ImeRelevance` / `PhysicalPos` / `ModifierKey` のみ参照。
`vk_code` / `scan_code` は等値比較のみ可。

**Why**: ADR-019。事前分類アーキテクチャ。プラットフォーム層が分類して渡し、
Engine は内容を「読む」だけ。

**禁則**:
- `vk_code.0 == 0xVV` のような hex 比較
- `match vk_code.0 { 0x10..=0x12 => ... }` のような範囲分岐
- `vk_code.is_xxx()` のような分類メソッド呼出

**検出**:
```sh
grep -rnE "vk_code\.0|VK_[A-Z]+" src/engine/
```
期待: 等値比較とフィールド参照のみ。

---

## カテゴリ B: ランタイム境界（オーケストレーション）

### B-1: `crate::APP` / `with_app` は限定モジュールのみ

**ルール**: `with_app(...)` / `crate::APP.with(...)` を呼ぶのは以下のみ。
他モジュールは Runtime メソッド経由で間接アクセス。

許容モジュール:
- `app/` 配下
- `runtime/`
- `executor.rs`
- `tsf/probe_bridge.rs` (メッセージループ統合)
- `tray/` (システムトレイ menu UI — UI lifecycle 操作はオーケストレーターが担う正当用途)
- `ime_diagnostic.rs` (診断 surface、read-only — 状態の読み取り表示のみで書き換えなし)
- spawn_local closure 内 (async path で再エントリ回避のため必須)

**Why**: ADR-004 (AppState orchestrator)。`crate::APP` の読み書きを集約し、
各モジュールが state にこっそり触れない構造を保つ。

**禁則**:
- `observer/` / `focus/` / `output/` / `ime/` / `state/` 内で `with_app(...)` を呼ぶ

**検出**:
```sh
grep -rn "with_app\|crate::APP\|APP\.with" crates/awase-windows/src/ \
  | grep -v "app/\|runtime/\|executor\.rs\|tsf/probe_bridge\.rs\|tray/\|ime_diagnostic\.rs\|spawn_local"
```
期待: ゼロ件（spawn_local 内の with_app は別途確認）。

### B-2: `output/` は named API のみ使用

**ルール**: `output/` 配下から TSF observation の atomic に触れるときは
`tsf/observer.rs` の named API (`gji_last_io_ms()` / `namechange_baseline()` 等)
を使う。`tsf_obs()` 直接呼出は禁止。

**Why**: ADR-030 / [[project_ime_layer_refactor]]。観測の意図を型に表現する。

**禁則**:
- `output/` 配下で `tsf::observer::tsf_obs()` を呼ぶ

**検出**:
```sh
grep -rn "tsf_obs()" crates/awase-windows/src/output/
```
期待: ゼロ件。

### B-3: `TSF_OBS` 直接アクセスは `tsf/` 内のみ

**ルール**: `TSF_OBS` static は `pub(in crate::tsf)` ガードされており、
`tsf/` モジュール外からの直接アクセスは禁止。

**Why**: ADR-030。observation の SSOT 化。

**検出**:
```sh
grep -rn "TSF_OBS" crates/awase-windows/src/
```
期待: `tsf/` モジュール内のみ。

---

## カテゴリ C: IME 状態 reducer 4 階層モデル（ADR-032）

ADR-032 で定義した 6 つの設計原則をコード上で守るためのチェック項目。

### C-1: Intent は `desired_open` を即時に変更できる唯一の経路

**ルール**: `ImeModel::reduce()` で `desired_open = ...` への代入は
`UserImeSetIntent` / `UserImeToggleIntent` アームのみ。

**Why**: ADR-032 設計原則 1。intent と observation の責務分離。

**禁則**:
- 他の event arm から `self.desired_open = ...` を行う
- reducer 外（observer / focus / executor 等）から `shadow_model.desired_open` を直接代入する

**検出**:
```sh
grep -rn "desired_open\s*=" crates/awase-windows/src/
```
期待: `state/ime_model.rs` の reduce 内 `UserImeSetIntent` / `UserImeToggleIntent`
アームのみ（初期化の `Default::default()` 等は除く）。

### C-2: Observer は `ImeEvent::ObserverReported` 経由で報告

**ルール**: `observer/` モジュールから shadow_model への書き込みは
`ImeEvent::ObserverReported` を dispatch する形のみ。

**Why**: ADR-032 設計原則 2 / [[feedback_observer_never_overrides_desired]]。
Observer が intent を破壊する経路を構造的に塞ぐ。

**禁則**:
- `observer/` 配下から `shadow_model.xxx = ...` の直接代入

**検出**:
```sh
grep -rn "shadow_model\." crates/awase-windows/src/observer/
```
期待: 読み取りのみ、または `dispatch_event(ImeEvent::ObserverReported {...})` 経由のみ。

### C-3: Apply 完了 event は generation 照合必須

**ルール**: `ImeApplySucceeded` / `ImeApplyFailed` を dispatch する直前で、
`shadow_model.pending.generation` と照合する。

**Why**: ADR-032 設計原則 3 / [[feedback_generation_check_for_async_apply]]。
stale な apply 完了が新しい transition を破壊する race を防ぐ。

**禁則**:
- generation を確認せず（または引数なしで）`ImeApplySucceeded` を作成する
- `applied_open = ...` を reduce 外で直接書く（`mirror_applied_open` 経由を除く）

**検出**:
```sh
grep -rn "ImeApplySucceeded\|ImeApplyFailed" crates/awase-windows/src/
```
期待: dispatch 直前に `pending.as_ref().map(|p| p.generation)` 等の取得処理あり。

確認箇所:
- `executor.rs::dispatch_ime_set_open` async path (生成時に照合)
- `runtime/mod.rs::flush_sync_apply_events` (sync path、`fffb522` で追加)

### C-4: App 固有分岐は `AppImePolicy` 配下のみ

**ルール**: `AppKind::*` や `class_name == "..."` 等のハードコード分岐は
`state/app_ime_policy.rs` または `focus/classifier.rs` 内のみ。

**Why**: ADR-032 設計原則 4。reducer 内に app 分岐を漏らさない。

**禁則**:
- reducer 内（`ime_model.rs::reduce`）で `app_kind ==` 分岐
- `class_name.contains("Chrome")` 等の文字列マッチを reducer 内で行う

**検出**:
```sh
grep -rn "AppKind::\|class_name ==" crates/awase-windows/src/ \
  | grep -v "app_ime_policy\|focus/classify\|focus/classifier\|focus/probe"
```
期待: focus/runtime/observer の分類用途のみ、reducer 内ゼロ件。

### C-5: Boolean guard 残骸ゼロ

**ルール**: 旧 boolean guard（`ctrl_bypass_hold`, `focus_transition_pending`,
`shadow_toggle_suppressed_vks`, `ImeRecoveryState`）への参照はコメント含めゼロ。
新規 edge case で boolean guard を追加するときは `InputBarrier` /
`ForceGuardSet` で表現できないかをまず検討する。

**Why**: ADR-032 設計原則 5。sideband guard の積み増しが複雑度の温床になった
履歴（[[project_ctrl_bypass_hold_fix]]）。

**禁則**:
- 旧 boolean guard 名を新規コードで使う
- 「`removed`」コメントを残さずに撤去する（撤去理由が追えない）

**検出**:
```sh
grep -rn "ctrl_bypass_hold\|focus_transition_pending\|shadow_toggle_suppressed\|ImeRecoveryState" \
  crates/awase-windows/src/
```
期待: 「撤去済み」コメントのみ、または完全ゼロ。

### C-6: Event は seq 全順序

**ルール**: 全 `ImeEvent` dispatch は `event_log.record()` 経由で seq が
付与される。reducer 内の順序判断は `envelope.time.seq` または
`envelope.time.monotonic` を使う。`tick_ms` は表示用のみ。

**Why**: ADR-032 設計原則 6。壁時計依存の排除、リプレイ可能性の確保。

**禁則**:
- `event_log.record()` を経由せず reducer を直接呼ぶ
- reducer 内の判断で `SystemTime::now()` / wall clock を使う

**検出**:
```sh
grep -rn "self\.shadow_model\.reduce\|model\.reduce" crates/awase-windows/src/
```
期待: `PlatformState::reduce_with_envelope` 内の 1 箇所のみ。

確認箇所:
- `crates/awase-windows/src/state/platform_state.rs::reduce_with_envelope` (唯一の reduce 呼出)

---

## カテゴリ D: VkCode カプセル化

### D-1: magic hex を `vk.rs` 外で書かない

**ルール**: `VkCode` の hex literal (`0xVV`) は `crates/awase-windows/src/vk.rs`
にのみ存在する。分類は `vk.rs` の helper、log は `UpperHex impl` を使う。

**Why**: [[feedback_vk_encapsulation]]。VK 定数の意図を helper / 定数名で表現。

**禁則**:
- `vk.rs` 外で `0x10..0xFE` 範囲の VK literal を直接書く
- `if vk == VkCode(0x10) { ... }` のような無名比較

**検出**:
```sh
grep -rnE "0x[0-9a-fA-F]{2,4}" crates/awase-windows/src/ \
  | grep -v "vk\.rs\|tests/\|^\s*//\|^\s*\*"
```
期待: VK 以外（タイミング定数、メモリアドレス、HRESULT 等）のみ。

### D-2: ImmCross アプリには物理 IME キーを見せない

**ルール**: `AppImeProfile::ImmCross` (LINE / Qt 等) では物理 KANJI VK
(`VK_KANJI` / `VK_DBE_DBCSCHAR` / `VK_DBE_SBCSCHAR` 等) を KeyDown / KeyUp
両方とも Consume する。passthrough すると spurious VK_F3/F4 連鎖で
shadow toggle が反転する。

**Why**: [[feedback_immcross_owns_kanji]] / [[project_kanji_imecross_spurious_vk3]]。
ImmCross は awase が IME を完全所有するモデル。

**禁則**:
- ImmCross profile で `is_kanji_event` を passthrough する
- 「KeyUp だけは通す」のような非対称処理

**検出**:
`crates/awase-windows/src/runtime/key_pipeline.rs::kp_stage_execute` の
`suppress_physical` 周辺を Read で確認。

---

## カテゴリ E: 1-shot / single-flight ルール

### E-1: `SendMessageTimeoutW` は `spawn_local` 経由

**ルール**: `SendMessageTimeoutW` および同等の同期 SendMessage 呼出は
`with_app` 内で直接呼ばず、`win32_async::spawn_local` 経由で
with_app の外に出す。

**Why**: [[project_in_with_app_removal]]。`SendMessageTimeoutW` がメッセージ
ポンプを動かすと hook callback が再入し、`crate::with_app` の re-entrancy
ガードに引っかかる。

**禁則**:
- `with_app(|app| ...)` 内で `SendMessageTimeoutW` を直接呼ぶ
- 同期 IME 制御 API を fire-and-forget なしで呼ぶ

**検出**:
```sh
grep -rn "SendMessageTimeoutW\|SendMessage\b" crates/awase-windows/src/ \
  | grep -v "imm\.rs\|ime\.rs\|tests/\|//"
```
期待: `imm.rs` (低レベル `send_ime_control` WM_IME_CONTROL ラッパ) または `ime.rs` の async wrapper 内のみ。

---

## audit 実行ガイド

### 一括実行コマンド例

```sh
# カテゴリ A
grep -rn "windows::\|core_foundation\|libc::" src/
grep -rnE "vk_code\.0|VK_[A-Z]+" src/engine/

# カテゴリ B
grep -rn "with_app\|crate::APP" crates/awase-windows/src/ | grep -v "app/\|runtime/\|executor\.rs\|tsf/probe_bridge\.rs\|tray/\|ime_diagnostic\.rs"
grep -rn "tsf_obs()" crates/awase-windows/src/output/
grep -rn "TSF_OBS" crates/awase-windows/src/

# カテゴリ C
grep -rn "desired_open\s*=" crates/awase-windows/src/
grep -rn "shadow_model\." crates/awase-windows/src/observer/
grep -rn "ImeApplySucceeded\|ImeApplyFailed" crates/awase-windows/src/
grep -rn "AppKind::\|class_name ==" crates/awase-windows/src/ | grep -v "app_ime_policy\|focus/classify"
grep -rn "ctrl_bypass_hold\|focus_transition_pending\|shadow_toggle_suppressed\|ImeRecoveryState" crates/awase-windows/src/

# カテゴリ D
grep -rnE "0x[0-9a-fA-F]{2,4}" crates/awase-windows/src/ | grep -v "vk\.rs\|tests/\|^\s*//"

# カテゴリ E
grep -rn "SendMessageTimeoutW" crates/awase-windows/src/ | grep -v "imm\.rs\|ime\.rs"
```

### 違反候補の分類

検出結果は以下 3 段階に分類してから対応を決める:

- **Violation**: 真の違反。即座に修正 PR を立てる
- **Transitional**: 過渡期で許容（`belief.ime_on` 等 Phase 3e 撤去予定のもの）。
  対応する撤去タスク [[project_ime_state_reducer_refactor]] 残タスク 2 に紐付ける
- **Comment-only**: コメントや「removed」マーカーのみ。grep が拾うが実害なし

### 優先度

architectural risk の高い順:
1. C-1 (Intent → desired_open 経路) — 違反すると Observer/Intent の責務分離崩壊
2. C-2 (Observer → ObserverReported 経由) — 違反すると intent 破壊復活
3. C-3 (generation 照合) — 違反すると stale apply race 復活
4. A-1, A-2 (lib プラットフォーム独立性) — 違反すると macOS/Linux 対応コスト爆発
5. その他

---

## 関連ドキュメント

- [ADR-019](adr/019-platform-independence.md) — プラットフォーム独立性
- [ADR-030](adr/030-tsf-three-layer-architecture.md) — TSF 3 層分離
- [ADR-032](adr/032-ime-state-reducer-4-layer-model.md) — IME 状態 reducer 4 階層モデル
- [ADR-004](adr/004-appstate-orchestrator.md) — AppState orchestrator
