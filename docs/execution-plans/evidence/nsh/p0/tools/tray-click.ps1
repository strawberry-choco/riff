param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "s4-tray-clicked"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class C {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, int e);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$shots = Join-Path $here "..\shots"

$p = Start-Process -FilePath $Exe -PassThru
$sw = [Diagnostics.Stopwatch]::StartNew()
$h = [IntPtr]::Zero
while ($sw.Elapsed.TotalSeconds -lt 20) {
    $p.Refresh()
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $h = $p.MainWindowHandle; break }
    Start-Sleep -Milliseconds 200
}
if ($h -eq [IntPtr]::Zero) { Write-Host "NO WINDOW"; Stop-Process -Id $p.Id -Force; exit 1 }
Start-Sleep -Milliseconds 2500
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds

function Click($x, $y) {
    [C]::SetCursorPos($x, $y) | Out-Null
    Start-Sleep -Milliseconds 250
    [C]::mouse_event(0x0002, 0, 0, 0, 0)
    [C]::mouse_event(0x0004, 0, 0, 0, 0)
    Start-Sleep -Milliseconds 700
}

Click ($b.Right - 187) ($b.Bottom - 33)
Click ($b.Right - 268) ($b.Bottom - 127)
[C]::keybd_event(0x1B, 0, 0, 0)
[C]::keybd_event(0x1B, 0, 2, 0)
Start-Sleep -Milliseconds 900

$r = New-Object C+RECT
[C]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $ht = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[C]::PrintWindow($h, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc); $g.Dispose()
$bmp.Save((Join-Path $shots "$Tag.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Host ("{0}: clicked overflow icon; window captured; alive={1}" -f $Tag, (-not $p.HasExited))
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue