param(
    [Parameter(Mandatory)][string]$Binary,
    [Parameter(Mandatory)][string]$Source,
    [Parameter(Mandatory)][string]$OutputRoot,
    [Parameter(Mandatory)][string]$Label,
    [ValidateSet('10k','100k')][string]$Scale = '10k',
    [ValidateSet('all','p3','waiting','both')][string]$Mode = 'all',
    [int]$Ticks = 512,
    [int]$Workers = 4,
    [switch]$Diagnostic,
    [string]$FrozenInputs = 'E:/projects/laneflow-evidence/issue-707/4de40e04'
)
$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path -LiteralPath $Binary).Path
$sourcePath = (Resolve-Path -LiteralPath $Source).Path
$artifactsPath = Join-Path $FrozenInputs "inputs/urban-$Scale"
$planPath = Join-Path $FrozenInputs "plans/$Scale-performance.toml"
$rootPath = [System.IO.Path]::GetFullPath($OutputRoot)
New-Item -ItemType Directory -Force -Path $rootPath | Out-Null
if ($Label -notmatch '^[a-zA-Z0-9-]+$') { throw 'invalid label' }
$runPath = Join-Path $rootPath $Label
$metaPath = Join-Path $rootPath "$Label.process.json"
if ((Test-Path -LiteralPath $runPath) -or (Test-Path -LiteralPath $metaPath)) { throw 'evidence exists' }
$identity = & python (Join-Path $PSScriptRoot 'seal.py') $sourcePath
if ($LASTEXITCODE -ne 0) { throw 'source seal failed' }
$psi = [System.Diagnostics.ProcessStartInfo]::new()
$psi.FileName = $exePath
$psi.WorkingDirectory = $sourcePath
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
foreach ($arg in @('scope-prefix',$artifactsPath,$planPath,$runPath,"$Workers","$Ticks")) {
    $psi.ArgumentList.Add($arg)
}
$psi.Environment['LF757_SCOPE'] = $Mode
$runId = [guid]::NewGuid().ToString()
$psi.Environment['LF757_RUN_ID'] = $runId
if ($Diagnostic) { $psi.Environment['LF757_DIAGNOSTIC'] = '1' }
else { [void]$psi.Environment.Remove('LF757_DIAGNOSTIC') }
$metadata = [ordered]@{
    label=$Label; run_id=$runId; mode=$Mode; scale=$Scale; workers=$Workers; ticks=$Ticks;
    diagnostic=[bool]$Diagnostic; source=$sourcePath; source_identity_before=$identity;
    bundle_head=(& git rev-parse HEAD); bundle_status=(& git status --porcelain | Out-String).Trim();
    binary=$exePath; binary_sha256=(Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash;
    plan=$planPath; plan_sha256=(Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash;
    hardware=($env:PROCESSOR_IDENTIFIER); logical_cpus=$env:NUMBER_OF_PROCESSORS;
    power=(& powercfg /getactivescheme | Out-String).Trim();
    battery=(@(Get-CimInstance Win32_Battery | Select-Object BatteryStatus,EstimatedChargeRemaining));
    started_utc=[DateTime]::UtcNow.ToString('o');
}
$metadata | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $metaPath -Encoding utf8NoBOM
$process = [System.Diagnostics.Process]::Start($psi)
$stdoutTask = $process.StandardOutput.ReadToEndAsync()
$stderrTask = $process.StandardError.ReadToEndAsync()
$process.WaitForExit()
$metadata['exit_code'] = $process.ExitCode
$summaryPath = Join-Path $runPath 'summary.json'
$metadata['peak_working_set_bytes'] = if (Test-Path -LiteralPath $summaryPath) {
    (Get-Content -LiteralPath $summaryPath -Raw | ConvertFrom-Json).peak_resident_bytes
} else { $null }
$metadata['cpu_seconds'] = $process.TotalProcessorTime.TotalSeconds
$metadata['ended_utc'] = [DateTime]::UtcNow.ToString('o')
$stdoutTask.Result | Set-Content -LiteralPath (Join-Path $rootPath "$Label.stdout") -Encoding utf8NoBOM
$stderrTask.Result | Set-Content -LiteralPath (Join-Path $rootPath "$Label.stderr") -Encoding utf8NoBOM
$afterIdentity = & python (Join-Path $PSScriptRoot 'seal.py') $sourcePath
if ($LASTEXITCODE -ne 0) { throw 'post-run source seal failed' }
$metadata['source_identity_after'] = $afterIdentity
$metadata['source_unchanged'] = $identity -eq $afterIdentity
$metadata['binary_unchanged'] = $metadata.binary_sha256 -eq (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash
$metadata['plan_unchanged'] = $metadata.plan_sha256 -eq (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash
$metadata['bundle_head_after'] = (& git rev-parse HEAD)
$metadata['bundle_head_unchanged'] = $metadata.bundle_head -eq $metadata.bundle_head_after
$metadata | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $metaPath -Encoding utf8NoBOM
Write-Output "$Label exit=$($process.ExitCode) peak=$($metadata.peak_working_set_bytes) source_unchanged=$($metadata.source_unchanged)"
if ($process.ExitCode -ne 0 -or -not $metadata.source_unchanged -or -not $metadata.binary_unchanged -or -not $metadata.plan_unchanged -or -not $metadata.bundle_head_unchanged) { throw "run failed: $Label" }
