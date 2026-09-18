param(
    [string]$Evidence = 'research/issue-711-spatial-buffer-swap/evidence',
    [string]$Results = 'research/issue-711-spatial-buffer-swap/results.csv',
    [string]$SummaryTable = 'research/issue-711-spatial-buffer-swap/summary-table.md'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo

# 场景矩阵。unit：call=每行数值按单次完整 extract_pose_batch 调用归一；
# sequence=一次计时区间为 1k→10k→100k 三次调用的完整增长序列，不除以调用次数。
# oracle=该场景在每份日志中都必须出现的完整批次摘要。
$FullCases = @(
    @{ case = 'steady'; records = 0; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'steady'; records = 1; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'steady'; records = 1000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'steady'; records = 10000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'steady'; records = 100000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'cold'; records = 10000; iters = 1; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'grow'; records = 100000; iters = 1; samples = 7; unit = 'sequence'; oracle = $true },
    @{ case = 'shrink'; records = 1000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'alternate'; records = 10000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    # 完整取证中的 fresh_output 计时边界含完整比较（后续已修复），只校验存在性；
    # 汇总一律取 fresh 阶段重采数据。
    @{ case = 'fresh_output'; records = 10000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'fail_last'; records = 10000; iters = 32; samples = 7; unit = 'call'; oracle = $false },
    @{ case = 'fail_last'; records = 100000; iters = 32; samples = 7; unit = 'call'; oracle = $false },
    @{ case = 'retry'; records = 10000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'retry'; records = 100000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'parking'; records = 10000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'parking'; records = 100000; iters = 32; samples = 7; unit = 'call'; oracle = $true },
    @{ case = 'retained_build'; records = 10000; iters = 3; samples = 1; unit = 'call'; oracle = $false },
    @{ case = 'retained_build'; records = 100000; iters = 3; samples = 1; unit = 'call'; oracle = $false }
)
# fresh 阶段：--only fresh_output 定点重采（完整比较移出计时区间后的口径）。
$FreshCases = @(
    @{ case = 'fresh_output'; records = 10000; iters = 32; samples = 7; unit = 'call'; oracle = $true }
)
$MinRuns = 3

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

function Read-Env([string]$path) {
    $raw = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    foreach ($field in @('baseline', 'baselineTree', 'manifest', 'lockfile', 'sources', 'rustc', 'cargo', 'binaries', 'os', 'cpu', 'time')) {
        if (-not $raw.PSObject.Properties[$field]) { throw "environment.json misses field ${field}: $path" }
    }
    $sourceMap = @{}
    foreach ($entry in $raw.sources) {
        $sourceMap[[IO.Path]::GetFileName($entry.Path)] = $entry.Hash
    }
    if (-not $sourceMap.ContainsKey('main.rs')) { throw "environment.json sources must cover main.rs: $path" }
    if (@($raw.binaries).Count -lt 2) { throw "environment.json must record both binaries: $path" }
    $filter = ''
    if ($raw.PSObject.Properties['caseFilter'] -and $null -ne $raw.caseFilter) { $filter = [string]$raw.caseFilter }
    @{
        baseline = [string]$raw.baseline
        tree = [string]$raw.baselineTree
        manifest = [string]$raw.manifest
        lockfile = [string]$raw.lockfile
        sources = $sourceMap
        rustc = (($raw.rustc -split "`r?`n") | Select-Object -First 1)
        cargo = [string]$raw.cargo
        caseFilter = $filter
        os = [string]$raw.os
        cpu = [string]$raw.cpu
        powerScheme = if ($raw.PSObject.Properties['powerScheme']) { [string]$raw.powerScheme } else { '<not recorded>' }
        time = [string]$raw.time
    }
}

# 样本层：样本编号必须恰好覆盖预期集合且无重复；iterations 列与口径一致。
function Test-SampleLayer($rows, $spec, [string]$context) {
    $matched = @($rows | Where-Object { $_.case -eq $spec.case -and $_.records -eq $spec.records })
    $groups = @($matched | Group-Object sample)
    if ($groups.Count -ne $spec.samples) {
        throw "$context expected $($spec.samples) distinct samples for $($spec.case)/$($spec.records), got $($groups.Count)"
    }
    foreach ($group in $groups) {
        if ($group.Count -ne 1) { throw "$context duplicate sample $($group.Name) for $($spec.case)/$($spec.records)" }
    }
    foreach ($id in 0..($spec.samples - 1)) {
        if ($groups.Name -notcontains "$id") { throw "$context missing sample $id for $($spec.case)/$($spec.records)" }
    }
    foreach ($row in $matched) {
        if ($row.iterations -ne $spec.iters) {
            throw "$context $($spec.case)/$($spec.records) sample $($row.sample) iterations $($row.iterations) != $($spec.iters)"
        }
        if (-not ($row.ns -gt 0)) { throw "$context $($spec.case)/$($spec.records) sample $($row.sample) ns must be positive" }
    }
}

# 正确性层：每份日志必须出现完整的预期 oracle 键集合，不多不少、无重复。
function Test-OracleLayer([string]$logPath, $expectedKeys, [string]$context) {
    $oracles = Read-Oracles $logPath
    $seen = @($oracles | ForEach-Object { $_.key })
    foreach ($key in $expectedKeys) {
        if ($seen -notcontains $key) { throw "$context oracle log misses key '$key'" }
    }
    foreach ($key in $seen) {
        if ($expectedKeys -notcontains $key) { throw "$context oracle log has unexpected key '$key'" }
    }
    $duplicates = @($seen | Group-Object | Where-Object { $_.Count -gt 1 })
    if ($duplicates.Count -gt 0) { throw "$context oracle log has duplicate keys" }
    $map = @{}
    foreach ($oracle in $oracles) { $map[$oracle.key] = $oracle.digest }
    $map
}

# 校验单个取证目录并返回聚合输入。
function Import-Run([string]$runPath, [string]$label, $specCases) {
    foreach ($file in @('wall.csv', 'wall.log', 'allocation.csv', 'allocation.log', 'environment.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $runPath $file))) { throw "Missing evidence file: $runPath/$file" }
    }
    $wallLog = Get-Content -LiteralPath (Join-Path $runPath 'wall.log') -Raw
    if ($wallLog -notmatch 'allocation=False') { throw "$label wall.log must be a non-allocation build" }
    $allocLog = Get-Content -LiteralPath (Join-Path $runPath 'allocation.log') -Raw
    if ($allocLog -notmatch 'allocation=True') { throw "$label allocation.log must be an allocation build" }

    $wallRows = Read-CsvRows (Join-Path $runPath 'wall.csv')
    $allocRows = Read-CsvRows (Join-Path $runPath 'allocation.csv')
    $expectedPairs = @($specCases | ForEach-Object { "$($_.case) $($_.records)" })
    foreach ($row in @($wallRows + $allocRows)) {
        $pair = "$($row.case) $($row.records)"
        if ($expectedPairs -notcontains $pair) { throw "$label unexpected row pair '$pair'" }
    }
    foreach ($spec in $specCases) {
        Test-SampleLayer $wallRows $spec "$label wall"
        Test-SampleLayer $allocRows $spec "$label allocation"
    }
    foreach ($row in $wallRows) {
        if ($row.allocations -ne 0 -or $row.reallocations -ne 0 -or $row.allocatedBytes -ne 0 -or $row.reallocatedBytes -ne 0) {
            throw "$label wall rows must keep placeholder zero allocation columns"
        }
    }

    $oracleKeys = @($specCases | Where-Object { $_.oracle } | ForEach-Object { "$($_.case) $($_.records)" })
    $wallOracles = Test-OracleLayer (Join-Path $runPath 'wall.log') $oracleKeys "$label wall"
    $allocOracles = Test-OracleLayer (Join-Path $runPath 'allocation.log') $oracleKeys "$label allocation"
    foreach ($key in $wallOracles.Keys) {
        if ($wallOracles[$key] -ne $allocOracles[$key]) {
            throw "$label oracle digest differs between wall and allocation logs for '$key'"
        }
    }

    $env = Read-Env (Join-Path $runPath 'environment.json')
    @{ label = $label; wall = $wallRows; alloc = $allocRows; oracles = $wallOracles; env = $env }
}

# 汇总一个变体一个阶段：来源层校验 + 跨轮 oracle 一致 + 逐场景中位数聚合。
function Import-Phase([string]$root, [string]$variant, [string]$phase, $specCases) {
    $runs = @(Get-ChildItem -LiteralPath $root -Directory -Filter 'run*' | Sort-Object Name)
    if ($runs.Count -lt $MinRuns) { throw "$variant/$phase needs at least $MinRuns runs" }
    $imported = @()
    foreach ($run in $runs) {
        $imported += Import-Run $run.FullName "$variant/$phase/$($run.Name)" $specCases
    }
    $reference = $imported[0]
    $expectedFilter = if ($phase -eq 'fresh') { 'fresh_output' } else { '' }
    foreach ($entry in $imported) {
        if ($entry.env.manifest -ne $reference.env.manifest -or $entry.env.lockfile -ne $reference.env.lockfile) {
            throw "$variant/$phase manifest or lockfile differ across runs"
        }
        if (Compare-Object @($entry.env.sources.GetEnumerator()) @($reference.env.sources.GetEnumerator())) {
            throw "$variant/$phase measurement sources differ across runs"
        }
        if ($entry.env.baseline -ne $reference.env.baseline) {
            throw "$variant/$phase production baseline differs across runs"
        }
        if ($entry.env.caseFilter -ne $expectedFilter) {
            throw "$variant/$phase caseFilter '$($entry.env.caseFilter)' != '$expectedFilter'"
        }
    }
    foreach ($entry in $imported) {
        foreach ($key in $entry.oracles.Keys) {
            if ($reference.oracles.ContainsKey($key) -and $reference.oracles[$key] -ne $entry.oracles[$key]) {
                throw "$variant/$phase oracle digest differs across runs for '$key'"
            }
        }
    }
    $valueByPair = @{}
    $allocByPair = @{}
    foreach ($spec in $specCases) {
        $pair = "$($spec.case) $($spec.records)"
        $runMedians = @()
        $allocMedians = @()
        foreach ($entry in $imported) {
            $rows = @($entry.wall | Where-Object { $_.case -eq $spec.case -and $_.records -eq $spec.records })
            $perUnit = @($rows | ForEach-Object {
                if ($spec.unit -eq 'call') { $_.ns / $_.iterations } else { $_.ns }
            })
            $runMedians += Get-Median $perUnit
            $allocRows = @($entry.alloc | Where-Object { $_.case -eq $spec.case -and $_.records -eq $spec.records })
            $allocPerUnit = @($allocRows | ForEach-Object {
                if ($spec.unit -eq 'call') { $_.allocations / $_.iterations } else { $_.allocations }
            })
            $allocMedians += Get-Median $allocPerUnit
        }
        $valueByPair[$pair] = @{ median = (Get-Median $runMedians); runs = $runMedians; unit = $spec.unit }
        $allocByPair[$pair] = Get-Median $allocMedians
    }
    @{
        variant = $variant
        phase = $phase
        values = $valueByPair
        allocs = $allocByPair
        oracles = $reference.oracles
        env = $reference.env
        runCount = $runs.Count
    }
}

$phases = @{
    before_full = Import-Phase (Join-Path $Evidence 'before') 'before' 'full' $FullCases
    after_full = Import-Phase (Join-Path $Evidence 'after') 'after' 'full' $FullCases
    before_fresh = Import-Phase (Join-Path $Evidence 'before/fresh') 'before' 'fresh' $FreshCases
    after_fresh = Import-Phase (Join-Path $Evidence 'after/fresh') 'after' 'fresh' $FreshCases
}

# 来源层：同一阶段 A/B 必须使用同一份测量程序（manifest/lockfile/src 全等）。
foreach ($phase in @('full', 'fresh')) {
    $a = $phases["before_$phase"]
    $b = $phases["after_$phase"]
    if ($a.env.manifest -ne $b.env.manifest -or $a.env.lockfile -ne $b.env.lockfile) {
        throw "$phase A/B manifest or lockfile differ"
    }
    if (Compare-Object @($a.env.sources.GetEnumerator()) @($b.env.sources.GetEnumerator())) {
        throw "$phase A/B measurement sources differ"
    }
}

# 正确性层：A/B 对拍——同一阶段内全部 oracle 摘要必须逐键一致。
foreach ($phase in @('full', 'fresh')) {
    $a = $phases["before_$phase"]
    $b = $phases["after_$phase"]
    foreach ($key in $a.oracles.Keys) {
        if (-not $b.oracles.ContainsKey($key)) { throw "$phase after misses oracle '$key'" }
        if ($a.oracles[$key] -ne $b.oracles[$key]) {
            throw "$phase A/B oracle digest mismatch for '${key}': $($a.oracles[$key]) vs $($b.oracles[$key])"
        }
    }
    foreach ($key in $b.oracles.Keys) {
        if (-not $a.oracles.ContainsKey($key)) { throw "$phase before misses oracle '$key'" }
    }
}

# 汇总：fresh_output 取 fresh 阶段，其余取 full 阶段。
function Get-Phase([string]$caseName) {
    if ($caseName -eq 'fresh_output') { 'fresh' } else { 'full' }
}

$lines = @('case,records,unit,before_ns,after_ns,delta_pct,before_alloc,after_alloc,evidence')
$summaryRows = @()
foreach ($spec in $FullCases) {
    $phase = Get-Phase $spec.case
    $before = $phases["before_$phase"]
    $after = $phases["after_$phase"]
    $pair = "$($spec.case) $($spec.records)"
    $beforeMedian = $before.values[$pair].median
    $afterMedian = $after.values[$pair].median
    $delta = if ($beforeMedian -gt 0) { [Math]::Round((($afterMedian - $beforeMedian) / $beforeMedian) * 100, 3) } else { 0 }
    $lines += "$($spec.case),$($spec.records),$($spec.unit),$beforeMedian,$afterMedian,$delta,$($before.allocs[$pair]),$($after.allocs[$pair]),run1-$($before.runCount)"
    $summaryRows += [ordered]@{
        case = $spec.case
        records = $spec.records
        unit = $spec.unit
        before = $beforeMedian
        after = $afterMedian
        delta = $delta
        beforeRuns = @($before.values[$pair].runs)
        afterRuns = @($after.values[$pair].runs)
    }
}
$oracleKeysAll = (@($phases.before_full.oracles.Keys) + @($phases.before_fresh.oracles.Keys)) | Sort-Object -Unique
$lines += "# oracle digests identical across before/after for: $($oracleKeysAll -join '; ')"
Set-Content -LiteralPath $Results -Value $lines -Encoding utf8

# 生成带单位的结果表与环境摘要；数值直接来自 evidence，供 README 引用。
$unitLabel = @{ call = 'µs/调用'; sequence = 'µs/增长序列' }
$table = @(
    '| 场景 | 记录数 | 单位 | before | after | 差值 |',
    '| --- | --- | --- | ---: | ---: | ---: |'
)
foreach ($row in $summaryRows) {
    $table += "| $($row.case) | $($row.records) | $($unitLabel[$row.unit]) | $([Math]::Round($row.before / 1000, 3)) | $([Math]::Round($row.after / 1000, 3)) | $($row.delta)% |"
}
$env = $phases.before_full.env
$summary = @(
    '',
    '## 测量环境（由各 run 的 environment.json 汇总）',
    '',
    '| 项 | 值 |',
    '| --- | --- |',
    "| CPU | $($env.cpu) |",
    "| OS | $($env.os) |",
    "| 工具链（取证记录） | $($env.rustc)；$($env.cargo) |",
    '| 电源方案 | powercfg 取证失败未记录（见各 environment.json） |',
    "| before/full 生产基线 | $($phases.before_full.env.baseline) |",
    "| after/full 生产基线 | $($phases.after_full.env.baseline) |",
    "| before/fresh 生产基线 | $($phases.before_fresh.env.baseline) |",
    "| after/fresh 生产基线 | $($phases.after_fresh.env.baseline) |",
    "| 测量程序 main.rs（full 阶段，A/B 相同） | $($phases.before_full.env.sources['main.rs']) |",
    "| 测量程序 main.rs（fresh 阶段，A/B 相同） | $($phases.before_fresh.env.sources['main.rs']) |"
)
Set-Content -LiteralPath $SummaryTable -Value ($table + $summary) -Encoding utf8
Write-Output $Results
