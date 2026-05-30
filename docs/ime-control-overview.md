# IME 制御 アーキテクチャ概要

> 最終更新: 2026-05-29  
> 対象: awase Windows 実装（`crates/awase-windows/src/`）

---

## 1. 全体像

awase の IME 制御は 6 つの責務レイヤーに分かれている。

```
┌──────────────────────────────────────────────────────┐
│  物理キーボード / Windows IME / Google日本語入力     │
└──────────────────────────────────────────────────────┘
         ↕ WH_KEYBOARD_LL フック / OS メッセージ
┌──────────────────────────────────────────────────────┐
│  Hook 層   hook.rs                                   │
│  全物理キーをインターセプト・分類し、エンジンスレッドへ転送 │
└──────────────────────────────────────────────────────┘
         ↕ PostThreadMessageW(WM_KEYEVENT)
┌──────────────────────────────────────────────────────┐
│  Engine 層   src/engine/                             │
│  状態機械(FSM)がキーを判定し Decision を生成          │
└──────────────────────────────────────────────────────┘
         ↕ Decision { PassThrough / Consume + Effects }
┌──────────────────────────────────────────────────────┐
│  Executor 層   executor.rs                           │
│  Effect を実行 (Input / Ime / Timer / Ui)             │
└──────────────────────────────────────────────────────┘
         ↕ ImeEffect::SetOpen { open, origin }
┌──────────────────────────────────────────────────────┐
│  Controller 層   ime_controller.rs                   │
│  Strategy Pattern で制御方式を選択                    │
│  ImmCross → GjiDirect → KanjiToggle                 │
└──────────────────────────────────────────────────────┘
         ↕ Windows API
┌──────────────────────────────────────────────────────┐
│  OS API 層   ime.rs / imm.rs                         │
│  SendMessageTimeoutW / SendInput(VK_KANJI, F13, F14) │
└──────────────────────────────────────────────────────┘
         ↕ Observer ポーリング（非同期）
┌──────────────────────────────────────────────────────┐
│  State 層   state/ime_model.rs 他                    │
│  ImeModel (SSOT) + ObservationStore + ImeEventLog    │
└──────────────────────────────────────────────────────┘
```

**注意: 上の図はデータフローの方向を示すパイプライン図。State 層は末端ではなく、
Runtime / Executor / Controller / Observer がすべて参照する中心に位置する。**

```
Hook → Runtime → Engine → Executor → Controller → OS API
                  ↑          ↓
                  │       ImeEffect
                  │          ↓
              ImeDecisionView
                  ↑
          ImeModel / Reducer (SSOT)
                  ↑
      Observer / Focus / GJI / Shadow
```

---

## 2. キーイベントから IME 操作までのフロー

### 2-1. Hook 層（hook.rs）

`SetWindowsHookExW(WH_KEYBOARD_LL)` が全物理キーを受け取る。

| 処理 | 詳細 |
|------|------|
| 自己注入除外 | `INJECTED_MARKER` / `TSF_MARKER` 付きは素通り |
| キー分類 | `classify_key()` → `KeyClassification + PhysicalPos` |
| IME 関連性 | `classify_ime_relevance()` → `ImeRelevance` |
| 転送 | `PostThreadMessageW(WM_KEYEVENT)` でエンジンスレッドへ |

**ImeRelevance の構造:**

```rust
pub struct ImeRelevance {
    pub may_change_ime: bool,      // この VK が IME ON/OFF に関係するか
    pub shadow_action: Option<ShadowImeAction>, // TurnOn / TurnOff / Toggle
    pub is_sync_key: bool,         // 半角全角など OS が直接制御するキー
    pub is_ime_control: bool,      // awase が制御する IME キー
}
```

**物理 IME キーのシャドウ更新順序:**

半角/全角・無変換など OS 自身が IME 状態を変えるキーが FilterMode で PassThrough される場合、
`shadow_action` は「OS が変えるであろう状態」の予測更新として扱われる。これは
`desired_open` を変更しない。`ObservedState` / `ShadowState` 側に入り、
後続の Observer 確認で検証される。

