# ADR-255(使い捨てのスパイク): ドライバ経由(スキャンコード→キーボードレイアウトの変換を通る物理相当)と、SendInput の vk 直接指定の違いを、
# 日本語キー(ひらがな・無変換・変換・半角/全角・英数 など)の Down/Up の vk で比べる。BUG-131/132 は「物理キーの Down と Up で vk が違う」非対称が原因で、
# vk を直接指定する従来の注入では再現できなかった疑いがある。作業ディレクトリはリポジトリのルート。
Add-Type -AssemblyName System.Windows.Forms
$src = Get-Content (Join-Path $PSScriptRoot 'AwaseKb.cs') -Raw
Add-Type -TypeDefinition $src
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class KbInfo { [DllImport("user32.dll")] public static extern int GetKeyboardType(int n); [DllImport("user32.dll")] public static extern IntPtr GetKeyboardLayout(uint t); [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool GetKeyboardLayoutName(System.Text.StringBuilder s); [DllImport("user32.dll")] public static extern uint MapVirtualKeyW(uint c, uint t); }'
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class KbLayout { [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr LoadKeyboardLayoutW(string klid, uint flags); [DllImport("user32.dll")] public static extern IntPtr ActivateKeyboardLayout(IntPtr h, uint flags); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); }'
# 測る窓: 前面・フォーカスを持つ Form を作り、スレッドのレイアウトを環境変数 PROBE_KLID(既定 00000411=日本語)にする。LL フックの vk は入力先のレイアウトで決まる。
$klid = if ($env:PROBE_KLID) { $env:PROBE_KLID } else { '00000411' }
$form = New-Object System.Windows.Forms.Form
$form.Text = 'layout-probe'; $form.TopMost = $true; $form.Width = 300; $form.Height = 120
$tb = New-Object System.Windows.Forms.TextBox; $form.Controls.Add($tb)
$form.Show(); $form.Activate(); $tb.Focus() | Out-Null
$hkl = [KbLayout]::LoadKeyboardLayoutW($klid, 1)
[void][KbLayout]::ActivateKeyboardLayout($hkl, 0)
[void][KbLayout]::SetForegroundWindow($form.Handle)
Start-Sleep -Milliseconds 800
[System.Windows.Forms.Application]::DoEvents()
$sb = New-Object System.Text.StringBuilder 16
[void][KbInfo]::GetKeyboardLayoutName($sb)
Write-Host ("GetKeyboardType: type={0} subtype={1} fkeys={2}" -f [KbInfo]::GetKeyboardType(0), [KbInfo]::GetKeyboardType(1), [KbInfo]::GetKeyboardType(2))
Write-Host ("layout name={0} hkl=0x{1:X}" -f $sb.ToString(), [int64][KbInfo]::GetKeyboardLayout(0))
$h = [AwaseKb]::CreateFile('\\.\AwaseKbInject', [uint32]3221225472, 3, [IntPtr]::Zero, 3, 0, [IntPtr]::Zero)
if ($h -eq [IntPtr]::new(-1)) { Write-Host '::error::制御デバイスを開けない'; exit 3 }
Write-Host "LL hook installed: $([AwaseKb]::Install())"
function Pump($ms) { $end = (Get-Date).AddMilliseconds($ms); while ((Get-Date) -lt $end) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 10 } }
Pump 300
# (名前, スキャンコード, E0)。JIS 配列の日本語キー。
$keys = @(
  @('ひらがな(カタカナひらがなローマ字)', 0x70, 0), @('無変換', 0x7B, 0), @('変換', 0x79, 0), @('半角/全角', 0x29, 0),
  @('英数(Caps)', 0x3A, 0), @('Fn 風 F13 相当(0x64)', 0x64, 0), @('円記号 ¥', 0x7D, 0), @('バックスラッシュ _', 0x73, 0), @('A(対照)', 0x1E, 0), @('左Alt', 0x38, 0), @('右Alt', 0x38, 2)
)
$rows = @()
foreach ($k in $keys) {
  foreach ($shift in @($false, $true)) {
    [AwaseKb]::Log.Clear()
    if ($shift) { [void][AwaseKb]::Inject($h, [uint16[]](0x2A, 0)); Pump 120 }   # 左 Shift の make
    [AwaseKb]::Log.Clear()
    $e0 = [uint16]$k[2]
    [void][AwaseKb]::Inject($h, [uint16[]]([uint16]$k[1], $e0)); Pump 150
    [void][AwaseKb]::Inject($h, [uint16[]]([uint16]$k[1], [uint16]($e0 -bor 1))); Pump 150
    $ev = @([AwaseKb]::Log | ForEach-Object { $_ })
    if ($shift) { [void][AwaseKb]::Inject($h, [uint16[]](0x2A, 1)); Pump 120 }
    $dn = ($ev | Where-Object { $_ -match 'down=1' } | Select-Object -First 1)
    $up = ($ev | Where-Object { $_ -match 'down=0' } | Select-Object -First 1)
    $f = { param($s) if ($s) { ($s -replace ' injected=0 extra=0x0','') } else { '(なし)' } }
    $rows += "| $($k[0]) scan=0x$('{0:X2}' -f $k[1])$(if($e0){' E0'}) shift=$([int]$shift) | $(& $f $dn) | $(& $f $up) |"
  }
}
[void][AwaseKb]::CloseHandle($h)
$out = @('| キー | Down(LL フック) | Up(LL フック) |', '| --- | --- | --- |') + $rows
$out | ForEach-Object { Write-Host $_ }
if ($env:GITHUB_STEP_SUMMARY) { $out | Add-Content -Path $env:GITHUB_STEP_SUMMARY -Encoding utf8 }
# 対照: vk 直接指定(従来の注入)の Down/Up。
Write-Host '--- SendInput(vk 直接指定、従来の注入)---'
foreach ($vk in @(0xF2, 0xF0, 0xF3, 0xF4, 0x1C, 0x1D)) {
  [AwaseKb]::Log.Clear()
  [AwaseKb]::SendKey([uint16]$vk, [uint16]0, $false, 0x5350494B); Pump 120
  [AwaseKb]::SendKey([uint16]$vk, [uint16]0, $true, 0x5350494B); Pump 150
  Write-Host ("vk=0x{0:X2}: {1}" -f $vk, (([AwaseKb]::Log | ForEach-Object { $_ -replace 'extra=0x5350494B','extra=marker' }) -join ' || '))
}
