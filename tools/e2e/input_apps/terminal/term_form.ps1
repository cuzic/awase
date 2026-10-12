# Windows Terminal 内で動かす「入力を受けて、そのまま {dump} へ書き出す」最小の入力先(typing_stress --form=ext --ext=windows-terminal 用)。
# コンソールが受け取った文字(IME で確定した結果を含む)を貯め、キーごとに UTF-8 のファイルへ全文を書き直す。
#   Ctrl+A のあとの Backspace = 全消去(typing_stress の clear が送る操作)。Backspace = 1 文字消す。Enter は無視する。
param([Parameter(Mandatory = $true)][string]$Dump)
$enc = New-Object System.Text.UTF8Encoding($false)
$sb = New-Object System.Text.StringBuilder
$selectAll = $false
# 原子的に書く(.tmp へ書いてから置き換える)。読み手が書き込み途中の本文を読まないようにし、共有違反などで落ちないよう数回やり直す。
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
