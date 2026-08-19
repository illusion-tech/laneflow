<#
.SYNOPSIS
Manages one loopback HTTP mcpls service for one LaneFlow Git worktree.

.DESCRIPTION
Ensure is the fail-open Codex Local Environment setup action. Start is the
strict manual action. Status, Stop, and Prune provide explicit lifecycle
control without matching or stopping processes by name.

The script never installs, downloads, or upgrades mcpls. Install the pinned
HTTP-enabled build separately, then let this script validate it.
#>
[CmdletBinding()]
param(
    [ValidateSet('Ensure', 'Start', 'Status', 'Stop', 'Prune')]
    [string]$Action = 'Ensure',

    [ValidateRange(5, 300)]
    [int]$StartupTimeoutSeconds = 60,

    [string]$WorktreeRoot,

    [string]$McplsPath,

    [Parameter(DontShow)]
    [string]$StateRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$script:McplsVersion = '0.3.9'
$script:StateSchemaVersion = 2
$script:TemplateSchemaVersion = 1
$script:GeneratedConfigSchemaVersion = 1
$script:PortMinimum = 41000
$script:PortMaximum = 48999
$script:HttpPath = '/mcp'
$script:GeneratedConfigMarker = '# laneflow-mcpls-generated-schema: 1'

function Get-NormalizedPath {
    param([Parameter(Mandatory)][string]$Path)

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    if (Test-Path -LiteralPath $fullPath) {
        $fullPath = [string](Get-Item -LiteralPath $fullPath -Force -ErrorAction Stop).FullName
    }
    return [System.IO.Path]::TrimEndingDirectorySeparator($fullPath)
}

function Test-PathEqual {
    param(
        [Parameter(Mandatory)][string]$Left,
        [Parameter(Mandatory)][string]$Right
    )

    try {
        return [string]::Equals(
            (Get-NormalizedPath -Path $Left),
            (Get-NormalizedPath -Path $Right),
            [System.StringComparison]::Ordinal
        )
    }
    catch {
        return $false
    }
}

function Get-ApplicationPath {
    param([Parameter(Mandatory)][string]$Name)

    $command = Get-Command -Name $Name -CommandType Application -ErrorAction Stop |
        Select-Object -First 1
    return [string]$command.Source
}

function Get-CanonicalWorktreeRoot {
    param([string]$RootHint)

    $git = Get-ApplicationPath -Name 'git'
    $candidate = if ([string]::IsNullOrWhiteSpace($RootHint)) {
        Split-Path -Parent $PSScriptRoot
    }
    else {
        $RootHint
    }

    $output = & $git '-C' $candidate 'rev-parse' '--show-toplevel' 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw "Not a Git worktree: $candidate"
    }

    $root = (($output | Out-String).Trim())
    if ([string]::IsNullOrWhiteSpace($root)) {
        throw "git rev-parse returned an empty worktree root for $candidate"
    }

    return Get-NormalizedPath -Path $root
}

function Get-Sha256Hex {
    param([Parameter(Mandatory)][string]$Value)

    $bytes = [System.Text.Encoding]::UTF8.GetBytes($Value)
    $hash = [System.Security.Cryptography.SHA256]::HashData($bytes)
    return [System.Convert]::ToHexString($hash).ToLowerInvariant()
}

function Get-FileSha256Hex {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Cannot hash missing file: $Path"
    }
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $hash = [System.Security.Cryptography.SHA256]::HashData($bytes)
    return [System.Convert]::ToHexString($hash).ToLowerInvariant()
}

function Get-WorktreeId {
    param([Parameter(Mandatory)][string]$CanonicalRoot)

    $identity = Get-NormalizedPath -Path $CanonicalRoot
    return Get-Sha256Hex -Value $identity
}

function Get-LaneFlowMcplsStateRoot {
    param([string]$Override)

    if (-not [string]::IsNullOrWhiteSpace($Override)) {
        return Get-NormalizedPath -Path $Override
    }
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw 'LOCALAPPDATA is not available.'
    }

    return Get-NormalizedPath -Path (
        Join-Path $env:LOCALAPPDATA 'LaneFlow\mcpls\worktrees'
    )
}

function Get-WorktreeContext {
    param(
        [string]$RootHint,
        [string]$StateRootOverride
    )

    $root = Get-CanonicalWorktreeRoot -RootHint $RootHint
    $worktreeId = Get-WorktreeId -CanonicalRoot $root
    $allStateRoot = Get-LaneFlowMcplsStateRoot -Override $StateRootOverride
    $stateDirectory = Join-Path $allStateRoot $worktreeId
    $lockDirectory = Join-Path $allStateRoot '.locks'

    return [pscustomobject]@{
        Root = $root
        WorktreeId = $worktreeId
        McplsConfigPath = Join-Path $root 'mcpls.toml'
        TemplatePath = Join-Path $root '.codex\config.template.toml'
        GeneratedConfigPath = Join-Path $root '.codex\config.toml'
        AllStateRoot = $allStateRoot
        StateDirectory = $stateDirectory
        StatePath = Join-Path $stateDirectory 'state.json'
        LogPath = Join-Path $stateDirectory 'lifecycle.log'
        LockDirectory = $lockDirectory
        WorktreeLockPath = Join-Path $lockDirectory "$worktreeId.lock"
        PortLockPath = Join-Path $lockDirectory 'port-allocation.lock'
    }
}

function Write-AtomicUtf8File {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Content
    )

    $directory = Split-Path -Parent $Path
    [System.IO.Directory]::CreateDirectory($directory) | Out-Null
    $temporaryPath = Join-Path $directory (
        ".$(Split-Path -Leaf $Path).$([System.Guid]::NewGuid().ToString('N')).tmp"
    )
    try {
        $encoding = [System.Text.UTF8Encoding]::new($false)
        [System.IO.File]::WriteAllText($temporaryPath, $Content, $encoding)
        [System.IO.File]::Move($temporaryPath, $Path, $true)
    }
    finally {
        if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
    }
}

function Write-LifecycleLog {
    param(
        [Parameter(Mandatory)]$Context,
        [Parameter(Mandatory)][string]$Message
    )

    [System.IO.Directory]::CreateDirectory($Context.StateDirectory) | Out-Null
    $line = "{0} {1}`n" -f [DateTimeOffset]::UtcNow.ToString('O'), $Message
    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::AppendAllText($Context.LogPath, $line, $encoding)
}

function Enter-FileLock {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][DateTimeOffset]$Deadline
    )

    $directory = Split-Path -Parent $Path
    [System.IO.Directory]::CreateDirectory($directory) | Out-Null
    while ([DateTimeOffset]::UtcNow -lt $Deadline) {
        try {
            $stream = [System.IO.File]::Open(
                $Path,
                [System.IO.FileMode]::OpenOrCreate,
                [System.IO.FileAccess]::ReadWrite,
                [System.IO.FileShare]::None
            )
            return [pscustomobject]@{ Stream = $stream; Path = $Path }
        }
        catch [System.IO.IOException] {
            Start-Sleep -Milliseconds 50
        }
    }
    throw "Timed out waiting for cross-session lock $Path"
}

function Exit-FileLock {
    param($Lease)

    if ($null -eq $Lease) {
        return
    }
    $Lease.Stream.Dispose()
}

