# N5 calibration: launch the shell over the fixture store at a known frame
# and capture one shot, so the sidebar drive's coordinates are measured, not
# guessed. Not an evidence artifact beyond the log of its run.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Out = "..\shots\n5-calibrate.png"
)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WinCal {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int ht, uint f);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.UseShellExecute = $false
$psi.EnvironmentVariables["RUST_LOG"] = "info"
$psi.EnvironmentVariables["RIFF_N3_STORE"] = Join-Path $PSScriptRoot "..\..\..\..\..\..\.scratch\nsh-n3\store\riff.sqlite3"
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$p = [Diagnostics.Process]::Start($psi)
$hwnd = [IntPtr]::Zero
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($sw.Elapsed.TotalSeconds -lt 30) {
    $p.Refresh()
    if ($p.HasExited) { break }
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $hwnd = $p.MainWindowHandle; break }
    Start-Sleep -Milliseconds 200
}
if ($hwnd -eq [IntPtr]::Zero) { throw "no window after 30 s" }

[WinCal]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Milliseconds 1500
[System.Windows.Forms.SendKeys]::SendWait("^%r")
Start-Sleep -Milliseconds 2000
[WinCal]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, 1200, 800, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2000

$r = New-Object WinCal+RECT
[WinCal]::GetWindowRect($hwnd, [ref]$r) | Out-Null
$bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[WinCal]::PrintWindow($hwnd, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc); $g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Host ("pid={0} window={1}x{2} out={3}" -f $p.Id, ($r.R - $r.L), ($r.B - $r.T), $Out)
Write-Host ("PID {0}" -f $p.Id)
$p.StandardOutput.ReadToEnd() | Set-Content "..\logs\n5-calibrate.stdout.log"
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
