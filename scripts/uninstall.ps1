# awase uninstaller script
#
# デフォルトではユーザーデータ（config.toml / layout/）を削除しない。
# 「アンインストール→インストール」をアップグレード手順として踏んでも
# 設定・配列ファイルが失われないようにするための挙動（ADR-099 決定1）。
# 完全に消去したい場合のみ -Purge を指定すること。
param([switch]$Purge)
$ErrorActionPreference = "Stop"

$installDir = "$env:LOCALAPPDATA\awase"

# Stop running process
Get-Process -Name "awase" -ErrorAction SilentlyContinue | Stop-Process -Force
Get-Process -Name "awase-settings" -ErrorAction SilentlyContinue | Stop-Process -Force

# Remove startup registry entry
$regPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
Remove-ItemProperty -Path $regPath -Name "awase" -ErrorAction SilentlyContinue

# Remove Start Menu shortcuts
$startMenu = [Environment]::GetFolderPath("StartMenu")
Remove-Item "$startMenu\Programs\awase.lnk" -ErrorAction SilentlyContinue
Remove-Item "$startMenu\Programs\awase Settings.lnk" -ErrorAction SilentlyContinue

if ($Purge) {
    # 完全削除: config.toml・layout/（カスタム配列）も含めて丸ごと削除する。
    if (Test-Path $installDir) {
        Remove-Item -Recurse -Force $installDir
        Write-Host "Removed $installDir (including config.toml and layout/)"
    }
} else {
    # 既定: プログラム本体（exe・プログラム資産）のみ削除し、
    # config.toml・layout/ は残す。
    #
    # cache.toml（IME capability 学習・GJI CLSID 等の自動学習キャッシュ）は
    # ここでは意図的に削除しない。data/ 等と違いユーザーが編集する対象
    # ではないが、削除すると次回起動時に学習し直しになりコールドスタート
    # コストが再発生する（ユーザー体感としては config.toml 消失とは別種の
    # 「前より遅くなった」regression になりうる）。config.toml/layout/ と
    # 同様「残しておいて実害がない」側に倒し、-Purge のみ対象にする
    # （コードレビュー指摘 P7b）。
    Remove-Item "$installDir\awase.exe" -ErrorAction SilentlyContinue
    Remove-Item "$installDir\awase-settings.exe" -ErrorAction SilentlyContinue
    Remove-Item "$installDir\data" -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item "$installDir\awase.log" -ErrorAction SilentlyContinue
    Write-Host "設定・配列ファイル（config.toml・layout/）は保持されました。"
    Write-Host "完全に削除したい場合は uninstall.ps1 -Purge を実行してください。"
}

Write-Host "awase uninstalled."
Write-Host "Startup registration: removed"
Write-Host "Start Menu shortcuts: removed"
