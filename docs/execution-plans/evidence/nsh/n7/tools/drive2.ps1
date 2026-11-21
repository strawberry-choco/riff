# N7 windowed drive, second pass: the scroll restore (through the debug
# jump key — the wheel delivery is dead in this session for the N6 and N7
# binaries alike, recorded in the trail), the search filter and restore
# with a no-match needle, and the folder tree's lazy children (the folder
# rows are shorter than the track rows, so the coordinates differ).
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "n7"
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
[System.Windows.Forms.SendKeys]::SendWait("^%r")
Start-Sleep -Milliseconds 2500
[N7]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, 1200, 800, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2500

$navAllTracks = @{ x = 140; y = 113 }
$navArtists   = @{ x = 140; y = 160 }
$navFolders   = @{ x = 140; y = 297 }

# --- Scroll restore: the debug jump lands row 40000, away, back ---------------
[System.Windows.Forms.SendKeys]::SendWait("^%j")
Start-Sleep -Seconds 3
Shot $h "$Tag-scrolled-deep"
Click $h $navArtists.x $navArtists.y
Start-Sleep -Milliseconds 1500
Shot $h "$Tag-scrolled-away"
Click $h $navAllTracks.x $navAllTracks.y
Start-Sleep -Seconds 2
Shot $h "$Tag-scroll-restored"

# --- Search: a no-match needle filters to the empty copy; Escape restores -----
Click $h 865 30
Start-Sleep -Milliseconds 400
Keys "zzzq" 2
Shot $h "$Tag-search-filtered"
Keys "{ESC}" 1
Shot $h "$Tag-search-restored"

# --- Folders: the root row, then a child, then a deeper folder ----------------
Click $h $navFolders.x $navFolders.y
Start-Sleep -Seconds 2
Shot $h "$Tag-folders"
Click $h 500 82
Start-Sleep -Milliseconds 1500
Shot $h "$Tag-folders-expanded"
Click $h 500 106
Start-Sleep -Milliseconds 1500
Shot $h "$Tag-folders-deep"

Stop-Process -Id $p.Id -Force
Write-Host "drive2 complete"
