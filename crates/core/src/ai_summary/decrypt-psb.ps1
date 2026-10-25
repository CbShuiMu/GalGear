#requires -Version 5.1
[CmdletBinding(PositionalBinding = $false)]
param(
    [Parameter(Mandatory = $true)]
    [string] $ToolRoot
)

# Fixed native bridge. ToolRoot is supplied by the host, never by stdin.
# Protocol: exactly {"data":"canonical-base64","key":unsigned-decimal-u32}.
# Only standard PSB v2/v3 layouts with names immediately after the header qualify.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$maxBytes = 32 * 1024 * 1024
$maxBase64Chars = [int] ([Math]::Ceiling($maxBytes / 3.0) * 4)
$maxJsonChars = $maxBase64Chars + 4096
$inputStream = $null
$reader = $null
$outputStream = $null
$outputReader = $null

function Assert-NativeHeaderBounds {
    param([object] $Header, [int] $Length, [int] $Version)

    $fixedLength = if ($Version -eq 2) { 40 } else { 44 }
    if ($Header.Version -ne $Version -or
        [string]::new($Header.Signature) -cne "PSB`0" -or
        $Header.HeaderLength -ne $fixedLength -or
        $Header.OffsetNames -ne $fixedLength) {
        throw 'Invalid native header.'
    }

    foreach ($field in @('OffsetNames', 'OffsetStrings', 'OffsetStringsData',
                        'OffsetChunkOffsets', 'OffsetChunkLengths',
                        'OffsetChunkData', 'OffsetEntries')) {
        $offset = [long] $Header.$field
        if ($offset -lt $fixedLength -or $offset -gt $Length) {
            throw 'Native offset exceeds the bounded input.'
        }
    }

    # FreeMote reads this many bytes from the fixed header end before copying
    # the remaining bytes. Check before calling Encode to prevent a bad-key
    # header from causing a large ReadBytes allocation.
    $bodyLength = [long] $Header.OffsetChunkOffsets - [long] $Header.OffsetNames
    if ($bodyLength -lt 0 -or $bodyLength -gt ($Length - $fixedLength)) {
        throw 'Native body interval exceeds the bounded input.'
    }
}

