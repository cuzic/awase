# awase uninstaller script
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

# Remove install directory
if (Test-Path $installDir) {
    Remove-Item -Recurse -Force $installDir
    Write-Host "Removed $installDir"
}

Write-Host "awase uninstalled."
Write-Host "Startup registration: removed"
Write-Host "Start Menu shortcuts: removed"
