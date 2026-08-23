# awase installer script for ZIP distribution
$ErrorActionPreference = "Stop"

# awase.exe / awase-settings.exe は MSVC ターゲットで動的リンクされており、
# Microsoft Visual C++ 2015-2022 再頒布可能パッケージ (vcruntime140.dll) が
# 別途必要。このチェックが無いと、ファイルコピー自体は成功したのに初回起動時
# に OS の分かりにくいダイアログ（「vcruntime140.dll が見つからないため、この
# プログラムを開始できません」）で失敗する。MSI 版（wix/main.wxs の
# VCRUNTIME140FOUND LaunchCondition）と同じ対策を ZIP 版でも行う。
# 32-bit PowerShell から "System32" を素朴に見ると、WOW64 ファイルシステム
# リダイレクタにより暗黙に SysWOW64（x86 用）へリダイレクトされてしまい、
# x86 版 vcruntime140.dll だけが入っている環境で誤って「見つかった」判定に
# なる（配布している awase.exe は x64 専用なのでチェックをすり抜ける）。
# Sysnative はそのリダイレクトを回避して常にネイティブ 64-bit System32 を
# 指す仮想パス（64-bit PowerShell では存在しないパス）なので、プロセスの
# ビット数で参照先を一意に選ぶ。
$system32Dir = if ([Environment]::Is64BitOperatingSystem -and -not [Environment]::Is64BitProcess) {
    Join-Path $env:SystemRoot "Sysnative"
} else {
    Join-Path $env:SystemRoot "System32"
}
if (-not (Test-Path (Join-Path $system32Dir "vcruntime140.dll"))) {
    # $ErrorActionPreference の値に依存せず必ずスクリプトを止めるため throw を使う
    # （Write-Error は非 terminating error であり、将来この前後に -ErrorAction
    # Continue や try/catch が入ると黙って無視されうる）。
    throw @"
awase の実行には Microsoft Visual C++ 2015-2022 再頒布可能パッケージ (x64) が必要です。
https://aka.ms/vs/17/release/vc_redist.x64.exe からダウンロードしてインストールした後、
このインストーラーを再実行してください。
"@
}

$installDir = "$env:LOCALAPPDATA\awase"

# Create directories
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
New-Item -ItemType Directory -Force -Path "$installDir\layout" | Out-Null
New-Item -ItemType Directory -Force -Path "$installDir\data" | Out-Null

# Copy files
Copy-Item "awase.exe" "$installDir\" -Force
Copy-Item "awase-settings.exe" "$installDir\" -Force

# layout/: 既存なら上書きしない（awase-settings の配列編集タブが
# その場で上書き保存するユーザーデータのため。ADR-099 決定2）。
function Copy-IfAbsent($sourceGlob, $destDir) {
    New-Item -ItemType Directory -Force -Path $destDir | Out-Null
    if (Test-Path $sourceGlob) {
        Get-ChildItem $sourceGlob | ForEach-Object {
            $dest = Join-Path $destDir $_.Name
            if (-not (Test-Path $dest)) {
                Copy-Item $_.FullName $dest
            }
        }
    }
}
Copy-IfAbsent "layout\*" "$installDir\layout"

# data/: ユーザーが編集する手段を持たないプログラム資産のため、
# 従来通り常に上書きする（ADR-099 決定2、MSI の NgramData コンポーネントと
# 挙動を揃える）。$ErrorActionPreference = "Stop" 下でソースが空/不在だと
# terminating error になるため Test-Path でガードする。
if (Test-Path "data\*") {
    Copy-Item "data\*" "$installDir\data\" -Force
}

# Config: don't overwrite if exists
if (-not (Test-Path "$installDir\config.toml")) {
    Copy-Item "config.toml" "$installDir\" -Force
}

# Register startup
$regPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
Set-ItemProperty -Path $regPath -Name "awase" -Value "$installDir\awase.exe"

# Create Start Menu shortcut
$shell = New-Object -ComObject WScript.Shell
$startMenu = [Environment]::GetFolderPath("StartMenu")
$shortcut = $shell.CreateShortcut("$startMenu\Programs\awase.lnk")
$shortcut.TargetPath = "$installDir\awase.exe"
$shortcut.WorkingDirectory = $installDir
$shortcut.Save()

$settingsShortcut = $shell.CreateShortcut("$startMenu\Programs\awase Settings.lnk")
$settingsShortcut.TargetPath = "$installDir\awase-settings.exe"
$settingsShortcut.WorkingDirectory = $installDir
$settingsShortcut.Save()

Write-Host "awase installed to $installDir"
Write-Host "Startup registration: OK"
Write-Host "Start Menu shortcuts: OK"
