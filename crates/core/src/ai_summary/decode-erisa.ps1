param([Parameter(Mandatory=$true)][string]$Assembly)
$ErrorActionPreference = 'Stop'
# Fixed byte codec: accepts only compressed bytes and an output length on stdin.
# The caller selects the bundled assembly. There are no game paths or commands.
try {
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Reflection;
public static class GalGearByteCodec {
    public static byte[] Decode(Type type, byte[] data, int length) {
        if (length < 0 || length > 33554432 || data.Length > 33554432)
            throw new InvalidDataException();
        using (var input = new MemoryStream(data))
        using (var decoder = (Stream)Activator.CreateInstance(type,
            BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic,
            null, new object[] { input, length }, null)) {
            var output = new byte[length];
            int position = 0;
            while (position < length) {
                int count = decoder.Read(output, position, length - position);
                if (count <= 0) throw new EndOfStreamException();
                position += count;
            }
            return output;
        }
    }
}
'@
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $length = [long]$request.size
    if ($length -lt 0 -or $length -gt 33554432) { throw 'Invalid decoded size' }
    $bytes = [Convert]::FromBase64String([string]$request.data)
    if ($bytes.Length -gt 33554432) { throw 'Invalid input size' }
    $type = [Reflection.Assembly]::LoadFrom($Assembly).GetType('GameRes.Formats.Entis.ErisaNemesisStream', $true)
    $result = [GalGearByteCodec]::Decode($type,$bytes,[int]$length)
    [Console]::Out.Write([Convert]::ToBase64String($result))
} catch {
    [Console]::Error.Write('Invalid or unsupported erisa_nemesis stream')
    exit 1
}
