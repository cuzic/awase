# OpenJFX(JavaFX)の jar を Maven Central から取得し、JavaFxForm をコンパイルする。
$ErrorActionPreference = 'Stop'
$v = '21.0.5'
$lib = Join-Path $PWD 'tools\e2e\input_apps\javafx\lib'
New-Item -ItemType Directory -Force -Path $lib | Out-Null
foreach ($m in 'base', 'graphics', 'controls') {
  Invoke-WebRequest "https://repo1.maven.org/maven2/org/openjfx/javafx-$m/$v/javafx-$m-$v-win.jar" -OutFile "$lib\javafx-$m.jar"
}
javac -encoding UTF-8 --module-path $lib --add-modules javafx.controls -d tools\e2e\input_apps\javafx tools\e2e\input_apps\javafx\JavaFxForm.java
if ($LASTEXITCODE -ne 0) { throw 'javac 失敗' }
