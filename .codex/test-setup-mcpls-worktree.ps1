<#
.SYNOPSIS
Runs dependency-free contract tests for setup-mcpls-worktree.ps1.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'setup-mcpls-worktree.ps1')

$script:Passed = 0

function Assert-True {
    param(
        [Parameter(Mandatory)][bool]$Condition,
        [Parameter(Mandatory)][string]$Message
    )
    if (-not $Condition) {
        throw "Assertion failed: $Message"
    }
    $script:Passed++
}

function Assert-Throws {
    param(
        [Parameter(Mandatory)][scriptblock]$Operation,
        [Parameter(Mandatory)][string]$Message
    )
    $threw = $false
    try {
        & $Operation
    }
    catch {
        $threw = $true
    }
    Assert-True -Condition $threw -Message $Message
}

$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) (
    "laneflow-mcpls-tests-$([System.Guid]::NewGuid().ToString('N'))"
)
[System.IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null

try {
    $repositoryRoot = Get-CanonicalWorktreeRoot -RootHint (Split-Path -Parent $PSScriptRoot)
    $repositoryContext = Get-WorktreeContext -RootHint $repositoryRoot `
        -StateRootOverride (Join-Path $temporaryRoot 'state')
    $template = Get-TemplateInfo -Context $repositoryContext

    Assert-True -Condition ($template.Hash -match '^[0-9a-f]{64}$') `
        -Message 'template hash is lowercase SHA-256'

    $firstId = Get-WorktreeId -CanonicalRoot $repositoryRoot
    $sameId = Get-WorktreeId -CanonicalRoot $repositoryRoot.ToUpperInvariant()
    $secondId = Get-WorktreeId -CanonicalRoot "$repositoryRoot-other"
    Assert-True -Condition ($firstId -eq $sameId) `
        -Message 'Windows worktree identity is case-insensitive'
    Assert-True -Condition ($firstId -ne $secondId) `
        -Message 'different worktree roots have different identities'

    $firstPort = Get-PreferredPort -WorktreeId $firstId
    $samePort = Get-PreferredPort -WorktreeId $firstId
    Assert-True -Condition ($firstPort -eq $samePort) `
        -Message 'preferred port is deterministic'
    Assert-True -Condition (
        $firstPort -ge $script:PortMinimum -and $firstPort -le $script:PortMaximum
    ) -Message 'preferred port is inside the frozen loopback range'

    $renderPort = Get-PortCandidate -WorktreeId $firstId -Offset 1
    $disabledPort = Get-PortCandidate -WorktreeId $firstId -Offset 2
    $identityPort = Get-PortCandidate -WorktreeId $firstId -Offset 3
    $enabledConfig = New-GeneratedConfigContent -TemplateInfo $template `
        -Endpoint "http://127.0.0.1:$renderPort/mcp" -Enabled $true
    Assert-True -Condition ($enabledConfig -match '(?m)^# laneflow-mcpls-generated-schema: 1$') `
        -Message 'generated config carries the managed marker'
    Assert-True -Condition ($enabledConfig -match '(?m)^enabled = true$') `
        -Message 'ready config is enabled'
    Assert-True -Condition ($enabledConfig -notmatch '__LANEFLOW_MCPLS_') `
        -Message 'generated config contains no unresolved placeholders'
    $toolNames = @(
        'get_hover',
        'get_definition',
        'get_references',
        'get_document_symbols',
        'workspace_symbol_search',
        'get_diagnostics'
    )
    foreach ($toolName in $toolNames) {
        Assert-True -Condition ($enabledConfig -match [regex]::Escape("`"$toolName`"")) `
            -Message "read-only tool $toolName remains enabled"
    }
    Assert-True -Condition ($enabledConfig -notmatch 'rename|format') `
        -Message 'write-oriented tools are absent'

    $configDirectory = Join-Path $temporaryRoot '.codex'
    [System.IO.Directory]::CreateDirectory($configDirectory) | Out-Null
    $testContext = [pscustomobject]@{
        Root = $repositoryRoot
        WorktreeId = $repositoryContext.WorktreeId
        McplsConfigPath = $repositoryContext.McplsConfigPath
        TemplatePath = $repositoryContext.TemplatePath
        GeneratedConfigPath = Join-Path $configDirectory 'config.toml'
        AllStateRoot = Join-Path $temporaryRoot 'state'
        StateDirectory = Join-Path (Join-Path $temporaryRoot 'state') $repositoryContext.WorktreeId
        StatePath = Join-Path (Join-Path (Join-Path $temporaryRoot 'state') $repositoryContext.WorktreeId) 'state.json'
        LogPath = Join-Path (Join-Path (Join-Path $temporaryRoot 'state') $repositoryContext.WorktreeId) 'lifecycle.log'
    }

    [System.IO.File]::WriteAllText(
        $testContext.GeneratedConfigPath,
        "[mcp_servers.unmanaged]`nenabled = true`n",
        [System.Text.UTF8Encoding]::new($false)
    )
    Assert-Throws -Operation {
        Assert-GeneratedConfigOwnership -Context $testContext
    } -Message 'unmanaged project config is never overwritten'

    [System.IO.File]::WriteAllText(
        $testContext.GeneratedConfigPath,
        "$script:GeneratedConfigMarker`n",
        [System.Text.UTF8Encoding]::new($false)
    )
    Write-GeneratedConfig -Context $testContext -TemplateInfo $template `
        -Endpoint "http://127.0.0.1:$disabledPort/mcp" -Enabled $false
    $disabledConfig = [System.IO.File]::ReadAllText($testContext.GeneratedConfigPath)
    Assert-True -Condition ($disabledConfig -match '(?m)^enabled = false$') `
        -Message 'managed config can be atomically disabled'
    Assert-True -Condition ($disabledConfig -match "# template-sha256: $($template.Hash)") `
        -Message 'generated config records the current template hash'

    $missingExecutable = Join-Path $temporaryRoot 'missing-mcpls.exe'
    $ensureResult = Invoke-EnsureAction -Context $testContext `
        -ExecutableOverride $missingExecutable -TimeoutSeconds 5
    Assert-True -Condition ($ensureResult.action -eq 'disabled') `
        -Message 'Ensure is fail-open when mcpls is unavailable'
    Assert-True -Condition (-not $ensureResult.config_enabled) `
        -Message 'Ensure reports unavailable mcpls as disabled'

    $featurelessExecutable = Join-Path $temporaryRoot 'featureless-mcpls.cmd'
    $featurelessBody = @'
