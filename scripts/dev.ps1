$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build-frontend.ps1')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Push-Location (Split-Path -Parent $PSScriptRoot)
try { cargo run -p galgear } finally { Pop-Location }

