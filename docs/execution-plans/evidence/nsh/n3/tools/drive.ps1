# N3 windowed drive: start the riff-nsh binary over the synthetic store,
# press play, capture the readout mid-playback, stop with the queue parked,
# and sample idle CPU. Keyboard drives the readout's debug keys
# (mod+alt+p play, mod+alt+o stop); shots ride PrintWindow like P0's tools.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "n3"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class N3 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$shots = Join-Path $here "..\shots"
$logs = Join-Path $here "..\logs"
New-Item -ItemType Directory -Force -Path $shots, $logs | Out-Null

$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.UseShellExecute = $false
$psi.EnvironmentVariables["RUST_LOG"] = "info"
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$p = [Diagnostics.Process]::Start($psi)
$stdout = Join-Path $logs "$Tag.stdout.log"
$stderr = Join-Path $logs "$Tag.stderr.log"

$sw = [Diagnostics.Stopwatch]::StartNew()
$h = [IntPtr]::Zero
while ($sw.Elapsed.TotalSeconds -lt 30) {
    if ($p.HasExited) { break }
    $p.Refresh()
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 200
}
if ($h -eq [IntPtr]::Zero -and -not $p.HasExited) { $p.Refresh(); $h = $p.MainWindowHandle }
if ($p.HasExited) {
    $p.StandardOutput.ReadToEnd() | Set-Content $stdout
    $p.StandardError.ReadToEnd() | Set-Content $stderr
    Write-Host "EXITED early code=$($p.ExitCode)"
    Get-Content $stderr -Tail 25
    exit 1
}
Write-Host ("window after {0} ms" -f $sw.ElapsedMilliseconds)

[N3]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 1500

function Click($x, $y) {
    [N3]::SetCursorPos($r.L + $x, $r.T + $y) | Out-Null
    Start-Sleep -Milliseconds 200
    [N3]::mouse_event(0x0002, 0, 0, 0, 0)
    [N3]::mouse_event(0x0004, 0, 0, 0, 0)
}

$r = New-Object N3+RECT
[N3]::GetWindowRect($h, [ref]$r) | Out-Null

# Play a random session: the engine fills the queue from the 50k store.
Click 57 251
Start-Sleep -Milliseconds 6000

[N3]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $ht = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
$ok = [N3]::PrintWindow($h, $hdc, 2)
$g.ReleaseHdc($hdc); $g.Dispose()
if (-not $ok) { throw "PrintWindow failed" }
$bmp.Save((Join-Path $shots "$Tag-readout.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Host "shot saved: $Tag-readout.png (${w}x${ht})"

# Stop with the queue parked: the idle scenario the spike sampled
# (pump running, playback stopped, 50k queue loaded).
Click 268 251
Start-Sleep -Milliseconds 2000
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds 20 -Out (Join-Path $logs "$Tag-idle.csv") | Out-Host

Start-Sleep -Milliseconds 500
$alive = -not $p.HasExited
Write-Host "alive=$alive"
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
$p.StandardOutput.ReadToEnd() | Set-Content $stdout
$p.StandardError.ReadToEnd() | Set-Content $stderr
