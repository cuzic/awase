# MSI アンインストールで、自分のインストール先の通知領域アイコン登録行だけが消えることを確かめる。
# wix/main.wxs の RemoveTrayIconRow（遅延 CA）の実機相当（windows-latest）検証。
#
# 実際のトレイ登録は伴わない: HKCU\Control Panel\NotifyIconSettings に疑似の行を直接作り、
# msiexec のインストール・アップグレード・アンインストール後に何が残るかを見る。
#   消えるべき  : 自分のパス（%LOCALAPPDATA%\awase\awase.exe）と、その既知フォルダ GUID 表記
#   残るべき    : 他のパス（ZIP 版・開発ビルド相当）、同じ親の別ディレクトリ・別 exe
#   アップグレード（旧版の削除）では、自分の行は消さない
param([string]$Out = "out")
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$wixBin = "C:\Program Files (x86)\WiX Toolset v3.14\bin"
$root = "HKCU:\Control Panel\NotifyIconSettings"
$own = Join-Path $env:LOCALAPPDATA "awase\awase.exe"
$ownGuid = "{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}\awase\awase.exe"
$keep = @{
    zip      = "C:\Users\someone\Downloads\awase-windows-x64\awase.exe"
    dev      = "C:\Users\someone\awase\target\debug\awase.exe"
    sibling  = (Join-Path $env:LOCALAPPDATA "awase2\awase.exe")
    settings = (Join-Path $env:LOCALAPPDATA "awase\awase-settings.exe")
}
$failures = New-Object System.Collections.Generic.List[string]
$summary = New-Object System.Collections.Generic.List[string]

function Build-Msi([string]$Version) {
    # 実バイナリは不要。LaunchApplication が起動しても即終了する実在の exe を awase.exe の代わりに置く。
    $d = "dist"
    New-Item -ItemType Directory -Force -Path "$d\layout", "$d\data" | Out-Null
    foreach ($n in "awase.exe", "awase-settings.exe", "awase-keymap-learn-win.exe") {
        Copy-Item "$env:WINDIR\System32\whoami.exe" "$d\$n" -Force
    }
    Copy-Item config.toml $d -Force
    Copy-Item layout\*.yab "$d\layout" -Force
    Copy-Item data\*.csv.gz "$d\data" -Force
    $msi = "$Out\awase-$Version.msi"
    & "$wixBin\candle.exe" wix\main.wxs "-dVersion=$Version" -out "$Out\main-$Version.wixobj" | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "candle failed" }
    & "$wixBin\light.exe" "$Out\main-$Version.wixobj" -out $msi -ext WixUIExtension | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "light failed" }
    return $msi
}

function Msi([string]$Verb, [string]$Msi, [string]$Tag) {
    $p = Start-Process msiexec.exe -ArgumentList @($Verb, "`"$Msi`"", "/qn", "/l*v", "`"$Out\$Tag.log`"") -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "msiexec $Verb ($Tag) exit $($p.ExitCode)" }
}

function Add-Row([string]$Path) {
    $k = Join-Path $root ([string]([UInt64](Get-Random -Minimum 1 -Maximum 2147483647) * 1000003))
    New-Item -Path $k -Force | Out-Null
    New-ItemProperty -Path $k -Name ExecutablePath -Value $Path -PropertyType String -Force | Out-Null
}

function Rows() {
    if (-not (Test-Path $root)) { return @() }
    @(Get-ChildItem $root | ForEach-Object { (Get-ItemProperty $_.PSPath).ExecutablePath })
}

function Clear-Rows() {
    if (Test-Path $root) {
        Get-ChildItem $root | Where-Object { (Get-ItemProperty $_.PSPath).ExecutablePath -match 'awase|someone' } |
            ForEach-Object { Remove-Item $_.PSPath -Recurse }
    }
}

function Seed() {
    Clear-Rows
    Add-Row $own
    Add-Row $ownGuid
    foreach ($v in $keep.Values) { Add-Row $v }
}

function Check([string]$Name, [bool]$Cond, [string]$Detail) {
    $mark = if ($Cond) { "OK " } else { "NG " }
    $line = "$mark $Name"
    Write-Host $line
    $summary.Add("- $line")
    if (-not $Cond) { $failures.Add("$Name : $Detail") }
}

function Report-Rows([string]$Title) {
    $r = (Rows | Where-Object { $_ -match 'awase|someone' }) -join "`n    "
    Write-Host "[$Title]`n    $r"
}

$v1 = Build-Msi "0.1.0"
$v2 = Build-Msi "0.2.0"

# --- シナリオ A: アンインストールで自分の行だけ消える ---
Msi "/i" $v1 "A-install"
Seed
Report-Rows "A: before uninstall"
Msi "/x" $v1 "A-uninstall"
Report-Rows "A: after uninstall"
$after = Rows
Check "A: 自分のパスの行が消える" (-not ($after -contains $own)) "own row remained"
Check "A: 自分の既知フォルダ GUID 表記の行が消える" (-not ($after -contains $ownGuid)) "own GUID row remained"
foreach ($k in $keep.Keys) {
    Check "A: 他の行が残る ($k)" ($after -contains $keep[$k]) "$($keep[$k]) was removed"
}

# --- シナリオ B: アップグレード（旧版の削除）では自分の行を消さない ---
Clear-Rows
Msi "/i" $v1 "B-install-old"
Seed
Msi "/i" $v2 "B-upgrade"
Report-Rows "B: after upgrade"
$afterUp = Rows
Check "B: アップグレード後も自分の行が残る" ($afterUp -contains $own) "own row removed during upgrade"
Msi "/x" $v2 "B-uninstall"
Report-Rows "B: after uninstall"
Check "B: その後のアンインストールで消える" (-not ((Rows) -contains $own)) "own row remained"
Check "B: 他の行は残る (zip)" ((Rows) -contains $keep.zip) "zip row removed"

Clear-Rows
$status = if ($failures.Count -eq 0) { "PASS" } else { "FAIL" }
@("# msi-tray-row-verify: $status", "") + $summary | Out-File "$Out\summary.md" -Encoding utf8
if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Host "FAIL: $_" }
    exit 1
}
