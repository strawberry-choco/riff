# N4 windowed drive: start the riff-nsh binary over the synthetic store,
# toggle the debug readout off so the shell shows, capture the stopped and
# playing shells, drive the player bar's own controls by coordinates (play,
# seek, pause, resume, next, volume, mute, shuffle, repeat), verify the
# system's caption buttons (double-click maximize, minimize), sample idle
# and playing CPU, and prove the geometry loop live: move and resize the
# window, close, relaunch, and capture the restored frame.
#
# The window is reset to 1200x800 at the origin first, so the coordinates
# below (measured off the shots: transport at y 762, seek track x 60..1140
# at y 726, volume track x 980..1176, mute at 950, toggles at 774 and 822,
# and the system caption buttons at y 35 with minimize at 1076) land where
# they say.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "n4"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class N4 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int ht, uint f);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
Add-Type -AssemblyName System.Windows.Forms

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$shots = Join-Path $here "..\shots"
$logs = Join-Path $here "..\logs"
New-Item -ItemType Directory -Force -Path $shots, $logs | Out-Null

function Shot($h, $name) {
    $r = New-Object N4+RECT
    [N4]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $ht
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $ok = [N4]::PrintWindow($h, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    if (-not $ok) { throw "PrintWindow failed" }
    $bmp.Save((Join-Path $shots "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "shot saved: $name.png (${w}x${ht})"
}

function Click($h, $x, $y) {
    $r = New-Object N4+RECT
    [N4]::GetWindowRect($h, [ref]$r) | Out-Null
    [N4]::SetCursorPos($r.L + $x, $r.T + $y) | Out-Null
    Start-Sleep -Milliseconds 200
    [N4]::mouse_event(0x0002, 0, 0, 0, 0)
    Start-Sleep -Milliseconds 60
    [N4]::mouse_event(0x0004, 0, 0, 0, 0)
}

function StartApp {
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables["RUST_LOG"] = "info"
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $p = [Diagnostics.Process]::Start($psi)
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt 30) {
        if ($p.HasExited) { break }
        $p.Refresh()
        if ($p.MainWindowHandle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 200
    }
    if ($p.HasExited) {
        Write-Host "EXITED early code=$($p.ExitCode)"
        $p.StandardError.ReadToEnd() | Set-Content (Join-Path $logs "$Tag.stderr.log")
        exit 1
    }
    Write-Host ("window after {0} ms pid {1}" -f $sw.ElapsedMilliseconds, $p.Id)
    return $p
}

# --- First launch: fresh window -------------------------------------------
$p = StartApp
$h = $p.MainWindowHandle
[N4]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 1500

# Debug builds open on the N3 readout; mod+alt+r toggles to the shell.
[System.Windows.Forms.SendKeys]::SendWait("^%r")
Start-Sleep -Milliseconds 1200

# Reset the geometry to a known frame, so the drive's coordinates are
# stable and the persistence proof has a delta to restore.
[N4]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, 1200, 800, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2000

Shot $h "$Tag-shell"

# Idle CPU: the shell open, playback stopped, the store loaded.
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds 20 -Out (Join-Path $logs "$Tag-idle.csv") | Out-Host

# --- Player bar drives ------------------------------------------------------
# Play: the middle, brand-filled button of the transport cluster. The
# engine fills the queue from the 50k store behind the pick.
Click $h 600 762
Start-Sleep -Milliseconds 4000
Shot $h "$Tag-playing"

# Playing CPU: decode + audio callback + pump. The fixture's tracks are
# three seconds long, so the session advances every ~3 s and the sampler
# catches decode bursts between idle stretches.
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds 20 -Out (Join-Path $logs "$Tag-playing.csv") | Out-Host

# Seek: a press at the slider's midpoint seeks to ~50% of the track.
Click $h 600 726
Start-Sleep -Milliseconds 1000

# Pause mid-track, then resume: the primary button flips with the state.
Click $h 600 762
Start-Sleep -Milliseconds 1000
Shot $h "$Tag-paused"
Click $h 600 762
Start-Sleep -Milliseconds 1000

# Next: the right ghost of the cluster.
Click $h 648 762
Start-Sleep -Milliseconds 1200

# Volume: a press at the slider's midpoint sets ~0.5.
Click $h 1078 762
Start-Sleep -Milliseconds 1000

# Mute: the speaker ghost left of the volume slider.
Click $h 950 762
Start-Sleep -Milliseconds 1000
Shot $h "$Tag-muted"

# Shuffle and repeat: the toggles at x 774 and 822.
Click $h 774 762
Start-Sleep -Milliseconds 800
Shot $h "$Tag-shuffled"
Click $h 822 762
Start-Sleep -Milliseconds 800
Shot $h "$Tag-toggled"

# --- The system's caption buttons ------------------------------------------
# The app draws no duplicate controls (the P0 question's answer), so the
# platform's own buttons carry these: the system maximize maximizes and
# the system minimize minimizes. (A double-click on the drag region also
# maximizes — proven in an earlier drive pass — but the button press is
# the deterministic one to capture.)
Click $h 1122 35
Start-Sleep -Milliseconds 1500
Shot $h "$Tag-maximized"

$r = New-Object N4+RECT
[N4]::GetWindowRect($h, [ref]$r) | Out-Null
$sysMin = ($r.R - $r.L) - 124
Click $h $sysMin 35
Start-Sleep -Milliseconds 1500
$minimized = [N4]::IsIconic($h)
Write-Host "minimized=$minimized"
[N4]::ShowWindow($h, 9) | Out-Null
Start-Sleep -Milliseconds 1000

# --- Geometry persistence: move, resize, close, relaunch --------------------
# SW_NORMAL first: a window restored from minimize keeps its maximized
# flag through SetWindowPos, and the row's mode must say Normal for the
# restore proof to show the frame itself.
[N4]::ShowWindow($h, 1) | Out-Null
Start-Sleep -Milliseconds 800
[N4]::SetWindowPos($h, [IntPtr]::Zero, 120, 90, 1000, 700, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2500

$p.Kill()
$p.WaitForExit()
Start-Sleep -Milliseconds 1500

$p2 = StartApp
$h2 = $p2.MainWindowHandle
[N4]::SetForegroundWindow($h2) | Out-Null
Start-Sleep -Milliseconds 1500

# Launch 2 also opens on the debug readout; the shell mounts only when the
# readout hides, and the restore runs at the shell's mount.
[System.Windows.Forms.SendKeys]::SendWait("^%r")
Start-Sleep -Milliseconds 2500
Shot $h2 "$Tag-restored"

$r = New-Object N4+RECT
[N4]::GetWindowRect($h2, [ref]$r) | Out-Null
Write-Host ("restored frame: L={0} T={1} W={2} H={3}" -f $r.L, $r.T, ($r.R - $r.L), ($r.B - $r.T))

# Titlebar corner, magnified: the caption-button duplication answer's
# visual — the system's buttons alone, no duplicates beside them.
$source = [System.Drawing.Image]::FromFile((Join-Path $shots "$Tag-restored.png"))
$corner = New-Object System.Drawing.Bitmap 400, 120
$g = [System.Drawing.Graphics]::FromImage($corner)
$g.DrawImage($source, `
    (New-Object System.Drawing.Rectangle 0, 0, 400, 120), `
    (New-Object System.Drawing.Rectangle ($source.Width - 400), 0, 400, 120), `
    [System.Drawing.GraphicsUnit]::Pixel)
$corner.Save((Join-Path $shots "$Tag-titlebar-corner.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$corner.Dispose(); $source.Dispose()
Write-Host "shot saved: $Tag-titlebar-corner.png"

$alive = -not $p2.HasExited
Write-Host "alive=$alive"
Stop-Process -Id $p2.Id -Force -ErrorAction SilentlyContinue
$p2.StandardOutput.ReadToEnd() | Set-Content (Join-Path $logs "$Tag.stdout.log")
$p2.StandardError.ReadToEnd() | Set-Content (Join-Path $logs "$Tag.stderr.log")
