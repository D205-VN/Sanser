# Sanser exam screenshots for Windows. Started once by exam_guard.rs and kept
# running: each stdin line "width quality" is answered with one line of base64
# JPEG of the primary screen (empty on failure). Ends when stdin closes, which
# also happens when Sanser is closed or killed.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System.Runtime.InteropServices;
public static class SanserDpi { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }
"@
[void][SanserDpi]::SetProcessDPIAware()
$encoder = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq 'image/jpeg' }
while ($true) {
  $line = [Console]::In.ReadLine()
  if ($null -eq $line) { break }
  $answer = ''
  $full = $null; $graphics = $null; $small = $null; $stream = $null
  try {
    $parts = $line.Trim().Split(' ')
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $full = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($full)
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $width = [Math]::Min([int]$parts[0], $bounds.Width)
    $height = [int]($bounds.Height * $width / $bounds.Width)
    $small = New-Object System.Drawing.Bitmap $full, $width, $height
    $parameters = New-Object System.Drawing.Imaging.EncoderParameters 1
    $parameters.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), ([long]$parts[1])
    $stream = New-Object System.IO.MemoryStream
    $small.Save($stream, $encoder, $parameters)
    $answer = [Convert]::ToBase64String($stream.ToArray())
  } catch {
    $answer = ''
  } finally {
    foreach ($item in @($stream, $small, $graphics, $full)) { if ($item) { $item.Dispose() } }
  }
  [Console]::Out.WriteLine($answer)
  [Console]::Out.Flush()
}