```
物理キーを hook が見る
↓ shadow_action として予測記録（desired_open は変えない）
OS に pass-through
↓
OS 側の IME 状態が変わる
↓
Observer が後から観測して予測を検証
```

### 2-2. Runtime 層（runtime/mod.rs）

```
on_key()
  ├─ InputContext を構築
  │   ├─ ime_on = shadow_model.effective_open()   ← Engine 向け policy-decorated 値
  │   ├─ input_mode = belief.input_mode()
  │   └─ modifiers スナップショット
  └─ engine.on_key(raw_event, input_context)
```

**`effective_open()` の意味:** これは実際の OS 観測値でも純粋なユーザー意図でもない。
「Engine が入力処理上 IME ON とみなすべきか」という policy-decorated な値である。
公開 API は `platform_state.ime_on()` でラップされる。詳細は §4-2 を参照。

### 2-3. Engine 層（src/engine/）

NICOLA FSM が状態遷移を計算し `Decision` を返す。

```rust
pub enum Decision {
    PassThrough,                          // OS に素通し
    PassThroughWith { effects: EffectVec }, // 素通し + 副作用
    Consume { effects: EffectVec },        // 握りつぶし + 副作用
}

pub enum ImeEffect {
    SetOpen { open: bool, origin: EffectOrigin },
    RequestRefresh,
}
```

### 2-4. Executor 層（executor.rs）

| モード | 動作 |
|--------|------|
| Filter | PassThrough キーは OS に通す。重い処理は `WM_EXECUTE_EFFECTS` に defer |
| Relay | 全キー消費。全 Effect をメッセージループで FIFO 実行 |

**FilterとRelayのIME制御への影響:**

| 観点 | Filter | Relay |
|------|--------|-------|
| OS への物理キー到達 | あり | なし / 再注入のみ |
| IME 状態変化の主体 | OS + awase | awase 中心 |
| race リスク | 高い | 低い |
| latency | 低い | やや高い |
| shadow 更新 | 予測が必要 | FIFO で管理しやすい |
| 対象 | 通常アプリ | 問題アプリ / TSF 系 |

物理 IME キーは `AppImePolicy.owns_physical_kanji` に従って Filter/Relay どちらのモードでも
awase が完全所有する（アプリに通さない）ケースがある。詳細は §5-3。

`ImeEffect::SetOpen` を受け取ると:

1. `ImeModel` に `UserImeSetIntent` event を記録
2. `build_ime_control_view()` でスナップショット化
3. `ImeController::apply(open, view)` を呼び出す

---

## 3. IME 制御戦略（Controller 層）

`ime_controller.rs` は Strategy Pattern で 3 段階のフォールスルーを実装する。

```
ImeController::apply(desired_open, view)
  ├─ [1] ImmCrossProcessStrategy    ← 標準 (IMM32-bridge)
  │       is_applicable(): profile.can_use_imm32_cross_process()
  │       実装: set_ime_open_cross_process()
  │         GetGUIThreadInfo() → hwnd_focus
  │         ImmGetDefaultIMEWnd(hwnd) → ime_wnd
  │         SendMessageTimeoutW(ime_wnd, WM_IME_CONTROL, IMC_SETOPENSTATUS)
  │
  ├─ [2] GjiDirectStrategy          ← Google日本語入力専用（全プロファイル共通）
  │       is_applicable(): gji_monitor_ok == true
  │       実装: post_gji_ime_on() / post_gji_ime_off()
  │         IME ON  → SendInput(F13)  // GJI ひらがなへ
  │         IME OFF → SendInput(F14)  // GJI IME-OFF
  │
  └─ [3] KanjiToggleStrategy        ← 最終フォールバック
          is_applicable(): 常に true
          実装: post_kanji_toggle_to_focused()
            effective_shadow (shadow || candidate_visible || candidate_was_seen) == desired
              → AlreadyMatched
            else → SendInput(VK_KANJI) with IME_KANJI_MARKER
```

