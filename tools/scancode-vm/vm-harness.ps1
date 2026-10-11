# Scancode Map の実機相当の検証(ドライバ層を通るキー入力)を、GitHub ホスト型の Windows ランナーの入れ子の Hyper-V VM で行う。
#
# 流れ(シナリオごと):
#   1. WinPE の boot.wim に、キー記録アプリ(probe.exe)と startnet.cmd を入れ、SYSTEM ハイブへ Scancode Map をオフラインで書く
#   2. FAT32 の起動用 VHDX に、WinPE のメディアと boot.wim を入れる
#   3. 入れ子の Gen2 VM を作って起動する
#   4. 番兵キー F11 → テストの打鍵列 → F11 の「塊」を、数秒おきに繰り返し送る(ADR-257 決定8。画面の検出に依存しない)。
#      画面がマゼンタになった(probe が F11 を受け取った)のを補助に、あと2塊送って早期に切り上げる。上限 150 秒
#   5. F12 で probe を終了させ、VM が止まったら VHDX をマウントして probe.log を読む
#   6. READY のあとで最初に完全な形でそろった塊を判定に使う。判定は LL フックの vkCode(左右の区別あり)。
#      Raw Input の MakeCode・E0 は Scancode Map より前の元の値(スパイクで確認)。
#
# 結果の種類(ADR-257 決定6・7): PASS / FAIL / INCONCLUSIVE(前提条件を欠く。VM を作り直して1回だけ再試行) / OBSERVED(kind=observe は
# 期待値なしで記録するだけ)。終了コード: 0=全 PASS(OBSERVED を含む)、1=FAIL あり、3=検証不能が残った。
param(
  [Parameter(Mandatory)] [string] $ProbeExe,
  [string] $WorkDir = 'C:\scancode_vm',
  [string] $ShotDir = 'C:\shots',
  # 塊を送り続ける秒数。WinPE の起動は約20秒(スパイクで probe が3塊目から受け取り始めた)ので、75秒は約3.5倍の余裕。足りなければ検証不能になり再試行される。
  [int] $ChunkSeconds = 75
)
$ErrorActionPreference = 'Stop'
$script:verifyExit = 0
Add-Type -AssemblyName System.Drawing
$ns = 'root/virtualization/v2'
$pe = 'C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Windows Preinstallation Environment\amd64'
New-Item -ItemType Directory -Force -Path $WorkDir, $ShotDir | Out-Null


