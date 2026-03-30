# ADR-004: AppState をオーケストレータとして集約、依存方向の逆転

## ステータス
採用

## コンテキスト
14個の `SingleThreadCell<T>` グローバル変数が散在し、`key_buffer.rs` や `focus/mod.rs` が `crate::ENGINE.get_mut()` 等で直接グローバルにアクセスしていた。依存方向が「モジュール → グローバル」で、テスト困難・変更影響が追いにくい状態。

## 決定
### Phase 1: グローバル集約
14個の `SingleThreadCell` を `AppState` 構造体に集約。`APP: SingleThreadCell<AppState>` の1つだけに。

### Phase 2: フリー関数のメソッド化
`invalidate_engine_context`, `refresh_ime_state_cache`, `toggle_engine`, `switch_layout` 等のフリー関数を `impl AppState` メソッドに変換。

### Phase 3: 依存方向の逆転
- `key_buffer.rs` → 純粋データ構造のみ（93行→削除、app_state.rs に統合）
- `focus/mod.rs` → サブモジュール宣言のみ（10行）。`win_event_proc` は main.rs に移動
- `focus/pattern.rs` → `KeyPatternTracker` のみ（純粋データ）
- 全オーケストレーションは `AppState` メソッド

### Phase 4: ファイル分離
`main.rs` (OS 統合) と `app_state.rs` (ロジック) に分離。

## 結果
- `crate::APP` アクセス: key_buffer.rs=0, focus/=1 (win_event_proc のみ)
- AppState メソッドは「純粋遷移」と「副作用実行」に分類
- `AppAction` enum で副作用を宣言的に返す

## 関連コミット
`32f5738`, `c8a98d4`, `47294ea`, `883d717`, `75a4d16`, `d7aa95e`
