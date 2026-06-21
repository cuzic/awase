# Changelog

All notable changes to this project will be documented in this file.

## [1.1.1] - 2026-06-20

### バグ修正

- **Chrome → WezTerm フォーカス切替後の IME-OFF Engine-ON 状態** を修正 ([12dd094](https://github.com/cuzic/awase/commit/12dd094), [56f5e49](https://github.com/cuzic/awase/commit/56f5e49))
  - TsfNative 入場時に GJI F21 を `shadow_on` を無視して強制送信するよう変更
  - TsfNative cache miss 時の belief を carry-over (true) から安全デフォルト OFF に変更
  - フォーカス cache TTL を 5 秒 → 1 時間に延長（IME ON でウィンドウを離れて戻ると cache miss 扱いになっていた問題を解消）
  - 短期フォーカス (< 100ms) の cache 保存をスキップ（通知ポップアップ等が正常な状態を上書きするのを防止）
- **WezTerm gji_resumed 後の LiteralDetect false positive** を修正（「あ」が「a」になるケース） ([da8dad1](https://github.com/cuzic/awase/commit/da8dad1))
- **WezTerm gji_resumed 後の composition 早期確認** を実装（GJI I/O 変化を検知して待ち時間を短縮） ([aa8a79d](https://github.com/cuzic/awase/commit/aa8a79d))
- **comp-probe: RUNTIME 再入借用バグ** を修正（`shadow_on` / `jp` が常時 false になっていた） ([2225578](https://github.com/cuzic/awase/commit/2225578))
- **nicola-fsm: ソロ連打** でシフトカウンターが残存するバグを修正 ([a53344b](https://github.com/cuzic/awase/commit/a53344b))

### 内部改善

- GJI プロセスの I/O 統計 (ReadOperationCount / ReadTransferCount) を監視ログに追加 ([93236a8](https://github.com/cuzic/awase/commit/93236a8))

---

## [1.1.0] - 2026-06-20

### 重要な変更

- **GJI キーバインドを F13/F14 → F21/F22 に変更** ([7f8291f](https://github.com/cuzic/awase/commit/7f8291f))
  - F21/F22 は実キーボードに存在しない仮想キーで VT エスケープシーケンスを生成しない
  - WezTerm・Windows Terminal での Nop バインド設定が不要になった
  - **アップグレード時は必ずトレイメニューから「Google 日本語入力のセットアップ」を再実行してください**

### 新機能

- **GJI keybind 自動監視**: config1.db から F21/F22 エントリが消去された場合、30 秒以内に検知して自動再登録 ([58557e9](https://github.com/cuzic/awase/commit/58557e9))
- **トレイメニュー拡張**: GJI teardown・自動起動トグルを追加 ([e67cb49](https://github.com/cuzic/awase/commit/e67cb49))

### バグ修正

- **WezTerm long-idle 後の最初の文字リテラル化**（「こ」→「ko」）を修正（LiteralDetect + BS 再送方式） ([84e6942](https://github.com/cuzic/awase/commit/84e6942))
- **GJI IME-ON 不能バグ**を修正: `DirectInput\tF21\tIMEOn` エントリ欠落により F21 が無視されていた ([9d11cd7](https://github.com/cuzic/awase/commit/9d11cd7))
- **Teams / Chrome での partial literal**（「kおんな」→「こんな」変換失敗）を修正 ([3744457](https://github.com/cuzic/awase/commit/3744457), [040f8f8](https://github.com/cuzic/awase/commit/040f8f8))
- **GJI long-idle 後の LiteralDetect false positive** を修正 ([a6b4c0d](https://github.com/cuzic/awase/commit/a6b4c0d))

### パフォーマンス

- フォーカス変更直後の probe 待ち時間を 300ms → 100ms に短縮（入力レスポンス改善） ([23052fb](https://github.com/cuzic/awase/commit/23052fb))

### ドキュメント

- awase.cc の全ページで F13/F14 → F21/F22 に更新、WezTerm Nop 設定手順を削除

### 内部改善

- TSF probe の KeySeq 機構を削除（dead code）([550781f](https://github.com/cuzic/awase/commit/550781f))

## [1.0.1] - 2026-06-15

### バグ修正

- **Chrome VK モード**の「んい→に」変換バグを修正 ([71a4d68](https://github.com/cuzic/awase/commit/71a4d68))
- **Imm32Unavailable** 入場時に stale な `ime_on=false` が残るバグを修正 ([bfad1a8](https://github.com/cuzic/awase/commit/bfad1a8))
- Ctrl/Alt/Win 保持中の **KeyUp** を `on_key_down` と対称にバイパスするよう修正 ([5904d67](https://github.com/cuzic/awase/commit/5904d67))
- executor: **Relay モード**で Timer を即時実行し deferred timer の誤発火を修正 ([71ebfb9](https://github.com/cuzic/awase/commit/71ebfb9))
- executor: イベントキュー (`VecDeque`) の `push` を `push_back` に修正 ([3823f12](https://github.com/cuzic/awase/commit/3823f12))

### ドキュメント

- **ランディングページ**を大幅リニューアル（技術的差別化・ネーミング由来を追加）
- **使い方ページ** (usage.html) を新設（設定画面・config.toml 全項目・緊急操作手順を掲載）
- **内部動作解説ページ** (internals.html) を新設
- **FAQ** を大幅拡充
  - 高速タイピング時のシフト漏れ
  - Google IME でのトグルではなく冪等な IME 制御
  - 他ツール（やまぶき R 等）で起きがちな4つの症状と対策
  - Windows Terminal / WezTerm の F21/F22 Nop 設定手順
- コメント内の用語を統一（IMM → IMM32、IME-ON/OFF → IME ON/OFF、Henkan/Muhenkan → 変換/無変換）

### 削除

- `awase-gji-setup.exe` を配布物から削除（機能は awase 本体に統合済み）

### 内部改善

- `config`: `#[serde(default)]` 構造体に昇格して `default_*` 関数群 18 個を撤去（-142 行）
- `vk`: `enum VkMarker` を導入して bool/fn_ptr によるマーカー選択を型統一
- `fsm` / `nicola_fsm` / `ngram`: 重複コード・ラッパー関数・dead code を整理（合計 -270 行）
- rustfmt / clippy 整形

## [1.0.0] - 2026-06-14

最初の安定版リリース。

**Full Changelog**: https://github.com/cuzic/awase/compare/v0.1.0...v1.0.0

[1.1.1]: https://github.com/cuzic/awase/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/cuzic/awase/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/cuzic/awase/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/cuzic/awase/compare/v0.1.0...v1.0.0
