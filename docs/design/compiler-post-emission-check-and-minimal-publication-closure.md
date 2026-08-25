# 编译器后发射检查与最小发布闭合

> **状态**：Accepted（#299 G2 实现；2026-08-18）<br>
> **日期**：2026-08-18<br>
> **权威决策**：ADR 0024<br>
> **实现状态**：G2 生产实现与本地验证已完成；G3/G4 证据和治理闭合仍按 Issue Gate Ledger 推进

## 1. 目标与非目标

本设计把 #299 收缩为一个 compiler 发布硬化切片：

1. 对最终 LFCA/LFSM/LFSD exact bytes 建立不可绕过的 bundle 级检查；
2. 让 LFCP v2 和 manifest 发布只能消费本次检查得到的局部能力；
3. 删除独立 validator、receipt 和第二套语义实现；
4. 保持检查无分配、线性、单线程且可由 #300 复用。

本设计不实现：

- 来源或 compiler IR 的第二次语义编译；
- 全量身份、所有权、拓扑、几何或规则复验；
- 独立认证、签名、审计服务或通用证明协议；
- 共享静态路网构建与 Runtime/Spatial 闭合；
- LFSD 的迁移授权或 Runtime 修订切换。

## 2. 当前实现基线

`laneflow-format` 当前已经提供：

- 对象 framing、registry 和直接值域预检；
- 借用型 `ValueCheckedObjectView`；
- 有界 writer；
- `no_std` 且不依赖 heap；
- 无分配的 `check_post_emission_bundle_v1` 与字段私有的
  `PostEmissionCheckedBundleV1`。

`laneflow-compiler` 当前已经：

- 从一个 `CompilationOutput` 产生 LFCA/LFSM/LFSD；
- 在 emitter 和 publication 中逐对象调用
  `preflight_object_values_v1`，并在发布路径调用 bundle 后发射检查；
- 在 compiler-private 代码中计算 `NetworkRevisionId`；
- 用 `PortablePublicationCandidate` 拥有三份 exact bytes 及缓存绑定；
- 按 LFCA/LFSM/LFSD/LFCP v2 顺序安装，随后恰好一次调用 manifest adapter。

#299 没有重写这些基础设施；它把 bundle 级计算和比较下沉到 `laneflow-format`，并已删除
receipt 路径与 LFCP v1 生产 API。

## 3. 包与职责

```text
laneflow-compiler ───────────────┐
                                 ├──> laneflow-format
future laneflow-static-network ──┘              │
                                                └──> laneflow-static-contract
```

| 包                         | 新增或保留职责                                                                            | 明确不拥有                                 |
| -------------------------- | ----------------------------------------------------------------------------------------- | ------------------------------------------ |
| `laneflow-static-contract` | 版本、格式硬上限、`NetworkRevisionId`、`Sha256Digest`、对象/字段登记                      | 读取字节、hash、发布                       |
| `laneflow-format`          | 单对象预检、bundle 后发射检查、对象摘要、revision 重算、跨对象 binding、借用型 capability | 来源/LIR、文件系统、manifest、完整路网语义 |
| `laneflow-compiler`        | 来源和 IR 语义、发射、候选拥有、LFCP v2、安装编排、manifest 提交                          | 第二套验证语义、对象内真实性               |
| #300                       | 复用 public checked view 构造进程内 `SharedNetworkRevision`                               | 反向依赖 compiler-private LIR/emitter      |

新增依赖：

```toml
# crates/laneflow-format/Cargo.toml
sha2 = { version = "0.11", default-features = false }
```

该版本已由 compiler 使用，不新增第三方包或 feature。

## 4. 公共检查 API

G2 实现保持下列语义形状；精确 Rust 字段布局可以在不扩大能力的前提下调整。

```rust
pub enum ExpectedSemanticDiffBaseV1 {
    Genesis,
    Artifact {
        network_revision_derivation_version: u16,
        network_revision: NetworkRevisionId,
        digest: Sha256Digest,
        byte_length: ExactByteLength,
    },
}

pub fn check_post_emission_bundle_v1<'a>(
    lfca: &'a [u8],
    lfsm: &'a [u8],
    lfsd: &'a [u8],
    expected_base: ExpectedSemanticDiffBaseV1,
    limits: FormatLimits,
) -> Result<PostEmissionCheckedBundleV1<'a>, PostEmissionCheckError>;
```

`PostEmissionCheckedBundleV1<'a>` 必须：

