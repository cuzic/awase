$ErrorActionPreference = 'Stop'

$packageArgs = @{
  packageName    = 'awase'
  fileType       = 'msi'
  silentArgs     = '/qn /norestart'
  validExitCodes = @(0, 3010, 1605, 1614, 1641)
}

[array]$key = Get-UninstallRegistryKey -SoftwareName 'awase*'

if ($key.Count -eq 1) {
  $key | ForEach-Object {
    $packageArgs['file'] = ''
    $packageArgs['silentArgs'] = "$($_.PSChildName) $($packageArgs['silentArgs'])"
    Uninstall-ChocolateyPackage @packageArgs
  }
} elseif ($key.Count -eq 0) {
  Write-Warning 'awase is not installed (no matching uninstall registry key found), nothing to do.'
} else {
  Write-Warning "$($key.Count) matches found for uninstall registry key 'awase*'. Please uninstall manually."
  $key | ForEach-Object { Write-Warning "- $($_.DisplayName) - $($_.PSChildName)" }
}
