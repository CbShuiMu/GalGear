$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName System.Runtime.WindowsRuntime
Add-Type -AssemblyName System.Drawing
$null = [Windows.Storage.StorageFile,Windows.Storage,ContentType=WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapDecoder,Windows.Foundation,ContentType=WindowsRuntime]
$null = [Windows.Media.Ocr.OcrEngine,Windows.Foundation,ContentType=WindowsRuntime]
$null = [Windows.Globalization.Language,Windows.Foundation,ContentType=WindowsRuntime]
$asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetGenericArguments().Count -eq 1 -and $_.GetParameters().Count -eq 1 } | Select-Object -First 1
function Await-WinRT($operation, $resultType) {
    $task = $asTask.MakeGenericMethod($resultType).Invoke($null, @($operation))
    return $task.GetAwaiter().GetResult()
}
$job = [Console]::In.ReadToEnd() | ConvertFrom-Json
$language = [Windows.Globalization.Language]::new($job.language)
$engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage($language)
if ($null -eq $engine) { throw "Windows OCR language unavailable: $($job.language)" }
$results = @()
foreach ($item in $job.images) {
    $bitmap = $null; $stream = $null
    try {
        $file = Await-WinRT ([Windows.Storage.StorageFile]::GetFileFromPathAsync($item.path)) ([Windows.Storage.StorageFile])
        $stream = Await-WinRT ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
        $decoder = Await-WinRT ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
        $transform = [Windows.Graphics.Imaging.BitmapTransform]::new()
        $scale = [Math]::Min(3, [Math]::Floor(2000 / [Math]::Max($decoder.PixelWidth, $decoder.PixelHeight)))
        $scale = [Math]::Max(1, $scale)
        $transform.ScaledWidth = $decoder.PixelWidth * $scale
        $transform.ScaledHeight = $decoder.PixelHeight * $scale
        $bitmap = Await-WinRT ($decoder.GetSoftwareBitmapAsync([Windows.Graphics.Imaging.BitmapPixelFormat]::Bgra8, [Windows.Graphics.Imaging.BitmapAlphaMode]::Ignore, $transform, [Windows.Graphics.Imaging.ExifOrientationMode]::IgnoreExifOrientation, [Windows.Graphics.Imaging.ColorManagementMode]::DoNotColorManage)) ([Windows.Graphics.Imaging.SoftwareBitmap])
        $recognized = Await-WinRT ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
        $lines = @($recognized.Lines | ForEach-Object { @{ text=$_.Text; words=@($_.Words | ForEach-Object { @{ text=$_.Text; x=$_.BoundingRect.X / $scale; y=$_.BoundingRect.Y / $scale; width=$_.BoundingRect.Width / $scale; height=$_.BoundingRect.Height / $scale } }) } })
        $results += @{ id=$item.id; text=$recognized.Text; lines=$lines; scale=$scale }
    } finally {
        if ($null -ne $bitmap) { $bitmap.Dispose() }
        if ($null -ne $stream) { $stream.Dispose() }
    }
}
@{ language=$job.language; images=$results } | ConvertTo-Json -Depth 10 -Compress
