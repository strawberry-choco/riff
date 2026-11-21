# N7 windowed drive: the browse stages, the search flow and the folder tree
# over the fixture store (never the live store). Captures the drill-down
# walk (Artists -> albums -> tracks, Albums -> tracks, Genres -> artists ->
# albums -> tracks), the search filtered and restored, the scroll restore,
# and the Folders stage over the seeded fixture root.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "n7"
)

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script:ShotsDir = Join-Path $here "..\shots"
$logs = Join-Path $here "..\logs"
New-Item -ItemType Directory -Force -Path $script:ShotsDir, $logs | Out-Null

# The helpers (Shot/Click/Wheel/Keys/Start-Riff) live in common.ps1, which
# also owns the Add-Type block; dot-source it with the parameters it needs.
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

# Sidebar row coordinates (1200x800 frame): the LIBRARY rows sit under the
# title bar and the section header, one step apart.
$navAllTracks = @{ x = 140; y = 113 }
$navArtists   = @{ x = 140; y = 160 }
$navAlbums    = @{ x = 140; y = 206 }
$navGenres    = @{ x = 140; y = 251 }
$navFolders   = @{ x = 140; y = 297 }

# --- The flat stage at rest ---------------------------------------------------
Shot $h "$Tag-alltracks"

# --- Artists: root, then the first artist's albums, then its tracks -----------
Click $h $navArtists.x $navArtists.y
Start-Sleep -Seconds 4
Shot $h "$Tag-artists-root"

Click $h 500 110
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-artist-albums"

Click $h 500 110
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-album-tracks"

# --- Albums: root, then a walk into the tracks --------------------------------
Click $h $navAlbums.x $navAlbums.y
Start-Sleep -Seconds 2
Shot $h "$Tag-albums-root"

Click $h 500 110
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-albums-walk-tracks"

# --- Genres: root, artists, albums, tracks ------------------------------------
Click $h $navGenres.x $navGenres.y
Start-Sleep -Seconds 2
Shot $h "$Tag-genres-root"

Click $h 500 110
Start-Sleep -Milliseconds 1200
Click $h 500 110
Start-Sleep -Milliseconds 1200
Click $h 500 110
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-genres-walk-tracks"

# --- Scroll restore: deep on the flat list, away, back ------------------------
Click $h $navAllTracks.x $navAllTracks.y
Start-Sleep -Milliseconds 1500
Wheel $h 15
Start-Sleep -Milliseconds 1500
Shot $h "$Tag-scrolled"
Click $h $navArtists.x $navArtists.y
Start-Sleep -Milliseconds 1200
Click $h $navAllTracks.x $navAllTracks.y
Start-Sleep -Milliseconds 1500
Shot $h "$Tag-scroll-restored"

# --- Search: type into the titlebar field, then Escape ------------------------
Click $h 865 30
Start-Sleep -Milliseconds 400
Keys "amber" 2
Shot $h "$Tag-search-filtered"
Keys "{ESC}" 1
Shot $h "$Tag-search-restored"

# --- Folders: the seeded fixture root's tree ----------------------------------
Click $h $navFolders.x $navFolders.y
Start-Sleep -Seconds 2
Shot $h "$Tag-folders"
Click $h 500 110
Start-Sleep -Milliseconds 1200
Shot $h "$Tag-folders-expanded"

# --- Idle CPU with the stages live: sampling runs alone -----------------------
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds 15 -Out (Join-Path $logs "$Tag-idle.csv") | Out-Host

Stop-Process -Id $p.Id -Force
Write-Host "drive complete"
