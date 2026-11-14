param([string]$PythonExecutable = '')

$ErrorActionPreference = 'Stop'
# 随附 Siglus 0.5.7 声明 Python >= 3.12，且使用新版 hashlib.file_digest。构建成功不代表旧运行时
# 能载入这些源码，因此在打包前验证解释器与 PyInstaller，而非运行时交给模型绕行。
$pythonCandidates = @()
if ($PythonExecutable) {
    $pythonCandidates += $PythonExecutable
} else {
    $pythonCommand = Get-Command python -ErrorAction SilentlyContinue
    if ($pythonCommand) { $pythonCandidates += $pythonCommand.Source }
    $launcher = Get-Command py -ErrorAction SilentlyContinue
    if ($launcher) {
        $launcherPython = & $launcher.Source -3 -c 'import sys; print(sys.executable)' 2>$null
        if ($LASTEXITCODE -eq 0) { $pythonCandidates += $launcherPython }
    }
}
$selectedPython = $null
foreach ($candidate in ($pythonCandidates | Select-Object -Unique)) {
    & $candidate -c 'import sys; sys.exit(1) if sys.version_info < (3, 12) else None; import PyInstaller' 2>$null
    if ($LASTEXITCODE -eq 0) { $selectedPython = $candidate; break }
}
if (-not $selectedPython) { throw 'AI host requires Python >= 3.12 with PyInstaller. Use -PythonExecutable to select it.' }
Write-Host "Building AI host with $selectedPython"
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
& $selectedPython -m PyInstaller @pyInstallerArgs
if ($LASTEXITCODE -ne 0) { throw 'AI host build failed. Python and PyInstaller are required on the build machine.' }
