$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build-frontend.ps1') -Release
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    cargo build -p galgear --release
    if ($LASTEXITCODE -ne 0) { throw '桌面应用编译失败。' }
    New-Item -ItemType Directory -Path artifacts -Force | Out-Null
    Copy-Item -LiteralPath target\release\galgear.exe -Destination artifacts\GalGear.exe -Force
    Write-Host '已生成 artifacts\GalGear.exe'
} finally { Pop-Location }
