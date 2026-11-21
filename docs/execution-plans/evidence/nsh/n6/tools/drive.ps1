# N6 windowed drive: the All Tracks virtual list, the selection-driven
# inspector and the covers over the RGBA path, live on the fixture store
# (never the live store). Captures: the list at rest (covers rendering),
# the inspector with a selection, a deep jump + wheel-scroll step series
# across albums (covers held, RSS sampled), idle CPU with the list live,
# and scroll CPU with wheel steps driven. Sampling runs alone: the CPU
# phases are separate from the input phases.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "n6"
)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class N6 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int ht, uint f);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$shots = Join-Path $here "..\shots"
$logs = Join-Path $here "..\logs"
New-Item -ItemType Directory -Force -Path $shots, $logs | Out-Null

function Shot($h, $name) {
    $r = New-Object N6+RECT
    [N6]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $ht
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $ok = [N6]::PrintWindow($h, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    if (-not $ok) { throw "PrintWindow failed" }
    $bmp.Save((Join-Path $shots "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "shot saved: $name.png (${w}x${ht})"
}

function Click($h, $x, $y) {
    $r = New-Object N6+RECT
    [N6]::GetWindowRect($h, [ref]$r) | Out-Null
    [N6]::SetCursorPos($r.L + $x, $r.T + $y) | Out-Null
    Start-Sleep -Milliseconds 200
    [N6]::mouse_event(0x0002, 0, 0, 0, 0)
    Start-Sleep -Milliseconds 60
    [N6]::mouse_event(0x0004, 0, 0, 0, 0)
}

# One wheel notch batch over the list's center; 10 notches of 120 each per call.
function Wheel($h, $notches) {
    $r = New-Object N6+RECT
    [N6]::GetWindowRect($h, [ref]$r) | Out-Null
    [N6]::SetCursorPos($r.L + 500, $r.T + 400) | Out-Null
    $delta = if ($notches -ge 0) { 120 } else { -120 }
    $n = [Math]::Abs($notches)
    for ($i = 0; $i -lt $n; $i++) {
        [N6]::mouse_event(0x0800, 0, 0, [uint32]$delta, 0)
        Start-Sleep -Milliseconds 30
    }
}

function Keys([string]$keys, [double]$settle) {
    [System.Windows.Forms.SendKeys]::SendWait($keys)
    Start-Sleep -Seconds $settle
}

$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.UseShellExecute = $false
$psi.EnvironmentVariables["RUST_LOG"] = "info"
$psi.EnvironmentVariables["RIFF_N3_STORE"] = Join-Path $here "..\..\..\..\..\..\.scratch\nsh-n3\store\riff.sqlite3"
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$p = [Diagnostics.Process]::Start($psi)
$h = [IntPtr]::Zero
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($sw.Elapsed.TotalSeconds -lt 30) {
    $p.Refresh()
    if ($p.HasExited) { break }
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $h = $p.MainWindowHandle; break }
    Start-Sleep -Milliseconds 200
}
if ($h -eq [IntPtr]::Zero) { throw "no window after 30 s" }
Write-Host ("window after {0} ms pid {1}" -f $sw.ElapsedMilliseconds, $p.Id)

[N6]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 1500
# Debug builds open on the N3 readout; mod+alt+r mounts the shell.
[System.Windows.Forms.SendKeys]::SendWait("^%r")
Start-Sleep -Milliseconds 2500
[N6]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, 1200, 800, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2500

# --- The list at rest: rows and the first covers ----------------------------
Shot $h "$Tag-list"

# Idle CPU with the list live: sampling runs alone.
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds 20 -Out (Join-Path $logs "$Tag-idle.csv") | Out-Host

# --- Selection: a row click drives the inspector ----------------------------
Click $h 600 90
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-inspector"

# --- The deep jump: covers prefetch across albums, RSS with 500 held --------
Write-Host "jumping to row 40000 and scrolling across albums; cover decode is the limiter (P0 correction 24)"
[System.Windows.Forms.SendKeys]::SendWait("^%j")
Start-Sleep -Milliseconds 2500

# Wheel down across enough albums for the walk to hold ~500 covers
# (500 albums x 5 tracks ~ 2500 rows; 12 notches ~ 36 rows), shooting a
# step series as the cover-rendering series.
for ($step = 1; $step -le 30; $step++) {
    Wheel $h 12
    Start-Sleep -Milliseconds 700
    if ($step % 5 -eq 0) { Shot $h "$Tag-step-$step" }
}

# The covers held: RSS sampled while the window sits on the scrolled state.
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds 15 -Out (Join-Path $logs "$Tag-covers-rss.csv") | Out-Host
Shot $h "$Tag-covers"

# --- Scroll CPU: wheel steps driven under the sampler -----------------------
$job = Start-Job -ScriptBlock {
    param($tools, $procId, $out)
    & (Join-Path $tools "sample.ps1") -ProcId $procId -IntervalMs 250 -Seconds 20 -Out $out | Out-Null
} -ArgumentList $here, $p.Id, (Join-Path $logs "$Tag-scroll-cpu.csv")
Start-Sleep -Milliseconds 500
for ($i = 0; $i -lt 14; $i++) {
    Wheel $h 8
    Start-Sleep -Milliseconds 600
    Wheel $h -8
    Start-Sleep -Milliseconds 600
}
Wait-Job $job | Out-Null
Remove-Job $job
Shot $h "$Tag-scrolled"

Write-Host ("drive done pid {0} exit-wait" -f $p.Id)
if (-not $p.HasExited) { $p.Kill() }
$stdout = $p.StandardOutput.ReadToEnd()
$stderr = $p.StandardError.ReadToEnd()
$stdout | Set-Content (Join-Path $logs "$Tag.stdout.log")
$stderr | Set-Content (Join-Path $logs "$Tag.stderr.log")
Write-Host "drive complete"