- 字段私有且没有 public/`unsafe` 构造器；
- 直接借用三个 exact-byte slice；
- 保存三对象的 `ValueCheckedObjectView`、SHA-256、`ExactByteLength`；
- 保存重算且已与 LFCA claim 比较的 `NetworkRevisionId`；
- 保存已从 LFSM 读取并完成跨对象比较的 compiler/source binding；
- 通过只读 accessor 暴露上述值；
- 不实现序列化、签名、trust 或 publication 状态转换。

轻量 capability 可以实现 `Clone`/`Copy`；不可伪造来自字段私有性和构造入口，而不是
一次性消费技巧。

#496 G1（Proposed，未 Pass）：G2 为 LFCA/LFSM/LFSD v2 提供并行的后发射受检能力，
从中派生 v2 路网输入。现行 v1 预检与 v1 bundle **不得**接纳对象版本 `2`。LFSM
`sourceMapFormatVersion = 2`，`canonicalArtifactFormatVersion` 必须与所绑 LFCA 一致。
LFSD `semanticDiffFormatVersion = 2`；Genesis target 合同行须与 LFCA v2 一致，
Artifact 两端合同行仍须相等（v1→v2 diff 拒绝）。`NetworkRevisionId` 仍按
`portable-canonical-artifact.md` §4.2 v1 算法重算（派生版本保持 `1`）。发布路径不得
把 LFCA v2 送进 v1 bundle。G2 决定入口名字。详见 ADR 0028。

## 5. 检查顺序

检查顺序固定以下安全约束，但不把内部扫描遍数冻结为协议：

1. 用三个 slice 的已知长度逐一检查 `ObjectBytes`；
2. checked-add 三个长度并检查 `CandidateStagingBytes`；
3. 分别运行 `preflight_object_values_v1`；
4. 计算三对象 SHA-256 和精确长度；
5. 从 LFCA 六个语义节重算 `NetworkRevisionId`；
6. 比较 LFCA claim；
7. 比较 LFSM→LFCA binding 和 LFCA/LFSM 重复 provenance binding；
8. 比较 LFSD target→LFCA binding；
9. 比较 LFSD base→`ExpectedSemanticDiffBaseV1` binding；
10. 构造 capability。

步骤 1–2 必须发生在 hash 或解析前。后续实现可以在不增加分配、不改变失败分类和结果的
前提下合并顺序访问。

### 5.1 LFCA

除既有格式和值域预检外，bundle 检查只增加：

- SHA-256 与 exact length；
- `NetworkRevisionId` 重算；
- declared revision 与重算结果比较。

不重新计算逐实体 `StableId128`，不建立第二份全局 identity/collision registry，也不从
LFCA 反向恢复 compiler LIR。

### 5.2 LFSM

检查 LFSM 中所有已有的规范制品绑定均与本次 LFCA 计算值相等，包括：

- LFCA 格式版本；
- revision derivation version；
- `NetworkRevisionId`；
- LFCA digest 与 exact length；
- 与 LFCA compiler provenance 重复的 compiler build/source collection binding。

位置、property path 和直接 owner-local 形状继续由既有值域预检负责。检查器不复演
compiler-private `SourceMapInput` 或证明外部来源真实性。

### 5.3 LFSD

LFSD 的 `targetNetworkRevisionDerivationVersion`、`targetNetworkRevision`、
`targetCanonicalArtifactDigest` 和 `targetCanonicalArtifactByteLength` 必须分别与本次 LFCA 的
`networkRevisionDerivationVersion`、revision、digest 和 exact length 相等。

`ExpectedSemanticDiffBaseV1::Genesis` 要求 LFSD 使用规范 Genesis 零绑定；
`Artifact` 要求 LFSD 的 `baseNetworkRevisionDerivationVersion`、`baseNetworkRevision`、
`baseCanonicalArtifactDigest` 和 `baseCanonicalArtifactByteLength` 分别与显式输入中的派生版本、
revision、digest 和 exact length 相等。
expected base 必须由 compiler 发射调用保存的实际 base binding 或后继调用方的受信上下文
提供，禁止从 LFSD 自报字段反向构造。

检查器不比较 base/target 完整实体集合，不重新生成 change rows，也不证明差异无遗漏。

## 6. 错误模型

`PostEmissionCheckError` 只保留稳定大类：

```rust
pub enum PostEmissionCheckError {
    Format(FormatError),
    LimitExceeded { /* existing dimension/actual/limit */ },
    NetworkRevisionMismatch,
    SourceMapBindingMismatch,
    SemanticDiffBaseBindingMismatch,
    SemanticDiffTargetBindingMismatch,
    ArithmeticOverflow,
}
```

