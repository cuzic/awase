# Windows側: スパイクを「毎回新しいプロセスで」N回起動し、ログを1つのファイルに追記する(1回のclipwire execで完結)。
# 起動し直す条件(新しいウィンドウ=フォーカス変更)を保つ。BUG-147は、1プロセスで連続実行するとまだ再現しなかったため。
# 引数は C:/Users/cuzic/e2e-args.txt(スパイクの引数)と e2e-loop-n.txt(回数)から読む。
$GapSec = 3
$SpikeArgs = '--auto'
if (Test-Path 'C:/Users/cuzic/e2e-args.txt') { $SpikeArgs = (Get-Content 'C:/Users/cuzic/e2e-args.txt').Trim() }
$N = 12
if (Test-Path 'C:/Users/cuzic/e2e-loop-n.txt') { $N = [int](Get-Content 'C:/Users/cuzic/e2e-loop-n.txt').Trim() }
$ErrorActionPreference = 'Continue'
$dir = 'C:\Users\cuzic\scoop\persist\msys2\home\cuzic\awase-spike\target\debug\examples'
Set-Location $dir
$multi = Join-Path $dir 'ime_key_matrix_spike.multi.log'
Remove-Item $multi, 'ime_key_matrix_spike.multi.done' -ErrorAction SilentlyContinue
Set-Content 'C:/Users/cuzic/e2e-since.txt' ((Get-Date).ToUniversalTime().ToString('HH:mm:ss'))
$enc = New-Object System.Text.UTF8Encoding($false)
for ($i = 1; $i -le $N; $i++) {
  Stop-Process -Name ime_key_matrix_spike -Force -ErrorAction SilentlyContinue
  Remove-Item 'ime_key_matrix_spike.log' -ErrorAction SilentlyContinue
  [IO.File]::AppendAllText($multi, "[RUN $i/$N 開始]`n", $enc)
  $p = Start-Process -FilePath .\ime_key_matrix_spike.exe -ArgumentList $SpikeArgs.Split(' ') -PassThru
  if (-not $p.WaitForExit(120000)) { Stop-Process -Id $p.Id -Force; [IO.File]::AppendAllText($multi, "[RUN $i/$N timeout]`n", $enc) }
  if (Test-Path 'ime_key_matrix_spike.log') { [IO.File]::AppendAllText($multi, ([IO.File]::ReadAllText((Join-Path $dir 'ime_key_matrix_spike.log'), $enc)), $enc) }
  Start-Sleep -Seconds $GapSec
}
Set-Content 'ime_key_matrix_spike.multi.done' 'done'