function Get-RemainingMilliseconds {
    param(
        [Parameter(Mandatory)][DateTimeOffset]$Deadline,
        [ValidateRange(1, 300000)][int]$Maximum = 30000
    )

    $remaining = [int][Math]::Floor(($Deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds)
    if ($remaining -le 0) {
        throw 'mcpls startup timeout expired.'
    }
    return [Math]::Min($remaining, $Maximum)
}

function Get-RemainingMillisecondsOrZero {
    param(
        [Parameter(Mandatory)][DateTimeOffset]$Deadline,
        [ValidateRange(0, 300000)][int]$Maximum = 30000
    )

    $remaining = [int][Math]::Floor(
        ($Deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds
    )
    return [Math]::Max(0, [Math]::Min($remaining, $Maximum))
}

function Get-RemainingProbeMilliseconds {
    param(
        [Parameter(Mandatory)][DateTimeOffset]$Deadline,
        [ValidateRange(50, 30000)][int]$Maximum = 30000
    )

    $remaining = Get-RemainingMilliseconds -Deadline $Deadline -Maximum $Maximum
    if ($remaining -lt 50) {
        throw 'mcpls startup timeout expired before a bounded health probe could start.'
    }
    return $remaining
}

function Get-WorktreeLockPath {
    param(
        [Parameter(Mandatory)][string]$AllStateRoot,
        [Parameter(Mandatory)][string]$WorktreeId
    )

    return Join-Path (Join-Path $AllStateRoot '.locks') "$WorktreeId.lock"
}

function Get-PruneLockPath {
    param([Parameter(Mandatory)][string]$AllStateRoot)

    return Join-Path (Join-Path $AllStateRoot '.locks') 'prune.lock'
}

function Get-PruneCursorPath {
    param([Parameter(Mandatory)][string]$AllStateRoot)

    return Join-Path (Join-Path $AllStateRoot '.locks') 'prune.cursor'
}

function Get-TemplateInfo {
    param([Parameter(Mandatory)]$Context)

    if (-not (Test-Path -LiteralPath $Context.TemplatePath -PathType Leaf)) {
        throw "Missing tracked mcpls template: $($Context.TemplatePath)"
    }

    $content = [System.IO.File]::ReadAllText($Context.TemplatePath)
    if ($content -notmatch '(?m)^# laneflow-mcpls-template-schema: 1\s*$') {
        throw 'Unsupported or missing mcpls template schema.'
    }
    if (($content.Split('__LANEFLOW_MCPLS_ENDPOINT__').Count - 1) -ne 1) {
        throw 'The mcpls endpoint placeholder must appear exactly once.'
    }
    if (($content.Split('__LANEFLOW_MCPLS_ENABLED__').Count - 1) -ne 1) {
        throw 'The mcpls enabled placeholder must appear exactly once.'
    }

    return [pscustomobject]@{
        Content = $content
        Hash = Get-Sha256Hex -Value $content
    }
}

function Test-ManagedGeneratedConfig {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $true
    }
    $content = [System.IO.File]::ReadAllText($Path)
    return $content -match '(?m)^# laneflow-mcpls-generated-schema: 1\s*$'
}

function Assert-GeneratedConfigOwnership {
    param([Parameter(Mandatory)]$Context)

    if (-not (Test-ManagedGeneratedConfig -Path $Context.GeneratedConfigPath)) {
        throw (
            "Refusing to overwrite unmanaged $($Context.GeneratedConfigPath). " +
            'Move or remove that file explicitly, then run setup again.'
        )
    }
}

function New-GeneratedConfigContent {
    param(
        [Parameter(Mandatory)]$TemplateInfo,
        [Parameter(Mandatory)][string]$Endpoint,
        [Parameter(Mandatory)][bool]$Enabled
    )

    if ($Endpoint -notmatch '^http://127\.0\.0\.1:[0-9]+/mcp$') {
        throw "Refusing non-loopback or malformed mcpls endpoint: $Endpoint"
    }

    $rendered = $TemplateInfo.Content.Replace(
        '__LANEFLOW_MCPLS_ENDPOINT__',
        $Endpoint
    ).Replace(
        '"__LANEFLOW_MCPLS_ENABLED__"',
        $Enabled.ToString().ToLowerInvariant()
    )
    $header = @(
        '# Generated by .codex/setup-mcpls-worktree.ps1; do not edit.'
        $script:GeneratedConfigMarker
        "# template-sha256: $($TemplateInfo.Hash)"
        ''
    ) -join "`n"
    return $header + $rendered.TrimStart("`r", "`n")
}

function Write-GeneratedConfig {
    param(
        [Parameter(Mandatory)]$Context,
        [Parameter(Mandatory)]$TemplateInfo,
        [Parameter(Mandatory)][string]$Endpoint,
        [Parameter(Mandatory)][bool]$Enabled
    )

    Assert-GeneratedConfigOwnership -Context $Context
    $content = New-GeneratedConfigContent -TemplateInfo $TemplateInfo `
        -Endpoint $Endpoint -Enabled $Enabled
    Write-AtomicUtf8File -Path $Context.GeneratedConfigPath -Content $content
}

function Read-ServiceState {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $null
    }
    try {
        $state = [System.IO.File]::ReadAllText($Path) | ConvertFrom-Json
        if ([int]$state.schema_version -ne $script:StateSchemaVersion) {
            throw "unsupported schema version $($state.schema_version)"
        }
        $requiredProperties = @(
            'worktree_id',
            'worktree_root',
            'status',
            'process_id',
            'process_started_at_utc',
            'executable_path',
            'mcpls_version',
            'mcpls_config_path',
            'mcpls_config_sha256',
            'port',
            'endpoint',
            'template_sha256'
        )
        foreach ($property in $requiredProperties) {
            $stateProperty = $state.PSObject.Properties[$property]
            if ($null -eq $stateProperty) {
                throw "missing required property $property"
            }
            if ($null -eq $stateProperty.Value) {
                throw "required property $property is null"
            }
        }

        foreach ($property in @(
            'worktree_id',
            'worktree_root',
            'status',
            'executable_path',
            'mcpls_version',
            'mcpls_config_path',
            'mcpls_config_sha256',
            'endpoint',
            'template_sha256'
        )) {
            if ($state.$property -isnot [string] -or
                [string]::IsNullOrWhiteSpace([string]$state.$property)) {
                throw "required property $property must be a non-empty string"
            }
        }
        if ($state.process_started_at_utc -isnot [DateTime] -and
            $state.process_started_at_utc -isnot [DateTimeOffset] -and
            $state.process_started_at_utc -isnot [string]) {
            throw 'process_started_at_utc must be an ISO-8601 timestamp'
        }
        try {
            [void][DateTimeOffset]::Parse(
                [string]$state.process_started_at_utc,
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::RoundtripKind
            )
        }
        catch {
            throw 'process_started_at_utc must be an ISO-8601 timestamp'
        }
        if ([string]$state.worktree_id -notmatch '^[0-9a-f]{64}$') {
            throw 'worktree_id must be a lowercase SHA-256 hex string'
        }
        if ([string]$state.mcpls_config_sha256 -notmatch '^[0-9a-f]{64}$' -or
            [string]$state.template_sha256 -notmatch '^[0-9a-f]{64}$') {
            throw 'state hashes must be lowercase SHA-256 hex strings'
        }
        foreach ($property in @('worktree_root', 'executable_path', 'mcpls_config_path')) {
            if (-not [System.IO.Path]::IsPathFullyQualified([string]$state.$property)) {
                throw "required property $property must be an absolute path"
            }
        }
        if ($state.process_id -isnot [int] -and $state.process_id -isnot [long]) {
            throw 'process_id must be an integer'
        }
        $terminalWithoutProcess = (
            [string]$state.status -eq 'stopped' -and [long]$state.process_id -eq 0
        )
        if (-not $terminalWithoutProcess -and (
            [long]$state.process_id -le 0 -or
            [long]$state.process_id -gt [int]::MaxValue
        )) {
            throw 'process_id must be positive, or zero only when status is stopped'
        }
        if ($state.port -isnot [int] -and $state.port -isnot [long]) {
            throw 'port must be an integer'
        }
        if ([long]$state.port -lt 1 -or [long]$state.port -gt 65535) {
            throw 'port must be within 1..65535'
        }
        $expectedEndpoint = "http://127.0.0.1:$([int]$state.port)$($script:HttpPath)"
        if (-not [string]::Equals(
            [string]$state.endpoint,
            $expectedEndpoint,
            [System.StringComparison]::Ordinal
        )) {
            throw 'endpoint must match the recorded loopback port and MCP path'
        }
        return $state
    }
    catch {
        throw "Invalid mcpls service state at ${Path}: $($_.Exception.Message)"
    }
}

function Write-ServiceState {
    param(
        [Parameter(Mandatory)]$Context,
        [Parameter(Mandatory)]$State
    )

    $State.updated_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
    $content = ($State | ConvertTo-Json -Depth 8) + "`n"
    Write-AtomicUtf8File -Path $Context.StatePath -Content $content
}

function Invoke-ApplicationCapture {
    param(
        [Parameter(Mandatory)][string]$Executable,
        [Parameter(Mandatory)][string[]]$Arguments,
        [Parameter(Mandatory)][DateTimeOffset]$Deadline
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        $startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $started = $false
    try {
        if (-not $process.Start()) {
            throw "Failed to start $Executable."
        }
        $started = $true
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $remaining = Get-RemainingMilliseconds -Deadline $Deadline
        if (-not $process.WaitForExit($remaining)) {
            $process.Kill($true)
            $cleanupWait = Get-RemainingMillisecondsOrZero `
                -Deadline $Deadline -Maximum 5000
            $null = $process.WaitForExit($cleanupWait)
            throw "Timed out running $Executable $($Arguments -join ' ')"
        }
        $stdoutText = $stdout.GetAwaiter().GetResult()
        $stderrText = $stderr.GetAwaiter().GetResult()
        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            StdOut = $stdoutText
            StdErr = $stderrText
            Output = $stdoutText + $stderrText
        }
    }
    finally {
        if ($started -and -not $process.HasExited) {
            try {
                $process.Kill($true)
                $cleanupWait = Get-RemainingMillisecondsOrZero `
                    -Deadline $Deadline -Maximum 5000
                $null = $process.WaitForExit($cleanupWait)
            }
            catch {
                # The primary validation failure remains authoritative.
            }
        }
        $process.Dispose()
    }
}

function Test-McplsExecutable {
    param(
        [string]$ExecutableOverride,
        [DateTimeOffset]$Deadline = [DateTimeOffset]::UtcNow.AddSeconds(30)
    )

    try {
        $executable = if ([string]::IsNullOrWhiteSpace($ExecutableOverride)) {
            Get-ApplicationPath -Name 'mcpls'
        }
        else {
            Get-NormalizedPath -Path $ExecutableOverride
        }
        if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
            throw "mcpls executable does not exist: $executable"
        }

        $versionResult = Invoke-ApplicationCapture -Executable $executable `
            -Arguments @('--version') -Deadline $Deadline
        $helpResult = Invoke-ApplicationCapture -Executable $executable `
            -Arguments @('--help') -Deadline $Deadline
        $versionOutput = ([string]$versionResult.Output).Trim()
        $helpOutput = [string]$helpResult.Output
        if ($versionResult.ExitCode -ne 0) {
            throw "mcpls --version failed with exit code $($versionResult.ExitCode)"
        }
        if ($helpResult.ExitCode -ne 0) {
            throw "mcpls --help failed with exit code $($helpResult.ExitCode)"
        }
        if ($versionOutput -notmatch '(?m)^mcpls 0\.3\.9(?:\s|$)') {
            throw "Expected mcpls $($script:McplsVersion), got: $versionOutput"
        }
        if ($helpOutput -notmatch '(?m)^\s*--listen\b' -or
            $helpOutput -notmatch '(?m)^\s*--http-path\b') {
            throw 'mcpls was not built with the transport-http feature.'
        }

        return [pscustomobject]@{
            Valid = $true
            Path = Get-NormalizedPath -Path $executable
            Version = $versionOutput
            Reason = $null
        }
    }
    catch {
        return [pscustomobject]@{
            Valid = $false
            Path = $null
            Version = $null
            Reason = $_.Exception.Message
        }
    }
}

