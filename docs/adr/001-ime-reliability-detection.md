# ADR-001: UIA FrameworkId ベースの IME 信頼度判定

## ステータス
採用

## コンテキスト
Windows 11 メモ帳 (WinUI 3) や Windows Terminal (XAML Islands) では、`ImmGetDefaultIMEWnd` + `WM_IME_CONTROL / IMC_GETOPENSTATUS` による IME ON/OFF 検出が実際の TSF IME 状態を反映しない。互換レイヤーのハンドルは返るが、常に `open=0` を返す。

当初はウィンドウクラス名 (`RichEditD2DPT`, `Windows.UI.Input.InputSite.WindowClass`) のハードコードで判定していたが、これは脆弱で網羅性がない。

## 決定
UIA (UI Automation) の `FrameworkId` プロパティを使って、IME クロスプロセス検出の信頼度を3段階で分類する:

- **Reliable** (`"Win32"`, `"WinForm"`) — CrossProcess 結果を信頼
- **Unreliable** (`"DirectUI"`, `"XAML"`, `"WPF"`) — CrossProcess=false を信頼しない
- **Unknown** (`"Chrome"`, `"Qt"`, 空文字等) — CrossProcess=false を信頼しない

Unreliable/Unknown の場合は shadow state（半角/全角キーの追跡）にフォールバックする。

## 結果
- Modern UI アプリ（メモ帳、Terminal、Chrome、LINE）全てで正しく動作
- クラス名のハードコードが不要に
- UIA 非同期判定のため、フォーカス変更直後の数ms は Unknown → 同期フォールバックで対応

## 関連コミット
`bd46d3a`, `946f1fc`
