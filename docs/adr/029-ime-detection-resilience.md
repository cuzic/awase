# ADR 029: IME 状態検出の耐障害性と SSOT 設計

## ステータス

採用済み

## コンテキスト

awase は IME 状態（ON/OFF、ローマ字/かな）を OS から読み取って Engine の活性化を判断する。しかし Windows の IME 状態検出は以下の理由で本質的に不安定である:

1. **IMM32 はスレッドローカル設計**: `ImmGetContext` はクロスプロセスで動作しない。`WM_IME_CONTROL` 経由のブリッジは UWP/Chrome/Electron で不完全
2. **ブロッキングリスク**: `GetGUIThreadInfo`, `ImmGetConversionStatus`, `AccessibleObjectFromWindow` 等は対象プロセスがハングすると無期限ブロック
3. **suspend/resume 後の遅延**: OS 復帰直後は IME サービスが未回復で検出が失敗する
4. **TSF CompartmentEventSink はスレッドローカル**: 他プロセスの IME 変更を通知できない

### 調査結果

AutoHotkey, zenhan, alt-ime-ahk, Keyhac 等の主要ツールを調査した結果、**クロスプロセス IME 検出の完全な解決策は存在しない**ことが判明。全ツールが同じ IMM32 ベースのアプローチを使用しており、検出失敗に対する各種フォールバックを持つ。

TSF `ITfCompartmentEventSink` を実装・検証したが、`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` は thread-manager スコープであり、他プロセスの変更を検知できないため削除した。

## 決定

### 多層防御アーキテクチャの採用

IME 状態管理を以下の 3 層で構成する:

```
Layer 1: Shadow 追跡（即時、キーイベントベース）
  ├─ ハードウェア IME キー（VK_KANA, VK_DBE_HIRAGANA 等）
  ├─ Config sync_keys（ユーザー定義）
  └─ Engine IME 制御キー（Ctrl+変換 等）

Layer 2: OS 検出（500ms ポーリング + フォーカスフック）
  ├─ ImmGetDefaultIMEWnd + SendMessageTimeoutW（クロスプロセス）
  ├─ ワーカースレッドタイムアウト保護（300ms）
  └─ ブラックリスト（Chrome/UWP/Electron）でスキップ

Layer 3: SSOT フォールバック（検出失敗時）
  ├─ ime_detect_miss_count ≥ 3 → awase が SSOT に昇格
  ├─ ime_force_on_guard でキャッシュ値を OS に書き戻し
  └─ フォーカス変更時にガードリセット（per-window 状態を尊重）
```

### IMM 能力の動的学習

アプリごとの IMM ブリッジ能力を実行時に学習し、キャッシュする:

1. **初回判定**: `ImmGetDefaultIMEWnd(hwnd)` — NULL なら TSF-only と推定
2. **ランタイム学習**: 検出成功/失敗の実績を `class_name` ごとに記録
3. **永続化**: `imm_cache.toml` に保存（再起動後も学習結果を維持）
4. **AppKind 昇格**: IMM Broken と学習されたアプリは `AppKind::Chrome` に昇格 → PerKey 出力

### AppKind ベースの出力モード自動選択

| AppKind | Romaji 出力 | 理由 |
|---------|------------|------|
| Chrome (Chrome, Edge, VS Code, wezterm 等) | PerKey | 独自 TSF text store が VK を composition |
| Win32 (Notepad, Word 等) | Unicode | IMM 経由の直接入力が安定 |
| UWP (ストアアプリ) | Unicode | TSF が VK を composition できない |

### ブロッキング保護

- `run_with_timeout()`: 任意の Win32 API 呼び出しをワーカースレッドで実行（300ms タイムアウト）
- リークスレッド GC: 完了したワーカースレッドを自動回収、上限 8 件で資源保護
- suspend/resume: `TIMER_IME_REFRESH` をキャンセルし 3 秒後に軽量復帰

### Decision::effects_mut バグの修正

`Decision::effects_mut()` と `push_effect()` が `PassThrough` を `Consume` に昇格させるバグがあり、Engine 活性化遷移で UiEffect を追加すると IME キーが OS に届かなくなっていた。`PassThroughWith` に正しく昇格させるよう修正。テストも修正済み。

## 結果

### メリット

- suspend/resume 後のハングが解消（ワーカースレッド + ブラックリスト）
- Chrome/UWP/Electron での IME 検出失敗が安全に処理される
- 新しいアプリでも IMM 能力を自動学習し適応する
- PerKey/Unicode の自動選択で IME composition の有無に正しく対応
- VK_KANA 等のハードウェア IME キーが正しく shadow 追跡される

### デメリット

- ブラックリストアプリでは言語バーのマウス操作による IME 切替が検知不能
- 学習キャッシュが誤った場合、ユーザーがトレイメニューから手動クリアが必要
- ワーカースレッドリークが上限に達すると IME 検出が一時停止する

### 影響を受けるファイル

| ファイル | 変更内容 |
|---------|---------|
| `src/engine/decision.rs` | effects_mut/push_effect の PassThrough→Consume バグ修正 |
| `src/engine/engine.rs` | compute_active, check_active_transition |
| `crates/awase-windows/src/win32.rs` | run_with_timeout, リークスレッド GC |
| `crates/awase-windows/src/runtime.rs` | SSOT 設計、ImmCapability 学習、EffectiveConfig |
| `crates/awase-windows/src/observer/ime_observer.rs` | observe() の検出失敗カウンタ、ガード |
| `crates/awase-windows/src/output.rs` | AppKind ベース Romaji 出力 |
| `crates/awase-windows/src/hook.rs` | shadow_action 追跡 |
| `crates/awase-windows/src/focus/classify.rs` | IMM ブラックリスト |
| `crates/awase-windows/src/executor.rs` | Relay モード処理 |
| `crates/awase-windows/src/tray.rs` | 学習キャッシュクリアメニュー |
| `crates/awase-windows/src/lib.rs` | Preconditions 拡張 |
| `crates/awase-windows/src/vk.rs` | ImeKeyKind 拡張（VK_KANA, VK_DBE_* 追跡） |
| `crates/awase-windows/src/main.rs` | shadow update, suspend/resume 処理 |
| `crates/awase-windows/src/ime.rs` | TSF sink 削除, set_ime_open タイムアウト保護 |
