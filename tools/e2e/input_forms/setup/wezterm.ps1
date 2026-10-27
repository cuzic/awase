# WezTerm (portable zip) for typing_stress --form=ext --ext=wezterm. Verification-only (ci/ branch).
# Pinned release and SHA-256 (supply chain): the hash below was computed from the downloaded zip on 2026-10-23
# and matches the .sha256 file published with the release.
$ErrorActionPreference = 'Stop'
$tag = '20240203-110809-5046fc22'
$sha = '57e5d03b585303d81e8b8e96d1230362852eb39aca92b3b29c7a42cfb82f9ac4'
$url = "https://github.com/wezterm/wezterm/releases/download/$tag/WezTerm-windows-$tag.zip"
$zip = Join-Path $env:RUNNER_TEMP 'wezterm.zip'
Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
$got = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
if ($got -ne $sha) { Write-Host "##[error]wezterm zip sha256 mismatch: $got"; exit 1 }
Expand-Archive -Path $zip -DestinationPath 'C:\wezterm' -Force
Get-ChildItem "C:\wezterm\WezTerm-windows-$tag\wezterm-gui.exe"
