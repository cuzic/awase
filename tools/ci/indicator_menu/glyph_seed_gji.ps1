# 字形の見本(GJI の全 6 状態)の撮影。GJI の config1.db をカスタムキー表に一時的に差し替え、自前の窓にモードキーを送りながら
# インジケーターを撮り、終わったら元の config1.db へ戻して SHA256 の一致を確認する。awase は止めて、終わったら再開する。
# 安全: バックアップに失敗したら何も変更せずに中止する。保存するのはインジケーターの小さな矩形のグレースケール画像(PGM)だけ。
param([string]$Tag = 'light100', [int]$N = 2, [string]$Theme = 'light')
$ErrorActionPreference = 'Continue'
$Out = "C:\Users\cuzic\glyph-seed\gji-$Tag"
Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Out | Out-Null
function Log($m) { Add-Content -Path "$Out\run.log" -Value ("[" + [DateTime]::UtcNow.ToString('HH:mm:ss') + "Z] " + $m) -Encoding utf8 }
$bin = 'C:\Users\cuzic\awase-glyph\target\debug\examples'
$dir = Join-Path $env:USERPROFILE 'AppData\LocalLow\Google\Google Japanese Input'
$db = Join-Path $dir 'config1.db'
$bak = "$Out\config1.db.orig"
$hadDb = Test-Path $db
$origHash = $null
if ($hadDb) {
  Copy-Item $db $bak -Force
  $origHash = (Get-FileHash $db -Algorithm SHA256).Hash
  if ((Get-FileHash $bak -Algorithm SHA256).Hash -ne $origHash) { Log 'バックアップの SHA256 が一致しない。中止'; exit 1 }
  Log "バックアップ済み sha256=$origHash"
} else { Log 'config1.db は元から無い(復元時は削除する)' }
function Restart-Gji {
  Get-Process | Where-Object { $_.Name -match 'googleime|mozc' } | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2
  Get-Process ctfmon -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 1
  Start-Process ctfmon.exe -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 5
}
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class ThemeX {
  [DllImport("user32.dll", CharSet=CharSet.Auto)] static extern IntPtr SendMessageTimeout(IntPtr h, uint m, UIntPtr w, string l, uint f, uint t, out UIntPtr r);
  public static void Broadcast() { UIntPtr r; SendMessageTimeout((IntPtr)0xFFFF, 0x1A, UIntPtr.Zero, "ImmersiveColorSet", 2, 5000, out r); }
}
"@
$pp = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
$origTheme = Get-ItemProperty $pp
$aw = Get-Process awase -ErrorAction SilentlyContinue | Select-Object -First 1
$awPath = $null; if ($aw) { $awPath = $aw.Path }
try {
  Stop-Process -Name awase -Force -ErrorAction SilentlyContinue
  if ($Theme -eq 'dark') {
    Set-ItemProperty $pp -Name AppsUseLightTheme -Value 0
    Set-ItemProperty $pp -Name SystemUsesLightTheme -Value 0
    [ThemeX]::Broadcast(); Start-Sleep -Seconds 4; Log 'ダークテーマにした'
  }
  # カスタムキー表: F5=ひらがな F6=全角カタカナ F7=全角英数 F8=半角カタカナ F9=半角英数 F12=IME OFF(直接入力)
  $rows = @('DirectInput,F5,InputModeHiragana', 'Precomposition,F5,InputModeHiragana', 'DirectInput,F6,InputModeFullKatakana', 'Precomposition,F6,InputModeFullKatakana',
    'DirectInput,F7,InputModeFullAlphanumeric', 'Precomposition,F7,InputModeFullAlphanumeric', 'DirectInput,F8,InputModeHalfKatakana', 'Precomposition,F8,InputModeHalfKatakana',
    'DirectInput,F9,InputModeHalfAlphanumeric', 'Precomposition,F9,InputModeHalfAlphanumeric', 'Precomposition,F12,IMEOff', 'Composition,F12,IMEOff', 'Conversion,F12,IMEOff') | ForEach-Object { $_ -replace ',', "`t" }
  $tbl = [Text.Encoding]::UTF8.GetBytes("status`tkey`tcommand`n" + ($rows -join "`n") + "`n")
  $bytes = [System.Collections.Generic.List[byte]]::new()
  $bytes.AddRange([byte[]](0xB0, 0x01, 0x00, 0xC8, 0x02, 0x00))   # check_default=0, session_keymap=0(CUSTOM)
  $bytes.AddRange([byte[]](0xD2, 0x02))
  $len = $tbl.Length
  while ($len -ge 0x80) { $bytes.Add([byte](($len -band 0x7F) -bor 0x80)); $len = $len -shr 7 }
  $bytes.Add([byte]$len)
  $bytes.AddRange($tbl)
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  [IO.File]::WriteAllBytes($db, $bytes.ToArray())
  Log 'カスタムキー表を書いた'
  Restart-Gji
  $p = Start-Process -FilePath "$bin\ime_indicator_probe.exe" -ArgumentList "--log=$Out\probe.log", "--duration=90", "--interval=50", "--raw=$Out" -PassThru -WindowStyle Hidden
  Start-Sleep -Seconds 3
  Log "probe pid=$($p.Id)"
  # 7B(F12 → 閉) の後に F5〜F9(0x74〜0x78)。2 周。
  & "$bin\indicator_scenario.exe" --scen=cycle --n=$N "--keys=7B,74,75,76,77,78,7B" "--log=$Out\scen.log" | Out-Null
  Start-Sleep -Seconds 3
  if (-not $p.HasExited) { $p.WaitForExit(60000) | Out-Null }
  Log "pgm=$((Get-ChildItem $Out -Filter *.pgm -ErrorAction SilentlyContinue).Count)"
} finally {
  # 復元
  Set-ItemProperty $pp -Name AppsUseLightTheme -Value $origTheme.AppsUseLightTheme
  Set-ItemProperty $pp -Name SystemUsesLightTheme -Value $origTheme.SystemUsesLightTheme
  [ThemeX]::Broadcast(); Start-Sleep -Seconds 3
  if ($hadDb) { Copy-Item $bak $db -Force } else { Remove-Item $db -Force -ErrorAction SilentlyContinue }
  Restart-Gji
  if ($hadDb) {
    $now = (Get-FileHash $db -Algorithm SHA256).Hash
    Log "復元 sha256=$now 一致=$($now -eq $origHash)"
  } else { Log "復元: 元から無かったので削除 存在=$(Test-Path $db)" }
  & "$bin\indicator_scenario.exe" --scen=cycle --n=0 "--log=$Out\restore.log" | Out-Null
  if ($awPath) { Start-Process -FilePath $awPath -WorkingDirectory (Split-Path $awPath) -WindowStyle Hidden; Log 'awase 再開' }
  Log '完了'
}
