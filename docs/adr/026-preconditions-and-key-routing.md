# ADR 026: Preconditions モデルと一元的キールーティング

## ステータス

承認済み（実装完了）

## コンテキスト

### 問題 1: Engine 状態の二重管理

Engine ON/OFF が単一の `enabled: bool` で管理されており、「ユーザーが無効にした」と「IME OFF で自動無効化された」の区別がつかなかった。ウィンドウ切替時にキャッシュから復元する仕組みがあったが、IME 状態との不整合を引き起こしていた。

### 問題 2: IME 状態のキャッシュ多重管理

IME ON/OFF の情報源が複数あり、更新経路が6本に分散していた:
- `IME_STATE_CACHE` (AtomicU8, Off/On/Unknown)
- `ImeCoordinator.shadow_on` (Engine 内部)
- `ImeCacheState` による resolve ロジック（shadow フォールバック）
- `ImeObservation` → Engine → `ImeCacheEffect` → Executor → AtomicU8

### 問題 3: 修飾キーの KeyDown/KeyUp 不整合

Ctrl/Alt/Win キーの処理が散在するバイパスチェック（修飾キー自体、修飾+文字、再入ガード）で行われており:
- KeyDown を Engine に送ったのに KeyUp が途中の修飾キー状態変化でバイパスされる
- Alt+D 等のショートカットが Engine に横取りされる
- Ctrl+Convert 等の Engine コンボが修飾キーバイパスで動かない

## 決定

### 1. Preconditions モデル: user_enabled と環境条件の分離

Engine の有効状態を2軸に分離する:

```
Engine active = user_enabled AND ime_on AND is_romaji AND is_japanese_ime
```

| 軸 | 変更元 | 例 |
|---|---|---|
| `user_enabled` | ユーザー操作のみ | ToggleEngine、Engine ON/OFF コンボ |
| `ime_on` | 環境変化 | IME ON/OFF 検出 |
| `is_romaji` | 環境変化 | ローマ字/かな入力モード検出 |
| `is_japanese_ime` | 環境変化 | 日本語 IME がアクティブか |

Engine は環境条件を内部にキャッシュせず、毎回の呼び出しで `InputContext` として Platform 層から受け取る。

```rust
pub struct InputContext {
    pub ime_on: bool,
    pub is_romaji: bool,
    pub is_japanese_ime: bool,
}
```

### 2. Platform 層のアトミック変数

環境条件は Platform 層の3つのアトミック変数で管理:

| 変数 | 更新経路 |
|------|---------|
| `PRECOND_IME_ON: AtomicBool` | ポーリング (500ms)、フォーカス変更 (即座)、shadow toggle (フック内) |
| `PRECOND_IS_JAPANESE: AtomicBool` | ポーリング (500ms) |
| `IME_IS_KANA_INPUT: AtomicBool` | ポーリング (500ms) |

`build_input_context()` がアトミック変数から `InputContext` を構築し、Engine の各エントリポイントに渡す。

### 3. Shadow IME toggle の Platform 移管

IME トグルキー検知時の shadow 更新（即座の `PRECOND_IME_ON` 反転）を Engine の `ImeCoordinator` から Platform 層（フックコールバック内）に移動。Engine は shadow を持たない。

### 4. 一元的キールーティング（classify_route）

フックコールバック内のすべてのバイパス判定を `classify_route()` 関数に集約。3つのルートで分類する:

```rust
enum KeyRoute {
    Engine,     // Engine が Consume/PassThrough を決める
    TrackOnly,  // Engine に送るが結果を無視、常に OS に PassThrough
    Bypass,     // Engine に一切送らない
}
```

#### KeyDown の分類

```
自己注入キー             → (classify_route の前にチェック、Bypass)
かな入力モード           → (classify_route の前にチェック、Bypass)
Ctrl/Alt/Win 自体        → TrackOnly
修飾キー併用 + 親指キー  → Engine（Ctrl+Convert 等のコンボ用）
修飾キー併用 + 文字キー  → Bypass（ショートカット）
Shift                    → Engine（NICOLA 小指シフト）
通常の文字キー           → Engine（NICOLA 変換対象）
親指キー                 → Engine（同時打鍵判定）
```

#### KeyUp のペア保証

`SENT_TO_ENGINE` ビットセット（256ビット = VK コード全範囲）で KeyDown の判定を記録:

- KeyDown が Engine → `SENT_TO_ENGINE` にビットセット → 対応 KeyUp も Engine
- KeyDown が TrackOnly → `SENT_TO_ENGINE` + `TRACK_ONLY_KEYS` にビットセット → 対応 KeyUp も TrackOnly
- KeyDown が Bypass → ビットなし → 対応 KeyUp も Bypass

