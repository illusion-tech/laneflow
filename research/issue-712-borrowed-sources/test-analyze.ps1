# 拒绝测试：对真实 evidence 的缺陷副本逐一断言 analyze.ps1 拒绝汇总。
param(
    [string]$Evidence = 'research/issue-712-borrowed-sources/evidence'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
$analyze = Join-Path $PSScriptRoot 'analyze.ps1'

function New-Copy([string]$name) {
    $target = Join-Path ([IO.Path]::GetTempPath()) "issue-712-reject-$name-$([guid]::NewGuid().ToString('N'))"
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
        if ($code -ne 0) { Write-Host "FAIL $name : analyze rejected a valid tree (exit $code)"; return 1 }
    } else {
        if ($code -eq 0) { Write-Host "FAIL $name : analyze accepted defective evidence"; return 1 }
    }
    Write-Host "PASS $name"
    0
}

$script:copies = @()
$failures = 0

$failures += Invoke-Test 'positive-control' { param($tree) } $true

$failures += Invoke-Test 'all-oracles-missing' {
    param($tree)
    Get-ChildItem -LiteralPath $tree -Recurse -Filter '*.log' | ForEach-Object {
        (Get-Content -LiteralPath $_.FullName | Where-Object { $_ -notlike 'oracle *' }) |
            Set-Content -LiteralPath $_.FullName
    }
} $false

$failures += Invoke-Test 'duplicate-sample' {
    param($tree)
    $csv = Join-Path $tree 'after/run1/wall.csv'
    $lines = @(Get-Content -LiteralPath $csv)
    $lines | ForEach-Object {
        if ($_ -like 'source_full,all_active_10000,1,32,*') { $_ -replace ',all_active_10000,1,', ',all_active_10000,0,' } else { $_ }
    } | Set-Content -LiteralPath $csv
} $false

$failures += Invoke-Test 'missing-sample' {
    param($tree)
    $csv = Join-Path $tree 'before/run1/wall.csv'
    (Get-Content -LiteralPath $csv | Where-Object { $_ -notlike 'adapter_full,all_active_10000,3,32,*' }) |
        Set-Content -LiteralPath $csv
} $false

$failures += Invoke-Test 'source-mismatch' {
    param($tree)
    $json = Join-Path $tree 'after/run2/environment.json'
    $raw = Get-Content -LiteralPath $json -Raw | ConvertFrom-Json
    $raw.manifest = ('0' * 64)
    $raw | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $json -Encoding utf8
} $false

$failures += Invoke-Test 'feature-mismatch' {
    param($tree)
    $json = Join-Path $tree 'before/run2/environment.json'
    $raw = Get-Content -LiteralPath $json -Raw | ConvertFrom-Json
    $raw.features = ''
    $raw | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $json -Encoding utf8
} $false

foreach ($copy in $script:copies) {
    Remove-Item -Recurse -Force -LiteralPath $copy -ErrorAction SilentlyContinue
}
if ($failures -gt 0) { Write-Host "test-analyze: $failures FAILED"; exit 1 }
Write-Host 'test-analyze: OK'
exit 0
