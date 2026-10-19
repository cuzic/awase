# Mozc Readable(cuzic/mozc-readable の Release の MSI)を入れ、ja-JP の入力方式をそれだけにする。
# 公式 Mozc とは別名なので、Google 日本語入力(install-gji.ps1)とは別の TIP として入る。
# BUG-207: Mozc 系の TIP で awase の I/O 監視(mozc_server)と raw-tsf-literal の判定を確かめるために使う。
param(
  [string]$Repo = 'cuzic/mozc-readable',
  [string]$Tag = '',          # 空なら最新の Release
  [string]$OutDir = 'mozc-readable-msi'
)
$ErrorActionPreference = 'Stop'
$Clsid   = '{AFC94560-63DA-488E-9227-B04F3EF34B5B}'
$Profile = '{7D7B9A38-A0A4-4390-9706-509E98A8C791}'

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
if ($Tag -eq '') {
  gh release download -R $Repo -p '*.msi' -D $OutDir --clobber
} else {
  gh release download $Tag -R $Repo -p '*.msi' -D $OutDir --clobber
}
if ($LASTEXITCODE -ne 0) { Write-Host "##[error]MSI のダウンロードに失敗"; exit 1 }
$msi = (Get-ChildItem $OutDir -Filter *.msi | Select-Object -First 1).FullName
Write-Host "MSI: $msi"

$p = Start-Process msiexec.exe -ArgumentList "/i `"$msi`" /qn /norestart /l*v `"$OutDir\install.log`"" -Wait -PassThru
Write-Host "msiexec exit code: $($p.ExitCode)"
if ($p.ExitCode -ne 0 -and $p.ExitCode -ne 3010) { Write-Host "##[error]MSI のインストールに失敗"; exit 1 }

$l = New-WinUserLanguageList ja-JP
$l[0].InputMethodTips.Clear()
$l[0].InputMethodTips.Add("0411:$Clsid$Profile")
Set-WinUserLanguageList $l -Force
try { Set-WinDefaultInputMethodOverride -InputTip "0411:$Clsid$Profile" -ErrorAction Stop }
catch { Write-Host "default input method override FAILED: $($_.Exception.Message)" }
Write-Host ((Get-WinUserLanguageList | ForEach-Object { "$($_.LanguageTag): $($_.InputMethodTips -join ',')" }) -join "`n")

Get-Process ctfmon -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
Start-Process ctfmon.exe -ErrorAction SilentlyContinue
Start-Sleep -Seconds 5
exit 0
