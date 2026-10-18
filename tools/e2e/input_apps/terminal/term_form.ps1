# Minimal input target that runs inside Windows Terminal (typing_stress --form=ext --ext=windows-terminal).
# It collects the characters the console receives (including IME-committed text) and rewrites the whole text to the dump file on every key.
#   Ctrl+A followed by Backspace = clear all (what typing_stress's clear sends). Backspace = delete one char. Enter is ignored.
# NOTE: keep this file ASCII-only. Windows PowerShell 5.1 reads a BOM-less UTF-8 file as ANSI, so non-ASCII comments become mojibake.
param([Parameter(Mandatory = $true)][string]$Dump)
$enc = New-Object System.Text.UTF8Encoding($false)
$sb = New-Object System.Text.StringBuilder
$selectAll = $false
# Write atomically (write to .tmp, then replace) so the reader never sees a half-written text; retry a few times on sharing violations.
function Save {
    $tmp = $Dump + '.tmp'
    for ($i = 0; $i -lt 5; $i++) {
        try {
            [IO.File]::WriteAllText($tmp, $sb.ToString(), $enc)
            Move-Item -LiteralPath $tmp -Destination $Dump -Force -ErrorAction Stop
            return
        } catch { Start-Sleep -Milliseconds 20 }
    }
}
Save
Write-Host 'term_form: type here (awase typing_stress target)'
while ($true) {
    $k = [Console]::ReadKey($true)
    $ctrl = ($k.Modifiers -band [ConsoleModifiers]::Control) -ne 0
    if ($ctrl -and $k.Key -eq 'A') { $selectAll = $true; continue }
    if ($k.Key -eq 'Backspace') {
        if ($selectAll) { [void]$sb.Clear() } elseif ($sb.Length -gt 0) { $sb.Length-- }
        $selectAll = $false
    } elseif ($k.Key -eq 'Enter') {
        $selectAll = $false
    } elseif (-not $ctrl -and $k.KeyChar -ge ' ') {
        $selectAll = $false
        [void]$sb.Append($k.KeyChar)
    }
    Save
}
