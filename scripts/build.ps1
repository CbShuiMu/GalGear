$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
& (Join-Path $PSScriptRoot 'build-frontend.ps1') -Release
& (Join-Path $PSScriptRoot 'build-ai-agent.ps1')
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    cargo build -p galgear --release
    if ($LASTEXITCODE -ne 0) { throw '桌面应用编译失败。' }
    New-Item -ItemType Directory -Path artifacts -Force | Out-Null
    Copy-Item -LiteralPath target\release\galgear.exe -Destination artifacts\GalGear.exe -Force
    $skillSource = (Resolve-Path -LiteralPath (Join-Path $taskRoot 'skills\GalgameResourceExtract')).Path
    $skillRoot = Join-Path $taskRoot 'artifacts\skills\GalgameResourceExtract'
    Get-ChildItem -LiteralPath $skillSource -Force -Recurse -File |
        Where-Object { $_.FullName -notmatch '[\\/](__pycache__|\.pytest_cache|\.mypy_cache|\.ruff_cache)[\\/]' -and $_.Extension -notin '.pyc','.pyo' } |
        ForEach-Object {
            $relative = $_.FullName.Substring($skillSource.Length + 1)
            $destination = Join-Path $skillRoot $relative
            New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
            Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
        }
    $agentOutput = Join-Path $taskRoot 'artifacts\tools'
    New-Item -ItemType Directory -Path $agentOutput -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $taskRoot 'tools\ai-agent') -Destination $agentOutput -Recurse -Force
    Write-Host '已生成 artifacts\GalGear.exe'
} finally { Pop-Location }
