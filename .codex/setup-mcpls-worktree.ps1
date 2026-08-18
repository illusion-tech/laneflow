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
$script:StateSchemaVersion = 1
$script:TemplateSchemaVersion = 1
$script:GeneratedConfigSchemaVersion = 1
$script:PortMinimum = 41000
$script:PortMaximum = 48999
$script:HttpPath = '/mcp'
$script:GeneratedConfigMarker = '# laneflow-mcpls-generated-schema: 1'

function Get-NormalizedPath {
    param([Parameter(Mandatory)][string]$Path)

    $fullPath = [System.IO.Path]::GetFullPath($Path)
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
            [System.StringComparison]::OrdinalIgnoreCase
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

function Get-WorktreeId {
    param([Parameter(Mandatory)][string]$CanonicalRoot)

    $identity = (Get-NormalizedPath -Path $CanonicalRoot).ToUpperInvariant()
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

function Enter-NamedMutex {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][TimeSpan]$Timeout
    )

    $mutex = [System.Threading.Mutex]::new($false, $Name)
    $acquired = $false
    try {
        try {
            $acquired = $mutex.WaitOne($Timeout)
        }
        catch [System.Threading.AbandonedMutexException] {
            $acquired = $true
        }
        if (-not $acquired) {
            throw "Timed out waiting for mutex $Name"
        }

        return [pscustomobject]@{ Mutex = $mutex; Acquired = $true }
    }
    catch {
        $mutex.Dispose()
        throw
    }
}

