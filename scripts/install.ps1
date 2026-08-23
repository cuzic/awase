# awase installer script for ZIP distribution
$ErrorActionPreference = "Stop"

$installDir = "$env:LOCALAPPDATA\awase"

# Create directories
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
New-Item -ItemType Directory -Force -Path "$installDir\layout" | Out-Null
New-Item -ItemType Directory -Force -Path "$installDir\data" | Out-Null

# Copy files
Copy-Item "awase.exe" "$installDir\" -Force
Copy-Item "awase-settings.exe" "$installDir\" -Force

# layout/: 既存なら上書きしない（awase-settings の配列編集タブが
# その場で上書き保存するユーザーデータのため。ADR-099 決定2）。
function Copy-IfAbsent($sourceGlob, $destDir) {
    New-Item -ItemType Directory -Force -Path $destDir | Out-Null
    if (Test-Path $sourceGlob) {
        Get-ChildItem $sourceGlob | ForEach-Object {
            $dest = Join-Path $destDir $_.Name
            if (-not (Test-Path $dest)) {
                Copy-Item $_.FullName $dest
            }
        }
    }
}
Copy-IfAbsent "layout\*" "$installDir\layout"

# data/: ユーザーが編集する手段を持たないプログラム資産のため、
# 従来通り常に上書きする（ADR-099 決定2、MSI の NgramData コンポーネントと
# 挙動を揃える）。$ErrorActionPreference = "Stop" 下でソースが空/不在だと
# terminating error になるため Test-Path でガードする。
if (Test-Path "data\*") {
    Copy-Item "data\*" "$installDir\data\" -Force
}

# Config: don't overwrite if exists
if (-not (Test-Path "$installDir\config.toml")) {
    Copy-Item "config.toml" "$installDir\" -Force
}

# Register startup
$regPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
Set-ItemProperty -Path $regPath -Name "awase" -Value "$installDir\awase.exe"

# Create Start Menu shortcut
$shell = New-Object -ComObject WScript.Shell
$startMenu = [Environment]::GetFolderPath("StartMenu")
$shortcut = $shell.CreateShortcut("$startMenu\Programs\awase.lnk")
$shortcut.TargetPath = "$installDir\awase.exe"
$shortcut.WorkingDirectory = $installDir
$shortcut.Save()

$settingsShortcut = $shell.CreateShortcut("$startMenu\Programs\awase Settings.lnk")
$settingsShortcut.TargetPath = "$installDir\awase-settings.exe"
$settingsShortcut.WorkingDirectory = $installDir
$settingsShortcut.Save()

Write-Host "awase installed to $installDir"
Write-Host "Startup registration: OK"
Write-Host "Start Menu shortcuts: OK"
