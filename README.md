# awase — 同時打鍵キーボードリマッパー

**awase**（合わせ）は、NICOLA 親指シフトに対応した同時打鍵キーボードリマッパーです。
文字キーと親指キーを同時に押すことで、ピアノのコードのように異なる文字を出力します。

## 概要

Windows 上で動作し、低レベルキーボードフックにより物理キー入力を横取りして、
NICOLA 配列に基づくかな文字入力をリアルタイムで行います。
やまぶき互換の `.yab` レイアウトファイルに対応しており、カスタム配列も利用可能です。

## 特徴

- **5 つの確定モード** — wait / speculative / two\_phase / adaptive\_timing / ngram\_predictive
- **n-gram 適応閾値** — 日本語文字頻度に基づく同時打鍵判定ウィンドウの自動調整
- **やまぶき互換 `.yab` 設定** — 物理キー位置ベース × ローマ字出力の CSV 形式
- **NICOLA 準拠 3 キー仲裁** — d1/d2 比較による正確な同時打鍵判定
- **IME 統合** — TSF + IMM32 ハイブリッド検出、IME OFF 時の自動バイパス
- **システムトレイ常駐** — 配列切替・ホットキートグル・設定画面起動
- **フォーカス自動検出** — テキスト入力コントロールを自動判定し、不要な変換を回避
- **timed-fsm** — 再利用可能なタイマー付き有限状態機械フレームワーク（ワークスペースクレート）

## 動作環境

- Windows 10 / 11
- Rust 1.70+

## インストール

```sh
cargo build --release --target x86_64-pc-windows-gnu
```

ビルド後、`target/x86_64-pc-windows-gnu/release/awase.exe` が生成されます。

## 使い方

1. `awase.exe` と同じディレクトリに `config.toml` と `layout/` フォルダを配置
2. `awase.exe` を起動するとシステムトレイに常駐
3. **Ctrl+Shift+F12** でエンジン ON/OFF を切替
4. **Ctrl+Shift+F11** でフォーカスオーバーライド（テキスト入力/バイパスの手動切替）
5. トレイアイコン右クリックでメニュー表示
   - レイアウト切替
   - 設定画面起動
   - 有効/無効切替
   - 終了

## 設定 (config.toml)

```toml
[general]
simultaneous_threshold_ms = 100   # 同時打鍵判定の閾値（ミリ秒）
toggle_hotkey = "Ctrl+Shift+F12"
layouts_dir = "layout"
default_layout = "nicola.yab"     # 起動時に読み込む配列

[engine]
confirm_mode = "adaptive_timing"  # 確定モード
# wait              — タイマー満了まで待機して確定
# speculative       — 投機的に確定し、誤りなら取消・再送
# two_phase         — 2 段階判定（高速確定 + 遅延補正）
# adaptive_timing   — 打鍵速度に応じて閾値を動的調整
# ngram_predictive  — n-gram 頻度に基づく予測確定

speculative_delay_ms = 50         # speculative / two_phase 用の遅延

[focus_overrides]
# 特定アプリケーションの動作を強制指定
force_text = ["notepad.exe", "code.exe"]    # 常にテキスト入力モード
force_bypass = ["cmd.exe", "powershell.exe"] # 常にバイパスモード
```

## レイアウトファイル (.yab)

やまぶき互換の CSV 形式で配列を定義します。

```
[ローマ字シフト無し]
．,ｋａ,ｔａ,ｋｏ,ｓａ,ｒａ,ｔｉ,ｋｕ,ｔｕ,'，',，,無
ｕ,ｓｉ,ｔｅ,ｋｅ,ｓｅ,ｈａ,ｔｏ,ｋｉ,ｉ,ｎｎ,後,逃
...

[ローマ字左親指シフト]
...

[ローマ字右親指シフト]
...
```

各セクションは物理キー位置に対応するローマ字出力を定義します。
`layout/` ディレクトリに `.yab` ファイルを配置すると、トレイメニューから選択可能になります。

## フォーカス判定

awase は現在フォーカスしているコントロールがテキスト入力欄かどうかを自動判定します。

1. **Phase 1** — ウィンドウクラス名による即時判定（Edit, RichEdit 等）
2. **Phase 2** — MSAA (Accessible Role) による同期判定
3. **Phase 3** — UI Automation による非同期判定（別スレッド）
4. **ヒューリスティック** — タイピングパターンの統計的分析による推定
5. **手動オーバーライド** — Ctrl+Shift+F11 で即時切替、学習キャッシュに記録

`config.toml` の `[focus_overrides]` でアプリケーション単位の強制指定も可能です。

## テスト

```sh
cargo test --lib                  # ユニットテスト
cargo test --test scenarios       # シナリオテスト
cargo test -p timed-fsm           # timed-fsm フレームワークテスト
```

## ライセンス

[Apache License, Version 2.0](LICENSE-APACHE) または [MIT License](LICENSE-MIT) のいずれかを選択できます。
