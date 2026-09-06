# LFCA 受检构建有限验证

本目录是 [#535](https://github.com/illusion-tech/laneflow/issues/535) 的验证入口。
适用合同为 LFCA 5 / LFCP 2；执行结果、工具链和 exact head 记录在该 Issue 的关闭 PR。
这是 #305 的构建链证据，不代表其余认证轴已完成。

## 范围

沿用现有固定向量、后发射检查、共享构建器和跨平台 workflow。新增测试仅连接尚未直接
覆盖的调用边界，不引入生产代码、通用检查器、receipt 或新的 CI 门禁。

| 验证项                | 有限证据                                                                                                     | 结论边界                                                                                              |
| --------------------- | ------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- |
| exact bytes           | `min-headless` / `full-spatial`，每个进程各编译两次，再比较两个独立进程的 LFCA/LFSM/LFSD/LFCP                | 当前工具链与这些输入；不宣称所有输入、工具链或平台均已穷举                                            |
| 后发射检查            | `portable_publication::tests` 的正常 bundle、错误 framing/version、digest/length/revision、LFSM/LFSD binding | 证明字节与对象间绑定，不代替 compiler 来源语义                                                        |
| LFCP binding 与共享根 | `lfcp_bindings_and_both_checked_inputs_reach_the_same_shared_origin`                                         | LFCP 的 LFCA/LFSM 字段对应实际受检来源；单对象和同进程 bundle 构建得到相同 origin，并保留可用 Spatial |
| 跨表拒绝              | `shared_root` 中既有策略关系测试及新增的 successor / identity 注入                                           | 通过格式和 revision 检查的损坏 LFCA，仍在共享构建时返回明确错误且无根                                 |

LFCP / manifest 的签名、真实性认证和持久化由宿主拥有。仓库没有宿主认证服务，
本项不创建测试替身来宣称它已被验证。加载方须先认证描述符，再比较收到的 exact bytes
与 LFCP 的 digest、length、revision 和其他 binding；单对象检查成功不等于发布准入成功。

## 复现

在关闭 PR 的 exact head、仓库根目录运行，保持 `DUMP_PORTABLE` 未设置，避免刷新固定向量。
现有依赖已缓存时可加 `--offline`；`CARGO_HOME` 的位置由本机环境决定。

```powershell
git rev-parse HEAD
rustc +1.98.0 -Vv
cargo +1.98.0 fmt --all -- --check
cargo +1.98.0 test -p laneflow-compiler -p laneflow-format -p laneflow-static-network --all-features --locked
```

复用已有 exporter，以下 PowerShell 命令产生两个独立进程的实际文件：

```powershell
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$evidence535 = Join-Path (Get-Location) ('target/issue535-exact-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $evidence535 | Out-Null
try {
    foreach ($run535 in @(@{name='run-a';bytes='4096'}, @{name='run-b';bytes='1048576'})) {
        $env:LANEFLOW_PORTABLE_EVIDENCE_DIR = Join-Path $evidence535 $run535.name
        $env:LANEFLOW_PORTABLE_ALLOCATION_PERTURBATION_BYTES = $run535.bytes
        cargo +1.98.0 test -p laneflow-compiler --lib --locked `
            compiler::portable_fixture_tests::ci_evidence::portable_exact_bytes_ci_evidence `
            -- --ignored --exact --test-threads=1
    }
} finally {
    Remove-Item Env:LANEFLOW_PORTABLE_EVIDENCE_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:LANEFLOW_PORTABLE_ALLOCATION_PERTURBATION_BYTES -ErrorAction SilentlyContinue
}
foreach ($workload535 in 'min-headless', 'full-spatial') {
    foreach ($kind535 in 'lfca', 'lfsm', 'lfsd', 'lfcp') {
        $left535 = [IO.File]::ReadAllBytes((Join-Path $evidence535 "run-a/objects/$workload535/actual.$kind535"))
        $right535 = [IO.File]::ReadAllBytes((Join-Path $evidence535 "run-b/objects/$workload535/actual.$kind535"))
        if (-not [Linq.Enumerable]::SequenceEqual[byte]($left535, $right535)) {
            throw "$workload535 $kind535 exact bytes differ"
        }
        "$workload535 $kind535 exact bytes equal; length=$($left535.Length)"
    }
}
```

现有 [Portable exact bytes workflow](../../.github/workflows/portable-exact-bytes.yml)
额外比较 Ubuntu / Windows 的两个进程、固定向量、摘要、长度及随机状态 / 分配扰动。
当次跨平台结果以 PR 对应运行记录为准。

## 跨表注入的拒绝层级

新增用例位于 compiler 的现有 `portable_emitter::lfsm::policy_tests::shared_root`
模块，复用 `lfca-world-policies/expected.lfca` 和既有的 owned-object 编码助手：

1. 首先确认原始 fixture 能构建共享根。
2. 把第一条 LaneEdge 的 successor 改成 `lane_count`，或只翻转实体表 StableId 的一位，
   保持 canonical identity 表不变。
3. 重编码 chunk 摘要并重算 `NetworkRevisionId`。
4. `check_canonical_network_input` 必须成功；否则测试在进入 builder 前失败。
5. `Omit` 与 `RetainAvailable` 两种构建选项均分别得到
   `ReferenceOutOfBounds { structure: LaneSuccessors, ... }` 和
   `StableIdMismatch { entity_kind: LaneEdge, ordinal: 0 }`，不返回共享根。

这些是有限的结构关系回归用例。它们没有验证所有畸形输入，也不把重绑定后的测试制品
视作可信发布物。现有策略跨表拒绝和 post-emission mutation 测试继续由上述包测试执行。

## 依据

- [ADR 0025](../../docs/adr/0025-checked-canonical-network-and-shared-static-network.md)
- [后发射检查与最小发布闭合](../../docs/design/compiler-post-emission-check-and-minimal-publication-closure.md)
- [共享静态路网](../../docs/design/shared-static-network.md)
- [LFCA 固定向量](../../crates/laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/README.md)
- [LFCP 固定向量](../../crates/laneflow-compiler/tests/fixtures/portable/lfcp-min-bindings/README.md)
