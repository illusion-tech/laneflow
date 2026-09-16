param(
    [Parameter(Mandatory)][string]$BaselineExecutable,
    [Parameter(Mandatory)][string]$CandidateExecutable,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(0, 62)][int]$Cpu = 16
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($OutputDirectory)
if ((Test-Path -LiteralPath $root) -and (Get-ChildItem -LiteralPath $root -Force | Select-Object -First 1)) {
    throw 'Use a new empty evidence directory'
}
New-Item -ItemType Directory -Path $root -Force | Out-Null
$binaries = @{
    A = (Resolve-Path -LiteralPath $BaselineExecutable).Path
    B = (Resolve-Path -LiteralPath $CandidateExecutable).Path
    Acontrol = (Resolve-Path -LiteralPath $BaselineExecutable).Path
}
$hashes = @{}
foreach ($kind in $binaries.Keys) { $hashes[$kind] = (Get-FileHash -LiteralPath $binaries[$kind]).Hash }
$schedule = @(@('A', 'B', 'Acontrol'), @('B', 'Acontrol', 'A'), @('Acontrol', 'A', 'B'))
@{
    schema = 'active-order-696-v1'; cpu = $Cpu; rounds = 3
    cases = 42; measuredBatches = 16; warmupBatches = 2
    binaries = $binaries; hashes = $hashes; schedule = $schedule
    started = [DateTimeOffset]::UtcNow.ToString('o')
} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $root 'plan.json') -Encoding utf8
$self = Get-Process -Id $PID
$previous = $self.ProcessorAffinity
$mask = [long]1 -shl $Cpu
$runs = @()
try {
    $self.ProcessorAffinity = [IntPtr]$mask
    foreach ($round in 1..3) {
        foreach ($kind in $schedule[$round - 1]) {
            if ((Get-FileHash -LiteralPath $binaries[$kind]).Hash -ne $hashes[$kind]) { throw 'Frozen binary changed' }
            $start = [DateTimeOffset]::UtcNow.ToString('o')
            $process = Start-Process -FilePath $binaries[$kind] -ArgumentList @('--ignored', '--exact', 'active_order_release_matrix', '--nocapture', '--test-threads=1') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $root "$kind-$round.log") -RedirectStandardError (Join-Path $root "$kind-$round.err")
            if ($process.ProcessorAffinity.ToInt64() -ne $mask) { $process.Kill(); throw 'Affinity mismatch' }
            $process.WaitForExit()
            if ($process.ExitCode -ne 0) { throw "Failed: $kind/$round" }
            $runs += [pscustomobject]@{ kind = $kind; round = $round; pid = $process.Id; start = $start; end = [DateTimeOffset]::UtcNow.ToString('o') }
            $runs | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'runs.json') -Encoding utf8
            Write-Output "$kind/$round complete"
        }
    }
} finally { $self.ProcessorAffinity = $previous }
