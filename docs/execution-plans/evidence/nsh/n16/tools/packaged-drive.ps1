param()
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script:ShotsDir = Join-Path $here "..\shots"
$store = "C:\projects\riff\.scratch\nsh-n3\store\riff.sqlite3"
. (Join-Path $here "common.ps1")
$session = Start-Riff "C:\projects\riff\crates\riff-gui-nsh\.nsh\out\riff\riff.exe" $store
Start-Sleep -Milliseconds 2500
Shot $session.Handle "n16-packaged-shell"
Stop-Process -Id $session.Process.Id -Force
Write-Host "packaged launch ok"