function Get-ProcessSnapshotFromHandle {
    param(
        [Parameter(Mandatory)][System.Diagnostics.Process]$Process,
        [DateTimeOffset]$Deadline = [DateTimeOffset]::MinValue
    )

    $processId = [int]$Process.Id
    try {
        $operationTimeoutSeconds = 3
        if ($Deadline -ne [DateTimeOffset]::MinValue) {
            $remaining = Get-RemainingMilliseconds -Deadline $Deadline -Maximum 3000
            $operationTimeoutSeconds = [Math]::Max(
                1,
                [int][Math]::Ceiling($remaining / 1000.0)
            )
        }
        $cim = Get-CimInstance -ClassName Win32_Process `
            -Filter "ProcessId = $ProcessId" `
            -OperationTimeoutSec $operationTimeoutSeconds -ErrorAction Stop
        if ($null -eq $cim) {
            throw 'Win32_Process returned no row for a PID that was live before inspection'
        }

        return [pscustomobject]@{
            ProcessId = $ProcessId
            ParentProcessId = [int]$cim.ParentProcessId
            Name = [string]$cim.Name
            ExecutablePath = Get-NormalizedPath -Path ([string]$Process.Path)
            CommandLine = [string]$cim.CommandLine
            StartedAtUtc = $Process.StartTime.ToUniversalTime()
        }
    }
    catch {
        throw "Unable to inspect live PID ${ProcessId}: $($_.Exception.Message)"
    }
}

function Get-ProcessSnapshot {
    param(
        [Parameter(Mandatory)][int]$ProcessId,
        [DateTimeOffset]$Deadline = [DateTimeOffset]::MinValue
    )

    try {
        $process = Get-Process -Id $ProcessId -ErrorAction Stop
    }
    catch {
        if ($_.FullyQualifiedErrorId -like 'NoProcessFoundForGivenId*') {
            return $null
        }
        throw "Unable to determine whether PID ${ProcessId} exists: $($_.Exception.Message)"
    }
    try {
        return Get-ProcessSnapshotFromHandle -Process $process -Deadline $Deadline
    }
    finally {
        $process.Dispose()
    }
}