@echo off
if "%~1"=="--version" (
  echo mcpls 0.3.9
  exit /b 0
)
if "%~1"=="--help" (
  echo   --config ^<FILE^>
  exit /b 0
)
exit /b 1
'@
    [System.IO.File]::WriteAllText(
        $featurelessExecutable,
        $featurelessBody,
        [System.Text.UTF8Encoding]::new($false)
    )
    $featurelessTool = Test-McplsExecutable -ExecutableOverride $featurelessExecutable
    Assert-True -Condition (
        -not $featurelessTool.Valid -and
        $featurelessTool.Reason -match 'transport-http'
    ) -Message 'same-version mcpls without HTTP feature is rejected'

    $wrongVersionExecutable = Join-Path $temporaryRoot 'wrong-version-mcpls.cmd'
    $wrongVersionBody = @'
@echo off
if "%~1"=="--version" (
  echo mcpls 0.3.8
  exit /b 0
)
if "%~1"=="--help" (
  echo   --listen ^<ADDR^>
  echo   --http-path ^<PATH^>
  exit /b 0
)
exit /b 1
'@
    [System.IO.File]::WriteAllText(
        $wrongVersionExecutable,
        $wrongVersionBody,
        [System.Text.UTF8Encoding]::new($false)
    )
    $wrongVersionTool = Test-McplsExecutable -ExecutableOverride $wrongVersionExecutable
    Assert-True -Condition (
        -not $wrongVersionTool.Valid -and
        $wrongVersionTool.Reason -match 'Expected mcpls 0.3.9'
    ) -Message 'unexpected mcpls version is rejected'

    Write-GeneratedConfig -Context $testContext -TemplateInfo $template `
        -Endpoint "http://127.0.0.1:$renderPort/mcp" -Enabled $true
    Assert-Throws -Operation {
        Invoke-StartAction -Context $testContext `
            -ExecutableOverride $missingExecutable -TimeoutSeconds 5
    } -Message 'strict Start returns an error when mcpls is unavailable'
    $strictFailureConfig = [System.IO.File]::ReadAllText($testContext.GeneratedConfigPath)
    Assert-True -Condition ($strictFailureConfig -match '(?m)^enabled = false$') `
        -Message 'strict Start failure leaves the generated config disabled'

    [System.IO.Directory]::CreateDirectory($testContext.StateDirectory) | Out-Null
    [System.IO.File]::WriteAllText(
        $testContext.StatePath,
        '{"schema_version":1}',
        [System.Text.UTF8Encoding]::new($false)
    )
    Assert-True -Condition ($null -eq (Read-ServiceState -Path $testContext.StatePath)) `
        -Message 'incomplete state records fail closed'

    $currentSnapshot = Get-ProcessSnapshot -ProcessId $PID
    Assert-True -Condition ($null -ne $currentSnapshot) `
        -Message 'current process snapshot is available for identity testing'
    $fakeState = [pscustomobject]@{
        schema_version = $script:StateSchemaVersion
        worktree_id = $repositoryContext.WorktreeId
        worktree_root = $repositoryRoot
        status = 'ready'
        process_id = $PID
        process_started_at_utc = $currentSnapshot.StartedAtUtc.ToString('O')
        executable_path = $currentSnapshot.ExecutablePath
        mcpls_version = $script:McplsVersion
        mcpls_config_path = $repositoryContext.McplsConfigPath
        port = $identityPort
        endpoint = "http://127.0.0.1:$identityPort/mcp"
        template_sha256 = $template.Hash
    }
    $identity = Test-ServiceProcessIdentity -State $fakeState -ExpectedRoot $repositoryRoot
    Assert-True -Condition (-not $identity.Matched) `
        -Message 'PID, start time, and executable alone cannot impersonate the service command line'
    Write-AtomicUtf8File -Path $testContext.StatePath `
        -Content (($fakeState | ConvertTo-Json -Depth 6) + "`n")
    Assert-Throws -Operation {
        Invoke-StopAction -Context $testContext
    } -Message 'Stop refuses a live PID whose command line does not match the service'
    Assert-True -Condition ($null -ne (Get-Process -Id $PID -ErrorAction SilentlyContinue)) `
        -Message 'identity-mismatched Stop leaves the unrelated process running'
    $pruneResult = @(Invoke-PruneStates -AllStateRoot $testContext.AllStateRoot)
    Assert-True -Condition (
        $pruneResult.Count -eq 1 -and
        $pruneResult[0].action -eq 'refused-live-identity-mismatch'
    ) -Message 'Prune reports a live identity mismatch instead of hiding it'
    Assert-True -Condition (Test-Path -LiteralPath $testContext.StateDirectory -PathType Container) `
        -Message 'Prune preserves state evidence for a live identity mismatch'
    Assert-True -Condition ($null -ne (Get-Process -Id $PID -ErrorAction SilentlyContinue)) `
        -Message 'identity-mismatched Prune leaves the unrelated process running'

    $health = Test-McpInitialize -Endpoint 'http://127.0.0.1:1/mcp' -TimeoutSeconds 1
    Assert-True -Condition (-not $health.Healthy) `
        -Message 'a closed TCP endpoint does not pass MCP initialize health'

    $listenerPort = Get-PortCandidate -WorktreeId $firstId -Offset 4
    $listener = [System.Net.Sockets.TcpListener]::new(
        [System.Net.IPAddress]::Loopback,
        $listenerPort
    )
    try {
        $listener.Start()
        Assert-True -Condition (-not (Test-LoopbackPortAvailable -Port $listenerPort)) `
            -Message 'port allocation rejects a port already bound by another process'
    }
    finally {
        $listener.Stop()
    }

    $safeStateRoot = Join-Path $temporaryRoot 'safe-delete-root'
    $safeStateDirectory = Join-Path $safeStateRoot ('a' * 64)
    [System.IO.Directory]::CreateDirectory($safeStateDirectory) | Out-Null
    [System.IO.File]::WriteAllText(
        (Join-Path $safeStateDirectory 'state.json'),
        '{}',
        [System.Text.UTF8Encoding]::new($false)
    )
    Remove-ValidatedStateDirectory -AllStateRoot $safeStateRoot `
        -StateDirectory $safeStateDirectory
    Assert-True -Condition (-not (Test-Path -LiteralPath $safeStateDirectory)) `
        -Message 'validated state child can be removed'
    Assert-Throws -Operation {
        Remove-ValidatedStateDirectory -AllStateRoot $safeStateRoot `
            -StateDirectory $safeStateRoot
    } -Message 'state cleanup refuses a broad root target'

    [pscustomobject]@{
        result = 'pass'
        assertions = $script:Passed
        external_dependencies = 'none'
    } | ConvertTo-Json
}
finally {
    $resolvedTemporaryRoot = Get-NormalizedPath -Path $temporaryRoot
    $systemTemporaryRoot = Get-NormalizedPath -Path ([System.IO.Path]::GetTempPath())
    if (-not $resolvedTemporaryRoot.StartsWith(
        $systemTemporaryRoot + [System.IO.Path]::DirectorySeparatorChar,
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "Refusing to remove unexpected test directory: $resolvedTemporaryRoot"
    }
    if (Test-Path -LiteralPath $resolvedTemporaryRoot -PathType Container) {
        Remove-Item -LiteralPath $resolvedTemporaryRoot -Recurse -Force
    }
}
