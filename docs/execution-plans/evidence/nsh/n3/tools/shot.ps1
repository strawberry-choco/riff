param(
    [Parameter(Mandatory = $true)][int]$ProcId,
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$WaitSeconds = 20
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

$sw = [Diagnostics.Stopwatch]::StartNew()
$h = [IntPtr]::Zero
while ($sw.Elapsed.TotalSeconds -lt $WaitSeconds) {
    $p = Get-Process -Id $ProcId -ErrorAction SilentlyContinue
    if ($p -and $p.MainWindowHandle -ne [IntPtr]::Zero) { $h = $p.MainWindowHandle; break }
    Start-Sleep -Milliseconds 300
}
if ($h -eq [IntPtr]::Zero) { throw "no main window for pid $ProcId after $WaitSeconds s" }

[Win]::ShowWindow($h, 9) | Out-Null
[Win]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 1200

$r = New-Object Win+RECT
[Win]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L
$ht = $r.B - $r.T
if ($w -le 0 -or $ht -le 0) { throw "empty window rect $w x $ht" }

$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# PW_RENDERFULLCONTENT = 2 asks DirectComposition windows to paint themselves.
$ok = [Win]::PrintWindow($h, $hdc, 2)
$g.ReleaseHdc($hdc)
$g.Dispose()
if (-not $ok) { throw "PrintWindow failed for pid $ProcId" }
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

$title = (Get-Process -Id $ProcId).MainWindowTitle
Write-Host ("window={0}x{1} title='{2}' out={3}" -f $w, $ht, $title, $Out)