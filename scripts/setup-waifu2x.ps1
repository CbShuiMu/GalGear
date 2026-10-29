$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$version = '20250915'
$toolsPath = Join-Path $taskRoot 'tools'
$destination = Join-Path $toolsPath 'waifu2x'
$archive = Join-Path $toolsPath "waifu2x-ncnn-vulkan-$version-windows.zip"
New-Item -ItemType Directory -Path $toolsPath -Force | Out-Null
if (-not (Test-Path -LiteralPath $archive)) {
    Invoke-WebRequest -Uri "https://github.com/nihui/waifu2x-ncnn-vulkan/releases/download/$version/waifu2x-ncnn-vulkan-$version-windows.zip" -OutFile $archive
}
$expected = '7425be94b94e4c8f37a1e433ac0e0100c43790e2c37418f4b65d8235adfbdc87'
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw 'waifu2x archive checksum mismatch.'
}
if (-not (Test-Path -LiteralPath (Join-Path $destination 'waifu2x-ncnn-vulkan.exe'))) {
    $staging = Join-Path $toolsPath ('waifu2x-staging-' + [Guid]::NewGuid().ToString('N'))
    Expand-Archive -LiteralPath $archive -DestinationPath $staging
    $binary = Get-ChildItem -LiteralPath $staging -Filter 'waifu2x-ncnn-vulkan.exe' -Recurse | Select-Object -First 1
    if (-not $binary) { throw 'waifu2x executable missing from archive.' }
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    Get-ChildItem -LiteralPath $binary.DirectoryName | Copy-Item -Destination $destination -Recurse -Force
}
Write-Host "waifu2x $version ready: $destination"
