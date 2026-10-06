# subosito/flutter-action で Flutter が入った後。Windows アプリを生成して lib/main.dart を差し替えビルドする。
$ErrorActionPreference = 'Stop'
$app = 'tools\e2e\input_apps\flutter\app'
flutter config --enable-windows-desktop
flutter create --platforms=windows --project-name flutter_form_input $app
Copy-Item tools\e2e\input_apps\flutter\main.dart "$app\lib\main.dart" -Force
Push-Location $app
flutter build windows --release
if ($LASTEXITCODE -ne 0) { throw 'flutter build 失敗' }
Pop-Location
