param([Parameter(Mandatory)][string]$Evidence, [Parameter(Mandatory)][string]$Output)
$ErrorActionPreference = 'Stop'
function Median($values) {
    $sorted = @($values | Sort-Object)
    if ($sorted.Count -eq 0) { throw 'No samples' }
    return $sorted[[int][Math]::Floor($sorted.Count / 2)]
}
$rows = @()
foreach ($count in @(10000,100000)) {
    $rounds = @()
    foreach ($round in 1..3) {
        $path = Join-Path $Evidence "wall-$round-$count.csv"
        $log = Get-Content -Raw -LiteralPath (Join-Path $Evidence "wall-$round-$count.log")
        if ($log -notmatch "oracle-ok n=$count ") { throw "Missing oracle success: $path" }
        $data = @(Import-Csv -LiteralPath $path)
        if ($data.Count -ne 175) { throw "Wrong sample count: $path ($($data.Count))" }
        $rounds += ,$data
    }
    $allocationLog = Get-Content -Raw -LiteralPath (Join-Path $Evidence "allocation-$count.log")
    if ($allocationLog -notmatch "allocation=true" -or $allocationLog -notmatch "oracle-ok n=$count ") { throw 'Incomplete allocation process' }
    $allocation = @(Import-Csv -LiteralPath (Join-Path $Evidence "allocation-$count.csv"))
    if ($allocation.Count -ne 175) { throw 'Wrong allocation sample count' }
    foreach ($group in ($rounds[0] | Group-Object case,n,k)) {
        $first = $group.Group[0]
        $medians = @()
        foreach ($data in $rounds) {
            $samples = @($data | Where-Object { $_.case -eq $first.case -and $_.n -eq $first.n -and $_.k -eq $first.k })
            if ($samples.Count -ne 7 -or (@($samples.sample | Sort-Object -Unique) -join ',') -ne '0,1,2,3,4,5,6') { throw 'Incomplete or duplicate samples' }
            $medians += Median @($samples | ForEach-Object { [double]$_.ns / [double]$_.iterations })
        }
        $alloc = @($allocation | Where-Object { $_.case -eq $first.case -and $_.n -eq $first.n -and $_.k -eq $first.k })
        if ($alloc.Count -ne 7) { throw 'Allocation group missing' }
        $rows += [pscustomobject][ordered]@{
            population = $count
            case = $first.case
            n = $first.n
            k = $first.k
            round1_ns = $medians[0]
            round2_ns = $medians[1]
            round3_ns = $medians[2]
            median_ns = Median $medians
            allocations = Median @($alloc | ForEach-Object { [double]$_.allocations / [double]$_.iterations })
            reallocations = Median @($alloc | ForEach-Object { [double]$_.reallocations / [double]$_.iterations })
            allocated_bytes = Median @($alloc | ForEach-Object { [double]$_.allocated_bytes / [double]$_.iterations })
            reallocated_bytes = Median @($alloc | ForEach-Object { [double]$_.reallocated_bytes / [double]$_.iterations })
        }
    }
}
$rows | Export-Csv -LiteralPath $Output -NoTypeInformation -Encoding utf8
