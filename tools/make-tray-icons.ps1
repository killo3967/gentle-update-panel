# Renders the two tray faces at 64 px (Windows scales down cleanly):
#   tray-idle.png   calm ring
#   tray-alert.png  ring plus a solid badge, visible even at 16 px
Add-Type -AssemblyName System.Drawing

$out = Join-Path (Split-Path -Parent $PSScriptRoot) "src-tauri/icons"
New-Item -ItemType Directory -Force -Path $out | Out-Null

function New-TrayIcon([string]$path, [bool]$alert) {
    $size = 64
    $bmp = New-Object System.Drawing.Bitmap($size, $size)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    $amber = [System.Drawing.Color]::FromArgb(255, 240, 166, 60)
    $margin = 6
    $diameter = $size - 2 * $margin
    $ring = New-Object System.Drawing.Pen($amber, $(if ($alert) { 7 } else { 6 }))
    $ring.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $ring.EndCap = [System.Drawing.Drawing2D.LineCap]::Round

    if ($alert) {
        $g.DrawEllipse($ring, $margin, $margin, $diameter, $diameter)
        # Badge sits bottom-right, inside the ring's bounding box, so it survives tiny sizes.
        $badge = 24
        $badgeBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 255, 92, 60))
        $g.FillEllipse($badgeBrush, $size - $badge - 2, $size - $badge - 2, $badge, $badge)
        $outline = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(255, 18, 17, 16), 4)
        $g.DrawEllipse($outline, $size - $badge - 2, $size - $badge - 2, $badge, $badge)
    } else {
        $g.DrawArc($ring, $margin, $margin, $diameter, $diameter, -60, 300)
        $dot = 12
        $dotX = $size / 2 + [math]::Cos([math]::PI / 180 * -60) * ($diameter / 2) - $dot / 2
        $dotY = $size / 2 + [math]::Sin([math]::PI / 180 * -60) * ($diameter / 2) - $dot / 2
        $dotBrush = New-Object System.Drawing.SolidBrush($amber)
        $g.FillEllipse($dotBrush, $dotX, $dotY, $dot, $dot)
    }

    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Output "wrote $path"
}

New-TrayIcon (Join-Path $out "tray-idle.png") $false
New-TrayIcon (Join-Path $out "tray-alert.png") $true