**フォールスルー規則:**

| `ImeOpenOutcome` | 次戦略に進むか |
|------------------|---------------|
| `Applied` | 停止（確認済み成功） |
| `AlreadyMatched` | 停止（no-op） |
| `FallbackSent` | 停止（未確認だが送信済み。追加送信は危険） |
| `Failed` | 次の戦略へ進む |
| `UnsafeToToggle`（将来追加） | 停止・guard 設定 |

**重要:** `FallbackSent`（VK_KANJI 送信済み）でも次の戦略には進まない。
IME 操作は「確認できないからもう一発」が最も危険（toggle 系なら反転する）。
ImmCross が `Applied` を返した場合も同様に後続戦略はスキップされる。

### アプリプロファイル分類（focus/class_names.rs）

| AppImeProfile | 対象アプリ | 使用戦略 |
|---------------|------------|----------|
| `Standard` | WezTerm, メモ帳, etc. | ImmCross → GJI → KANJI |
| `Imm32Unavailable` | Chrome, Edge | GJI → KANJI |
| `TsfNative` | UWP アプリ等 | TSF 経由 |

### AppImeProfile と AppImePolicy の責務分離

| 型 | ファイル | 役割 |
|----|----------|------|
| `AppImeProfile` | `focus/class_names.rs` | 技術的な IME 制御方式の分類（クラス名から導出） |
| `AppImePolicy` | `state/app_ime_policy.rs` | awase としての振る舞い制約（policy オブジェクト） |

`AppImePolicy` の主なフィールド:
- `owns_physical_kanji`: 物理 KANJI キーを awase が完全所有するか
- `observer_false_on_focus`: フォーカス直後の false 観測の扱い
- `actuator_kind`: IME 制御 actuator の種別
- `focus_settle_ms`: フォーカス後に observer を信頼できるまでの待ち時間
- `observer_poll_role`: observer の役割（belief 更新 vs health checker）

LINE/Qt のような「IMM32 では制御できるが物理 IME キーを見せたくない」ケースは
Profile ではなく Policy で表現する:

```text
LINE = Standard（AppImeProfile）+ owns_physical_kanji=true（AppImePolicy）
```

### KanjiToggleStrategy の confidence gate

VK_KANJI はトグル操作であり**冪等ではない**。F13/F14（GJI）や IMM32 SetOpen と異なり、
shadow が stale な状態で送ると意図と逆方向に反転する。

現在の実装では `effective_shadow`（shadow || candidate_visible || candidate_was_seen）を
guard として使っているが、以下の条件が揃ったときのみ安全に使える:

- shadow が信頼できる（observer が正常に動作している）
- focus 直後でない（OS 側の状態が安定している）
- pending transition がない
- 直前の toggle から十分な時間が経過している

`ImeControlView` にこれらの信頼度フィールドが追加された段階で、
将来的に `UnsafeToToggle` を返す条件として実装する（§6 原則7 参照）。

---

## 4. 状態管理（ImeModel SSOT）

### 4-1. ImeModel の構造

```
ImeModel (state/ime_model.rs)
├─ desired_open: bool          ← SSOT: ユーザー意図 (UserIntent のみ変更可)
├─ last_intent                 ← 最後の意図ソース
├─ observations: ObservationStore ← per-source 観測値
│   ├─ Priority 0-5 (多段観測)
│   ├─ Shadow (物理フック)
│   ├─ Fallback (タイムアウト時)
│   └─ Suspicious (信頼度低)
├─ app_policy: AppImePolicy    ← アプリ別ポリシー
├─ input_barrier              ← Ctrl+IME chord transaction
├─ force_guards: ForceGuardSet ← 強制 ON ガード
├─ drift_monitor: DriftMonitor ← 観測失敗追跡
├─ pending: Option<ImeTransition> ← apply 進行中 transition
├─ applied_open: Option<bool>  ← 最後の apply 成功状態
└─ applied_at_ms: u64         ← apply 成功時刻 (0=未確認)
```

