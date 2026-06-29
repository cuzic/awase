# Changelog

All notable changes to this project will be documented in this file.

## [1.6.0] - 2026-06-29

### 新機能

- **Microsoft IME 完全対応** ([45edf19](https://github.com/cuzic/awase/commit/45edf19), [56cd9a5](https://github.com/cuzic/awase/commit/56cd9a5))
  - `MsImeDirectStrategy`（`VK_DBE_HIRAGANA` / `VK_DBE_ALPHANUMERIC`）による冪等 ON/OFF 制御（ADR-063）
  - 起動時に MS-IME を検出するとキー割り当てを自動設定（無変換 → IME オフ、変換 → IME オン）
  - `WM_IME_KIND_CHANGED` でランタイムに GJI / MS-IME を切り替え
- **TSF EnumProfiles による GJI CLSID 動的発見** ([55233f0](https://github.com/cuzic/awase/commit/55233f0), [005f0a6](https://github.com/cuzic/awase/commit/005f0a6))
  - プロセス名依存から CLSID ベースの IME 種別判定に移行
  - CLSID を `cache.toml` に永続化し再起動後のコールドスタートコストを削減

### 改善

- **GJI IME 制御を VK_IME_ON/OFF (0x16/0x1A) に移行** — config1.db パッチ不要に ([b271aee](https://github.com/cuzic/awase/commit/b271aee), [8c39b8e](https://github.com/cuzic/awase/commit/8c39b8e))
  - `gji.rs`（config1.db パッチ・プロセス管理）を完全削除（-692 行）
  - トレイメニューの「GJI セットアップ / 解除」を撤去、初回インストール作業が不要に
  - Chrome / WezTerm / Windows Terminal すべてで VK_IME_ON/OFF の動作を実機確認（2026-06-28）
  - `gji_keybinds_ok` フラグ・Observer の config1.db 監視ループを撤去
- **IME 種別判定を CLSID ベースに一本化** ([2d5cfe9](https://github.com/cuzic/awase/commit/2d5cfe9))
  - `gji_write_idle_ms` ヒューリスティックを撤去し判定精度を向上

### バグ修正

- **WezTerm で Enter 後の最初の文字（「な」等）が突然消えるバグを修正** ([3ffbe66](https://github.com/cuzic/awase/commit/3ffbe66))
  - ReinjectConfirmKey + TSF mode で `nc_fired` を昇格し LiteralDetect 誤検出を抑制
- **MS-IME アクティブ時の LiteralDetect 誤発火による BS 連射を修正** ([699ab5f](https://github.com/cuzic/awase/commit/699ab5f))
- **パニックリセットでカタカナ・JIS かな状態からもローマ字ひらがなに復帰** ([a88bb36](https://github.com/cuzic/awase/commit/a88bb36), [6ee20bf](https://github.com/cuzic/awase/commit/6ee20bf))
  - `IMC_SETCONVERSIONMODE` で `NATIVE | FULLSHAPE | ROMAN` を強制し `KATAKANA` を落とす
  - 半角カタカナ（JIS かな入力モード含む）・全角カタカナからでもローマ字ひらがなに戻る

## [1.5.0] - 2026-06-27

### 新機能

- **トレイメニューに「内部状態をリセット」を追加** ([1591593](https://github.com/cuzic/awase/commit/1591593))
  - IME 状態や FSM が壊れたときにマウス操作だけで内部状態を初期化可能
  - キーボードが正常に動かない状況でも確実にリセットできる（ADR-052）
- **StepCoro — タイマー駆動コルーチン基盤を導入** ([e548e63](https://github.com/cuzic/awase/commit/e548e63))
  - `GjiWarmupFsm` / `LiteralDetectFsm` / `SacrificialWarmupFsm` / `TsfProbeMachine` を StepCoro に置き換え（ADR-053）
  - FSM ステートの中間変数をコルーチンのローカル変数として記述でき、コード量を大幅削減
  - `timed_fsm::coro` モジュールとして昇格（[118b3cf](https://github.com/cuzic/awase/commit/118b3cf)）
- **InjectionMode を cache.toml で永続化** ([c6fed60](https://github.com/cuzic/awase/commit/c6fed60))
  - `InjectionModeStore` を追加し、セッションをまたいで注入モードを記憶
  - 再起動後の cold-start コストを削減（ADR-058）
  - `imm_cache.toml` を `cache.toml` に統合（[10918c2](https://github.com/cuzic/awase/commit/10918c2)）
- **InjectionMode 事後昇格: GJI write_bytes 観測で自動昇格** ([68c24e1](https://github.com/cuzic/awase/commit/68c24e1))
  - フォーカス直後に Unicode モードで送信したキーが GJI に処理されたか WriteTransferCount で判定
  - GJI が動作していれば injection_mode を Tsf に自動昇格（ADR-062）
- **Unicode long-cold 対応: UnicodeColdWarmupFsm** ([70d3fed](https://github.com/cuzic/awase/commit/70d3fed))
  - Unicode モードで長時間アイドル後に VK_IME_ON poke を送り GJI の起動を確認してから文字を送信
  - GJI が応答するまで文字送信を保留することで partial literal を防止
- **競合ソフトウェア起動時チェックを追加** ([9875c2c](https://github.com/cuzic/awase/commit/9875c2c))
  - やまぶき等の NICOLA 対応 IME が同時に起動している場合、バルーン通知で警告（ADR-060）

### 改善

- **自動起動を schtasks → HKCU\\Run レジストリに移行** ([0d09d80](https://github.com/cuzic/awase/commit/0d09d80))
  - ログオン即起動（30 秒遅延が不要に）、管理者権限なしで登録可能（ADR-059）
  - 旧 schtasks タスクは起動時に自動削除（[584897a](https://github.com/cuzic/awase/commit/584897a)）
- **Windows Terminal を force_tsf に追加** ([4e49ee1](https://github.com/cuzic/awase/commit/4e49ee1))
  - GJI コンポジションを確実に有効化し、Windows Terminal での変換精度を向上

### バグ修正

- **Win キー押下中の IME キー注入をスキップ** ([6469f51](https://github.com/cuzic/awase/commit/6469f51))
  - Win+A でスタートメニューが開いてしまう誤動作を修正（ADR-061）
- **仮想デスクトップ切替時の誤 IME キー送信を防止** ([a97173a](https://github.com/cuzic/awase/commit/a97173a))
  - 仮想デスクトップを切り替えた直後に LINE 等へ誤って IME キーが送信されていたフォーカスガード漏れを修正
- **Partial literal の残骸を BS で除去** ([f3ff84d](https://github.com/cuzic/awase/commit/f3ff84d))
  - TSF literal recovery が give-up したとき terminal に残ったリテラル文字を BS で削除
- **Partial literal 検出後の resend を SacrificialWarmup 化** ([62ad28b](https://github.com/cuzic/awase/commit/62ad28b))
  - 部分リテラル検出後の再送を安全な warmup フローに統一
- **SacrificialWarmup Phase2 早期 HIDE 後に IPC settle 待機を追加** ([88d562f](https://github.com/cuzic/awase/commit/88d562f))
  - 候補ウィンドウが早期 HIDE された後、GJI IPC が完了するまで待機してから次入力を解放
- **Unicode long-cold で VK_IME_ON 単体 → VK_IME_ON+VK_A+BS 犠牲キーに変更** ([6cb175f](https://github.com/cuzic/awase/commit/6cb175f))
  - GJI の WriteTransferCount を確実に増加させ cold 判定の精度を向上
- **TsfNative/Imm32Unavailable で shadow 値を代替観測として記録** ([1e77002](https://github.com/cuzic/awase/commit/1e77002))
  - IMM32 が利用不可なウィンドウでも shadow_on を観測値として保存し状態推定を安定化

### 内部改善

- **`#[allow]` を `#[expect]` に一括置換**（全クレート）([c2b0685](https://github.com/cuzic/awase/commit/c2b0685))
  - Rust 2024 edition の lint 強化に対応、抑制理由が不要になった時点でコンパイラが警告
- **`unsafe extern` を明示**（Rust 2024 lint 対応）([f645d3c](https://github.com/cuzic/awase/commit/f645d3c))
- **awase-settings を edition 2024 に移行** ([ee2487c](https://github.com/cuzic/awase/commit/ee2487c))
- **GjiWarmupFsm / StartLiteralDetect を撤去**（StepCoro 移行後の dead code 除去）([f01d401](https://github.com/cuzic/awase/commit/f01d401))
- **MSRV を 1.85 に引き上げ**（`timed_fsm::coro` が `async`/`await` 構文を使用）

---

## [1.4.0] - 2026-06-25

### 新機能

- **[[post_bypass]] 設定を追加**: tmux (`Ctrl+B`) や screen など、コマンドキーの直後の次打鍵を NICOLA スキップする設定を追加 ([a67ebb5](https://github.com/cuzic/awase/commit/a67ebb5))
  - `[[post_bypass]]` セクションで対象キーと待機時間を指定可能
  - tmux 向けのコメントと設定例を TOML サンプルに追記 ([49b8343](https://github.com/cuzic/awase/commit/49b8343))
- **SacrificialWarmup を導入**: Chrome cold-start と WezTerm long-idle の部分リテラル化を、自己犠牲 VK_A+BS バッチで解消 ([28fc97a](https://github.com/cuzic/awase/commit/28fc97a), [16a288d](https://github.com/cuzic/awase/commit/16a288d))
  - Chrome の「kおのなかで」型 partial literal を warmup フェーズで事前修正
  - WezTerm が長時間アイドル後に Engine-ON/IME-OFF 状態になるケースを解消
  - long-cold + TSF mode 専用に限定して通常時の性能低下を防止 ([d02ec44](https://github.com/cuzic/awase/commit/d02ec44))
- **ImeModeFsm + ChromeGjiReinitFsm を実装**: F22→F21 送信後、GJI が Hiragana モードに落ち着いたことを確認してから次入力を許可する FSM を追加 ([2c8d647](https://github.com/cuzic/awase/commit/2c8d647))
- **Chrome 用 LiteralDetector を WriteTransferCount ベースに改善**: パイプへの書き込みバイト数で composition 完了を判定し、誤 BS を抑制 ([c7ef500](https://github.com/cuzic/awase/commit/c7ef500))
- **F21/F22 全送信パスで belief 更新 + VK 直後の FocusChange 抑制**: IME モード state の信頼度を高め、不要な re-probe を削減 ([1a0e54e](https://github.com/cuzic/awase/commit/1a0e54e))

### バグ修正

**Ctrl+J 問題の完全修正**

- **IME ON 状態で Ctrl+J が tmux に届かない問題を修正** ([32a037d](https://github.com/cuzic/awase/commit/32a037d))
  - TsfGate が IME ON 中の Ctrl+J を GJI warmup パスに誤送信していた
- **Ctrl+J が GJI に横取りされる問題を再修正** ([ee0b1fd](https://github.com/cuzic/awase/commit/ee0b1fd))
- **Ctrl バイパス後に modifier 先行リリースが来ると J↑ を誤 Suppress する問題を修正** ([bd51ea1](https://github.com/cuzic/awase/commit/bd51ea1))
- **Ctrl+key bypass 直後の次キーを NICOLA スキップするよう修正** ([ddb6b58](https://github.com/cuzic/awase/commit/ddb6b58))
  - バイパス直後のキーが親指シフト同時打鍵と誤判定されるケースを排除

**SacrificialWarmup の安定化**

- **gate=Bypass で即終了するバグを修正** ([0426fb6](https://github.com/cuzic/awase/commit/0426fb6))
- **VK_A+BS を同一バッチで送信して文字フラッシュを防止** ([af906b1](https://github.com/cuzic/awase/commit/af906b1))
- **write_bytes ベースラインを VK_A 送信前に取得し閾値を 350B に調整** ([26bc0fe](https://github.com/cuzic/awase/commit/26bc0fe))
- **Chrome cold タイムアウト時に F22→F21 で GJI を強制リセット** ([9630acb](https://github.com/cuzic/awase/commit/9630acb))
- **Chrome warm 確認後、HIDE 待機してから実ローマ字を再送するよう修正** ([2355761](https://github.com/cuzic/awase/commit/2355761))
- **VK_A+BS atomic batch 後の早期 HIDE で「おお」が消える IPC race を修正** ([123efb1](https://github.com/cuzic/awase/commit/123efb1))
- **`composition_was_seen` が最初の tick で false になる drain 順序バグを修正** ([e40a2eb](https://github.com/cuzic/awase/commit/e40a2eb))

**WezTerm**

- **WezTerm long-idle 後の 2 文字目リテラル化を PendingGjiConfirm 状態で修正** ([7041b20](https://github.com/cuzic/awase/commit/7041b20))
- **フォーカス直後 injection_mode を同期し、Unicode long-cold 時に F22→F21 再初期化** ([6ecb8e9](https://github.com/cuzic/awase/commit/6ecb8e9))
- **HWND キャスト型を `*mut c_void` に修正**（`isize` への暗黙変換が Windows API 境界で UB）([ff1f092](https://github.com/cuzic/awase/commit/ff1f092))

**GJI FSM**

- **Unicode モードの `StartProbe` で即 `WarmupComplete` を dispatch**（OnCold 固着バグ修正）([444e9a6](https://github.com/cuzic/awase/commit/444e9a6))
- **ImeOn 直後の FocusChange で proactive probe を維持するよう修正** ([782f10d](https://github.com/cuzic/awase/commit/782f10d))
- **FocusChange spawn_local ポーリングに generation ガードを追加**（二重適用防止）([90c9962](https://github.com/cuzic/awase/commit/90c9962))

**その他**

- `ValidatedConfig` に `post_bypass` フィールドを追加（ビルドエラー修正）([089858d](https://github.com/cuzic/awase/commit/089858d))
- Unknown → 実値 の IME 状態遷移を drift WARN から initial confirm DEBUG に格下げ ([4347928](https://github.com/cuzic/awase/commit/4347928))
- `sleep_ms` に `u64` を渡していた型ミスマッチを修正 ([ddc6be3](https://github.com/cuzic/awase/commit/ddc6be3))

### 内部改善

- **HoldingGate / GateAction を timed-fsm クレートへ移植**: ゲートロジックをライブラリ層に集約し再利用性を向上 ([ebbc9e4](https://github.com/cuzic/awase/commit/ebbc9e4))
- **HwndId に `to_hwnd()` を追加し raw cast を一箇所に集約** ([1932948](https://github.com/cuzic/awase/commit/1932948))
- **`gji_candidate_visible` を env 経由で参照**（直接ポーリング撤去）([19f4372](https://github.com/cuzic/awase/commit/19f4372))
- **SacrificialResend から不要な `plan` / `observations` フィールドを削除** ([61c4c12](https://github.com/cuzic/awase/commit/61c4c12))

---

## [1.3.0] - 2026-06-23

### バグ修正

- **ALT+TAB が連続して押せない問題を修正** ([d6cb1a4](https://github.com/cuzic/awase/commit/d6cb1a4))
  - フォーカス変化のたびに F21/F22 (IME ON/OFF キー) を送信する際、ALT を一時的に解放していたため ALT+TAB スイッチャーが「ALT 離した＝確定」と誤認していた
  - F21/F22 は GJI 専用の仮想 VK のため ALT を保持したまま送信しても正常に動作する
- **GJI long-idle 後の「kお」cold start バグを修正** ([e4a6248](https://github.com/cuzic/awase/commit/e4a6248), [c571acf](https://github.com/cuzic/awase/commit/c571acf))
  - GJI が 8〜10 秒以上アイドル後、最初のキー入力が部分リテラル化（「こ」→「kお」）するケースを修正
  - medium idle (7〜10 秒) でも GJI 無応答タイムアウト時に F2 をバッチ同梱するよう改善 ([2159dca](https://github.com/cuzic/awase/commit/2159dca))
- **部分リテラル（「kお」「seつぞく」）の修正** ([5562b6a](https://github.com/cuzic/awase/commit/5562b6a), [d54f5b1](https://github.com/cuzic/awase/commit/d54f5b1), [125d2c1](https://github.com/cuzic/awase/commit/125d2c1))
  - GJI I/O 応答後に `gji_resumed` を設定して部分リテラルを救済
  - TSF mode でも LiteralDetect を有効化（「seつぞく」の再発防止）
  - 部分リテラル BS 数を `chars.len()` から 2 固定に修正
- **Chrome パスの LiteralDetector を改善** ([c279fc7](https://github.com/cuzic/awase/commit/c279fc7))
  - `new_gji_resumed` に切り替えて GJI resume 後のリテラル誤検出を抑制
- **probe-fsm: F2 二重送信・遅延の修正** ([d195c43](https://github.com/cuzic/awase/commit/d195c43), [4de228b](https://github.com/cuzic/awase/commit/4de228b))
  - ReWarmup/non-eager パスで TSF バッチへの F2 二重送信を抑制
  - GJI pre-idle 時に fresh F2 + NameChangeWait をスキップして遅延を削減
- **composition-fsm: Long cold 状態を正しく維持するよう修正** ([c96598f](https://github.com/cuzic/awase/commit/c96598f))

### 新機能

- **GjiWarmupFsm を新規作成**: GJI cold-start warmup 専用 FSM を導入し warm-up ロジックを独立化 ([fcd1b82](https://github.com/cuzic/awase/commit/fcd1b82), [f768944](https://github.com/cuzic/awase/commit/f768944))
- **LiteralDetectFsm を新規作成**: warm パス・GJI post-transmit で共用するリテラル検出 FSM ([8608062](https://github.com/cuzic/awase/commit/8608062), [660ee19](https://github.com/cuzic/awase/commit/660ee19))
- **ChromeProbe を新規作成**: `pending_tsf` を `Box<dyn TickableFsm>` に換装し Chrome 専用 probe を追加 ([51901c0](https://github.com/cuzic/awase/commit/51901c0))
- **ColdKind::Medium を追加**: GJI idle 時間を Long / Medium / Short に分類して warmup 戦略を最適化 ([bf3eade](https://github.com/cuzic/awase/commit/bf3eade))
- **NameChangeWait で candidate 可視時に即 transmit**: WezTerm での probe 待ち時間を最大 300ms 短縮 ([25182eb](https://github.com/cuzic/awase/commit/25182eb))

### 内部改善

- **TickableFsm トレイト定義**: `TsfProbeMachine` / `GjiWarmupFsm` / `LiteralDetectFsm` / `ChromeProbe` が共通インターフェースを実装 ([d22e987](https://github.com/cuzic/awase/commit/d22e987))
- **ImeWarmupStrategy トレイト定義**: `GjiFsm` / `MsImeStrategy` を統一インターフェースで扱えるよう抽象化 ([eb8b9d4](https://github.com/cuzic/awase/commit/eb8b9d4))
- **GjiFsm 大規模リファクタリング**: `ProbeStatus` を `Authorized+Executing` に分離し Cell 3本を撤去、`OutputActiveGuard` を Output に移動 ([0ca92b2](https://github.com/cuzic/awase/commit/0ca92b2), [65e0a1a](https://github.com/cuzic/awase/commit/65e0a1a))
- **transport リファクタリング**: `PassthroughQueue` を抽出・`PhysicalKeyDisposition::plan()` に F2 ケースを統合 ([3cfc57b](https://github.com/cuzic/awase/commit/3cfc57b), [005ed17](https://github.com/cuzic/awase/commit/005ed17))

---

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
