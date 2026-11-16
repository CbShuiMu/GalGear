# 固定官方 LEP 发布包；不安装右键扩展，不启动安装器或独立更新器。
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$version = 'v2.0.6.0'
$expectedHash = '21022a8a11922b8ffb0763ab479c8d4e353d21ea61f7c2b956a8afb25a431f12'
$repository = 'https://github.com/julixian/LocaleEmulatorPlus'
$destination = Join-Path $taskRoot 'tools/locale-emulator'
$required = @('LEPProc_x86.exe', 'LEPProc_x64.exe', 'LEPCommonLibrary.dll', 'LEPGUI.exe',
    'LoaderDll_x86.dll', 'LoaderDll_x64.dll', 'LocaleEmulatorPlus_x86.dll', 'LocaleEmulatorPlus_x64.dll',
    'LEPVersion.xml', 'COPYING', 'COPYING.LESSER', 'README_ORG.md')
$manifest = Join-Path $destination 'bundle.json'
if (Test-Path -LiteralPath $manifest) {
    $installed = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
    $missing = @($required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $destination $_) -PathType Leaf) })
    if ($installed.version -eq $version -and $installed.sha256 -eq $expectedHash -and -not $missing.Count) {
        Write-Host "LocaleEmulatorPlus $version ready: $destination"
        return
    }
}
$cache = Join-Path $taskRoot 'target/locale-emulator-plus'
New-Item -ItemType Directory -Path $cache -Force | Out-Null
$archive = Join-Path $cache 'LocaleEmulatorPlus.zip'
if (-not (Test-Path -LiteralPath $archive)) {
    Invoke-WebRequest "$repository/releases/download/$version/LocaleEmulatorPlus.zip" -OutFile $archive -TimeoutSec 120
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHash) {
    throw 'LocaleEmulatorPlus archive checksum mismatch; existing tools are preserved.'
}
$stage = Join-Path $cache ([guid]::NewGuid().ToString('N'))
Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $stage 'unpacked')
$source = Join-Path $stage 'unpacked/LocaleEmulatorPlus'
$ready = Join-Path $stage 'ready'
New-Item -ItemType Directory -Path $ready -Force | Out-Null
foreach ($name in $required) {
    $target = Join-Path $ready $name
    if ($name -in @('COPYING', 'COPYING.LESSER', 'README_ORG.md')) {
        Invoke-WebRequest "https://raw.githubusercontent.com/julixian/LocaleEmulatorPlus/$version/$name" -OutFile $target -TimeoutSec 30
    } else {
        Copy-Item -LiteralPath (Join-Path $source $name) -Destination $target
    }
}
Copy-Item -LiteralPath (Join-Path $source 'Lang') -Destination $ready -Recurse
# 用户配置只留在本机；首次启动由 core 迁移旧 XML，发行包不携带本机配置。
foreach ($name in @('LEConfig.xml', 'LEPConfig.xml')) {
    $existing = Join-Path $destination $name
    if (Test-Path -LiteralPath $existing) { Copy-Item -LiteralPath $existing -Destination $ready }
}
@{ version = $version; sha256 = $expectedHash; source = "$repository/tree/$version" } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $ready 'bundle.json') -Encoding utf8
# 先完整校验再替换目录；旧工具保留在 target 中，失败时恢复。
$backup = Join-Path $stage 'previous'
$workspace = [IO.Path]::GetFullPath($taskRoot).TrimEnd('\') + '\'
foreach ($path in @($destination, $ready, $backup)) {
    if (-not [IO.Path]::GetFullPath($path).StartsWith($workspace, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Tool staging path outside workspace: $path"
    }
}
New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
if (Test-Path -LiteralPath $destination) { Move-Item -LiteralPath $destination -Destination $backup }
try { Move-Item -LiteralPath $ready -Destination $destination }
catch {
    if (Test-Path -LiteralPath $backup) { Move-Item -LiteralPath $backup -Destination $destination }
    throw
}
Write-Host "LocaleEmulatorPlus $version installed: $destination"