直接格式和值域错误继续保留 `FormatError` 的结构和 offset。跨对象错误不建立 check ID、
结果向量或策略插件；测试只依赖稳定大类和必要绑定值。

## 7. Compiler 候选与发布生命周期

`PortablePublicationCandidate` 继续拥有 LFCA/LFSM/LFSD `Box<[u8]>`，并新增/保存从实际
`PortableDiffBase` 计算的 expected base binding。它仍是未发布候选，不因 emitter 完成
而可信。

```text
CompilationOutput
    │
    ├─ emit LFCA/LFSM/LFSD
    ▼
PortablePublicationCandidate (owns bytes; unpublished)
    │
    ├─ check_post_emission_bundle_v1
    ▼
PostEmissionCheckedBundleV1<'candidate> (borrowed; in-memory only)
    │
    ├─ install LFCA/LFSM/LFSD
    ├─ build/install LFCP v2
    └─ authenticated manifest commit exactly once
    ▼
ManifestCommittedPortablePublication
```

`commit_portable_publication_v2` 必须在任何 installer 调用前完成 bundle 检查。内部
LFCP builder 接受 checked capability，不从 candidate 缓存字段读取 digest/revision；
object key 从 capability 的 digest 派生。

公开 installer 仍可安装普通内容寻址对象，但“已安装”不等于“已发布”。只有
`PortableManifestCommitter` 返回成功后才能构造 committed capability。

## 8. LFCP v2

LFCP v2 保持 magic `LFCP`，设置
`canonicalPublicationDescriptorVersion = 2`，精确包含三个 section/table：

