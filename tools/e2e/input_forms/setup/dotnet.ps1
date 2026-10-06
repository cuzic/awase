& "$PSScriptRoot\webview2.ps1"
dotnet publish tools\e2e\input_apps\dotnet\DotnetForm.csproj -c Release -o tools\e2e\input_apps\dotnet\out
if ($LASTEXITCODE -ne 0) { throw 'dotnet publish 失敗' }
