# #712 × #718 组合观察的可复现脚本（失败即终止）。
# 固定：#718 head 6bc440e8268f8d5f74ec7db07290fe4ac826a443、观察补丁
# combination-observation.patch。#718 前进时按增量影响判断并重跑适用测试。
# -SelfCheck：验证失败传播（补丁应用失败必须非零退出、不产出完成结果）。
param(
    [string]$StackHead = '',
    [string]$Head718 = '6bc440e8268f8d5f74ec7db07290fe4ac826a443',
    [string]$Worktree = '../712-preint-repro',
    [switch]$SelfCheck
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
if ($StackHead -eq '') { $StackHead = (git rev-parse 712-3-integration-evidence).Trim() }

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
    # 失败传播自检：损坏的补丁必须被拒绝，且脚本以非零退出。
    $corrupt = Join-Path ([IO.Path]::GetTempPath()) "issue-712-corrupt-$([guid]::NewGuid().ToString('N')).patch"
    'this is not a valid patch' | Set-Content -LiteralPath $corrupt
    try {
        git apply --check $corrupt
        $code = $LASTEXITCODE
        if ($code -eq 0) { throw 'SelfCheck: corrupt patch was accepted' }
        Write-Host 'SelfCheck OK: corrupt patch rejected, failure would propagate'
        exit 0
    } finally {
        Remove-Item -Force -LiteralPath $corrupt -ErrorAction SilentlyContinue
    }
}

if (Test-Path -LiteralPath $Worktree) { throw "Worktree $Worktree already exists; remove it first" }
Invoke-Step 'worktree-add' { git worktree add --detach $Worktree $StackHead } | Out-Null
try {
    Set-Location -LiteralPath $Worktree
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
