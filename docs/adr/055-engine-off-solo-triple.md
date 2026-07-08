# ADR-055: 無変換3連打によるエンジン OFF 緊急回復

## ステータス

採用済み（2026-06-06 実装、2026-06-20 バグ修正）

## コンテキスト

awase のエンジン ON/OFF はデフォルトで Ctrl+Shift+F12 などの修飾キー付きホットキーで操作する。
しかし Ctrl が stuck した状態（VcXsrv 等で Ctrl KeyUp が欠落）では、エンジン自身がホットキーを
受け取っても「モディファイアなしのキー入力」として解釈してしまい、
ホットキーとして認識されない。

このような異常状態では Ctrl+Shift+F12 でエンジンを切れなくなるため、
ユーザーはエンジンが動き続けたまま通常の文字入力もできない状態に陥る。

## 決定

「修飾キー一切なし」で単独打鍵した親指キー（デフォルト: VK_NONCONVERT）を
400ms 以内に3回連打するとエンジンを強制 OFF にする緊急回復機能を追加した。

### ConsecutiveSoloCounter（`src/engine/consecutive_counter.rs`）

汎用ソロ連打カウンターを新設した。前回と異なる VK か、前回の記録から
`timeout_us` を超過した場合はカウントを 1 にリセットし、同一 VK かつ
タイムアウト以内のときはインクリメントする。

```rust
pub struct ConsecutiveSoloCounter {
    count: u32,
    last_vk: VkCode,
    last_us: Timestamp,
    timeout_us: u64,
}

pub fn record(&mut self, vk: VkCode, timestamp: Timestamp) -> u32 {
    let gap = timestamp.saturating_sub(self.last_us);
    if vk != self.last_vk || gap > self.timeout_us {
        self.count = 1;
    } else {
        self.count += 1;
    }
    self.last_vk = vk;
    self.last_us = timestamp;
    self.count
}
```

タイムアウトは `SOLO_TRIPLE_TIMEOUT_US = 400_000`（400ms）、
閾値は `SOLO_TRIPLE_COUNT = 3`。

### NicolaFsm でのチェック（`timeout_pending_thumb`）

親指キーのタイムアウト確定（ソロ打鍵確定）時のみカウントを記録する。
3回に達したら `engine_off_requested` フラグを立て、その打鍵は OS に渡さず suppress する。

```rust
fn timeout_pending_thumb(&mut self, scan_code, vk_code, timestamp) -> Resp {
    if self.engine_off_triple_vk.0 != 0 && vk_code == self.engine_off_triple_vk {
        let count = self.solo_counter.record(vk_code, timestamp);
        if count >= SOLO_TRIPLE_COUNT {
            self.solo_counter.reset();
            self.engine_off_requested = true;
            return self.build_response(SmallVec::new(), true, TimerIntent::CancelAll);
        }
    } else {
        self.solo_counter.reset();
    }
    // 通常のソロ確定処理へ…
}
```

### Engine での発動

`Engine::on_timeout` は FSM の処理後に `take_engine_off_requested()` を確認し、
フラグが立っていれば `apply_special_key_match(&SpecialKeyMatch::EngineOff, ctx)` を呼ぶ。

### 設定（`src/config.rs`）

```toml
[keys]
engine_off_solo_triple = "VK_NONCONVERT"  # 空文字列または省略で無効化
```

デフォルトは `"VK_NONCONVERT"`。`None` または空文字列で機能を無効にできる。

### バグ修正: consume_thumb でリセット（a53344b）

連続する同時打鍵（例: 「う」「り」などのシフト打鍵の連打）では親指キーが
`consume_thumb` で消費されるため、ソロ確定にはならない。しかし初期実装では
`solo_counter` がリセットされず、ソロ確定との混在で誤カウントが起きる
可能性があった。`consume_thumb` の先頭に `self.solo_counter.reset()` を追加して修正した。

## なぜこの設計か / 検討した代替案

**タイムアウト確定時のみカウント**: 同時打鍵として消費された親指キーは
ソロ確定パスを通らないため、自然にカウントされない。KeyDown 時点でカウントすると
同時打鍵との区別が難しい。

**400ms ウィンドウ**: NICOLA の同時打鍵判定閾値（通常 30〜80ms）よりはるかに長い。
通常の連続入力でも誤発動しない程度に広く、ユーザーが意図的に連打できる範囲に設定した。

**3回という閾値**: 誤操作防止のため2回では少なく、4回以上では緊急時に打ちにくい。
3回が適切なバランスと判断した。

**N 回目の suppress**: 3回目の無変換を OS に流すと IME 操作が走る可能性があるため、
suppress（`true`）で OS への送出をブロックする。

## 結果

- Ctrl スタック等でホットキーが効かない状態でも、無変換3連打でエンジンを確実に OFF にできる
- 修飾キーを一切必要としないため、stuck modifier の影響を受けない
- 設定で別のキーに変更可能、または無効化可能
- 通常の NICOLA 入力中に誤発動する可能性はほぼない（同時打鍵はカウントされず、400ms 制限あり）

## 関連 ADR

- ADR-015: シフト-リデュースパーサー（NicolaFsm の基本設計）
- ADR-008: 物理サム状態の分離（親指キーの consume_thumb 機構）
- ADR-024: 修飾キーパススルーと Watchdog（Ctrl stuck 問題の背景）

## 追補（2026-07-08）: 閾値を3→5に引き上げ + 発動時通知を追加

### 背景

スリープ復帰直後、GJI の conv mode が一発だけカタカナへ誤観測され
（BUG-19 追補2 のガードにより engine 側は意図的に actuate しない設計）、
かつ無変換連打の3回目（400ms 以内）でこの緊急回復機構が発動し、
`user_enabled` が false になった。ユーザーは「IME が直らない」と焦って
無変換キーをさらに連打していたため、本来 Ctrl スタック時の緊急脱出用に
用意したはずの機構が、無関係な別の不具合（カタカナ誤観測）からの復旧試行と
衝突して engine を止めてしまった。しかも一度 OFF になると、無変換/変換の
単発連打（`ime_on`/`ime_off`）では `user_enabled` は戻らず、明示的に
`Ctrl+Shift+変換`（`engine_on`）を押す必要があるため、「何を押しても直らない」
という体験になった。

### 変更

- `SOLO_TRIPLE_COUNT`(3) → `SOLO_OFF_TRIGGER_COUNT`(5) に引き上げ、誤発動しにくくした
  （`src/engine/nicola_fsm.rs`。`SOLO_TRIPLE_TIMEOUT_US` も `SOLO_OFF_TIMEOUT_US` に改称、
  値の 400ms は変更なし）。
- 発動時に `Engine` が 1 ショットフラグを公開し、awase-windows 側で
  トレイバルーン通知（「エンジンを緊急停止しました。戻すには Ctrl+Shift+変換」）を
  表示するようにした。従来はログにしか残らず、ユーザーは何が起きたか分からないまま
  無変換/変換を連打し続けていた。

### 上記「3回という閾値」セクションの現状

本文中「3回が適切なバランスと判断した」は 2026-06-06 時点の判断であり、
上記の実インシデントにより 5 回へ改定した。判断枠組み（2回は少ない／緊急時に
打ちにくくなり過ぎない範囲）自体は維持しつつ、実際に3回で誤発動した事例が
出たため上限側に寄せた。
