# CPU 使用率を 1 秒周期で cpu.csv(`UTC時刻,CPU%`)に追記し続ける。e2e の run が負荷で汚れていないかを
# check_run_validity.py が後から判定するための記録(情報用)。止めるときは呼び出し側が Stop-Process する。
param([Parameter(Mandatory = $true)][string]$Out)
while ($true) {
  try {
    $v = (Get-Counter '\Processor(_Total)\% Processor Time' -ErrorAction Stop).CounterSamples[0].CookedValue
    ('{0},{1:N1}' -f (Get-Date).ToUniversalTime().ToString('o'), $v) | Add-Content -Path $Out -Encoding ascii
  } catch { Start-Sleep -Seconds 1 }
}
