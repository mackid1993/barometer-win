# SPDX-License-Identifier: GPL-3.0-only
#
# Barometer - a system monitor for the Windows taskbar
# Copyright (c) 2026 David Brustein

<#
.SYNOPSIS
    Pack assets\AppIcon.png into assets\barometer.ico.

.DESCRIPTION
    The artwork is the macOS icon, unchanged: a dark squircle, a cyan arch and
    a white trace across it. It is reused deliberately - the two apps are the
    same app on different desktops and should look it - so this script converts
    rather than draws.

    Two things have to happen on the way across.

    A macOS icon is authored inside a 1024 canvas with roughly a tenth of the
    frame left transparent on every side, because the system draws it that way.
    Windows does not: an icon is expected to fill its box, and left alone the
    Windows icon would sit visibly smaller than every other icon beside it in
    the taskbar. So the opaque bounds are measured and the margin is cropped
    away rather than assumed, which also means the script keeps working if the
    artwork is ever re-exported with different padding.

    And the small sizes are resampled from the full-resolution original every
    time, not from the size above. Chaining the downscales softens the trace
    into a gray smear by 16 pixels; going back to the 1024 for each one keeps
    the edges the artwork actually has.
#>

[CmdletBinding()]
param(
    [string]$Source,
    [string]$Out
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

# Resolved here rather than in the parameter defaults: $PSScriptRoot is not
# always populated by the time those are evaluated, and the failure is an
# unhelpful complaint about an empty string from Split-Path.
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent $root
if (-not $Source) { $Source = Join-Path $repo 'assets\AppIcon.png' }
if (-not $Out)    { $Out    = Join-Path $repo 'assets\barometer.ico' }

if (-not (Test-Path $Source)) { throw "Not found: $Source" }
# Named apart from the $Source parameter on purpose: PowerShell variables are
# case-insensitive, so $source *is* $Source, and assigning a bitmap to a
# parameter declared [string] silently coerces it back to a string.
$art = [System.Drawing.Bitmap]::FromFile($Source)

# The opaque bounds, so the transparent macOS margin goes and the artwork
# fills its box the way a Windows icon is expected to.
$minX = $art.Width; $minY = $art.Height; $maxX = -1; $maxY = -1
$rect = New-Object System.Drawing.Rectangle 0, 0, $art.Width, $art.Height
$data = $art.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly,
                         [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
try {
    $bytes = New-Object byte[] ($data.Stride * $art.Height)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
    for ($y = 0; $y -lt $art.Height; $y++) {
        $row = $y * $data.Stride
        for ($x = 0; $x -lt $art.Width; $x++) {
            # A threshold rather than any alpha at all: the artwork has a soft
            # drop shadow whose outermost pixels are almost transparent, and
            # counting those would find no margin to crop.
            if ($bytes[$row + $x * 4 + 3] -gt 24) {
                if ($x -lt $minX) { $minX = $x }
                if ($x -gt $maxX) { $maxX = $x }
                if ($y -lt $minY) { $minY = $y }
                if ($y -gt $maxY) { $maxY = $y }
            }
        }
    }
} finally {
    $art.UnlockBits($data)
}
if ($maxX -lt 0) { throw 'The source image is entirely transparent.' }

# Squared off around the center of what was found, so a shadow heavier on one
# side cannot make the icon lean.
$width  = $maxX - $minX + 1
$height = $maxY - $minY + 1
$side   = [Math]::Max($width, $height)
$cropX  = [Math]::Max(0, [int](($minX + $maxX) / 2 - $side / 2))
$cropY  = [Math]::Max(0, [int](($minY + $maxY) / 2 - $side / 2))
$side   = [Math]::Min($side, [Math]::Min($art.Width - $cropX, $art.Height - $cropY))
$crop   = New-Object System.Drawing.Rectangle $cropX, $cropY, $side, $side
Write-Host ("Cropped {0}x{1} to {2}x{2} at {3},{4}" -f $art.Width, $art.Height, $side, $cropX, $cropY)

# 256 down to 16. Everything Explorer, the taskbar, Alt+Tab and the installer
# ask for; anything else Windows scales from the nearest.
#
# The form differs by size, and that is not fussiness. PNG entries are read by
# the Windows shell at every size, but GDI+ cannot decode them at all - so a
# .NET tool, an installer builder or a screenshot script asking for the 32
# pixel image gets "Requested range extends past the end of the array" and no
# icon. The old DIB-with-mask form is read by everything. So the sizes anything
# might ask for are DIBs, and PNG is used only where the size makes the file
# worth compressing and only the shell will ever look.
$sizes = 256, 128, 64, 48, 40, 32, 24, 20, 16
$images = @()
foreach ($size in $sizes) {
    $bitmap = New-Object System.Drawing.Bitmap $size, $size,
        ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bitmap)
    $g.InterpolationMode  = 'HighQualityBicubic'
    $g.PixelOffsetMode    = 'HighQuality'
    $g.SmoothingMode      = 'HighQuality'
    $g.CompositingQuality = 'HighQuality'
    # Always from the original, never from the size above.
    $target = New-Object System.Drawing.Rectangle 0, 0, $size, $size
    $g.DrawImage($art, $target, $crop.X, $crop.Y, $crop.Width, $crop.Height,
                 [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()

    if ($size -ge 128) {
        $stream = New-Object System.IO.MemoryStream
        $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
        $blob = $stream.ToArray()
        $stream.Dispose()
    } else {
        # A DIB: a BITMAPINFOHEADER claiming twice the height, the pixels
        # bottom-up as BGRA, then a 1-bit AND mask. The doubled height is what
        # the format demands and is the single easiest thing to get wrong; the
        # mask is left all zeroes because the alpha channel already says what
        # is transparent, and Windows honors it.
        $rowBytes = $size * 4
        $maskStride = [int](([Math]::Floor(($size + 31) / 32)) * 4)
        $body = New-Object System.IO.MemoryStream
        $bw = New-Object System.IO.BinaryWriter $body
        $bw.Write([uint32]40)
        $bw.Write([int32]$size)
        $bw.Write([int32]($size * 2))
        $bw.Write([uint16]1)
        $bw.Write([uint16]32)
        $bw.Write([uint32]0)
        $bw.Write([uint32]($rowBytes * $size + $maskStride * $size))
        $bw.Write([int32]0); $bw.Write([int32]0)
        $bw.Write([uint32]0); $bw.Write([uint32]0)

        $rect2 = New-Object System.Drawing.Rectangle 0, 0, $size, $size
        $bits = $bitmap.LockBits($rect2, [System.Drawing.Imaging.ImageLockMode]::ReadOnly,
                                 [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $all = New-Object byte[] ($bits.Stride * $size)
            [System.Runtime.InteropServices.Marshal]::Copy($bits.Scan0, $all, 0, $all.Length)
            for ($y = $size - 1; $y -ge 0; $y--) {
                $bw.Write($all, $y * $bits.Stride, $rowBytes)
            }
        } finally {
            $bitmap.UnlockBits($bits)
        }
        $bw.Write((New-Object byte[] ($maskStride * $size)))
        $bw.Flush()
        $blob = $body.ToArray()
        $bw.Dispose(); $body.Dispose()
    }
    $bitmap.Dispose()
    $images += ,@{ Size = $size; Bytes = $blob }
}
$art.Dispose()

# The container, written by hand: ICONDIR, one ICONDIRENTRY each, then the
# images in the same order.
$file = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $file
$w.Write([uint16]0)                  # reserved
$w.Write([uint16]1)                  # 1 = icon
$w.Write([uint16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($image in $images) {
    # 256 is written as zero: the field is one byte and 256 does not fit.
    $dim = [byte]($(if ($image.Size -ge 256) { 0 } else { $image.Size }))
    $w.Write($dim); $w.Write($dim)
    $w.Write([byte]0)                # palette entries
    $w.Write([byte]0)                # reserved
    $w.Write([uint16]1)              # color planes
    $w.Write([uint16]32)             # bits per pixel
    $w.Write([uint32]$image.Bytes.Length)
    $w.Write([uint32]$offset)
    $offset += $image.Bytes.Length
}
foreach ($image in $images) { $w.Write($image.Bytes) }
$w.Flush()
[System.IO.File]::WriteAllBytes($Out, $file.ToArray())
$w.Dispose(); $file.Dispose()

Write-Host ("Wrote {0} ({1:N0} bytes, {2} sizes)" -f $Out, (Get-Item $Out).Length, $images.Count)
