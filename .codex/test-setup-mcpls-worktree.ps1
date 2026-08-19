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
    $secondId = Get-WorktreeId -CanonicalRoot "$repositoryRoot-other"
    $caseAlias = $repositoryRoot.ToUpperInvariant()
    if (Test-Path -LiteralPath $caseAlias -PathType Container) {
        $sameId = Get-WorktreeId -CanonicalRoot $caseAlias
        Assert-True -Condition ($firstId -eq $sameId) `
            -Message 'existing Windows path aliases canonicalize to their on-disk casing'
    }
    Assert-True -Condition ($firstId -ne $secondId) `
        -Message 'different worktree roots have different identities'
    $caseSensitiveUpper = Get-WorktreeId -CanonicalRoot (
        Join-Path $temporaryRoot 'Synthetic-Case-Path'
    )
    $caseSensitiveLower = Get-WorktreeId -CanonicalRoot (
        Join-Path $temporaryRoot 'synthetic-case-path'
    )
    Assert-True -Condition ($caseSensitiveUpper -ne $caseSensitiveLower) `
        -Message 'non-existing case-distinct roots retain distinct identities'

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
    Assert-True -Condition ($enabledConfig -match '(?m)^enabled = true\r?$') `
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
        LockDirectory = Join-Path (Join-Path $temporaryRoot 'state') '.locks'
        WorktreeLockPath = Join-Path (Join-Path (Join-Path $temporaryRoot 'state') '.locks') "$($repositoryContext.WorktreeId).lock"
        PortLockPath = Join-Path (Join-Path (Join-Path $temporaryRoot 'state') '.locks') 'port-allocation.lock'
    }

    $configHashPath = Join-Path $temporaryRoot 'hash-input.toml'
    [System.IO.File]::WriteAllText(
        $configHashPath,
        "[workspace]`nroot = 'first'`n",
        [System.Text.UTF8Encoding]::new($false)
    )
    $firstConfigHash = Get-FileSha256Hex -Path $configHashPath
    [System.IO.File]::WriteAllText(
        $configHashPath,
        "[workspace]`nroot = 'second'`n",
        [System.Text.UTF8Encoding]::new($false)
    )
    $secondConfigHash = Get-FileSha256Hex -Path $configHashPath
    Assert-True -Condition ($firstConfigHash -ne $secondConfigHash) `
        -Message 'mcpls.toml content changes alter the persisted service hash'

    $testLockPath = Join-Path $testContext.LockDirectory 'exclusive-test.lock'
    $firstLease = Enter-FileLock -Path $testLockPath `
        -Deadline ([DateTimeOffset]::UtcNow.AddSeconds(1))
    try {
        Assert-Throws -Operation {
            Enter-FileLock -Path $testLockPath `
                -Deadline ([DateTimeOffset]::UtcNow.AddMilliseconds(100))
        } -Message 'filesystem locks serialize callers across independent handles'
    }
    finally {
        Exit-FileLock -Lease $firstLease
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
    Assert-True -Condition ($disabledConfig -match '(?m)^enabled = false\r?$') `
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

    $originalPruneStates = (Get-Command Invoke-PruneStates).ScriptBlock
    $script:CapturedPruneDeadline = [DateTimeOffset]::MinValue
    $deadlineCaptureStarted = [DateTimeOffset]::UtcNow
    try {
        Set-Item -Path Function:Invoke-PruneStates -Value {
            param(
                $AllStateRoot,
                $ExcludeWorktreeId,
                $Limit,
                [switch]$Automatic,
                [DateTimeOffset]$OperationDeadline
            )
            $script:CapturedPruneDeadline = $OperationDeadline
            return @()
        }
        $null = Invoke-EnsureAction -Context $testContext `
            -ExecutableOverride $missingExecutable -TimeoutSeconds 5
    }
    finally {
        Set-Item -Path Function:Invoke-PruneStates -Value $originalPruneStates
    }
    Assert-True -Condition (
        $script:CapturedPruneDeadline -gt $deadlineCaptureStarted -and
        $script:CapturedPruneDeadline -le $deadlineCaptureStarted.AddSeconds(6)
    ) -Message 'Ensure creates and passes its startup deadline before automatic pruning'

    $missingWorktreeRoot = Join-Path $temporaryRoot 'not-a-worktree'
    $contextFailureResult = Invoke-LaneFlowMcpls -RequestedAction 'Ensure' `
        -RootHint $missingWorktreeRoot -StateRootOverride $testContext.AllStateRoot `
        -TimeoutSeconds 5
    Assert-True -Condition (
        $contextFailureResult.action -eq 'disabled' -and
        $null -eq $contextFailureResult.config_enabled -and
        $contextFailureResult.reason -match 'disabling was not completed'
    ) -Message 'context failure reports an unknown prior config state when disabling is unsafe'
    Assert-Throws -Operation {
        Invoke-LaneFlowMcpls -RequestedAction 'Start' `
            -RootHint $missingWorktreeRoot `
            -StateRootOverride $testContext.AllStateRoot -TimeoutSeconds 5
    } -Message 'strict Start still rejects a context discovery failure'

    $fallbackRoot = Join-Path $temporaryRoot 'fallback-root'
    $fallbackCodexDirectory = Join-Path $fallbackRoot '.codex'
    [System.IO.Directory]::CreateDirectory($fallbackCodexDirectory) | Out-Null
    $fallbackTemplatePath = Join-Path $fallbackCodexDirectory 'config.template.toml'
    $fallbackGeneratedPath = Join-Path $fallbackCodexDirectory 'config.toml'
    [System.IO.File]::WriteAllText(
        $fallbackTemplatePath,
        $template.Content,
        [System.Text.UTF8Encoding]::new($false)
    )
    $fallbackContext = [pscustomobject]@{
        TemplatePath = $fallbackTemplatePath
        GeneratedConfigPath = $fallbackGeneratedPath
    }
    Write-GeneratedConfig -Context $fallbackContext -TemplateInfo $template `
        -Endpoint "http://127.0.0.1:$renderPort/mcp" -Enabled $true
    $fallbackDisable = Try-DisableGeneratedConfigWithoutWorktreeContext `
        -RootHint $fallbackRoot
    $fallbackContent = [System.IO.File]::ReadAllText($fallbackGeneratedPath)
    Assert-True -Condition (
        $fallbackDisable.Succeeded -and -not $fallbackDisable.ConfigEnabled
    ) -Message 'context-free fallback can disable a safely located managed config'
    Assert-True -Condition ($fallbackContent -match '(?m)^enabled = false\r?$') `
        -Message 'context-free fallback atomically renders the managed config disabled'

    $featurelessExecutable = Join-Path $temporaryRoot 'featureless-mcpls.exe'
    $wrongVersionExecutable = Join-Path $temporaryRoot 'wrong-version-mcpls.exe'
    [System.IO.File]::WriteAllText(
        $featurelessExecutable,
        'capability fixture; execution is mocked',
        [System.Text.UTF8Encoding]::new($false)
    )
    [System.IO.File]::WriteAllText(
        $wrongVersionExecutable,
        'version fixture; execution is mocked',
        [System.Text.UTF8Encoding]::new($false)
    )
    $originalApplicationCapture = (Get-Command Invoke-ApplicationCapture).ScriptBlock
    try {
        Set-Item -Path Function:Invoke-ApplicationCapture -Value {
            param($Executable, $Arguments, $Deadline)
            $isFeatureless = (Split-Path -Leaf $Executable) -eq 'featureless-mcpls.exe'
            if ($Arguments[0] -eq '--version') {
                return [pscustomobject]@{
                    ExitCode = 0
                    Output = if ($isFeatureless) { 'mcpls 0.3.9' } else { 'mcpls 0.3.8' }
                }
            }
            return [pscustomobject]@{
                ExitCode = 0
                Output = if ($isFeatureless) {
                    "  --config <FILE>`n"
                }
                else {
                    "  --listen <ADDR>`n  --http-path <PATH>`n"
                }
            }
        }
        $featurelessTool = Test-McplsExecutable `
            -ExecutableOverride $featurelessExecutable
        Assert-True -Condition (
            -not $featurelessTool.Valid -and
            $featurelessTool.Reason -match 'transport-http'
        ) -Message 'same-version mcpls without HTTP feature is rejected'

        $wrongVersionTool = Test-McplsExecutable `
            -ExecutableOverride $wrongVersionExecutable
        Assert-True -Condition (
            -not $wrongVersionTool.Valid -and
            $wrongVersionTool.Reason -match 'Expected mcpls 0.3.9'
        ) -Message 'unexpected mcpls version is rejected'
    }
    finally {
        Set-Item -Path Function:Invoke-ApplicationCapture `
            -Value $originalApplicationCapture
    }

    $currentPowerShell = (Get-Process -Id $PID).Path
    $captureStarted = [DateTimeOffset]::UtcNow
    Assert-Throws -Operation {
        Invoke-ApplicationCapture -Executable $currentPowerShell `
            -Arguments @('-NoLogo', '-NoProfile', '-Command', 'Start-Sleep -Seconds 60') `
            -Deadline ([DateTimeOffset]::UtcNow.AddMilliseconds(250))
    } -Message 'the shared startup deadline terminates a real hanging executable'
    Assert-True -Condition (
        ([DateTimeOffset]::UtcNow - $captureStarted).TotalSeconds -lt 2
    ) -Message 'hanging executable cleanup does not add fixed post-deadline waits'
    $streamCapture = Invoke-ApplicationCapture -Executable $currentPowerShell `
        -Arguments @(
            '-NoLogo',
            '-NoProfile',
            '-Command',
            "[Console]::Out.Write('root'); [Console]::Error.Write('warning')"
        ) -Deadline ([DateTimeOffset]::UtcNow.AddSeconds(5))
    Assert-True -Condition (
        $streamCapture.StdOut -eq 'root' -and $streamCapture.StdErr -eq 'warning'
    ) -Message 'application capture preserves stdout separately from stderr warnings'
    Assert-Throws -Operation {
        Get-RemainingProbeMilliseconds `
            -Deadline ([DateTimeOffset]::UtcNow.AddMilliseconds(10)) -Maximum 3000
    } -Message 'reuse health probing fails as timeout before the 50ms minimum'

    Write-GeneratedConfig -Context $testContext -TemplateInfo $template `
        -Endpoint "http://127.0.0.1:$renderPort/mcp" -Enabled $true
    Assert-Throws -Operation {
        Invoke-StartAction -Context $testContext `
            -ExecutableOverride $missingExecutable -TimeoutSeconds 5
    } -Message 'strict Start returns an error when mcpls is unavailable'
    $strictFailureConfig = [System.IO.File]::ReadAllText($testContext.GeneratedConfigPath)
    Assert-True -Condition ($strictFailureConfig -match '(?m)^enabled = false\r?$') `
        -Message 'strict Start failure leaves the generated config disabled'

    [System.IO.Directory]::CreateDirectory($testContext.StateDirectory) | Out-Null
    [System.IO.File]::WriteAllText(
        $testContext.StatePath,
        '{"schema_version":1}',
        [System.Text.UTF8Encoding]::new($false)
    )
    Assert-Throws -Operation {
        Read-ServiceState -Path $testContext.StatePath
    } -Message 'incomplete state records fail closed instead of appearing absent'
    $invalidStatus = Invoke-StatusAction -Context $testContext
    Assert-True -Condition ($invalidStatus.status -eq 'invalid-state') `
        -Message 'Status reports corrupt state explicitly without process action'

    $currentSnapshot = Get-ProcessSnapshot -ProcessId $PID
    Assert-True -Condition ($null -ne $currentSnapshot) `
        -Message 'current process snapshot is available for identity testing'
    Assert-True -Condition (
        $null -eq (Get-ProcessSnapshot -ProcessId ([int]::MaxValue))
    ) -Message 'a confirmed missing PID remains distinct from inspection failure'
    Set-Item -Path Function:Get-CimInstance -Value { throw 'injected CIM failure' }
    try {
        Assert-Throws -Operation {
            Get-ProcessSnapshot -ProcessId $PID
        } -Message 'CIM failure for a live PID fails closed instead of reporting it dead'
    }
    finally {
        Remove-Item -Path Function:Get-CimInstance -Force
    }
    $fakeState = [pscustomobject]@{
        schema_version = $script:StateSchemaVersion
        worktree_id = $repositoryContext.WorktreeId
        worktree_root = $repositoryRoot
        status = 'ready'
        last_error = $null
        process_id = $PID
        process_started_at_utc = $currentSnapshot.StartedAtUtc.ToString('O')
        executable_path = $currentSnapshot.ExecutablePath
        mcpls_version = $script:McplsVersion
        mcpls_config_path = $repositoryContext.McplsConfigPath
        mcpls_config_sha256 = Get-FileSha256Hex -Path $repositoryContext.McplsConfigPath
        port = $identityPort
        endpoint = "http://127.0.0.1:$identityPort/mcp"
        template_sha256 = $template.Hash
    }
    $stoppedState = (($fakeState | ConvertTo-Json -Depth 6) | ConvertFrom-Json)
    $stoppedState.status = 'stopped'
    $stoppedState.process_id = 0
    Write-AtomicUtf8File -Path $testContext.StatePath `
        -Content (($stoppedState | ConvertTo-Json -Depth 6) + "`n")
    $readStoppedState = Read-ServiceState -Path $testContext.StatePath
    Assert-True -Condition (
        $readStoppedState.status -eq 'stopped' -and
        [int]$readStoppedState.process_id -eq 0
    ) -Message 'stopped state uses zero to represent the absence of an active PID'
    $nullPidState = (($fakeState | ConvertTo-Json -Depth 6) | ConvertFrom-Json)
    $nullPidState.process_id = $null
    Write-AtomicUtf8File -Path $testContext.StatePath `
        -Content (($nullPidState | ConvertTo-Json -Depth 6) + "`n")
    Assert-Throws -Operation {
        Read-ServiceState -Path $testContext.StatePath
    } -Message 'a null required state value is rejected before PID conversion'
    $nullPidStatus = Invoke-StatusAction -Context $testContext
    Assert-True -Condition ($nullPidStatus.status -eq 'invalid-state') `
        -Message 'Status reports null-valued state as invalid'
    $matchingReuseInputs = Get-ServiceReuseInputs -State $fakeState `
        -ExecutablePath $currentSnapshot.ExecutablePath `
        -McplsConfigHash ([string]$fakeState.mcpls_config_sha256)
    $changedReuseInputs = Get-ServiceReuseInputs -State $fakeState `
        -ExecutablePath $currentSnapshot.ExecutablePath `
        -McplsConfigHash ('0' * 64)
    Assert-True -Condition $matchingReuseInputs.Reusable `
        -Message 'matching executable, version, and mcpls.toml hash permit reuse'
    Assert-True -Condition (
        -not $changedReuseInputs.Reusable -and -not $changedReuseInputs.SameConfig
    ) -Message 'a changed mcpls.toml hash forces service replacement'

    $cycleSnapshot = @(
        [pscustomobject]@{ ProcessId = 9001; ParentProcessId = 9002; Name = 'cycle-a' },
        [pscustomobject]@{ ProcessId = 9002; ParentProcessId = 9001; Name = 'cycle-b' }
    )
    $cycleDescendants = @(Get-DescendantProcessesFromSnapshot `
        -Processes $cycleSnapshot -RootProcessId 9001)
    Assert-True -Condition (
        $cycleDescendants.Count -eq 1 -and
        [int]$cycleDescendants[0].ProcessId -eq 9002
    ) -Message 'descendant traversal terminates when a static PID snapshot contains a cycle'
    $identity = Test-ServiceProcessIdentity -State $fakeState -ExpectedRoot $repositoryRoot
    Assert-True -Condition (-not $identity.Matched) `
        -Message 'PID, start time, and executable alone cannot impersonate the service command line'
    Assert-Throws -Operation {
        Stop-VerifiedServiceProcessTree -State $fakeState -ExpectedRoot $repositoryRoot
    } -Message 'verified stop revalidates full process identity on its held handle'
    Assert-True -Condition ($null -ne (Get-Process -Id $PID -ErrorAction SilentlyContinue)) `
        -Message 'verified stop leaves an identity-mismatched PID running'
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

    $originalIdentity = (Get-Command Test-ServiceProcessIdentity).ScriptBlock
    $originalHealth = (Get-Command Test-McpInitialize).ScriptBlock
    $originalVerifiedStop = (Get-Command Stop-VerifiedServiceProcessTree).ScriptBlock
    $originalStopWriteState = (Get-Command Write-ServiceState).ScriptBlock
    $originalDisableConfig = (Get-Command Write-DisabledGeneratedConfig).ScriptBlock
    $originalLifecycleLog = (Get-Command Write-LifecycleLog).ScriptBlock
    $script:StopDisableAttempted = $false
    $script:StopPersistedPid = -1
    try {
        Set-Item -Path Function:Test-ServiceProcessIdentity -Value {
            param($State, $ExpectedRoot, $Deadline)
            [pscustomobject]@{ Matched = $true; Reason = $null; Snapshot = $null }
        }
        Set-Item -Path Function:Test-McpInitialize -Value {
            param($Endpoint, $TimeoutMilliseconds)
            [pscustomobject]@{ Healthy = $true; Reason = $null }
        }
        Set-Item -Path Function:Stop-VerifiedServiceProcessTree -Value {
            param($State, $ExpectedRoot, $TimeoutMilliseconds, $Deadline)
        }
        Set-Item -Path Function:Write-DisabledGeneratedConfig -Value {
            param($Context, $Endpoint)
            $script:StopDisableAttempted = $true
        }
        Set-Item -Path Function:Write-ServiceState -Value {
            param($Context, $State)
            $script:StopPersistedPid = [int]$State.process_id
            throw 'injected stopped-state persistence failure'
        }
        Set-Item -Path Function:Write-LifecycleLog -Value {
            param($Context, $Message)
        }
        Write-AtomicUtf8File -Path $testContext.StatePath `
            -Content (($fakeState | ConvertTo-Json -Depth 6) + "`n")
        $stopFailure = $null
        try {
            Invoke-StopAction -Context $testContext
        }
        catch {
            $stopFailure = $_.Exception.Message
        }
        Assert-True -Condition (-not [string]::IsNullOrWhiteSpace($stopFailure)) `
            -Message 'Stop reports post-termination state persistence failure'
        Assert-True -Condition $script:StopDisableAttempted `
            -Message "Stop still disables generated config when state persistence fails: $stopFailure"
        Assert-True -Condition ($script:StopPersistedPid -eq 0) `
            -Message 'Stop clears the active PID before persisting terminal state'
    }
    finally {
        Set-Item -Path Function:Test-ServiceProcessIdentity -Value $originalIdentity
        Set-Item -Path Function:Test-McpInitialize -Value $originalHealth
        Set-Item -Path Function:Stop-VerifiedServiceProcessTree -Value $originalVerifiedStop
        Set-Item -Path Function:Write-ServiceState -Value $originalStopWriteState
        Set-Item -Path Function:Write-DisabledGeneratedConfig -Value $originalDisableConfig
        Set-Item -Path Function:Write-LifecycleLog -Value $originalLifecycleLog
    }

    $ownershipStateRoot = Join-Path $temporaryRoot 'ownership-state'
    $ownershipDirectory = Join-Path $ownershipStateRoot ('b' * 64)
    [System.IO.Directory]::CreateDirectory($ownershipDirectory) | Out-Null
    Write-AtomicUtf8File -Path (Join-Path $ownershipDirectory 'state.json') `
        -Content (($fakeState | ConvertTo-Json -Depth 6) + "`n")
    $ownershipResult = @(Invoke-PruneStates -AllStateRoot $ownershipStateRoot)
    Assert-True -Condition (
        $ownershipResult.Count -eq 1 -and
        $ownershipResult[0].action -eq 'refused-state-ownership-mismatch'
    ) -Message 'Prune refuses state whose directory, ID, and root hash disagree'
    Assert-True -Condition (Test-Path -LiteralPath $ownershipDirectory -PathType Container) `
        -Message 'ownership-mismatched state remains available for manual review'

    $rotationStateRoot = Join-Path $temporaryRoot 'rotation-state'
    foreach ($name in @(('c' * 64), ('d' * 64), ('e' * 64))) {
        $directory = Join-Path $rotationStateRoot $name
        [System.IO.Directory]::CreateDirectory($directory) | Out-Null
        Write-AtomicUtf8File -Path (Join-Path $directory 'state.json') `
            -Content '{"schema_version":0}'
    }
    $firstRotation = @(Invoke-PruneStates -AllStateRoot $rotationStateRoot -Limit 1)
    $secondRotation = @(Invoke-PruneStates -AllStateRoot $rotationStateRoot -Limit 1)
    Assert-True -Condition (
        $firstRotation.Count -eq 1 -and
        $firstRotation[0].action -eq 'refused-invalid-state'
    ) -Message 'Prune preserves and reports corrupt state'
    Assert-True -Condition (
        $secondRotation.Count -eq 1 -and
        $secondRotation[0].worktree_id -ne $firstRotation[0].worktree_id
    ) -Message 'bounded Prune rotates past an earlier preserved directory'

    $redirectProbe = Start-McplsProcess -Executable (Get-Process -Id $PID).Path `
        -Context $testContext -Port $identityPort
    try {
        $redirectProbe.WaitForExit(5000) | Out-Null
    }
    finally {
        if (-not $redirectProbe.HasExited) {
            $redirectProbe.Kill($true)
            $redirectProbe.WaitForExit(5000) | Out-Null
        }
        $redirectProbe.Dispose()
    }
    Assert-True -Condition (
        (Test-Path -LiteralPath (Join-Path $testContext.StateDirectory 'mcpls.stdout.log') `
            -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $testContext.StateDirectory 'mcpls.stderr.log') `
            -PathType Leaf)
    ) -Message 'long-lived service streams redirect to managed files'

    $originalStartProcess = (Get-Command Start-McplsProcess).ScriptBlock
    $originalPortAvailable = (Get-Command Test-LoopbackPortAvailable).ScriptBlock
    $originalPortListening = (Get-Command Test-LoopbackPortListening).ScriptBlock
    $originalWriteState = (Get-Command Write-ServiceState).ScriptBlock
    $script:StartupTestPid = 0
    $script:StartupTestExecutable = (Get-Process -Id $PID).Path
    try {
        Set-Item -Path Function:Start-McplsProcess -Value {
            param($Executable, $Context, $Port)
            $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
            $startInfo.FileName = $script:StartupTestExecutable
            $startInfo.UseShellExecute = $false
            $startInfo.CreateNoWindow = $true
            $startInfo.ArgumentList.Add('-NoLogo')
            $startInfo.ArgumentList.Add('-NoProfile')
            $startInfo.ArgumentList.Add('-Command')
            $startInfo.ArgumentList.Add('Start-Sleep -Seconds 60')
            $child = [System.Diagnostics.Process]::Start($startInfo)
            $script:StartupTestPid = $child.Id
            return $child
        }
        Set-Item -Path Function:Test-LoopbackPortAvailable -Value { return $true }
        Set-Item -Path Function:Test-LoopbackPortListening -Value { return $true }
        Set-Item -Path Function:Write-ServiceState -Value { throw 'injected state persistence failure' }
        $startupTool = [pscustomobject]@{ Path = $script:StartupTestExecutable }
        Assert-Throws -Operation {
            Start-NewMcplsService -Context $testContext -Tool $startupTool `
                -TemplateInfo $template -McplsConfigHash $secondConfigHash `
                -Deadline ([DateTimeOffset]::UtcNow.AddSeconds(5))
        } -Message 'startup reports a bookkeeping failure instead of orphaning the child'
        Assert-True -Condition (
            $script:StartupTestPid -gt 0 -and
            $null -eq (Get-Process -Id $script:StartupTestPid -ErrorAction SilentlyContinue)
        ) -Message 'startup transaction kills its child when state persistence fails'
    }
    finally {
        Set-Item -Path Function:Start-McplsProcess -Value $originalStartProcess
        Set-Item -Path Function:Test-LoopbackPortAvailable -Value $originalPortAvailable
        Set-Item -Path Function:Test-LoopbackPortListening -Value $originalPortListening
        Set-Item -Path Function:Write-ServiceState -Value $originalWriteState
        if ($script:StartupTestPid -gt 0) {
            Stop-Process -Id $script:StartupTestPid -Force -ErrorAction SilentlyContinue
        }
    }

    $health = Test-McpInitialize -Endpoint 'http://127.0.0.1:1/mcp' `
        -TimeoutMilliseconds 1000
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
