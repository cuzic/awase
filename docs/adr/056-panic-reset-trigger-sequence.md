# ADR-056: パニックリセットトリガー: 同一キー連打 → OFF→ON→OFF シーケンス

## ステータス

採用済み（2026-06-06 実装、commit b812345）

## コンテキスト

### パニックリセット機能の目的

awase は IME 状態が内部的に不整合を起こした場合に備え、ユーザが意図的に
全状態をリセットできる「パニックリセット」機能を持つ。

### 初期設計の問題

初期実装（commit 59d40ce / c909624）では、「IME 関連ショートカット（無変換・変換等）を
1000ms 以内に3回連打」することをトリガーとしていた。

```rust
// 旧実装: タイムスタンプのみ保持
buf: [u64; 3],
const WINDOW_MS: u64 = 1000;

pub fn push(&mut self, now_ms: u64) -> bool {
    // 最古エントリとの差分だけ見ていた
    let oldest = *self.buf.iter().min().unwrap_or(&0);
    now_ms.saturating_sub(oldest) < Self::WINDOW_MS
}
```

しかしこの設計には重大な誤発動問題があった。`Ctrl+無変換` を IME-OFF ショートカットとして
設定しているユーザが、LINE アプリ等で素早くメッセージを送信しようとして
`Ctrl+無変換` を連打すると、意図せずパニックリセットが発動し IME が ON 状態に
戻ってしまうバグが発生した。

## 決定

### トリガー条件を「OFF→ON→OFF の交互シーケンス」に変更（commit b812345）

`RapidPressTracker` のバッファを `[u64; 3]`（タイムスタンプのみ）から
`[(bool, u64); 3]`（`is_on` フラグ + タイムスタンプ）に変更した。

```rust
pub struct RapidPressTracker {
    /// 直近3エントリ: (is_on, timestamp_ms)
    buf: [(bool, u64); 3],
    cursor: usize,
    count: usize,
}

impl RapidPressTracker {
    const WINDOW_MS: u64 = 2000;  // 1000ms → 2000ms に拡大

    pub fn push(&mut self, is_on: bool, now_ms: u64) -> bool {
        self.buf[self.cursor] = (is_on, now_ms);
        self.cursor = (self.cursor + 1) % Self::THRESHOLD;
        // ...
        let oldest = self.buf[self.cursor];
        let middle = self.buf[(self.cursor + 1) % Self::THRESHOLD];
        let newest = self.buf[(self.cursor + 2) % Self::THRESHOLD];
        // OFF → ON → OFF のシーケンスかつ全体が WINDOW_MS 以内
        let is_off_on_off = !oldest.0 && middle.0 && !newest.0;
        let within_window = now_ms.saturating_sub(oldest.1) < Self::WINDOW_MS;
        is_off_on_off && within_window
    }
}
```

`PanicTriggerCombo` に `is_on: bool` フィールドを追加し、キー押下時に
`get_panic_trigger_direction()` が `Option<bool>` を返すよう変更した。
呼び出し側は `is_on` 値を `record_ime_keydown(is_on, now_ms)` に渡す。

### 時間窓の拡大

誤発動防止のため条件を厳しくした分、時間窓を 1000ms から 2000ms に拡大した。
「意図的な交互押し」は 2000ms 以内に完了できる一方、
「意図せず同じキーを3連打」が OFF→ON→OFF になることは実質ありえない。

## なぜこの設計か / 検討した代替案

### 代替案: 連打回数を増やす（例: 5連打）

単純に回数を増やすだけでは問題の本質を解決しない。
同一キーの連打であれば何回でも誤発動の可能性が残る。

### 採用した設計の優位性

「OFF→ON→OFF」という交互シーケンスは自然なタイピングでは発生しない。
意図的に「IME を OFF にし、ON にし、また OFF にする」という操作を
2000ms 以内に行う必要があるため、誤発動が構造的に不可能になる。

同一キー（例: 無変換のみ）を何度連打してもシーケンス条件を満たせない
（`is_off_on_off = !oldest.0 && middle.0 && !newest.0` が false になる）。

## 結果

- `Ctrl+無変換` 連打による誤パニックリセットが発動しなくなった
- 意図的な IME-OFF → IME-ON → IME-OFF の操作でリセットを発動できる
- トレイメニューからの直接発動（ADR-052）と組み合わせることで、
  キーボードが使えない状況でもリセット手段が確保されている

## 関連 ADR

- ADR-052: トレイメニューからのパニックリセット（別経路のリセット発動）
- ADR-032: レイヤー境界監査（`panic_detect.rs` の awase-windows 内配置の根拠）
