$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
foreach ($relative in @('tools\magpie\Magpie.exe', 'tools\magpie\config\config.json', 'tools\locale-emulator\LEProc.exe', 'tools\locale-emulator\LEConfig.xml')) {
    if (-not (Test-Path -LiteralPath (Join-Path $taskRoot $relative) -PathType Leaf)) {
        throw "Missing $relative. Run scripts/import-game-tools.ps1 with the installed tools and Magpie configuration first."
    }
}
& (Join-Path $PSScriptRoot 'setup-waifu2x.ps1')
& (Join-Path $PSScriptRoot 'setup-7zip.ps1')
& (Join-Path $PSScriptRoot 'build-frontend.ps1') -Release
& (Join-Path $PSScriptRoot 'build-ai-agent.ps1')
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    cargo build -p galgear --release
    if ($LASTEXITCODE -ne 0) { throw '桌面应用编译失败。' }
    New-Item -ItemType Directory -Path artifacts -Force | Out-Null
    Copy-Item -LiteralPath target\release\galgear.exe -Destination artifacts\GalGear.exe -Force
    # Keep downloaded skills and their commit marker across executable updates.
    # The runtime compares commits and downloads only a changed or missing package.
    $agentOutput = Join-Path $taskRoot 'artifacts\tools'
    New-Item -ItemType Directory -Path $agentOutput -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $taskRoot 'tools\ai-agent') -Destination $agentOutput -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $taskRoot 'tools\waifu2x') -Destination $agentOutput -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $taskRoot 'tools\7zip') -Destination $agentOutput -Recurse -Force
    foreach ($name in @('magpie', 'locale-emulator')) {
        $source = (Resolve-Path -LiteralPath (Join-Path $taskRoot "tools\$name")).Path
        Get-ChildItem -LiteralPath $source -Recurse -File | ForEach-Object {
            $relative = $_.FullName.Substring($source.Length + 1)
            $destination = Join-Path $agentOutput "$name\$relative"
            # Preserve settings changed in the output copy when rebuilding it.
            if ($relative -in @('config\config.json', 'LEConfig.xml') -and (Test-Path -LiteralPath $destination)) { return }
            New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
            Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
        }
    }
    $hash = (Get-FileHash -LiteralPath artifacts\GalGear.exe -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  GalGear.exe" | Set-Content -LiteralPath artifacts\GalGear.sha256 -Encoding ascii
    Copy-Item -LiteralPath (Join-Path $taskRoot 'docs\bundled-tools.txt') -Destination (Join-Path $taskRoot 'artifacts\内置工具说明.txt') -Force
    Write-Host '已生成 artifacts\GalGear.exe'
} finally { Pop-Location }
