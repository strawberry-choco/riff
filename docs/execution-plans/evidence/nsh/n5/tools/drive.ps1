# N5 windowed drive: the sidebar live over the fixture store. Captures the
# counts shot, walks a selection click pair (the highlight moves and
# survives the pump's ticks), the Folders / smart-list selections, the
# keyboard traversal series (arrows through every section, Enter and Space
# activations, the wrap back into the chrome), the fold chevron, the footer
# at the restored 1000x700 frame (the N4 flag), and samples idle CPU with
# the sidebar live.
#
# Coordinates are measured off n5-calibrate.png at the 1200x800 frame this
# drive sets first: LIBRARY rows at y 114/160/206/252/298, the SMART LISTS
# chevron at (258, 342), smart rows at y 388..526, all x 150.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "n5"
)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class N5 {
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
    $r = New-Object N5+RECT
    [N5]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $ht
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $ok = [N5]::PrintWindow($h, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    if (-not $ok) { throw "PrintWindow failed" }
    $bmp.Save((Join-Path $shots "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "shot saved: $name.png (${w}x${ht})"
}

function Click($h, $x, $y) {
    $r = New-Object N5+RECT
    [N5]::GetWindowRect($h, [ref]$r) | Out-Null
    [N5]::SetCursorPos($r.L + $x, $r.T + $y) | Out-Null
    Start-Sleep -Milliseconds 200
    [N5]::mouse_event(0x0002, 0, 0, 0, 0)
    Start-Sleep -Milliseconds 60
    [N5]::mouse_event(0x0004, 0, 0, 0, 0)
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

[N5]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 1500
# Debug builds open on the N3 readout; mod+alt+r mounts the shell.
[System.Windows.Forms.SendKeys]::SendWait("^%r")
Start-Sleep -Milliseconds 2000
[N5]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, 1200, 800, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2000

# --- Counts on every row ----------------------------------------------------
Shot $h "$Tag-counts"

# Idle CPU with the sidebar live: sampling runs alone.
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds 20 -Out (Join-Path $logs "$Tag-idle.csv") | Out-Host

# --- Selection clicks: the highlight moves and survives the pump ------------
Click $h 150 160
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-selected-artists"

Click $h 150 252
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-selected-genres"

Click $h 150 298
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-selected-folders"

Click $h 150 526
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-selected-favorites"

# --- Keyboard traversal: arrows reach every section, Enter and Space --------
# Escape clears both focus segments; nine downs walk the chrome ring's
# seven slots into the sidebar's second row (Artists); Enter activates it.
Keys "{ESC}" 0.5
foreach ($i in 1..9) { Keys "{DOWN}" 0.25 }
Keys "{ENTER}" 1.2
Shot $h "$Tag-kb-artists"

# Seven more downs reach the last visible smart row (Favorites); Enter.
foreach ($i in 1..7) { Keys "{DOWN}" 0.25 }
Shot $h "$Tag-kb-favorites-focused"
Keys "{ENTER}" 1.2
Shot $h "$Tag-kb-favorites"

# One more down wraps out of the rows back into the chrome (search field).
Keys "{DOWN}" 0.7
Shot $h "$Tag-kb-wrapped"

# Space activates a focused row the same way: from the wrapped search slot,
# ten downs re-enter the rows and land on Genres (the chrome's seven slots
# first), then space.
foreach ($i in 1..10) { Keys "{DOWN}" 0.25 }
Keys " " 1.2
Shot $h "$Tag-kb-space-genres"

# --- The fold chevron --------------------------------------------------------
Click $h 258 342
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-collapsed"

# --- The footer at the restored 1000x700 frame (the N4 flag) -----------------
[N5]::SetWindowPos($h, [IntPtr]::Zero, 120, 90, 1000, 700, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2500
Shot $h "$Tag-footer-1000x700"

$alive = -not $p.HasExited
Write-Host "alive=$alive"
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 800
$p.StandardOutput.ReadToEnd() | Set-Content (Join-Path $logs "$Tag.stdout.log")
$p.StandardError.ReadToEnd() | Set-Content (Join-Path $logs "$Tag.stderr.log")
Write-Host "done"
