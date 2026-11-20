param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "s4-tray"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class T {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
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

# The notification area is system-drawn, so GDI capture is valid here.
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$tw = 500; $th = 60
$bmp = New-Object System.Drawing.Bitmap $tw, $th
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Right - $tw, $b.Bottom - $th, 0, 0, (New-Object System.Drawing.Size $tw, $th))
$bmp.Save((Join-Path $shots "$Tag-taskbar.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()

$r = New-Object T+RECT
[T]::GetWindowRect($h, [ref]$r) | Out-Null
[T]::SetCursorPos($r.R - 20, $r.T + 15) | Out-Null
Start-Sleep -Milliseconds 200
[T]::mouse_event(0x0002, 0, 0, 0, 0)
[T]::mouse_event(0x0004, 0, 0, 0, 0)
Start-Sleep -Milliseconds 2500
$p.Refresh()
$aliveAfterClose = -not $p.HasExited
Write-Host ("{0}: window opened; alive after clicking system Close = {1}" -f $Tag, $aliveAfterClose)
if ($aliveAfterClose) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }