# ADR-083: `conv_mode_policy = force` — cold 転換時に awase トレイの目標 conv モードを強制する opt-in 設定

## ステータス

実装済み（2026-08-05）。デフォルトは `observe`（従来動作、無効）。Windows 実機での
動作確認は未実施。

## コンテキスト

BUG-47（`docs/known-bugs.md`）の調査を通じて、`VK_DBE_KATAKANA`/`VK_DBE_ALPHANUMERIC`
等の物理キー漏洩により、awase が一切書き込みをしていないのに実 IME の conv モード
（英数/ひらがな/カタカナ × 半角/全角）が意図せず変化しうることが実機で確認された。
BUG-47 自体はこの漏洩経路を塞いだが、根本的に「実 IME の conv モードが何らかの経路で
awase の意図と乖離しうる」という前提そのものは消えない（BUG-47 の穴が塞がれても、
将来別の経路で同様の乖離が起きる可能性は残る）。

ADR-078（IME conv-mode belief の三分割）は、この種の乖離への根本的な解決として
「観測を信じず awase 自身の意図を権威にする」設計への全面移行を提案しているが、
`DesiredMode`/`EffectiveMode`/`ModeConstraint` 型分割・`ModeEvent`/`ModeEffect`・
config1.db 対応まで含む大掛かりな設計で、Phase 1a（増幅ループの実質撤去のみ）を
除き未実装のまま残っている。

本 ADR は、ADR-078 の全面実装を待たず、**乖離を許容しつつ定期的に正す**という
より軽量な緩和策を、ユーザーが選択できる opt-in 設定として先に提供する。

## 決定

### 3軸の整理

conv モードは実際には独立した複数の軸を持つ。誤って IME ON/OFF と混同しないこと
（本 ADR の設計レビューでユーザーから指摘・訂正された点）:

- **軸1: IME ON/OFF**（`ImeModel::desired_open`、既存・本 ADR は触らない）—
  IME コンポーネント自体が有効かどうか。
- **軸2〜4: conv モード**（`ConvMode` = `Charset` × `romaji`、`awase::engine::conv`）—
  IME が ON の状態でのみ意味を持つ。`Charset` は 英数/ひらがな/カタカナ × 半角/全角
  の組み合わせ（`Hiragana`/`ZenkakuKatakana`/`HankakuKatakana`/`ZenkakuAlpha`/
  `HankakuAlpha`）。

軸2〜4は「IME が開いたまま英数モードになる」（`ImeFullAlpha`/`ImeHalfAlpha` トレイ
コマンド、`open=true` で conv だけ変える）という既存挙動が示す通り、軸1とは独立。

### 新設: `conv_mode_policy`（config.toml）

`GeneralConfig::conv_mode_policy: ConvModePolicy`（`observe` | `force`、デフォルト
`observe`）。`awase-settings` の「詳細設定」タブに UI を追加。

- `observe`（デフォルト）: 従来どおり。`ConvModeMgr` は conv を観測するのみで、
  cold 転換時は ROMAN ビット確保のみ（BUG-19 で撤去された挙動のまま）。
- `force`: cold 転換のたびに、`ConvModeMgr::desired_mode()` へ冪等に強制書き込みする。

### 新設: `ConvModeMgr::desired_mode`

awase トレイの `ImeHiragana`/`ImeFullKatakana`/`ImeHalfKatakana`/`ImeFullAlpha`/
`ImeHalfAlpha`（既存コマンド、`message_handlers.rs`）が唯一の書き込み点。
GJI/MS-IME 側のトレイやその他の経路で実 conv が変わっても `desired_mode` 自体は
変化しない — 次の cold 転換で `force` ポリシーが上書きする、という設計。
デフォルト値は全角ひらがな（`Charset::Hiragana, romaji: true`）。

### 強制の実装位置: `cold_warmup.rs::run_start`

全 cold セッションが通る唯一の入口（BUG-19 追補8で確立済み）。既存の
「ROMAN ビットのみ復元」ロジックを、`policy == Force` のときだけ
`ConvMode::to_conv_bits()`（本 ADR で新設、`desired_mode` から完全な conv
ビット列を計算する純粋関数）による完全な目標値に差し替える。既存の非同期・
冪等な書き込みインフラ（`spawn_local` + `set_ime_romaji_mode_with_target_async`）
をそのまま再利用するため、新規の同期 Win32 呼び出しは増えていない。

### なぜ BUG-19 の自己増幅ループを再現しないか

BUG-19 の破綻は「**観測した**カタカナに**追従**して同じ方向へ書き込み続ける」
という正のフィードバックループだった（一発の誤読が確定 → warmup がそれを見て
カタカナキーを送信 → 実際にカタカナに固定 → 以後の観測もカタカナ → 確定を強化）。

本設計は逆方向: **観測結果を一切参照せず**、常に固定の `desired_mode` へ引き戻す
一方向の書き込みのみ行う。観測が書き込みの引き金にならないため、
「観測→書き込み→観測強化」という増幅経路が構造的に存在しない。

## 不変条件

