# Turns the generated sprite sheets (flat magenta background) into trimmed transparent PNGs:
# assets/sprites/*.png for the app (embedded into the exe) and site/assets/sprites/ for the web demo.
#   powershell -ExecutionPolicy Bypass -File scripts\sprites.ps1 -Src <folder with out_*.png>
param([Parameter(Mandatory = $true)][string]$Src)
$root = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) ".."
$outs = @((Join-Path $root "assets\sprites"), (Join-Path $root "site\assets\sprites"))

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class Spr {
    public static Bitmap Magenta(Bitmap src) {
        int w = src.Width, h = src.Height;
        var dst = new Bitmap(w, h, PixelFormat.Format32bppArgb);
        var sd = src.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        var dd = dst.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
        var px = new byte[w * h * 4];
        Marshal.Copy(sd.Scan0, px, 0, px.Length);
        for (int i = 0; i < px.Length; i += 4) {
            float b = px[i], g = px[i + 1], r = px[i + 2];
            float m = Math.Min(r, b) - g;
            float a = 1f - Clamp((m - 70f) / 120f);
            if (a < 1f && a > 0f) { float spill = (1f - a) * m; r = Clamp255(r - spill); b = Clamp255(b - spill); }
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

# largest side in pixels: about twice the size they are drawn at, so they stay crisp at 200% scale
$sizes = [ordered]@{
    plow = 384; box = 224; arm = 160; nose = 112
    hat = 192; hat_santa = 192; hat_beanie = 192; hat_bucket = 192
    scarf = 224; scarf_blue = 224; scarf_green = 224
    face = 160; face_wink = 160; face_grin = 160
}
foreach ($o in $outs) { New-Item -ItemType Directory -Force $o | Out-Null }
foreach ($name in $sizes.Keys) {
    $img = [System.Drawing.Bitmap]::FromFile((Join-Path $Src "out_$name.png"))
    $keyed = [Spr]::Magenta($img)
    $rect = [Spr]::Bounds($keyed)
    $max = $sizes[$name]
    $k = [Math]::Min(1.0, $max / [Math]::Max($rect.Width, $rect.Height))
    $w = [Math]::Max(1, [int]($rect.Width * $k)); $h = [Math]::Max(1, [int]($rect.Height * $k))
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $gr = [System.Drawing.Graphics]::FromImage($bmp)
    $gr.InterpolationMode = 'HighQualityBicubic'; $gr.PixelOffsetMode = 'HighQuality'
    $gr.DrawImage($keyed, (New-Object System.Drawing.Rectangle 0, 0, $w, $h), $rect, [System.Drawing.GraphicsUnit]::Pixel)
    $gr.Dispose()
    foreach ($o in $outs) { $bmp.Save((Join-Path $o "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png) }
    "{0,-6} {1}x{2}" -f $name, $w, $h
    $bmp.Dispose(); $keyed.Dispose(); $img.Dispose()
}
