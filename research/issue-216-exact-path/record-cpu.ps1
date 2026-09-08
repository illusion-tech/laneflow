param([Parameter(Mandatory)][string]$OutputDirectory)

# This elevated helper only starts/stops four fixed, owned WPR sessions.
# It never executes workload commands or changes persistent system policy.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$allowed = [IO.Path]::GetFullPath('target/issue-216-cpu-profile', $repo)
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
if (-not $output.StartsWith($allowed + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Capture output must be a new child of target/issue-216-cpu-profile'
}
if (-not [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'The recorder requires a user-approved elevated process'
}
$wpr = 'C:/Windows/System32/wpr.exe'
$cases = @('smoke', '10k-256', '10k-16', '1k-256')
$deadline = [DateTime]::UtcNow.AddMinutes(15)
$activeInstance = $null
$activeTrace = $null
$result = [ordered]@{ recorder_pid = $PID; completed = $false; error = $null; traces = @() }

function Wait-CaptureMarker([string]$Name, [int]$Seconds) {
    $until = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not (Test-Path -LiteralPath (Join-Path $output $Name))) {
        if ((Test-Path -LiteralPath (Join-Path $output 'abort')) -or
            [DateTime]::UtcNow -ge $until -or [DateTime]::UtcNow -ge $deadline) {
            throw "Capture aborted or timed out while waiting for $Name"
        }
        Start-Sleep -Milliseconds 100
    }
}

Start-Transcript -LiteralPath (Join-Path $output 'recorder.log') -NoClobber | Out-Null
try {
    foreach ($case in $cases) {
        Wait-CaptureMarker "$case.begin" 600
        $instance = "LaneFlow216-$PID-$case"
        $trace = Join-Path $output "$case.etl"
        if (Test-Path -LiteralPath $trace) { throw "Refusing to replace $trace" }
        & $wpr -start CPU -filemode -instancename $instance
        $startExit = $LASTEXITCODE
        if ($startExit -ne 0) { throw "WPR start failed for $case : $startExit" }
        $activeInstance = $instance
        $activeTrace = $trace
        [IO.File]::WriteAllText((Join-Path $output "$case.ready"), $instance)
        Wait-CaptureMarker "$case.stop" 180
        & $wpr -stop $trace -skipPdbGen -compress -instancename $instance
        $stopExit = $LASTEXITCODE
        if ($stopExit -ne 0) { throw "WPR stop failed for $instance : $stopExit" }
        $activeInstance = $null
        $result.traces += $trace
        [IO.File]::WriteAllText((Join-Path $output "$case.stopped"), [DateTime]::UtcNow.ToString('o'))
    }
    $result.completed = $true
}
catch {
    $result.error = $_.ToString()
    Write-Output $result.error
}
finally {
    if ($null -ne $activeInstance) {
        # Preserve a partial trace and stop only this helper's own instance.
        & $wpr -stop $activeTrace -skipPdbGen -compress -instancename $activeInstance
        $result.cleanup_exit = $LASTEXITCODE
    }
    $result | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $output 'recorder-result.json') -Encoding utf8
    Stop-Transcript | Out-Null
}
if (-not $result.completed) { exit 1 }
