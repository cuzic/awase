# ADR-019: lib クレートのプラットフォーム非依存化

## ステータス

採用

## コンテキスト

awase の lib クレート（Engine, NicolaFsm, config 等）に Windows 固有のコードが散在していた。VK コード定数 (0x08, 0x10 等)、Windows スキャンコードマッピング、VK 名前テーブル、SysKeyDown/SysKeyUp イベント型、Win32 API 参照等が Engine 内部に直接埋め込まれており、macOS/Linux でのビルド・動作が不可能だった。

## 決定

### 事前分類アーキテクチャ

プラットフォーム層が `RawKeyEvent` を構築する時点で全ての分類を完了させ、Engine はプラットフォーム固有の値を一切検査しない設計にする。

```
OS キーイベント
  ↓ プラットフォーム層（awase-windows / awase-macos / awase-linux）
  ↓ classify_key()      → KeyClassification, PhysicalPos
  ↓ classify_modifier() → ModifierKey
  ↓ classify_ime()      → ImeRelevance
  ↓
RawKeyEvent（全フィールド分類済み）
  ↓
Engine（VkCode/ScanCode の値を一切検査しない）
```

### 新しい型

- `KeyClassification`: Char / LeftThumb / RightThumb / Passthrough
- `SpecialKey`: Backspace / Enter / Escape / Space / Delete
- `ModifierKey`: Ctrl / Shift / Alt / Meta
- `ShadowImeAction`: TurnOn / TurnOff / Toggle
- `ImeRelevance`: may_change_ime, shadow_action, is_sync_key, sync_direction, is_ime_control
- `KeyboardModel`: Jis / Us（.yab パーサーの行サイズ決定用）

### VkCode / ScanCode の扱い

VkCode と ScanCode は型としては lib に残すが、Engine はオペーク値として保持するのみ（再注入・ログ用）。値の検査（0x08 == Backspace 等）はプラットフォーム層の責務。

### 移動したコード

| 移動元 (lib) | 移動先 (awase-windows) |
|---|---|
| src/vk.rs | crates/awase-windows/src/vk.rs |
| src/gui/main.rs | crates/awase-windows/src/gui/main.rs |
| scanmap.rs の scan_to_pos/pos_to_scan | crates/awase-windows/src/scanmap.rs |
| config.rs の vk_name_to_code/parse_hotkey/parse_key_combo | crates/awase-windows/src/vk.rs |

### SysKeyDown/SysKeyUp の廃止

Windows 固有の SysKeyDown/SysKeyUp をフック層で KeyDown/KeyUp に統合。lib の KeyEventType は KeyDown と KeyUp の2値のみ。

## 結果

- lib クレートのプロダクションコードに Windows 依存ゼロ
- Engine コア（engine.rs, nicola_fsm.rs, ime_coordinator.rs 等）から crate::vk, scan_to_pos の参照ゼロ
- macOS/Linux クレートが同じ Engine をそのまま使用可能
- config.toml のデフォルトキー名をプラットフォーム非依存化（Nonconvert, Convert, Kanji 等）
- 後方互換: awase-windows の vk_name_to_code が VK_ プレフィックス付き名前も受け付ける

## 追記（2026-07、P5: lib からの Windows 概念退去 第2弾）

ADR-019 で概ね達成した lib のプラットフォーム非依存化を、概念レベルで残っていた
Windows 固有要素についてさらに進めた。以下を lib（`src/`）から awase-windows へ移設した。

| 移動元 (lib) | 移動先 (awase-windows) | 備考 |
|---|---|---|
| src/tsf.rs（`TsfGate` 等） | crates/awase-windows/src/tsf/tsf_gate.rs | 語彙が全て TSF 固有。lib→awase-windows のレイヤ逆転 doc 参照も解消 |
| src/types.rs の `AppKind` / `FocusKind`（atomic ヘルパ・テスト含む） | crates/awase-windows/src/focus/kinds.rs | `AppKind` は Windows 固有（Win32/TSF/UWP）。`FocusKind` は中立概念だが事前分類の境界複製として awase-windows 側に配置 |
| tests/e2e_windows.rs | crates/awase-windows/tests/e2e_windows.rs | ルート Cargo.toml の windows dev-dependency も撤去（awase-windows は windows を通常依存で保持済み） |

加えて `PlatformRuntime` トレイトから TSF composition 特有のフック7メソッド
（`composition_output` / `output_in_flight_ms` / `is_composition_warm` / `is_tsf_mode` /
`on_ime_applied` / `on_passthrough_key` / `on_reinject_key`）を新トレイト `TsfComposition`
（`src/platform.rs`）へ分離した。macOS/Linux 実装者はコアの `PlatformRuntime`
（send_keys / reinject / timer / set_ime_open / tray / send_engine_state_ime_key）だけを
実装すればよく、`TsfComposition` は全メソッドが default 実装を持つため composition 機構が
不要なら実装を省略できる。`send_engine_state_ime_key` は IME モードキー制御であり
（TSF composition ではなく）`&mut dyn PlatformRuntime` 経由で呼ばれるため、コア側に残した。
supertrait 化は避けた（default 持ちの TSF メソッドを空実装で強制することになり、
非 Windows のエルゴノミクスを却って損なうため）。
