# 字形の見本の撮影: 自前の窓に GJI のモードキーを順に送りながら、入力インジケーターを撮影する(awase は止めて、終わったら再開する)。
# 保存するのは、インジケーターの小さな矩形のグレースケール画像(PGM)と、重複を除いて拡大した一覧画像だけ。他のアプリの画面は撮らない。
param([string]$Tag = 'light100', [string]$Keys = 'F2,F1,F3,F4,F0,1A', [int]$N = 2)
$ErrorActionPreference = 'Continue'
$Out = "C:\Users\cuzic\glyph-seed\$Tag"
Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$bin = 'C:\Users\cuzic\awase-glyph\target\debug\examples'
function Log($m) { Add-Content -Path "$Out\run.log" -Value ("[" + [DateTime]::UtcNow.ToString('HH:mm:ss') + "Z] " + $m) -Encoding utf8 }
$aw = Get-Process awase -ErrorAction SilentlyContinue | Select-Object -First 1
$awPath = $null; if ($aw) { $awPath = $aw.Path }
try {
  Stop-Process -Name awase -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
  $p = Start-Process -FilePath "$bin\ime_indicator_probe.exe" -ArgumentList "--log=$Out\probe.log", "--duration=70", "--interval=50", "--raw=$Out" -PassThru -WindowStyle Hidden
  Start-Sleep -Seconds 3
  Log "probe pid=$($p.Id)"
  & "$bin\indicator_scenario.exe" --scen=cycle --n=$N "--keys=$Keys" "--log=$Out\scen.log" | Out-Null
  Start-Sleep -Seconds 3
  if (-not $p.HasExited) { $p.WaitForExit(40000) | Out-Null }
  Log "pgm=$((Get-ChildItem $Out -Filter *.pgm -ErrorAction SilentlyContinue).Count)"
} finally {
  & "$bin\indicator_scenario.exe" --scen=cycle --n=0 "--log=$Out\restore.log" | Out-Null
  if ($awPath) { Start-Process -FilePath $awPath -WorkingDirectory (Split-Path $awPath) -WindowStyle Hidden; Log 'awase 再開' }
}
# 一覧画像: 重複を除いた PGM(ファイル名のハッシュで判定)を、時刻順に 8 倍へ拡大して並べる
Add-Type -AssemblyName System.Drawing
$files = Get-ChildItem $Out -Filter *.pgm | Sort-Object Name
$seen = @{}; $uniq = @()
foreach ($f in $files) { $h = ($f.BaseName -split '-')[1]; if (-not $seen.ContainsKey($h)) { $seen[$h] = 1; $uniq += $f } }
Log "unique=$($uniq.Count)"
$cell = 8; $cols = 4; $pad = 16; $lab = 18
$imgs = @()
foreach ($f in $uniq) {
  $b = [IO.File]::ReadAllBytes($f.FullName)
  # P5\nW H\n255\n
  $i = 0; $tok = @(); $cur = ''
  while ($tok.Count -lt 4 -and $i -lt $b.Length) { $c = [char]$b[$i]; $i++; if ($c -match '\s') { if ($cur) { $tok += $cur; $cur = '' } } else { $cur += $c } }
  $w = [int]$tok[1]; $h = [int]$tok[2]
  $imgs += , @($f.BaseName, $w, $h, $b, $i)
}
if ($imgs.Count -gt 0) {
  $mw = ($imgs | ForEach-Object { $_[1] } | Measure-Object -Maximum).Maximum
  $mh = ($imgs | ForEach-Object { $_[2] } | Measure-Object -Maximum).Maximum
  $rows = [Math]::Ceiling($imgs.Count / $cols)
  $sheet = New-Object System.Drawing.Bitmap ($cols * ($mw * $cell + $pad)), ($rows * ($mh * $cell + $pad + $lab))
  $g = [System.Drawing.Graphics]::FromImage($sheet); $g.Clear([System.Drawing.Color]::White)
  $font = New-Object System.Drawing.Font('Arial', 9)
  for ($k = 0; $k -lt $imgs.Count; $k++) {
    $name, $w, $h, $b, $off = $imgs[$k]
    $cx = ($k % $cols) * ($mw * $cell + $pad); $cy = [Math]::Floor($k / $cols) * ($mh * $cell + $pad + $lab)
    for ($y = 0; $y -lt $h; $y++) { for ($x = 0; $x -lt $w; $x++) {
      $v = $b[$off + $y * $w + $x]; $col = [System.Drawing.Color]::FromArgb($v, $v, $v)
      $br = New-Object System.Drawing.SolidBrush $col
      $g.FillRectangle($br, $cx + $x * $cell, $cy + $lab + $y * $cell, $cell, $cell); $br.Dispose()
    } }
    $g.DrawString("#$k $($name.Substring(0,6))", $font, [System.Drawing.Brushes]::Black, $cx, $cy)
  }
  $sheet.Save("$Out\sheet.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $sheet.Dispose()
  Log 'sheet.png 保存'
}
Log '完了'
