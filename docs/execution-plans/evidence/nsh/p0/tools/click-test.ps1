param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [int]$X = 747,
    [int]$Y = 28,
    [string]$Tag = "click"
)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Click {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

$p = Start-Process -FilePath $Exe -PassThru
$sw = [Diagnostics.Stopwatch]::StartNew()
$h = [IntPtr]::Zero
while ($sw.Elapsed.TotalSeconds -lt 20) {
    $p.Refresh()
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $h = $p.MainWindowHandle; break }
    Start-Sleep -Milliseconds 200
}
if ($h -eq [IntPtr]::Zero) { Write-Host "NO WINDOW"; Stop-Process -Id $p.Id -Force; exit 1 }
Start-Sleep -Milliseconds 1200

$r = New-Object Click+RECT
[Click]::GetWindowRect($h, [ref]$r) | Out-Null
[Click]::SetCursorPos($r.L + $X, $r.T + $Y) | Out-Null
Start-Sleep -Milliseconds 200
[Click]::mouse_event(0x0002, 0, 0, 0, 0)
[Click]::mouse_event(0x0004, 0, 0, 0, 0)

$exited = $p.WaitForExit(6000)
Write-Host ("{0}: clicked window-relative ({1},{2}); process exited={3}" -f $Tag, $X, $Y, $exited)
if (-not $exited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }