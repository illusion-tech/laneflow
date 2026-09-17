$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$root = Join-Path $repo ('target/pose-681/analyzer-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'evidence') -File | Copy-Item -Destination $root
$output = Join-Path $root 'result.csv'
& (Join-Path $PSScriptRoot 'analyze.ps1') -Evidence $root -Output $output
if (@(Import-Csv -LiteralPath $output).Count -ne 50) { throw 'Expected fifty groups' }
function Expect-Rejection([string]$Label) {
    $failed = $false
    try { & (Join-Path $PSScriptRoot 'analyze.ps1') -Evidence $root -Output $output }
    catch { $failed = $true }
    if (-not $failed) { throw "Analyzer accepted $Label" }
}
$csv = Join-Path $root 'wall-1-10000.csv'
$original = Get-Content -Raw -LiteralPath $csv
$lines = @(Get-Content -LiteralPath $csv)
$lines[0..($lines.Count - 2)] | Set-Content -LiteralPath $csv -Encoding utf8
Expect-Rejection 'missing sample'
Set-Content -LiteralPath $csv -Value $original -NoNewline -Encoding utf8
$data = @(Import-Csv -LiteralPath $csv)
$data[1].sample = $data[0].sample
$data | Export-Csv -LiteralPath $csv -NoTypeInformation -Encoding utf8
Expect-Rejection 'duplicate sample'
Set-Content -LiteralPath $csv -Value $original -NoNewline -Encoding utf8
$log = Join-Path $root 'wall-1-10000.log'
$originalLog = Get-Content -Raw -LiteralPath $log
Set-Content -LiteralPath $log -Value 'allocation=false missing oracle marker' -Encoding utf8
Expect-Rejection 'missing oracle'
Set-Content -LiteralPath $log -Value ($originalLog -replace 'allocation=false','allocation=true') -NoNewline -Encoding utf8
Expect-Rejection 'instrumented wall-clock process'
Set-Content -LiteralPath $log -Value ($originalLog -replace 'allocation=false','') -NoNewline -Encoding utf8
Expect-Rejection 'missing wall-clock build mode'
Set-Content -LiteralPath $log -Value $originalLog -NoNewline -Encoding utf8
$csv = Join-Path $root 'allocation-10000.csv'
$data = @(Import-Csv -LiteralPath $csv)
$data[1].sample = $data[0].sample
$data | Export-Csv -LiteralPath $csv -NoTypeInformation -Encoding utf8
Expect-Rejection 'duplicate allocation sample'
Write-Output '7 analyzer checks passed'