try {
    $textBuilder = [System.Text.StringBuilder]::new()
    $textBuffer = [char[]]::new(8192)
    while (($readCount = [Console]::In.Read($textBuffer, 0, $textBuffer.Length)) -gt 0) {
        if ($textBuilder.Length + $readCount -gt $maxJsonChars) {
            throw 'Request exceeds the bounded protocol.'
        }
        [void] $textBuilder.Append($textBuffer, 0, $readCount)
    }

    # A fixed two-field wire grammar deliberately accepts only literal field
    # names, canonical Base64 characters and decimal integer keys. Anchoring
    # rejects duplicates, unknown fields, comments, extra values and coercions.
    $wirePattern = '\A[ \t\r\n]*\{[ \t\r\n]*(?:"data"[ \t\r\n]*:[ \t\r\n]*"(?<data>[A-Za-z0-9+/]*={0,2})"[ \t\r\n]*,[ \t\r\n]*"key"[ \t\r\n]*:[ \t\r\n]*(?<key>0|[1-9][0-9]{0,9})|"key"[ \t\r\n]*:[ \t\r\n]*(?<key>0|[1-9][0-9]{0,9})[ \t\r\n]*,[ \t\r\n]*"data"[ \t\r\n]*:[ \t\r\n]*"(?<data>[A-Za-z0-9+/]*={0,2})")[ \t\r\n]*\}[ \t\r\n]*\z'
    $wireRegex = [System.Text.RegularExpressions.Regex]::new(
        $wirePattern,
        [System.Text.RegularExpressions.RegexOptions]::CultureInvariant,
        [TimeSpan]::FromSeconds(5))
    $request = $wireRegex.Match($textBuilder.ToString())
    if (-not $request.Success) { throw 'Invalid fixed request.' }

    $encoded = $request.Groups['data'].Value
    if ($encoded.Length -gt $maxBase64Chars) { throw 'Input exceeds the bounded protocol.' }
    [uint32] $key = 0
    if (-not [uint32]::TryParse(
            $request.Groups['key'].Value,
            [System.Globalization.NumberStyles]::None,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [ref] $key)) {
        throw 'Invalid unsigned key.'
    }

    [byte[]] $inputBytes = [Convert]::FromBase64String($encoded)
    if ($inputBytes.Length -gt $maxBytes -or $inputBytes.Length -lt 40 -or
        [Convert]::ToBase64String($inputBytes) -cne $encoded) {
        throw 'Invalid bounded Base64 payload.'
    }
    if ($inputBytes[0] -ne 0x50 -or $inputBytes[1] -ne 0x53 -or
        $inputBytes[2] -ne 0x42 -or $inputBytes[3] -ne 0) {
        throw 'Invalid PSB signature.'
    }
    $version = [int] [BitConverter]::ToUInt16($inputBytes, 4)
    if (($version -ne 2 -and $version -ne 3) -or
        ($version -eq 3 -and $inputBytes.Length -lt 44) -or
        [BitConverter]::ToUInt16($inputBytes, 6) -gt 1) {
        throw 'Unsupported PSB version.'
    }

    # The model has no assembly, filename, offset, mode or position controls.
    foreach ($assemblyName in @('System.Runtime.CompilerServices.Unsafe.dll',
                                'System.Buffers.dll', 'System.Memory.dll',
                                'System.Numerics.Vectors.dll',
                                'Microsoft.IO.RecyclableMemoryStream.dll',
                                'ICSharpCode.SharpZipLib.dll', 'FreeMote.dll')) {
        [void] [System.Reflection.Assembly]::LoadFrom((Join-Path -Path $ToolRoot -ChildPath $assemblyName))
    }
    [FreeMote.Consts]::Verbose = $false

    $inputStream = [System.IO.MemoryStream]::new($inputBytes, $false)
    $reader = [System.IO.BinaryReader]::new($inputStream, [System.Text.Encoding]::UTF8, $true)
    $nativeHeader = [FreeMote.PsbHeader]::Load($reader)
    $headerEncrypted = [FreeMote.PsbFile]::TestHeaderEncrypted($inputStream, $nativeHeader)
    $bodyEncrypted = [FreeMote.PsbFile]::TestBodyEncrypted($reader, $nativeHeader)
    if (-not $headerEncrypted -and -not $bodyEncrypted) {
        throw 'Plaintext is not eligible for Auto decryption.'
    }

    $inputStream.Position = 0
    $boundedHeader = if ($headerEncrypted) {
        [FreeMote.PsbHeader]::Load($reader, $key)
    } else {
        $nativeHeader
    }
    Assert-NativeHeaderBounds -Header $boundedHeader -Length $inputBytes.Length -Version $version

    $outputStream = [System.IO.MemoryStream]::new($inputBytes.Length)
    [void] [FreeMote.PsbFile]::Encode(
        $key, [FreeMote.EncodeMode]::Decrypt, [FreeMote.EncodePosition]::Auto,
        $inputStream, $outputStream)
    [byte[]] $decoded = $outputStream.ToArray()
    if ($decoded.Length -ne $inputBytes.Length -or $decoded.Length -gt $maxBytes -or
        $decoded[0] -ne 0x50 -or $decoded[1] -ne 0x53 -or
        $decoded[2] -ne 0x42 -or $decoded[3] -ne 0 -or
        [BitConverter]::ToUInt16($decoded, 4) -ne $version -or
        [BitConverter]::ToUInt16($decoded, 6) -ne 0) {
        throw 'Decoded envelope failed validation.'
    }

    $outputStream.Position = 0
    $outputReader = [System.IO.BinaryReader]::new($outputStream, [System.Text.Encoding]::UTF8, $true)
    $decodedHeader = [FreeMote.PsbHeader]::Load($outputReader)
    Assert-NativeHeaderBounds -Header $decodedHeader -Length $decoded.Length -Version $version
    if ([FreeMote.PsbFile]::TestHeaderEncrypted($outputStream, $decodedHeader) -or
        [FreeMote.PsbFile]::TestBodyEncrypted($outputReader, $decodedHeader)) {
        throw 'Decoded native checks failed.'
    }

    # The fixed Python host must validate the complete typed tree and resource
    # spans before accepting this candidate output.
    [Console]::Out.Write([Convert]::ToBase64String($decoded))
}
catch {
    [Console]::Error.WriteLine('PSB_DECRYPT_REJECTED')
    exit 1
}
finally {
    if ($null -ne $outputReader) { $outputReader.Dispose() }
    if ($null -ne $reader) { $reader.Dispose() }
    if ($null -ne $outputStream) { $outputStream.Dispose() }
    if ($null -ne $inputStream) { $inputStream.Dispose() }
}
