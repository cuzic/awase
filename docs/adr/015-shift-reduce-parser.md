# ADR-015: NicolaFsm のシフト-リデュースパーサーモデル

## ステータス

採用

## コンテキスト

NicolaFsm の同時打鍵判定は本質的に「トークン（キーイベント）を1つずつ受け取り、バッファに積むか（Shift）、パターンを認識して出力するか（Reduce）」というストリーミングパーサーだった。

しかし実装は再帰的な「resolve + handle_idle + combine_prev_and_new」パターンで、以下の問題があった:

- `update_history()` が `finalize_plan()` の外でも呼ばれる（履歴更新の経路が分散）
- `combine_prev_and_new()` で2つの Response を手動マージ（脆い）
- `self.phys` の暗黙依存（再帰の奥で何を参照しているか不明）

## 決定

### ParseAction enum（パーサーアクション）

```rust
enum ParseAction {
    Shift { timer: TimerIntent },
    Reduce { actions, record, timer },
    ReduceAndContinue { actions, record, remaining },
    PassThrough { timer: TimerIntent },
}
```

### decide_and_transition()（アクションテーブル）

(state, input) → ParseAction を返す純粋な判断メソッド。

### timed_fsm::parse() ループ（パーサーエンジン）

timed-fsm クレートに汎用の `ShiftReduceParser` trait + `parse()` 関数を追加。NicolaFsm はこの trait を実装し、ループは timed-fsm が駆動する。

```rust
fn on_key_down(&mut self, event) -> Resp {
    self.update_timing(event);
    let ev = self.phys.classified;
    if let Some(reason) = self.bypass_reason(&ev) {
        return self.handle_bypass(reason);
    }
    timed_fsm::parse(self, ev)  // フレームワークがループを駆動
}
```

### on_reduce() コールバック

`ShiftReduceParser::on_reduce()` が `update_history()` を呼ぶ唯一のポイント。

## 結果

- `combine_prev_and_new` 削除
- `update_history` がループ内1箇所に集約
- `StepResult` / `FinalizePlan` 削除（ParseAction に統合）
- パーサーフレームワークは timed-fsm の一部として再利用可能
- NicolaFsm の on_key_down は3行