| Section  | Table                      | 字段                                                                                                                                                                                                    |
| -------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0x0001` | `CanonicalArtifactBinding` | `1:canonicalArtifactFormatVersion:u16:R`, `2:networkRevisionDerivationVersion:u16:R`, `3:networkRevision:Sha256:R`, `4:canonicalArtifactDigest:Sha256:R`, `5:canonicalArtifactByteLength:u64:R`         |
| `0x0002` | `SourceMapBinding`         | `1:sourceMapFormatVersion:u16:R`, `2:sourceMapDigest:Sha256:R`, `3:sourceMapByteLength:u64:R`, `4:compilerBuildId:Utf8:R`, `5:sourceCollectionDigestVersion:u16:R`, `6:sourceCollectionDigest:Sha256:R` |
| `0x0003` | `PublicationProvenance`    | `1:publisherKind:u8:R`, `2:publisherBuildId:Utf8:R`, `3:artifactObjectKey:Utf8:R`, `4:sourceMapObjectKey:Utf8:R`, `5:controlledBuildProvenance:Utf8:O`, `6:controlledTimestamp:Utf8:O`                  |

LFCP v2 独立冻结 `PortablePublisherKindV2` 的封闭编码，数值与 v1 保持一致但不依赖 v1
parser 或 registry：

| Wire 值 | `PortablePublisherKindV2` | 含义         |
| ------- | ------------------------- | ------------ |
| `0`     | `LocalTool`               | 本地发布工具 |
| `1`     | `Ci`                      | CI 发布流程  |
| `2`     | `ReleaseService`          | 受控发布服务 |

任何其他 `u8` 值都必须由 LFCP v2 直接值域预检以 `Format` 类错误拒绝，不能映射为
unknown/future 值，也不能由 `publisherBuildId` 推断种类。

因此：

- object preamble section count 为 `3`；
- registry table 总数为 `3`；
- 第一节规范 offset 为 `32 + 3 * 24 = 104 (0x0068)`；
- `artifactObjectKey` 和 `sourceMapObjectKey` 必须分别等于
  `"sha256/" + hexLower(digest)`；
- 不存在 `ValidationReceiptBinding`、`receiptObjectKey` 或 LFSD binding。

LFCP v2 exact bytes 由新的
[`LFCP-V2-MIN-BINDINGS`](../../crates/laneflow-compiler/tests/fixtures/portable-v2/lfcp-v2-min-bindings/README.md)
固定向量覆盖。

### 8.1 v1 处置

生产实现直接把 `CANONICAL_PUBLICATION_DESCRIPTOR_VERSION` 提升为 `2`，registry 只
接受 v2。不保留 v1 parser/schema/branch。

G2 已删除：

- `CanonicalPublicationReceiptViewV1`；
- receipt subject/binding structs；
- receipt validation/install/error branches；
- test-only opaque receipt；
- `LFCP-V1-MIN-BINDINGS` 的生产兼容测试。

旧 fixture 和 #298 文档仍可作为 Git/GitHub 历史证据引用，但不得继续作为当前 accepted
wire。若 G2 前发现 GitHub Releases 之外已有 LFCP v1 公开消费者，必须返回 G1 重开兼容
决定。

## 9. 原子发布事务

成功顺序固定为：

1. bundle 检查；
2. LFCA 安装和 winner binding 比较；
3. LFSM 安装和 winner binding 比较；
4. LFSD 安装和 winner binding 比较；
5. LFCP v2 构造、格式预检、安装和 winner binding 比较；
6. 外部认证 manifest 恰好一次提交；
7. 返回 committed capability。

LFSD 当前仍是未被 LFCP/manifest 引用的内容对象。安装它只保留后继 #302 的候选输入，
不表示发布、认证、迁移授权或激活。

任一步失败都不调用后续步骤，不返回部分 committed 状态。已经成功安装的 immutable
内容对象可以保持未引用并供相同 bytes 重用。

## 10. 性能与资源

硬约束：

- `laneflow-format` 继续 `no_std`；
- checker 自身零 heap allocation；
- 零完整对象复制；
- 不建立第二份路网对象图；
- 单线程、确定性；
- 总成本 O(total exact bytes)；
- 每次 publication 调用一次完整 bundle check。

checker 内每对象 digest 最多计算一次，revision 最多计算一次。安装器从文件系统重新
读取/hash winner 属于持久化完整性边界，不能省略，也不计入 checker 门槛。

G2 一次性复用 #298 `LF-COMP-PRODUCTION-CORRIDOR-v1`：

- 同一 `LF-P100-REF-01`；
- 两次 fresh process；
- 各自预热一次并记录七个正式样本；
- 单独记录 checker 与 `compile + emit + check`；
- 最高级 checker median 在两个进程中均不超过同进程 emitter median 的 `30%`；
- installer I/O 单独记录；
- 不新增常驻 benchmark 基础设施。

## 11. 验证矩阵

G2 最小测试集合：

| 类别       | 必需证据                                                                              |
| ---------- | ------------------------------------------------------------------------------------- |
| 成功       | Genesis 与 Artifact base 的完整 emit→check→install→LFCP v2→manifest                   |
| 单对象     | LFCA/LFSM/LFSD 截断、追加、错误 kind/version、caller limit                            |
| revision   | LFCA declared revision 单独篡改后稳定失败                                             |
| source map | LFCA digest/length/revision/compiler/source binding 任一错配                          |
| diff       | Genesis 非零 base；Artifact base 或 target 的派生版本/revision/digest/length 任一错配 |
| 原子性     | 任一 check failure 时 installer 与 manifest 调用次数均为零                            |
| LFCP v2    | 固定 exact bytes/digest/offset；receipt 字段不存在                                    |
| installer  | 每个对象和 LFCP v2 的 write/flush/close/install/winner/manifest fault                 |
| 资源       | checker allocation 为零；P100 两进程均满足 30% 门槛                                   |
| 平台       | Windows/Ubuntu 对现有 LFCA/LFSM/LFSD 与新 LFCP v2 fixed vector 一致                   |

不新增独立 fuzz service、证明 oracle 或测试 DSL。现有格式 mutation/property 测试可以
继续复用，但不成为新的产品层。

## 12. G2 修改范围

实际修改集中在：

- `crates/laneflow-format`：bundle checker、hash、capability、错误和测试；
- `crates/laneflow-static-contract`：LFCP 当前版本与 v2 registry；
- `crates/laneflow-compiler`：expected base binding、publication v2、LFCP v2、删除 receipt；
- portable exact-byte fixtures/workflow；
- #300/#302 的依赖说明。

本 Issue 不新建 crate，不修改 Runtime/Adapter API，不实现共享静态路网或修订切换。

## 13. G1/G2 停止条件

出现下列任一情况必须返回 G1：

- 无法在不分配/复制完整对象的前提下完成已接受检查；
- checker P100 门槛失败且局部优化不能关闭；
- LFCP v1 已存在未登记的公开消费者；
- #300 必须依赖新的语义能力而非本设计的 checked view；
- 实现需要完整身份、图语义或 LFSD 重建才能满足验收；
- 需要改变 LFCA/LFSM/LFSD v1 wire，而不仅是 LFCP v2。
