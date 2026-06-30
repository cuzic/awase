# ADR-067: F21/F22 → VK_IME_ON/OFF への完全移行と config1.db バインド廃止

## ステータス

採用済み（2026-06-28 実装、ADR-057 を廃止・置換）

実装コミット: `800233e` → `b271aee` → `8c39b8e` → `fdacda6`

## コンテキスト

### F21/F22 + config1.db バインドが使われていた経緯（ADR-057）

awase は Google 日本語入力（GJI）の IME ON/OFF 制御に、GJI の設定ファイル
`config1.db` へカスタムキーバインド（F21→IMEOn / F22→IMEOff）を書き込み、
そのキーを注入する方式を採っていた（ADR-057）。

この方式は以下の付帯機能を必要としていた：

- `gji.rs`: `config1.db` への差分パッチ（`patch` / `unpatch`）、GJI プロセスの
  停止・再起動、バインドエントリ（`ENTRIES`）の管理
- トレイメニューの「GJI セットアップ」「GJI 解除」項目（ユーザーが初回に手動実行）
- `gji_keybinds_ok` フラグ: バインドが `config1.db` に登録済みかを監視し、
  未登録なら GjiDirectStrategy を無効化する仕組み

つまり「awase を使う前に GJI の設定ファイルを書き換える」というセットアップ手順が
ユーザー体験に組み込まれていた。

### VK_IME_ON/OFF が使える理由（ADR-063）

MS-IME 対応（ADR-063）の過程で、TSF 共通層と IME 固有層を分離する設計を進めた結果、
GJI 環境でも Windows 標準の冪等キー `VK_IME_ON` (0x16) / `VK_IME_OFF` (0x1A) が
そのまま機能することが判明した。

- `VK_IME_ON` / `VK_IME_OFF` は GJI がネイティブに処理する冪等キーであり、
  `config1.db` へのカスタムバインドを一切必要としない
- Chrome / WezTerm / Windows Terminal すべてで動作を実機確認済み（2026-06-28）

これにより、ADR-057 が解決しようとしていた「F13/F14 が terminal でエスケープ
シーケンスを生成する」問題も、そもそも物理ファンクションキーを注入しないことで
構造的に消滅した。

## 決定

`VK_IME_ON` / `VK_IME_OFF` は GJI の `config1.db` バインドなしで動作するため、
F21/F22 を経由する必要がなくなった。これに伴い、F21/F22 キーバインドと
それを支える `config1.db` 管理機構の **全体** を削除する。

具体的には：

1. GJI の IME ON/OFF 送信キーを F21/F22 → `VK_IME_ON`/`VK_IME_OFF` へ置換する
2. `GjiDirectStrategy` の適用判定から `gji_keybinds_ok` 条件を撤去する
   （config1.db バインドの登録状態に依存しなくなる）
3. `config1.db` パッチ機構（`gji.rs`）・トレイの GJI セットアップ機能・
   `gji_keybinds_ok` 監視を全て削除する

### コミット群

| コミット | 内容 |
|---------|------|
| `800233e` | **test(gji)**: `GjiDirectStrategy` の IME ON/OFF を `VK_IME_ON`/`VK_IME_OFF` に切り替えて動作確認。`ime.rs` / `ime_controller.rs` / `output/mod.rs` を実装変更しテストで検証。 |
| `b271aee` | **refactor(gji)**: F21/F22 を `VK_IME_ON`/`VK_IME_OFF` に完全移行・`VK_F21`/`VK_F22` 定数削除。 |
| `8c39b8e` | **refactor(gji)**: `config1.db` F21/F22 バインド管理を完全削除（合計 -692 行）。 |
| `fdacda6` | **fix(tray)**: `tray.rs` 内の `GjiSetup`/`GjiTeardown` match arm を削除（残漏れ修正）。 |

`b271aee` の詳細：

- `send_unicode_cold_warmup_keys`: F21 → `VK_IME_ON`
- `send_chrome_gji_reinit_and_poll`: F22→F21 → `VK_IME_OFF`→`VK_IME_ON`
- `GjiDirectStrategy::is_applicable` から `gji_keybinds_ok` 条件を撤去
  （config1.db バインド不要になったため、GJI 検出のみで適用可）
