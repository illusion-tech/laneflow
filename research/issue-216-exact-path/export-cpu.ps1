param(
    [Parameter(Mandatory)][string]$CaptureDirectory,
    [Parameter(Mandatory)][ValidateSet('smoke', '10k-256', '10k-16', '1k-256')][string]$Case
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$capture = (Resolve-Path -LiteralPath $CaptureDirectory).Path
$metadata = Get-Content -LiteralPath (Join-Path $capture 'environment.json') -Raw | ConvertFrom-Json
$trace = (Resolve-Path -LiteralPath (Join-Path $capture "$Case.etl")).Path
$output = Join-Path $capture "$Case-export"
if (Test-Path -LiteralPath $output) { throw "Refusing to replace existing export: $output" }
New-Item -ItemType Directory -Path $output | Out-Null
$cache = Join-Path $capture 'symcache'
if (-not (Test-Path -LiteralPath $cache)) { New-Item -ItemType Directory -Path $cache | Out-Null }
$env:_NT_SYMBOL_PATH = Split-Path -Parent $metadata.binary
$env:_NT_SYMCACHE_PATH = $cache
$wpt = 'C:/Program Files (x86)/Windows Kits/10/Windows Performance Toolkit'
& (Join-Path $wpt 'xperf.exe') -i $trace -o (Join-Path $output 'trace-summary.txt') -a tracestats -timespan
if ($LASTEXITCODE -ne 0) { throw 'Trace header export failed' }
& (Join-Path $wpt 'xperf.exe') -i $trace -o (Join-Path $output 'process-images.txt') -a process -image
if ($LASTEXITCODE -ne 0) { throw 'Process image export failed' }
& (Join-Path $wpt 'xperf.exe') -i $trace -o (Join-Path $output 'trace-stats.txt') -a tracestats -detail stack
if ($LASTEXITCODE -ne 0) { throw 'Trace statistics export failed' }
& (Join-Path $wpt 'xperf.exe') -i $trace -o (Join-Path $output 'function-samples.txt') -symbols -a profile -detail
if ($LASTEXITCODE -ne 0) { throw 'Function sample export failed' }
& (Join-Path $wpt 'wpaexporter.exe') -i $trace -symbols -profile (Join-Path $PSScriptRoot 'cpu-samples.wpaProfile') -outputfolder $output *> (Join-Path $output 'wpa.log')
if ($LASTEXITCODE -ne 0) {
    Get-Content -LiteralPath (Join-Path $output 'wpa.log') -Tail 30
    throw 'WPA stack sample export failed'
}
Write-Output "CPU_EXPORT=$output"