function Test-ServiceProcessSnapshotIdentity {
    param(
        [Parameter(Mandatory)]$State,
        [Parameter(Mandatory)][string]$ExpectedRoot,
        [Parameter(Mandatory)]$Snapshot
    )

    try {
        if ([int]$State.schema_version -ne $script:StateSchemaVersion) {
            throw 'state schema mismatch'
        }
        $expectedWorktreeId = Get-WorktreeId -CanonicalRoot $ExpectedRoot
        if (-not [string]::Equals(
            [string]$State.worktree_id,
            $expectedWorktreeId,
            [System.StringComparison]::Ordinal
        )) {
            throw 'worktree ID mismatch'
        }
        if (-not (Test-PathEqual -Left ([string]$State.worktree_root) -Right $ExpectedRoot)) {
            throw 'worktree root mismatch'
        }
        if ([int]$State.process_id -le 0) {
            throw 'state has no running PID'
        }
        if ([int]$Snapshot.ProcessId -ne [int]$State.process_id) {
            throw 'process ID mismatch'
        }
        if (-not (Test-PathEqual -Left $Snapshot.ExecutablePath -Right ([string]$State.executable_path))) {
            throw 'executable path mismatch'
        }

        $storedStartedAt = $State.process_started_at_utc
        $expectedStartedAtUtc = if ($storedStartedAt -is [DateTime]) {
            ([DateTime]$storedStartedAt).ToUniversalTime()
        }
        elseif ($storedStartedAt -is [DateTimeOffset]) {
            ([DateTimeOffset]$storedStartedAt).UtcDateTime
        }
        else {
            [DateTimeOffset]::ParseExact(
                [string]$storedStartedAt,
                'O',
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::RoundtripKind
            ).UtcDateTime
        }
        $actualStartedAt = $Snapshot.StartedAtUtc.ToUniversalTime()
        $startDeltaSeconds = [Math]::Abs(
            ($actualStartedAt.Ticks - $expectedStartedAtUtc.Ticks) /
            [double][TimeSpan]::TicksPerSecond
        )
        if ($startDeltaSeconds -gt 2) {
            throw 'process start time mismatch'
        }

        $configPath = Get-NormalizedPath -Path ([string]$State.mcpls_config_path)
        $listen = "127.0.0.1:$([int]$State.port)"
        if ($Snapshot.CommandLine.IndexOf($configPath, [StringComparison]::Ordinal) -lt 0) {
            throw 'command line does not contain the worktree mcpls.toml path'
        }
        if ($Snapshot.CommandLine.IndexOf($listen, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
            throw 'command line does not contain the recorded loopback endpoint'
        }
        if ($Snapshot.CommandLine.IndexOf('--http-path', [StringComparison]::OrdinalIgnoreCase) -lt 0 -or
            $Snapshot.CommandLine.IndexOf($script:HttpPath, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
            throw 'command line does not contain the HTTP path'
        }

        return [pscustomobject]@{
            Matched = $true
            Reason = $null
            Snapshot = $Snapshot
        }
    }
    catch {
        return [pscustomobject]@{
            Matched = $false
            Reason = $_.Exception.Message
            Snapshot = $null
        }
    }
}

function Test-ServiceProcessIdentity {
    param(
        [Parameter(Mandatory)]$State,
        [Parameter(Mandatory)][string]$ExpectedRoot,
        [DateTimeOffset]$Deadline = [DateTimeOffset]::MinValue
    )

    try {
        $snapshot = Get-ProcessSnapshot -ProcessId ([int]$State.process_id) `
            -Deadline $Deadline
        if ($null -eq $snapshot) {
            throw 'recorded process is not running'
        }
        return Test-ServiceProcessSnapshotIdentity -State $State `
            -ExpectedRoot $ExpectedRoot -Snapshot $snapshot
    }
    catch {
        return [pscustomobject]@{
            Matched = $false
            Reason = $_.Exception.Message
            Snapshot = $null
        }
    }
}

function Get-ServiceReuseInputs {
    param(
        [Parameter(Mandatory)]$State,
        [Parameter(Mandatory)][string]$ExecutablePath,
        [Parameter(Mandatory)][string]$McplsConfigHash
    )

    $sameTool = Test-PathEqual -Left ([string]$State.executable_path) `
        -Right $ExecutablePath
    $sameConfig = [string]::Equals(
        [string]$State.mcpls_config_sha256,
        $McplsConfigHash,
        [System.StringComparison]::Ordinal
    )
    $sameVersion = [string]$State.mcpls_version -eq $script:McplsVersion
    return [pscustomobject]@{
        Reusable = $sameTool -and $sameConfig -and $sameVersion
        SameTool = $sameTool
        SameConfig = $sameConfig
        SameVersion = $sameVersion
    }
}

function Remove-McpProbeSession {
    param(
        [Parameter(Mandatory)][System.Net.Http.HttpClient]$Client,
        [Parameter(Mandatory)][string]$Endpoint,
        [Parameter(Mandatory)][string]$SessionId,
        [Parameter(Mandatory)][string]$ProtocolVersion,
        [Parameter(Mandatory)][System.Threading.CancellationToken]$CancellationToken
    )

    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::Delete,
            $Endpoint
        )
        $request.Headers.TryAddWithoutValidation('Mcp-Session-Id', $SessionId) | Out-Null
        $request.Headers.TryAddWithoutValidation(
            'MCP-Protocol-Version',
            $ProtocolVersion
        ) | Out-Null
        $response = $Client.SendAsync(
            $request,
            $CancellationToken
        ).GetAwaiter().GetResult()
        if (-not $response.IsSuccessStatusCode) {
            $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            return [pscustomobject]@{
                Succeeded = $false
                Reason = "HTTP session DELETE returned $([int]$response.StatusCode): $body"
            }
        }
        return [pscustomobject]@{ Succeeded = $true; Reason = $null }
    }
    catch {
        return [pscustomobject]@{ Succeeded = $false; Reason = $_.Exception.Message }
    }
    finally {
        if ($null -ne $response) {
            $response.Dispose()
        }
        if ($null -ne $request) {
            $request.Dispose()
        }
    }
}

function Test-McpInitialize {
    param(
        [Parameter(Mandatory)][string]$Endpoint,
        [ValidateRange(50, 30000)][int]$TimeoutMilliseconds = 3000
    )

    $client = [System.Net.Http.HttpClient]::new()
    $client.Timeout = [System.Threading.Timeout]::InfiniteTimeSpan
    $timeout = [System.Threading.CancellationTokenSource]::new($TimeoutMilliseconds)
    $sessionId = $null
    $protocolVersion = $null
    try {
        $payload = [ordered]@{
            jsonrpc = '2.0'
            id = 1
            method = 'initialize'
            params = [ordered]@{
                protocolVersion = '2025-03-26'
                capabilities = @{}
                clientInfo = [ordered]@{
                    name = 'laneflow-mcpls-lifecycle'
                    version = '1.0'
                }
            }
        } | ConvertTo-Json -Depth 8 -Compress

        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::Post,
            $Endpoint
        )
        $request.Headers.TryAddWithoutValidation(
            'Accept',
            'application/json, text/event-stream'
        ) | Out-Null
        $request.Content = [System.Net.Http.StringContent]::new(
            $payload,
            [System.Text.UTF8Encoding]::new($false),
            'application/json'
        )

        $response = $client.SendAsync(
            $request,
            $timeout.Token
        ).GetAwaiter().GetResult()
        try {
            $responseBody = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            if (-not $response.IsSuccessStatusCode) {
                throw "HTTP initialize returned $([int]$response.StatusCode): $responseBody"
            }
            if ($response.Headers.Contains('Mcp-Session-Id')) {
                $sessionId = ($response.Headers.GetValues('Mcp-Session-Id') | Select-Object -First 1)
            }

            $jsonText = $responseBody.Trim()
            if ($jsonText -notmatch '^\{') {
                $dataLine = $responseBody -split "`r?`n" |
                    Where-Object { $_ -match '^data:\s*\{' } |
                    Select-Object -First 1
                if ($null -eq $dataLine) {
                    throw 'HTTP initialize did not return JSON or an SSE JSON data event.'
                }
                $jsonText = $dataLine -replace '^data:\s*', ''
            }

            $message = $jsonText | ConvertFrom-Json
            $errorProperty = $message.PSObject.Properties['error']
            $resultProperty = $message.PSObject.Properties['result']
            if (($null -ne $errorProperty -and $null -ne $errorProperty.Value) -or
                $null -eq $resultProperty -or $null -eq $resultProperty.Value) {
                throw "MCP initialize returned an error: $jsonText"
            }
            $result = $resultProperty.Value
            $protocolVersion = [string]$result.protocolVersion
            $serverInfo = $result.PSObject.Properties['serverInfo']
            $serverName = if ($null -ne $serverInfo -and $null -ne $serverInfo.Value) {
                [string]$serverInfo.Value.name
            }
            else {
                $null
            }

            if (-not [string]::IsNullOrWhiteSpace($sessionId)) {
                if ([string]::IsNullOrWhiteSpace($protocolVersion)) {
                    throw 'MCP initialize returned a session ID without a protocol version.'
                }
                $cleanup = Remove-McpProbeSession -Client $client -Endpoint $Endpoint `
                    -SessionId $sessionId -ProtocolVersion $protocolVersion `
                    -CancellationToken $timeout.Token
                if (-not $cleanup.Succeeded) {
                    throw "MCP probe session cleanup failed: $($cleanup.Reason)"
                }
                $sessionId = $null
            }

            return [pscustomobject]@{
                Healthy = $true
                Reason = $null
                ServerName = $serverName
                ProtocolVersion = $protocolVersion
            }
        }
        finally {
            $response.Dispose()
            $request.Dispose()
        }
    }
    catch {
        return [pscustomobject]@{
            Healthy = $false
            Reason = $_.Exception.Message
            ServerName = $null
            ProtocolVersion = $null
        }
    }
    finally {
        if (-not [string]::IsNullOrWhiteSpace($sessionId) -and
            -not [string]::IsNullOrWhiteSpace($protocolVersion)) {
            $null = Remove-McpProbeSession -Client $client -Endpoint $Endpoint `
                -SessionId $sessionId -ProtocolVersion $protocolVersion `
                -CancellationToken $timeout.Token
        }
        $timeout.Dispose()
        $client.Dispose()
    }
}

function Test-LoopbackPortListening {
    param(
        [Parameter(Mandatory)][ValidateRange(1, 65535)][int]$Port,
        [ValidateRange(50, 5000)][int]$TimeoutMilliseconds = 250
    )

    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $task = $client.ConnectAsync('127.0.0.1', $Port)
        return $task.Wait($TimeoutMilliseconds) -and $client.Connected
    }
    catch {
        return $false
    }
    finally {
        $client.Dispose()
    }
}

function Test-LoopbackPortAvailable {
    param([Parameter(Mandatory)][ValidateRange(1, 65535)][int]$Port)

    $listener = [System.Net.Sockets.TcpListener]::new(
        [System.Net.IPAddress]::Loopback,
        $Port
    )
    try {
        $listener.Start()
        return $true
    }
    catch {
        return $false
    }
    finally {
        $listener.Stop()
    }
}

function Get-PreferredPort {
    param([Parameter(Mandatory)][string]$WorktreeId)

    $seed = [uint32]::Parse(
        $WorktreeId.Substring(0, 8),
        [System.Globalization.NumberStyles]::HexNumber,
        [System.Globalization.CultureInfo]::InvariantCulture
    )
    $size = $script:PortMaximum - $script:PortMinimum + 1
    return $script:PortMinimum + [int]($seed % $size)
}

function Get-PortCandidate {
    param(
        [Parameter(Mandatory)][string]$WorktreeId,
        [ValidateRange(0, 7999)][int]$Offset,
        [int]$PreferredPort = 0
    )

    $size = $script:PortMaximum - $script:PortMinimum + 1
    $base = if ($PreferredPort -ge $script:PortMinimum -and
        $PreferredPort -le $script:PortMaximum) {
        $PreferredPort
    }
    else {
        Get-PreferredPort -WorktreeId $WorktreeId
    }
    return $script:PortMinimum + (($base - $script:PortMinimum + $Offset) % $size)
}

function Start-McplsProcess {
    param(
        [Parameter(Mandatory)][string]$Executable,
        [Parameter(Mandatory)]$Context,
        [Parameter(Mandatory)][int]$Port
    )

    [System.IO.Directory]::CreateDirectory($Context.StateDirectory) | Out-Null
    $standardOutputPath = Join-Path $Context.StateDirectory 'mcpls.stdout.log'
    $standardErrorPath = Join-Path $Context.StateDirectory 'mcpls.stderr.log'
    $quotedConfigPath = '"' + $Context.McplsConfigPath + '"'
    $arguments = @(
        '--config', $quotedConfigPath,
        '--listen', "127.0.0.1:$Port",
        '--http-path', $script:HttpPath
    )

    $process = Start-Process -FilePath $Executable -ArgumentList $arguments `
        -WorkingDirectory $Context.Root -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput $standardOutputPath `
        -RedirectStandardError $standardErrorPath
    if ($null -eq $process) {
        throw 'Failed to create the mcpls process.'
    }
    return $process
}

function Stop-OwnedProcessTree {
    param(
        [Parameter(Mandatory)][int]$ProcessId,
        [ValidateRange(0, 10000)][int]$TimeoutMilliseconds = 10000,
        [DateTimeOffset]$Deadline = [DateTimeOffset]::MinValue
    )

    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        return
    }
    try {
        $process.Kill($true)
        $exitWait = if ($Deadline -eq [DateTimeOffset]::MinValue) {
            $TimeoutMilliseconds
        }
        else {
            Get-RemainingMillisecondsOrZero -Deadline $Deadline `
                -Maximum $TimeoutMilliseconds
        }
        if (-not $process.WaitForExit($exitWait)) {
            throw (
                "Process tree rooted at PID $ProcessId did not exit within " +
                "$exitWait milliseconds."
            )
        }
    }
    finally {
        $process.Dispose()
    }
}

