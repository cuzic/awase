# 実機確認: #709(字形の取り込み shadow)のビルドを glyph = "observe" で一定時間動かし、journal の [indicator-glyph] の行を取り出す。
# awase を止めて動かし、終わったら元の awase を再開する。記録は大きさ・点数・所要・失敗の理由だけ(画素・画像は無い)。
param([int]$Seconds = 75)
$ErrorActionPreference = 'Continue'
$Out = 'C:\Users\cuzic\glyph-live'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$exe = 'C:\Users\cuzic\awase-menu\target\debug\awase.exe'
$aw = Get-Process awase -ErrorAction SilentlyContinue | Select-Object -First 1
$awPath = $null; if ($aw) { $awPath = $aw.Path }
Remove-Item "$Out\awase.log", "$Out\summary.txt", "$Out\run.log" -ErrorAction SilentlyContinue
function Log($m) { Add-Content -Path "$Out\run.log" -Value ("[" + [DateTime]::UtcNow.ToString('HH:mm:ss') + "Z] " + $m) -Encoding utf8 }
try {
  Stop-Process -Name awase -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
  Copy-Item $exe "$Out\awase.exe" -Force
  [IO.File]::WriteAllText("$Out\config.toml", "[general]`n[indicator_evidence]`nglyph = `"observe`"`n", (New-Object System.Text.UTF8Encoding($false)))
  $env:RUST_LOG = 'debug'
  $p = Start-Process -FilePath "$Out\awase.exe" -WorkingDirectory $Out -PassThru -WindowStyle Hidden
  Log "awase 起動 pid=$($p.Id)(元の awase=$awPath)"
  Start-Sleep -Seconds $Seconds
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
  Log 'awase 停止'
} finally {
  if ($awPath) { Start-Process -FilePath $awPath -WorkingDirectory (Split-Path $awPath) -WindowStyle Hidden; Log 'awase 再開' }
  if (Test-Path "$Out\awase.log") {
    $lines = Get-Content "$Out\awase.log" | Where-Object { $_ -match '\[indicator-glyph\]' }
    $lines | Set-Content "$Out\summary.txt" -Encoding utf8
    Log "log の [indicator-glyph] 行 = $($lines.Count)"
  } else { Log 'awase.log が無い' }
  Log '完了'
}
