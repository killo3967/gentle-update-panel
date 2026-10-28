# Captures the panel window without stealing focus or clicking anything.
# Tries PrintWindow(PW_RENDERFULLCONTENT) first so an occluded window still renders,
# then falls back to a screen copy if the result looks blank.
param(
    [string]$Out = ".smoke/window.png"
)

Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class Win {
    public delegate bool EnumProc(IntPtr handle, IntPtr param);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr handle, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr handle);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr handle, out RECT rect);
    [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr handle);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr handle, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr handle, IntPtr hdc, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    public static List<string> Find(uint pid) {
        var results = new List<string>();
        EnumWindows((handle, param) => {
            uint owner; GetWindowThreadProcessId(handle, out owner);
            if (owner != pid) return true;
            RECT rect; if (!GetWindowRect(handle, out rect)) return true;
            int length = GetWindowTextLength(handle);
            var text = new StringBuilder(length + 1);
            GetWindowText(handle, text, text.Capacity);
            results.Add(string.Join("|", new string[] {
                handle.ToString(), IsWindowVisible(handle).ToString(),
                rect.Left.ToString(), rect.Top.ToString(),
                (rect.Right - rect.Left).ToString(), (rect.Bottom - rect.Top).ToString(), text.ToString() }));
            return true;
        }, IntPtr.Zero);
        return results;
    }
}
"@

$process = Get-Process -Name "gentle-update-panel" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $process) { Write-Error "panel not running"; exit 1 }

$target = [Win]::Find([uint32]$process.Id) |
    ForEach-Object { $p = $_ -split '\|'; [pscustomobject]@{
        Handle = [intptr]::new([int64]$p[0]); Visible = $p[1]; Left = [int]$p[2];
        Top = [int]$p[3]; Width = [int]$p[4]; Height = [int]$p[5]; Title = $p[6] } } |
    Where-Object { $_.Title -ne "" -and $_.Width -gt 300 -and $_.Height -gt 300 } |
    Sort-Object { $_.Width * $_.Height } -Descending |
    Select-Object -First 1

if (-not $target) { Write-Error "panel window not found"; exit 1 }

$bitmap = New-Object System.Drawing.Bitmap($target.Width, $target.Height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$hdc = $graphics.GetHdc()
$printed = [Win]::PrintWindow($target.Handle, $hdc, 2)   # PW_RENDERFULLCONTENT
$graphics.ReleaseHdc($hdc)

if (-not $printed) {
    $graphics.CopyFromScreen($target.Left, $target.Top, 0, 0, $bitmap.Size)
}

$directory = Split-Path -Parent $Out
if ($directory) { New-Item -ItemType Directory -Force -Path $directory | Out-Null }
$bitmap.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose(); $bitmap.Dispose()

Write-Output "captured '$($target.Title)' $($target.Width)x$($target.Height) print=$printed -> $Out"
