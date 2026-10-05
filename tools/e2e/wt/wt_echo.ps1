# Windows Terminal の中で動かす、受け取ったキーの記録器(wt_vocab.launch が起動する)。
# 1 キーにつき `経過ms<TAB>KeyChar の文字コード<TAB>ConsoleKey<TAB>修飾` を 1 行、$Out に追記する。
# IME で確定した文字(かな)・余計に出た文字(「@」)・ローマ字のリテラルを、画面を読まずに文字コードで区別するため。
# 画面にも同じ文字を出す(UIA の TextPattern で読み戻せるかを測るため)。
param([Parameter(Mandatory = $true)][string]$Out, [int]$Seconds = 180)
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$sw = [Diagnostics.Stopwatch]::StartNew()
"READY" | Out-File -FilePath $Out -Encoding utf8
Write-Host "wt_echo ready"
while ($sw.Elapsed.TotalSeconds -lt $Seconds) {
  $k = [Console]::ReadKey($true)
  ("{0}`t{1}`t{2}`t{3}" -f $sw.ElapsedMilliseconds, [int]$k.KeyChar, [int]$k.Key, $k.Modifiers) | Add-Content -Path $Out -Encoding utf8
  if ([int]$k.KeyChar -eq 13) { Write-Host "" } else { Write-Host -NoNewline $k.KeyChar }
}
