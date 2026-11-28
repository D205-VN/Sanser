# One JPEG of the primary screen, base64 on stdout. Run by exam_guard.rs.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System.Runtime.InteropServices;
public static class SanserDpi { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }
"@
[void][SanserDpi]::SetProcessDPIAware()
$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$full = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$graphics = [System.Drawing.Graphics]::FromImage($full)
$graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
$width = [Math]::Min([int]$env:SANSER_SHOT_WIDTH, $bounds.Width)
$height = [int]($bounds.Height * $width / $bounds.Width)
$small = New-Object System.Drawing.Bitmap $full, $width, $height
$encoder = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq 'image/jpeg' }
$parameters = New-Object System.Drawing.Imaging.EncoderParameters 1
$parameters.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), ([long]$env:SANSER_SHOT_QUALITY)
$stream = New-Object System.IO.MemoryStream
$small.Save($stream, $encoder, $parameters)
[Console]::Out.Write([Convert]::ToBase64String($stream.ToArray()))
