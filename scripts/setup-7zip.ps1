$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$toolsPath = Join-Path $taskRoot 'tools'
$destination = Join-Path $toolsPath '7zip'
$version = '26.05'
$release = "https://github.com/ip7z/7zip/releases/download/$version"
$downloads = @(
    @{ Name = '7z2605-x64.exe'; File = '7z2605-x64.exe'; Hash = 'fd99e86f57d8f429ef5ed045bf98315af962e2309b0f1ba87a147a44c71647d0' },
    @{ Name = '7zr-bootstrap-2605.exe'; File = '7zr.exe'; Hash = '9bf582b894dfa665f852d7e9112b02290a1c303c2d95a21fcdc41ffcb03143b7' }
)
New-Item -ItemType Directory -Path $destination -Force | Out-Null
foreach ($download in $downloads) {
    $archivePath = Join-Path $toolsPath $download.Name
    if (-not (Test-Path -LiteralPath $archivePath)) {
        Invoke-WebRequest -Uri "$release/$($download.File)" -OutFile $archivePath
    }
    if ((Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $download.Hash) {
        throw "7-Zip checksum mismatch: $($download.Name)"
    }
}
# Extract only the CLI, codecs and license; never launch the installer.
& (Join-Path $toolsPath '7zr-bootstrap-2605.exe') x (Join-Path $toolsPath '7z2605-x64.exe') "-o$destination" -y 7z.exe 7z.dll License.txt readme.txt | Out-Null
if ($LASTEXITCODE -ne 0) { throw '7-Zip extraction failed.' }
foreach ($name in @('7z.exe', '7z.dll', 'License.txt', 'readme.txt')) {
    if (-not (Test-Path -LiteralPath (Join-Path $destination $name) -PathType Leaf)) {
        throw "Missing bundled 7-Zip file: $name"
    }
}
Write-Host "7-Zip $version ready: $destination"
