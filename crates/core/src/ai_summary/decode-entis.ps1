param([string]$Assembly, [string]$Jobs)
$ErrorActionPreference = 'Stop'
# Fixed host decoder. This source never contains AI output or API credentials.
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Reflection;
public static class GalGearNemesis {
 public static byte[] Decode(Type type, byte[] data, int offset, int count, int length) {
  if(length<0 || length>8388608) throw new InvalidDataException();
  using(var input = new MemoryStream(data,offset,count))
  using(var stream = (Stream)Activator.CreateInstance(type, BindingFlags.Instance|BindingFlags.Public|BindingFlags.NonPublic, null, new object[]{input,length}, null)) {
   var result = new byte[length]; int p=0;
   while(p<length) {int n=stream.Read(result,p,length-p);if(n<=0)throw new EndOfStreamException();p+=n;}
   return result;
  }
 }
}
'@
$type = [Reflection.Assembly]::LoadFrom($Assembly).GetType('GameRes.Formats.Entis.ErisaNemesisStream')
foreach ($job in (Get-Content -LiteralPath $Jobs -Raw | ConvertFrom-Json)) {
    $bytes = [IO.File]::ReadAllBytes($job.input)
    $base = if ($bytes.Length -gt 64 -and $bytes[0] -eq 66 -and $bytes[1] -eq 77) { [BitConverter]::ToUInt32($bytes,2) } else { 0 }
    $pos = [long]$base + 64
    while ($pos + 20 -le $bytes.Length) {
        $tag = [Text.Encoding]::ASCII.GetString($bytes,$pos,8)
        $size = [BitConverter]::ToInt64($bytes,$pos+8)
        if ($size -lt 0 -or $pos+16+$size -gt $bytes.Length) { throw 'Invalid context section' }
        if ($tag -eq 'context ') {
            $length = [BitConverter]::ToInt32($bytes,$pos+16)
            $decoded = [GalGearNemesis]::Decode($type,$bytes,($pos+20),($size-4),$length)
            # Retain the title section as well as decoded VM fields.
            $output = New-Object IO.MemoryStream
            $output.Write($bytes,$base,($pos-$base))
            $output.Write($decoded,0,$decoded.Length)
            [IO.File]::WriteAllBytes($job.output,$output.ToArray())
            $output.Dispose()
            break
        }
        $pos += 16 + $size
    }
    if (-not [IO.File]::Exists($job.output)) { throw 'Context not found' }
}
