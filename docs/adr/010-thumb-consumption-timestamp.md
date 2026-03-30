# ADR-010: Option<Timestamp> による親指キー消費追跡

## ステータス

採用

## コンテキスト

NICOLA 同時打鍵で親指キー（変換/無変換）を1文字とペアリングした後、同じ物理押下が後続のキーにも適用され、2文字連続で親指シフトされるバグがあった。

最初の修正案は `bool` フラグ（`left_thumb_consumed: bool`）で、`on_event()` 内で物理状態の変化を検出して手動リセットしていた。しかしリセット漏れのリスクがあった。

## 決定

`Option<Timestamp>` を使用する。消費した親指押下のタイムスタンプを記録し、`phys.left_thumb_down` と一致すれば消費済み、不一致なら未消費と判定する。

```rust
left_thumb_consumed: Option<Timestamp>,  // 消費した押下のタイムスタンプ
right_thumb_consumed: Option<Timestamp>,
```

判定:
```rust
let consumed = self.phys.left_thumb_down.is_some()
    && self.left_thumb_consumed == self.phys.left_thumb_down;
```

## 結果

- 新しい KeyDown → タイムスタンプが変わる → 自動不一致 → 未消費に戻る
- KeyUp → `None` になる → 自動不一致
- 明示的なリセット処理が**構造的に不要**
- `on_event()` のリセットコードを削除（リセット漏れのバグクラスを排除）
