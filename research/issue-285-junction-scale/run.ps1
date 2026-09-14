param(
    [Parameter(Mandatory)][string]$EvidenceDirectory,
    [ValidateRange(1, 600000)][int]$MaxWallMilliseconds = 600000
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'process-memory.ps1')
. (Join-Path $PSScriptRoot 'environment.ps1')
$freezePath = Join-Path $EvidenceDirectory 'freeze.json'
$freeze = Get-Content -LiteralPath $freezePath -Raw | ConvertFrom-Json -AsHashtable
if ($freeze.schema -ne 'junction-scale-freeze-v2' -or $freeze.protocol.rounds -ne 1 -or $freeze.protocol.maxTicks -ne 4096 -or $freeze.protocol.warmupTicks -ne 0 -or $freeze.protocol.observationTicks -ne 4096 -or $freeze.protocol.batchWallLimitMilliseconds -ne 600000) { throw 'Expected the bounded #285 protocol' }
if ($freeze.inputs.Count -ne 2 -or (@($freeze.inputs.vehicles | Sort-Object) -join ',') -ne '10000,100000') { throw 'Expected exactly the one-world 10000/100000 cases' }
$batchPath = Join-Path $EvidenceDirectory 'batch.json'
if (Test-Path -LiteralPath $batchPath) { throw 'Batch already recorded; use a new result directory' }
$freezeHash = (Get-FileHash -LiteralPath $freezePath -Algorithm SHA256).Hash.ToLowerInvariant()
$runEnvironment = Get-JunctionEnvironment
foreach ($binary in $freeze.executables.Values) {
    if ((Get-FileHash -LiteralPath $binary.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $binary.sha256) { throw 'Frozen binary changed' }
}
foreach ($inputCase in $freeze.inputs) {
    if (-not $inputCase.files['prepared.initial.lfrs'] -or $inputCase.files['prepared.initial.lfrs'] -ne $inputCase.prepared.snapshot_sha256 -or -not $inputCase.files['prepared.json']) { throw 'Prepared snapshot or preparation record is not authenticated' }
    foreach ($file in $inputCase.files.Keys) {
        if ((Get-FileHash -LiteralPath (Join-Path $inputCase.directory $file) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $inputCase.files[$file]) { throw 'Frozen input changed' }
    }
}

function Invoke-EvidenceProcess([string]$Name, [string]$Executable, [string[]]$Arguments) {
    $powerBefore = Get-JunctionPowerState
    $frozenPowerKey = (Get-JunctionPowerKey $freeze.environment) -join "`n"
    $expectedBinary = @($freeze.executables.Values | Where-Object { $_.path -eq $Executable })
    if ($expectedBinary.Count -ne 1 -or (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedBinary[0].sha256) { throw 'Frozen binary changed before process launch' }
    $metadataPath = Join-Path $EvidenceDirectory "$Name.process.json"
    foreach ($path in @($metadataPath, (Join-Path $EvidenceDirectory "$Name.stdout.log"), (Join-Path $EvidenceDirectory "$Name.stderr.log"), (Join-Path $EvidenceDirectory "$Name.json"), (Join-Path $EvidenceDirectory "$Name.png"))) {
        if (Test-Path -LiteralPath $path) { throw "Run artifact already exists: $path" }
    }
    # 每档最多使用剩余批次时间的均分份额，保证后一档也有运行机会。
    $wallLimit = [int][Math]::Floor(($MaxWallMilliseconds - $batchClock.Elapsed.TotalMilliseconds) / $remainingCases)
    if ($wallLimit -lt 1) { throw 'Batch wall-clock budget exhausted before the next case' }
    $argumentsWithLimit = @($Arguments) + @([string]$wallLimit)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $argumentsWithLimit) { $start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $stdoutFile = $null
    $stderrFile = $null
    $launched = $false
    try {
        $started = [DateTime]::UtcNow
        $processClock = [Diagnostics.Stopwatch]::StartNew()
        $launched = $process.Start()
        if (-not $launched) { throw "Unable to start $Name" }
        $processHandle = $process.SafeHandle
        $stdoutFile = [IO.File]::Open((Join-Path $EvidenceDirectory "$Name.stdout.log"), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
        $stderrFile = [IO.File]::Open((Join-Path $EvidenceDirectory "$Name.stderr.log"), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
        $stdout = $process.StandardOutput.BaseStream.CopyToAsync($stdoutFile)
        $stderr = $process.StandardError.BaseStream.CopyToAsync($stderrFile)
        $peakWorkingSet = 0L
        $peakPrivateSampled = 0L
        $peakPaged = 0L
        $timedOut = $false
        $lastStatus = [DateTime]::MinValue
        while (-not $process.HasExited) {
            $remaining = [Math]::Min($wallLimit - $processClock.Elapsed.TotalMilliseconds, $MaxWallMilliseconds - $batchClock.Elapsed.TotalMilliseconds)
            if ($remaining -le 0) {
                $timedOut = $true
                $process.Kill()
                $process.WaitForExit()
                break
            }
            if ($process.WaitForExit([int][Math]::Max(1, [Math]::Min(500, $remaining)))) { break }
            $memory = [JunctionProcessMemory]::Read($processHandle)
            $peakWorkingSet = [Math]::Max($peakWorkingSet, [long]$memory.PeakWorkingSetSize.ToUInt64())
            $peakPrivateSampled = [Math]::Max($peakPrivateSampled, [long]$memory.PrivateUsage.ToUInt64())
            $peakPaged = [Math]::Max($peakPaged, [long]$memory.PeakPagefileUsage.ToUInt64())
            if (([DateTime]::UtcNow - $lastStatus).TotalSeconds -ge 10) {
                @{ status = 'running'; run = $Name; pid = $process.Id; startedUtc = $started.ToString('o'); elapsedSeconds = $processClock.Elapsed.TotalSeconds; wallLimitMilliseconds = $wallLimit; batchElapsedSeconds = $batchClock.Elapsed.TotalSeconds; batchWallLimitMilliseconds = $MaxWallMilliseconds } |
                    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'running.json') -Encoding utf8NoBOM
                $lastStatus = [DateTime]::UtcNow
            }
        }
        $processElapsed = $processClock.Elapsed.TotalSeconds
        $memory = [JunctionProcessMemory]::Read($processHandle)
        $peakWorkingSet = [Math]::Max($peakWorkingSet, [long]$memory.PeakWorkingSetSize.ToUInt64())
        $peakPrivateSampled = [Math]::Max($peakPrivateSampled, [long]$memory.PrivateUsage.ToUInt64())
        $peakPaged = [Math]::Max($peakPaged, [long]$memory.PeakPagefileUsage.ToUInt64())
        [void]$stdout.GetAwaiter().GetResult()
        [void]$stderr.GetAwaiter().GetResult()
        $stdoutFile.Flush()
        $stderrFile.Flush()
        $powerAfter = Get-JunctionPowerState
        $metadata = @{ schema = 'junction-scale-process-v2'; name = $Name; pid = $process.Id; sourceCommit = $freeze.sourceCommit; freezeSha256 = $freezeHash
            stableEnvironmentSha256 = $runEnvironment.stableEnvironmentSha256; powerBefore = $powerBefore; powerAfter = $powerAfter; powerMatchesFreeze = (((Get-JunctionPowerKey $powerAfter) -join "`n") -eq $frozenPowerKey)
            executable = $Executable; binarySha256 = $expectedBinary[0].sha256; arguments = $argumentsWithLimit; environmentOverrides = @{}
            startedUtc = $started.ToString('o'); finishedUtc = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $processElapsed; exitCode = $process.ExitCode; wallLimitMilliseconds = $wallLimit; timedOut = $timedOut
            peakWorkingSetBytes = $peakWorkingSet; privateBytesSampledPeak = $peakPrivateSampled; processCommitPeakBytes = $peakPaged
            memorySampling = '500ms private bytes samples and retained-handle OS lifetime peaks through process exit; not a component ledger'
        }
        $metadata | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $metadataPath -Encoding utf8NoBOM
        if ($timedOut -or $process.ExitCode -ne 0) { throw "Execution failed or hard deadline reached; artifacts preserved: $Name" }
        Write-Output "$Name exit=0 elapsed=$([Math]::Round($processElapsed,1))s"
    } finally {
        if ($launched -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
        if ($stdoutFile) { $stdoutFile.Dispose() }
        if ($stderrFile) { $stderrFile.Dispose() }
        $process.Dispose()
    }
}

$batchStarted = [DateTime]::UtcNow
$batchClock = [Diagnostics.Stopwatch]::StartNew()
$remainingCases = $freeze.inputs.Count
$completedCases = @()
$batchError = $null
try {
    foreach ($inputCase in $freeze.inputs) {
        $name = "render-$($inputCase.vehicles)-round-1"
        $arguments = @((Join-Path $inputCase.directory 'network.lfca'), (Join-Path $inputCase.directory 'grid.catalog.toml'), [string]$inputCase.vehicles, '0', '4096', 'catchup', (Join-Path $EvidenceDirectory "$name.json"))
        Invoke-EvidenceProcess $name $freeze.executables['junction_scale_render']['path'] $arguments
        $completedCases += $inputCase.vehicles
        $remainingCases--
    }
    if ($batchClock.Elapsed.TotalMilliseconds -gt $MaxWallMilliseconds) { throw 'Batch exceeded its wall-clock budget' }
} catch {
    $batchError = $_.Exception.Message
    throw
} finally {
    $batch = @{ schema = 'junction-scale-batch-v1'; sourceCommit = $freeze.sourceCommit; freezeSha256 = $freezeHash; environment = $runEnvironment; startedUtc = $batchStarted.ToString('o'); finishedUtc = [DateTime]::UtcNow.ToString('o'); elapsedMilliseconds = $batchClock.Elapsed.TotalMilliseconds; wallLimitMilliseconds = $MaxWallMilliseconds; completedScales = $completedCases; error = $batchError; status = $(if ($batchError) { 'failed' } else { 'completed' }) }
    $batch | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $batchPath -Encoding utf8NoBOM
    $batch | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'running.json') -Encoding utf8NoBOM
}