function Stop-VerifiedServiceProcessTree {
    param(
        [Parameter(Mandatory)]$State,
        [Parameter(Mandatory)][string]$ExpectedRoot,
        [ValidateRange(0, 10000)][int]$TimeoutMilliseconds = 10000,
        [DateTimeOffset]$Deadline = [DateTimeOffset]::MinValue
    )

    $processId = [int]$State.process_id
    try {
        $process = Get-Process -Id $processId -ErrorAction Stop
    }
    catch {
        if ($_.FullyQualifiedErrorId -like 'NoProcessFoundForGivenId*') {
            return
        }
        throw "Unable to determine whether PID ${processId} exists before stop: $($_.Exception.Message)"
    }

    try {
        $snapshot = Get-ProcessSnapshotFromHandle -Process $process -Deadline $Deadline
        $identity = Test-ServiceProcessSnapshotIdentity -State $State `
            -ExpectedRoot $ExpectedRoot -Snapshot $snapshot
        if (-not $identity.Matched) {
            throw "Refusing to stop a revalidated identity-mismatched process: $($identity.Reason)"
        }

        $process.Kill($true)
        $exitWait = if ($Deadline -eq [DateTimeOffset]::MinValue) {
            $TimeoutMilliseconds
        }
        else {
            Get-RemainingMillisecondsOrZero -Deadline $Deadline `
                -Maximum $TimeoutMilliseconds
        }
        if (-not $process.WaitForExit($exitWait)) {
            throw (
                "Verified process tree rooted at PID $processId did not exit within " +
                "$exitWait milliseconds."
            )
        }
    }
    finally {
        $process.Dispose()
    }
}

function Get-DescendantProcessesFromSnapshot {
    param(
        [Parameter(Mandatory)][object[]]$Processes,
        [Parameter(Mandatory)][int]$RootProcessId
    )

    $result = [System.Collections.Generic.List[object]]::new()
    $frontier = [System.Collections.Generic.Queue[int]]::new()
    $visited = [System.Collections.Generic.HashSet[int]]::new()
    $visited.Add($RootProcessId) | Out-Null
    $frontier.Enqueue($RootProcessId)
    while ($frontier.Count -gt 0) {
        $parent = $frontier.Dequeue()
        foreach ($child in $Processes | Where-Object { [int]$_.ParentProcessId -eq $parent }) {
            $childProcessId = [int]$child.ProcessId
            if ($visited.Add($childProcessId)) {
                $result.Add($child)
                $frontier.Enqueue($childProcessId)
            }
        }
    }
    return @($result)
}

function Get-DescendantProcesses {
    param([Parameter(Mandatory)][int]$RootProcessId)

    try {
        $all = @(Get-CimInstance -ClassName Win32_Process -ErrorAction Stop)
    }
    catch {
        return @()
    }
    return @(Get-DescendantProcessesFromSnapshot -Processes $all `
        -RootProcessId $RootProcessId)
}

function New-ServiceState {
    param(
        [Parameter(Mandatory)]$Context,
        [Parameter(Mandatory)]$Tool,
        [Parameter(Mandatory)][int]$Port,
        [Parameter(Mandatory)]$Process,
        [Parameter(Mandatory)]$TemplateInfo,
        [Parameter(Mandatory)][string]$McplsConfigHash,
        [Parameter(Mandatory)][string]$Status
    )

    $endpoint = "http://127.0.0.1:$Port$($script:HttpPath)"
    return [pscustomobject][ordered]@{
        schema_version = $script:StateSchemaVersion
        worktree_id = $Context.WorktreeId
        worktree_root = $Context.Root
        status = $Status
        process_id = [int]$Process.Id
        process_started_at_utc = $Process.StartTime.ToUniversalTime().ToString('O')
        executable_path = $Tool.Path
        mcpls_version = $script:McplsVersion
        mcpls_config_path = $Context.McplsConfigPath
        mcpls_config_sha256 = $McplsConfigHash
        command_summary = @(
            '--config', $Context.McplsConfigPath,
            '--listen', "127.0.0.1:$Port",
            '--http-path', $script:HttpPath
        )
        port = $Port
        endpoint = $endpoint
        template_sha256 = $TemplateInfo.Hash
        service_started_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
        updated_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
        last_error = $null
    }
}

function Wait-McplsReady {
    param(
        [Parameter(Mandatory)]$Process,
        [Parameter(Mandatory)][string]$Endpoint,
        [Parameter(Mandatory)][DateTimeOffset]$Deadline
    )

    $lastReason = 'not yet checked'
    while ([DateTimeOffset]::UtcNow -lt $Deadline) {
        if ($Process.HasExited) {
            return [pscustomobject]@{
                Healthy = $false
                Reason = "mcpls exited with code $($Process.ExitCode)"
            }
        }

        $remainingMilliseconds = [int][Math]::Floor(
            ($Deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds
        )
        if ($remainingMilliseconds -lt 50) {
            break
        }
        $health = Test-McpInitialize -Endpoint $Endpoint `
            -TimeoutMilliseconds ([Math]::Min(3000, $remainingMilliseconds))
        if ($health.Healthy) {
            return $health
        }
        $lastReason = $health.Reason
        Start-Sleep -Milliseconds 250
    }

    return [pscustomobject]@{
        Healthy = $false
        Reason = "startup timeout: $lastReason"
    }
}

