# ADR-008: 物理親指キー状態と FSM 解決ロジックの分離

## ステータス
採用

## コンテキスト
`left_thumb_down` / `right_thumb_down` が2つの異なる目的で使われていた:

1. **物理キー状態追跡** — 連続シフト用（親指を押しながら複数文字を打つ）
2. **FSM 同時打鍵確定の副作用** — `resolve_char_thumb_as_simultaneous` 等の4箇所でセット

`on_key_up` で先頭の親指状態クリアが、後続の `resolve_char_thumb_as_simultaneous` で再セットされ、物理的にはキーが離されているのに `thumb_down = Some` のままになる。次の文字キーに親指シフトが「漏れる」バグ。

## 決定
`left_thumb_down` / `right_thumb_down` を **純粋に物理キー状態の追跡** に限定:

- **セット**: `on_key_down` の先頭（後に `InputTracker.process()` に移動）
- **クリア**: `on_key_up` の先頭（同上）
- **FSM 解決関数**: `thumb_down` に一切触らない（4箇所の代入を全削除）

FSM は `pending_thumb` で同時打鍵判定を管理し、連続シフトは物理キーが押されているかどうかだけで判断する。

## 結果
- 親指シフトの「漏れ」問題が構造的に解消
- `thumb_down` のライフサイクルが明確（KeyDown でセット、KeyUp でクリア、それだけ）
- 後に InputTracker に移動され、Engine から完全に分離

## 関連コミット
`af087fa`
