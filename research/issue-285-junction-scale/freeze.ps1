param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Grid32Directory,
    [Parameter(Mandatory)][string]$Grid320Directory,
    [Parameter(Mandatory)][string]$LedgerExecutable
)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Freeze directory must be new' }
$sourceCommit = (& git -C $repository rev-parse HEAD).Trim()
if (& git -C $repository status --porcelain) { throw 'Commit the measured sources before freezing' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$evidenceDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$executables = @{}
foreach ($name in @('junction_scale', 'junction_scale_allocation', 'junction_scale_render')) {
    $path = Join-Path $repository "target/release/examples/$name.exe"
    $executables[$name] = @{ path = $path; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$ledgerPath = (Resolve-Path -LiteralPath $LedgerExecutable).Path
$executables['junction_ledger'] = @{ path = $ledgerPath; sha256 = (Get-FileHash -LiteralPath $ledgerPath -Algorithm SHA256).Hash.ToLowerInvariant() }
$inputs = @()
foreach ($inputCase in @(@{ vehicles = 10000; cells = 32; directory = $Grid32Directory }, @{ vehicles = 100000; cells = 320; directory = $Grid320Directory })) {
    $destination = Join-Path $evidenceDirectory "input-$($inputCase.vehicles)"
    New-Item -ItemType Directory -Path $destination | Out-Null
    $files = @{}
    foreach ($file in @('network.lfca', 'grid.catalog.toml', 'source-config.toml')) {
        Copy-Item -LiteralPath (Join-Path $inputCase.directory $file) -Destination (Join-Path $destination $file)
        $files[$file] = (Get-FileHash -LiteralPath (Join-Path $destination $file) -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $preparedPath = Join-Path $destination 'prepared.json'
    & $executables['junction_scale']['path'] (Join-Path $destination 'network.lfca') (Join-Path $destination 'grid.catalog.toml') $inputCase.vehicles 18012 36024 prepare $preparedPath
    if ($LASTEXITCODE -ne 0) { throw 'Input preparation failed' }
    $prepared = Get-Content -LiteralPath $preparedPath -Raw | ConvertFrom-Json
    $inputs += @{ vehicles = $inputCase.vehicles; cells = $inputCase.cells; directory = $destination; files = $files; prepared = $prepared }
}
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
    protocol = @{ rounds = 3; frameSteps = @(0,1,2,8); warmupTicks = 18012; observationTicks = 36024; signalCycleTicks = 4503; fixedDeltaMs = 16; routeLegs = 64
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
