param(
    [Parameter(Mandatory = $true)][string]$Output,
    # 基线取证时测量程序目录尚未提交；只允许该前缀下的未跟踪文件。
    [string]$AllowUntracked = ''
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo

$baseline = & git rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve evidence baseline' }
$baselineTree = & git rev-parse 'HEAD^{tree}'
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve evidence source tree' }

function Assert-StableBaseline {
    $head = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or $head -ne $baseline) { throw 'Evidence baseline changed' }
    $status = @(& git status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect Git worktree' }
    foreach ($line in $status) {
        if ($line.StartsWith('?? ')) {
            $path = $line.Substring(3)
            if ($AllowUntracked -eq '' -or -not $path.StartsWith($AllowUntracked)) {
                throw "Untracked file outside allowance: $path"
            }
        } else {
            throw "Tracked file must stay clean: $line"
        }
    }
}

Assert-StableBaseline
if (Test-Path -LiteralPath $Output) { throw 'Evidence directory must be new' }
New-Item -ItemType Directory -Path $Output | Out-Null
$out = (Resolve-Path -LiteralPath $Output).Path

$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
& cargo build --locked --offline --release --manifest-path $manifest --target-dir target
if ($LASTEXITCODE -ne 0) { throw 'Normal build failed' }
Copy-Item -LiteralPath target/release/laneflow-issue-711.exe -Destination (Join-Path $out 'wall.exe')
& cargo build --locked --offline --release --features allocation --manifest-path $manifest --target-dir target
if ($LASTEXITCODE -ne 0) { throw 'Allocation build failed' }
Copy-Item -LiteralPath target/release/laneflow-issue-711.exe -Destination (Join-Path $out 'allocation.exe')
Assert-StableBaseline

$metadata = [ordered]@{
    baseline = $baseline
    baselineTree = $baselineTree
    allowUntracked = $AllowUntracked
    worktreeStable = $true
    rustc = (& rustc -Vv) -join "`n"
    cargo = (& cargo -V)
    os = [Environment]::OSVersion.VersionString
    cpu = (Get-CimInstance Win32_Processor | Select-Object -ExpandProperty Name) -join ', '
    logicalProcessors = [Environment]::ProcessorCount
    powerScheme = (& powercfg /getactivesscheme) -join "`n"
    time = (Get-Date).ToUniversalTime().ToString('o')
    binaries = @(Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $out 'wall.exe'), (Join-Path $out 'allocation.exe') | Select-Object Path, Hash)
    sources = @(Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'src') -Filter '*.rs' | Get-FileHash -Algorithm SHA256 | Select-Object Path, Hash)
    lockfile = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $PSScriptRoot 'Cargo.lock')).Hash
    manifest = (Get-FileHash -Algorithm SHA256 -LiteralPath $manifest).Hash
}
$metadata | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $out 'environment.json') -Encoding utf8

$process = [Diagnostics.Process]::GetCurrentProcess()
$previousAffinity = $process.ProcessorAffinity
try {
    # 固定单个 logical processor 减少调度噪声；处理器数不足时退回最高位。
    $cpus = [Environment]::ProcessorCount
    $pin = if ($cpus -ge 17) { [IntPtr]65536 } else { [IntPtr]([Math]::Pow(2, $cpus - 1)) }
    $process.ProcessorAffinity = $pin
    & (Join-Path $out 'wall.exe') > (Join-Path $out 'wall.csv') 2> (Join-Path $out 'wall.log')
    if ($LASTEXITCODE -ne 0) { throw 'Wall run failed' }
    & (Join-Path $out 'allocation.exe') > (Join-Path $out 'allocation.csv') 2> (Join-Path $out 'allocation.log')
    if ($LASTEXITCODE -ne 0) { throw 'Allocation run failed' }
} finally { $process.ProcessorAffinity = $previousAffinity }
Assert-StableBaseline
Write-Output $out
