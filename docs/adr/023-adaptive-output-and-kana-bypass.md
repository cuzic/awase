# ADR 023: アプリ適応出力とかな入力バイパス

## ステータス

承認済み（実装完了）

## コンテキスト

awase は NICOLA 親指シフトの変換結果をアプリケーションに送信するが、アプリの UI フレームワークと IME の入力方式によって最適な出力方法が異なる。

### 問題 1: Chrome での全角記号問題

Chrome/Electron は `KEYEVENTF_UNICODE` で全角 ASCII 記号（？、！、数字等）を送ると半角に変換してしまう。
Win32 クラシックアプリや UWP アプリでは `KEYEVENTF_UNICODE` で問題なく全角文字が入力される。

### 問題 2: IME かな入力モード

IME がかな入力方式（JIS かな）に設定されている場合、awase のローマ字出力が誤変換される。
ユーザーがローマ字入力 ⇔ かな入力を切り替えて使いたいケースがある。

## 決定

### AppKind による適応出力

`AppKind` 列挙型でアプリを 3 分類し、**Chrome だけを特別扱い**する。

```rust
enum AppKind {
    Win32,   // クラシック Win32 / WinForms
    Chrome,  // Chromium 系（Chrome, Edge, Electron, VS Code, Firefox）
    Uwp,     // UWP / XAML / DirectUI / WPF
}
```

#### 出力方式

Chrome だけが VK キーストローク方式を使い、Win32 と UWP は同じ Unicode 直接出力方式を使う。

| | KeyAction::Char | KeyAction::KeySequence | KeyAction::Romaji |
|---|---|---|---|
| **Chrome** | VK（`send_char_as_vk`） | VK（`send_char_as_vk`） | ローマ字 VK（変更なし） |
| **Win32/UWP** | Unicode 直接 | Unicode 直接 | ローマ字 Unicode（変更なし） |

#### Chrome VK モードの send_char_as_vk

Chrome 用の VK 出力は 3 段階のフォールバックで全文字をカバーする:

1. **かな → ローマ字逆引き** — `kana_to_romaji` テーブル（`か` → `"ka"` → VK(k), VK(a) → IME が変換）
2. **記号・数字 → VK テーブル** — `symbol_to_vk` テーブル（半角・全角の数字、ASCII 記号、日本語句読点すべて）
3. **フォールバック → Unicode** — テーブルにない文字は Unicode 直接出力

`symbol_to_vk` テーブルは JIS キーボード前提で以下を網羅:
- 半角数字 `0-9`、全角数字 `０-９`
- 半角 ASCII 記号全種（`!@#$%^&*()_+-=[]{}|;:'"<>,.?/~\``）
- 全角 ASCII 記号全種（`！＠＃＄％…`）
- 日本語句読点（`、。・「」ー～`）
- 全角マイナス `－`、全角スラッシュ `／` 等

#### 検出方法

**同期（フォーカス変更時）:** ウィンドウクラス名から判定
- `Chrome_` で始まるクラス（`Chrome_WidgetWin_1`, `Chrome_RenderWidgetHostHWND`）, `MozillaWindowClass` → Chrome
- `Windows.UI.Core.CoreWindow`, `ApplicationFrameWindow`, `Windows.UI.Input.*` → Uwp
- その他 → Win32

**非同期（UIA）:** `FrameworkId` から補正
- `"Win32"`, `"WinForm"` → Win32
- `"XAML"`, `"DirectUI"`, `"WPF"` → Uwp
- `"Chrome"` → app_kind=None（クラス名判定を維持）

### かな入力バイパス

IME の入力方式を `IME_CMODE_ROMAN` フラグで検出する。

- `IME_CMODE_ROMAN` あり → ローマ字入力 → awase 有効
- `IME_CMODE_ROMAN` なし → かな入力の可能性 → `ImmGetConversionStatus` で直接再確認
- 判定不能 → 安全側（ローマ字）にフォールバック

Google 日本語入力はクロスプロセス検出（`WM_IME_CONTROL`）で `IME_CMODE_ROMAN` を返さないことが判明。
`ImmGetConversionStatus` による直接チェックでも `None` が返る（Chrome 等の別プロセスウィンドウでは IME コンテキストを取得できない）。
このため、現在のところかな入力バイパスは Google 日本語入力では発動しない（安全側にフォールバック）。

## 実装の詳細

### ファイル構成

| ファイル | 役割 |
|----------|------|
| `src/types.rs` | `AppKind { Win32, Chrome, Uwp }` 列挙型 |
| `src/kana_table.rs` | `build_kana_to_romaji()` 逆引きテーブル |
| `crates/awase-windows/src/lib.rs` | `APP_KIND`, `IME_IS_KANA_INPUT` atomic |
| `crates/awase-windows/src/output.rs` | `send_char_as_vk()`, `build_symbol_to_vk()`, AppKind 分岐 |
| `crates/awase-windows/src/ime.rs` | `detect_kana_input_method()`, `detect_kana_direct()` |
| `crates/awase-windows/src/hook.rs` | かな入力バイパスチェック |
| `crates/awase-windows/src/observer/focus_observer.rs` | `classify_app_kind()` |
| `crates/awase-windows/src/focus/uia.rs` | UIA FrameworkId → AppKind |

### ログ出力

`awase.log` ファイルに出力（`#![windows_subsystem = "windows"]` のため stderr は不可）。
`RUST_LOG=debug` で AppKind 判定、VK/Unicode ディスパッチ、かな入力検出の詳細が確認できる。

## 結果

### メリット

- Chrome での全角記号・数字問題が自動で解消される
- ユーザーはアプリごとの出力方式を意識する必要がない
- かな入力モードと NICOLA を IME 設定切り替えだけで行き来できる（将来的に）
- 既存の UIA フォーカス検出インフラを再利用
- シンプルな方針: Chrome だけ特別扱い、他は全部 Unicode

### デメリット

- Chrome 用の VK マッピングテーブルが JIS キーボード固定（US キーボードでは記号が異なる）
- かな入力検出は Google 日本語入力では実質未対応（`IME_CMODE_ROMAN` が取得不能）
- `Chrome_RenderWidgetHostHWND` のクラス名が将来変更される可能性

### 代替案（不採用）

- **全アプリ VK 方式:** UWP で VK が正しく処理されない
- **全アプリ Unicode 方式:** Chrome で全角記号が半角になる
- **config で手動指定:** ユーザー負担が大きい。自動検出で十分な精度が出る
- **かな入力時にローマ字モードに強制切替:** ユーザーの IME 設定を勝手に変更するのは望ましくない