function Get-Thumbnail([string]$vmName, [int]$w, [int]$h) {
  $svc = Get-CimInstance -Namespace $ns -ClassName Msvm_VirtualSystemManagementService
  $vm = Get-CimInstance -Namespace $ns -ClassName Msvm_ComputerSystem -Filter "ElementName='$vmName'"
  $sd = Get-CimAssociatedInstance -InputObject $vm -ResultClassName Msvm_VirtualSystemSettingData |
    Where-Object { $_.VirtualSystemType -eq 'Microsoft:Hyper-V:System:Realized' } | Select-Object -First 1
  $r = Invoke-CimMethod -InputObject $svc -MethodName GetVirtualSystemThumbnailImage -Arguments @{ TargetSystem = $sd; WidthPixels = [uint16]$w; HeightPixels = [uint16]$h }
  return [byte[]]$r.ImageData
}
# 16bit のサムネイルを 5-5-5 のフィールドとして読む(スパイクの PNG で、青と赤が逆・緑が暗く出ていたので、青が上位という仮定をやめた)。
# 5-5-5(X1R5G5B5)でも 5-6-5 でも、純粋な緑は g5>=12・r5<=3・b5<=3、マゼンタは r5>=12・b5>=12・g5<=3 になる(どちらの並びでも成り立つように選んだ閾値)。
function Get-ScreenColors([string]$vmName) {
  # 160x120 で取ったサムネイルは検出に使えなかった(常に空)。320x240 で取り、3画素おきに見る。先頭に 4 バイトの余りがある。
  $w = 320; $h = 240
  $bytes = Get-Thumbnail $vmName $w $h
  if ($null -eq $bytes) { return @{ green = 0; magenta = 0 } }
  $green = 0; $magenta = 0
  for ($i = 0; $i -lt ($w * $h); $i += 3) {
    $o = 4 + 2 * $i; $v = $bytes[$o] -bor ($bytes[$o + 1] -shl 8)
    $b5 = $v -band 0x1F; $g5 = ($v -shr 5) -band 0x1F; $r5 = ($v -shr 10) -band 0x1F
    if ($g5 -ge 12 -and $r5 -le 3 -and $b5 -le 3) { $green++ }
    elseif ($r5 -ge 12 -and $b5 -ge 12 -and $g5 -le 3) { $magenta++ }
  }
  return @{ green = $green; magenta = $magenta }
}
function Save-Screen([string]$vmName, [string]$path) {
  $w = 320; $h = 240
  $bytes = Get-Thumbnail $vmName $w $h
  if ($null -eq $bytes) { Write-Host "no thumbnail"; return }
  $bmp = New-Object System.Drawing.Bitmap $w, $h
  for ($y = 0; $y -lt $h; $y++) { for ($x = 0; $x -lt $w; $x++) {
    $o = 2 * ($y * $w + $x); $v = $bytes[$o] -bor ($bytes[$o + 1] -shl 8)
    $bmp.SetPixel($x, $y, [System.Drawing.Color]::FromArgb((($v -shr 10) -band 0x1F) * 8, (($v -shr 5) -band 0x1F) * 8, ($v -band 0x1F) * 8))
  } }
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
}
# TypeScancodes で生のスキャンコード(セット1。拡張キーは E0 の前置、離すときは 0x80 を足す)を送る。JIS 専用キーは PressKey の VK では送れない(Q3)。
function Send-Scan([string]$vmName, [int]$sc, [int]$ext, [int]$holdMs = 150) {
  $ok = $false
  try {
    $vm = Get-CimInstance -Namespace $ns -ClassName Msvm_ComputerSystem -Filter "ElementName='$vmName'"
    $kbd = Get-CimAssociatedInstance -InputObject $vm -ResultClassName Msvm_Keyboard
    # if の結果を代入すると、1要素の配列は単なる byte に展開されてしまう(CIM の引数が配列でなくなる)。型付きの変数に代入する。
    if ($ext) { [byte[]]$make = 0xE0, $sc; [byte[]]$brk = 0xE0, ($sc -bor 0x80) } else { [byte[]]$make = , $sc; [byte[]]$brk = , ($sc -bor 0x80) }
    $a = Invoke-CimMethod -InputObject $kbd -MethodName TypeScancodes -Arguments @{ Scancodes = $make }
    Start-Sleep -Milliseconds $holdMs
    $b = Invoke-CimMethod -InputObject $kbd -MethodName TypeScancodes -Arguments @{ Scancodes = $brk }
    $ok = ($a.ReturnValue -eq 0 -and $b.ReturnValue -eq 0)
  } catch { $ok = $false }
  Start-Sleep -Milliseconds 250
  return $ok
}
# PressKey/ReleaseKey を送る。戻り値が 0 でなければ失敗(終了キー以外の失敗は、前提条件2で見る)。成功なら $true。
function Send-Key([string]$vmName, [uint32]$vk, [int]$holdMs = 150) {
  $ok = $false
  try {
    $vm = Get-CimInstance -Namespace $ns -ClassName Msvm_ComputerSystem -Filter "ElementName='$vmName'"
    $kbd = Get-CimAssociatedInstance -InputObject $vm -ResultClassName Msvm_Keyboard
    $a = Invoke-CimMethod -InputObject $kbd -MethodName PressKey -Arguments @{ keyCode = $vk }
    Start-Sleep -Milliseconds $holdMs
    $b = Invoke-CimMethod -InputObject $kbd -MethodName ReleaseKey -Arguments @{ keyCode = $vk }
    $ok = ($a.ReturnValue -eq 0 -and $b.ReturnValue -eq 0)
  } catch { $ok = $false }
  Start-Sleep -Milliseconds 250
  return $ok
}

