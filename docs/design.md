# awase アーキテクチャ設計書

> 対象読者: awase に興味を持った開発者・コントリビューター。「なぜこの設計か」の背景を説明する。

---

## 1. プロジェクト概要

awase（合わせ）は、Windows 向けの **親指シフト（NICOLA）キーボードエミュレータ** です。
`WH_KEYBOARD_LL` グローバルキーフックで物理キーを横取りし、同時打鍵判定・変換・IME 制御を行った上で `SendInput` でキーを再注入します。

### 解決している問題

| 課題 | awase の解法 |
|---|---|
| 同時打鍵の不確実性 | timed-FSM による「イベント駆動優先 + タイマー安全網」ハイブリッド判定 |
| IME の誤状態 | per-window IME state cache + shadow model による intent/observation 分離 |
| TSF ベース IME（Google 日本語入力等）の cold start 問題 | TSF probe FSM + deferred キュー |
| LINE / Qt アプリの IME 誤連動 | ImmCross モードで物理 IME キーを完全 Consume |
| macOS / Linux への将来対応 | lib クレートをプラットフォーム非依存に設計（ADR-019） |

---

## 2. クレート構成

```
awase（ワークスペースルート）
├── src/                      ← lib クレート（OS 非依存）
│   ├── engine/               ← NicolaFsm・同時打鍵判定・配列変換
│   ├── config/               ← TOML 設定読み込み・検証
│   ├── yab/                  ← やまぶき互換 .yab 配列定義パーサ
│   ├── ngram/                ← n-gram モデル（入力補助）
│   ├── tsf/                  ← TSF 抽象（プラットフォーム非依存側）
│   └── types.rs              ← 共通型（RawKeyEvent, KeyClassification, ...）
│
├── crates/awase-windows/     ← Windows 実装
│   ├── src/app/              ← エントリポイント・メッセージループ
│   ├── src/runtime/          ← オーケストレーション（engine ↔ executor 配線）
│   │   ├── executor.rs       ← DecisionExecutor（IME 実行・キー出力決定）
│   │   └── key_pipeline.rs   ← フックイベント処理パイプライン
│   ├── src/output/           ← SendInput ラッパ・TSF probe FSM
│   ├── src/focus/            ← フォーカス検知・ウィンドウ分類
│   ├── src/state/            ← IME shadow model・reducer
│   ├── src/observer/         ← IME / TSF 状態観測
│   ├── src/tsf/              ← TSF 操作（probe, observation）
│   ├── src/imm.rs            ← IMM32 低レベルラッパ
│   └── src/platform.rs       ← WindowsPlatform（高レベル API 集約）
│
├── crates/timed-fsm/         ← タイムアウト付き FSM ライブラリ（crates.io 公開予定）
├── crates/win32-async/       ← シングルスレッド spawn_local
├── crates/win32-worker/      ← バックグラウンドスレッド
├── crates/awase-settings/    ← 設定 GUI（iced）
└── crates/awase-yab-editor/  ← .yab 配列エディタ（独立 GUI）
```

**lib クレート（`src/`）は Windows API を一切参照しない。** macOS / Linux 対応のための設計上の制約です（ADR-019）。

---

## 3. シングルスレッド・イベント駆動

awase は **Win32 メッセージループの単一スレッド** で動作します。
tokio / async-std 等のマルチスレッドランタイムは使いません。

```
┌─── GetMessageW ループ ───────────────────────────────────────┐
│                                                              │
│  WH_KEYBOARD_LL コールバック ──► Runtime::process_key_event  │
│  WM_TIMER ─────────────────────► Runtime::on_timer           │
│  WM_HOTKEY ────────────────────► トグル処理                   │
│  WM_IME_NOTIFY / WinEvent ─────► Observer → ImeEvent dispatch│
│  WM_QUIT ──────────────────────► ループ終了                   │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

**なぜシングルスレッドか:**

- `WH_KEYBOARD_LL` コールバックは `GetMessageW` を呼んでいるスレッドで実行される。同じスレッドなら Mutex / Arc が不要
- コールバックには約 300ms のタイムアウト制約がある。コールバック内は「受付 + 握りつぶし判定」のみ行い、重い処理はメッセージループ側に委ねる
- `SetTimer` は OS カーネルタイマーであり、CPU/メモリ消費がほぼゼロ

---

## 4. キーイベント処理パイプライン

```
物理キー押下
    │
    ▼
WH_KEYBOARD_LL コールバック（hook.rs）
    │
    ├─ dwExtraInfo == INJECTED_MARKER? → 素通し（無限ループ防止）
    ├─ 事前分類（classify_key / classify_modifier / classify_ime）
    │       ↓
    │   RawKeyEvent { key_classification, physical_pos, ime_relevance, ... }
    │
    ▼
