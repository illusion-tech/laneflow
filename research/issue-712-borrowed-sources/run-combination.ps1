# #712 × #718 组合观察的可复现脚本（失败即终止）。
# 固定：#718 head 6bc440e8268f8d5f74ec7db07290fe4ac826a443、观察补丁
# combination-observation.patch。#718 前进时按增量影响判断并重跑适用测试。
# -SelfCheck：验证失败传播（补丁应用失败必须非零退出、不产出完成结果）。
param(
    # 默认经 PR ref 解析第三层交付 head：GitHub 永久保留 refs/pull/<n>/head，
    # 干净克隆可 fetch；且不受 Merge Queue rebase 改写提交 SHA 的影响。
    [string]$StackHead = '',
    [string]$StackPullRef = 'refs/pull/733/head',
    [string]$Head718 = '6bc440e8268f8d5f74ec7db07290fe4ac826a443',
    [string]$Worktree = '../712-preint-repro',
    [switch]$SelfCheck
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo


# 外部命令失败即终止；过滤执行的测试还须证明确实运行了目标测试。
function Invoke-Step([string]$name, [scriptblock]$action, [int]$expectExit = 0) {
    $output = @(& $action 2>&1)
    $code = $LASTEXITCODE
    $output | Write-Host
    if ($code -ne $expectExit) {
        throw "step '$name' exited $code (expected $expectExit)"
    }
    return $output
}

function Invoke-TestFilter([string]$name, [string[]]$cargoArgs, [string]$filter, [int]$minPassed) {
    $output = Invoke-Step $name { cargo +1.98.0 test --locked @cargoArgs -- $filter --nocapture } 0
    $matched = @($output | Select-String -Pattern 'test result: ok\. (\d+) passed')
    if ($matched.Count -eq 0) { throw "step '$name': no test-result lines found" }
    $totalPassed = ($matched | ForEach-Object { [int]$_.Matches[0].Groups[1].Value } | Measure-Object -Sum).Sum
    if ($totalPassed -lt $minPassed) {
        throw "step '$name': expected at least $minPassed passed, got $totalPassed"
    }
    Write-Host "step '$name': $totalPassed tests passed"
}

if ($SelfCheck) {
    # 失败传播自检：损坏补丁必须穿过 Invoke-Step 的退出码检查并抛出异常。
    # 测试器（本脚本）确认预期异常后返回 0；被测包装器确实传播了失败。
    $corrupt = Join-Path ([IO.Path]::GetTempPath()) "issue-712-corrupt-$([guid]::NewGuid().ToString('N')).patch"
    'this is not a valid patch' | Set-Content -LiteralPath $corrupt
    try {
        $propagated = $false
        try {
            # 期望成功（退出 0）：坏补丁被意外接受时同样必须异常退出，
            # 而不是只依赖“预期非零”的单向校验。
            Invoke-Step 'selfcheck-corrupt-patch' { git apply --check $corrupt } 0 | Out-Null
        } catch {
            $propagated = $true
            Write-Host "SelfCheck OK: failure propagated through Invoke-Step: $($_.Exception.Message)"
        }
        if (-not $propagated) {
            throw 'SelfCheck: Invoke-Step did not propagate the corrupt-patch failure'
        }
        exit 0
    } finally {
        Remove-Item -Force -LiteralPath $corrupt -ErrorAction SilentlyContinue
    }
}

if ($StackHead -eq '') {
    Invoke-Step 'fetch-stack-pull-ref' { git fetch -q origin $StackPullRef } | Out-Null
    $StackHead = (git rev-parse FETCH_HEAD).Trim()
    Write-Host "stack head resolved from ${StackPullRef}: $StackHead"
}

if (Test-Path -LiteralPath $Worktree) { throw "Worktree $Worktree already exists; remove it first" }
Invoke-Step 'worktree-add' { git worktree add --detach $Worktree $StackHead } | Out-Null
try {
    Set-Location -LiteralPath $Worktree
    # 经 PR ref 取回 #718（分支删除后仍可达）并核对实际 head 与固定 SHA 一致。
    Invoke-Step 'fetch-718' { git fetch -q origin refs/pull/718/head } | Out-Null
    $fetched718 = (git rev-parse FETCH_HEAD).Trim()
    if ($fetched718 -ne $Head718) {
        throw "PR #718 head moved: fetched $fetched718, expected $Head718"
    }
    Invoke-Step 'merge-718' { git merge --no-commit $Head718 } | Out-Null
    Invoke-Step 'commit-merge' { git commit -m "chore: 712-3 与 #718 head $Head718 的临时预集成`n`nRefs: #712" } | Out-Null
    Invoke-Step 'apply-observation-patch' {
        git apply (Join-Path $repo 'research/issue-712-borrowed-sources/combination-observation.patch')
    } | Out-Null

    Invoke-TestFilter 'runtime-observation' @('-p', 'laneflow-runtime', '--lib') 'pose_source_observation' 2
    Invoke-TestFilter 'runtime-combination' @('-p', 'laneflow-runtime', '--test', 'pose_source_execution_equivalence', '--test', 'pose_source_query') '' 12
    Invoke-TestFilter 'runtime-parallel' @('-p', 'laneflow-runtime', '--test', 'parallel_preview_equivalence', '--test', 'preview_at_threshold_equivalence') '' 8
    Invoke-TestFilter 'adapter-capacity' @('-p', 'laneflow-bevy', '--lib') 'capacity' 5
    Invoke-TestFilter 'adapter-extraction' @('-p', 'laneflow-bevy', '--test', 'pose_extraction_commit', '--test', 'pose_extraction_allocation') '' 5

    Write-Host "combination reproducible head: $(git rev-parse HEAD)"
} finally {
    Set-Location -LiteralPath $repo
    git worktree remove --force $Worktree
}
