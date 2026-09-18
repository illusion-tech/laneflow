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
    $sorted = $values | Sort-Object
    [double]$sorted[[int][Math]::Floor($sorted.Count / 2)]
}

function Read-CsvRows([string]$path) {
    @(Get-Content -LiteralPath $path | Where-Object { $_ -and $_ -notmatch '^case,' } | ForEach-Object {
        $parts = $_ -split ','
        [ordered]@{
            case = $parts[0]
            dataset = $parts[1]
            sample = [int]$parts[2]
            iterations = [int]$parts[3]
            ns = [double]$parts[4]
            allocations = [double]$parts[5]
            reallocations = [double]$parts[6]
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

function Test-SampleLayer($rows, $case, [string]$context) {
    $matched = @($rows | Where-Object { $_.case -eq $case })
    foreach ($datasetGroup in @($matched | Group-Object dataset)) {
        $ids = @($datasetGroup.Group | ForEach-Object { [int]$_.sample })
        $unique = @($ids | Sort-Object -Unique)
        if ($unique.Count -ne $Samples) {
            throw "$context $case/$($datasetGroup.Name) expected $Samples distinct samples, got $($unique.Count)"
        }
        if ($ids.Count -ne $unique.Count) { throw "$context duplicate sample ids for $case/$($datasetGroup.Name)" }
        foreach ($row in $datasetGroup.Group) {
            if (-not ($row.ns -gt 0)) { throw "$context $case/$($datasetGroup.Name) ns must be positive" }
        }
    }
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
$oracleBefore = @{}
$oracleAfter = @{}
foreach ($variant in @(@{ name = 'before'; runs = $before }, @{ name = 'after'; runs = $after })) {
    foreach ($run in $variant.runs) {
        $wallRows = Read-CsvRows (Join-Path $run.path 'wall.csv')
        $allocRows = Read-CsvRows (Join-Path $run.path 'allocation.csv')
        foreach ($spec in $CaseSpec) {
            Test-SampleLayer $wallRows $spec.case "$($variant.name)/$($run.path) wall"
            Test-SampleLayer $allocRows $spec.case "$($variant.name)/$($run.path) allocation"
            $wallCases = @($wallRows | Where-Object { $_.case -eq $spec.case })
            $allocCases = @($allocRows | Where-Object { $_.case -eq $spec.case })
            if ($wallCases.Count -eq 0 -and $spec.case -notin @('fresh_output', 'alternate')) {
                throw "$($variant.name) $($run.path) misses case $($spec.case)"
            }
            foreach ($row in $wallCases) {
                if ($row.iterations -ne $spec.iters) { throw "$($variant.name) iterations mismatch for $($spec.case)" }
                if ($row.allocations -ne 0 -or $row.reallocations -ne 0) { throw 'wall rows must keep zero allocation columns' }
            }
            foreach ($group in ($wallCases | Group-Object dataset)) {
                $pair = "$($spec.case) $($group.Name)"
                $medians = @($group.Group | ForEach-Object { $_.ns / $_.iterations })
                if (-not $valueByPair.ContainsKey("$($variant.name)|$pair")) { $valueByPair["$($variant.name)|$pair"] = @() }
                $valueByPair["$($variant.name)|$pair"] += Get-Median $medians
                $allocGroup = @($allocCases | Where-Object { $_.dataset -eq $group.Name })
                $allocMedians = @($allocGroup | ForEach-Object { $_.allocations / $_.iterations })
                if ($allocMedians.Count -gt 0) {
                    if (-not $allocByPair.ContainsKey("$($variant.name)|$pair")) { $allocByPair["$($variant.name)|$pair"] = @() }
                    $allocByPair["$($variant.name)|$pair"] += Get-Median $allocMedians
                }
            }
        }
        $cold = @($wallRows | Where-Object { $_.case -eq 'cold' })
        if ($cold.Count -ne $Samples) { throw "$($variant.name) cold needs $Samples samples" }
        $coldMedians = @($cold | ForEach-Object { $_.ns })
        if (-not $valueByPair.ContainsKey("$($variant.name)|cold cold_probe")) { $valueByPair["$($variant.name)|cold cold_probe"] = @() }
        $valueByPair["$($variant.name)|cold cold_probe"] += Get-Median $coldMedians
        $coldAlloc = @(($allocRows | Where-Object { $_.case -eq 'cold' }) | ForEach-Object { $_.allocations })
        if (-not $allocByPair.ContainsKey("$($variant.name)|cold cold_probe")) { $allocByPair["$($variant.name)|cold cold_probe"] = @() }
        $allocByPair["$($variant.name)|cold cold_probe"] += Get-Median $coldAlloc

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

$lines = @('case,dataset,before_ns,after_ns,delta_pct,before_alloc,after_alloc')
$table = @('| 场景 | 数据集 | before µs | after µs | 差值 |', '| --- | --- | ---: | ---: | ---: |')
foreach ($pair in ($valueByPair.Keys | Sort-Object)) {
    $beforeMedian = Get-Median ([double[]]$valueByPair["before|$pair"])
    $afterMedian = Get-Median ([double[]]$valueByPair["after|$pair"])
    if (-not $afterMedian -and $afterMedian -ne 0) { throw "after misses pair '$pair'" }
    $delta = if ($beforeMedian -gt 0) { [Math]::Round((($afterMedian - $beforeMedian) / $beforeMedian) * 100, 3) } else { 0 }
    $beforeAlloc = if ($allocByPair.ContainsKey("before|$pair")) { Get-Median ([double[]]$allocByPair["before|$pair"]) } else { '' }
    $afterAlloc = if ($allocByPair.ContainsKey("after|$pair")) { Get-Median ([double[]]$allocByPair["after|$pair"]) } else { '' }
    $parts = $pair -split ' ', 2
    $lines += "$($parts[0]),$($parts[1]),$beforeMedian,$afterMedian,$delta,$beforeAlloc,$afterAlloc"
    $table += "| $($parts[0]) | $($parts[1]) | $([Math]::Round($beforeMedian / 1000, 3)) | $([Math]::Round($afterMedian / 1000, 3)) | $($delta)% |"
}
$lines += "# oracle digests identical across before/after for: $((@($oracleBefore.Keys) | Sort-Object) -join '; ')"
Set-Content -LiteralPath $Results -Value $lines -Encoding utf8

$env = $refBefore
$summary = @(
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
