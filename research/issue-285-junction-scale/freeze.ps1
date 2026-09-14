param([Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'environment.ps1')
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory, (Get-Location).Path)
if ($OutputDirectory.Equals($repository, [StringComparison]::OrdinalIgnoreCase) -or $OutputDirectory.StartsWith($repository + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Evidence directory must be outside the worktree' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Freeze directory must be new' }
$sourceCommit = (& git -C $repository rev-parse HEAD).Trim()
if (& git -C $repository status --porcelain) { throw 'Commit the measured sources before freezing' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$evidenceDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$binaryDirectory = Join-Path $evidenceDirectory 'bin'
New-Item -ItemType Directory -Path $binaryDirectory | Out-Null
$executables = @{}
function Build-FrozenExecutables([string]$Name, [string[]]$CargoArguments, [hashtable]$Targets) {
    $buildLog = Join-Path $evidenceDirectory "$Name.build.jsonl"
    Push-Location -LiteralPath $repository
    try {
        & cargo +1.98.0 @CargoArguments --message-format=json 1> $buildLog 2> (Join-Path $evidenceDirectory "$Name.build.log")
        if ($LASTEXITCODE -ne 0) { throw "Controlled build failed: $Name" }
    } finally { Pop-Location }
    $artifacts = @(Get-Content -LiteralPath $buildLog | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.executable })
    foreach ($targetName in $Targets.Keys) {
        $artifact = @($artifacts | Where-Object { $_.target.name -eq $targetName })
        if ($artifact.Count -ne 1) { throw "Expected one built executable: $targetName" }
        $sourcePath = $artifact[0].executable
        $name = $Targets[$targetName]
        $path = Join-Path $binaryDirectory "$name.exe"
        Copy-Item -LiteralPath $sourcePath -Destination $path
        $executables[$name] = @{ path = $path; sourcePath = $sourcePath; buildLog = $buildLog; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
    }
}
Build-FrozenExecutables 'bevy' @('build', '--release', '--locked', '-p', 'laneflow-bevy', '--example', 'junction_scale', '--example', 'junction_scale_render', '--example', 'junction_scale_analyze', '--features', 'native-example') @{ junction_scale = 'junction_scale'; junction_scale_render = 'junction_scale_render'; junction_scale_analyze = 'junction_scale_analyze' }
Build-FrozenExecutables 'generator' @('build', '--release', '--locked', '-p', 'laneflow-junction-generator', '--example', 'generate_grid') @{ generate_grid = 'generate_grid' }
if ((& git -C $repository rev-parse HEAD).Trim() -ne $sourceCommit -or (& git -C $repository status --porcelain)) { throw 'Sources changed during controlled build' }
$configPath = Join-Path $evidenceDirectory 'source-config.toml'
Copy-Item -LiteralPath (Join-Path $repository 'examples/config/v0.1-complex-junction.toml') -Destination $configPath
$configHash = (Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash.ToLowerInvariant()
$inputs = @()
foreach ($inputCase in @(@{ vehicles = 10000; cells = 32 }, @{ vehicles = 100000; cells = 320 })) {
    $destination = Join-Path $evidenceDirectory "input-$($inputCase.vehicles)"
    & $executables['generate_grid']['path'] $configPath $inputCase.cells $destination 2> (Join-Path $evidenceDirectory "generate-$($inputCase.vehicles).log")
    if ($LASTEXITCODE -ne 0) { throw 'Grid generation failed' }
    $files = @{}
    foreach ($file in @('network.lfca', 'grid.catalog.toml', 'source-config.toml')) {
        $files[$file] = (Get-FileHash -LiteralPath (Join-Path $destination $file) -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if ($files['source-config.toml'] -ne $configHash) { throw 'Grid configuration differs from frozen source configuration' }
    $preparedPath = Join-Path $destination 'prepared.json'
    & $executables['junction_scale']['path'] (Join-Path $destination 'network.lfca') (Join-Path $destination 'grid.catalog.toml') $inputCase.vehicles 0 4096 prepare $preparedPath
    if ($LASTEXITCODE -ne 0) { throw 'Input preparation failed' }
    $prepared = Get-Content -LiteralPath $preparedPath -Raw | ConvertFrom-Json
    if ($prepared.input.cells -ne $inputCase.cells -or $prepared.input.vehicles -ne $inputCase.vehicles -or $prepared.input.fixed_delta_ms -ne 16) { throw 'Prepared workload differs from declared scale' }
    foreach ($file in @('prepared.json', 'prepared.initial.lfrs')) {
        $files[$file] = (Get-FileHash -LiteralPath (Join-Path $destination $file) -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if ($files['prepared.initial.lfrs'] -ne $prepared.snapshot_sha256) { throw 'Prepared initial snapshot digest mismatch' }
    $inputs += @{ vehicles = $inputCase.vehicles; cells = $inputCase.cells; directory = $destination; files = $files; prepared = $prepared }
}
if ((& git -C $repository rev-parse HEAD).Trim() -ne $sourceCommit -or (& git -C $repository status --porcelain)) { throw 'Sources changed during input freeze' }
$environment = Get-JunctionEnvironment
$freeze = @{
    schema = 'junction-scale-freeze-v2'; createdUtc = [DateTime]::UtcNow.ToString('o'); sourceCommit = $sourceCommit
    executables = $executables; inputs = $inputs; environment = $environment
    protocol = @{ rounds = 1; batchWallLimitMilliseconds = 600000; maxTicks = 4096; frameInputQuanta = @(0,1,2,4,0); maxCatchUpSteps = 2; warmupTicks = 0; observationTicks = 4096; fixedDeltaMs = 16; routeLegs = 64
        workload = 'independent complex-junction reference grid in one TrafficWorld'; seed = 0
        resourceLoad = 'report actual observed membership, reservations and requests; no minimum long-window coverage claim'
        renderer = '1600x1000 offscreen unlit vehicle cuboids; all presented entities in view; synchronous GPU completion'
        timing = 'one bounded cold-start integrated process per scale; at most 4096 ticks each, at most 600 seconds for the whole batch'
        acceptance = 'runnable and report actual performance; budget compliance and product certification are not required'
        allocation = 'not measured by this protocol'
        ledger = 'not measured by this protocol; process memory peaks are reported'
    }
}
$freeze | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath (Join-Path $evidenceDirectory 'freeze.json') -Encoding utf8NoBOM
Write-Output "Frozen before bounded timing: $evidenceDirectory"
