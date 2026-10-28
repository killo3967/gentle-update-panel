# Renders the app mark: a graphite tile with an amber "pulse" ring and a leading dot.
# Output: assets/icon.png (1024x1024) used by `npx tauri icon` to build every platform size.
Add-Type -AssemblyName System.Drawing

$size = 1024
$bmp = New-Object System.Drawing.Bitmap($size, $size)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g.Clear([System.Drawing.Color]::Transparent)

# Graphite tile with generous rounding so the mark reads at 16 px.
$r = 232
$path = New-Object System.Drawing.Drawing2D.GraphicsPath
$d = $r * 2
$path.AddArc(0, 0, $d, $d, 180, 90)
$path.AddArc($size - $d, 0, $d, $d, 270, 90)
$path.AddArc($size - $d, $size - $d, $d, $d, 0, 90)
$path.AddArc(0, $size - $d, $d, $d, 90, 90)
$path.CloseFigure()

$tileBrush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    (New-Object System.Drawing.Point(0, 0)),
    (New-Object System.Drawing.Point($size, $size)),
    [System.Drawing.Color]::FromArgb(255, 32, 29, 25),
    [System.Drawing.Color]::FromArgb(255, 14, 13, 12))
$g.FillPath($tileBrush, $path)

# Faint inner edge to keep the tile from looking flat on light taskbars.
$edge = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(40, 255, 255, 255), 6)
$g.DrawPath($edge, $path)

# Amber pulse ring: 300 degrees of arc, opening at the top-right.
$margin = 250
$diameter = $size - 2 * $margin
$amber = [System.Drawing.Color]::FromArgb(255, 240, 166, 60)
$ring = New-Object System.Drawing.Pen($amber, 108)
$ring.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
$ring.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$g.DrawArc($ring, $margin, $margin, $diameter, $diameter, -66, 300)

# Leading dot at the ring opening.
$dotSize = 132
$dotX = $size / 2 + [math]::Cos([math]::PI / 180 * -66) * ($diameter / 2) - $dotSize / 2
$dotY = $size / 2 + [math]::Sin([math]::PI / 180 * -66) * ($diameter / 2) - $dotSize / 2
$dotBrush = New-Object System.Drawing.SolidBrush($amber)
$g.FillEllipse($dotBrush, $dotX, $dotY, $dotSize, $dotSize)

$out = Join-Path (Split-Path -Parent $PSScriptRoot) "assets/icon.png"
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $out) | Out-Null
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)

$g.Dispose(); $bmp.Dispose(); $path.Dispose()
Write-Output "wrote $out"
