# ADR-049: TSF mode LiteralDetect と WezTerm long-idle warm 維持パターン

## ステータス

採用済み（2026-06-07〜2026-06-18 実装）

## コンテキスト

### WezTerm の特性

WezTerm は TSF native アプリであり、GJI の TSF text service (ITfKeyEventSink) が
キーイベントを横取りする。そのため:

- `ImmGetDefaultIMEWnd` が NULL を返す（IMM32 クロスプロセス不可）
- F13/F14 が WezTerm に ESC[25~/26~ として届く（IME ON/OFF の副作用）
- GJI の I/O が TSF 経由のため `GetProcessIoCounters.WriteTransferCount` で観測可能

### 問題: WezTerm long-idle 後の2文字目リテラル化

WezTerm で長期アイドル（55797ms 例）後に "こ" を入力すると "koちら" になる。

**タイムライン（実測）**:
```
t=0ms    物理 F2 + eager warmup F2 送信
t=312ms  GjiProbe 完了 → probe fresh F2 (#3) 送信
t=312ms  NameChangeWait（300ms）開始
t=612ms  NameChangeWait 終了 → K + O 送信
t=656ms  WezTerm が F2 処理完了（composition context 再初期化完了）
         ← K + O はこの 44ms 前に送信済み
→ K + O が GJI を素通りして ASCII 'ko' がターミナルに直接出力
```

WezTerm の F2 後 composition context 再初期化には **344ms** 必要（idle 時間に比例して増加）。
NameChangeWait の 300ms 固定値では追い付かない。

### 否定された代替案

**NameChangeWait を 300ms → 500ms に延長**（Fix #1）:
- WezTerm の再初期化時間は idle 時間に依存するため、固定値での対処は競合条件を
  別の閾値に移すだけ。
- idle が極端に長い場合（10分など）に 500ms でも追い付かない可能性がある。

## 決定

### アプローチ: LiteralDetect + BS×2 + warm 再送

リテラル化を事前に防ぐのではなく、**リテラル化を検出してから修復する**。

#### 1. TSF mode で LiteralDetect を有効化（commit 84e6942）

`probe_io.rs` の `needs_literal` 条件から `&& !io.is_tsf_mode()` を除去した。

**判定方法**: `gji_candidate_show.has_changed()` の観測。
- warm（GJI が composition を処理）: K + O 後 300ms 以内に GJI SHOW イベントが来る
- cold（GJI を素通り）: 300ms 以内に SHOW が来ない → `SuspectedLiteral` → `RawTsfLiteralRecovery`

#### 2. warm を維持したまま BS + 再送（commit 84e6942）

`RawTsfLiteralRecovery` で `is_tsf_mode() = true` のとき:

```rust
// NG: mark_cold_raw_tsf() を呼ぶと cold 経路に入る
// cold 経路 → flush_raw_tsf_literal_romaji → F2 warmup を再送
//           → WezTerm の 344ms タイマーがリセット → また遅延

// OK: increment_consecutive_count() のみ（warm を維持）
// warm 経路 → send_romaji_as_tsf_warm → VK 直接送信
//           → WezTerm がこの時点で準備完了 → "こ" ✓
```

**warm を維持する理由**: cold にすると `flush_raw_tsf_literal_romaji` が
F2 warmup を再送してしまい、WezTerm の 344ms 初期化タイマーを再起動させる。
リテラルを検出した時点では WezTerm はすでに準備完了しているため、
warm のまま VK を直接再送するのが正しい。

**無限ループ防止**: `send_romaji_as_tsf_warm` には `!self.is_tsf_mode()` ガードが
すでにあるため、warm 再送では LiteralDetect が再起動しない。

#### 3. GjiDirectStrategy の TsfNative 除外を撤廃（commit 関連）

従来 WezTerm（TsfNative）は GjiDirectStrategy から除外されており、
VK_KANJI（トグル）フォールバックを使っていた。トグルはべき等でなく、
スリープ復帰後の desync で2回目以降がスキップされる問題があった。

**解決策**: WezTerm 側で F13/F14 を `Nop` にバインドする。
GJI の TSF text service が F14 を消費することを実機確認。
→ awase が F14（IME OFF）を送ると GJI が消費し、WezTerm には届かない。
→ GjiDirectStrategy がべき等な IME OFF を実現。

```lua
-- ~/.config/wezterm/wezterm.lua
{ key = 'F13', action = act.DisableDefaultAssignment },
{ key = 'F14', action = act.DisableDefaultAssignment },
```

#### 4. フォーカス直後の injection_mode 同期

フォーカス変更直後に `injection_mode` が前のウィンドウの値を持っている場合、
LiteralDetect の `is_tsf_mode()` が誤った値を返す問題があった（commit 6ecb8e9）。

フォーカスバリア消費時に `injection_mode` を即時同期するよう修正した。
（一度 revert → 再適用のいきさつ: commit c089757 → 97c922e → 6ecb8e9）

## 結果

- WezTerm long-idle 後の2文字目リテラル化が根本解消
- 固定タイムアウト値への依存から「検出して修復」パターンへ移行
- GjiDirectStrategy がべき等な IME ON/OFF を WezTerm でも実現
- 無限ループを cold/warm の責務分離で構造的に防止

## 一般化された設計原則

このパターンは「TSF native アプリでの cold-start 問題」の一般解として適用可能:

> タイミング競合を固定値で回避しようとすると別の閾値に競合が移るだけ。
> リテラルを検出してから warm 状態で再送する方が、アプリの初期化遅延に依存しない。

## 関連 ADR

- ADR-047: TickableFsm / ImeWarmupStrategy（LiteralDetectFsm の位置づけ）
- ADR-046: GjiFsm（GjiDirectStrategy TsfNative 除外撤廃の背景）
- ADR-034: GJI Direct Strategy
- workarounds.md: TSF mode 固有ワークアラウンド
