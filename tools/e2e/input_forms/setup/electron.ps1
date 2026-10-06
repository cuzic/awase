corepack enable
Push-Location tools\e2e\input_apps\electron
pnpm install
if ($LASTEXITCODE -ne 0) { throw 'pnpm install 失敗' }
Pop-Location
