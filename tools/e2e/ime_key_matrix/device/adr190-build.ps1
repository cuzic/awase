# ADR-190 real-device verification: build (ASCII only; PowerShell 5 reads BOM-less .ps1 as ANSI).
# Uses a separate git worktree so the user's awase checkout / running awase are not touched.
# Output: <wt>\target\bin-baseline\{awase.exe,ime_key_matrix_spike.exe} and <wt>\target\bin-a10\... (ImmCross forced to fail)
$ErrorActionPreference = 'Continue'
$main = 'C:\Users\cuzic\scoop\persist\msys2\home\cuzic\awase'
$wt   = 'C:\Users\cuzic\awase-adr190'
$log  = 'C:\Users\cuzic\adr190-build.log'
$done = 'C:\Users\cuzic\adr190-build.done'
Remove-Item $done -ErrorAction SilentlyContinue
'start ' + (Get-Date -Format o) | Out-File $log -Encoding ascii
function Log($m) { $m | Out-File $log -Append -Encoding ascii }
Set-Location $main
git fetch origin ci/e2e-scenarios 2>&1 | ForEach-Object { Log $_ }
if (Test-Path $wt) {
  Set-Location $wt
  git checkout -f --detach origin/ci/e2e-scenarios 2>&1 | ForEach-Object { Log $_ }
} else {
  git worktree add --detach $wt origin/ci/e2e-scenarios 2>&1 | ForEach-Object { Log $_ }
  Set-Location $wt
}
Log ('HEAD=' + (git rev-parse --short HEAD))
function Build-Copy($variant) {
  cargo build -p awase-windows --bin awase --example ime_key_matrix_spike 2>&1 | ForEach-Object { Log $_ }
  $dst = "$wt\target\bin-$variant"
  New-Item -ItemType Directory -Force -Path $dst | Out-Null
  Copy-Item "$wt\target\debug\awase.exe" $dst -Force
  Copy-Item "$wt\target\debug\examples\ime_key_matrix_spike.exe" $dst -Force
  Log ("copied to $dst : " + (Get-Item "$dst\awase.exe").LastWriteTime)
}
Build-Copy 'baseline'
# a10: force the ImmCross write to Failed (skip the actual write) so the fallback (MsImeDirect) is exercised.
$f = "$wt\crates\awase-windows\src\runtime\open_chain.rs"
$s = [IO.File]::ReadAllText($f)
$pat = '(?s)(let raw = )match op \{(.*?)\r?\n    \};(\r?\n\r?\n    let mut post_failed_reobservation)'
if ($s -match $pat) {
  $s2 = [regex]::Replace($s, $pat, '${1}if true { ActuationOutcome::Failed } else { match op {${2}' + "`n    } };" + '${3}')
  [IO.File]::WriteAllText($f, $s2, (New-Object Text.UTF8Encoding($false)))
  Log 'a10 mutation applied'
  Build-Copy 'a10'
  git checkout -- crates/awase-windows/src/runtime/open_chain.rs 2>&1 | ForEach-Object { Log $_ }
} else { Log 'a10 mutation pattern NOT FOUND' }
Log ('end ' + (Get-Date -Format o))
'ok' | Out-File $done -Encoding ascii
