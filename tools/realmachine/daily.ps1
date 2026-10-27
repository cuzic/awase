# 実機の日次確認(awase 常駐 + typing_stress)。clipwire の awase-daily ターゲットから背景で起動される。
# 結果: C:\awase-daily\summary.txt(集計)、awase.log、stress.log。実行中は実機のキーボードを奪う。
$ErrorActionPreference = 'Continue'
$out = 'C:\awase-daily'
New-Item -ItemType Directory -Force $out | Out-Null
Get-ChildItem $out | Remove-Item -Force -ErrorAction SilentlyContinue
Stop-Process -Name awase -Force -ErrorAction SilentlyContinue
git fetch origin develop 2>&1 | Out-File "$out\git.log"
git checkout --detach origin/develop 2>&1 | Out-File "$out\git.log" -Append
git rev-parse --short HEAD | Out-File "$out\commit.txt"
cargo build -p awase-windows --bin awase --example typing_stress 2>&1 | Out-File "$out\build.log"
if ($LASTEXITCODE -ne 0) { 'BUILD FAILED' | Out-File "$out\summary.txt"; exit 1 }
$env:RUST_LOG = 'debug'
$a = Start-Process -FilePath target\debug\awase.exe -RedirectStandardError "$out\awase.log" -RedirectStandardOutput "$out\awase.out" -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 5
foreach ($ime in @('--msime', '--activate-gji')) {
  cargo run -p awase-windows --example typing_stress -- --form=edit --interval=20 --layout=layout\nicola_keytop.yab $ime --settle-read 2>&1 | Out-File "$out\stress$ime.log"
}
Stop-Process -Id $a.Id -Force -ErrorAction SilentlyContinue
$log = Get-Content "$out\awase.log" -ErrorAction SilentlyContinue
$n = { param($p) ($log | Select-String -Pattern $p).Count }
@(
  "commit=$(Get-Content $out\commit.txt)",
  "effective-open-flip(MostRecentTrusted)=$(& $n 'effective-open-flip.*MostRecentTrusted')",
  "eisu-adopt candidate=$(& $n 'eisu-adopt.*candidate')",
  "eisu-candidate confirmed=$(& $n 'eisu-candidate.*confirmed')",
  "eisu-candidate cleared=$(& $n 'eisu-candidate.*cleared')",
  "eisu-candidate expired=$(& $n 'eisu-candidate.*expired')",
  "NotRomajiInput deactivations=$(& $n 'NotRomajiInput')"
) | Out-File "$out\summary.txt"
'DONE' | Out-File "$out\done.txt"
