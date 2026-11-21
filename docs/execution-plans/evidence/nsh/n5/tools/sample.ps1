param(
    [Parameter(Mandatory = $true)][int]$ProcId,
    [int]$IntervalMs = 250,
    [int]$Seconds = 20,
    [Parameter(Mandatory = $true)][string]$Out
)

$ErrorActionPreference = 'Stop'
$cores = [Environment]::ProcessorCount
$sw = [Diagnostics.Stopwatch]::StartNew()
$prevCpu = $null
$prevMs = 0
$rows = New-Object System.Collections.Generic.List[string]
$rows.Add("t_ms,cpu_pct_one_core,cpu_pct_machine,working_set_mb,private_mb,threads,handles")

while ($sw.Elapsed.TotalSeconds -lt $Seconds) {
    $p = Get-Process -Id $ProcId -ErrorAction SilentlyContinue
    if (-not $p) { break }
    $ms = $sw.Elapsed.TotalMilliseconds
    $cpu = $p.TotalProcessorTime.TotalMilliseconds
    $cpuCore = ''
    $cpuMachine = ''
    if ($null -ne $prevCpu -and $ms -gt $prevMs) {
        $wall = $ms - $prevMs
        $pct = (($cpu - $prevCpu) / $wall) * 100.0
        $cpuCore = [math]::Round($pct, 2)
        $cpuMachine = [math]::Round($pct / $cores, 2)
    }
    $prevCpu = $cpu
    $prevMs = $ms
    $rows.Add(("{0},{1},{2},{3},{4},{5},{6}" -f `
                [math]::Round($ms, 1), $cpuCore, $cpuMachine, `
                [math]::Round($p.WorkingSet64 / 1MB, 2), `
                [math]::Round($p.PrivateMemorySize64 / 1MB, 2), `
                $p.Threads.Count, $p.HandleCount))
    Start-Sleep -Milliseconds $IntervalMs
}

Set-Content -Path $Out -Value $rows -Encoding UTF8
Write-Host ("samples={0} cores={1} out={2}" -f ($rows.Count - 1), $cores, $Out)