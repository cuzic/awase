# WebView2 ランタイムが無い runner 向け。あれば何もしない。
$key = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
if (Test-Path $key) { "WebView2 ランタイム済み: $((Get-ItemProperty $key).pv)"; exit 0 }
Invoke-WebRequest 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile "$env:TEMP\wv2setup.exe"
Start-Process "$env:TEMP\wv2setup.exe" -ArgumentList '/silent /install' -Wait
if (Test-Path $key) { "WebView2 ランタイムを入れた: $((Get-ItemProperty $key).pv)" } else { throw 'WebView2 ランタイムの導入に失敗' }
