param([string]$Source, [string]$Toolset, [switch]$Fixtures, [switch]$Update)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$commit = '9d4a59290076177a037f21784af31652bb5235ac'
$manifest = Join-Path $root 'tools/font-hook/adapter.json'
if (-not $Update -and (Test-Path -LiteralPath $manifest)) {
    $installed = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
    if ($installed.protocol -eq 2 -and $installed.upstream -match '^[0-9a-f]{40}$') { $commit = $installed.upstream }
}

if (-not $Source) { $Source = Join-Path $root 'tools/font-hook-source' }
if (-not (Test-Path -LiteralPath $Source)) {
    git clone https://github.com/SuQiandYing/SimpleFontHook.git $Source
    if ($LASTEXITCODE) { throw 'SimpleFontHook clone failed' }
}
$Source = (Resolve-Path -LiteralPath $Source).Path
if ($Update) {
    $remote = git ls-remote https://github.com/SuQiandYing/SimpleFontHook.git refs/heads/main
    if ($LASTEXITCODE -or -not $remote) { throw 'Cannot check SimpleFontHook update; existing components are preserved.' }
    $commit = ($remote -split '\s+')[0]
    if ($commit -notmatch '^[0-9a-f]{40}$') { throw 'Unexpected upstream revision' }
    git -C $Source cat-file -e ($commit + '^{commit}') 2>$null
    if ($LASTEXITCODE) {
        git -C $Source fetch --depth=1 https://github.com/SuQiandYing/SimpleFontHook.git main
        if ($LASTEXITCODE) { throw 'SimpleFontHook update failed; existing components are preserved.' }
        $commit = (git -C $Source rev-parse FETCH_HEAD).Trim()
        if ($LASTEXITCODE) { throw 'Cannot resolve updated commit' }
    }
}
# Export a clean commit, then apply the explicit runtime compatibility patch set.
$stage = Join-Path $root ('target/font-hook-build/' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$archive = Join-Path $stage 'source.zip'
git -c core.autocrlf=false -C $Source archive --format=zip "--output=$archive" $commit
if ($LASTEXITCODE) { throw 'Cannot export upstream commit' }
Expand-Archive -LiteralPath $archive -DestinationPath $stage
. (Join-Path $PSScriptRoot 'font-hook-compat.ps1')
$compatPaths = @(Add-FontHookCompatibility $stage $root)
$picker = Join-Path $stage 'SimpleFontHook/ui/font_picker.cpp'
$pickerText = [IO.File]::ReadAllText($picker)
$seam = '#include "internal/font_picker_lifecycle.cppinc"'
if (($pickerText.Split($seam).Count - 1) -ne 1) { throw 'Upstream UI boundary changed; adapter update required. Existing components are preserved.' }
$header = @'
#define NOMINMAX
#include "../galgear_protocol.h"
#include "../framework.h"
#include "../galgear_face.h"
extern "C" __declspec(dllexport) unsigned GalGearFontBridgeV2() { return 2; }
'@
[IO.File]::WriteAllText($picker, $header + [Environment]::NewLine + $pickerText.Replace($seam, '#include "internal/galgear_bridge.cppinc"'))
Copy-Item -LiteralPath (Join-Path $root 'native/font-hook/bridge.cpp') -Destination (Join-Path $stage 'SimpleFontHook/ui/internal/galgear_bridge.cppinc')
Copy-Item -LiteralPath (Join-Path $root 'native/font-hook/protocol.h') -Destination (Join-Path $stage 'SimpleFontHook/galgear_protocol.h')
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'Visual Studio C++ tools are required' }
if (-not $Toolset) {
    $Toolset = Get-ChildItem -Path (Join-Path $vs 'MSBuild/Microsoft/VC/v*/Platforms/x64/PlatformToolsets/v*') -Directory | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty Name
    if (-not $Toolset) { throw 'No C++ platform toolset found' }
}
$msbuild = Join-Path $vs 'MSBuild/Current/Bin/MSBuild.exe'
foreach ($arch in @('x86','x64')) {
    $platform = if ($arch -eq 'x86') { 'Win32' } else { 'x64' }
    & $msbuild (Join-Path $stage 'SimpleFontHook.sln') /t:Build /p:Configuration=Release "/p:Platform=$platform" "/p:PlatformToolset=$Toolset" /p:PostBuildEventUseInBuild=false /v:minimal
    if ($LASTEXITCODE) { throw "Hook compatibility adapter no longer compiles: $arch. Existing components are preserved." }
    $out = Join-Path $stage "components/$arch"
    New-Item -ItemType Directory -Path $out -Force | Out-Null
    $dll = if ($arch -eq 'x86') { 'Release/winmm.dll' } else { 'x64/Release/winmm.dll' }
    Copy-Item -LiteralPath (Join-Path $stage $dll) -Destination (Join-Path $out 'GalGearFontHook.dll')
    $batch = Join-Path $stage "loader-$arch.cmd"
    $vcvars = Join-Path $vs 'VC/Auxiliary/Build/vcvarsall.bat'
    $loader = Join-Path $root 'native/font-hook/loader.cpp'
    $exe = Join-Path $out 'font-hook-loader.exe'
    @('@echo off', ('call "{0}" {1} >nul' -f $vcvars,$arch), ('cl /nologo /std:c++17 /EHsc /O2 /MT /utf-8 /DUNICODE /D_UNICODE "{0}" /Fe:"{1}" /Fo:"{2}/loader-{3}.obj" /link user32.lib' -f $loader,$exe,$stage,$arch)) | Set-Content -LiteralPath $batch -Encoding ascii
    & $env:ComSpec /d /c $batch
    if ($LASTEXITCODE) { throw "Loader build failed: $arch" }
    if ($Fixtures) {
        $fixtureDir = Join-Path $root "target/font-fixtures/$arch"
        New-Item -ItemType Directory -Path $fixtureDir -Force | Out-Null
        $fixture = Join-Path $root 'native/font-hook/fixture.cpp'
        $fixtureExe = Join-Path $fixtureDir 'font-fixture.exe'
        @('@echo off', ('call "{0}" {1} >nul' -f $vcvars,$arch), ('cl /nologo /std:c++17 /EHsc /O2 /MT /utf-8 "{0}" /Fe:"{1}" /Fo:"{2}/fixture-{3}.obj" /link gdi32.lib user32.lib' -f $fixture,$fixtureExe,$stage,$arch)) | Set-Content -LiteralPath $batch -Encoding ascii
        & $env:ComSpec /d /c $batch
        if ($LASTEXITCODE) { throw "Fixture build failed: $arch" }
        $compatFixture = Join-Path $root 'native/font-hook/compat_fixture.cpp'
        $compatExe = Join-Path $fixtureDir 'compat-fixture.exe'
        @('@echo off', ('call "{0}" {1} >nul' -f $vcvars,$arch), ('cl /nologo /std:c++17 /EHsc /O2 /MT /utf-8 "{0}" /Fe:"{1}" /Fo:"{2}/compat-{3}.obj" /link user32.lib advapi32.lib' -f $compatFixture,$compatExe,$stage,$arch)) | Set-Content -LiteralPath $batch -Encoding ascii
        & $env:ComSpec /d /c $batch
        if ($LASTEXITCODE) { throw "Compatibility fixture build failed: $arch" }
        & $compatExe
        if ($LASTEXITCODE) { throw "Compatibility helper regression: $arch. Existing components are preserved." }
    }
}
# Verify untouched core files after compilation, before replacing either architecture.
$tree = git -C $Source ls-tree -r $commit -- SimpleFontHook/dllmain.cpp SimpleFontHook/utils.cpp SimpleFontHook/framework.h SimpleFontHook/hooks SimpleFontHook/font
if ($LASTEXITCODE) { throw 'Cannot read upstream core tree' }
$expectedHashes = @()
$corePaths = @()
foreach ($line in $tree) {
    $entry = $line -split "`t", 2
    if ($compatPaths -contains $entry[1]) { continue }
    $expectedHashes += ($entry[0] -split ' ')[2]
    $corePaths += Join-Path $stage $entry[1]
}
$actualHashes = @($corePaths | git hash-object --no-filters --stdin-paths)
if ($LASTEXITCODE -or $actualHashes.Count -ne $expectedHashes.Count) { throw 'Cannot hash staged core' }
for ($i = 0; $i -lt $expectedHashes.Count; $i++) {
    if ($expectedHashes[$i] -ne $actualHashes[$i]) { throw "Unexpected core modification: $($corePaths[$i])" }
}
$destination = Join-Path $root 'tools/font-hook'
New-Item -ItemType Directory -Path $destination -Force | Out-Null
foreach ($arch in @('x86','x64')) {
    Copy-Item -LiteralPath (Join-Path $stage "components/$arch") -Destination $destination -Recurse -Force
}
Copy-Item -LiteralPath (Join-Path $stage 'LICENSE.txt') -Destination (Join-Path $destination 'SimpleFontHook-LICENSE.txt') -Force
Copy-Item -LiteralPath (Join-Path $root 'docs/licenses/Detours-MIT.txt') -Destination (Join-Path $destination 'Detours-LICENSE.txt') -Force
@{ upstream = $commit; protocol = 2; buildFlavor = 'compatibility'; compatibilityRevision = 1; coreUnmodified = $false; patchedCoreFiles = $compatPaths } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'adapter.json') -Encoding utf8
Write-Host "Built compatibility adapter v2 for upstream $commit. Unpatched core files verified."
