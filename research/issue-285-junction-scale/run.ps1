param([Parameter(Mandatory)][string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
$freezePath = Join-Path $EvidenceDirectory 'freeze.json'
$freeze = Get-Content -LiteralPath $freezePath -Raw | ConvertFrom-Json -AsHashtable
$freezeHash = (Get-FileHash -LiteralPath $freezePath -Algorithm SHA256).Hash.ToLowerInvariant()
foreach ($binary in $freeze.executables.Values) {
    if ((Get-FileHash -LiteralPath $binary.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $binary.sha256) { throw 'Frozen binary changed' }
}
foreach ($inputCase in $freeze.inputs) {
    foreach ($file in $inputCase.files.Keys) {
        if ((Get-FileHash -LiteralPath (Join-Path $inputCase.directory $file) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $inputCase.files[$file]) { throw 'Frozen input changed' }
    }
}

function Invoke-EvidenceProcess([string]$Name, [string]$Executable, [string[]]$Arguments, [hashtable]$Environment = @{}) {
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
    $stdoutFile = [IO.File]::Open((Join-Path $EvidenceDirectory "$Name.stdout.log"), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
    $stderrFile = [IO.File]::Open((Join-Path $EvidenceDirectory "$Name.stderr.log"), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
    $stdout = $process.StandardOutput.BaseStream.CopyToAsync($stdoutFile)
    $stderr = $process.StandardError.BaseStream.CopyToAsync($stderrFile)
    $peakWorkingSet = 0L
    $peakPrivateSampled = 0L
    $peakPaged = 0L
    $lastStatus = [DateTime]::MinValue
    while (-not $process.WaitForExit(500)) {
        $process.Refresh()
        $peakWorkingSet = [Math]::Max($peakWorkingSet, $process.PeakWorkingSet64)
        $peakPrivateSampled = [Math]::Max($peakPrivateSampled, $process.PrivateMemorySize64)
        $peakPaged = [Math]::Max($peakPaged, $process.PeakPagedMemorySize64)
        if (([DateTime]::UtcNow - $lastStatus).TotalSeconds -ge 10) {
            @{ run = $Name; pid = $process.Id; startedUtc = $started.ToString('o'); elapsedSeconds = ([DateTime]::UtcNow - $started).TotalSeconds; peakWorkingSetBytes = $peakWorkingSet; privateBytesSampledPeak = $peakPrivateSampled } |
                ConvertTo-Json | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'running.json') -Encoding utf8NoBOM
            $lastStatus = [DateTime]::UtcNow
        }
    }
    $stdout.GetAwaiter().GetResult()
    $stderr.GetAwaiter().GetResult()
    $stdoutFile.Dispose()
    $stderrFile.Dispose()
    $metadata = @{ schema = 'junction-scale-process-v1'; name = $Name; pid = $process.Id; sourceCommit = $freeze.sourceCommit; freezeSha256 = $freezeHash
        executable = $Executable; binarySha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant(); arguments = $Arguments; environmentOverrides = $Environment
        startedUtc = $started.ToString('o'); finishedUtc = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = ([DateTime]::UtcNow - $started).TotalSeconds; exitCode = $process.ExitCode
        peakWorkingSetBytes = $peakWorkingSet; privateBytesSampledPeak = $peakPrivateSampled; processCommitPeakBytes = $peakPaged
        memorySampling = '500ms private bytes; OS lifetime peak working set and peak pagefile-backed commit; component ledger is separate'
        backgroundProcessesBefore = $background
    }
    $metadata | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $metadataPath -Encoding utf8NoBOM
    Write-Output "$Name exit=$($process.ExitCode) elapsed=$([Math]::Round($metadata.elapsedSeconds,1))s"
    if ($process.ExitCode -ne 0) { throw "Execution failed; artifacts preserved: $Name" }
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