# 土台の WinPE イメージ(probe.exe と startnet.cmd を入れたもの)を1回だけ作る。
$base = "$WorkDir\base.wim"
$mount = "$WorkDir\mount"
function New-BaseWim {
  Copy-Item "$pe\en-us\winpe.wim" $base -Force
  New-Item -ItemType Directory -Force -Path $mount | Out-Null
  dism /Mount-Image /ImageFile:$base /Index:1 /MountDir:$mount | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'dism mount (base) failed' }
  Copy-Item $ProbeExe "$mount\Windows\System32\probe.exe" -Force
  # WinPE は、起動用ディスクの FAT32 ボリュームにドライブ文字を付けない。この VM のボリュームは1つだけ(Volume 0、ラベル WINPE)
  # なので、diskpart で Volume 0 にドライブ文字 L: を割り当てる。ボリューム一覧は画面に残す(診断用)。
  Set-Content -Path "$mount\Windows\System32\assign.txt" -Value @('rescan', 'list volume', 'select volume 0', 'assign letter=L', 'list volume') -Encoding ASCII
  $startnet = @(
    'wpeinit',
    'diskpart /s X:\Windows\System32\assign.txt',
    'set LOGDRV=',
    'if exist L:\SCANVM.MARK set LOGDRV=L:',
    'echo logdrive=%LOGDRV%',
    'probe.exe %LOGDRV%\probe.log',
    'echo probe exited',
    'wpeutil shutdown'
  )
  Set-Content -Path "$mount\Windows\System32\startnet.cmd" -Value $startnet -Encoding ASCII
  dism /Unmount-Image /MountDir:$mount /Commit | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'dism unmount (base) failed' }
}

function Write-ScancodeMap([string]$wim, [byte[]]$map) {
  dism /Mount-Image /ImageFile:$wim /Index:1 /MountDir:$mount | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'dism mount failed' }
  try {
    reg load HKLM\PE_SYS "$mount\Windows\System32\config\SYSTEM" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'reg load failed' }
    $hex = ($map | ForEach-Object { $_.ToString('X2') }) -join ''
    reg add 'HKLM\PE_SYS\ControlSet001\Control\Keyboard Layout' /v 'Scancode Map' /t REG_BINARY /d $hex /f | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'reg add failed' }
    [gc]::Collect(); Start-Sleep -Seconds 1
    reg unload HKLM\PE_SYS | Out-Null
  } finally {
    dism /Unmount-Image /MountDir:$mount /Commit | Out-Null
  }
}

function New-BootVhdx([string]$wim, [string]$vhdx) {
  if (Test-Path $vhdx) { Remove-Item $vhdx -Force }
  New-VHD -Path $vhdx -SizeBytes 2GB -Dynamic | Out-Null
  $disk = Mount-VHD -Path $vhdx -Passthru | Initialize-Disk -PartitionStyle GPT -PassThru
  $part = New-Partition -DiskNumber $disk.Number -UseMaximumSize -AssignDriveLetter -GptType '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}'
  Format-Volume -Partition $part -FileSystem FAT32 -NewFileSystemLabel WINPE -Confirm:$false | Out-Null
  $drive = "$($part.DriveLetter):"
  Copy-Item "$pe\Media\*" "$drive\" -Recurse -Force
  New-Item -ItemType Directory -Force -Path "$drive\sources" | Out-Null
  Copy-Item $wim "$drive\sources\boot.wim" -Force
  Set-Content -Path "$drive\SCANVM.MARK" -Value 'scancode-vm' -Encoding ASCII
  Dismount-VHD -Path $vhdx
}

