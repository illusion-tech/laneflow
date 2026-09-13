param([Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
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
Build-FrozenExecutables 'bevy' @('build', '--release', '--locked', '-p', 'laneflow-bevy', '--example', 'junction_scale', '--example', 'junction_scale_allocation', '--example', 'junction_scale_render', '--example', 'junction_scale_analyze', '--features', 'native-example') @{ junction_scale = 'junction_scale'; junction_scale_allocation = 'junction_scale_allocation'; junction_scale_render = 'junction_scale_render'; junction_scale_analyze = 'junction_scale_analyze' }
Build-FrozenExecutables 'generator' @('build', '--release', '--locked', '-p', 'laneflow-junction-generator', '--example', 'generate_grid') @{ generate_grid = 'generate_grid' }
Build-FrozenExecutables 'ledger' @('test', '--release', '--locked', '-p', 'laneflow-runtime', '--lib', '--no-run') @{ laneflow_runtime = 'junction_ledger' }
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
    & $executables['junction_scale']['path'] (Join-Path $destination 'network.lfca') (Join-Path $destination 'grid.catalog.toml') $inputCase.vehicles 18012 36024 prepare $preparedPath
    if ($LASTEXITCODE -ne 0) { throw 'Input preparation failed' }
    $prepared = Get-Content -LiteralPath $preparedPath -Raw | ConvertFrom-Json
    if ($prepared.input.cells -ne $inputCase.cells -or $prepared.input.vehicles -ne $inputCase.vehicles -or $prepared.input.fixed_delta_ms -ne 16) { throw 'Prepared workload differs from declared scale' }
    $inputs += @{ vehicles = $inputCase.vehicles; cells = $inputCase.cells; directory = $destination; files = $files; prepared = $prepared }
}
if ((& git -C $repository rev-parse HEAD).Trim() -ne $sourceCommit -or (& git -C $repository status --porcelain)) { throw 'Sources changed during input freeze' }
# 原始 SMBIOS 值只在内存用于计算，不写入结果或终端。
$identityParts = @(
    (Get-CimInstance Win32_ComputerSystemProduct).UUID,
    (Get-CimInstance Win32_BIOS).SerialNumber,
    (Get-CimInstance Win32_BaseBoard).SerialNumber
) | ForEach-Object { ($_ -replace '\s', '').ToUpperInvariant() }
$identityBytes = [Text.Encoding]::UTF8.GetBytes($identityParts -join "`n")
$hardwareDigest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($identityBytes)).ToLowerInvariant()
$environment = @{
    hardwareIdentityScheme = 'laneflow-p100-hardware-identity-v2'; hardwareIdentitySha256 = $hardwareDigest
    expectedReferenceMachine = 'LF-P100-REF-01'; hardwareIdentityMatches = $hardwareDigest -eq 'be3637be955f6c2c9e9e55b80419794adfac64b709d573602a37da9a8672fd20'
    os = Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber,TotalVisibleMemorySize
    cpu = Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors
    memory = @(Get-CimInstance Win32_PhysicalMemory | Select-Object Capacity,Speed,ConfiguredClockSpeed)
    bios = Get-CimInstance Win32_BIOS | Select-Object Manufacturer,SMBIOSBIOSVersion,ReleaseDate
    gpu = @(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion)
    battery = @(Get-CimInstance -Namespace root/wmi -ClassName BatteryStatus -ErrorAction SilentlyContinue | Select-Object PowerOnline,Charging,Discharging)
    powerPlan = @(& powercfg /getactivescheme); vendorPerformanceMode = 'not programmatically measured'
    rustc = @(& rustc +1.98.0 -Vv); cargo = @(& cargo +1.98.0 -V)
    backgroundProcesses = @(Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 30 ProcessName,Id,CPU,WorkingSet64)
    certification = 'Uncertified: P10 unspecified; release OS and product memory ceilings not frozen'
}
$freeze = @{
    schema = 'junction-scale-freeze-v1'; createdUtc = [DateTime]::UtcNow.ToString('o'); sourceCommit = $sourceCommit
    executables = $executables; inputs = $inputs; environment = $environment
    protocol = @{ rounds = 3; frameInputQuanta = @(0,1,2,4,0); maxCatchUpSteps = 2; warmupTicks = 18012; observationTicks = 36024; signalCycleTicks = 4503; fixedDeltaMs = 16; routeLegs = 64
        workload = 'independent complex-junction reference grid in one TrafficWorld'; seed = 0
        resourceLoad = 'mixed persistent membership/reservation and repeated requests; actual counts are mandatory outputs'
        renderer = '1600x1000 offscreen unlit vehicle cuboids; all presented entities in view; synchronous GPU completion'
        timing = 'three fresh non-instrumented integrated processes per scale; frame classes reported separately'
        allocation = 'separate instrumented full-window process per scale; never used as latency evidence'
        ledger = 'separate optimized test process replays warm snapshot through H/2H/4H; does not time product latency'
    }
}
$freeze | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath (Join-Path $evidenceDirectory 'freeze.json') -Encoding utf8NoBOM
Write-Output "Frozen before formal timing: $evidenceDirectory"
