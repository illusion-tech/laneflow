param([string]$Output = 'target/pose-681/evidence')
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
function Assert-CleanBaseline {
    $status = @(& git status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect Git worktree' }
    if ($status.Count -ne 0) { throw 'Evidence requires a clean committed worktree' }
    $head = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or $head -ne $baseline) { throw 'Evidence baseline changed' }
}
$baseline = & git rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve evidence baseline' }
$baselineTree = & git rev-parse 'HEAD^{tree}'
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve evidence source tree' }
Assert-CleanBaseline
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
Assert-CleanBaseline
$metadata = [ordered]@{
    baseline = $baseline
    baselineTree = $baselineTree
    worktreeClean = $true
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
    manifest = (Get-FileHash -Algorithm SHA256 -LiteralPath $manifest).Hash
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
Assert-CleanBaseline
Write-Output $out
