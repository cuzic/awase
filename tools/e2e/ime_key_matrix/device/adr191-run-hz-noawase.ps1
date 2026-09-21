# ADR-191 real-device: measure Hankaku/Zenkaku (0xF3/0xF4) real effects with awase NOT running (ASCII only).
# Every key is pressed from a known state: 1A(IME_OFF)->closed, 16(IME_ON)->open. Result: C:\Users\cuzic\adr191-hz-out\hz\spike.log
$ErrorActionPreference = 'Continue'
$wt   = 'C:\Users\cuzic\awase-adr190'
$sbin = "$wt\target\bin-tooling"
$out  = 'C:\Users\cuzic\adr191-hz-out'
$log  = 'C:\Users\cuzic\adr191-hz.log'
$done = 'C:\Users\cuzic\adr191-hz.done'
Remove-Item $done -ErrorAction SilentlyContinue
Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path "$out\hz" | Out-Null
'start ' + (Get-Date -Format o) | Out-File $log -Encoding ascii
$doneA = [regex]::Unescape('\u5168\u624b\u9806\u5b8c\u4e86')
$doneB = [regex]::Unescape('\u624b\u5b8c\u4e86')
# F3/F4 from closed and from open, twice each
$seq = '--seq=1A,F3,1A,F4,16,F3,16,F4,1A,F3,1A,F4,16,F3,16,F4'
Get-Process awase, ime_key_matrix_spike -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
Remove-Item "$sbin\ime_key_matrix_spike.log" -ErrorAction SilentlyContinue
Push-Location $sbin
Start-Process -FilePath "$sbin\ime_key_matrix_spike.exe" -ArgumentList ("--auto --hold=180 --activate-gji " + $seq) -WorkingDirectory $sbin
$deadline = (Get-Date).AddSeconds(150)
while ((Get-Date) -lt $deadline) {
  Start-Sleep -Seconds 2
  if ((Test-Path ime_key_matrix_spike.log) -and (Select-String -Path ime_key_matrix_spike.log -Pattern "($doneA|$doneB)" -Encoding utf8 -Quiet)) { break }
}
Pop-Location
Copy-Item "$sbin\ime_key_matrix_spike.log" "$out\hz\spike.log" -ErrorAction SilentlyContinue
Get-Process ime_key_matrix_spike -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
'end ' + (Get-Date -Format o) | Out-File $log -Append -Encoding ascii
'ok' | Out-File $done -Encoding ascii
