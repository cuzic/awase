# ADR-255 の通し(e2e-ime.yml の via_settings='preset' から呼ぶ): dist\config.toml を awase-settings.exe で開き、
# 再割り当てタブの「無変換・変換を IME OFF のとき Space にする」を押して Ctrl+S で保存する(GUI が書いた [[keymaps]] を awase が読む)。
# 窓の位置は起動ごとに変わるので EnumWindows でプロセスの最大の可視窓を探す。クリック位置: 診断が1件のとき button は窓の左上から y≈245、
# 2件のとき≈259 なので両方に当たる y=252。Ctrl+S の前にタイトルバーを押してフォーカスを取る。作業ディレクトリはリポジトリのルート。
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition '
using System;
using System.Runtime.InteropServices;
public class WinFind2 {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
  public static int[] Largest(uint pid) {
    int[] best = null; long area = 0;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) {
        R r; GetWindowRect(h, out r);
        long a = (long)(r.Rt - r.L) * (r.B - r.T);
        if (a > area) { area = a; best = new int[] { r.L, r.T, r.Rt - r.L, r.B - r.T }; }
      }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
'
[void][WinFind2]::SetProcessDPIAware()
function Snap2($name) {
  $r = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $r.Width, $r.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen(0, 0, 0, 0, $bmp.Size)
  $bmp.Save("$PWD\settings-save-$name.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose()
}
$env:AWASE_SETTINGS_INITIAL_TAB = 'keymap'
$env:AWASE_SETTINGS_WINDOW_SIZE = '760x700'
$sp = Start-Process -FilePath "$PWD\dist\awase-settings.exe" -ArgumentList "`"$PWD\dist\config.toml`"" -PassThru
$b = $null
for ($i = 0; $i -lt 60 -and $null -eq $b; $i++) {
  Start-Sleep -Seconds 1
  $b = [WinFind2]::Largest([uint32]$sp.Id)
  if ($null -ne $b -and $b[2] -lt 300) { $b = $null }
}
Start-Sleep -Seconds 6
$b = [WinFind2]::Largest([uint32]$sp.Id)
Write-Host "settings window: $($b -join ',')"
Snap2 '1-before'
if ($null -eq $b) { throw 'awase-settings の窓が見つからない' }
[void][WinFind2]::SetCursorPos($b[0] + 200, $b[1] + 252)
Start-Sleep -Milliseconds 400
[WinFind2]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 100; [WinFind2]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Seconds 2
Snap2 '2-after-click'
# Ctrl+S を確実に窓へ届けるため、タイトルバーを押して前面・フォーカスを取る(run 38121166980: クリックでルールは足せたが ^s は届かなかった)。
[void][WinFind2]::SetCursorPos($b[0] + 400, $b[1] + 15)
Start-Sleep -Milliseconds 300
[WinFind2]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 100; [WinFind2]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Seconds 1
[System.Windows.Forms.SendKeys]::SendWait('^s')
Start-Sleep -Seconds 6
Snap2 '3-after-save'
if (-not $sp.HasExited) { Stop-Process -Id $sp.Id -Force }
Write-Host '===== config.toml after settings save ====='
Get-Content "$PWD\dist\config.toml" -Raw -Encoding utf8 | Select-String -Pattern 'keymaps|ime =|from =|to =' | ForEach-Object { Write-Host $_.Line }
Write-Host '===== end ====='
$saved = Get-Content "$PWD\dist\config.toml" -Raw -Encoding utf8
if (([regex]::Matches($saved, 'ime\s*=\s*"off"')).Count -ne 2) { Write-Host '::error::設定画面の保存で ime = off が 2 件書かれなかった'; exit 3 }
