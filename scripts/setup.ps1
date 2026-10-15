$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$toolsPath = Join-Path $taskRoot 'tools'
New-Item -ItemType Directory -Path $toolsPath -Force | Out-Null
$version = '0.2.127'
$asset = "wasm-bindgen-$version-x86_64-pc-windows-msvc.tar.gz"
$base = "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$version"
$archive = Join-Path $toolsPath $asset
if (-not (Test-Path -LiteralPath (Join-Path $toolsPath 'wasm-bindgen.exe'))) {
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile $archive
    $checksum = (Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset.sha256sum").Content
    if ($checksum -is [byte[]]) { $checksum = [Text.Encoding]::UTF8.GetString($checksum) }
    $expected = ($checksum -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw 'wasm-bindgen 下载校验失败。' }
    tar -xzf $archive -C $toolsPath
    if ($LASTEXITCODE -ne 0) { throw 'wasm-bindgen 解压失败。' }
    $executable = Get-ChildItem -LiteralPath $toolsPath -Filter wasm-bindgen.exe -Recurse | Select-Object -First 1
    Copy-Item -LiteralPath $executable.FullName -Destination (Join-Path $toolsPath 'wasm-bindgen.exe') -Force
}
rustup target add wasm32-unknown-unknown
if ($LASTEXITCODE -ne 0) { throw 'WebAssembly 编译目标安装失败。' }
Write-Host '构建工具已准备好。'

