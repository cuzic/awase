# ADR-012: VkCode / ScanCode newtype の全面適用

## ステータス

採用

## コンテキスト

仮想キーコード（VK）とスキャンコードは `u16` / `u32` として扱われていた。関数シグネチャからは引数が VK コードなのか、スキャンコードなのか、タイマーIDなのか区別できなかった。

`KeyAction::Key(u16)` は VK コードを保持するが、型上は任意の u16 を受け付ける。`ImeSyncKeys` の `Vec<u16>` も同様。

## 決定

既存の `VkCode(pub u16)` / `ScanCode(pub u32)` newtype を、codebase 全体で一貫して使用する。

主な変更:
- `KeyAction::Key(u16)` → `Key(VkCode)`, `KeyUp(u16)` → `KeyUp(VkCode)`
- `ParsedKeyCombo::vk: u16` → `vk: VkCode`
- `vk_name_to_code()` → `Option<VkCode>`
- `ImeSyncKeys` の `Vec<u16>` → `Vec<VkCode>`
- `scan_to_pos(u32)` → `scan_to_pos(ScanCode)`
- `pos_to_scan()` → `Option<ScanCode>`

Win32 API 境界でのみ `.0` で内部値を取り出す。

## 結果

- VK コードとスキャンコードの取り違えがコンパイル時に検出される
- 関数シグネチャが自己文書化（`vk: VkCode` vs `timer_id: usize`）
- `_typed` ラッパー関数が不要に（本体が直接 newtype を受け取る）
- Win32 API 呼び出し直前の `.0` 抽出が「型の境界」を明示する
