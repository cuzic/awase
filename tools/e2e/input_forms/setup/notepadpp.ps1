choco install notepadplusplus -y --no-progress 2>&1 | Select-Object -Last 3
Get-ChildItem 'C:\Program Files\Notepad++\notepad++.exe'
