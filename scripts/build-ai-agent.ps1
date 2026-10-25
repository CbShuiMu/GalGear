$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$agentSource = Join-Path $taskRoot 'crates\core\src\ai_summary\agent_host.py'
$agentOutput = Join-Path $taskRoot 'tools\ai-agent'
$agentBuild = Join-Path $taskRoot 'target\ai-agent-build'
New-Item -ItemType Directory -Path $agentOutput,$agentBuild -Force | Out-Null
$runtimeDirectory = Split-Path -Parent $agentSource
$pyInstallerArgs = @('--noconfirm', '--onefile', '--console')
# The host loads pure verifier modules by filename. Include every production
# helper so new verification capabilities also exist in the frozen runtime.
Get-ChildItem -LiteralPath $runtimeDirectory -File |
    Where-Object { $_.Extension -in '.py', '.ps1' -and $_.Name -notlike 'test_*' -and $_.FullName -ne $agentSource } |
    Sort-Object Name |
    ForEach-Object { $pyInstallerArgs += @('--add-data', "$($_.FullName);.") }
$pyInstallerArgs += @('--hidden-import', 'bz2', '--hidden-import', 'lzma', '--hidden-import', 'argparse',
    '--hidden-import', 'copy', '--hidden-import', 'datetime', '--hidden-import', 'difflib',
    '--hidden-import', 'functools', '--name', 'GalGear.AiAgent', '--distpath', $agentOutput,
    '--workpath', $agentBuild, '--specpath', $agentBuild, $agentSource)
python -m PyInstaller @pyInstallerArgs
if ($LASTEXITCODE -ne 0) { throw 'AI host build failed. Python and PyInstaller are required on the build machine.' }
