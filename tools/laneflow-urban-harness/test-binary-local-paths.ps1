#Requires -Version 7.0
<#
.SYNOPSIS
  扫描二进制中的采集机本机路径（#876）。命中任何已登记根即失败。
.DESCRIPTION
  覆盖的根：用户目录、Cargo 主目录、rustup 主目录、1.98.0 工具链 sysroot、
  构建工作树，以及调用方追加的证据根/其他根。每个根同时检查 UTF-8（两种
  目录分隔符）与 UTF-16LE 编码。输出与回执只含占位符（<user-profile>、
  <cargo-home>、<rustup-home>、<rustup-toolchain>、<worktree>、
  <evidence-root>、<extra-N>），不出现实际路径，扫描输出本身可随证据公开。
  以 pwsh -File 调用时命中返回退出码 1，通过返回 0。
.EXAMPLE
  pwsh -NoProfile -File tools/laneflow-urban-harness/test-binary-local-paths.ps1 `
    -BinaryPath <worktree>/target/release-measurement/release/laneflow-urban-harness.exe
#>
param(
  [Parameter(Mandatory)][string]$BinaryPath,
  [string]$EvidenceRoot,
  [string[]]$ExtraRoot = @(),
  [string]$ReceiptPath
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$binary = (Resolve-Path -LiteralPath $BinaryPath).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path

$roots = [Collections.Generic.List[object]]::new()
$seenPaths = @{}
function Add-Root([string]$Placeholder, [string]$Path) {
  if (-not $Path) { return }
  $full = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
  if (-not (Test-Path -LiteralPath $full)) { return }
  if ($seenPaths.ContainsKey($full.ToLowerInvariant())) { return }
  $script:seenPaths[$full.ToLowerInvariant()] = $true
  $script:roots.Add(@{ placeholder = $Placeholder; path = $full })
}
Add-Root '<user-profile>' $env:USERPROFILE
Add-Root '<cargo-home>' $(if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' })
Add-Root '<rustup-home>' $(if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' })
$sysroot = (& rustc +1.98.0 --print sysroot).Trim()
if ($LASTEXITCODE -eq 0 -and $sysroot) { Add-Root '<rustup-toolchain>' $sysroot }
Add-Root '<worktree>' $repoRoot
Add-Root '<evidence-root>' $EvidenceRoot
for ($index = 0; $index -lt $ExtraRoot.Count; $index++) {
  Add-Root "<extra-$($index + 1)>" $ExtraRoot[$index]
}
if ($roots.Count -eq 0) { throw 'No scan roots resolved' }

# Latin-1 把字节一对一映射为字符，使任意字节序列可用字符串搜索；
# needle 先按 UTF-8/UTF-16LE 编码再作同样映射。
$bytes = [IO.File]::ReadAllBytes($binary)
$latin1 = [Text.Encoding]::GetEncoding(28591)
$haystack = $latin1.GetString($bytes)
$hits = [Collections.Generic.List[object]]::new()
foreach ($root in $roots) {
  foreach ($separator in @('\', '/')) {
    $surface = $root.path -replace '[\\/]', $separator
    foreach ($encoding in @([Text.Encoding]::UTF8, [Text.Encoding]::Unicode)) {
      $needle = $latin1.GetString($encoding.GetBytes($surface))
      $count = 0
      $firstOffset = -1
      $at = 0
      while (($at = $haystack.IndexOf($needle, $at, [StringComparison]::Ordinal)) -ge 0) {
        if ($firstOffset -lt 0) { $firstOffset = $at }
        $count++
        $at += 1
      }
      if ($count -gt 0) {
        $hits.Add([ordered]@{
            root       = $root.placeholder
            encoding   = $(if ($encoding -is [Text.UnicodeEncoding]) { 'utf-16le' } else { 'utf-8' })
            separator  = $separator
            count      = $count
            first_byte = $firstOffset
          })
      }
    }
  }
}

$hash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
$receipt = [ordered]@{
  version = 'urban-binary-local-path-scan-v1'
  binary  = [ordered]@{
    name   = [IO.Path]::GetFileName($binary)
    sha256 = $hash
    bytes  = $bytes.Length
  }
  roots   = @($roots | ForEach-Object { $_.placeholder })
  hits    = @($hits)
  status  = $(if ($hits.Count -eq 0) { 'pass' } else { 'fail' })
}
$json = $receipt | ConvertTo-Json -Depth 5
if ($ReceiptPath) {
  if (Test-Path -LiteralPath $ReceiptPath) { throw "Receipt already exists: $ReceiptPath" }
  [IO.File]::WriteAllText([IO.Path]::GetFullPath($ReceiptPath), "$json`n", [Text.UTF8Encoding]::new($false))
}
Write-Output $json
if ($hits.Count -gt 0) { exit 1 }
exit 0
