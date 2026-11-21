# N15/N16 windowed drive: the shipped shell, its new surfaces, and the
# settings pages, over the synthetic fixture store (never the live store).
# Each step is the production control or a debug-only key that performs the
# control's own write; release builds carry none of the debug keys.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "n15"
)

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script:ShotsDir = Join-Path $here "..\shots"
$logs = Join-Path $here "..\logs"
New-Item -ItemType Directory -Force -Path $script:ShotsDir, $logs | Out-Null
$store = Join-Path $here "..\..\..\..\..\..\.scratch\nsh-n3\store\riff.sqlite3"
. (Join-Path $here "common.ps1")

$session = Start-Riff $Exe $store
$p = $session.Process
$h = $session.Handle
[N7]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 1500
# Debug builds open on the N3 readout; mod+alt+r mounts the shell.
[System.Windows.Forms.SendKeys]::SendWait("^%r")
Start-Sleep -Milliseconds 2500
[N7]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, 1200, 800, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2500

Shot $h "$Tag-shell"

# Play: the player bar's primary button, then the seeded queue's first row.
Click $h 600 768
Start-Sleep -Milliseconds 1500
Shot $h "$Tag-playing"

# Now Playing: the debug navigation performs the bar toggle's own write.
Keys "^%n" 2
Shot $h "$Tag-now-playing"
Keys "^%n" 2

# The Up Next panel.
Keys "^%q" 2
Shot $h "$Tag-queue"
Keys "^%q" 2

# Settings, page by page. The page list is the settings column's nav.
Keys "^%s" 2
Shot $h "$Tag-settings-library"
Click $h 300 128
Start-Sleep -Milliseconds 800
Shot $h "$Tag-settings-playback"
Click $h 300 157
Start-Sleep -Milliseconds 800
Shot $h "$Tag-settings-appearance"
Click $h 300 186
Start-Sleep -Milliseconds 800
Shot $h "$Tag-settings-advanced"
Click $h 300 215
Start-Sleep -Milliseconds 800
Shot $h "$Tag-settings-about"
Click $h 300 260
Start-Sleep -Milliseconds 800

# The smart list editor: the SMART LISTS section's plus.
Click $h 197 291
Start-Sleep -Milliseconds 1500
Shot $h "$Tag-smart-editor"

# The playlist create row: the PLAYLISTS section's plus.
Click $h 218 488
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-playlist-create"

# Idle CPU with the shell live: sampling runs alone.
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds 15 -Out (Join-Path $logs "$Tag-idle.csv") | Out-Host

Stop-Process -Id $p.Id -Force
Write-Host "drive complete"