- `on_ime_mode_vk_sent` の `VK_F21`/`VK_F22` dead condition を除去
- `vk.rs` から `VK_F21` / `VK_F22` 定数を削除

`8c39b8e` の詳細：

- `gji.rs` を削除（428 行）: `ENTRIES` / `patch` / `unpatch` / `run_gji_setup` /
  `teardown` および GJI プロセス管理
- `tray.rs`: `GjiSetup` / `GjiTeardown` メニュー項目・ハンドラを削除（-186 行）
- `tsf/observer.rs`: `gji_keybinds_ok` AtomicBool・`check_keybinds_in_db()`・
  `notify_gji_keybinds_registered/removed()` を削除
- `state/ime_decision_view.rs`: `gji_keybinds_ok` フィールドを削除
- `runtime/ime_refresh.rs`: `gji_keybinds_ok` 条件を撤去
- `tuning.rs`: `GJI_CONFIG_RECHECK_INTERVAL_MS` を削除

## 効果

- `gji.rs`（428 行）＋関連コード約 700 行を削除し、IME 制御パスから
  config1.db 監視・パッチという可動部品が丸ごと消えた
- トレイメニューから「GJI セットアップ」「GJI 解除」が消え、ユーザーは
  awase を起動するだけで GJI 制御が効くようになった（初回セットアップ手順が不要に）
- `gji_keybinds_ok` という「バインド登録済みか」の状態追跡が不要になり、
  GjiDirectStrategy の適用判定が「GJI 検出」のみのシンプルな条件になった
- 物理ファンクションキー（F21/F22）を一切注入しなくなり、ADR-057 が対象としていた
  terminal エスケープシーケンス漏れ／DirectInput 競合のリスク源が根本から消滅した

## 検討した代替案

### F21/F22 を維持しつつ config1.db 管理を続ける案

→ 採用しなかった。`VK_IME_ON`/`VK_IME_OFF` が config1.db バインドなしで全環境
（Chrome / WezTerm / Windows Terminal）で動作することを実機確認できた以上、
F21/F22 を経由する技術的理由が消えた。config1.db パッチ・GJI プロセス管理・
セットアップ UI・登録状態監視を維持し続けるコストに見合う利点がなく、
むしろ削除によってユーザー体験と保守性の両方が改善する。

## 変更ファイル一覧

| ファイル | 変更内容 |
|---------|---------|
| `ime.rs` | IME ON/OFF 送信キーを `VK_IME_ON`/`VK_IME_OFF` へ |
| `ime_controller.rs` | `GjiDirectStrategy::is_applicable` から `gji_keybinds_ok` 条件撤去 |
| `output/mod.rs` | cold warmup / reinit シーケンスのキーを置換 |
| `output/probe_io.rs` | probe シーケンスのキーを置換 |
| `vk.rs` | `VK_F21` / `VK_F22` 定数を削除 |
| `gji.rs` | ファイルごと削除（428 行: ENTRIES/patch/unpatch/run_gji_setup/teardown/プロセス管理） |
| `tray.rs` | `GjiSetup`/`GjiTeardown` メニュー項目・ハンドラ・match arm を削除 |
| `tsf/observer.rs` | `gji_keybinds_ok` AtomicBool・`check_keybinds_in_db()`・notify 関数群を削除 |
| `state/ime_decision_view.rs` | `gji_keybinds_ok` フィールドを削除 |
| `runtime/ime_refresh.rs` | `gji_keybinds_ok` 条件を撤去 |
| `runtime/message_handlers.rs` | GJI バインド関連ハンドラを削除 |
| `tuning.rs` | `GJI_CONFIG_RECHECK_INTERVAL_MS` を削除 |
| `lib.rs` | `gji` モジュール宣言を削除 |

## 関連 ADR

- ADR-057: GJI キーバインド F13/F14 → F21/F22 への移行（**本 ADR により廃止・置換**）
- ADR-063: TSF 共通層と IME 固有層の分離 + MS-IME 対応（VK_IME_ON/OFF が
  使えることが判明した基盤）
- ADR-034: GJI Direct Strategy（VK_IME_ON/OFF 移行セクションに本決定の経緯を反映済み）
