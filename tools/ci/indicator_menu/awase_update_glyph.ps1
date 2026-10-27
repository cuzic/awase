# awase-latest を develop 最新に更新し、config.toml(利用者の設定)に glyph = "observe" を足して、ビルドして再起動する。
# 利用者の config.toml は先にバックアップを取り、更新後に差し戻して追記する(SHA256 をログに残す)。
$ErrorActionPreference = 'Continue'
$repo = 'C:\Users\cuzic\awase-latest'
$Out = 'C:\Users\cuzic\awase-update'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
Remove-Item "$Out\run.log" -ErrorAction SilentlyContinue
function Log($m) { Add-Content -Path "$Out\run.log" -Value ("[" + [DateTime]::UtcNow.ToString('HH:mm:ss') + "Z] " + $m) -Encoding utf8 }
$cfgPath = "$repo\config.toml"
Copy-Item $cfgPath "$Out\config.toml.bak" -Force
$h0 = (Get-FileHash $cfgPath -Algorithm SHA256).Hash
if ((Get-FileHash "$Out\config.toml.bak" -Algorithm SHA256).Hash -ne $h0) { Log 'バックアップ不一致。中止'; exit 1 }
Log "config バックアップ済み sha256=$h0"
$aw = Get-Process awase -ErrorAction SilentlyContinue | Select-Object -First 1
$awPath = $null; if ($aw) { $awPath = $aw.Path }
Set-Location $repo
$ok = $false
try {
  Stop-Process -Name awase -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
  git stash push config.toml 2>&1 | ForEach-Object { Log "stash: $_" }
  git fetch origin develop 2>&1 | Out-Null
  git checkout --detach origin/develop 2>&1 | ForEach-Object { Log "checkout: $_" }
  Log "HEAD=$(git log --oneline -1)"
  # 利用者の設定を戻し、glyph = "observe" を足す
  Copy-Item "$Out\config.toml.bak" $cfgPath -Force
  $cfg = [IO.File]::ReadAllText($cfgPath, [Text.Encoding]::UTF8)
  if ($cfg -match '(?m)^\[indicator_evidence\]') {
    if ($cfg -match '(?m)^glyph\s*=.*$') { $cfg = [regex]::Replace($cfg, '(?m)^glyph\s*=.*$', 'glyph = "observe"') }
    else { $cfg = [regex]::Replace($cfg, '(?m)^\[indicator_evidence\][ \t]*\r?$', "[indicator_evidence]`r`nglyph = `"observe`"") }
  } else { $cfg = $cfg.TrimEnd() + "`r`n`r`n[indicator_evidence]`r`nglyph = `"observe`"`r`n" }
  [IO.File]::WriteAllText($cfgPath, $cfg, (New-Object System.Text.UTF8Encoding($false)))
  Log 'config に glyph = observe を足した'
  cargo build -p awase-windows --bin awase 2>&1 | Select-Object -Last 3 | ForEach-Object { Log "build: $_" }
  if (Test-Path "$repo\target\debug\awase.exe") { $ok = $true }
} finally {
  if ($awPath) {
    Start-Process -FilePath $awPath -WorkingDirectory (Split-Path $awPath) -WindowStyle Hidden
    Start-Sleep -Seconds 6
    $p = Get-Process awase -ErrorAction SilentlyContinue | Select-Object -First 1
    Log "awase 再起動 running=$([bool]$p) path=$awPath"
  }
  $log = "$repo\target\debug\awase.log"
  if (Test-Path $log) { Get-Content $log -Tail 400 | Where-Object { $_ -match 'indicator-glyph|Loading config' } | Select-Object -Last 4 | ForEach-Object { Log "log: $_" } }
  Log '完了'
}
