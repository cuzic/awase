# awase — 親指シフト（NICOLA）キーボードリマッパー

*[English](README.en.md)*

**awase**（合わせ）は、Windows で親指シフト入力を実現するキーボードリマッパーです。

---

## 親指シフトとは

親指シフト（NICOLA 配列）は、スペースバー両隣の「変換」「無変換」キーを親指シフトキーとして使い、文字キーと同時押しすることでかな文字を直接入力する方式です。ローマ字入力より少ないキー操作で日本語を入力でき、習得後は高速・高効率なタイピングが可能です。

awase は低レベルキーボードフックで物理キー入力を横取りし、同時打鍵を検出して IME にローマ字として送信します。IME は通常どおり漢字変換します。

---

## 特徴

- **NICOLA 準拠の同時打鍵判定** — d1/d2 比較による 3 キー仲裁
- **5 つの確定モード** — wait / speculative / two\_phase / adaptive\_timing / ngram\_predictive
- **n-gram 適応閾値** — Wikipedia コーパス由来の 2/3-gram で判定ウィンドウを動的調整し精度向上
- **やまぶき互換 `.yab` 配列ファイル** — 既存の配列データをそのまま利用可能
- **幅広いアプリ対応** — Win32 / UWP / TSF ネイティブ（Chrome・VS Code・WezTerm 等）を自動識別
- **多重耐障害設計** — フック死活監視・スリープ復帰・IME 検出失敗フォールバック・TSF コールドスタート自動回復を多段装備
- **非同期アーキテクチャ** — Windows メッセージループベースの非同期エグゼキュータ、ブロッキング API は別スレッドで隔離しタイムアウト保護
- **フォーカス自動検出** — テキスト入力欄以外では変換を自動停止
- **システムトレイ常駐** — 配列切替・設定画面・有効/無効トグル
- **US配列対応** — `keyboard_model = "us"` で US 物理配列に切替。無変換/変換キーが無いぶん、左右 Alt キーへの親指キーなりすまし・Space 親指キー化にも対応

技術的な設計の詳細は [ARCHITECTURE.md](ARCHITECTURE.md) を参照してください。

---

## 動作環境

- Windows 10 / 11（64 ビット）
- Google 日本語入力（推奨）または MS-IME
- Rust 1.85 以上（ビルド時のみ）

---

## クイックスタート

### 1. ビルド

```sh
cargo build --release --target x86_64-pc-windows-msvc
```

生成物: `target/x86_64-pc-windows-msvc/release/awase.exe`

### 2. ファイル配置

以下の構成で配置します。

```
awase.exe
config.toml          ← 設定ファイル
layout/
  nicola.yab         ← NICOLA 配列（同梱）
data/
  ngram_hiragana.csv.gz  ← n-gram コーパス（任意）
```

### 3. 起動

`awase.exe` をダブルクリックするとシステムトレイに常駐します。

### 4. エンジンを ON にする

デフォルトのキーバインド：

| 操作 | キー |
|------|------|
| エンジン ON | **Ctrl+Shift+変換** |
| エンジン OFF | **Ctrl+Shift+無変換** |
| IME ON | **Ctrl+変換**（IME が既に ON の場合はひらがな・ローマ字・CapsLock OFF へリセット） |
| IME OFF | **Ctrl+無変換** |
| IME-ON 半角英数トグル（MS-IME のみ） | **左Shift 単独タップ**（他キーを介さず押して離す。もう一度タップで解除） |
| アプリ別動作を手動切替 | **Ctrl+Shift+F11** |

> トレイアイコンを右クリック → 「設定」から GUI で変更できます。

### 5. 親指キーの確認

デフォルトは「無変換」が左親指、「変換」が右親指です。  
`config.toml` の `left_thumb_key` / `right_thumb_key` で変更できます。

---

## 設定ファイル (config.toml)

最小構成：

```toml
[general]
simultaneous_threshold_ms = 100   # 同時打鍵判定の閾値（ms）。NICOLA 規格は 100ms
left_thumb_key  = "無変換"
right_thumb_key = "変換"
layouts_dir     = "layout"
default_layout  = "nicola.yab"
```

フルサンプルは同梱の `config.toml` を参照してください。

### 主なオプション

