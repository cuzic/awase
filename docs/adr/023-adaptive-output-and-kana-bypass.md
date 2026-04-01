# ADR 023: アプリ適応出力とかな入力バイパス

## ステータス

承認済み

## コンテキスト

awase は NICOLA 親指シフトの変換結果をアプリケーションに送信するが、アプリの UI フレームワークと IME の入力方式によって最適な出力方法が異なる。

### 問題 1: アプリ別の出力方式

| アプリ種別 | 問題 |
|-----------|------|
| Chrome/Electron | `KEYEVENTF_UNICODE` で全角 ASCII 記号（？等）を送ると半角に変換される |
| UWP/XAML | VK キーストロークが正しく処理されない場合がある |
| Win32 クラシック | 問題なし（デフォルト動作で OK） |

### 問題 2: IME かな入力モード

IME がかな入力方式（JIS かな）に設定されている場合、awase のローマ字出力が誤変換される。ユーザーがローマ字入力 ⇔ かな入力を切り替えて使いたいケースがある。

## 決定

### AppKind による適応出力

`AppKind` 列挙型でアプリを 3 分類し、出力方式を自動切り替えする。

```rust
enum AppKind {
    Win32,   // クラシック Win32 / WinForms
    Chrome,  // Chromium 系（Chrome, Edge, Electron, VS Code）
    Uwp,     // UWP / XAML / DirectUI
}
```

| AppKind | KeyAction::Char | KeyAction::KeySequence |
|---------|----------------|----------------------|
| Win32 | Unicode 直接 | VK キーストローク |
| Chrome | VK キーストローク | VK キーストローク |
| Uwp | Unicode 直接 | Unicode 直接 |

**検出方法:**
1. **同期（フォーカス変更時）:** ウィンドウクラス名から判定
   - `Chrome_WidgetWin_1`, `MozillaWindowClass` → Chrome
   - `Windows.UI.Core.CoreWindow`, `ApplicationFrameWindow` → Uwp
   - その他 → Win32
2. **非同期（UIA）:** `FrameworkId` から補正
   - `"Win32"`, `"WinForm"` → Win32
   - `"XAML"`, `"DirectUI"`, `"WPF"` → Uwp

### かな入力バイパス

IME の入力方式を `ImmGetConversionStatus` の `IME_CMODE_ROMAN` フラグで検出する。

- `IME_CMODE_ROMAN` あり → ローマ字入力 → awase 有効
- `IME_CMODE_ROMAN` なし（かつ `IME_CMODE_NATIVE` あり） → かな入力 → awase バイパス

IME ポーリング（500ms）で検出し、`IME_IS_KANA_INPUT` atomic フラグに格納。フックコールバックの最初でチェックし、かな入力モードなら即座に `CallNextHookEx` で全キーをパススルーする。

## 結果

### メリット

- ユーザーはアプリごとの出力方式を意識する必要がない
- Chrome での全角記号問題が自動で解消される
- かな入力モードと NICOLA を IME 設定切り替えだけで行き来できる
- 既存の UIA フォーカス検出インフラを再利用

### デメリット

- AppKind の分類が不正確な場合がある（未知のフレームワーク → Win32 デフォルト）
- かな入力検出のポーリング間隔（500ms）分のラグがある
- Chromium 系と Firefox を同一カテゴリ（Chrome）にまとめている

### 代替案（不採用）

- **config で手動指定:** ユーザー負担が大きい。自動検出で十分な精度が出る。
- **かな入力時にローマ字モードに強制切替:** ユーザーの IME 設定を勝手に変更するのは望ましくない。
- **かな入力用の JIS キーコード出力:** 濁点が2キーストロークになる等の複雑さがあり、バイパスの方がシンプル。
