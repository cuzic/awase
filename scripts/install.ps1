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
Copy-Item "layout\*" "$installDir\layout\" -Force
Copy-Item "data\*" "$installDir\data\" -Force

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
