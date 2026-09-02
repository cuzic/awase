# ADR-118: Teams(WebView2/MS-IME) のかな入力ロック検知と通知

## ステータス

採用・実装済み（2026-09-02）。issue #137。Opus 2体（proposer/critic）による
3ラウンドの敵対的レビュー後の収束案に従う。自動復旧は BUG-61/BUG-62 追補4で
実機上不可能と確定済みのため、通知と案内だけを行う。

## 決定

1. 検知は `GetKeyState(VK_KANA)&1` の OS かなロック直読みを使う。conv の
   ROMAN ビットは使わない。Teams focus 中に言語バー操作で MS-IME を
   「かな入力」へ切替えると `off -> on`、「ローマ字入力」へ戻すと
   `on -> off` に追従することを `spike_kana_lock_probe.rs` で実機確認済み。
2. 復旧は案内のみ。`ImmSetConversionStatus`、`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`
   注入、TSF compartment write、言語バー COM 操作など、自動復旧の書き込みは
   恒久的にスコープ外。
3. belief/engine/IMM への書き込みは0本にする。`ImeEvent` は dispatch しない。
   偽陽性時の実害上限を「トレイ通知1回」に固定する。
4. 通知面は既存トレイ右クリックメニューとツールチップだけを使う。
   `NIM_SETVERSION` 化、`NIN_BALLOONUSERCLICK`、ADR-116 の診断画面や
   settings 側パネルへの相乗りはしない。
5. Google 日本語入力は今回の検知の対象外。`GetKeyState(VK_KANA)` は本来
   IME 非依存のはずだが、今回の実機テストでは GJI 側で言語バーからの手動切替
   自体が機能しなかった。これは別 issue とし、本 ADR のスコープに含めない。
6. ヒステリシス閾値は暫定値とする。On 3連続で Warn、Off 2連続で ClearWarned。
   BUG-14 の1打鍵ごとの foreign-injected `VK_KANA` エコーは hook 層で既に
   swallow 済みなので過大な N にはしない。未知の flap が Phase 0 で観測されたら、
   ログに基づいて調整する。

## 実装

Pure 層は `src/engine/kana_input_warn.rs`。`Unknown` は streak だけを切り、
`warned` は変えない。Observe 層は `crates/awase-windows/src/observer/kana_lock.rs`
で、`crate::vk::VK_KANA` を使って `GetKeyState` を読む。Apply 層は
`Runtime` に `KanaLockHysteresis` を保持し、`runtime/key_pipeline.rs` の
romaji VK/TSF 送信前にフック経由の打鍵1回につき1サンプルで観測する。

検知は romaji VK/TSF 送信の直前にだけサンプリングするため、Teams 側で
かな入力ロックへ反転した直後の最初の `KANA_LOCK_WARN_STREAK` (=3) 打鍵は、
警告閾値に達する前に JIS かなとして化けてから警告が出る。これは偽陰性側の
遅延特性として受け入れる。

相方の親指キーが来ずタイムアウトで確定する単独打鍵など、タイマー経由で
`Executor::execute_from_loop` から送信される効果は現状サンプリング対象外。
フック経由の通常打鍵だけで十分なサンプルを取れる想定だが、「1打鍵1サンプル」
という表現は実態として「フック経由の打鍵1回につき1サンプル」を意味する。

`Warn` 時はトレイの警告メニュー項目と警告ツールチップを表示し、フォアグラウンド
クラス名つきで `log::warn!` を残す。`ClearWarned` 時はメニュー項目を消し、
ツールチップを既定表示へ戻す。

## 非決定

この ADR は Teams/WebView2/MS-IME で発生した入力方式反転を検知するためのもの。
入力方式を戻す手段の再試行、conv の ROMAN ビットに基づく推定、journal replay
fixture の新規追加、`swallow_alt_kana_input_method_switch` の既定値変更は行わない。
