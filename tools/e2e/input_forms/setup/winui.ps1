dotnet publish tools\e2e\input_apps\winui\WinUiForm.csproj -c Release -p:Platform=x64 -o tools\e2e\input_apps\winui\out
if ($LASTEXITCODE -ne 0) { throw 'dotnet publish(WinUI 3)失敗' }
