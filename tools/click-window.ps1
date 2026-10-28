# Clicks a point inside the panel window and captures the result.
param(
    [int]$X = 60,
    [int]$Y = 285,
    [string]$Out = ".smoke/click.png",
    [int]$WaitMs = 900
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
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr handle);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr handle, int command);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    public const uint LEFTDOWN = 0x0002;
    public const uint LEFTUP = 0x0004;

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
    Where-Object { $_.Visible -eq "True" -and $_.Width -gt 300 } |
    Sort-Object { $_.Width * $_.Height } -Descending |
    Select-Object -First 1

if (-not $target) { Write-Error "panel window not found"; exit 1 }

[void][Win]::ShowWindow($target.Handle, 9)
[void][Win]::SetForegroundWindow($target.Handle)
Start-Sleep -Milliseconds 500

$screenX = $target.Left + $X
$screenY = $target.Top + $Y
[void][Win]::SetCursorPos($screenX, $screenY)
Start-Sleep -Milliseconds 120
[Win]::mouse_event([Win]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 60
[Win]::mouse_event([Win]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds $WaitMs

$bitmap = New-Object System.Drawing.Bitmap($target.Width, $target.Height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($target.Left, $target.Top, 0, 0, $bitmap.Size)
$directory = Split-Path -Parent $Out
if ($directory) { New-Item -ItemType Directory -Force -Path $directory | Out-Null }
$bitmap.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose(); $bitmap.Dispose()

Write-Output "clicked ($X,$Y) -> screen ($screenX,$screenY); captured $Out"
