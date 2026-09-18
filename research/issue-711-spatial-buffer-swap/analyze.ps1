param(
    [string]$Evidence = 'research/issue-711-spatial-buffer-swap/evidence',
    [string]$Results = 'research/issue-711-spatial-buffer-swap/results.csv'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo

$Samples = 7
$ExpectedCases = @(
    @{ case = 'steady'; records = 0 },
    @{ case = 'steady'; records = 1 },
    @{ case = 'steady'; records = 1000 },
    @{ case = 'steady'; records = 10000 },
    @{ case = 'steady'; records = 100000 },
    @{ case = 'cold'; records = 10000 },
    @{ case = 'grow'; records = 100000 },
    @{ case = 'shrink'; records = 1000 },
    @{ case = 'alternate'; records = 10000 },
    @{ case = 'fresh_output'; records = 10000 },
    @{ case = 'fail_last'; records = 10000 },
    @{ case = 'fail_last'; records = 100000 },
    @{ case = 'retry'; records = 10000 },
    @{ case = 'retry'; records = 100000 },
    @{ case = 'parking'; records = 10000 },
    @{ case = 'parking'; records = 100000 },
    @{ case = 'retained_build'; records = 10000; samples = 1 },
    @{ case = 'retained_build'; records = 100000; samples = 1 }
)

function Get-Median([double[]]$values) {
    $sorted = $values | Sort-Object
    [double]$sorted[[int][Math]::Floor($sorted.Count / 2)]
}

function Read-CsvRows([string]$path) {
    @(Get-Content -LiteralPath $path | Where-Object { $_ -and $_ -notmatch '^case,' } | ForEach-Object {
        $parts = $_ -split ','
        [ordered]@{
            case = $parts[0]
            records = [int]$parts[1]
            sample = [int]$parts[2]
            iterations = [int]$parts[3]
            ns = [double]$parts[4]
            allocations = [double]$parts[5]
            reallocations = [double]$parts[6]
            allocatedBytes = [double]$parts[7]
            reallocatedBytes = [double]$parts[8]
        }
    })
}

function Read-Oracles([string]$path) {
    @(Get-Content -LiteralPath $path | Where-Object { $_ -like 'oracle *' } | ForEach-Object {
        $parts = $_ -split ' '
        @{ key = "$($parts[1]) $($parts[2])"; digest = $parts[3] }
    })
}

# 校验一个变体的全部取证目录，返回按 case+records 聚合的每调用纳秒中位数。
function Import-Variant([string]$root, [string]$name) {
    $runs = @(Get-ChildItem -LiteralPath $root -Directory | Sort-Object Name)
    if ($runs.Count -lt 3) { throw "$name needs at least 3 runs" }
    $oracleMap = @{}
    $perCall = @{}
    $allocPerCall = @{}
    foreach ($run in $runs) {
        $wallCsv = Join-Path $run.FullName 'wall.csv'
        $wallLog = Join-Path $run.FullName 'wall.log'
        $allocCsv = Join-Path $run.FullName 'allocation.csv'
        $allocLog = Join-Path $run.FullName 'allocation.log'
        foreach ($path in @($wallCsv, $wallLog, $allocCsv, $allocLog, (Join-Path $run.FullName 'environment.json'))) {
            if (-not (Test-Path -LiteralPath $path)) { throw "Missing evidence file: $path" }
        }
        if ((Get-Content -LiteralPath $wallLog -Raw) -notmatch 'allocation=False|allocation=false') {
            throw "Wall log of $run must be a non-allocation build"
        }
        if ((Get-Content -LiteralPath $allocLog -Raw) -notmatch 'allocation=True|allocation=true') {
            throw "Allocation log of $run must be an allocation build"
        }
        $wallRows = Read-CsvRows $wallCsv
        foreach ($expected in $ExpectedCases) {
            $key = "$($expected.case) $($expected.records)"
            $expectedSamples = if ($expected.samples) { $expected.samples } else { $Samples }
            $rows = @($wallRows | Where-Object { $_.case -eq $expected.case -and $_.records -eq $expected.records })
            if ($rows.Count -ne $expectedSamples) {
                throw "$name $($run.Name) $key expected $expectedSamples samples, got $($rows.Count)"
            }
            $medians = @($rows | ForEach-Object { $_.ns / $_.iterations })
            $runMedian = Get-Median $medians
            if (-not $perCall.ContainsKey($key)) { $perCall[$key] = @() }
            $perCall[$key] += $runMedian
        }
        foreach ($oracle in (Read-Oracles $wallLog) + (Read-Oracles $allocLog)) {
            if ($oracleMap.ContainsKey($oracle.key) -and $oracleMap[$oracle.key] -ne $oracle.digest) {
                throw "Oracle digest mismatch within $name : $($oracle.key)"
            }
            $oracleMap[$oracle.key] = $oracle.digest
        }
        $allocRows = Read-CsvRows $allocCsv
        foreach ($group in ($allocRows | Group-Object { "$($_.case) $($_.records)" })) {
            $medians = @($group.Group | ForEach-Object { $_.allocations / $_.iterations })
            $allocPerCall[$group.Name] = Get-Median $medians
        }
    }
    @{ name = $name; perCall = $perCall; oracles = $oracleMap; alloc = $allocPerCall }
}

$before = Import-Variant (Join-Path $Evidence 'before') 'before'
$after = Import-Variant (Join-Path $Evidence 'after') 'after'

# A/B 对拍：所有 oracle 摘要必须逐 case 一致。
foreach ($key in $before.oracles.Keys) {
    if (-not $after.oracles.ContainsKey($key)) { throw "After runs miss oracle $key" }
    if ($before.oracles[$key] -ne $after.oracles[$key]) {
        throw "A/B oracle digest mismatch for ${key}: $($before.oracles[$key]) vs $($after.oracles[$key])"
    }
}
foreach ($key in $after.oracles.Keys) {
    if (-not $before.oracles.ContainsKey($key)) { throw "Before runs miss oracle $key" }
}

$lines = @('case,records,before_ns_per_call,after_ns_per_call,delta_pct,before_alloc_per_call,after_alloc_per_call')
foreach ($expected in $ExpectedCases) {
    $key = "$($expected.case) $($expected.records)"
    $beforeMedian = Get-Median ([double[]]$before.perCall[$key])
    $afterMedian = Get-Median ([double[]]$after.perCall[$key])
    $delta = if ($beforeMedian -gt 0) { [Math]::Round((($afterMedian - $beforeMedian) / $beforeMedian) * 100, 3) } else { 0 }
    $beforeAlloc = if ($before.alloc.ContainsKey($key)) { $before.alloc[$key] } else { '' }
    $afterAlloc = if ($after.alloc.ContainsKey($key)) { $after.alloc[$key] } else { '' }
    $lines += "$($expected.case),$($expected.records),$beforeMedian,$afterMedian,$delta,$beforeAlloc,$afterAlloc"
}
$lines += "# oracle digests identical across before/after for: $((@($before.oracles.Keys) | Sort-Object) -join '; ')"
Set-Content -LiteralPath $Results -Value $lines -Encoding utf8
Write-Output $Results
