# Google 日本語入力を chocolatey で入れる。CI で choco の GoogleJapaneseInput が
# 約7分かかったうえ exited -1 で落ちる揺れがあった(e2e-ime run 37865154190)ため、
# 1回あたりに時間の上限を付けて最大 3 回やり直す。通常は約30秒。
param(
  [int]$MaxAttempts = 3,
  [int]$AttemptTimeoutSec = 240
)
$ErrorActionPreference = 'Continue'
for ($i = 1; $i -le $MaxAttempts; $i++) {
  Write-Host "GJI install attempt $i/$MaxAttempts"
  $job = Start-Job { choco install googlejapaneseinput -y --no-progress --force 2>&1 | Select-Object -Last 5; "EXIT=$LASTEXITCODE" }
  if (Wait-Job $job -Timeout $AttemptTimeoutSec) {
    $out = Receive-Job $job
    $out | ForEach-Object { Write-Host $_ }
    Remove-Job $job -Force
    if ($out -contains 'EXIT=0') { Write-Host "GJI install ok (attempt $i)"; exit 0 }
  } else {
    Write-Host "GJI install timed out after ${AttemptTimeoutSec}s"
    Stop-Job $job; Remove-Job $job -Force
    Get-Process choco, msiexec -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  }
  Start-Sleep -Seconds 10
}
Write-Host "##[error]GJI install failed after $MaxAttempts attempts"
exit 1
