corepack enable
Push-Location tools\e2e\input_apps\electron
pnpm install
# pnpm のバージョンによっては postinstall(Electron 本体のダウンロード)が許可されないので、無ければ自分で走らせる。
if (-not (Test-Path node_modules\electron\dist\electron.exe)) { node node_modules\electron\install.js }
if (-not (Test-Path node_modules\electron\dist\electron.exe)) { throw 'electron.exe を用意できない' }
Pop-Location
