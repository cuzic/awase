# ADR-072: conv_mode_authority を apply 完了ごとに再同期する

## ステータス

採用済み（2026-07-01 実装、commit e2199e7）

## コンテキスト

### conv_mode_authority とは

`ConvModeAuthority` は TSF warmup（`TsfWarmupCoordinator`）が送信を行う前に
「変換モードを awase が所有しているか（`AwaseOwned`）、ユーザーが所有しているか（`UserOwned`）」
を判定するために使う値。

`AwaseOwned` でない場合、TSF warmup は `non-AwaseOwned` として送信をスキップする。

### 問題: EngineStateChanged の発火漏れ

`conv_mode_authority` は `UiEffect::EngineStateChanged`（Engine の活性状態の遷移エッジ）
でのみ更新される設計だった。

しかし次のシナリオで更新が漏れる：

1. パニックリセット（無変換→変換→無変換の高速連打で発火する緊急復帰機構）の直後
2. Engine がすでに `Active` な状態で、さらに IME-ON だけをやり直す経路
   （2 回目の `Ctrl+変換` など）

このとき Engine の `Active` 状態は変化しないため `EngineStateChanged` が発火せず、
`conv_mode_authority` が古い値（`UserOwned`）のまま残る。

### バグの症状

```
apply-ime → Confirmed を返す
         → TSF warmup: "non-AwaseOwned → スキップ"
         → OS 側: IME OFF 表示のまま
         → Engine: ON（desync）
```

ログ上の「なぜか突然 IME OFF Engine ON になり、Ctrl+変換 を押しても直らない」という
症状がこのパスで発生していた。

### 根本原因の構造

`EngineStateChanged` は「状態が変化した」という遷移エッジ (edge) であり、
「IME が apply された」という事実とは直交する。
apply が成功/失敗するたびに authority が更新されるべきなのに、
遷移エッジの発火に依存することで経路依存のバグが生まれていた。

## 決定

`record_ime_apply_result`（sync / async 両経路が通る唯一の apply 完了地点）で、
apply の effective 結果に応じて `conv_mode_authority` を直接補正する。

```rust
pub(crate) fn record_ime_apply_result(
    &mut self,
    effective: bool,
    outcome: ImeOpenOutcome,
) {
    // ... existing logic ...

    // conv_mode_authority を apply 結果と再同期する。
    // EngineStateChanged (遷移エッジ) への依存を撤廃し、
    // apply が完了するたびに確定した effective から直接補正する。
    let corrected = if effective {
        ConvModeAuthority::AwaseOwned
    } else {
        ConvModeAuthority::UserOwned
    };
    if self.model().conv_mode_authority() != corrected {
        self.model_mut().set_conv_mode_authority(corrected);
    }
}
```

### なぜ `record_ime_apply_result` か

- sync apply（`apply_ime_open`）と async apply（generation 照合後の完了コールバック）の
  両方が必ずこの関数を通る唯一の apply 完了地点
- ここで補正することで「どの経路で apply が行われたか」に依存しない

### effective の意味

- `effective=true`: IME ON に確定（`AwaseOwned`）
- `effective=false`: IME OFF に確定（`UserOwned`：awase がひらがなモードを所有していない）

## 検討した代替案

### EngineStateChanged の発火条件を緩和して「Active 維持」でも発火させる

→ 採用しなかった。`EngineStateChanged` の意味を変えると他の受信者が影響を受ける。
  apply 完了という独立したイベントで更新するほうが局所的で安全。

### パニックリセット後に `conv_mode_authority` を手動で初期化する

→ 採用しなかった。パニックリセット以外にも「Engine Active のまま IME だけ再 apply」
  するパスが存在しうる（非 panic 経路）。パニック固有の処理ではなく
  apply 完了の一般的な副作用として修正するほうが防御的。

## 結果

- apply 完了ごとに `conv_mode_authority` が正しい値に収束するようになった
- パニックリセット後の「TSF warmup スキップ → IME desync」が解消
- `EngineStateChanged` の発火漏れによる `conv_mode_authority` の古値残存が
  構造的に不可能になった

## 関連 ADR

- ADR-064: conv_mode ポリシーゲート（`ConvModeAuthority` の意味と役割）
- ADR-038: ForceGuard / DriftMonitor（authority desync の別アプローチ）
- ADR-056: パニックリセットトリガーシーケンス（パニックリセットの発火条件）
