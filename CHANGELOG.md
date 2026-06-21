# Changelog

All notable changes to this project will be documented in this file.

## [1.2.0] - 2026-06-21

### 新機能

- **GjiFsm を新規追加**: GJI (Google 日本語入力) の内部 composition 状態を推測する FSM を導入 ([b152c7e](https://github.com/cuzic/awase/commit/b152c7e))
  - Phase 2a: GjiFsm を Output に接続し FocusChange / ImeOn / ImeOff / WarmupComplete を配線 ([bb228c2](https://github.com/cuzic/awase/commit/bb228c2))
  - Phase 2b: CompositionReset・KeyInput を配線し `is_composition_warm` を FSM 化 ([d49c516](https://github.com/cuzic/awase/commit/d49c516))
  - Phase 3: `is_composition_warm` を GjiFsm SSOT に切替 ([2b6d25f](https://github.com/cuzic/awase/commit/2b6d25f))
  - Phase 4: legacy epoch warm 追跡を撤去し GjiFsm を SSOT に一本化 ([588ea32](https://github.com/cuzic/awase/commit/588ea32))
- **panic ログ強化**: panic 発生時の場所とメッセージを `awase.log` に記録するフックを追加 ([de0226a](https://github.com/cuzic/awase/commit/de0226a))
- **更新履歴ページ自動生成**: GitHub Actions で `CHANGELOG.md` → `changelog.html` を自動生成するワークフローを追加 ([efc4310](https://github.com/cuzic/awase/commit/efc4310), [6fb7a38](https://github.com/cuzic/awase/commit/6fb7a38))

### バグ修正

- **WezTerm TSF cold start の F2+S race** と `gji_resumed` 後の false-positive BS を修正 ([b754277](https://github.com/cuzic/awase/commit/b754277))
- **CoreWindow キャッシュミス時** の IME ON carry-over によるひらがな注入を修正 ([1c5cc91](https://github.com/cuzic/awase/commit/1c5cc91))
- **NICOLA 同時打鍵** で `StartProbe` が上書きされる `debug_assert` パニックを修正 ([a5a9412](https://github.com/cuzic/awase/commit/a5a9412))
- **Chrome: f2_gji_long_idle** フラグ有効時も programmatic F2 を強制送信するよう修正 ([43dca5a](https://github.com/cuzic/awase/commit/43dca5a))
- **probe: SetOpenTrue** 時も `consecutive_count` をリセットするよう修正 ([cbd1946](https://github.com/cuzic/awase/commit/cbd1946))
- **tray**: `WM_CLOSE` を明示的にハンドルして意図しないシャットダウンを防止 ([c924eed](https://github.com/cuzic/awase/commit/c924eed))

### 内部改善

- **probe-fsm**: `TransmitPlan` / `ProbeObservations` 導入により FSM レイヤー境界を整理 ([3c36f21](https://github.com/cuzic/awase/commit/3c36f21))
- **chord 管理**: ImeStateHub に完全集約（Phase 2 完了） ([fd17da0](https://github.com/cuzic/awase/commit/fd17da0))
  - Chord 開始/終了判断を reducer に集約 ([a7218e0](https://github.com/cuzic/awase/commit/a7218e0))
  - `pending_warmup_on_keyup` を CompositionFsm に昇格 ([f3b0448](https://github.com/cuzic/awase/commit/f3b0448))
- **transport**: `suppress_physical` を `PhysicalKeyDisposition` に分離 ([8a045bb](https://github.com/cuzic/awase/commit/8a045bb))
- Clippy pedantic 対応（CI Rust 1.96） ([881f824](https://github.com/cuzic/awase/commit/881f824))

---

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

[1.2.0]: https://github.com/cuzic/awase/compare/v1.1.1...v1.2.0
[1.1.1]: https://github.com/cuzic/awase/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/cuzic/awase/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/cuzic/awase/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/cuzic/awase/compare/v0.1.0...v1.0.0
