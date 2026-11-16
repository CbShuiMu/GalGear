param(
    [Parameter(Mandatory)][string]$MagpieDirectory,
    [string]$LocaleDirectory,
    [Parameter(Mandatory)][string]$MagpieConfig
)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$magpieSource = (Resolve-Path -LiteralPath $MagpieDirectory).Path
$configSource = (Resolve-Path -LiteralPath $MagpieConfig).Path
foreach ($file in @(
    (Join-Path $magpieSource 'Magpie.exe'), (Join-Path $magpieSource 'Microsoft.UI.Xaml.dll')
)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Required tool file missing: $file" }
}
$config = Get-Content -LiteralPath $configSource -Raw | ConvertFrom-Json
if (-not $config.shortcuts.scale -or -not $config.scalingModes.Count) { throw 'Magpie must have a scaling hotkey and at least one scaling mode.' }
foreach ($pair in ,@($magpieSource, 'magpie')) {
    $source = $pair[0]
    $destination = Join-Path $taskRoot ('tools\' + $pair[1])
    if ([IO.Path]::GetFullPath($destination).StartsWith($source.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Tool source must not contain the destination.' }
    Get-ChildItem -LiteralPath $source -Recurse -File |
        Where-Object { $_.FullName.Substring($source.Length + 1) -notmatch '^(logs|cache|config)[\\/]' } |
        ForEach-Object {
            $target = Join-Path $destination $_.FullName.Substring($source.Length + 1)
            New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
            Copy-Item -LiteralPath $_.FullName -Destination $target -Force
        }
}
$portableConfig = Join-Path $taskRoot 'tools\magpie\config'
New-Item -ItemType Directory -Path $portableConfig -Force | Out-Null
# Magpie 0.13 recognizes config/config.json beside its executable as portable mode.
# Keep the imported scaling modes while making direct Magpie launches elevated too.
$config | Add-Member -NotePropertyName 'alwaysRunAsAdmin' -NotePropertyValue $true -Force
$config | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath (Join-Path $portableConfig 'config.json') -Encoding utf8
# 保留旧参数供已有构建命令使用；旧 LE 安装目录不再作为内置二进制来源。
if ($LocaleDirectory) { Write-Host 'LocaleDirectory is no longer used; bundled LEP comes from the pinned official release.' }
& (Join-Path $PSScriptRoot 'setup-locale-emulator.ps1')
Write-Host 'Imported Magpie configuration and prepared LocaleEmulatorPlus in tools/.'
