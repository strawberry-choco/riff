param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "s7-bridge"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class B {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$shots = Join-Path $here "..\shots"
$logs = Join-Path $here "..\logs"

$p = Start-Process -FilePath $Exe -PassThru
$sw = [Diagnostics.Stopwatch]::StartNew()
$h = [IntPtr]::Zero
while ($sw.Elapsed.TotalSeconds -lt 30) {
    $p.Refresh()
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $h = $p.MainWindowHandle; break }
    Start-Sleep -Milliseconds 200
}
if ($h -eq [IntPtr]::Zero) { Write-Host "NO WINDOW"; Stop-Process -Id $p.Id -Force; exit 1 }
Start-Sleep -Milliseconds 2000
[B]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 500
$r = New-Object B+RECT
[B]::GetWindowRect($h, [ref]$r) | Out-Null

function Click($x, $y) {
    [B]::SetCursorPos($r.L + $x, $r.T + $y) | Out-Null
    Start-Sleep -Milliseconds 200
    [B]::mouse_event(0x0002, 0, 0, 0, 0)
    [B]::mouse_event(0x0004, 0, 0, 0, 0)
}
function Shot($name) {
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $ht
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [B]::PrintWindow($h, $hdc, 2) | Out-Null
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save((Join-Path $shots "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

Click 62 111
Start-Sleep -Milliseconds 3500
Shot "$Tag-playing"
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds 8 -Out (Join-Path $logs "$Tag-playing.csv") | Out-Host
Click 142 111
Start-Sleep -Milliseconds 1200
Shot "$Tag-paused"
Click 242 111
Start-Sleep -Milliseconds 1200
Shot "$Tag-seek"
Write-Host ("{0}: drove play, pause, seek; alive={1}" -f $Tag, (-not $p.HasExited))
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue