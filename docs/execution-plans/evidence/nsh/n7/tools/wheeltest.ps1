# Wheel probe: three wheel batches with a shot after each — does the flat
# list move at all under the windowed wheel?
param(
    [Parameter(Mandatory = $true)][string]$Exe
)

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script:ShotsDir = Join-Path $here "..\shots"
New-Item -ItemType Directory -Force -Path $script:ShotsDir | Out-Null

$store = Join-Path $here "..\..\..\..\..\..\.scratch\nsh-n3\store\riff.sqlite3"
. (Join-Path $here "common.ps1")
$session = Start-Riff $Exe $store
$h = $session.Handle

[N7]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 1500
[System.Windows.Forms.SendKeys]::SendWait("^%r")
Start-Sleep -Milliseconds 2500
[N7]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, 1200, 800, 0x0040) | Out-Null
Start-Sleep -Milliseconds 2500

Shot $h "n7-wheelsi-0"
Wheel $h 10
Start-Sleep -Milliseconds 1200
Shot $h "n7-wheelsi-1"
Wheel $h 10
Start-Sleep -Milliseconds 1200
Shot $h "n7-wheelsi-2"
Stop-Process -Id $session.Process.Id -Force
Write-Host "wheel probe complete"