### 4-2. IME 状態の 4 種類の意味論

IME 制御では「何の状態か」を混同しないことが最重要。以下の 4 つは明確に区別する:

| 値 / メソッド | 意味 | 変更可能な経路 |
|---|---|---|
| `desired_open` | ユーザーが意図している状態 | UserIntent イベントのみ |
| `observations`（ObservationStore） | OS / IME から実際に観測した状態 | Observer イベントのみ |
| `applied_open` | awase が最後に apply 成功とみなした状態 | ImeApplySucceeded（generation 照合済み） |
| `effective_open()` | Engine が入力処理上 IME ON と扱うべきか | derived（force_guards override 込み） |

`effective_open()` は実際の OS 観測値でも純粋なユーザー意図でもなく、
**force_guards による policy-decorated な判断値**である。
Engine の InputContext 構築に使う（`platform_state.ime_on()` 経由）。
Controller に渡す目標値も `effective_open()` の値を使う。

### 4-3. effective_open() の詳細

```rust
// force_guards が active な間は desired_open を無視して true を返す
fn effective_open(&self) -> bool {
    self.force_guards.effective_open(self.desired_open)
}
```

**既知の制限:**
- `FocusChanged` 時に `force_guards` をクリアしない（旧フォーカスの guard が引き継がれる）
- `UserImeSetIntent` で逆方向の intent が来ても `force_guards` が残る

フォーカス変更時または UserIntent が来たときに `force_guards` を invalidate する仕組みは
今後の改善として必要。`ForceGuard` に `focus_generation` フィールドを追加し、
フォーカス変更時に世代が変わった guard を失効させるのが望ましい。

### 4-4. ForceGuardSet

```rust
pub struct ForceGuard {
    pub reason: ForceOnReason,     // BrokenAppBootstrap / PanicReset / DetectMissThreshold / ProfilePolicy
    pub expires_at: Option<Instant>, // TTL（None = 永続）
    pub generation: u64,           // 発火時の状態 generation
}
```

guard が active な条件: `guards` が空でない（TTL 未失効）。

**TTL 設計の方針:**
- `BrokenAppBootstrap`: フォーカス変更で無効化すべき（`focus_generation` 追加が必要）
- `PanicReset`: 確認済み観測で無効化すべき
- `DetectMissThreshold`: Observer 成功で無効化（実装済み）

### 4-5. Observer ループ

```
TIMER_IME_REFRESH (統合タイマー)
  ├─ 通常ポーリング: 500ms 周期
  ├─ フォーカス変更後: 50ms (Engine がリセット)
  └─ SetOpen 実行後: 20ms (安全ネット)

refresh_ime_state()
  └─ read_ime_state_full(hwnd_focus, 50ms timeout)
       ├─ get_ime_open_status()  → Some(true/false) or None
       ├─ get_ime_conversion_mode()
       └─ detect_ime_language()
  └─ classify_ime_snapshot() → ImeUpdate
  └─ PlatformState::apply_ime_update(ImeUpdate)
       └─ shadow_model.reduce(ObservationEvent)
            ├─ observations に記録
            ├─ miss_count reset/increment
            └─ force_guard を clear (成功時)
```

**Observer の 3 値意味論:**

| 結果 | 意味 | 動作 |
|------|------|------|
| `Some(true/false)` | 観測成功 | observations に記録、キャッシュ更新 |
| `None` | 観測失敗 (timeout 等) | 前回値を維持 |
| miss_count ≥ 3 | 連続失敗 | `force_on_broken_app_bootstrap` = true |

---

## 5. 特殊ケース処理

### 5-1. Chrome / Edge（Imm32Unavailable）

IMM32 ブリッジが使えないため GJI → KANJI のフォールバックを辿る。

- **GJI 検出済み**: F13/F14 で直接制御（`candidate_visible` 中も動作）
- **GJI 未検出**: VK_KANJI トグル（`effective_shadow` と `desired` が異なる時のみ送信）

