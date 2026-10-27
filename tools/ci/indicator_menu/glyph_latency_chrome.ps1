# 字形の遅れの実測・実 Chrome 版(ADR-273 R7: 外れ値の構成): 押下から字形が最初に変わるまで。-KeepAwase なら awase を止めずに測る。
# (元: 字形の見本の撮影。)GJI の config1.db をカスタムキー表に一時的に差し替え、自前の窓にモードキーを送りながら
# インジケーターを撮り、終わったら元の config1.db へ戻して SHA256 の一致を確認する。awase は止めて、終わったら再開する。
# 安全: バックアップに失敗したら何も変更せずに中止する。保存するのはインジケーターの小さな矩形のグレースケール画像(PGM)だけ。
param([string]$Tag = 'chrome-on', [int]$N = 6, [switch]$KeepAwase)
$ErrorActionPreference = 'Continue'
$Out = "C:\Users\cuzic\glyph-lat\$Tag"
Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Out | Out-Null
Start-Transcript -Path "$Out\transcript.txt" -Force | Out-Null
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
  if (-not $KeepAwase) { Stop-Process -Name awase -Force -ErrorAction SilentlyContinue }
  if ($false) {
    Set-ItemProperty $pp -Name AppsUseLightTheme -Value 0
    Set-ItemProperty $pp -Name SystemUsesLightTheme -Value 0
    [ThemeX]::Broadcast(); Start-Sleep -Seconds 4; Log 'ダークテーマにした'
  }
  # カスタムキー表: F5=ひらがな F6=全角カタカナ F7=全角英数 F8=半角カタカナ F9=半角英数 F12=IME OFF(直接入力)
  $rows = @('DirectInput,F13,InputModeHiragana', 'Precomposition,F13,InputModeHiragana', 'DirectInput,F14,InputModeFullKatakana', 'Precomposition,F14,InputModeFullKatakana',
    'DirectInput,F15,InputModeFullAlphanumeric', 'Precomposition,F15,InputModeFullAlphanumeric', 'DirectInput,F16,InputModeHalfKatakana', 'Precomposition,F16,InputModeHalfKatakana',
    'DirectInput,F17,InputModeHalfAlphanumeric', 'Precomposition,F17,InputModeHalfAlphanumeric', 'Precomposition,F18,IMEOff', 'Composition,F18,IMEOff', 'Conversion,F18,IMEOff') | ForEach-Object { $_ -replace ',', "`t" }
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
  $p = Start-Process -FilePath "$bin\ime_indicator_probe.exe" -ArgumentList "--log=$Out\probe.log", "--duration=$([int](12 + 11 * $N + 15))", "--interval=50", "--raw=$Out" -PassThru -WindowStyle Hidden
  Start-Sleep -Seconds 3
  Log "probe pid=$($p.Id)"
  # 7B(F18 → 閉) の後に F5〜F9(0x74〜0x78)。2 周。
  # 自前の Chrome(使い捨てのプロファイル・ローカルの空ページ・textarea)を前面にして、F12→F5〜F9→F12 を SendInput で押す。
  # 前面の窓のタイトルが自前のページでなければ押さない(利用者の窓には絶対に送らない)。
  Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class KbX {
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  public static string FgTitle() { var sb = new StringBuilder(256); GetWindowText(GetForegroundWindow(), sb, 256); return sb.ToString(); }
  public static void Tap(byte vk) { keybd_event(vk, 0, 0, UIntPtr.Zero); keybd_event(vk, 0, 2, UIntPtr.Zero); }
}
"@
  $prof = "$Out\chrome-profile"
  $page = "$Out\lat.html"
  [IO.File]::WriteAllText($page, '<!doctype html><meta charset=utf-8><title>AWASE-LAT</title><textarea id=t autofocus rows=6 cols=40></textarea><script>document.getElementById("t").focus()</script>', (New-Object System.Text.UTF8Encoding($false)))
  $chrome = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
  if (-not (Test-Path $chrome)) { $chrome = 'C:\Program Files (x86)\Google\Chrome\Application\chrome.exe' }
  if (-not (Test-Path $chrome)) { $chrome = 'C:\Users\cuzic\scoop\apps\googlechrome\current\chrome.exe' }
  $cp = Start-Process -FilePath $chrome -ArgumentList "--user-data-dir=`"$prof`"", '--no-first-run', '--no-default-browser-check', '--new-window', "file:///$($page -replace '\\','/')" -PassThru
  Start-Sleep -Seconds 6
  $pressLog = "$Out\scen.log"
  function PLog($m) { Add-Content -Path $pressLog -Value ("[" + [DateTime]::UtcNow.ToString('HH:mm:ss.fff') + "Z] " + $m) -Encoding utf8 }
  $ok = $false
  for ($i = 0; $i -lt 10; $i++) {
    $h = (Get-Process chrome -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle -like '*AWASE-LAT*' } | Select-Object -First 1).MainWindowHandle
    if ($h) { [KbX]::SetForegroundWindow($h) | Out-Null }
    Start-Sleep -Milliseconds 700
    if ([KbX]::FgTitle() -like '*AWASE-LAT*') { $ok = $true; break }
  }
  if (-not $ok) { Log '自前の Chrome を前面にできなかった。何も押さずに終了' } else {
    Log '自前の Chrome が前面'
    [KbX]::Tap(0x16); Start-Sleep -Milliseconds 800   # 'U'(最初のキーを捨てる)
    foreach ($round in 1..$N) {
      foreach ($vk in 0x81,0x7C,0x7D,0x7E,0x7F,0x80,0x81) {
        if (-not ([KbX]::FgTitle() -like '*AWASE-LAT*')) { PLog 'ABORT foreground changed'; break }
        PLog ("PRESS vk=0x{0:x2}" -f $vk)
        [KbX]::Tap([byte]$vk)
        Start-Sleep -Milliseconds 1600
      }
    }
  }
  Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" | Where-Object { $_.CommandLine -like "*glyph-lat*chrome-profile*" } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
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
  if ($awPath -and -not $KeepAwase) { Start-Process -FilePath $awPath -WorkingDirectory (Split-Path $awPath) -WindowStyle Hidden; Log 'awase 再開' }
  Log '完了'
}
