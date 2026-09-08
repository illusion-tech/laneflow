param(
    [Parameter(Mandatory)][string] $Baseline,
    [Parameter(Mandatory)][string] $Candidate,
    [Parameter(Mandatory)][string] $OutputDirectory
)

$ErrorActionPreference = 'Stop'
$baselinePath = (Resolve-Path -LiteralPath $Baseline).Path
$candidatePath = (Resolve-Path -LiteralPath $Candidate).Path
if (Test-Path -LiteralPath $OutputDirectory) {
    throw 'Choose a new output directory; existing rounds must not be overwritten.'
}
$outputPath = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$cases = @(
    'journal_ab::runtime_journal_ab_1k_256'
    'journal_ab::runtime_journal_ab_10k_256'
    'journal_ab::runtime_journal_ab_10k_16'
    'journal_ab::runtime_journal_ab_10k_256_armed'
)
$binaries = @{
    baseline = $baselinePath
    candidate = $candidatePath
}
$hashes = @{
    baseline = (Get-FileHash -LiteralPath $baselinePath -Algorithm SHA256).Hash
    candidate = (Get-FileHash -LiteralPath $candidatePath -Algorithm SHA256).Hash
}
$records = [System.Collections.Generic.List[object]]::new()
for ($round = 0; $round -lt 12; $round++) {
    for ($offset = 0; $offset -lt $cases.Count; $offset++) {
        $caseIndex = ($round + $offset) % $cases.Count
        $order = if ($round % 2 -eq 0) { @('baseline', 'candidate') } else { @('candidate', 'baseline') }
        foreach ($variant in $order) {
            $info = [System.Diagnostics.ProcessStartInfo]::new()
            $info.FileName = $binaries[$variant]
            $info.UseShellExecute = $false
            $info.CreateNoWindow = $true
            $info.RedirectStandardOutput = $true
            $info.RedirectStandardError = $true
            foreach ($argument in @('--ignored', '--exact', $cases[$caseIndex], '--nocapture', '--test-threads=1')) {
                $info.ArgumentList.Add($argument)
            }
            $started = [DateTimeOffset]::UtcNow
            $process = [System.Diagnostics.Process]::Start($info)
            $processId = $process.Id
            $stdout = $process.StandardOutput.ReadToEndAsync()
            $stderr = $process.StandardError.ReadToEndAsync()
            $process.WaitForExit()
            $stem = 'round-{0:D2}-case-{1}-{2}' -f $round, $caseIndex, $variant
            $stdout.Result | Set-Content -LiteralPath (Join-Path $outputPath "$stem.log") -Encoding utf8
            $stderr.Result | Set-Content -LiteralPath (Join-Path $outputPath "$stem.stderr.log") -Encoding utf8
            $records.Add([ordered]@{
                round = $round
                case = $caseIndex
                variant = $variant
                order = $order -join '/'
                pid = $processId
                started_utc = $started.ToString('o')
                ended_utc = [DateTimeOffset]::UtcNow.ToString('o')
                exit_code = $process.ExitCode
                binary_sha256 = $hashes[$variant]
                stdout = "$stem.log"
                stderr = "$stem.stderr.log"
            })
            $records | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputPath 'processes.json') -Encoding utf8
            if ($process.ExitCode -ne 0) { throw "A/B process failed: $stem" }
            Write-Output "Completed $stem"
            $process.Dispose()
        }
    }
}
