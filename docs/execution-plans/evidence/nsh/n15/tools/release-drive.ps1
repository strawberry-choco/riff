param()
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script:ShotsDir = Join-Path $here "..\shots"
$logs = Join-Path $here "..\logs"
$store = Join-Path $here "..\..\..\..\..\..\.scratch\nsh-n3\store\riff.sqlite3"
. (Join-Path $here "common.ps1")
$session = Start-Riff "C:\projects\riff\target\release\riff.exe" $store
$p = $session.Process
$h = $session.Handle
[N7]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 2000
[N7]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, 1200, 800, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2000
Shot $h "n15-release-shell"
Click $h 600 768
Start-Sleep -Milliseconds 1800
Shot $h "n15-release-playing"
Stop-Process -Id $p.Id -Force
Write-Host "release drive complete"
