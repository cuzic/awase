# JetBrains Runtime(IntelliJ 系 IDE が使う、IME 周りに独自修正の入った JDK)を tools\e2e\input_apps\jbr\jbr に展開する。
$ErrorActionPreference = 'Stop'
$dest = Join-Path $PWD 'tools\e2e\input_apps\jbr'
New-Item -ItemType Directory -Force -Path $dest | Out-Null
$url = 'https://cache-redirector.jetbrains.com/intellij-jbr/jbr-21.0.8-windows-x64-b1138.52.tar.gz'
Invoke-WebRequest $url -OutFile "$dest\jbr.tar.gz"
tar -xzf "$dest\jbr.tar.gz" -C $dest
$jbr = Get-ChildItem $dest -Directory | Where-Object { Test-Path "$($_.FullName)\bin\java.exe" } | Select-Object -First 1
if (-not $jbr) { throw 'JBR を展開できなかった' }
Rename-Item $jbr.FullName "$dest\jbr"
& "$dest\jbr\bin\java.exe" -version
javac -encoding UTF-8 -d tools\e2e\java_forms tools\e2e\java_forms\JavaForm.java
if ($LASTEXITCODE -ne 0) { throw 'javac 失敗' }
