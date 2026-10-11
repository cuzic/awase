# ADR-255(CI 専用、使い捨ての VM でだけ実行する): キーボードフィルタドライバ awasekbf をビルド・テスト署名・インストールする。
# windows-latest は bcdedit の testsigning が既に Yes・Secure Boot なし・HVCI なしなので、テスト署名のカーネルドライバが読める(run 38132643604 で確認)。
# 流れ: WDK(NuGet)取得 → cl/link → 自己署名の証明書を Root/TrustedPublisher へ → signtool 署名 → サービス登録 → Hyper-V Virtual Keyboard の
# デバイスの UpperFilters に載せて pnputil /restart-device。成功すると \\.\AwaseKbInject が開け、ドライバ経由のキーは物理キーと同じ経路で入る
# (LLKHF_INJECTED なし、dwExtraInfo 0)。作業ディレクトリはリポジトリのルート。
$ErrorActionPreference = 'Stop'
# 安全装置: このドライバはテスト署名のカーネルドライバ(キー入力を OS に流せる)。利用者の PC に入れてはならないので、GitHub Actions のホスト型ランナー以外では実行しない。
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'install-driver.ps1 は GitHub のホスト型ランナー専用です(使い捨ての VM でだけ実行する)' }
$work = Join-Path $PWD 'kbd-driver-build'
New-Item -ItemType Directory -Force -Path $work | Out-Null

$idx = Invoke-RestMethod 'https://api.nuget.org/v3-flatcontainer/microsoft.windows.wdk.x64/index.json'
$ver = ($idx.versions | Where-Object { $_ -like '10.0.26100.*' } | Select-Object -Last 1)
Write-Host "WDK package version: $ver"
Invoke-WebRequest 'https://dist.nuget.org/win-x86-commandline/latest/nuget.exe' -OutFile "$work\nuget.exe"
& "$work\nuget.exe" install Microsoft.Windows.WDK.x64 -Version $ver -OutputDirectory "$work\pk" -NonInteractive | Select-Object -Last 3
$root = (Get-ChildItem "$work\pk" -Directory -Filter 'Microsoft.Windows.WDK.x64*' | Select-Object -First 1).FullName
$km = (Get-ChildItem $root -Recurse -Filter ntddk.h | Select-Object -First 1).Directory.FullName
$incRoot = (Get-Item "$km\..").FullName
$lib = (Get-ChildItem $root -Recurse -Filter ntoskrnl.lib | Where-Object { $_.FullName -match 'x64' } | Select-Object -First 1).Directory.FullName
$sdkInc = 'C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0'
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products * -property installationPath
$bat = @(
  "call `"$vs\VC\Auxiliary\Build\vcvars64.bat`"",
  "cl /nologo /c /kernel /W3 /Gz /GS- /Zi /O2 /D_AMD64_ /DAMD64 /D_WIN64 /DNTDDI_VERSION=0x0A000010 /D_WIN32_WINNT=0x0A00 /DPOOL_NX_OPTIN=1 /I`"$km`" /I`"$incRoot\shared`" /I`"$sdkInc\shared`" /Fo`"$work\awasekbf.obj`" tools\kbd-inject-driver\awasekbf.c",
  'if errorlevel 1 exit /b 1',
  "link /nologo /DRIVER /KERNEL /ENTRY:DriverEntry /SUBSYSTEM:NATIVE /NODEFAULTLIB /OUT:`"$work\awasekbf.sys`" /LIBPATH:`"$lib`" ntoskrnl.lib hal.lib BufferOverflowFastFailK.lib `"$work\awasekbf.obj`""
)
Set-Content -Path "$work\build.cmd" -Value $bat -Encoding ASCII
cmd /c "$work\build.cmd"
if ($LASTEXITCODE -ne 0) { throw "driver build failed ($LASTEXITCODE)" }

$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=awase-ci-test-signing' -CertStoreLocation Cert:\LocalMachine\My -KeyExportPolicy Exportable -NotAfter (Get-Date).AddDays(2)
$cer = "$work\awase-ci.cer"
Export-Certificate -Cert $cert -FilePath $cer | Out-Null
Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null
$signtool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse -Filter signtool.exe | Where-Object { $_.FullName -match '\\x64\\' } | Select-Object -First 1 -ExpandProperty FullName
& $signtool sign /fd sha256 /sha1 $cert.Thumbprint /s My /sm "$work\awasekbf.sys" 2>&1 | Select-Object -Last 3

Copy-Item "$work\awasekbf.sys" C:\Windows\System32\drivers\awasekbf.sys -Force
sc.exe create awasekbf type= kernel start= demand binPath= 'System32\drivers\awasekbf.sys' | Write-Host
$kb = Get-PnpDevice -Class Keyboard | Where-Object { $_.FriendlyName -match 'Hyper-V Virtual Keyboard' } | Select-Object -First 1
if (-not $kb) { throw 'Hyper-V Virtual Keyboard が見つからない' }
$key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$($kb.InstanceId)"
New-ItemProperty -Path $key -Name UpperFilters -PropertyType MultiString -Value @('awasekbf') -Force | Out-Null
pnputil /restart-device "$($kb.InstanceId)" | Write-Host
Start-Sleep -Seconds 4
$state = (sc.exe query awasekbf | Out-String)
Write-Host $state
if ($state -notmatch 'RUNNING') { throw 'awasekbf が RUNNING にならない' }
Write-Host 'awasekbf installed and running'
