$ErrorActionPreference = 'Stop'

$version = '1.10.0'

$packageArgs = @{
  packageName    = 'awase'
  fileType       = 'msi'
  url64bit       = "https://github.com/cuzic/awase/releases/download/v$version/awase-$version-x64.msi"
  checksum64     = 'B870BD1A5786F23B12D623FDA25AFDB36FBCABCBD076F10B0AC39311235EE2B3'
  checksumType64 = 'sha256'
  silentArgs     = '/quiet /qn /norestart'
  validExitCodes = @(0, 3010)
}

Install-ChocolateyPackage @packageArgs