途中で修飾キー状態が変わっても、KeyDown の判定が KeyUp に自動追随するため、ペアが構造的に保証される。

#### ビットセットの OS 同期

500ms ポーリングで `sync_sent_to_engine()` を呼び出し、OS の `GetAsyncKeyState` と照合。OS で離されているのにビットが残っているキーをクリアする（KeyUp 取りこぼしの自動回収）。

### 5. TrackOnly の動作

Ctrl/Alt/Win は `TrackOnly` ルートで処理:

1. フックが Engine にイベントを送信（InputTracker の修飾キー状態が更新される）
2. Engine は `Decision::PassThrough` を返す（Passthrough 分類のため）
3. **フックは Engine の結果を無視**して、常に OS に PassThrough

これにより:
- Engine の InputTracker が Ctrl 押下を知る → `Ctrl+Convert` コンボが正しくマッチ
- Engine が誤って Consume しても OS には必ず届く → 修飾キースタック防止
- KeyDown/KeyUp ペアが `SENT_TO_ENGINE` + `TRACK_ONLY_KEYS` で追跡される

### 6. 削除したもの

| 削除項目 | 理由 |
|---------|------|
| `ImeCacheState` (Off/On/Unknown) | `PRECOND_IME_ON: AtomicBool` に統合 |
| `IME_STATE_CACHE: AtomicU8` | 同上 |
| `ImeCacheEffect` (Effect enum) | Engine がキャッシュを管理しなくなった |
| `ImeObservation` 構造体 | Observer がアトミック変数を直接更新 |
| `ImeCoordinator.shadow_on` | Platform 層の `PRECOND_IME_ON` に移動 |
| `ImeObservation.resolve()` | shadow フォールバックが不要に |
| `FocusObservation.ime_open_at_focus` | Observer が `PRECOND_IME_ON` を直接更新 |
| `EngineCommand::SyncImeState` | `RefreshState` に統合 |
| `EngineCommand::ImeObserved` | `RefreshState` にリネーム（引数なし） |
| `ImeReliability` | 削除して動作確認中 |
| `Preconditions` 構造体 | InputContext に統合 |
| ウィンドウごとの Engine 状態キャッシュ | IME 追随モデルで不要に |
| `PlatformRuntime::update_ime_cache` | Effect 経由の更新が不要に |
| `PlatformRuntime::invalidate_ime_cache` | 同上 |
| `PlatformRuntime::save_engine_state` | ウィンドウキャッシュ廃止 |

## 結果

### メリット

- Engine が IME 状態をキャッシュしない → 内部状態と OS 状態の不整合が構造的に発生しない
- user_enabled と環境条件が分離 → 「ユーザーが OFF にした」と「IME OFF で自動 OFF」が区別できる
- キールーティングが `classify_route()` 1箇所に集約 → 散在する early return がなくなった
- KeyDown/KeyUp ペアが SENT_TO_ENGINE ビットセットで構造的に保証 → 修飾キースタック防止
- TrackOnly ルートで「Engine の状態追跡」と「OS への確実な配送」を両立
- ビットセットの 500ms 自動同期で取りこぼしに対する自己修復能力

### デメリット

- `SENT_TO_ENGINE` / `TRACK_ONLY_KEYS` ビットセットという内部状態が増えた（500ms 同期で緩和）
- TrackOnly ルートは Engine に送るが結果を無視するため、Engine の処理が無駄になる（修飾キーのみなので実質ゼロコスト）
- `ImeReliability` を削除したため、Chrome での IME ON 誤検知が発生する可能性がある（動作確認中）

### 実装ファイル

| ファイル | 変更内容 |
|----------|----------|
| `src/engine/engine.rs` | `prev_active` + `compute_active(ctx)` + `check_active_transition(ctx)` |
| `src/engine/decision.rs` | `InputContext { ime_on, is_romaji, is_japanese_ime }`, `EngineCommand::RefreshState` |
| `src/engine/ime_coordinator.rs` | shadow 削除、ガード/deferred のみ残存 |
| `crates/awase-windows/src/hook.rs` | `classify_route()`, `KeyRoute { Engine, TrackOnly, Bypass }`, `SENT_TO_ENGINE` / `TRACK_ONLY_KEYS` ビットセット, `sync_sent_to_engine()` |
| `crates/awase-windows/src/lib.rs` | `PRECOND_IME_ON`, `PRECOND_IS_JAPANESE` アトミック変数 |
| `crates/awase-windows/src/runtime.rs` | `build_input_context()`, `on_command` に `&InputContext` パラメータ追加 |
| `crates/awase-windows/src/observer/ime_observer.rs` | アトミック変数を直接更新（`ImeObservation` 廃止） |
