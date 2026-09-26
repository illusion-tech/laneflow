#Requires -Version 7.0
param(
    [Parameter(Mandatory)][string]$Artifacts,
    [Parameter(Mandatory)][string]$Plan,
    [Parameter(Mandatory)][string]$Output,
    [Parameter(Mandatory)][string]$RemoteBranch,
    [ValidateRange(1, 100)][int]$Percent = 10
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
function Invoke-Native([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed: $LASTEXITCODE" }
}
function Read-Git([string[]]$Arguments) {
    $value = & git @Arguments
    if ($LASTEXITCODE -ne 0) { throw 'git failed' }
    return ($value -join "`n").Trim()
}
$head = Read-Git @('rev-parse', 'HEAD')
if (Read-Git @('status', '--porcelain')) { throw 'Frozen clean checkout required' }
$remote = Read-Git @('ls-remote', 'origin', "refs/heads/$RemoteBranch")
if (-not $remote.StartsWith("$head`t")) { throw 'Source must be reachable at the named remote branch head' }
if (-not $env:LANEFLOW_HARDWARE_ROLE -or -not $env:LANEFLOW_POWER_ROLE) { throw 'Hardware and power roles required' }
$artifactsPath = (Resolve-Path -LiteralPath $Artifacts).Path
$planPath = (Resolve-Path -LiteralPath $Plan).Path
$destination = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $destination) { throw 'Output must be a new directory' }
$null = New-Item -ItemType Directory -Path $destination
$binDirectory = Join-Path $destination 'bin'
$null = New-Item -ItemType Directory -Path $binDirectory
$binaries = @{}
$builds = [Collections.Generic.List[object]]::new()
foreach ($build in @(
    @{name='wall'; feature='adapter'},
    @{name='allocation'; feature='allocation'},
    @{name='profile'; feature='pose-profiling'}
)) {
    $arguments = @('+1.98.0', 'build', '--locked', '--release', '-p', 'laneflow-urban-harness', '--features', $build.feature)
    Invoke-Native 'cargo' $arguments
    if ((Read-Git @('rev-parse', 'HEAD')) -ne $head -or (Read-Git @('status', '--porcelain'))) { throw 'Source changed during build' }
    $binary = Join-Path $binDirectory "$($build.name).exe"
    Copy-Item -LiteralPath (Join-Path $repo 'target/release/laneflow-urban-harness.exe') -Destination $binary
    $binaries[$build.name] = $binary
    $builds.Add(@{name=$build.name; arguments=$arguments; sha256=(Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()})
}
$configurations = @(
    @{name='original'; mode='FullValidation'},
    @{name='stable-full'; mode='FullValidationSelected'; stride=0},
    @{name='stable-selected'; mode='SelectedPresentation'; stride=0},
    @{name='dynamic-full'; mode='FullValidationSelected'; stride=137},
    @{name='dynamic-selected'; mode='SelectedPresentation'; stride=137}
)
foreach ($configuration in $configurations) {
    $config = @{mode=$configuration.mode}
    if ($configuration.name -ne 'original') {
        $config.selection = @{percent=$Percent; offset=53; stride=$configuration.stride; reverse=$true}
    }
    $config | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $destination "$($configuration.name).json") -Encoding utf8NoBOM
}
$rows = [Collections.Generic.List[object]]::new()
$journal = @{version='issue-713-batch-v1'; source=$head; remote_branch=$RemoteBranch; remote_evidence=$remote; builds=$builds; rows=$rows; complete=$false}
function Save-Journal { $journal | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $destination 'batch.json') -Encoding utf8NoBOM }
Save-Journal
# 每行一个独立进程；轮换全量/选择执行先后，不并行竞争 CPU。
foreach ($kind in @('wall', 'allocation', 'profile')) {
    $rounds = if ($kind -eq 'wall') { 3 } else { 1 }
    for ($round = 1; $round -le $rounds; $round++) {
        $order = @($configurations)
        if ($round % 2 -eq 0) { [array]::Reverse($order) }
        foreach ($configuration in $order) {
            if ($kind -ne 'wall' -and $configuration.name -eq 'original') { continue }
            $name = "$kind-$($configuration.name)-$round"
            $runDirectory = Join-Path $destination $name
            $stdoutPath = Join-Path $destination "$name.stdout.log"
            $stderrPath = Join-Path $destination "$name.stderr.log"
            $info = [Diagnostics.ProcessStartInfo]::new()
            $info.FileName = $binaries[$kind]
            $info.WorkingDirectory = $repo
            $info.UseShellExecute = $false
            $info.CreateNoWindow = $true
            $info.RedirectStandardOutput = $true
            $info.RedirectStandardError = $true
            foreach ($argument in @('evidence', $artifactsPath, $planPath, $runDirectory, 'adapter', '--presentation-config', (Join-Path $destination "$($configuration.name).json"))) {
                $info.ArgumentList.Add($argument)
            }
            $stdout = [IO.File]::Create($stdoutPath)
            $stderr = [IO.File]::Create($stderrPath)
            try {
                $process = [Diagnostics.Process]::Start($info)
                Write-Output "$name pid=$($process.Id) started"
                $outCopy = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
                $errCopy = $process.StandardError.BaseStream.CopyToAsync($stderr)
                $process.WaitForExit()
                $outCopy.GetAwaiter().GetResult()
                $errCopy.GetAwaiter().GetResult()
                if ($process.ExitCode -ne 0) { throw "$name failed: $($process.ExitCode); see $stderrPath" }
            } finally {
                $stdout.Dispose()
                $stderr.Dispose()
            }
            $evidencePath = Join-Path $runDirectory 'evidence.json'
            $evidence = Get-Content -LiteralPath $evidencePath -Raw | ConvertFrom-Json
            if ($evidence.source.commit -ne $head -or $evidence.error -or $evidence.wall_limit_ms -or $evidence.prefix_ticks -or $evidence.completed_ticks -ne $evidence.window.warm_up_ticks + $evidence.window.observation_ticks) { throw "$name incomplete or source drift" }
            $rows.Add(@{name=$name; kind=$kind; configuration=$configuration.name; round=$round; evidence_sha256=(Get-FileHash -LiteralPath $evidencePath -Algorithm SHA256).Hash.ToLowerInvariant(); execution_id=$evidence.execution_id})
            Save-Journal
            Write-Output "$name completed ticks=$($evidence.completed_ticks)"
        }
    }
}
$journal.complete = $true
Save-Journal
Invoke-Native 'node' @((Join-Path $PSScriptRoot 'analyze.mjs'), $destination)
