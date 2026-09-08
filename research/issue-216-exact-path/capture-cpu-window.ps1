param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$TestBinary,
    [Parameter(Mandatory)][ValidateSet('smoke', '10k-256', '10k-16', '1k-256')][string]$Case
)

# Run the workload at the caller's ordinary privilege level. The separately
# user-approved recorder only consumes fixed begin/stop markers.
$ErrorActionPreference = 'Stop'
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$binary = (Resolve-Path -LiteralPath $TestBinary).Path
$tests = @{
    'smoke' = 'cpu_sampling::cpu_sampling_windows_match_frozen_digests'
    '10k-256' = 'cpu_sampling::runtime_cpu_sampling_10k_256'
    '10k-16' = 'cpu_sampling::runtime_cpu_sampling_10k_16'
    '1k-256' = 'cpu_sampling::runtime_cpu_sampling_1k_256'
}
$process = $null
$started = $false
$run = [ordered]@{
    case = $Case
    binary = $binary
    binary_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
    test = $tests[$Case]
    started_utc = [DateTime]::UtcNow.ToString('o')
}

function Wait-RecorderMarker([string]$Name, [int]$Seconds) {
    $until = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not (Test-Path -LiteralPath (Join-Path $output $Name))) {
        if ([DateTime]::UtcNow -ge $until -or (Test-Path -LiteralPath (Join-Path $output 'recorder-result.json'))) {
            throw "Recorder did not produce $Name; inspect recorder.log"
        }
        Start-Sleep -Milliseconds 100
    }
}

try {
    if (Test-Path -LiteralPath (Join-Path $output "$Case.begin")) { throw 'Capture case already exists' }
    $busy = @(Get-Process | Where-Object {
        $_.ProcessName -match '^(cargo|rustc|rust-lld|link|xtask|laneflow-urban-harness|laneflow_runtime-.+|runtime_profile_.+)$'
    })
    if ($busy.Count -ne 0) { throw ('Wait for competing work: ' + ($busy.ProcessName -join ', ')) }
    [IO.File]::WriteAllText((Join-Path $output "$Case.begin"), $run.started_utc)
    $started = $true
    Wait-RecorderMarker "$Case.ready" 90
    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $binary
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.ArgumentList.Add($tests[$Case])
    $psi.ArgumentList.Add('--exact')
    if ($Case -ne 'smoke') { $psi.ArgumentList.Add('--ignored') }
    $psi.ArgumentList.Add('--nocapture')
    $psi.ArgumentList.Add('--test-threads=1')
    $process = [Diagnostics.Process]::Start($psi)
    $run.workload_pid = $process.Id
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $until = [DateTime]::UtcNow.AddSeconds(150)
    while (-not $process.WaitForExit(500)) {
        if ([DateTime]::UtcNow -ge $until) {
            $process.Kill()
            throw 'Owned workload exceeded the 150-second limit'
        }
    }
    [IO.File]::WriteAllText((Join-Path $output "$Case.stdout.txt"), $stdout.GetAwaiter().GetResult())
    [IO.File]::WriteAllText((Join-Path $output "$Case.stderr.txt"), $stderr.GetAwaiter().GetResult())
    $run.exit_code = $process.ExitCode
    if ($process.ExitCode -ne 0) { throw "Workload failed: $($process.ExitCode)" }
    $afterHash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
    if ($afterHash -ne $run.binary_sha256) { throw 'Workload binary changed during capture' }
}
catch {
    $run.error = $_.ToString()
    [IO.File]::WriteAllText((Join-Path $output 'abort'), $run.error)
    throw
}
finally {
    if ($null -ne $process -and -not $process.HasExited) { $process.Kill() }
    if ($started) { [IO.File]::WriteAllText((Join-Path $output "$Case.stop"), [DateTime]::UtcNow.ToString('o')) }
    $run.completed_utc = [DateTime]::UtcNow.ToString('o')
    $run | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $output "$Case.run.json") -Encoding utf8
}
Wait-RecorderMarker "$Case.stopped" 90
Get-Content -LiteralPath (Join-Path $output "$Case.stdout.txt")
Write-Output "CPU_CAPTURE=$output/$Case.etl"
