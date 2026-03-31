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
