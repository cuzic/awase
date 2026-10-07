# install-qt-action で Qt が入った後。line.exe(LINE と同じプロセス名)を作り、DLL を同じ場所へ集める。
cmake -S tools\e2e\input_apps\qt -B qt-build -DCMAKE_BUILD_TYPE=Release
cmake --build qt-build --config Release
New-Item -ItemType Directory -Force -Path tools\e2e\input_apps\qt\out | Out-Null
Copy-Item qt-build\Release\line.exe tools\e2e\input_apps\qt\out\
windeployqt --release --no-translations --no-opengl-sw --no-system-d3d-compiler --no-compiler-runtime tools\e2e\input_apps\qt\out\line.exe
