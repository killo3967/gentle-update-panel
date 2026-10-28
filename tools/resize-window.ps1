# Resizes the panel window (no focus needed) so taller views can be captured.
param(
    [int]$Width = 1116,
    [int]$Height = 1060
)

Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class WinSize {
    public delegate bool EnumProc(IntPtr handle, IntPtr param);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr handle, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr handle, out RECT rect);
    [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr handle);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr handle, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr handle, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    public static List<string> Find(uint pid) {
        var results = new List<string>();
        EnumWindows((handle, param) => {
            uint owner; GetWindowThreadProcessId(handle, out owner);
            if (owner != pid) return true;
            RECT rect; if (!GetWindowRect(handle, out rect)) return true;
            int len = GetWindowTextLength(handle);
            var text = new StringBuilder(len + 1);
            GetWindowText(handle, text, text.Capacity);
            results.Add(string.Join("|", new string[] { handle.ToString(), rect.Left.ToString(), rect.Top.ToString(),
                (rect.Right - rect.Left).ToString(), (rect.Bottom - rect.Top).ToString(), text.ToString() }));
            return true;
        }, IntPtr.Zero);
        return results;
    }
}
"@

$process = Get-Process -Name "gentle-update-panel" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $process) { Write-Error "panel not running"; exit 1 }

$target = [WinSize]::Find([uint32]$process.Id) |
    ForEach-Object { $p = $_ -split '\|'; [pscustomobject]@{
        Handle = [intptr]::new([int64]$p[0]); Left = [int]$p[1]; Top = [int]$p[2];
        Width = [int]$p[3]; Height = [int]$p[4]; Title = $p[5] } } |
    Where-Object { $_.Title -ne "" -and $_.Width -gt 300 } |
    Select-Object -First 1

if (-not $target) { Write-Error "window not found"; exit 1 }

# SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
[void][WinSize]::SetWindowPos($target.Handle, [IntPtr]::Zero, 0, 0, $Width, $Height, 0x0002 -bor 0x0004 -bor 0x0010)
Start-Sleep -Milliseconds 900
Write-Output "resized to ${Width}x${Height}"
