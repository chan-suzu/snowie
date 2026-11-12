# Turns the generated illustrations (flat magenta background) into transparent, cropped PNGs for
# the site, and builds the favicon and the social preview card.
#   powershell -ExecutionPolicy Bypass -File scripts\site_assets.ps1 -Src <folder with out_*.png>
param([Parameter(Mandatory = $true)][string]$Src, [string]$Out = "")
# $PSScriptRoot is empty inside param defaults on Windows PowerShell 5.1
if (-not $Out) { $Out = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) "..\site\assets" }

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class Key {
    // Removes a flat magenta background: alpha from the distance to pure magenta, with the
    // magenta tint taken out of semi-transparent edge pixels.
    public static Bitmap Magenta(Bitmap src) {
        int w = src.Width, h = src.Height;
        var dst = new Bitmap(w, h, PixelFormat.Format32bppArgb);
        var sd = src.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        var dd = dst.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
        var px = new byte[w * h * 4];
        Marshal.Copy(sd.Scan0, px, 0, px.Length);
        for (int i = 0; i < px.Length; i += 4) {
            float b = px[i], g = px[i + 1], r = px[i + 2];
            // magenta = high red and blue, low green
            float m = Math.Min(r, b) - g;           // how magenta it is, 0..255
            float a = 1f - Clamp((m - 70f) / 120f);  // 1 = keep, 0 = background
            if (a < 1f && a > 0f) {
                // despill: pull red/blue down towards green by the amount of magenta
                float spill = (1f - a) * m;
                r = Clamp255(r - spill); b = Clamp255(b - spill);
            }
            px[i] = (byte)b; px[i + 1] = (byte)g; px[i + 2] = (byte)r; px[i + 3] = (byte)(a * 255f);
        }
        Marshal.Copy(px, 0, dd.Scan0, px.Length);
        src.UnlockBits(sd); dst.UnlockBits(dd);
        return dst;
    }
    public static Rectangle Bounds(Bitmap bmp) {
        int w = bmp.Width, h = bmp.Height, x0 = w, y0 = h, x1 = 0, y1 = 0;
        var d = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        var px = new byte[w * h * 4];
        Marshal.Copy(d.Scan0, px, 0, px.Length);
        bmp.UnlockBits(d);
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) if (px[(y * w + x) * 4 + 3] > 20) {
            if (x < x0) x0 = x; if (y < y0) y0 = y; if (x > x1) x1 = x; if (y > y1) y1 = y;
        }
        return x1 < x0 ? new Rectangle(0, 0, w, h) : Rectangle.FromLTRB(x0, y0, x1 + 1, y1 + 1);
    }
    static float Clamp(float v) { return v < 0 ? 0 : v > 1 ? 1 : v; }
    static float Clamp255(float v) { return v < 0 ? 0 : v > 255 ? 255 : v; }
}
"@

New-Item -ItemType Directory -Force $Out | Out-Null
function Save-Scaled($bmp, $rect, $max, $path, $square) {
    $scale = [Math]::Min(1.0, $max / [Math]::Max($rect.Width, $rect.Height))
    $w = [int]($rect.Width * $scale); $h = [int]($rect.Height * $scale)
    $cw = $w; $ch = $h
    if ($square) { $cw = [Math]::Max($w, $h); $ch = $cw }
    $out = New-Object System.Drawing.Bitmap $cw, $ch
    $g = [System.Drawing.Graphics]::FromImage($out)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $dx = [int](($cw - $w) / 2); $dy = $ch - $h
    $g.DrawImage($bmp, (New-Object System.Drawing.Rectangle $dx, $dy, $w, $h), $rect, [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()
    $out.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $out.Dispose()
}

$map = @{ hero = 'tems-hero'; throw = 'tems-throw'; snowman = 'tems-snowman'; sparkler = 'tems-sparkler'; sit = 'tems-sit' }
foreach ($k in $map.Keys) {
    $img = [System.Drawing.Bitmap]::FromFile((Join-Path $Src "out_$k.png"))
    $keyed = [Key]::Magenta($img)
    $rect = [Key]::Bounds($keyed)
    $max = if ($k -eq 'hero') { 900 } else { 520 }
    Save-Scaled $keyed $rect $max (Join-Path $Out "$($map[$k]).png") ($k -ne 'sit')
    if ($k -eq 'hero') { $heroKeyed = $keyed; $heroRect = $rect } else { $keyed.Dispose() }
    $img.Dispose()
    "{0,-10} -> {1}.png" -f $k, $map[$k]
}

# social preview card: night gradient, falling snow, Tems on the right, the name on the left
$og = New-Object System.Drawing.Bitmap 1200, 630
$g = [System.Drawing.Graphics]::FromImage($og)
$g.SmoothingMode = 'AntiAlias'; $g.TextRenderingHint = 'AntiAliasGridFit'; $g.InterpolationMode = 'HighQualityBicubic'
$bg = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.Point 0, 0), (New-Object System.Drawing.Point 0, 630), ([System.Drawing.Color]::FromArgb(20, 36, 80)), ([System.Drawing.Color]::FromArgb(8, 14, 32))
$g.FillRectangle($bg, 0, 0, 1200, 630)
$rnd = New-Object System.Random 7
for ($i = 0; $i -lt 160; $i++) {
    $r = 1 + $rnd.NextDouble() * 3.5; $a = [int](80 + $rnd.NextDouble() * 170)
    $g.FillEllipse((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb($a, 255, 255, 255))), [float]($rnd.NextDouble() * 1200), [float]($rnd.NextDouble() * 630), [float]$r, [float]$r)
}
$g.FillRectangle((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(248, 251, 255))), 0, 590, 1200, 40)
$th = 560; $tw = [int]($heroRect.Width * $th / $heroRect.Height)
$g.DrawImage($heroKeyed, (New-Object System.Drawing.Rectangle (1200 - $tw - 70), 40, $tw, $th), $heroRect, [System.Drawing.GraphicsUnit]::Pixel)
$white = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::White)
$lime = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(166, 240, 122))
$weak = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(190, 205, 235))
$g.DrawString("Snowie", (New-Object System.Drawing.Font "Segoe UI", 88, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)), $white, 70, 150)
$g.DrawString("Снег на рабочем столе Windows", (New-Object System.Drawing.Font "Segoe UI Semibold", 38, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)), $lime, 74, 270)
$g.DrawString("Сугробы на окнах · снежки · снеговики`nснегоуборщик · новогодний салют", (New-Object System.Drawing.Font "Segoe UI", 28, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)), $weak, 74, 340)
$g.Dispose()
$og.Save((Join-Path $Out "og.png"), [System.Drawing.Imaging.ImageFormat]::Png)
"og.png"

# favicon from the app icon
$ico = New-Object System.Drawing.Icon ("$PSScriptRoot\..\assets\snowie.ico"), 64, 64
$ico.ToBitmap().Save((Join-Path $Out "favicon.png"), [System.Drawing.Imaging.ImageFormat]::Png)
"favicon.png"