Runtime::process_key_event
    │
    ├─ Ctrl+無変換 IME-OFF 救済窓チェック
    ├─ Engine::on_key_event(event)
    │       ↓ EngineDecision
    │       ├─ Emit(actions)   → DecisionExecutor::execute()
    │       ├─ Pending         → SetTimer 起動
    │       └─ PassThrough     → CallNextHookEx
    │
    └─ DecisionExecutor::execute()
            ├─ output::send_romaji()   ← SendInput でローマ字送出
            ├─ output::send_vk()       ← SendInput で仮想キー送出
            └─ ime_apply()             ← IME 開閉を WindowsPlatform 経由で実行
```

### 無限ループ防止

`SendInput` で注入したキーは `dwExtraInfo == INJECTED_MARKER`（`0x4B45_594D` = "KEYM"）を付与し、フック冒頭で素通しします。

---

## 5. 同時打鍵判定（timed-FSM）

NicolaFsm は `timed-fsm` クレートの `TimedStateMachine` を実装します。「イベント駆動優先 + SetTimer 安全網」のハイブリッド方式です。

```
【パターン1: 文字先行 → 時間内に親指】
  文字キー ████████
  親指キー     ████████
  → 親指 Down 到着時にイベント駆動で同時打鍵確定（KillTimer）

【パターン2: 親指先行 → 文字キー到着】
  親指キー ████████
  文字キー     ████
  → 親指押下中に文字キー到着 → 即時確定（タイマー不要）

【パターン3: 文字単独 → タイムアウト】
  文字キー ████████
  → WM_TIMER 到着で単独打鍵として確定

【パターン4: 親指単独 → タイムアウト】
  親指キー ████████
  → WM_TIMER 到着でスペース/変換/無変換として確定
```

判定閾値（デフォルト 100ms）は `config.toml` で調整可能です。

---

## 6. 事前分類アーキテクチャ（ADR-019）

Engine は VkCode の数値（`0x51` 等）を一切検査しません。
プラットフォーム層がイベント構築時に分類を完了させ、Engine は分類結果だけを見ます。

```
OS キーイベント（VkCode 数値）
    ↓ awase-windows/src/focus/classify.rs
    ↓  classify_key()      → KeyClassification（CharKey / ThumbLeft / ThumbRight / ...）
    ↓  classify_modifier() → ModifierKey（Ctrl / Shift / Alt / ...）
    ↓  classify_ime()      → ImeRelevance（SyncOn / SyncOff / Toggle / Unrelated）
    ↓
RawKeyEvent（分類済み）
    ↓
Engine（VkCode の値を参照しない）
```

**効果:** `src/` 配下の Engine は Windows 固有の VkCode 定数を知らなくてよい。macOS / Linux ではそれぞれのプラットフォーム層が分類して渡せばいい。

---

## 7. IME 状態管理（ADR-032）

IME の状態管理は「4 つの責務」に分離されています。

```
┌─────────────────────────────────────────────────────────┐
│  Intent（ユーザーの意図）                                 │
│  ImeModel::desired_open                                   │
│  書き換えは UserImeSetIntent / UserImeToggleIntent のみ   │
└─────────────────────────────────────────────────────────┘
            ↓ reduce()（PlatformState::reduce_with_envelope）
┌─────────────────────────────────────────────────────────┐
│  Shadow Model（awase が把握している IME 状態）             │
│  applied_open / pending / effective_open                  │
│  全更新は ImeEvent → reducer 経由のみ（直接代入禁止）     │
└─────────────────────────────────────────────────────────┘
            ↓ on_ime_apply_complete（generation 照合必須）
┌─────────────────────────────────────────────────────────┐
│  Apply 実行（DecisionExecutor / WindowsPlatform）         │
│  VK_KANJI / VK_IME_ON/OFF / IMM32 / TSF probe で適用     │
└─────────────────────────────────────────────────────────┘
            ↑ ObserverReported（Observer は desired_open を書かない）
┌─────────────────────────────────────────────────────────┐
│  Observation（OS の実際の状態観測）                        │
│  IMM32 / TSF / WinEvent / フォーカス変化でイベントを dispatch│
└─────────────────────────────────────────────────────────┘
```

**なぜこの分離か:**
旧設計では「ユーザーの意図」と「OS の観測値」が同じフィールドに 5 優先度で書き込まれ、Observer が意図を上書きするバグが多発しました（ADR-032）。reducer パターンに移行することで、intent と observation の責務を構造的に分離しています。

---

## 8. フォーカス検知と per-window IME キャッシュ

アプリごとに最適な IME 制御方式が異なるため、フォーカス変化を検知して分類します。

```
WinEvent（EVENT_SYSTEM_FOREGROUND 等）
    ↓
