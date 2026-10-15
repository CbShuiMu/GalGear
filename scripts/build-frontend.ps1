param([switch]$Release)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
Push-Location $taskRoot
try {
    $bindgen = Join-Path $taskRoot 'tools\wasm-bindgen.exe'
    if (-not (Test-Path -LiteralPath $bindgen)) { throw '请先运行 scripts\setup.ps1 下载 WebAssembly 构建工具。' }
    $profile = 'debug'
    if ($Release) { cargo build -p galgear-ui --target wasm32-unknown-unknown --release; $profile = 'release' }
    else { cargo build -p galgear-ui --target wasm32-unknown-unknown }
    if ($LASTEXITCODE -ne 0) { throw 'Rust 前端编译失败。' }
    New-Item -ItemType Directory -Path (Join-Path $taskRoot 'dist') -Force | Out-Null
    & $bindgen --target web --out-dir (Join-Path $taskRoot 'dist\pkg') --out-name galgear_ui (Join-Path $taskRoot "target\wasm32-unknown-unknown\$profile\galgear_ui.wasm")
    if ($LASTEXITCODE -ne 0) { throw 'WebAssembly 绑定生成失败。' }
    Copy-Item -LiteralPath (Join-Path $taskRoot 'web\index.html'),(Join-Path $taskRoot 'web\styles.css'),(Join-Path $taskRoot 'web\theme.css'),(Join-Path $taskRoot 'web\detail.css'),(Join-Path $taskRoot 'web\library.css'),(Join-Path $taskRoot 'web\appearance.css'),(Join-Path $taskRoot 'web\bridge.js') -Destination (Join-Path $taskRoot 'dist') -Force
    Copy-Item -LiteralPath (Join-Path $taskRoot 'web\assets') -Destination (Join-Path $taskRoot 'dist') -Recurse -Force
} finally { Pop-Location }

