param(
    [string]$Evidence = 'research/issue-712-borrowed-sources/evidence',
    [string]$Results = 'research/issue-712-borrowed-sources/results.csv',
    [string]$SummaryTable = 'research/issue-712-borrowed-sources/summary-table.md'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo

$Samples = 7
# 场景矩阵：case × dataset × iterations。
$Datasets = @('all_active_10000', 'all_active_100000', 'mixed_parking_10000', 'mixed_parking_100000', 'high_completed_10000', 'sparse_presentable_10000')
$CaseSpec = @(
    @{ case = 'source_full'; iters = 32; oracle = $true },
    @{ case = 'adapter_full'; iters = 32; oracle = $true },
    @{ case = 'transform_convert'; iters = 32; oracle = $false },
    @{ case = 'fresh_output'; iters = 32; oracle = $false },
    @{ case = 'alternate'; iters = 32; oracle = $true }
)
$MinRuns = 3
# A 侧必须以 legacy-source 构建，B 侧必须默认 feature：这是两侧仅有的
# 声明差异（薄适配层），manifest/lock/src 必须全等。
$ExpectedFeature = @{ before = 'legacy-source'; after = '' }

function Get-Median([double[]]$values) {
    $sorted = @($values | Sort-Object)
    $count = $sorted.Count
    if ($count % 2 -eq 1) {
        [double]$sorted[[int](($count - 1) / 2)]
    } else {
        # 偶数个样本取两个中位值的算术平均，不偏向单侧。
        ([double]$sorted[$count / 2 - 1] + [double]$sorted[$count / 2]) / 2.0
    }
}

function Read-CsvRows([string]$path) {
    @(Get-Content -LiteralPath $path | Where-Object { $_ -and $_ -notmatch '^case,' } | ForEach-Object {
        $parts = $_ -split ','
        if ($parts.Count -ne 9) { throw "truncated CSV row (expected 9 fields, got $($parts.Count)): $_" }
        $numeric = @{}
        foreach ($index in 4..8) {
            $raw = $parts[$index]
            $value = 0.0
            if ($raw -eq '' -or -not [double]::TryParse($raw, [ref]$value) -or
                [double]::IsNaN($value) -or [double]::IsInfinity($value) -or $value -lt 0) {
                throw "invalid numeric field at column ${index}: '$raw' in row: $_"
            }
            $numeric[$index] = $value
        }
        [ordered]@{
            case = $parts[0]
            dataset = $parts[1]
            sample = [int]$parts[2]
            iterations = [int]$parts[3]
            ns = $numeric[4]
            allocations = $numeric[5]
            reallocations = $numeric[6]
        }
    })
}

function Read-Oracles([string]$path) {
    @(Get-Content -LiteralPath $path | Where-Object { $_ -like 'oracle *' } | ForEach-Object {
        $parts = $_ -split ' '
        if ($parts.Count -ne 4 -or $parts[3] -notmatch '^[0-9a-f]{64}$') {
            throw "invalid oracle line (expected 'oracle <case> <dataset> <64-hex>'): $_"
        }
        @{ key = "$($parts[1]) $($parts[2])"; digest = $parts[3] }
    })
}

function Read-Env([string]$path) {
    $raw = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    foreach ($field in @('baseline', 'baselineTree', 'manifest', 'lockfile', 'sources', 'rustc', 'cargo', 'binaries', 'os', 'cpu', 'logicalProcessors', 'features', 'time')) {
        if (-not $raw.PSObject.Properties[$field]) { throw "environment.json misses field ${field}: $path" }
    }
    $sourceMap = @{}
    foreach ($entry in $raw.sources) {
        $sourceMap[[IO.Path]::GetFileName($entry.Path)] = $entry.Hash
    }
    if (-not $sourceMap.ContainsKey('main.rs')) { throw "environment.json sources must cover main.rs: $path" }
    if (@($raw.binaries).Count -lt 2) { throw "environment.json must record both binaries: $path" }
    @{
        baseline = [string]$raw.baseline
        manifest = [string]$raw.manifest
        lockfile = [string]$raw.lockfile
        sources = $sourceMap
        rustc = ([string]$raw.rustc -replace "`r`n", "`n")
        cargo = [string]$raw.cargo
        os = [string]$raw.os
        cpu = [string]$raw.cpu
        logicalProcessors = [int]$raw.logicalProcessors
        features = [string]$raw.features
        powerScheme = if ($raw.PSObject.Properties['powerScheme']) { [string]$raw.powerScheme } else { '' }
    }
}

function Compare-EnvIdentity($a, $b, [string]$context) {
    foreach ($field in @('rustc', 'cargo', 'os', 'cpu', 'logicalProcessors', 'powerScheme')) {
        if ([string]$a[$field] -ne [string]$b[$field]) {
            throw "$context environment '$field' differs"
        }
    }
}

# 完整预期测量矩阵：23 项（6 数据集 × 3 指标 + 2 规模 × 2 生命周期 + cold）。
# 每轮墙钟与分配数据都必须逐项存在；样本编号恰为预期集合、无重复、
# iterations 与口径一致、数值合法。
$ExpectedMatrix = @()
foreach ($dataset in $Datasets) {
    foreach ($spec in @(
        @{ case = 'source_full'; iters = 32 },
        @{ case = 'adapter_full'; iters = 32 },
        @{ case = 'transform_convert'; iters = 32 }
    )) {
        $ExpectedMatrix += @{ case = $spec.case; dataset = $dataset; iters = $spec.iters }
    }
}
foreach ($dataset in @('all_active_10000', 'all_active_100000')) {
    foreach ($spec in @(
        @{ case = 'fresh_output'; iters = 32 },
        @{ case = 'alternate'; iters = 32 }
    )) {
        $ExpectedMatrix += @{ case = $spec.case; dataset = $dataset; iters = $spec.iters }
    }
}
$ExpectedMatrix += @{ case = 'cold'; dataset = 'cold_probe'; iters = 1 }

function Test-ExpectedIds([int[]]$ids, [string]$context) {
    $unique = @($ids | Sort-Object -Unique)
    if ($ids.Count -ne $Samples) { throw "$context expected $Samples rows, got $($ids.Count)" }
    if ($unique.Count -ne $Samples) { throw "$context duplicate sample ids" }
    foreach ($id in 0..($Samples - 1)) {
        if ($unique -notcontains $id) { throw "$context sample id set must be exactly 0..$($Samples - 1)" }
    }
}

function Test-FullMatrix($rows, [string]$context) {
    foreach ($item in $ExpectedMatrix) {
        $matched = @($rows | Where-Object {
            $_.case -eq $item.case -and [string]$_.dataset -eq $item.dataset
        })
        if ($matched.Count -eq 0) { throw "$context misses pair '$($item.case)/$($item.dataset)'" }
        Test-ExpectedIds @($matched | ForEach-Object { [int]$_.sample }) "$context $($item.case)/$($item.dataset)"
        foreach ($row in $matched) {
            if ($row.iterations -ne $item.iters) {
                throw "$context $($item.case)/$($item.dataset) iterations $($row.iterations) != $($item.iters)"
            }
            if (-not ($row.ns -gt 0)) { throw "$context $($item.case)/$($item.dataset) ns must be positive" }
        }
    }
    # 不允许矩阵之外的行。
    foreach ($row in $rows) {
        $known = @($ExpectedMatrix | Where-Object {
            $_.case -eq $row.case -and $_.dataset -eq [string]$row.dataset
        })
        if ($known.Count -eq 0) { throw "$context unexpected pair '$($row.case)/$($row.dataset)'" }
    }
}

# 正确性层：每份日志必须出现完整的预期 oracle 键集合，不多不少、无重复。
$ExpectedOracleKeys = @(
    'source all_active_10000', 'source all_active_100000',
    'source mixed_parking_10000', 'source mixed_parking_100000',
    'source high_completed_10000', 'source sparse_presentable_10000',
    'adapter all_active_10000', 'adapter all_active_100000',
    'adapter mixed_parking_10000', 'adapter mixed_parking_100000',
    'adapter high_completed_10000', 'adapter sparse_presentable_10000',
    'alternate all_active_10000', 'alternate all_active_100000',
    'cold cold'
)

function Test-OracleLayer([string]$logPath, [string]$context) {
    $oracles = Read-Oracles $logPath
    $seen = @($oracles | ForEach-Object { $_.key })
    foreach ($key in $ExpectedOracleKeys) {
        if ($seen -notcontains $key) { throw "$context oracle log misses key '$key'" }
    }
    foreach ($key in $seen) {
        if ($ExpectedOracleKeys -notcontains $key) { throw "$context oracle log has unexpected key '$key'" }
    }
    if ($seen.Count -ne $ExpectedOracleKeys.Count) { throw "$context oracle log has duplicate keys" }
    $map = @{}
    foreach ($oracle in $oracles) { $map[$oracle.key] = $oracle.digest }
    $map
}

function Import-Variant([string]$root, [string]$name) {
    $runs = @(Get-ChildItem -LiteralPath $root -Directory -Filter 'run*' | Sort-Object Name)
    if ($runs.Count -lt $MinRuns) { throw "$name needs at least $MinRuns runs" }
    $imported = @()
    foreach ($run in $runs) {
        foreach ($file in @('wall.csv', 'wall.log', 'allocation.csv', 'allocation.log', 'environment.json')) {
            if (-not (Test-Path -LiteralPath (Join-Path $run.FullName $file))) { throw "Missing evidence file: $run/$file" }
        }
        $wallLog = Get-Content -LiteralPath (Join-Path $run.FullName 'wall.log') -Raw
        if ($wallLog -notmatch 'allocation=False') { throw "$($run.Name) wall.log must be a non-allocation build" }
        $allocLog = Get-Content -LiteralPath (Join-Path $run.FullName 'allocation.log') -Raw
        if ($allocLog -notmatch 'allocation=True') { throw "$($run.Name) allocation.log must be an allocation build" }
        $env = Read-Env (Join-Path $run.FullName 'environment.json')
        if ($env.features -ne $ExpectedFeature[$name]) {
            throw "$name/$($run.Name) features '$($env.features)' != '$($ExpectedFeature[$name])'"
        }
        $wallOracles = Test-OracleLayer (Join-Path $run.FullName 'wall.log') "$name/$($run.Name) wall"
        $allocOracles = Test-OracleLayer (Join-Path $run.FullName 'allocation.log') "$name/$($run.Name) allocation"
        foreach ($key in $wallOracles.Keys) {
            if ($wallOracles[$key] -ne $allocOracles[$key]) {
                throw "$name/$($run.Name) oracle digest differs between wall and allocation logs for '$key'"
            }
        }
        $imported += @{ path = $run.FullName; env = $env }
    }
    $reference = $imported[0]
    foreach ($entry in $imported) {
        if ($entry.env.manifest -ne $reference.env.manifest -or $entry.env.lockfile -ne $reference.env.lockfile) {
            throw "$name manifest or lockfile differ across runs"
        }
        if (Compare-Object @($entry.env.sources.GetEnumerator()) @($reference.env.sources.GetEnumerator())) {
            throw "$name measurement sources differ across runs"
        }
        if ($entry.env.baseline -ne $reference.env.baseline) { throw "$name baseline differs across runs" }
        Compare-EnvIdentity $entry.env $reference.env "$name/$($entry.path)"
    }
    if ($reference.env.manifest -ne $null) { }
    $imported
}

$before = Import-Variant (Join-Path $Evidence 'before') 'before'
$after = Import-Variant (Join-Path $Evidence 'after') 'after'

# A/B 程序身份：manifest/lockfile/src 必须全等（feature 是声明的构建差异）。
$refBefore = $before[0].env
$refAfter = $after[0].env
if ($refBefore.manifest -ne $refAfter.manifest -or $refBefore.lockfile -ne $refAfter.lockfile) {
    throw 'A/B manifest or lockfile differ'
}
if (Compare-Object @($refBefore.sources.GetEnumerator()) @($refAfter.sources.GetEnumerator())) {
    throw 'A/B measurement sources differ'
}
Compare-EnvIdentity $refBefore $refAfter 'A/B'

# 逐轮校验行、oracle 与聚合。
$valueByPair = @{}
$allocByPair = @{}
$reallocByPair = @{}
$oracleBefore = @{}
$oracleAfter = @{}
foreach ($variant in @(@{ name = 'before'; runs = $before }, @{ name = 'after'; runs = $after })) {
    foreach ($run in $variant.runs) {
        $wallRows = Read-CsvRows (Join-Path $run.path 'wall.csv')
        $allocRows = Read-CsvRows (Join-Path $run.path 'allocation.csv')
        Test-FullMatrix $wallRows "$($variant.name)/$($run.path) wall"
        Test-FullMatrix $allocRows "$($variant.name)/$($run.path) allocation"
        foreach ($row in $wallRows) {
            if ($row.allocations -ne 0 -or $row.reallocations -ne 0) { throw 'wall rows must keep zero allocation columns' }
        }
        foreach ($spec in $CaseSpec) {
            $wallCases = @($wallRows | Where-Object { $_.case -eq $spec.case })
            $allocCases = @($allocRows | Where-Object { $_.case -eq $spec.case })
            $datasetNames = @($wallCases | ForEach-Object { [string]$_.dataset } | Sort-Object -Unique)
            foreach ($name in $datasetNames) {
                $pair = "$($spec.case) $name"
                $datasetRows = @($wallCases | Where-Object { [string]$_.dataset -eq $name })
                $medians = @($datasetRows | ForEach-Object { $_.ns / $_.iterations })
                if (-not $valueByPair.ContainsKey("$($variant.name)|$pair")) { $valueByPair["$($variant.name)|$pair"] = @() }
                $valueByPair["$($variant.name)|$pair"] += Get-Median $medians
                $allocGroup = @($allocCases | Where-Object { [string]$_.dataset -eq $name })
                $allocMedians = @($allocGroup | ForEach-Object { $_.allocations / $_.iterations })
                if ($allocMedians.Count -gt 0) {
                    if (-not $allocByPair.ContainsKey("$($variant.name)|$pair")) { $allocByPair["$($variant.name)|$pair"] = @() }
                    $allocByPair["$($variant.name)|$pair"] += Get-Median $allocMedians
                }
                $reallocMedians = @($allocGroup | ForEach-Object { $_.reallocations / $_.iterations })
                if ($reallocMedians.Count -gt 0) {
                    if (-not $reallocByPair.ContainsKey("$($variant.name)|$pair")) { $reallocByPair["$($variant.name)|$pair"] = @() }
                    $reallocByPair["$($variant.name)|$pair"] += Get-Median $reallocMedians
                }
            }
        }
        $coldMedians = @(($wallRows | Where-Object { $_.case -eq 'cold' }) | ForEach-Object { $_.ns })
        if (-not $valueByPair.ContainsKey("$($variant.name)|cold cold_probe")) { $valueByPair["$($variant.name)|cold cold_probe"] = @() }
        $valueByPair["$($variant.name)|cold cold_probe"] += Get-Median $coldMedians
        $coldAlloc = @(($allocRows | Where-Object { $_.case -eq 'cold' }) | ForEach-Object { $_.allocations })
        if (-not $allocByPair.ContainsKey("$($variant.name)|cold cold_probe")) { $allocByPair["$($variant.name)|cold cold_probe"] = @() }
        $allocByPair["$($variant.name)|cold cold_probe"] += Get-Median $coldAlloc

        $coldRealloc = @(($allocRows | Where-Object { $_.case -eq 'cold' }) | ForEach-Object { $_.reallocations })
        if (-not $reallocByPair.ContainsKey("$($variant.name)|cold cold_probe")) { $reallocByPair["$($variant.name)|cold cold_probe"] = @() }
        $reallocByPair["$($variant.name)|cold cold_probe"] += Get-Median $coldRealloc

        foreach ($oracle in (Read-Oracles (Join-Path $run.path 'wall.log')) + (Read-Oracles (Join-Path $run.path 'allocation.log'))) {
            $map = if ($variant.name -eq 'before') { $oracleBefore } else { $oracleAfter }
            if ($map.ContainsKey($oracle.key) -and $map[$oracle.key] -ne $oracle.digest) {
                throw "$($variant.name) oracle digest differs across runs for '$($oracle.key)'"
            }
            $map[$oracle.key] = $oracle.digest
        }
    }
}

# A/B 对拍：全部 oracle 摘要逐键一致。
foreach ($key in $oracleBefore.Keys) {
    if (-not $oracleAfter.ContainsKey($key)) { throw "after misses oracle '$key'" }
    if ($oracleBefore[$key] -ne $oracleAfter[$key]) { throw "A/B oracle digest mismatch for '${key}'" }
}
foreach ($key in $oracleAfter.Keys) {
    if (-not $oracleBefore.ContainsKey($key)) { throw "before misses oracle '$key'" }
}

$lines = @('case,dataset,before_ns,after_ns,delta_pct,before_alloc,after_alloc,before_realloc,after_realloc')
$table = @('| 场景 | 数据集 | before µs | after µs | 差值 |', '| --- | --- | ---: | ---: | ---: |')
$allPairs = @($valueByPair.Keys | ForEach-Object { $_ -replace '^[^|]+\|', '' } | Sort-Object -Unique)
foreach ($pair in $allPairs) {
    $beforeMedian = Get-Median ([double[]]$valueByPair["before|$pair"])
    $afterMedian = Get-Median ([double[]]$valueByPair["after|$pair"])
    if (-not $afterMedian -and $afterMedian -ne 0) { throw "after misses pair '$pair'" }
    $delta = if ($beforeMedian -gt 0) { [Math]::Round((($afterMedian - $beforeMedian) / $beforeMedian) * 100, 3) } else { 0 }
    $beforeAlloc = if ($allocByPair.ContainsKey("before|$pair")) { Get-Median ([double[]]$allocByPair["before|$pair"]) } else { '' }
    $afterAlloc = if ($allocByPair.ContainsKey("after|$pair")) { Get-Median ([double[]]$allocByPair["after|$pair"]) } else { '' }
    $beforeRealloc = if ($reallocByPair.ContainsKey("before|$pair")) { Get-Median ([double[]]$reallocByPair["before|$pair"]) } else { '' }
    $afterRealloc = if ($reallocByPair.ContainsKey("after|$pair")) { Get-Median ([double[]]$reallocByPair["after|$pair"]) } else { '' }
    $parts = $pair -split ' ', 2
    $lines += "$($parts[0]),$($parts[1]),$beforeMedian,$afterMedian,$delta,$beforeAlloc,$afterAlloc,$beforeRealloc,$afterRealloc"
    $table += "| $($parts[0]) | $($parts[1]) | $([Math]::Round($beforeMedian / 1000, 3)) | $([Math]::Round($afterMedian / 1000, 3)) | $($delta)% |"
}
Set-Content -LiteralPath $Results -Value $lines -Encoding utf8

$env = $refBefore
$oracleNote = "oracle 摘要 A/B 逐键一致，共 $($oracleBefore.Count) 组：$((@($oracleBefore.Keys) | Sort-Object) -join '; ')"
$summary = @(
    '',
    '## oracle 对拍',
    '',
    $oracleNote,
    '',
    '## 测量环境（由各 run 的 environment.json 汇总）',
    '',
    '| 项 | 值 |',
    '| --- | --- |',
    "| CPU | $($env.cpu)（$($env.logicalProcessors) logical processors） |",
    "| OS | $($env.os) |",
    "| 工具链（取证记录） | $((($env.rustc -split "`n") | Select-Object -First 1))；$($env.cargo) |",
    "| 电源方案 | $(if ($env.powerScheme.Trim() -eq '') { '未记录（取证失败）' } else { $env.powerScheme.Trim() }) |",
    "| before 生产基线 | $($refBefore.baseline)（features=legacy-source） |",
    "| after 生产基线 | $($refAfter.baseline)（features=默认） |",
    "| 测量程序 main.rs（A/B 相同） | $($env.sources['main.rs']) |"
)
Set-Content -LiteralPath $SummaryTable -Value ($table + $summary) -Encoding utf8
Write-Output $Results
