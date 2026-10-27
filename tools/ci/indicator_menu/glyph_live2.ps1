# 実機確認 2: 利用者の config.toml のコピー(字形の観測だけ足したもの)で、develop のビルドを一定時間「通常使い」で動かし、
# journal の [indicator-glyph] の行を取り出す。元の config.toml・awase は触らず、終わったら元の awase を再開する。
param([int]$Minutes = 10)
$ErrorActionPreference = 'Continue'
$Out = 'C:\Users\cuzic\glyph-live2'
Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Out | Out-Null
function Log($m) { Add-Content -Path "$Out\run.log" -Value ("[" + [DateTime]::UtcNow.ToString('HH:mm:ss') + "Z] " + $m) -Encoding utf8 }
$exeDir = 'C:\Users\cuzic\awase-menu\target\debug'
$exe = "$exeDir\awase.exe"
$srcDir = 'C:\Users\cuzic\awase-latest'
$aw = Get-Process awase -ErrorAction SilentlyContinue | Select-Object -First 1
$awPath = $null; if ($aw) { $awPath = $aw.Path }
# 利用者の設定のコピーに glyph = "observe" を足す
$cfg = [IO.File]::ReadAllText("$srcDir\config.toml", [Text.Encoding]::UTF8)
if ($cfg -match '(?m)^\[indicator_evidence\]') {
  if ($cfg -match '(?m)^glyph\s*=.*$') { $cfg = [regex]::Replace($cfg, '(?m)^glyph\s*=.*$', 'glyph = "observe"') }
  else { $cfg = [regex]::Replace($cfg, '(?m)^\[indicator_evidence\][ \t]*\r?$', "[indicator_evidence]`r`nglyph = `"observe`"") }
} else { $cfg = $cfg.TrimEnd() + "`r`n`r`n[indicator_evidence]`r`nglyph = `"observe`"`r`n" }
[IO.File]::WriteAllText("$Out\config.toml", $cfg, (New-Object System.Text.UTF8Encoding($false)))
Log 'config のコピーを作った(glyph = observe)'
# 学習済みのキャッシュ(窓の IMM の能力など)を引き継ぐ。元のものは退避する
if (Test-Path "$exeDir\cache.toml") { Copy-Item "$exeDir\cache.toml" "$Out\cache.toml.prev" -Force }
if (Test-Path "$srcDir\target\debug\cache.toml") { Copy-Item "$srcDir\target\debug\cache.toml" "$exeDir\cache.toml" -Force }
Remove-Item "$exeDir\awase.log" -ErrorAction SilentlyContinue
try {
  Stop-Process -Name awase -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
  $env:RUST_LOG = 'info,awase::journal=debug'
  $p = Start-Process -FilePath $exe -ArgumentList "`"$Out\config.toml`"" -WorkingDirectory $exeDir -PassThru -WindowStyle Hidden
  Log "テスト用の awase を起動 pid=$($p.Id)(元の awase=$awPath)。$Minutes 分動かす"
  Start-Sleep -Seconds ($Minutes * 60)
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
  Log 'テスト用の awase を停止'
} finally {
  if (Test-Path "$Out\cache.toml.prev") { Copy-Item "$Out\cache.toml.prev" "$exeDir\cache.toml" -Force }
  if ($awPath) { Start-Process -FilePath $awPath -WorkingDirectory (Split-Path $awPath) -WindowStyle Hidden; Log '元の awase を再開' }
  if (Test-Path "$exeDir\awase.log") {
    $lines = Get-Content "$exeDir\awase.log" | Where-Object { $_ -match '\[indicator-glyph\]' }
    $lines | Set-Content "$Out\summary.txt" -Encoding utf8
    Log "log の [indicator-glyph] 行 = $($lines.Count)"
  } else { Log 'awase.log が無い' }
  Log '完了'
}
