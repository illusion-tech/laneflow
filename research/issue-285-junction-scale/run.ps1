param([Parameter(Mandatory)][string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'process-memory.ps1')
. (Join-Path $PSScriptRoot 'environment.ps1')
$freezePath = Join-Path $EvidenceDirectory 'freeze.json'
$freeze = Get-Content -LiteralPath $freezePath -Raw | ConvertFrom-Json -AsHashtable
$freezeHash = (Get-FileHash -LiteralPath $freezePath -Algorithm SHA256).Hash.ToLowerInvariant()
$runEnvironment = Get-JunctionEnvironment
if (-not $freeze.environment.stableEnvironmentSha256 -or $runEnvironment.stableEnvironmentSha256 -ne $freeze.environment.stableEnvironmentSha256) { throw 'Machine, OS, firmware or drivers differ from frozen environment; refreeze before timing' }
foreach ($binary in $freeze.executables.Values) {
    if ((Get-FileHash -LiteralPath $binary.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $binary.sha256) { throw 'Frozen binary changed' }
}
foreach ($inputCase in $freeze.inputs) {
    foreach ($file in $inputCase.files.Keys) {
        if ((Get-FileHash -LiteralPath (Join-Path $inputCase.directory $file) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $inputCase.files[$file]) { throw 'Frozen input changed' }
    }
}

function Invoke-EvidenceProcess([string]$Name, [string]$Executable, [string[]]$Arguments, [hashtable]$Environment = @{}) {
    $powerBefore = Get-JunctionPowerState
    $frozenPowerKey = (Get-JunctionPowerKey $freeze.environment) -join "`n"
    if (((Get-JunctionPowerKey $powerBefore) -join "`n") -ne $frozenPowerKey) { throw 'Power plan or AC state differs from freeze before process launch' }
    $expectedBinary = @($freeze.executables.Values | Where-Object { $_.path -eq $Executable })
    if ($expectedBinary.Count -ne 1 -or (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedBinary[0].sha256) { throw 'Frozen binary changed before process launch' }
    $metadataPath = Join-Path $EvidenceDirectory "$Name.process.json"
    if (Test-Path -LiteralPath $metadataPath) { throw "Run already recorded: $Name" }
    foreach ($suffix in @('stdout.log', 'stderr.log', 'json', 'csv', 'png', 'warm.lfrs')) {
        if (Test-Path -LiteralPath (Join-Path $EvidenceDirectory "$Name.$suffix")) { throw "Run artifact already exists: $Name.$suffix" }
    }
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    foreach ($key in $Environment.Keys) { $start.Environment[$key] = [string]$Environment[$key] }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $started = [DateTime]::UtcNow
    $background = @(Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 30 ProcessName,Id,CPU,WorkingSet64)
    if (-not $process.Start()) { throw "Unable to start $Name" }
    $processHandle = $process.SafeHandle
    $stdoutFile = [IO.File]::Open((Join-Path $EvidenceDirectory "$Name.stdout.log"), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
    $stderrFile = [IO.File]::Open((Join-Path $EvidenceDirectory "$Name.stderr.log"), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
    $stdout = $process.StandardOutput.BaseStream.CopyToAsync($stdoutFile)
    $stderr = $process.StandardError.BaseStream.CopyToAsync($stderrFile)
    $peakWorkingSet = 0L
    $peakPrivateSampled = 0L
    $peakPaged = 0L
    $lastStatus = [DateTime]::MinValue
    while (-not $process.WaitForExit(500)) {
        $memory = [JunctionProcessMemory]::Read($processHandle)
        $peakWorkingSet = [Math]::Max($peakWorkingSet, [long]$memory.PeakWorkingSetSize.ToUInt64())
        $peakPrivateSampled = [Math]::Max($peakPrivateSampled, [long]$memory.PrivateUsage.ToUInt64())
        $peakPaged = [Math]::Max($peakPaged, [long]$memory.PeakPagefileUsage.ToUInt64())
        if (([DateTime]::UtcNow - $lastStatus).TotalSeconds -ge 10) {
            @{ run = $Name; pid = $process.Id; startedUtc = $started.ToString('o'); elapsedSeconds = ([DateTime]::UtcNow - $started).TotalSeconds; peakWorkingSetBytes = $peakWorkingSet; privateBytesSampledPeak = $peakPrivateSampled } |
                ConvertTo-Json | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'running.json') -Encoding utf8NoBOM
            $lastStatus = [DateTime]::UtcNow
        }
    }
    # Includes short processes and allocations during final result serialization.
    $memory = [JunctionProcessMemory]::Read($processHandle)
    $peakWorkingSet = [Math]::Max($peakWorkingSet, [long]$memory.PeakWorkingSetSize.ToUInt64())
    $peakPrivateSampled = [Math]::Max($peakPrivateSampled, [long]$memory.PrivateUsage.ToUInt64())
    $peakPaged = [Math]::Max($peakPaged, [long]$memory.PeakPagefileUsage.ToUInt64())
    [void]$stdout.GetAwaiter().GetResult()
    [void]$stderr.GetAwaiter().GetResult()
    $stdoutFile.Dispose()
    $stderrFile.Dispose()
    $powerAfter = Get-JunctionPowerState
    $powerMatches = ((Get-JunctionPowerKey $powerAfter) -join "`n") -eq $frozenPowerKey
    $metadata = @{ schema = 'junction-scale-process-v1'; name = $Name; pid = $process.Id; sourceCommit = $freeze.sourceCommit; freezeSha256 = $freezeHash
        stableEnvironmentSha256 = $runEnvironment.stableEnvironmentSha256; powerBefore = $powerBefore; powerAfter = $powerAfter; powerMatchesFreeze = $powerMatches
        executable = $Executable; binarySha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant(); arguments = $Arguments; environmentOverrides = $Environment
        startedUtc = $started.ToString('o'); finishedUtc = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = ([DateTime]::UtcNow - $started).TotalSeconds; exitCode = $process.ExitCode
        peakWorkingSetBytes = $peakWorkingSet; privateBytesSampledPeak = $peakPrivateSampled; processCommitPeakBytes = $peakPaged
        memorySampling = '500ms private bytes; final retained-handle read of OS lifetime peak working set and peak pagefile-backed commit after exit; component ledger is separate'
        backgroundProcessesBefore = $background
    }
    $metadata | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $metadataPath -Encoding utf8NoBOM
    Write-Output "$Name exit=$($process.ExitCode) elapsed=$([Math]::Round($metadata.elapsedSeconds,1))s"
    $exitCode = $process.ExitCode
    $process.Dispose()
    if ($exitCode -ne 0) { throw "Execution failed; artifacts preserved: $Name" }
    if (-not $powerMatches) { throw "Power state changed; artifacts preserved but round invalid: $Name" }
}

for ($round = 1; $round -le 3; $round++) {
    # 跨轮轮换两档顺序；不同测量进程不重叠。
    $cases = if ($round % 2) { @($freeze.inputs) } else { @($freeze.inputs[1], $freeze.inputs[0]) }
    foreach ($inputCase in $cases) {
        $name = "render-$($inputCase.vehicles)-round-$round"
        $arguments = @((Join-Path $inputCase.directory 'network.lfca'), (Join-Path $inputCase.directory 'grid.catalog.toml'), [string]$inputCase.vehicles, '18012', '36024', 'catchup', (Join-Path $EvidenceDirectory "$name.json"))
        Invoke-EvidenceProcess $name $freeze.executables['junction_scale_render']['path'] $arguments
    }
}
foreach ($inputCase in $freeze.inputs) {
    $name = "allocation-$($inputCase.vehicles)"
    $arguments = @((Join-Path $inputCase.directory 'network.lfca'), (Join-Path $inputCase.directory 'grid.catalog.toml'), [string]$inputCase.vehicles, '18012', '36024', 'catchup', (Join-Path $EvidenceDirectory "$name.json"))
    Invoke-EvidenceProcess $name $freeze.executables['junction_scale_allocation']['path'] $arguments
}
foreach ($inputCase in $freeze.inputs) {
    $name = "ledger-$($inputCase.vehicles)"
    $ledgerEnvironment = @{
        JUNCTION_LEDGER_LFCA = Join-Path $inputCase.directory 'network.lfca'
        JUNCTION_LEDGER_SNAPSHOT = Join-Path $EvidenceDirectory "render-$($inputCase.vehicles)-round-1.warm.lfrs"
        JUNCTION_LEDGER_VEHICLES = [string]$inputCase.vehicles
        JUNCTION_LEDGER_CELLS = [string]$inputCase.cells
        JUNCTION_LEDGER_OUTPUT = Join-Path $EvidenceDirectory "$name.csv"
    }
    Invoke-EvidenceProcess $name $freeze.executables['junction_ledger']['path'] @('kernel::junction_ledger::junction_reference_ledger', '--ignored', '--exact', '--nocapture', '--test-threads=1') $ledgerEnvironment
}
@{ status = 'integrated, allocation and ledger rounds completed'; finishedUtc = [DateTime]::UtcNow.ToString('o') } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'running.json') -Encoding utf8NoBOM
