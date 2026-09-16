param(
    [Parameter(Mandatory)][string]$Baseline,
    [Parameter(Mandatory)][string]$Candidate,
    [string]$OutputDirectory = 'target/preflight-680/measurements',
    [ValidateRange(0, 63)][int]$Processor = 16
)

$ErrorActionPreference = 'Stop'
$executables = @{
    baseline = (Resolve-Path -LiteralPath $Baseline).Path
    candidate = (Resolve-Path -LiteralPath $Candidate).Path
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$outputRoot = (Resolve-Path -LiteralPath $OutputDirectory).Path
if (Test-Path -LiteralPath (Join-Path $outputRoot 'manifest.json')) {
    throw 'Use a fresh output directory to preserve existing measurements.'
}
$manifest = [ordered]@{
    startedUtc = [DateTime]::UtcNow.ToString('o')
    processor = $Processor
    baseline = @{ path = $executables.baseline; sha256 = (Get-FileHash -LiteralPath $executables.baseline -Algorithm SHA256).Hash }
    candidate = @{ path = $executables.candidate; sha256 = (Get-FileHash -LiteralPath $executables.candidate -Algorithm SHA256).Hash }
    order = @()
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputRoot 'manifest.json') -Encoding utf8

for ($round = 1; $round -le 3; $round++) {
    $variants = if ($round % 2 -eq 1) { @('baseline', 'candidate') } else { @('candidate', 'baseline') }
    foreach ($variant in $variants) {
        $name = "round-$round-$variant"
        $parameters = @{
            FilePath = $executables[$variant]
            ArgumentList = @('road_editing::preflight::benchmark::preflight_scaling', '--ignored', '--exact', '--nocapture', '--test-threads=1')
            WindowStyle = 'Hidden'
            PassThru = $true
            RedirectStandardOutput = (Join-Path $outputRoot "$name.log")
            RedirectStandardError = (Join-Path $outputRoot "$name.err")
        }
        $process = Start-Process @parameters
        $process.ProcessorAffinity = [IntPtr]([int64]1 -shl $Processor)
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) { throw "$name failed with exit code $($process.ExitCode)" }
        $samples = @(Get-Content -LiteralPath $parameters.RedirectStandardOutput | Select-String 'PREFLIGHT,')
        if ($samples.Count -ne 33) { throw "$name produced $($samples.Count) samples; expected 33" }
        $manifest.order += $name
        $manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputRoot 'manifest.json') -Encoding utf8
        Write-Output "$name completed: $($samples.Count) samples"
    }
}
$manifest.completedUtc = [DateTime]::UtcNow.ToString('o')
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputRoot 'manifest.json') -Encoding utf8
