#Requires -Version 7.0
param(
    [Parameter(Mandatory)][string]$Artifacts10k,
    [Parameter(Mandatory)][string]$Artifacts100k,
    [Parameter(Mandatory)][string]$Plan10k,
    [Parameter(Mandatory)][string]$Plan100k,
    [Parameter(Mandatory)][string]$Output,
    [string]$Executable = 'target/release/laneflow-urban-harness.exe'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$binary = (Resolve-Path -LiteralPath $Executable).Path
$a10 = (Resolve-Path -LiteralPath $Artifacts10k).Path
$a100 = (Resolve-Path -LiteralPath $Artifacts100k).Path
$p10 = (Resolve-Path -LiteralPath $Plan10k).Path
$p100 = (Resolve-Path -LiteralPath $Plan100k).Path
$destination = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $destination) { throw "Output already exists: $destination" }
if (-not $env:LANEFLOW_HARDWARE_ROLE -or -not $env:LANEFLOW_POWER_ROLE) { throw 'Hardware and power roles are required' }
$head = (& git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve source' }
$status = & git status --porcelain
if ($LASTEXITCODE -ne 0 -or $status) { throw 'A clean frozen source checkout is required' }
$binaryHash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
$rows = @(
    @{ scale = '10k'; mode = 'headless'; artifacts = $a10; plan = $p10 },
    @{ scale = '10k'; mode = 'adapter'; artifacts = $a10; plan = $p10 },
    @{ scale = '100k'; mode = 'headless'; artifacts = $a100; plan = $p100 },
    @{ scale = '100k'; mode = 'adapter'; artifacts = $a100; plan = $p100 }
)
# Preparation/build are outside this clock. Directory creation, process launch,
# installation, sampling, process exit and all report writes are inside it.
$clock = [Diagnostics.Stopwatch]::StartNew()
$null = New-Item -ItemType Directory -Path $destination
$results = [Collections.Generic.List[object]]::new()
$failure = $null
$process = $null
try {
    for ($index = 0; $index -lt $rows.Count; $index++) {
        $row = $rows[$index]
        $remaining = 600000 - $clock.ElapsedMilliseconds - 5000
        $budget = [long][Math]::Floor($remaining / ($rows.Count - $index))
        if ($budget -le 10000) { throw 'No nonempty observation budget remains' }
        $name = "$($row.scale)-$($row.mode)"
        $rowDirectory = Join-Path $destination $name
        Write-Output "$name starting with $budget ms process budget"
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $binary
        $info.WorkingDirectory = (Get-Location).Path
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        foreach ($argument in @('evidence', $row.artifacts, $row.plan, $rowDirectory, $row.mode, '--wall-ms', "$budget")) {
            $info.ArgumentList.Add($argument)
        }
        $rowStarted = $clock.ElapsedMilliseconds
        $process = [Diagnostics.Process]::Start($info)
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $killed = $false
        while (-not $process.WaitForExit(100)) {
            if ($clock.ElapsedMilliseconds - $rowStarted -ge $budget -or $clock.ElapsedMilliseconds -ge 595000) {
                $process.Kill($true)
                $process.WaitForExit()
                $killed = $true
                break
            }
        }
        $exitCode = $process.ExitCode
        $processId = $process.Id
        $processMs = $clock.ElapsedMilliseconds - $rowStarted
        $stdout.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $destination "$name.stdout.log") -Encoding utf8
        $stderr.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $destination "$name.stderr.log") -Encoding utf8
        $process.Dispose()
        $process = $null
        $summaryPath = Join-Path $rowDirectory 'evidence.json'
        if ($killed -or $exitCode -ne 0 -or $processMs -gt $budget -or -not (Test-Path -LiteralPath $summaryPath)) {
            $results.Add(@{ scale=$row.scale; mode=$row.mode; status='failed'; pid=$processId; killed=$killed; exit_code=$exitCode; process_ms=$processMs; wall_budget_ms=$budget })
            throw "Failed bounded row: $name"
        }
        $summary = Get-Content -LiteralPath $summaryPath -Raw | ConvertFrom-Json
        if ($summary.status -ne 'bounded-observation-complete' -or $summary.completed_ticks -le 0 -or $summary.completed_ticks -gt 4096 -or
            $summary.committed_world_tick -ne $summary.completed_ticks -or $summary.mode -ne $row.mode -or $summary.scale -ne $row.scale -or
            $summary.source.commit -ne $head -or $summary.source.binary.sha256 -ne $binaryHash -or $summary.pid -ne $processId -or
            $summary.stop_reason -notin @('tick-limit','wall-limit') -or $summary.wall_limit_ms -ne $budget) {
            throw "Invalid bounded result: $name"
        }
        $results.Add(@{ scale=$row.scale; mode=$row.mode; status=$summary.status; pid=$processId; killed=$false; exit_code=$exitCode;
            process_ms=$processMs; wall_budget_ms=$budget; completed_ticks=$summary.completed_ticks; stop_reason=$summary.stop_reason;
            evidence_sha256=(Get-FileHash -LiteralPath $summaryPath -Algorithm SHA256).Hash.ToLowerInvariant() })
        Write-Output "$name completed $($summary.completed_ticks) ticks in $processMs ms ($($summary.stop_reason))"
    }
    if ((& git rev-parse HEAD).Trim() -ne $head -or $LASTEXITCODE -ne 0) { throw 'Source changed' }
    $endStatus = & git status --porcelain
    if ($LASTEXITCODE -ne 0 -or $endStatus) { throw 'Source is no longer clean' }
    if ($clock.ElapsedMilliseconds -ge 600000) { throw 'The complete batch exceeded 600 seconds' }
} catch {
    $failure = $_.Exception.Message
} finally {
    if ($null -ne $process) {
        if (-not $process.HasExited) { $process.Kill($true); $process.WaitForExit() }
        $process.Dispose()
    }
}
$report = @{
    version='urban-bounded-batch-v1'; status=$(if ($failure) { 'failed' } else { 'bounded-batch-complete' });
    source_commit=$head; binary_sha256=$binaryHash; rows=$results.ToArray(); error=$failure;
    limit_ms=600000; elapsed_ms_before_final_write=$clock.ElapsedMilliseconds;
    timing_basis='Serial four-process batch; startup, install, run, exit and log/report writes included; build and static preparation excluded';
    product_certification=$false
}
$report | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath (Join-Path $destination 'batch.json') -Encoding utf8
$elapsed = $clock.ElapsedMilliseconds
if ($elapsed -ge 600000 -and -not $failure) { $failure = 'The complete batch exceeded 600 seconds while writing its report' }
$report['elapsed_ms_including_first_final_write'] = $elapsed
$report['error'] = $failure
if ($failure) { $report['status'] = 'failed' }
$report | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath (Join-Path $destination 'batch.json') -Encoding utf8
if ($failure) { throw $failure }
Write-Output "Four-row bounded observation complete in $($clock.ElapsedMilliseconds) ms"
