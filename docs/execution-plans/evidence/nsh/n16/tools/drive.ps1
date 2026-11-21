# The N16 surfaces' evidence, driven by the debug navigation keys (each key
# performs the production control's own write). One app run per surface so a
# later surface never depends on a prior screen's focus.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "n16"
)
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script:ShotsDir = Join-Path $here "..\shots"
$logs = Join-Path $here "..\logs"
New-Item -ItemType Directory -Force -Path $script:ShotsDir, $logs | Out-Null
$store = Join-Path $here "..\..\..\..\..\..\.scratch\nsh-n3\store\riff.sqlite3"
. (Join-Path $here "common.ps1")

function Start-Shell {
    $session = Start-Riff $Exe $store
    [N7]::SetForegroundWindow($session.Handle) | Out-Null
    Start-Sleep -Milliseconds 1500
    [System.Windows.Forms.SendKeys]::SendWait("^%r")
    Start-Sleep -Milliseconds 2500
    [N7]::SetWindowPos($session.Handle, [IntPtr]::Zero, 0, 0, 1200, 800, 0x0040) | Out-Null
    Start-Sleep -Milliseconds 2000
    $session
}

# Smart list editor.
$a = Start-Shell
Keys "^+m" 2
Shot $a.Handle "$Tag-smart-editor"
Stop-Process -Id $a.Process.Id -Force

# Playlist create row.
$b = Start-Shell
Keys "^+p" 2
Shot $b.Handle "$Tag-playlist-create"
Stop-Process -Id $b.Process.Id -Force

# Settings pages.
$c = Start-Shell
Keys "^%s" 2
Keys "^+1" 1
Shot $c.Handle "$Tag-settings-library"
Keys "^+2" 1
Shot $c.Handle "$Tag-settings-playback"
Keys "^+3" 1
Shot $c.Handle "$Tag-settings-appearance"
Keys "^+4" 1
Shot $c.Handle "$Tag-settings-advanced"
Keys "^+5" 1
Shot $c.Handle "$Tag-settings-about"
Stop-Process -Id $c.Process.Id -Force

Write-Host "drive3 complete"