### 5-2. Google 日本語入力（GJI）検出

```
gji_observer.rs::observe_gji_after_focus()
  ├─ tsf_obs().gji_last_io_ms() を読む
  ├─ フォーカス変更後 2500ms 以内に I/O あり → GJI 検出
  └─ gji_monitor_ok = true → GjiDirectStrategy 使用
```

**現在の実装:** `gji_monitor_ok: AtomicBool`（2値）

**既知の限界と将来方向:** focus 直後の 2500ms は判定が不確定。将来は以下の4状態に拡張予定:

| 状態 | 意味 | KanjiToggle へのフォールバック |
|------|------|-------------------------------|
| `Unknown` | まだ判定できない（focus 後 2500ms 以内）| 禁止 |
| `Present` | GJI I/O を確認済み | 不要（F13/F14 使用） |
| `Absent` | GJI なしと確認 | 条件付きで許可 |
| `Broken` | GJI はあるが異常 | 禁止または保守的動作 |

`Unknown` と `Absent` を区別することで、未確定状態での VK_KANJI 誤送信を防げる。

**VK_KANJI を使わない理由:** GJI は VK_KANJI を「トグル」として解釈するが、
F13/F14 は冪等（ON なら ON、OFF なら OFF）のため desync が起きない。

### 5-3. ImmCross アプリ（LINE, Qt 等）

IMM32 経由では制御できるが、物理の IME キー（VK_KANJI）を見せると
spurious な連鎖反応が起きるアプリ群。

**設計原則:** ImmCross アプリには物理 IME キーを一切通さない。
VK_KANJI を送る場合は `IME_KANJI_MARKER` を付けて自己注入として識別し、
フック側で素通りさせる。

### 5-4. Ctrl+無変換 IME-OFF 救済機構

LINE 等で `Ctrl+無変換` を押したとき、Ctrl を早く離すと
親指シフトキーとして誤認識されるバグへの対策。

```
Ctrl↓ → CTRL_CONSUMED_SINCE_DOWN = false
他キー↓ → CTRL_CONSUMED_SINCE_DOWN = true
無変換↓
  └─ ctrl_consumed_since_down() == true
       → 50ms 救済窓を設定 (TIMER_IME_OFF_RESCUE)
       → 窓内に Ctrl↑ 来ても IME-OFF を実行

hook.rs: 親指キーを SavedRescueEvent から除外
         → thumb shift として化けるバグを構造的に撲滅
```

---

## 6. 設計 7 原則（ADR-032）

これらは PR レビューの絶対チェックポイント。違反すると IME 状態が破壊される。

| # | 原則 | 違反パターン |
|---|------|------------|
| 1 | **UserIntent だけが `desired_open` を即時変更できる** | observer から直接 `desired_open = ...` |
| 2 | **Observer は `desired_open` を直接書き換えない** | `observer/` から代入（reducer 経由必須） |
| 3 | **Apply は generation 照合必須** | stale な `ImeApplySucceeded` で `applied_open` を上書き |
| 4 | **アプリ固有差分は `AppImePolicy` に閉じ込める** | reducer に `class_name == "..."` のベタ書き |
| 5 | **Boolean guard は transaction/barrier/force guard に置換** | `bool` フラグ追加で edge case を塞ぐ |
| 6 | **Event は immutable record + seq による全順序** | 壁時計依存の reducer ロジック |
| 7 | **Toggle 操作は confidence gate を通過した場合のみ許可する** | stale shadow / focus 直後 / pending 中に VK_KANJI を送る |

**原則 7 の違反パターン（VK_KANJI 逆転リスク）:**

```
観測失敗中 / focus 直後 / pending 中 / stale shadow のまま VK_KANJI を送る
→ IME 実態 OFF → VK_KANJI → ON（意図と逆）
```

F13/F14 や IMM32 SetOpen は冪等だが、VK_KANJI は冪等でない。
`KanjiToggleStrategy` を安全に使える条件は §3 参照。