function Start-NewMcplsService {
    param(
        [Parameter(Mandatory)]$Context,
        [Parameter(Mandatory)]$Tool,
        [Parameter(Mandatory)]$TemplateInfo,
        [Parameter(Mandatory)][string]$McplsConfigHash,
        [Parameter(Mandatory)][DateTimeOffset]$Deadline,
        [int]$PreferredPort = 0
    )

    if (-not (Test-Path -LiteralPath $Context.McplsConfigPath -PathType Leaf)) {
        throw "Missing worktree mcpls.toml: $($Context.McplsConfigPath)"
    }

    $lastFailure = $null
    $portRangeSize = $script:PortMaximum - $script:PortMinimum + 1
    for ($offset = 0; $offset -lt $portRangeSize; $offset++) {
        $null = Get-RemainingMilliseconds -Deadline $Deadline
        $allocationLease = $null
        $process = $null
        $state = $null
        $committed = $false
        $port = Get-PortCandidate -WorktreeId $Context.WorktreeId `
            -Offset $offset -PreferredPort $PreferredPort
        try {
            $allocationLease = Enter-FileLock -Path $Context.PortLockPath `
                -Deadline $Deadline
            if (-not (Test-LoopbackPortAvailable -Port $port)) {
                $lastFailure = "loopback port $port is already occupied"
                continue
            }

            $null = Get-RemainingMilliseconds -Deadline $Deadline
            $process = Start-McplsProcess -Executable $Tool.Path `
                -Context $Context -Port $port
            $listenDeadline = [DateTimeOffset]::UtcNow.AddSeconds(5)
            if ($listenDeadline -gt $Deadline) {
                $listenDeadline = $Deadline
            }
            $bound = $false
            while ([DateTimeOffset]::UtcNow -lt $listenDeadline -and -not $process.HasExited) {
                $remainingMilliseconds = [int][Math]::Floor(
                    ($listenDeadline - [DateTimeOffset]::UtcNow).TotalMilliseconds
                )
                if ($remainingMilliseconds -lt 50) {
                    break
                }
                if (Test-LoopbackPortListening -Port $port `
                    -TimeoutMilliseconds ([Math]::Min(250, $remainingMilliseconds))) {
                    $bound = $true
                    break
                }
                Start-Sleep -Milliseconds 100
            }
            if ($process.HasExited -or -not $bound) {
                $lastFailure = "mcpls did not bind 127.0.0.1:$port"
                try {
                    if (-not $process.HasExited) {
                        Stop-OwnedProcessTree -ProcessId ([int]$process.Id) `
                            -TimeoutMilliseconds 5000 -Deadline $Deadline
                    }
                }
                finally {
                    $process.Dispose()
                    $process = $null
                }
                continue
            }
        }
        finally {
            Exit-FileLock -Lease $allocationLease
        }

        try {
            $state = New-ServiceState -Context $Context -Tool $Tool -Port $port `
                -Process $process -TemplateInfo $TemplateInfo `
                -McplsConfigHash $McplsConfigHash -Status 'starting'
            Write-ServiceState -Context $Context -State $state
            Write-LifecycleLog -Context $Context `
                -Message "started pid=$($process.Id) endpoint=$($state.endpoint)"

            $health = Wait-McplsReady -Process $process -Endpoint $state.endpoint `
                -Deadline $Deadline
            if (-not $health.Healthy) {
                throw "mcpls failed HTTP initialize: $($health.Reason)"
            }

            $state.status = 'ready'
            $state.last_error = $null
            Write-ServiceState -Context $Context -State $state
            Write-GeneratedConfig -Context $Context -TemplateInfo $TemplateInfo `
                -Endpoint ([string]$state.endpoint) -Enabled $true
            Write-LifecycleLog -Context $Context `
                -Message "ready pid=$($state.process_id) endpoint=$($state.endpoint)"
            $committed = $true
            return $state
        }
        catch {
            $failureReason = $_.Exception.Message
            if ($null -ne $process -and -not $process.HasExited) {
                try {
                    Stop-OwnedProcessTree -ProcessId ([int]$process.Id) `
                        -Deadline $Deadline
                }
                catch {
                    $failureReason = "$failureReason Cleanup failed: $($_.Exception.Message)"
                }
            }
            if ($null -ne $state) {
                $state.status = 'failed'
                $state.last_error = $failureReason
                try {
                    Write-ServiceState -Context $Context -State $state
                }
                catch {
                    # The original persistence failure remains authoritative.
                }
                try {
                    Write-DisabledGeneratedConfig -Context $Context `
                        -Endpoint ([string]$state.endpoint)
                }
                catch {
                    # Ensure/Start will report any remaining config failure to the caller.
                }
                try {
                    Write-LifecycleLog -Context $Context `
                        -Message "startup-failed pid=$($process.Id) reason=$failureReason"
                }
                catch {
                    # Cleanup must not be bypassed by bookkeeping failure.
                }
            }
            throw "mcpls startup transaction failed: $failureReason"
        }
        finally {
            if ($null -ne $process) {
                if (-not $committed -and -not $process.HasExited) {
                    try {
                        Stop-OwnedProcessTree -ProcessId ([int]$process.Id) `
                            -Deadline $Deadline
                    }
                    catch {
                        # The primary failure already records the cleanup attempt.
                    }
                }
                $process.Dispose()
            }
        }
    }

    throw "No mcpls port could be started in the bounded full-range probe. Last failure: $lastFailure"
}

function Invoke-StartOrReuse {
    param(
        [Parameter(Mandatory)]$Context,
        [string]$ExecutableOverride,
        [Parameter(Mandatory)][int]$TimeoutSeconds,
        [DateTimeOffset]$OperationDeadline = [DateTimeOffset]::MinValue
    )

    $deadline = if ($OperationDeadline -eq [DateTimeOffset]::MinValue) {
        [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    }
    else {
        $OperationDeadline
    }
    $null = Get-RemainingMilliseconds -Deadline $deadline
    Assert-GeneratedConfigOwnership -Context $Context
    $template = Get-TemplateInfo -Context $Context
    $tool = Test-McplsExecutable -ExecutableOverride $ExecutableOverride `
        -Deadline $deadline
    if (-not $tool.Valid) {
        throw $tool.Reason
    }
    if (-not (Test-Path -LiteralPath $Context.McplsConfigPath -PathType Leaf)) {
        throw "Missing worktree mcpls.toml: $($Context.McplsConfigPath)"
    }
    $mcplsConfigHash = Get-FileSha256Hex -Path $Context.McplsConfigPath
    $lease = $null
    try {
        $lease = Enter-FileLock -Path $Context.WorktreeLockPath -Deadline $deadline
        [System.IO.Directory]::CreateDirectory($Context.StateDirectory) | Out-Null

        $existingState = Read-ServiceState -Path $Context.StatePath
        $preferredPort = 0
        if ($null -ne $existingState) {
            try {
                $preferredPort = [int]$existingState.port
            }
            catch {
                $preferredPort = 0
            }

            $identity = Test-ServiceProcessIdentity -State $existingState `
                -ExpectedRoot $Context.Root -Deadline $deadline
            if ($identity.Matched) {
                $healthTimeout = Get-RemainingProbeMilliseconds -Deadline $deadline `
                    -Maximum 3000
                $health = Test-McpInitialize -Endpoint ([string]$existingState.endpoint) `
                    -TimeoutMilliseconds $healthTimeout
                $reuseInputs = Get-ServiceReuseInputs -State $existingState `
                    -ExecutablePath $tool.Path -McplsConfigHash $mcplsConfigHash
                if ($health.Healthy -and $reuseInputs.Reusable) {
                    $existingState.status = 'ready'
                    $existingState.template_sha256 = $template.Hash
                    $existingState.last_error = $null
                    Write-ServiceState -Context $Context -State $existingState
                    Write-GeneratedConfig -Context $Context -TemplateInfo $template `
                        -Endpoint ([string]$existingState.endpoint) -Enabled $true
                    Write-LifecycleLog -Context $Context `
                        -Message "reused pid=$($existingState.process_id) endpoint=$($existingState.endpoint)"
                    return [pscustomobject]@{
                        action = 'reused'
                        worktree_id = $Context.WorktreeId
                        worktree_root = $Context.Root
                        process_id = [int]$existingState.process_id
                        endpoint = [string]$existingState.endpoint
                        config_enabled = $true
                    }
                }

                Write-LifecycleLog -Context $Context `
                    -Message "replacing-owned pid=$($existingState.process_id) healthy=$($health.Healthy) same_tool=$($reuseInputs.SameTool) same_config=$($reuseInputs.SameConfig) same_version=$($reuseInputs.SameVersion)"
                Stop-VerifiedServiceProcessTree -State $existingState `
                    -ExpectedRoot $Context.Root -Deadline $deadline
            }
            elseif ([int]$existingState.process_id -gt 0) {
                $recordedProcess = Get-ProcessSnapshot `
                    -ProcessId ([int]$existingState.process_id) -Deadline $deadline
                if ($null -ne $recordedProcess) {
                    Write-LifecycleLog -Context $Context `
                        -Message "refused-live-identity-mismatch pid=$($existingState.process_id) reason=$($identity.Reason)"
                    throw (
                        "Recorded PID $($existingState.process_id) is still live but its identity " +
                        "does not match: $($identity.Reason). Refusing to overwrite the state or " +
                        'start a duplicate service.'
                    )
                }
                Write-LifecycleLog -Context $Context `
                    -Message "recovering-dead-stale-state pid=$($existingState.process_id) reason=$($identity.Reason)"
            }
        }

        $state = Start-NewMcplsService -Context $Context -Tool $tool `
            -TemplateInfo $template -McplsConfigHash $mcplsConfigHash `
            -Deadline $deadline `
            -PreferredPort $preferredPort
        return [pscustomobject]@{
            action = 'started'
            worktree_id = $Context.WorktreeId
            worktree_root = $Context.Root
            process_id = [int]$state.process_id
            endpoint = [string]$state.endpoint
            config_enabled = $true
        }
    }
    finally {
        Exit-FileLock -Lease $lease
    }
}

function Write-DisabledGeneratedConfig {
    param(
        [Parameter(Mandatory)]$Context,
        [string]$Endpoint = 'http://127.0.0.1:1/mcp'
    )

    $template = Get-TemplateInfo -Context $Context
    Write-GeneratedConfig -Context $Context -TemplateInfo $template `
        -Endpoint $Endpoint -Enabled $false
}

function Try-DisableGeneratedConfigWithoutWorktreeContext {
    param([string]$RootHint)

    $candidate = if ([string]::IsNullOrWhiteSpace($RootHint)) {
        Split-Path -Parent $PSScriptRoot
    }
    else {
        $RootHint
    }
    try {
        $root = [System.IO.Path]::TrimEndingDirectorySeparator(
            [System.IO.Path]::GetFullPath($candidate)
        )
        $fallbackContext = [pscustomobject]@{
            TemplatePath = Join-Path $root '.codex\config.template.toml'
            GeneratedConfigPath = Join-Path $root '.codex\config.toml'
        }
        Write-DisabledGeneratedConfig -Context $fallbackContext
        return [pscustomobject]@{
            Succeeded = $true
            ConfigEnabled = $false
            Reason = $null
        }
    }
    catch {
        return [pscustomobject]@{
            Succeeded = $false
            ConfigEnabled = $null
            Reason = $_.Exception.Message
        }
    }
}

function Test-ValidRecordedWorktree {
    param(
        [Parameter(Mandatory)][string]$Root,
        [DateTimeOffset]$Deadline = [DateTimeOffset]::UtcNow.AddSeconds(30)
    )

    $git = Get-ApplicationPath -Name 'git'
    $result = Invoke-ApplicationCapture -Executable $git `
        -Arguments @('-C', $Root, 'rev-parse', '--show-toplevel') `
        -Deadline $Deadline
    if ($result.ExitCode -ne 0) {
        return $false
    }
    $canonical = ([string]$result.StdOut).Trim()
    if ([string]::IsNullOrWhiteSpace($canonical)) {
        return $false
    }
    $canonical = [System.IO.Path]::TrimEndingDirectorySeparator(
        [System.IO.Path]::GetFullPath($canonical)
    )
    $recorded = [System.IO.Path]::TrimEndingDirectorySeparator(
        [System.IO.Path]::GetFullPath($Root)
    )
    return [string]::Equals(
        $canonical,
        $recorded,
        [System.StringComparison]::Ordinal
    )
}

function Test-StateDirectoryOwnership {
    param(
        [Parameter(Mandatory)]$State,
        [Parameter(Mandatory)][string]$DirectoryName
    )

    try {
        if (-not [string]::Equals(
            [string]$State.worktree_id,
            $DirectoryName,
            [System.StringComparison]::Ordinal
        )) {
            throw 'state worktree_id does not match the state directory'
        }
        $expectedId = Get-WorktreeId -CanonicalRoot ([string]$State.worktree_root)
        if (-not [string]::Equals(
            $expectedId,
            $DirectoryName,
            [System.StringComparison]::Ordinal
        )) {
            throw 'recorded worktree root does not hash to the state directory'
        }
        return [pscustomobject]@{ Matched = $true; Reason = $null }
    }
    catch {
        return [pscustomobject]@{ Matched = $false; Reason = $_.Exception.Message }
    }
}

function Get-RotatingStateDirectories {
    param(
        [Parameter(Mandatory)][string]$AllStateRoot,
        [string]$ExcludeWorktreeId,
        [Parameter(Mandatory)][ValidateRange(1, 256)][int]$Limit,
        [ValidateRange(50, 120000)][int]$LockTimeoutMilliseconds = 2000
    )

    $directories = @(Get-ChildItem -LiteralPath $AllStateRoot -Directory |
        Where-Object { $_.Name -match '^[0-9a-f]{64}$' } |
        Sort-Object -Property Name)
    if ($directories.Count -eq 0) {
        return @()
    }

    $lease = $null
    try {
        $deadline = [DateTimeOffset]::UtcNow.AddMilliseconds($LockTimeoutMilliseconds)
        $lease = Enter-FileLock -Path (Get-PruneLockPath -AllStateRoot $AllStateRoot) `
            -Deadline $deadline
        $cursorPath = Get-PruneCursorPath -AllStateRoot $AllStateRoot
        $cursor = if (Test-Path -LiteralPath $cursorPath -PathType Leaf) {
            ([System.IO.File]::ReadAllText($cursorPath)).Trim()
        }
        else {
            $null
        }
        $startIndex = 0
        if (-not [string]::IsNullOrWhiteSpace($cursor)) {
            for ($index = 0; $index -lt $directories.Count; $index++) {
                if ([string]::Equals(
                    $directories[$index].Name,
                    $cursor,
                    [System.StringComparison]::Ordinal
                )) {
                    $startIndex = ($index + 1) % $directories.Count
                    break
                }
            }
        }

        $selected = [System.Collections.Generic.List[object]]::new()
        for ($offset = 0; $offset -lt $directories.Count -and
            $selected.Count -lt $Limit; $offset++) {
            $directory = $directories[($startIndex + $offset) % $directories.Count]
            if (-not [string]::Equals(
                $directory.Name,
                $ExcludeWorktreeId,
                [System.StringComparison]::Ordinal
            )) {
                $selected.Add($directory)
            }
        }
        if ($selected.Count -gt 0) {
            Write-AtomicUtf8File -Path $cursorPath `
                -Content ([string]$selected[$selected.Count - 1].Name)
        }
        return @($selected)
    }
    finally {
        Exit-FileLock -Lease $lease
    }
}

function Remove-ValidatedStateDirectory {
    param(
        [Parameter(Mandatory)][string]$AllStateRoot,
        [Parameter(Mandatory)][string]$StateDirectory
    )

    $root = Get-NormalizedPath -Path $AllStateRoot
    $target = Get-NormalizedPath -Path $StateDirectory
    $parent = Get-NormalizedPath -Path (Split-Path -Parent $target)
    $leaf = Split-Path -Leaf $target
    if (-not (Test-PathEqual -Left $root -Right $parent) -or
        $leaf -notmatch '^[0-9a-f]{64}$') {
        throw "Refusing to remove unvalidated state directory: $target"
    }
    if (Test-Path -LiteralPath $target -PathType Container) {
        Remove-Item -LiteralPath $target -Recurse -Force
    }
}

function Invoke-PruneStates {
    param(
        [Parameter(Mandatory)][string]$AllStateRoot,
        [string]$ExcludeWorktreeId,
        [ValidateRange(1, 256)][int]$Limit = 64,
        [switch]$Automatic,
        [DateTimeOffset]$OperationDeadline = [DateTimeOffset]::MinValue
    )

    if (-not (Test-Path -LiteralPath $AllStateRoot -PathType Container)) {
        return @()
    }

    $deadline = if ($OperationDeadline -eq [DateTimeOffset]::MinValue) {
        [DateTimeOffset]::UtcNow.AddMinutes(2)
    }
    else {
        $OperationDeadline
    }
    $null = Get-RemainingMilliseconds -Deadline $deadline
    $results = [System.Collections.Generic.List[object]]::new()
    $selectionLockTimeout = if ($Automatic) {
        Get-RemainingProbeMilliseconds -Deadline $deadline -Maximum 100
    }
    else {
        Get-RemainingMilliseconds -Deadline $deadline -Maximum 120000
    }
    $directories = @(Get-RotatingStateDirectories -AllStateRoot $AllStateRoot `
        -ExcludeWorktreeId $ExcludeWorktreeId -Limit $Limit `
        -LockTimeoutMilliseconds $selectionLockTimeout)
    foreach ($directory in $directories) {
        $null = Get-RemainingMilliseconds -Deadline $deadline
        $lease = $null
        try {
            try {
                $lockTimeout = if ($Automatic) {
                    Get-RemainingProbeMilliseconds -Deadline $deadline -Maximum 100
                }
                else {
                    Get-RemainingMilliseconds -Deadline $deadline -Maximum 120000
                }
                $lease = Enter-FileLock `
                    -Path (Get-WorktreeLockPath -AllStateRoot $AllStateRoot `
                        -WorktreeId $directory.Name) `
                    -Deadline ([DateTimeOffset]::UtcNow.AddMilliseconds($lockTimeout))
            }
            catch {
                continue
            }

            $statePath = Join-Path $directory.FullName 'state.json'
            try {
                $state = Read-ServiceState -Path $statePath
            }
            catch {
                $results.Add([pscustomobject]@{
                    worktree_id = $directory.Name
                    action = 'refused-invalid-state'
                    reason = $_.Exception.Message
                })
                continue
            }
            if ($null -eq $state) {
                Remove-ValidatedStateDirectory -AllStateRoot $AllStateRoot `
                    -StateDirectory $directory.FullName
                $results.Add([pscustomobject]@{
                    worktree_id = $directory.Name
                    action = 'removed-empty-state'
                })
                continue
            }

            $ownership = Test-StateDirectoryOwnership -State $state `
                -DirectoryName $directory.Name
            if (-not $ownership.Matched) {
                $results.Add([pscustomobject]@{
                    worktree_id = $directory.Name
                    action = 'refused-state-ownership-mismatch'
                    reason = $ownership.Reason
                })
                continue
            }

            $rootValid = Test-ValidRecordedWorktree `
                -Root ([string]$state.worktree_root) -Deadline $deadline
            $identity = Test-ServiceProcessIdentity -State $state `
                -ExpectedRoot ([string]$state.worktree_root) -Deadline $deadline
            $health = if ($identity.Matched) {
                if ($Automatic) {
                    [pscustomobject]@{
                        Healthy = $rootValid
                        Reason = 'automatic prune does not perform HTTP health probes'
                    }
                }
                else {
                    $firstHealthTimeout = Get-RemainingProbeMilliseconds `
                        -Deadline $deadline -Maximum 1000
                    $firstHealth = Test-McpInitialize `
                        -Endpoint ([string]$state.endpoint) `
                        -TimeoutMilliseconds $firstHealthTimeout
                    if (-not $firstHealth.Healthy -and $rootValid) {
                        Start-Sleep -Milliseconds 250
                        $secondHealthTimeout = Get-RemainingProbeMilliseconds `
                            -Deadline $deadline -Maximum 1000
                        Test-McpInitialize `
                            -Endpoint ([string]$state.endpoint) `
                            -TimeoutMilliseconds $secondHealthTimeout
                    }
                    else {
                        $firstHealth
                    }
                }
            }
            else {
                [pscustomobject]@{ Healthy = $false; Reason = $identity.Reason }
            }
            if ($rootValid -and $identity.Matched -and $health.Healthy) {
                continue
            }

            if ($identity.Matched) {
                $stopTimeout = Get-RemainingMilliseconds -Deadline $deadline -Maximum 10000
                Stop-VerifiedServiceProcessTree -State $state `
                    -ExpectedRoot ([string]$state.worktree_root) `
                    -TimeoutMilliseconds $stopTimeout -Deadline $deadline
            }
            elseif ([int]$state.process_id -gt 0 -and
                $null -ne (Get-ProcessSnapshot -ProcessId ([int]$state.process_id) `
                    -Deadline $deadline)) {
                $results.Add([pscustomobject]@{
                    worktree_id = $directory.Name
                    action = 'refused-live-identity-mismatch'
                    root_valid = $rootValid
                    identity_matched = $false
                    healthy = $false
                    reason = $identity.Reason
                })
                continue
            }
            if ($rootValid) {
                try {
                    $context = Get-WorktreeContext -RootHint ([string]$state.worktree_root) `
                        -StateRootOverride $AllStateRoot
                    Write-DisabledGeneratedConfig -Context $context `
                        -Endpoint ([string]$state.endpoint)
                }
                catch {
                    # Prune never overwrites an unmanaged config and remains bounded.
                }
            }
            Remove-ValidatedStateDirectory -AllStateRoot $AllStateRoot `
                -StateDirectory $directory.FullName
            $results.Add([pscustomobject]@{
                worktree_id = $directory.Name
                action = if ($identity.Matched) { 'stopped-and-removed' } else { 'removed-stale-state' }
                root_valid = $rootValid
                identity_matched = $identity.Matched
                healthy = $health.Healthy
            })
        }
        finally {
            Exit-FileLock -Lease $lease
        }
    }
    return @($results)
}

function Invoke-EnsureAction {
    param(
        [Parameter(Mandatory)]$Context,
        [string]$ExecutableOverride,
        [Parameter(Mandatory)][int]$TimeoutSeconds
    )

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    try {
        try {
            $null = @(Invoke-PruneStates -AllStateRoot $Context.AllStateRoot `
                -ExcludeWorktreeId $Context.WorktreeId -Limit 16 -Automatic `
                -OperationDeadline $deadline)
        }
        catch {
            try {
                Write-LifecycleLog -Context $Context `
                    -Message "automatic-prune-skipped reason=$($_.Exception.Message)"
            }
            catch {
                # Optional housekeeping cannot disable the current worktree service.
            }
        }
        return Invoke-StartOrReuse -Context $Context `
            -ExecutableOverride $ExecutableOverride -TimeoutSeconds $TimeoutSeconds `
            -OperationDeadline $deadline
    }
    catch {
        $reason = $_.Exception.Message
        try {
            Write-DisabledGeneratedConfig -Context $Context
        }
        catch {
            $reason = "$reason Disabled config was not written: $($_.Exception.Message)"
        }
        try {
            Write-LifecycleLog -Context $Context -Message "ensure-disabled reason=$reason"
        }
        catch {
            # Ensure is deliberately fail-open for an optional developer tool.
        }
        Write-Warning "mcpls remains disabled: $reason"
        return [pscustomobject]@{
            action = 'disabled'
            worktree_id = $Context.WorktreeId
            worktree_root = $Context.Root
            process_id = $null
            endpoint = $null
            config_enabled = $false
            reason = $reason
        }
    }
}

function Invoke-StartAction {
    param(
        [Parameter(Mandatory)]$Context,
        [string]$ExecutableOverride,
        [Parameter(Mandatory)][int]$TimeoutSeconds
    )

    try {
        return Invoke-StartOrReuse -Context $Context `
            -ExecutableOverride $ExecutableOverride -TimeoutSeconds $TimeoutSeconds
    }
    catch {
        $reason = $_.Exception.Message
        try {
            Write-DisabledGeneratedConfig -Context $Context
        }
        catch {
            $reason = "$reason Disabled config was not written: $($_.Exception.Message)"
        }
        throw $reason
    }
}

function Invoke-StatusAction {
    param([Parameter(Mandatory)]$Context)

    try {
        $state = Read-ServiceState -Path $Context.StatePath
    }
    catch {
        return [pscustomobject]@{
            worktree_id = $Context.WorktreeId
            worktree_root = $Context.Root
            status = 'invalid-state'
            state_reason = $_.Exception.Message
            process_id = $null
            endpoint = $null
            identity_matched = $false
            healthy = $false
            rust_analyzer_descendants = 0
        }
    }
    if ($null -eq $state) {
        return [pscustomobject]@{
            worktree_id = $Context.WorktreeId
            worktree_root = $Context.Root
            status = 'not-recorded'
            process_id = $null
            endpoint = $null
            identity_matched = $false
            healthy = $false
            rust_analyzer_descendants = 0
        }
    }

    $identity = Test-ServiceProcessIdentity -State $state -ExpectedRoot $Context.Root
    $health = if ($identity.Matched) {
        Test-McpInitialize -Endpoint ([string]$state.endpoint)
    }
    else {
        [pscustomobject]@{ Healthy = $false; Reason = $identity.Reason }
    }
    $descendants = if ($identity.Matched) {
        @(Get-DescendantProcesses -RootProcessId ([int]$state.process_id))
    }
    else {
        @()
    }
    $rustAnalyzerCount = @($descendants | Where-Object {
        [string]$_.Name -ieq 'rust-analyzer.exe' -or [string]$_.Name -ieq 'rust-analyzer'
    }).Count

    return [pscustomobject]@{
        worktree_id = $Context.WorktreeId
        worktree_root = $Context.Root
        status = [string]$state.status
        process_id = [int]$state.process_id
        endpoint = [string]$state.endpoint
        identity_matched = $identity.Matched
        identity_reason = $identity.Reason
        healthy = $health.Healthy
        health_reason = $health.Reason
        rust_analyzer_descendants = $rustAnalyzerCount
        template_sha256 = [string]$state.template_sha256
    }
}

function Invoke-StopAction {
    param([Parameter(Mandatory)]$Context)

    $lease = $null
    try {
        $lease = Enter-FileLock -Path $Context.WorktreeLockPath `
            -Deadline ([DateTimeOffset]::UtcNow.AddMinutes(2))
        $state = Read-ServiceState -Path $Context.StatePath
        if ($null -eq $state) {
            throw 'No mcpls state is recorded for this worktree.'
        }

        $identity = Test-ServiceProcessIdentity -State $state -ExpectedRoot $Context.Root
        if (-not $identity.Matched) {
            throw "Refusing to stop an identity-mismatched process: $($identity.Reason)"
        }
        $health = Test-McpInitialize -Endpoint ([string]$state.endpoint) `
            -TimeoutMilliseconds 3000
        if (-not $health.Healthy) {
            throw "Refusing normal Stop because MCP initialize failed: $($health.Reason). Use Prune only after reviewing Status."
        }

        $stoppedProcessId = [int]$state.process_id
        Stop-VerifiedServiceProcessTree -State $state -ExpectedRoot $Context.Root
        $state.status = 'stopped'
        $state.process_id = 0
        $state.last_error = $null
        $bookkeepingErrors = [System.Collections.Generic.List[string]]::new()
        try {
            Write-DisabledGeneratedConfig -Context $Context `
                -Endpoint ([string]$state.endpoint)
        }
        catch {
            $bookkeepingErrors.Add("config disable failed: $($_.Exception.Message)")
            $state.last_error = $bookkeepingErrors[0]
        }
        try {
            Write-ServiceState -Context $Context -State $state
        }
        catch {
            $bookkeepingErrors.Add("state persistence failed: $($_.Exception.Message)")
        }
        try {
            Write-LifecycleLog -Context $Context -Message "stopped pid=$stoppedProcessId"
        }
        catch {
            $bookkeepingErrors.Add("lifecycle log failed: $($_.Exception.Message)")
        }
        if ($bookkeepingErrors.Count -gt 0) {
            throw (
                "mcpls process $stoppedProcessId stopped, but post-stop bookkeeping failed: " +
                ($bookkeepingErrors -join '; ')
            )
        }
        return [pscustomobject]@{
            action = 'stopped'
            worktree_id = $Context.WorktreeId
            process_id = $stoppedProcessId
            endpoint = [string]$state.endpoint
            config_enabled = $false
        }
    }
    finally {
        Exit-FileLock -Lease $lease
    }
}

function Invoke-LaneFlowMcpls {
    param(
        [Parameter(Mandatory)][string]$RequestedAction,
        [string]$RootHint,
        [string]$ExecutableOverride,
        [string]$StateRootOverride,
        [Parameter(Mandatory)][int]$TimeoutSeconds
    )

    if ($env:OS -ne 'Windows_NT') {
        if ($RequestedAction -eq 'Ensure') {
            Write-Warning 'The managed HTTP lifecycle is currently supported on Windows only; mcpls remains optional.'
            return
        }
        throw 'The managed HTTP lifecycle is currently supported on Windows only.'
    }

    try {
        $context = Get-WorktreeContext -RootHint $RootHint `
            -StateRootOverride $StateRootOverride
    }
    catch {
        if ($RequestedAction -eq 'Ensure') {
            $reason = "context discovery failed: $($_.Exception.Message)"
            $disable = Try-DisableGeneratedConfigWithoutWorktreeContext `
                -RootHint $RootHint
            if (-not $disable.Succeeded) {
                $reason = (
                    "$reason Managed config disabling was not completed: " +
                    $disable.Reason
                )
            }
            $warningPrefix = if ($disable.Succeeded) {
                'mcpls remains disabled'
            }
            else {
                'mcpls setup is unavailable and the prior config state is unknown'
            }
            Write-Warning "${warningPrefix}: $reason"
            return [pscustomobject]@{
                action = 'disabled'
                worktree_id = $null
                worktree_root = $RootHint
                process_id = $null
                endpoint = $null
                config_enabled = $disable.ConfigEnabled
                reason = $reason
            }
        }
        throw
    }
    switch ($RequestedAction) {
        'Ensure' {
            Invoke-EnsureAction -Context $context `
                -ExecutableOverride $ExecutableOverride -TimeoutSeconds $TimeoutSeconds
        }
        'Start' {
            Invoke-StartAction -Context $context `
                -ExecutableOverride $ExecutableOverride -TimeoutSeconds $TimeoutSeconds
        }
        'Status' {
            Invoke-StatusAction -Context $context
        }
        'Stop' {
            Invoke-StopAction -Context $context
        }
        'Prune' {
            Invoke-PruneStates -AllStateRoot $context.AllStateRoot
        }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        $result = Invoke-LaneFlowMcpls -RequestedAction $Action `
            -RootHint $WorktreeRoot -ExecutableOverride $McplsPath `
            -StateRootOverride $StateRoot -TimeoutSeconds $StartupTimeoutSeconds
        if ($null -ne $result) {
            $result | ConvertTo-Json -Depth 8
        }
    }
    catch {
        Write-Error $_.Exception.Message
        exit 1
    }
}
