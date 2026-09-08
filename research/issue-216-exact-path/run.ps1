param(
    [Parameter(Mandatory)][string]$TestBinary,
    [Parameter(Mandatory)][string]$ProductionBinary,
    [string]$OutputDirectory = ('target/issue-216-results-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
Push-Location -LiteralPath $repo
try {
    $unit = (Resolve-Path -LiteralPath $TestBinary).Path
    $production = (Resolve-Path -LiteralPath $ProductionBinary).Path
    $output = [IO.Path]::GetFullPath($OutputDirectory, $repo)
    if (Test-Path -LiteralPath $output) {
        throw "Refusing to replace an existing evidence directory: $output"
    }
    New-Item -ItemType Directory -Path $output | Out-Null

    function Assert-NoHeavyProcess {
        $busy = @(Get-Process | Where-Object {
            $_.ProcessName -match '^(cargo|rustc|rust-lld|link|laneflow-urban-harness|laneflow_runtime-.+|runtime_profile_.+)$'
        })
        if ($busy.Count -ne 0) {
            throw ('Finish compilation and other Runtime runs first: ' + ($busy.ProcessName -join ', '))
        }
    }

    $files = @(
        'Cargo.lock',
        'crates/laneflow-runtime/Cargo.toml',
        'crates/laneflow-runtime/src/kernel/mod.rs',
        'crates/laneflow-runtime/src/kernel/occupancy.rs',
        'crates/laneflow-runtime/src/kernel/tick.rs',
        'crates/laneflow-runtime/src/kernel/phase_equivalence.rs',
        'crates/laneflow-runtime/src/kernel/performance_profile.rs',
        'crates/laneflow-runtime/src/kernel/tests/exact_path.rs',
        'crates/laneflow-runtime/src/kernel/tests/occupancy_exact_candidate.rs',
        'crates/laneflow-runtime/src/kernel/tests/performance_profile/runtime_profile.rs',
        'crates/laneflow-runtime/src/kernel/tests/performance_profile/cutover_scale.rs',
        'crates/laneflow-runtime/src/kernel/tests/performance_profile/runtime_profile_evidence.rs'
    )
    $blobs = [ordered]@{}
    foreach ($file in $files) {
        $blobs[$file] = (& git hash-object -- $file)
        if ($LASTEXITCODE -ne 0) { throw "Cannot hash source: $file" }
    }
    $unitHash = (Get-FileHash -LiteralPath $unit -Algorithm SHA256).Hash
    $productionHash = (Get-FileHash -LiteralPath $production -Algorithm SHA256).Hash
    Assert-NoHeavyProcess
    $environment = [ordered]@{
        started_utc = [DateTime]::UtcNow.ToString('o')
        git_head = (& git rev-parse HEAD)
        source_blobs = $blobs
        rustc = @(& rustc +1.98.0 -vV)
        cargo = (& cargo +1.98.0 --version)
        cpu = (Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores, NumberOfLogicalProcessors)
        os = (Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber)
        power_scheme = (& powercfg /getactivescheme)
        battery = (Get-CimInstance Win32_Battery | Select-Object BatteryStatus, EstimatedChargeRemaining)
        unit_sha256 = $unitHash
        production_sha256 = $productionHash
        profile = 'release default; no CPU affinity or frequency lock; test-build A/B retains disabled timing hooks and StatsAlloc<System>'
    }

    $runs = @(
        @($production, 'runtime_profile_wall_clock', 'wall-clock.txt'),
        @($unit, 'runtime_profile_stages', 'stages.txt'),
        @($unit, 'occupancy_exact_attribution', 'attribution.txt'),
        @($unit, 'occupancy_exact_paired_windows', 'pairs.txt'),
        @($unit, 'occupancy_exact_equivalence_windows', 'equivalence.txt')
    )
    foreach ($run in $runs) {
        Assert-NoHeavyProcess
        $arguments = @($run[1], '--ignored', '--nocapture', '--test-threads=1')
        & $run[0] @arguments 2>&1 | Tee-Object -FilePath (Join-Path $output $run[2])
        if ($LASTEXITCODE -ne 0) { throw "Research entry failed: $($run[1])" }
        $log = Join-Path $output $run[2]
        $content = Get-Content -LiteralPath $log -Raw
        [IO.File]::WriteAllText($log, $content.TrimEnd() + "`n")
    }
    foreach ($file in $files) {
        $actual = & git hash-object -- $file
        if ($LASTEXITCODE -ne 0 -or $actual -ne $blobs[$file]) {
            throw "Source changed during measurement: $file"
        }
    }
    if ((Get-FileHash -LiteralPath $unit -Algorithm SHA256).Hash -ne $unitHash -or
        (Get-FileHash -LiteralPath $production -Algorithm SHA256).Hash -ne $productionHash) {
        throw 'A measured binary changed during the run'
    }
    $environment.completed_utc = [DateTime]::UtcNow.ToString('o')
    $environment | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'environment.json') -Encoding utf8
    Write-Output "Evidence saved to $output"
}
finally {
    Pop-Location
}