---

## 7. タイミング定数一覧

| 定数 | 値 | 用途 |
|------|----|------|
| IMM32 Set タイムアウト | 150ms | `SendMessageTimeoutW`（composition tear-down 待ち） |
| IMM32 Get タイムアウト | 50ms | `SendMessageTimeoutW`（軽い照会） |
| OUTPUT_GUARD | 150ms | reinject キー間の OS 状態安定化 |
| 通常ポーリング周期 | 500ms | `TIMER_IME_REFRESH` デフォルト |
| フォーカス変更後リフレッシュ | 50ms | Engine がリセット |
| SetOpen 後リフレッシュ | 20ms | 安全ネット |
| GJI 検出窓 | 2500ms | フォーカス後の GJI I/O 確認 |
| Ctrl+無変換 救済窓 | 50ms | TIMER_IME_OFF_RESCUE |
| drift_monitor 閾値 | 3 | 連続観測失敗 → force_on 発動 |

**タイミング定数の限界:** 複数のタイマーが重なる状況では数値調整だけでは対処しきれない。
将来的には遷移フェーズ（`IntentRecorded → ApplyDispatched → AwaitingFirstObservation → Verified`）
に各タイムアウトを紐づける設計が望ましい（`state/transition.rs` の `ImeTransition` を拡張）。

---

## 8. 主要ファイル一覧

### ime 制御コア

| ファイル | 役割 |
|----------|------|
| `ime_controller.rs` | Strategy Pattern で制御方式を選択 |
| `ime.rs` | OS API ラッパー（cross-process, GJI, KANJI） |
| `imm.rs` | IMM32 低レベルユーティリティ（RAII・タイムアウト） |

### 状態管理

| ファイル | 役割 |
|----------|------|
| `state/ime_model.rs` | ImeModel SSOT（reducer） |
| `state/ime_event.rs` | ImeEvent enum 10 variants |
| `state/ime_event_log.rs` | 512 エントリリングバッファ |
| `state/observation_store.rs` | per-source 観測値ストア |
| `state/transition.rs` | ImeTransition + generation 管理 |
| `state/force_guard.rs` | ForceGuardSet + DriftMonitor |
| `state/input_barrier.rs` | CtrlImeChord / FocusTransition |
| `state/app_ime_policy.rs` | アプリ別ポリシー |
| `state/ime_decision_view.rs` | Engine 判断用統一ビュー |

### 観測層

| ファイル | 役割 |
|----------|------|
| `observer/ime_observer.rs` | IME 状態ポーリング・スナップショット分類 |
| `observer/gji_observer.rs` | GJI I/O 検出 |
| `observer/focus_observer.rs` | フォーカス・修飾キー観測 |

### フック・実行

| ファイル | 役割 |
|----------|------|
| `hook.rs` | WH_KEYBOARD_LL コールバック・キー分類 |
| `executor.rs` | Decision → Effect 実行（Filter/Relay） |
| `runtime/mod.rs` | InputContext 構築・Engine 呼び出し |

---

## 9. 関連 ADR

| ADR | タイトル |
|-----|---------|
| [ADR-027](adr/027-ime-state-refresh-and-control.md) | IME 状態リフレッシュと制御キーの設計 |
| [ADR-029](adr/029-ime-detection-resilience.md) | IME 検出の耐障害性と SSOT |
| [ADR-032](adr/032-ime-state-reducer-4-layer-model.md) | IME 状態モデルの 4 階層 reducer アーキテクチャ |
| [ADR-021](adr/021-deferred-effect-execution.md) | Effect 遅延実行 |
| [ADR-014](adr/014-observer-executor-runtime.md) | Observer / Executor / Runtime 分離 |
| [ADR-030](adr/030-tsf-three-layer-architecture.md) | TSF 3 層分離 |
| [layer-boundaries.md](layer-boundaries.md) | 7 設計原則の grep audit ルール集 |
