#Requires -Version 7.0
<#
.SYNOPSIS
  构建正式测量二进制（#876）：冻结 cargo 命令 + 本机根路径映射 + 构建后路径扫描。
.DESCRIPTION
  内部执行 `cargo +1.98.0 build -p laneflow-urban-harness --release --locked`，
  通过 CARGO_ENCODED_RUSTFLAGS 附加三条冻结的 --remap-path-prefix（工作树、
  Cargo 主目录、rustup 工具链 sysroot，固定顺序），并显式 CARGO_INCREMENTAL=0。
  同一张映射表同时写出归一化记录 LANEFLOW_MEASUREMENT_RUSTFLAGS（来源端为
  占位符，不含本机路径），由 harness 编译期 option_env! 嵌入二进制——wire
  审计禁止 harness 携带 build.rs，不能用构建脚本生成代码。正式测量
  provenance（urban-performance-measurements-v5）逐项校验嵌入值；不用本脚本
  构建的二进制不能进入正式测量。构建后先正向断言二进制嵌入了期望的归一化
  flag 串（防 cargo 缓存复用陈旧嵌入），再固定执行
  test-binary-local-paths.ps1 扫描，命中任何本机根即失败。回执只含占位符与
  摘要，可直接随证据公开。
.EXAMPLE
  pwsh -NoProfile -File tools/laneflow-urban-harness/build-measurement.ps1
#>
param(
  [string]$TargetDir,
  [string]$EvidenceRoot,
  [string]$ReceiptPath
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
if (-not (Test-Path -LiteralPath (Join-Path $repoRoot 'Cargo.lock'))) { throw 'Not a LaneFlow checkout root' }
if (-not $TargetDir) { $TargetDir = Join-Path $repoRoot 'target\release-measurement' }
if (-not $ReceiptPath) { $ReceiptPath = Join-Path $TargetDir 'build-receipt.json' }
if (Test-Path -LiteralPath $ReceiptPath) { throw "Receipt already exists: $ReceiptPath" }

$sysroot = (& rustc +1.98.0 --print sysroot).Trim()
if ($LASTEXITCODE -ne 0 -or -not $sysroot) { throw 'rustc +1.98.0 --print sysroot failed' }
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }

# 唯一映射表：实际 flag 与归一化记录同源于此，二者不可能漂移。
# 固定顺序与 harness 冻结的 EXPECTED_BUILD_RUSTFLAGS 一一对应；顺序或目标端
# 变化会改变嵌入记录，正式测量 provenance 将拒绝。
$map = @(
  @{ from = $repoRoot; to = '<worktree>' },
  @{ from = $cargoHome; to = '<cargo-home>' },
  @{ from = $sysroot; to = '<rustup-toolchain>' }
)
$actualFlags = ($map | ForEach-Object { "--remap-path-prefix=$($_.from)=$($_.to)" }) -join [char]31
$normalizedFlags = ($map | ForEach-Object { "--remap-path-prefix=$($_.to)=$($_.to)" }) -join ' '

$saved = @{}
foreach ($name in 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_INCREMENTAL', 'CARGO_TARGET_DIR', 'RUSTFLAGS', 'LANEFLOW_MEASUREMENT_RUSTFLAGS') {
  $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
  # CARGO_ENCODED_RUSTFLAGS 在 Cargo 的 flag 来源中优先级最高，覆盖用户级
  # config 的 build.rustflags，保证实际 flag 集合恰为冻结值。
  [Environment]::SetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', $actualFlags, 'Process')
  [Environment]::SetEnvironmentVariable('CARGO_INCREMENTAL', '0', 'Process')
  [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $TargetDir, 'Process')
  [Environment]::SetEnvironmentVariable('RUSTFLAGS', $null, 'Process')
  [Environment]::SetEnvironmentVariable('LANEFLOW_MEASUREMENT_RUSTFLAGS', $normalizedFlags, 'Process')
  & cargo +1.98.0 build -p laneflow-urban-harness --release --locked
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed: $LASTEXITCODE" }
} finally {
  foreach ($name in $saved.Keys) {
    [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process')
  }
}

$binary = Join-Path $TargetDir 'release\laneflow-urban-harness.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw "Expected binary missing: $binary" }

# 正向断言：二进制必须嵌入了期望的归一化 flag 串。option_env! 不进 cargo
# 指纹，陈旧复用会在本步暴露，而不是留到测量 provenance 才失败。
$latin1 = [Text.Encoding]::GetEncoding(28591)
$haystack = $latin1.GetString([IO.File]::ReadAllBytes($binary))
if (-not $haystack.Contains($normalizedFlags)) {
  throw "Binary does not embed the expected normalized rustflags; remove $TargetDir and rebuild"
}

$scanArgs = @('-NoProfile', '-File', (Join-Path $PSScriptRoot 'test-binary-local-paths.ps1'), '-BinaryPath', $binary)
if ($EvidenceRoot) { $scanArgs += @('-EvidenceRoot', $EvidenceRoot) }
$scanJson = & pwsh @scanArgs
if ($LASTEXITCODE -ne 0) {
  Write-Output $scanJson
  throw 'Local path scan failed; binary must not be published'
}
$scan = $scanJson | ConvertFrom-Json
$receipt = [ordered]@{
  version              = 'urban-measurement-build-v1'
  build_parameters     = 'pwsh -NoProfile -File tools/laneflow-urban-harness/build-measurement.ps1'
  cargo_command        = 'cargo +1.98.0 build -p laneflow-urban-harness --release --locked'
  rustflags_normalized = $normalizedFlags
  build_profile        = 'release;debug-assertions=off'
  binary               = $scan.binary
  scan                 = $scan
}
[IO.File]::WriteAllText([IO.Path]::GetFullPath($ReceiptPath), "$(($receipt | ConvertTo-Json -Depth 8))`n", [Text.UTF8Encoding]::new($false))
Write-Output "build-measurement: pass; receipt written to $ReceiptPath"