- `desired_mode` は awase トレイの Ime系コマンド以外から書き込まれない
  （`ConvModeMgr::set_desired_mode` の唯一の呼び出し元は `message_handlers.rs`
  の `set_desired_conv_mode`）。
- `policy = observe`（デフォルト）のとき、`cold_warmup.rs::run_start` の挙動は
  本 ADR 以前と完全に同一（`forced_target = None` → 従来どおり ROMAN ビットのみ）。
- IME ON/OFF（`ImeModel::desired_open`）は本機能から一切参照・変更されない。

## 未対応・今後の課題

- Windows 実機での動作確認（`force` ポリシー有効時の cold 転換頻度・レイテンシ
  への影響、実際に BUG-47 的な乖離を正せるか）は未実施。
- `desired_mode` はプロセス再起動でデフォルト（全角ひらがな）にリセットされる
  （config.toml への永続化はスコープ外、トレイでの都度選択を想定）。
- ADR-078 の全面実装（観測モデル自体の再設計）は本 ADR の対象外。本 ADR は
  「観測を信じない」方向への軽量な一歩ではあるが、`ConvModeMgr::update_from_conv`
  の観測・デバウンスロジック自体は変更していない。

## 追記（2026-08-07）: `conv_mode_policy = force` を IME ON/OFF 軸にも適用

**きっかけ:** ユーザー実機報告「なぜか、IME OFF Engine ONの状態になりました」
（`test/combined-katakana-fixes` での試験運用中）。タイピングすると変換されず
ローマ字がそのまま出力された。ログを追ったが、divergence の発生した瞬間は
可視範囲内に見つからなかった。

**構造的な原因:** `Blacklist`（Chrome/WindowsTerminal 等、IMM32 クロスプロセス
制御が使えない `Imm32Unavailable`/`TsfNative` プロファイル）アプリでは、実 IME
の open/close 状態を独立してポーリングする経路が存在しない
（`ir_stage_observe` の `ImeReadStrategy::Blacklist` 分岐、
`Skipping IMM query for known-broken class`）。したがって既存の
`ir_apply_drift_correction`（`observed != desired` を検出して補正する仕組み）は
Blacklist アプリでは `observed` が更新されないため実質的に発動し得ない。

既存の `apply_force_on_for_imm_broken()`（`runtime/mod.rs`、Blacklist アプリ向けに
belief=ON のとき idempotent な VK_IME_ON 系キーを再送する専用パス、500ms ごとの
`ir_stage_notify` から呼ばれる）はこの用途にほぼ合致していたが、
「`applied`（awase 自身が記録する『前回のapply結果』キャッシュ）が既に ON なら
送らない」という自己スロットリングを持つ。`applied` が一度でも誤って
「成功」記録されると（実 IME が別経路で無音のうちに閉じた等）、以後は永久に
再送されず、`belief=ON` × `実IME=OFF` の乖離を検出も訂正もできなくなる —
conv モードで `desired_mode` を導入した動機（BUG-47/BUG-19）と全く同じ構造の
問題が、open/close 軸にも存在していたことになる。

**修正:** `conv_mode_policy = force` のときは `apply_force_on_for_imm_broken()`
の `applied` スロットルを無視し、500ms ごとに無条件で idempotent な
`VK_IME_ON` 系キーを再送するようにした。conv モードの `desired_mode` 強制と
同じ設定・同じ設計意図（観測を信じず awase 自身の意図を権威にする）を
再利用しており、新しい config 項目は追加していない。

**未対応:** `applied` がそもそもなぜ「実態と異なるまま成功」と誤記録され得るか
（今回のユーザー報告の根本トリガー）は未解明のまま。本追記は「検出・訂正できない」
という構造的な穴を塞ぐ対症的な対策であり、`applied` 誤記録自体の発生経路の
調査は今後の課題。実機での動作確認は未実施。

## 関連ファイル

`src/config.rs`（`ConvModePolicy`）、`src/engine/conv.rs`（`ConvMode::to_conv_bits`）、
`crates/awase-windows/src/state/conv_mode.rs`（`ConvModeMgr::desired_mode`/`policy`）、
`crates/awase-windows/src/runtime/message_handlers.rs`（`set_desired_conv_mode`）、
`crates/awase-windows/src/tsf/warmup/cold_warmup.rs`（`run_start`）、
`crates/awase-windows/src/app/bootstrap.rs`・`runtime/mod.rs`（起動時/reload 時の
`set_policy` 配線、`apply_force_on_for_imm_broken` の force 分岐）、
`crates/awase-settings/src/main.rs`（`tab_advanced` UI）。

## 関連 ADR

- ADR-078: IME conv-mode belief の三分割（未実装のまま提案中）— 本 ADR が対象と
  する乖離問題への、より根本的で大掛かりな解決案。本 ADR はその全面実装を待たず
  提供する軽量な opt-in 緩和策という位置づけ。
- `docs/known-bugs.md` BUG-19: 観測追従型の自己増幅ループ（本 ADR の設計が
  再現を避ける対象）。
- `docs/known-bugs.md` BUG-47: 本 ADR のきっかけとなった物理キー漏洩バグ。
