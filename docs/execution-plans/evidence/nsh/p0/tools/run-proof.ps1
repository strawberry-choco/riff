param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Arguments = "",
    [Parameter(Mandatory = $true)][string]$Tag,
    [int]$IdleSeconds = 12,
    [string]$WorkDir = "",
    [switch]$KeepOpen
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$shots = Join-Path $here "..\shots"
$logs = Join-Path $here "..\logs"
New-Item -ItemType Directory -Force -Path $shots, $logs | Out-Null

$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.Arguments = $Arguments
$psi.UseShellExecute = $false
if ($WorkDir) { $psi.WorkingDirectory = $WorkDir }
$psi.EnvironmentVariables["RUST_LOG"] = "info"
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true

$sw = [Diagnostics.Stopwatch]::StartNew()
$p = [Diagnostics.Process]::Start($psi)
$stdout = Join-Path $logs "$Tag.stdout.log"
$stderr = Join-Path $logs "$Tag.stderr.log"

$windowMs = $null
while ($sw.Elapsed.TotalSeconds -lt 30) {
    if ($p.HasExited) { break }
    $p.Refresh()
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $windowMs = $sw.Elapsed.TotalMilliseconds; break }
    Start-Sleep -Milliseconds 100
}

if ($p.HasExited) {
    $p.StandardOutput.ReadToEnd() | Set-Content $stdout
    $p.StandardError.ReadToEnd() | Set-Content $stderr
    Write-Host "EXITED early code=$($p.ExitCode) after $([math]::Round($sw.Elapsed.TotalMilliseconds))ms"
    Get-Content $stderr -Tail 25
    exit 1
}

$shot = Join-Path $shots "$Tag.png"
$shotOk = "skipped"
if ($windowMs) {
    try {
        & (Join-Path $here "shot.ps1") -ProcId $p.Id -Out $shot -WaitSeconds 5 | Out-Host
        $shotOk = $shot
    } catch { $shotOk = "FAILED: $($_.Exception.Message)" }
}

$csv = Join-Path $logs "$Tag.idle.csv"
& (Join-Path $here "sample.ps1") -ProcId $p.Id -IntervalMs 250 -Seconds $IdleSeconds -Out $csv | Out-Host

$p.Refresh()
$peakWs = [math]::Round($p.WorkingSet64 / 1MB, 2)
$threads = $p.Threads.Count
$handles = $p.HandleCount

if (-not $KeepOpen) {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
}
$p.StandardOutput.ReadToEnd() | Set-Content $stdout
$p.StandardError.ReadToEnd() | Set-Content $stderr

$rows = Import-Csv $csv | Where-Object { $_.cpu_pct_one_core -ne '' }
$cpu = $rows | ForEach-Object { [double]$_.cpu_pct_one_core } | Sort-Object
$ws = $rows | ForEach-Object { [double]$_.working_set_mb } | Sort-Object
function Med($a) { if ($a.Count -eq 0) { return 'n/a' }; $i = [int][math]::Floor($a.Count / 2); return $a[$i] }

Write-Host ("TAG={0} window_ms={1} idle_cpu_median_pct_of_one_core={2} idle_cpu_range={3}..{4} ws_median_mb={5} ws_peak_mb={6} threads={7} handles={8} samples={9} shot={10}" -f `
    $Tag, $(if ($windowMs) { [math]::Round($windowMs) } else { 'none' }), (Med $cpu), $(if ($cpu.Count) { $cpu[0] } else { 'n/a' }), $(if ($cpu.Count) { $cpu[-1] } else { 'n/a' }), (Med $ws), $peakWs, $threads, $handles, $rows.Count, $shotOk)