FocusTracker::on_focus_change()
    ├─ MSAA / UIA で UI フレームワーク検出
    ├─ ImmCapability（IMM32 対応 / TSF 専用 / ImmCross）
    └─ HwndImeCache（ウィンドウごとの IME 状態キャッシュ）
```

### アプリ種別と制御方式

| 分類 | 例 | IME 制御方式 |
|---|---|---|
| `ImmCapable` | メモ帳, VSCode | IMM32（ImmGetContext 経由） |
| `TsfOnly` | WezTerm, Windows Terminal | TSF probe + VK_DBE_HIRAGANA |
| `ImmCross` | LINE, Qt アプリ | 別プロセス ImmSetOpenStatus + KANJI 物理キー Consume |
| `Unknown` | ゲーム等 | パススルー |

---

## 9. TSF Cold Start 問題と Probe FSM（ADR-030）

Google 日本語入力（GJI）等 TSF ベース IME は、起動直後（cold start）に VK_DBE_HIRAGANA を送っても無視されます。

**解法:** TSF probe FSM で「IME が受け付け可能か」を事前検出し、受け付け可能になるまでキー送信を deferred キューに退避します。

```
VK_DBE_HIRAGANA 送信要求
    │
    ▼
probe_fsm: Probing 状態
    │ （GJI の I/O カウンタが変化するまで待機）
    ├─ 変化あり → Warm と判断 → deferred キューを一括送信
    └─ タイムアウト → Cold → fallback（IMM32 or VK_KANJI）
```

---

## 10. レイヤー境界ルール

コードベースには `docs/layer-boundaries.md` にまとめたレイヤー境界ルールがあります。主要なものを抜粋します。

| ルール | 内容 |
|---|---|
| A-1 | `src/`（lib）は OS API 非依存 |
| A-2 | Engine は `KeyClassification` / `ImeRelevance` / `PhysicalPos` のみ参照 |
| B-1 | `with_app()` は `app/` / `runtime/` / `executor.rs` のみ呼び出し可 |
| C-1 | `desired_open` への代入は `UserImeSetIntent` / `UserImeToggleIntent` アームのみ |
| C-2 | Observer は `ImeEvent::ObserverReported` 経由でのみ shadow model に報告 |
| C-3 | Apply 完了 event は generation 照合必須 |
| D-2 | ImmCross アプリには物理 IME キー（KANJI 等）を passthrough しない |

詳細は [layer-boundaries.md](layer-boundaries.md) を参照してください。

---

## 11. 設定ファイル

`config.toml` で動作を細かく制御できます。

```toml
[general]
simultaneous_threshold_ms = 100   # 同時打鍵判定閾値（ms）
left_thumb_key = "VK_MUHENKAN"    # 左親指キー
right_thumb_key = "VK_CONVERT"    # 右親指キー

[layout]
name = "my-layout"
file = "layout/nicola-jis.yab"    # やまぶき互換 .yab ファイル
```

`.yab` ファイルは「やまぶき」互換のタブ区切り CSV で、物理キー位置（行・列）ごとに通常面・左親指面・右親指面の出力ローマ字を定義します。

---

## 12. 技術的な制約・既知のトレードオフ

| 項目 | 内容 |
|---|---|
| フックタイムアウト | コールバックは約 300ms 以内に戻る必要がある。`SendInput` はコールバック内ではなくメッセージループ側で発行 |
| SetTimer 精度 | Windows のタイマー解像度（約 10〜16ms）の誤差がある。100ms 閾値では許容範囲 |
| 管理者権限アプリ | UAC が有効な環境では、非管理者フックは管理者権限アプリに効かない場合がある |
| DirectInput / Raw Input | ゲームでこれらを使用している場合、`WH_KEYBOARD_LL` フックが効かないことがある |
| セキュリティソフト | グローバルキーフックを不審な動作として検出するアンチウイルスがある |

---

## 13. ADR 一覧

主要な設計判断の記録は `docs/adr/` に格納しています。

| ADR | タイトル |
|---|---|
| 001 | IME 検出戦略 |
| 002 | TSF cold start warmup |
| 003 | Chrome VK 注入 |
| 005 | フォーカス分類 |
| 009 | データ保持型 Engine 状態 |
| 012 | VkCode / ScanCode newtype |
| 014 | Observer / Executor / Runtime 分離 |
| 019 | lib クレートのプラットフォーム非依存化 |
| 030 | TSF 状態管理の3層分離アーキテクチャ |
| 032 | IME 状態モデルの4階層 reducer アーキテクチャ |
