# ADR-062: InjectionMode 事後昇格: GJI write_bytes 観測による自動昇格

## ステータス

採用済み（2026-06-25 実装: 68c24e1、WriteTransferCount ベース: c7ef500）

## コンテキスト

awase はキー注入モードとして Unicode / Vk / Tsf の3種類を使い分ける。
フォーカス直後はウィンドウクラスが未学習であれば `InjectionMode::Unicode` から始まる。
WezTerm / Windows Terminal のような TSF text store 専用アプリは Unicode 直接注入
（`KEYEVENTF_UNICODE`）を TSF 経由で受け取らないため、文字がそのまま
リテラルとして貼り付けられてしまう。

正しいモード（Tsf）を知るには probe が必要だが、probe 用に余分な VK を送ると
やはりリテラル化するリスクがある。「リテラル化を起こさずに Tsf かどうかを判定する」
方法が求められた。

### WriteTransferCount の着想（c7ef500）

ADR-048（SacrificialWarmup）の Chrome 検証フローで、GJI プロセスの
`GetProcessIoCounters.WriteTransferCount` が「モード切り替えキー（F2）では
増加しないが、文字変換（VK_A→あ）では +200〜400B 増加する」ことが判明した。
この性質を転用し「Unicode 送信後に GJI が write したかどうか」を 100ms 待機して
バイト数差分で判断するアプローチを採用した。

## 決定

### UnicodeLiteralObserverFsm（commit 68c24e1）

`Platform::send_keys` が `InjectionMode::Unicode` かつ未学習クラスのとき、
`output.request_unicode_observation()` でフラグをセットする。

```rust
// platform.rs
if self.output.injection_mode == InjectionMode::Unicode
    && !self.focus.has_learned_injection_mode_tsf(self.focus.class_name())
{
    self.output.request_unicode_observation();
}
```

`KeyAction::Romaji` 処理時にフラグを消費し、送信直前の `gji_write_bytes()` を
ベースラインとして `UnicodeLiteralObserverFsm` を `pending_tsf` にインストールする。

```rust
// unicode_literal_observer.rs
const OBSERVATION_WINDOW_MS: u64 = 100;

fn tick(&mut self, _env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
    self.elapsed_ms += 10;
    if self.elapsed_ms < OBSERVATION_WINDOW_MS { return vec![]; }
    let current = crate::tsf::observer::gji_write_bytes();
    if current == self.baseline_bytes {
        // GJI write なし → TSF アプリ → 昇格
        vec![ProbeAction::UpgradeToTsf, ProbeAction::Done]
    } else {
        // GJI write あり → 標準 IMM32 アプリ → Unicode 維持
        vec![ProbeAction::Done]
    }
}
```

### 昇格時の処理（commit 68c24e1）

`DispatchResult::LearnedTsf` を受けた `advance_tsf_probe` が:

1. `focus.learn_injection_mode_tsf(class_name)` — ADR-058 の `InjectionModeStore` 経由で `cache.toml` に永続化
2. `output.update_injection_mode(InjectionMode::Tsf)` — 現セッションに即時適用
3. `output.mark_composition_cold_focus_change()` — 次回 TSF probe が正しく cold-start を踏むようリセット

## なぜこの設計か / 検討した代替案

| 案 | 評価 |
|---|---|
| probe 専用 VK を別途送る | リテラル化リスクがある（本課題の本質）|
| フォーカス時に静的クラス判別 | 未知クラスには対応できない |
| SacrificialWarmup の VK_A を再利用 | SacrificialWarmup は TSF mode 確定後に動くため、未学習クラスではトリガーしない |
| **Romaji 送信後 100ms 観測** | 採用。余分な VK を追加せず、既存の文字送信を probe に転用 |

`gji_last_io_ms`（時刻ベース）ではなく `gji_write_bytes`（バイト数ベース）を使う理由:
F2（モード切り替え）も I/O 時刻を更新するため誤検知するが、
WriteTransferCount は F2 で増加しない（`w_KB=+0.0`）ため分離できる。

## 結果

- 未学習クラスの初回フォーカス後、最初の Romaji 送信から 100ms で Tsf 判定が完了する
- 2回目以降のフォーカスは `cache.toml` から即座に `ForceTsf` が適用される（cold-start コストなし）
- 余分な probe VK を送らないため、IMM32 アプリへの影響ゼロ

## 関連 ADR

- ADR-004: Injection Mode 設計（Unicode/Vk/Tsf の使い分け方針）
- ADR-047: TickableFsm / ImeWarmupStrategy（`pending_tsf` / `TickableFsm` の仕組み）
- ADR-048: SacrificialWarmup（WriteTransferCount 観測の着想元）
- ADR-058: InjectionMode の cache.toml 永続化（`InjectionModeStore.learn_tsf()`）
