param([Parameter(Mandatory=$true)][string]$Assembly, [switch]$Persistent)
$ErrorActionPreference = 'Stop'
# Fixed pure math bridge: no model-supplied assembly, type, method or file paths.
try {
    [void][Reflection.Assembly]::LoadFrom($Assembly)
    function Invoke-Reference($request) {
    $result = [Collections.Generic.List[object]]::new()
    foreach ($item in $request.items) {
        if ($request.algorithm -eq 'garbro_crc32normal_table') {
            $result.Add([GameRes.Utility.Crc32Normal]::Table)
        } elseif ($request.algorithm -eq 'garbro_mt19937') {
            $generator = [GameRes.Cryptography.MersenneTwister]::new([uint32]$item.seed)
            $numbers = [Collections.Generic.List[uint32]]::new()
            for ($i = 0; $i -lt [int]$item.count; $i++) { $numbers.Add($generator.Rand()) }
            $result.Add($numbers.ToArray())
        } elseif ($request.algorithm -eq 'garbro_blowfish') {
            $key = [Convert]::FromBase64String([string]$item.key)
            $bytes = [Convert]::FromBase64String([string]$item.data)
            $cipher = [GameRes.Cryptography.Blowfish]::new($key)
            if ($item.encrypt) { $cipher.Encipher($bytes, $bytes.Length) }
            else { $cipher.Decipher($bytes, $bytes.Length) }
            $result.Add([Convert]::ToBase64String($bytes))
        } else { throw 'Unsupported reference primitive' }
    }
        return @{result=$result.ToArray()}
    }
    if ($Persistent) {
        while ($null -ne ($line = [Console]::In.ReadLine())) {
            try {
                $reply = Invoke-Reference ($line | ConvertFrom-Json)
            } catch { $reply = @{error=$_.Exception.Message} }
            [Console]::Out.WriteLine((ConvertTo-Json -InputObject $reply -Depth 6 -Compress))
            [Console]::Out.Flush()
        }
    } else {
        $reply = Invoke-Reference ([Console]::In.ReadToEnd() | ConvertFrom-Json)
        [Console]::Out.Write((ConvertTo-Json -InputObject $reply.result -Depth 4 -Compress))
    }
} catch {
    [Console]::Error.Write('Reference primitive failed: ' + $_.Exception.Message)
    exit 1
}
