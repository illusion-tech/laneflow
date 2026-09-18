# 拒绝测试：对真实 evidence 的缺陷副本逐一断言 analyze.ps1 拒绝汇总。
# 阳性对照断言未篡改副本可以成功汇总。全部通过输出 OK 并以 0 退出。
param(
    [string]$Evidence = 'research/issue-711-spatial-buffer-swap/evidence'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
$analyze = Join-Path $PSScriptRoot 'analyze.ps1'

function New-Copy([string]$name) {
    $target = Join-Path ([IO.Path]::GetTempPath()) "issue-711-reject-$name-$([guid]::NewGuid().ToString('N'))"
    Copy-Item -Recurse -LiteralPath $Evidence $target
    $script:copies += $target
    $target
}

function Invoke-Analyze([string]$tree) {
    & pwsh -NoProfile -File $analyze -Evidence $tree `
        -Results (Join-Path $tree 'out-results.csv') `
        -SummaryTable (Join-Path $tree 'out-summary.md') *> $null
    $LASTEXITCODE
}

function Invoke-Test([string]$name, [scriptblock]$mutate, [bool]$expectSuccess) {
    $tree = New-Copy $name
    & $mutate $tree
    $code = Invoke-Analyze $tree
    if ($expectSuccess) {
        if ($code -ne 0) { Write-Output "FAIL $name : analyze rejected a valid tree (exit $code)"; return 1 }
    } else {
        if ($code -eq 0) { Write-Output "FAIL $name : analyze accepted defective evidence"; return 1 }
    }
    Write-Output "PASS $name"
    0
}

$script:copies = @()
$failures = 0

# 阳性对照：未篡改的完整 evidence 必须汇总成功。
$failures += Invoke-Test 'positive-control' { param($tree) } $true

# 1 全部 oracle 缺失：A/B oracle map 为空也不能生成“对拍一致”结论。
$failures += Invoke-Test 'all-oracles-missing' {
    param($tree)
    Get-ChildItem -LiteralPath $tree -Recurse -Filter '*.log' | ForEach-Object {
        (Get-Content -LiteralPath $_.FullName | Where-Object { $_ -notlike 'oracle *' }) |
            Set-Content -LiteralPath $_.FullName
    }
} $false

# 2 单轮单场景 oracle 缺失：变体级并集不能补齐单轮完成证明。
$failures += Invoke-Test 'single-run-oracle-missing' {
    param($tree)
    $log = Join-Path $tree 'before/run1/wall.log'
    (Get-Content -LiteralPath $log | Where-Object { $_ -notlike 'oracle cold 10000 *' }) |
        Set-Content -LiteralPath $log
} $false

# 3 重复样本：样本编号 0,0,2,3,4,5,6 且总数不变也要被拒绝。
$failures += Invoke-Test 'duplicate-sample' {
    param($tree)
    $csv = Join-Path $tree 'after/run2/wall.csv'
    Get-Content -LiteralPath $csv | ForEach-Object {
        if ($_ -like 'steady,100000,1,32,*') { $_ -replace '^steady,100000,1,', 'steady,100000,0,' } else { $_ }
    } | Set-Content -LiteralPath $csv
} $false

# 4 缺失样本：行数减少必须被拒绝。
$failures += Invoke-Test 'missing-sample' {
    param($tree)
    $csv = Join-Path $tree 'before/run3/wall.csv'
    (Get-Content -LiteralPath $csv | Where-Object { $_ -notlike 'retry,10000,3,32,*' }) |
        Set-Content -LiteralPath $csv
} $false

# 5 分配 CSV 缺样本：与墙钟同等强度的校验。
$failures += Invoke-Test 'allocation-row-missing' {
    param($tree)
    $csv = Join-Path $tree 'after/run1/allocation.csv'
    (Get-Content -LiteralPath $csv | Where-Object { $_ -notlike 'parking,10000,2,32,*' }) |
        Set-Content -LiteralPath $csv
} $false

# 6 来源信息不匹配：同变体同阶段各轮 manifest 必须一致。
$failures += Invoke-Test 'source-mismatch' {
    param($tree)
    $json = Join-Path $tree 'before/fresh/run2/environment.json'
    $raw = Get-Content -LiteralPath $json -Raw | ConvertFrom-Json
    $raw.manifest = ('0' * 64)
    $raw | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $json -Encoding utf8
} $false

# 7 墙钟构建出现非零分配列：占位零约束。
$failures += Invoke-Test 'wall-allocation-column-nonzero' {
    param($tree)
    $csv = Join-Path $tree 'after/run1/wall.csv'
    Get-Content -LiteralPath $csv | ForEach-Object {
        if ($_ -like 'shrink,1000,4,32,*,0,0,0,0') { $_ -replace ',4,32,(\d+),0,0,0,0$', ",4,32,`$1,5,0,0,0" } else { $_ }
    } | Set-Content -LiteralPath $csv
} $false

foreach ($copy in $script:copies) {
    Remove-Item -Recurse -Force -LiteralPath $copy -ErrorAction SilentlyContinue
}
if ($failures -gt 0) { Write-Output "test-analyze: $failures FAILED"; exit 1 }
Write-Output 'test-analyze: OK'
exit 0