function Read-ProbeLog([string]$vhdx) {
  $disk = Mount-VHD -Path $vhdx -Passthru | Get-Disk
  $part = Get-Partition -DiskNumber $disk.Number | Where-Object { $_.DriveLetter } | Select-Object -First 1
  if (-not $part) { $part = Get-Partition -DiskNumber $disk.Number | Where-Object { $_.Type -ne 'Reserved' } | Select-Object -First 1; $part | Add-PartitionAccessPath -AssignDriveLetter }
  $part = Get-Partition -DiskNumber $disk.Number | Where-Object { $_.DriveLetter } | Select-Object -First 1
  $path = "$($part.DriveLetter):\probe.log"
  $text = if (Test-Path $path) { Get-Content $path } else { @('(no probe.log)') }
  Dismount-VHD -Path $vhdx
  return $text
}


# --- ログの解析 ---
function Read-Events($log) {
  $raw = @(); $ll = @()
  foreach ($l in $log) {
    if ($l -match '^KEY make=0x([0-9A-F]+) break=(\d) e0=(\d) e1=(\d) vk=0x([0-9A-F]+)') {
      $raw += [pscustomobject]@{ make = [Convert]::ToInt32($Matches[1], 16); down = ($Matches[2] -eq '0'); e0 = [int]$Matches[3]; vk = [Convert]::ToInt32($Matches[5], 16) }
    } elseif ($l -match '^WM msg=0x(100|104) wp=0x([0-9A-F]+) sc=0x([0-9A-F]+) ext=(\d)') {
      # ウィンドウメッセージ(WM_KEYDOWN/WM_SYSKEYDOWN)。LL フックは WinPE で呼ばれない(Q4)ので、第2の観測点はこれ。
      # wParam は仮想キー(左右の区別なし)だが、拡張ビットとスキャンコードで左右が分かる。押下のみ取る。
      $ll += [pscustomobject]@{ sc = [Convert]::ToInt32($Matches[3], 16); vk = [Convert]::ToInt32($Matches[2], 16); down = $true; ext = [int]$Matches[4] }
    }
  }
  return @{ raw = $raw; ll = $ll }
}
# 押下だけを取り、番兵(F11、make=0x57)で挟まれた、キー数がちょうど $n の最初の塊を返す。無ければ $null。
function Get-Chunk($events, [string]$kind, [int]$n) {
  $downs = @($events | Where-Object { $_.down })
  $isSentinel = if ($kind -eq 'raw') { { param($e) $e.make -eq 0x57 } } else { { param($e) $e.sc -eq 0x57 } }
  $idx = @(); for ($i = 0; $i -lt $downs.Count; $i++) { if (& $isSentinel $downs[$i]) { $idx += $i } }
  for ($k = 0; $k + 1 -lt $idx.Count; $k++) {
    if ($idx[$k + 1] - $idx[$k] - 1 -eq $n) { return @($downs[($idx[$k] + 1)..($idx[$k + 1] - 1)]) }
  }
  return $null
}

