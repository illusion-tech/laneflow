param([string]$Output = 'target/pose-681/evidence')
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
if (Test-Path -LiteralPath $Output) { throw 'Evidence directory must be new' }
New-Item -ItemType Directory -Path $Output | Out-Null
$out = (Resolve-Path -LiteralPath $Output).Path
$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
& cargo build --locked --offline --release --manifest-path $manifest --target-dir target
if ($LASTEXITCODE -ne 0) { throw 'Normal build failed' }
Copy-Item -LiteralPath target/release/laneflow-issue-681.exe -Destination (Join-Path $out 'wall.exe')
& cargo build --locked --offline --release --features allocation --manifest-path $manifest --target-dir target
if ($LASTEXITCODE -ne 0) { throw 'Allocation build failed' }
Copy-Item -LiteralPath target/release/laneflow-issue-681.exe -Destination (Join-Path $out 'allocation.exe')
$metadata = [ordered]@{
    baseline = (& git rev-parse HEAD)
    productionBase = (& git merge-base HEAD origin/main)
    rustc = (& rustc -Vv) -join "`n"
    cargo = (& cargo -V)
    os = [Environment]::OSVersion.VersionString
    cpu = (Get-CimInstance Win32_Processor | Select-Object -ExpandProperty Name) -join ', '
    logicalProcessors = [Environment]::ProcessorCount
    affinity = 'logical processor 16'
    powerScheme = (& powercfg /getactivescheme) -join "`n"
    time = (Get-Date).ToUniversalTime().ToString('o')
    binaries = @(Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $out 'wall.exe'), (Join-Path $out 'allocation.exe') | Select-Object Path,Hash)
    sources = @(Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'src') -Filter '*.rs' | Get-FileHash -Algorithm SHA256 | Select-Object Path,Hash)
    lockfile = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $PSScriptRoot 'Cargo.lock')).Hash
    processSnapshot = @(Get-Process cargo,rustc -ErrorAction SilentlyContinue | Select-Object ProcessName,Id)
}
$metadata | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $out 'environment.json') -Encoding utf8
$process = [Diagnostics.Process]::GetCurrentProcess()
$previousAffinity = $process.ProcessorAffinity
try {
    $process.ProcessorAffinity = [IntPtr]65536
    for ($round = 1; $round -le 3; $round++) {
        foreach ($count in @(10000,100000)) {
            & (Join-Path $out 'wall.exe') $count > (Join-Path $out "wall-$round-$count.csv") 2> (Join-Path $out "wall-$round-$count.log")
            if ($LASTEXITCODE -ne 0) { throw "Wall run $round/$count failed" }
        }
    }
    foreach ($count in @(10000,100000)) {
        & (Join-Path $out 'allocation.exe') $count > (Join-Path $out "allocation-$count.csv") 2> (Join-Path $out "allocation-$count.log")
        if ($LASTEXITCODE -ne 0) { throw "Allocation run $count failed" }
    }
} finally { $process.ProcessorAffinity = $previousAffinity }
Write-Output $out