| キー | デフォルト | 説明 |
|------|-----------|------|
| `simultaneous_threshold_ms` | 100 | 同時打鍵と判定する時間幅（ms） |
| `left_thumb_key` | `無変換` | 左親指シフトキー |
| `right_thumb_key` | `変換` | 右親指シフトキー |
| `confirm_mode` | `wait` | 確定モード（後述） |
| `output_mode` | `unicode` | 出力方式（通常は変更不要） |
| `engine_toggle_hotkey` | なし | エンジン ON/OFF トグルホットキー |
| `keyboard_model` | `jis` | 物理キーボード配列。US 配列なら `"us"`（`default_layout` も `nicola_us.yab` に変更） |

### 確定モード

| モード | 特徴 |
|--------|------|
| `wait` | タイムアウトまで待機。最も正確、わずかに遅延あり |
| `speculative` | 即座に出力し誤りなら取消・再送。高速だがちらつきあり |
| `two_phase` | 短い待機後に投機出力。wait と speculative の中間 |
| `adaptive_timing` | 打鍵速度に応じて自動調整 |
| `ngram_predictive` | Wikipedia 由来の n-gram 統計で閾値を動的調整（n-gram ファイル推奨） |

迷ったら `wait` から始め、遅延が気になったら `adaptive_timing` を試してください。

n-gram の仕組みの詳細は [ARCHITECTURE.md](ARCHITECTURE.md#n-gram-による同時打鍵判定の精度向上) を参照してください。

### アプリ別設定 ([app_overrides])

特定アプリで動作が合わない場合に強制指定します。

```toml
[app_overrides]
# 常にテキスト入力として扱う
force_text = [
    { process = "myapp.exe", class = "Edit" },
]
# エンジンを常に無効にする
force_bypass = [
    { process = "launcher.exe", class = "LauncherClass" },
]
# TSF ネイティブモード（WezTerm 等）
force_tsf = [
    { process = "wezterm-gui.exe", class = "org.wezfurlong.wezterm" },
]
```

プロセス名とクラス名は `RUST_LOG=debug awase.exe` のログで確認できます。

---

## 配列ファイル (.yab)

やまぶき互換の CSV 形式で配列を定義します。`layout/` に `.yab` ファイルを置くとトレイメニューから切り替えられます。

設定画面（`awase-settings.exe`）の「配列編集」タブから、テキストエディタで CSV を直接編集する代わりに、キーボード風グリッドをクリックしてビジュアルに編集・保存もできます。

```
; コメント行はセミコロンで始める
[ローマ字シフト無し]
'。',ka,ta,ko,sa, ra,ti,ku,tu,'，','、',無
u, si,te,ke,se, ha,to,ki, i, nn, 後, 逃
...

[ローマ字左親指シフト]
...

[ローマ字右親指シフト]
...
```

NICOLA 標準配列は `layout/nicola.yab`（JIS 配列）と `layout/nicola_us.yab`（US 配列）として同梱しています。US 配列では無変換/変換キーが物理的に無いため、設定画面で左右 Alt キーを親指キーとしてなりすまさせる、または Space キーを親指キーに割り当てることができます。

---

## アプリ対応

awase はフォーカス中のアプリを自動識別し、出力方式を切り替えます。手動設定は不要です。

| アプリ種別 | 例 | 出力方式 |
|-----------|-----|---------|
| Win32 / WinForms | メモ帳、Word、Excel | Unicode 直接注入 |
| TSF ネイティブ | Chrome, Edge, VS Code, WezTerm, Electron 系 | VK キーストローク |
| UWP / XAML | Windows ストアアプリ | Unicode 直接注入 |

識別結果はアプリのクラス名ごとに学習・キャッシュされ（`cache.toml`）、再起動後も維持されます。自動識別が合わない場合は `[app_overrides]` で手動指定できます。

---

## トラブルシューティング

**文字が入力されない / おかしな文字になる**  
→ エンジンが OFF になっている可能性。Ctrl+Shift+変換 で ON にする。

**特定アプリで動作しない**  
→ `RUST_LOG=debug awase.exe` で起動してログを確認し、`[app_overrides]` に追加。

**IME が自動で ON/OFF される**  
→ `config.toml` の `[keys.ime_detect]` でシャドウ追跡キーを確認する。

**同時打鍵の誤判定が多い**  
→ `simultaneous_threshold_ms` を 80〜120ms の範囲で調整する。

**IME や FSM が壊れた状態になった**  
→ トレイアイコンを右クリック → 「内部状態をリセット」で全内部状態を初期化できます。

---

## ライセンス

[Apache License, Version 2.0](LICENSE-APACHE) または [MIT License](LICENSE-MIT) のいずれかを選択できます。