# keys: @{ vk; sc; ext } の配列(押すキーと、その写像前のスキャンコード)。expectSc: 各キーを押したときに出るはずのキーのスキャンコード
#   (verify のとき。手書き。期待の vk は、対照シナリオで測った表 $script:ctrlVk から引く)。kind: verify / observe / control。
function Invoke-Scenario([string]$name, [string]$kind, [byte[]]$map, $keys, $expectSc) {
  Write-Host "::group::scenario $name ($kind)"
  $wim = "$WorkDir\$name.wim"; $vhdx = "$WorkDir\$name.vhdx"
  Copy-Item $base $wim -Force
  if ($null -ne $map) { Write-ScancodeMap $wim $map }
  New-BootVhdx $wim $vhdx
  Remove-Item $wim -Force
  if (Get-VM -Name $name -ErrorAction SilentlyContinue) { Stop-VM -Name $name -TurnOff -Force -ErrorAction SilentlyContinue; Remove-VM -Name $name -Force }
  New-VM -Name $name -Generation 2 -MemoryStartupBytes 1GB -NoVHD | Out-Null
  Set-VMFirmware -VMName $name -EnableSecureBoot Off
  Set-VMProcessor -VMName $name -Count 2
  $hd = Add-VMHardDiskDrive -VMName $name -Path $vhdx -Passthru
  Set-VMFirmware -VMName $name -FirstBootDevice $hd
  $sw = [Diagnostics.Stopwatch]::StartNew()
  Start-VM -Name $name
  $chunkOk = New-Object System.Collections.Generic.List[bool]
  # 塊: F11 → テストの打鍵列 → F11 を、probe が READY になるまでの間も含めて数秒おきに送り続ける。
  while ($sw.Elapsed.TotalSeconds -lt $ChunkSeconds) {
    $ok = Send-Key $name 0x7A   # F11
    foreach ($k in $keys) { $sent = if ($k.scan) { Send-Scan $name $k.sc $k.ext } else { Send-Key $name $k.vk }; if (-not $sent) { $ok = $false } }
    if (-not (Send-Key $name 0x7A)) { $ok = $false }
    $chunkOk.Add($ok)
    Start-Sleep -Seconds 2
  }
  Write-Host ("host: chunks sent={0} stop_at={1}s" -f $chunkOk.Count, [int]$sw.Elapsed.TotalSeconds)
  Save-Screen $name "$ShotDir\$name-last.png"
  $thumb = Get-Thumbnail $name 320 240
  Write-Host ('thumbnail bytes: {0}' -f $thumb.Length)
  [IO.File]::WriteAllBytes("$ShotDir\$name-thumb.bin", $thumb)
  [void](Send-Key $name 0x7B)   # F12: probe を終了させる。終了で VM が止まるので、失敗は見ない
  $off = $false
  while ($sw.Elapsed.TotalSeconds -lt 300) { if ((Get-VM -Name $name).State -eq 'Off') { $off = $true; break }; Start-Sleep -Seconds 2 }
  Write-Host "VM shut down by WinPE: $off after $([int]$sw.Elapsed.TotalSeconds)s"
  if (-not $off) { Save-Screen $name "$ShotDir\$name-notoff.png"; Stop-VM -Name $name -TurnOff -Force }
  Remove-VM -Name $name -Force
  $log = Read-ProbeLog $vhdx
  $log | Set-Content "$ShotDir\$name-probe.log"
  Write-Host '--- probe.log ---'; $log | ForEach-Object { Write-Host $_ }; Write-Host '--- end ---'
  Remove-Item $vhdx -Force
  $ev = Read-Events $log
  $n = @($keys).Count
  $rawC = Get-Chunk $ev.raw 'raw' $n
  $llC = Get-Chunk $ev.ll 'll' $n
  # 前提条件(ADR-257 決定6): 1 START/READY、2 最後の2塊の PressKey が成功、3 塊が完全(Raw と WM とも打鍵数が一致)、4 写像前の値が送ったキーと一致
  $why = @()
  if (-not (($log -contains 'START') -and ($log -contains 'READY'))) { $why += '1(START/READY が無い)' }
  $tail = @($chunkOk | Select-Object -Last 2)
  if ($tail.Count -eq 0 -or ($tail -contains $false)) { $why += '2(PressKey が失敗した)' }
  if ($null -eq $rawC -or $null -eq $llC) { $why += '3(完全な塊が無い: raw=' + ($null -ne $rawC) + ' ll=' + ($null -ne $llC) + ')' }
  elseif ($kind -ne 'observe') {
    for ($i = 0; $i -lt $n; $i++) {
      if ($rawC[$i].make -ne $keys[$i].sc -or $rawC[$i].e0 -ne $keys[$i].ext) { $why += ("4(写像前の値が違う: キー{0} 期待 0x{1:X2}/e{2} 実際 0x{3:X2}/e{4})" -f $i, $keys[$i].sc, $keys[$i].ext, $rawC[$i].make, $rawC[$i].e0) }
    }
  }
  $result = 'PASS'; $detail = ''
  if ($why.Count -gt 0) { $result = 'INCONCLUSIVE'; $detail = 'conditions: ' + ($why -join '; ') }
  else {
    # Q5: LL フックの scanCode は写像の前か後か(Raw Input の make と比べる)。
    $q5 = @(); for ($i = 0; $i -lt $n; $i++) { $q5 += ('raw0x{0:X2}e{2}/ll0x{1:X2}e{3}' -f $rawC[$i].make, $llC[$i].sc, $rawC[$i].e0, $llC[$i].ext) }
    Write-Host ("Q5 {0}: {1}" -f $name, ($q5 -join ' '))
    $got = @($llC | ForEach-Object { $_.vk })
    $rawVks = @($rawC | ForEach-Object { $_.vk })
    Write-Host ("LL vk=[{0}] Raw VKey=[{1}]" -f (($got | ForEach-Object { '0x{0:X2}' -f $_ }) -join ','), (($rawVks | ForEach-Object { '0x{0:X2}' -f $_ }) -join ','))
    if ($kind -eq 'control') {
      # 対照の PASS は、写像を使わない独立の条件(ADR-257 決定6-5): (a) make が表と一致(条件4 で済み)、(b) LL の scanCode+拡張が Raw の make+E0 と一致、
      # (c) 英字は手書きの vk と一致。
      for ($i = 0; $i -lt $n; $i++) {
        if ($llC[$i].sc -ne $rawC[$i].make -or $llC[$i].ext -ne $rawC[$i].e0) { $result = 'FAIL'; $detail += ("(b) key{0} LL sc=0x{1:X2}/e{2} != Raw make=0x{3:X2}/e{4}; " -f $i, $llC[$i].sc, $llC[$i].ext, $rawC[$i].make, $rawC[$i].e0) }
        if ($keys[$i].letter -and $llC[$i].vk -ne $keys[$i].vk) { $result = 'FAIL'; $detail += ("(c) key{0} LL vk=0x{1:X2} != 0x{2:X2}; " -f $i, $llC[$i].vk, $keys[$i].vk) }
      }
      if ($result -eq 'PASS') { for ($i = 0; $i -lt $n; $i++) { $script:ctrlVk[$keys[$i].sc * 2 + $keys[$i].ext] = $llC[$i].vk } }
    } elseif ($kind -eq 'observe') {
      $result = 'OBSERVED'
    } else {
      $exp = @(); foreach ($sc in $expectSc) { $exp += $script:ctrlVk[$sc * 2] }
      if ($exp -contains $null) { $result = 'INCONCLUSIVE'; $detail = 'conditions: 5(対照の表に無いキー)' }
      elseif (($got -join ',') -ne ($exp -join ',')) { $result = 'FAIL'; $detail = ('expected LL vk=[{0}] got=[{1}]' -f (($exp | ForEach-Object { '0x{0:X2}' -f $_ }) -join ','), (($got | ForEach-Object { '0x{0:X2}' -f $_ }) -join ',')) }
    }
  }
  Write-Host ("RESULT {0}: {1} {2}" -f $name, $result, $detail)
  Write-Host '::endgroup::'
  return $result
}
# 検証不能なら、VM を作り直して1回だけ再試行する(FAIL は再試行しない)。
function Invoke-WithRetry([string]$name, [string]$kind, $map, $keys, $expectSc) {
  $r = Invoke-Scenario $name $kind $map $keys $expectSc
  if ($r -eq 'INCONCLUSIVE') { Write-Host "retry $name once (INCONCLUSIVE)"; $r = Invoke-Scenario $name $kind $map $keys $expectSc }
  return $r
}

