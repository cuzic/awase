# ADR 028: フォーカスイベント処理の再設計

## ステータス

承認済み（未実装）

## 背景

現在のフォーカスイベント処理は `EVENT_OBJECT_FOCUS` を全て受け取り、事後フィルタリングで不要なイベントを除外する設計。例外が増え続けている:

- builtin bypass リスト（ForegroundStaging, XamlExplorerHostIslandWindow 等 10 種類以上）
- NullHwnd 無視（hwnd=0x0）
- 同一プロセス降格防止（check_same_process_skip）
- config オーバーライド

これらは「本当のフォーカス移動」以外のイベントに反応してしまうことへの個別パッチ。

## 問題

1. **中間ウィンドウでの誤 flush**: IME 候補ウィンドウ（hwnd=0x0）や Qt の内部ウィンドウ（Qt673QWindow → class="" → NullHwnd）がフォーカスを一瞬奪い、NICOLA FSM の pending キーが flush される。結果として「されていますか」が「されすていまか」のようにキー順が乱れる。
2. **builtin bypass リストの肥大化**: 新しい Windows バージョンや UI フレームワークが増えるたびに追加が必要。スケールしない。
3. **同一プロセス内フォーカス移動への過剰反応**: アプリ内部のパネル切替等で不要な IME refresh が走る。
4. **短時間に大量のフォーカスイベント**: ウィンドウ切替時に 5-10 イベントが連続し、それぞれが Engine に通知される。

## フォーカスイベントの目的

awase がフォーカス変更に反応する目的は 2 つだけ:

1. **Engine の有効/無効を切り替える**: TextInput → Engine 有効、NonText → Engine 無効
2. **IME 状態を再取得する**: ウィンドウごとに IME ON/OFF が異なるため

## 要件

### R1: 「ユーザーの入力先が変わった」ときだけ反応する

- ユーザーが意図的にアプリを切り替えた場合のみ
- システムの中間ウィンドウ、候補ウィンドウ、ポップアップは無視
- 同一アプリ内のフォーカス移動は基本的に再分類の対象（ただし即座には反応しない）

### R2: pending キーの flush は最小限にする

- フォーカス変更で flush するのは「ユーザーが本当にアプリを離れた」場合のみ
- 一瞬の中間状態では flush しない
- **flush はデバウンス後にのみ行う**（現在の即座 flush が誤 flush の直接原因）

### R3: IME 状態の再取得はデバウンス後に行う

- フォーカスチェーン（複数イベント連続）が落ち着いてから
- 中間ウィンドウの IME 状態を拾わない
- 現在の 50ms デバウンスの考え方は正しい

### R4: 分類（TextInput/NonText）は最終的な宛先で判定する

- 中間ウィンドウで判定しない
- 判定不能（Undetermined）なら前の状態を維持する

### R5: bypass リストに依存しない設計

- 個別のクラス名を列挙する方式はスケールしない
- 汎用的な基準で中間ウィンドウを除外できるべき

## 決定: デバウンス後のみ処理する設計

### 現在の処理フロー（問題あり）

```
EVENT_OBJECT_FOCUS 到着
  ↓
classify_focus(hwnd) → TextInput/NonText/Undetermined
  ↓
Engine.handle_focus_changed() → 即座に flush ← ★ 問題
  ↓
デバウンスタイマー設定（50ms）
  ↓
[50ms 後] IME 状態再取得
```

### 新しい処理フロー

```
EVENT_OBJECT_FOCUS 到着
  ↓
hwnd == 0x0 → 無視（return）
  ↓
デバウンスタイマーをリセット（50ms）
  ↓
[50ms 後]
  ↓
最終 hwnd を GetGUIThreadInfo で取得
  ↓
classify_focus(hwnd) で TextInput/NonText を判定
  ↓
前の focus_kind と比較
  ↓
変化あり → Engine に FocusChanged 通知（flush + IME refresh）
変化なし → IME refresh のみ（flush しない）
```

### 核心的な変更点

1. **`win_event_proc` はデバウンスタイマーの（再）設定のみ行う**。Engine への即座の通知を削除。
2. **分類・flush・IME refresh は全てデバウンス後に行う**。
3. **Engine の `handle_focus_changed` は変更不要**。呼ばれるタイミングがデバウンス後のみになるため、flush の即座実行は正しい（「確定した」フォーカス変更のみ来る）。

### デバウンス中のキー入力

デバウンス中（50ms）にキーが入力された場合、旧コンテキスト（前のウィンドウ）で処理される。実害なし: ユーザーがアプリを切り替えている最中に 50ms 以内に入力を開始することは実質的にない。

### builtin bypass リストの扱い

デバウンス後に最終 hwnd で判定する設計にすれば、中間ウィンドウ（ForegroundStaging 等）は最終状態に残らないため、bypass リストの多くが不要になる可能性がある。ただし、タスクバー等の「最終的にフォーカスが留まる非テキストウィンドウ」への対応は引き続き必要。

## 変更対象ファイル

| ファイル | 変更内容 |
|---------|---------|
| `crates/awase-windows/src/main.rs` | `win_event_proc`: デバウンスタイマーのみ設定、即座の Engine 通知を削除 |
| `crates/awase-windows/src/main.rs` | WM_TIMER ハンドラ: デバウンス完了時に classify + Engine 通知 |
| `crates/awase-windows/src/observer/focus_observer.rs` | `observe()`: デバウンス後に呼ばれる前提で簡略化 |
| `src/engine/engine.rs` | `handle_focus_changed`: 変更不要（呼び出し元が変わるだけ） |

## 検証項目

- wezterm で入力中に IME 候補ウィンドウが出てもキー順序が乱れないこと
- アプリ切替（Alt+Tab）後に IME 状態が正しく更新されること
- 同一アプリ内でテキスト欄→ツールバークリック後、Engine が NonText になること
- builtin bypass リスト内のウィンドウが一瞬フォーカスを得ても flush されないこと
