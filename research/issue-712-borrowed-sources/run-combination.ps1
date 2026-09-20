# #712 × #718 组合观察的可复现脚本。
# 固定：#712 栈 head（本分支）、#718 head 6bc440e8268f8d5f74ec7db07290fe4ac826a443、
# 组合提交 e619371eb9e15ae59484bf26e811bb388aebf3e5、观察补丁
# combination-observation.patch。#718 前进时按增量影响判断并重跑适用测试。
param(
    [string]$StackHead = (git rev-parse 712-3-integration-evidence),
    [string]$Head718 = '6bc440e8268f8d5f74ec7db07290fe4ac826a443',
    [string]$Worktree = '../712-preint-repro'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
git worktree add --detach $Worktree $StackHead
try {
    Set-Location -LiteralPath $Worktree
    git merge --no-commit $Head718
    git commit -m "chore: 712-3 与 #718 head $Head718 的临时预集成`n`nRefs: #712"
    git apply (Join-Path $repo 'research/issue-712-borrowed-sources/combination-observation.patch')
    cargo +1.98.0 test --locked -p laneflow-runtime --lib pose_source_observation
    cargo +1.98.0 test --locked -p laneflow-runtime --test pose_source_execution_equivalence --test pose_source_query --test parallel_preview_equivalence --test preview_at_threshold_equivalence
    cargo +1.98.0 test --locked -p laneflow-bevy --lib capacity --test pose_extraction_commit --test pose_extraction_allocation
    Write-Output "combination head: $(git rev-parse HEAD)"
} finally {
    Set-Location -LiteralPath $repo
}