# Scancode Map のバイト列(ヘッダ + 件数+1 + エントリ(to 2バイト, from 2バイト, LE) + 終端)。entries は (from, to) の並び。
function New-MapBytes([int[][]]$entries) {
  $b = New-Object System.Collections.Generic.List[byte]
  $b.AddRange([byte[]](0,0,0,0, 0,0,0,0)); $b.AddRange([BitConverter]::GetBytes([uint32]($entries.Count + 1)))
  foreach ($e in $entries) { $from = $e[0]; $to = $e[1]; $b.AddRange([BitConverter]::GetBytes([uint16]$to)); $b.AddRange([BitConverter]::GetBytes([uint16]$from)) }
  $b.AddRange([byte[]](0,0,0,0))
  return ,$b.ToArray()
}

# Q3: Msvm_Keyboard のメソッドの一覧(TypeScancodes があるか)。
Get-CimClass -Namespace $ns -ClassName Msvm_Keyboard | ForEach-Object { $_.CimClassMethods } | ForEach-Object { Write-Host ('Msvm_Keyboard method: {0}({1})' -f $_.Name, (($_.Parameters | ForEach-Object { $_.Name + ':' + $_.CimType }) -join ', ')) }

New-BaseWim
$script:ctrlVk = @{}   # (scancode*2+ext) → 対照で測った LL vkCode
function K([int]$vk, [int]$sc, [int]$ext = 0, [bool]$letter = $false, [bool]$scan = $false) { return [pscustomobject]@{ vk = $vk; sc = $sc; ext = $ext; letter = $letter; scan = $scan } }
$kA = K 0x41 0x1E 0 $true; $kS = K 0x53 0x1F 0 $true; $kD = K 0x44 0x20 0 $true; $kB = K 0x42 0x30 0 $true
$A = 0x1E; $S = 0x1F; $D = 0x20; $B = 0x30
$results = [ordered]@{}
# 対照(最初に単独で回す)。写像なしで A,S,D,B を押し、LL の vkCode の表を作る。対照が通らなければ、残りは検証不能にする。
$results['control'] = Invoke-WithRetry 'control' 'control' $null @($kA, $kS, $kD, $kB) $null
if ($results['control'] -eq 'PASS') {
  # ペア: A の位置は B を出し、B の位置は A を出す(期待は手書き: A を押すと B のキーが出る)。
  $results['pair'] = Invoke-WithRetry 'pair' 'verify' (New-MapBytes @(@($A, $B), @($B, $A))) @($kA, $kB) @($B, $A)
  # 3巡回 A→S→D→A(A の位置は S を、S の位置は D を、D の位置は A を出す)。1回だけの適用なら A→S, S→D, D→A。
  $results['cycle'] = Invoke-WithRetry 'cycle' 'verify' (New-MapBytes @(@($A, $S), @($S, $D), @($D, $A))) @($kA, $kS, $kD) @($S, $D, $A)
  # 逆向き: A の位置は D を、D の位置は S を、S の位置は A を出す。
  $results['cycle-reverse'] = Invoke-WithRetry 'cycle-reverse' 'verify' (New-MapBytes @(@($A, $D), @($D, $S), @($S, $A))) @($kA, $kS, $kD) @($D, $A, $S)
  # Q3: 送れるキーの範囲。写像なし・期待値なしで、受け取った値を記録するだけ。PressKey の VK(修飾・拡張キー)と、TypeScancodes の生スキャンコード(JIS 専用キー)。
  $q3pk = @((K 0xA2 0x1D 0), (K 0xA3 0x1D 1), (K 0xA4 0x38 0), (K 0xA5 0x38 1), (K 0x25 0x4B 1))
  $results['q3-pressk'] = Invoke-Scenario 'q3-pressk' 'observe' $null $q3pk $null
  $q3sc = @((K 0 0x7B 0 $false $true), (K 0 0x79 0 $false $true), (K 0 0x70 0 $false $true), (K 0 0x29 0 $false $true), (K 0 0x73 0 $false $true), (K 0 0x7D 0 $false $true), (K 0 0x1D 1 $false $true), (K 0 0x38 1 $false $true))
  $results['q3-scan'] = Invoke-Scenario 'q3-scan' 'observe' $null $q3sc $null
}
$results.GetEnumerator() | ForEach-Object { Write-Host "SUMMARY $($_.Key) = $($_.Value)" }
if ($results.Values -contains 'FAIL') { exit 1 }
if ($results.Values -contains 'INCONCLUSIVE' -or $results['control'] -ne 'PASS') { Write-Host 'INCONCLUSIVE: see conditions above'; exit 3 }