function Exit-NamedMutex {
    param($Lease)

    if ($null -eq $Lease) {
        return
    }
    try {
        if ($Lease.Acquired) {
            $Lease.Mutex.ReleaseMutex()
        }
    }
    finally {
        $Lease.Mutex.Dispose()
    }
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
            return $null
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
            'port',
            'endpoint',
            'template_sha256'
        )
        foreach ($property in $requiredProperties) {
            if ($null -eq $state.PSObject.Properties[$property]) {
                return $null
            }
        }
        return $state
    }
    catch {
        return $null
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

function Test-McplsExecutable {
    param([string]$ExecutableOverride)

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

        $versionOutput = (& $executable '--version' 2>&1 | Out-String).Trim()
        $versionExitCode = $LASTEXITCODE
        $helpOutput = (& $executable '--help' 2>&1 | Out-String)
        $helpExitCode = $LASTEXITCODE
        if ($versionExitCode -ne 0) {
            throw "mcpls --version failed with exit code $versionExitCode"
        }
        if ($helpExitCode -ne 0) {
            throw "mcpls --help failed with exit code $helpExitCode"
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

function Get-ProcessSnapshot {
    param([Parameter(Mandatory)][int]$ProcessId)

    try {
        $process = Get-Process -Id $ProcessId -ErrorAction Stop
        $cim = Get-CimInstance -ClassName Win32_Process `
            -Filter "ProcessId = $ProcessId" -ErrorAction Stop
        if ($null -eq $cim) {
            return $null
        }

        return [pscustomobject]@{
            ProcessId = $ProcessId
            ParentProcessId = [int]$cim.ParentProcessId
            Name = [string]$cim.Name
            ExecutablePath = Get-NormalizedPath -Path ([string]$process.Path)
            CommandLine = [string]$cim.CommandLine
            StartedAtUtc = $process.StartTime.ToUniversalTime()
        }
    }
    catch {
        return $null
    }
}

function Test-ServiceProcessIdentity {
    param(
        [Parameter(Mandatory)]$State,
        [Parameter(Mandatory)][string]$ExpectedRoot
    )

    try {
        if ([int]$State.schema_version -ne $script:StateSchemaVersion) {
            throw 'state schema mismatch'
        }
        if (-not (Test-PathEqual -Left ([string]$State.worktree_root) -Right $ExpectedRoot)) {
            throw 'worktree root mismatch'
        }
        if ([int]$State.process_id -le 0) {
            throw 'state has no running PID'
        }

        $snapshot = Get-ProcessSnapshot -ProcessId ([int]$State.process_id)
        if ($null -eq $snapshot) {
            throw 'recorded process is not running'
        }
        if (-not (Test-PathEqual -Left $snapshot.ExecutablePath -Right ([string]$State.executable_path))) {
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
        $actualStartedAt = $snapshot.StartedAtUtc.ToUniversalTime()
        $startDeltaSeconds = [Math]::Abs(
            ($actualStartedAt.Ticks - $expectedStartedAtUtc.Ticks) /
            [double][TimeSpan]::TicksPerSecond
        )
        if ($startDeltaSeconds -gt 2) {
            throw 'process start time mismatch'
        }

        $configPath = Get-NormalizedPath -Path ([string]$State.mcpls_config_path)
        $listen = "127.0.0.1:$([int]$State.port)"
        if ($snapshot.CommandLine.IndexOf($configPath, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
            throw 'command line does not contain the worktree mcpls.toml path'
        }
        if ($snapshot.CommandLine.IndexOf($listen, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
            throw 'command line does not contain the recorded loopback endpoint'
        }
        if ($snapshot.CommandLine.IndexOf('--http-path', [StringComparison]::OrdinalIgnoreCase) -lt 0 -or
            $snapshot.CommandLine.IndexOf($script:HttpPath, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
            throw 'command line does not contain the HTTP path'
        }

        return [pscustomobject]@{
            Matched = $true
            Reason = $null
            Snapshot = $snapshot
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

function Test-McpInitialize {
    param(
        [Parameter(Mandatory)][string]$Endpoint,
        [ValidateRange(1, 30)][int]$TimeoutSeconds = 3
    )

    $client = [System.Net.Http.HttpClient]::new()
    $client.Timeout = [TimeSpan]::FromSeconds($TimeoutSeconds)
    $sessionId = $null
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

        $response = $client.SendAsync($request).GetAwaiter().GetResult()
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
            $serverInfo = $result.PSObject.Properties['serverInfo']
            $serverName = if ($null -ne $serverInfo -and $null -ne $serverInfo.Value) {
                [string]$serverInfo.Value.name
            }
            else {
                $null
            }

            return [pscustomobject]@{
                Healthy = $true
                Reason = $null
                ServerName = $serverName
                ProtocolVersion = [string]$result.protocolVersion
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
        if (-not [string]::IsNullOrWhiteSpace($sessionId)) {
            try {
                $delete = [System.Net.Http.HttpRequestMessage]::new(
                    [System.Net.Http.HttpMethod]::Delete,
                    $Endpoint
                )
                $delete.Headers.TryAddWithoutValidation('Mcp-Session-Id', $sessionId) | Out-Null
                $deleteResponse = $client.SendAsync($delete).GetAwaiter().GetResult()
                $deleteResponse.Dispose()
                $delete.Dispose()
            }
            catch {
                # Session cleanup is best effort; initialize success remains authoritative.
            }
        }
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

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = $Context.Root
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    foreach ($argument in @(
        '--config', $Context.McplsConfigPath,
        '--listen', "127.0.0.1:$Port",
        '--http-path', $script:HttpPath
    )) {
        $startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::Start($startInfo)
    if ($null -eq $process) {
        throw 'Failed to create the mcpls process.'
    }
    return $process
}

function Stop-OwnedProcessTree {
    param([Parameter(Mandatory)][int]$ProcessId)

    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        return
    }
    $process.Kill($true)
    $process.WaitForExit(10000) | Out-Null
}

function Get-DescendantProcesses {
    param([Parameter(Mandatory)][int]$RootProcessId)

    try {
        $all = @(Get-CimInstance -ClassName Win32_Process -ErrorAction Stop)
    }
    catch {
        return @()
    }
    $result = [System.Collections.Generic.List[object]]::new()
    $frontier = [System.Collections.Generic.Queue[int]]::new()
    $frontier.Enqueue($RootProcessId)
    while ($frontier.Count -gt 0) {
        $parent = $frontier.Dequeue()
        foreach ($child in $all | Where-Object { [int]$_.ParentProcessId -eq $parent }) {
            $result.Add($child)
            $frontier.Enqueue([int]$child.ProcessId)
        }
    }
    return @($result)
}

function New-ServiceState {
    param(
        [Parameter(Mandatory)]$Context,
        [Parameter(Mandatory)]$Tool,
        [Parameter(Mandatory)][int]$Port,
        [Parameter(Mandatory)]$Process,
        [Parameter(Mandatory)]$TemplateInfo,
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

        $health = Test-McpInitialize -Endpoint $Endpoint -TimeoutSeconds 3
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
        [Parameter(Mandatory)][int]$TimeoutSeconds,
        [int]$PreferredPort = 0
    )

    if (-not (Test-Path -LiteralPath $Context.McplsConfigPath -PathType Leaf)) {
        throw "Missing worktree mcpls.toml: $($Context.McplsConfigPath)"
    }

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastFailure = $null
    for ($offset = 0; $offset -lt 32; $offset++) {
        $allocationLease = $null
        $process = $null
        $port = Get-PortCandidate -WorktreeId $Context.WorktreeId `
            -Offset $offset -PreferredPort $PreferredPort
        try {
            $allocationLease = Enter-NamedMutex `
                -Name 'Local\LaneFlow-mcpls-port-allocation-v1' `
                -Timeout ([TimeSpan]::FromMinutes(2))
            if (-not (Test-LoopbackPortAvailable -Port $port)) {
                continue
            }

            $process = Start-McplsProcess -Executable $Tool.Path `
                -Context $Context -Port $port
            $listenDeadline = [DateTimeOffset]::UtcNow.AddSeconds(5)
            while ([DateTimeOffset]::UtcNow -lt $listenDeadline -and
                -not $process.HasExited -and
                -not (Test-LoopbackPortListening -Port $port)) {
                Start-Sleep -Milliseconds 100
            }
            if ($process.HasExited -or -not (Test-LoopbackPortListening -Port $port)) {
                $lastFailure = "mcpls did not bind 127.0.0.1:$port"
                if (-not $process.HasExited) {
                    $process.Kill($true)
                    $process.WaitForExit(5000) | Out-Null
                }
                $process.Dispose()
                $process = $null
                continue
            }
        }
        finally {
            Exit-NamedMutex -Lease $allocationLease
        }

        $state = New-ServiceState -Context $Context -Tool $Tool -Port $port `
            -Process $process -TemplateInfo $TemplateInfo -Status 'starting'
        Write-ServiceState -Context $Context -State $state
        Write-LifecycleLog -Context $Context `
            -Message "started pid=$($process.Id) endpoint=$($state.endpoint)"

        $health = Wait-McplsReady -Process $process -Endpoint $state.endpoint `
            -Deadline $deadline
        if (-not $health.Healthy) {
            $state.status = 'failed'
            $state.last_error = $health.Reason
            Write-ServiceState -Context $Context -State $state
            Write-LifecycleLog -Context $Context `
                -Message "startup-failed pid=$($process.Id) reason=$($health.Reason)"
            if (-not $process.HasExited) {
                $process.Kill($true)
                $process.WaitForExit(10000) | Out-Null
            }
            $process.Dispose()
            throw "mcpls failed HTTP initialize: $($health.Reason)"
        }

        $state.status = 'ready'
        $state.last_error = $null
        Write-ServiceState -Context $Context -State $state
        $process.Dispose()
        return $state
    }

    throw "No mcpls port could be started in the bounded probe window. Last failure: $lastFailure"
}

function Invoke-StartOrReuse {
    param(
        [Parameter(Mandatory)]$Context,
        [string]$ExecutableOverride,
        [Parameter(Mandatory)][int]$TimeoutSeconds
    )

    Assert-GeneratedConfigOwnership -Context $Context
    $template = Get-TemplateInfo -Context $Context
    $tool = Test-McplsExecutable -ExecutableOverride $ExecutableOverride
    if (-not $tool.Valid) {
        throw $tool.Reason
    }

    $lease = $null
    try {
        $lease = Enter-NamedMutex `
            -Name "Local\LaneFlow-mcpls-worktree-$($Context.WorktreeId)" `
            -Timeout ([TimeSpan]::FromMinutes(2))
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
                -ExpectedRoot $Context.Root
            if ($identity.Matched) {
                $health = Test-McpInitialize -Endpoint ([string]$existingState.endpoint)
                $sameTool = Test-PathEqual -Left ([string]$existingState.executable_path) `
                    -Right $tool.Path
                if ($health.Healthy -and $sameTool -and
                    [string]$existingState.mcpls_version -eq $script:McplsVersion) {
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
                    -Message "replacing-owned pid=$($existingState.process_id) healthy=$($health.Healthy) same_tool=$sameTool"
                Stop-OwnedProcessTree -ProcessId ([int]$existingState.process_id)
            }
            elseif ([int]$existingState.process_id -gt 0) {
                $recordedProcess = Get-ProcessSnapshot `
                    -ProcessId ([int]$existingState.process_id)
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
            -TemplateInfo $template -TimeoutSeconds $TimeoutSeconds `
            -PreferredPort $preferredPort
        Write-GeneratedConfig -Context $Context -TemplateInfo $template `
            -Endpoint ([string]$state.endpoint) -Enabled $true
        Write-LifecycleLog -Context $Context `
            -Message "ready pid=$($state.process_id) endpoint=$($state.endpoint)"
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
        Exit-NamedMutex -Lease $lease
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

function Test-ValidRecordedWorktree {
    param([Parameter(Mandatory)][string]$Root)

    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        return $false
    }
    try {
        $canonical = Get-CanonicalWorktreeRoot -RootHint $Root
        return Test-PathEqual -Left $canonical -Right $Root
    }
    catch {
        return $false
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
        [ValidateRange(1, 256)][int]$Limit = 64
    )

    if (-not (Test-Path -LiteralPath $AllStateRoot -PathType Container)) {
        return @()
    }

    $results = [System.Collections.Generic.List[object]]::new()
    $directories = @(Get-ChildItem -LiteralPath $AllStateRoot -Directory |
        Where-Object { $_.Name -match '^[0-9a-f]{64}$' } |
        Select-Object -First $Limit)
    foreach ($directory in $directories) {
        if ($directory.Name -eq $ExcludeWorktreeId) {
            continue
        }
        $lease = $null
        try {
            try {
                $lease = Enter-NamedMutex `
                    -Name "Local\LaneFlow-mcpls-worktree-$($directory.Name)" `
                    -Timeout ([TimeSpan]::FromMilliseconds(100))
            }
            catch {
                continue
            }

            $statePath = Join-Path $directory.FullName 'state.json'
            $state = Read-ServiceState -Path $statePath
            if ($null -eq $state) {
                Remove-ValidatedStateDirectory -AllStateRoot $AllStateRoot `
                    -StateDirectory $directory.FullName
                $results.Add([pscustomobject]@{ worktree_id = $directory.Name; action = 'removed-invalid-state' })
                continue
            }

            $rootValid = Test-ValidRecordedWorktree -Root ([string]$state.worktree_root)
            $identity = Test-ServiceProcessIdentity -State $state `
                -ExpectedRoot ([string]$state.worktree_root)
            $health = if ($identity.Matched) {
                Test-McpInitialize -Endpoint ([string]$state.endpoint)
            }
            else {
                [pscustomobject]@{ Healthy = $false; Reason = $identity.Reason }
            }
            if ($rootValid -and $identity.Matched -and $health.Healthy) {
                continue
            }

            if ($identity.Matched) {
                Stop-OwnedProcessTree -ProcessId ([int]$state.process_id)
            }
            elseif ([int]$state.process_id -gt 0 -and
                $null -ne (Get-ProcessSnapshot -ProcessId ([int]$state.process_id))) {
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
            Exit-NamedMutex -Lease $lease
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

    try {
        $null = @(Invoke-PruneStates -AllStateRoot $Context.AllStateRoot `
            -ExcludeWorktreeId $Context.WorktreeId -Limit 16)
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

    $state = Read-ServiceState -Path $Context.StatePath
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
        $lease = Enter-NamedMutex `
            -Name "Local\LaneFlow-mcpls-worktree-$($Context.WorktreeId)" `
            -Timeout ([TimeSpan]::FromMinutes(2))
        $state = Read-ServiceState -Path $Context.StatePath
        if ($null -eq $state) {
            throw 'No mcpls state is recorded for this worktree.'
        }

        $identity = Test-ServiceProcessIdentity -State $state -ExpectedRoot $Context.Root
        if (-not $identity.Matched) {
            throw "Refusing to stop an identity-mismatched process: $($identity.Reason)"
        }
        $health = Test-McpInitialize -Endpoint ([string]$state.endpoint)
        if (-not $health.Healthy) {
            throw "Refusing normal Stop because MCP initialize failed: $($health.Reason). Use Prune only after reviewing Status."
        }

        Stop-OwnedProcessTree -ProcessId ([int]$state.process_id)
        $state.status = 'stopped'
        $state.last_error = $null
        Write-ServiceState -Context $Context -State $state
        Write-DisabledGeneratedConfig -Context $Context -Endpoint ([string]$state.endpoint)
        Write-LifecycleLog -Context $Context -Message "stopped pid=$($state.process_id)"
        return [pscustomobject]@{
            action = 'stopped'
            worktree_id = $Context.WorktreeId
            process_id = [int]$state.process_id
            endpoint = [string]$state.endpoint
            config_enabled = $false
        }
    }
    finally {
        Exit-NamedMutex -Lease $lease
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

    $context = Get-WorktreeContext -RootHint $RootHint `
        -StateRootOverride $StateRootOverride
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
