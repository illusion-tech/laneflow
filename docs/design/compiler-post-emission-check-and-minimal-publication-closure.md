# 编译器后发射检查与最小发布闭合

> **状态**：Accepted<br>
> **日期**：2026-08-30<br>
> **权威决策**：ADR 0024<br>

## 1. 目标与非目标

本设计定义 compiler 最终制品与宿主发布之间的硬化边界：

1. 对最终 LFCA/LFSM/LFSD exact bytes 建立不可绕过的 bundle 级检查；
2. 让 LFCP v2 只能从本次检查得到的局部能力构造；
3. 删除独立 validator、receipt 和第二套语义实现；
4. 保持检查有界、线性、单线程且可由共享静态路网构建复用。

本设计不实现：

- 来源或 compiler IR 的第二次语义编译；
- 全量身份、所有权、拓扑、几何或规则复验；
- 独立认证、签名、审计服务或通用证明协议；
- 共享静态路网构建与 Runtime/Spatial 闭合；
- LFSD 的迁移授权或 Runtime 修订切换。
- 内容寻址仓库、并发 winner、atomic no-replace 文件晋升、目录持久化或 manifest 提交；
  这些属于选择持久化制品的宿主、CI、打包工具或未来独立存储后端。

## 2. 合同基线

`laneflow-format` 必须提供：

- 对象 framing、registry 和直接值域预检；
- 借用型 `ValueCheckedObjectView`；
- 有界 writer；
- checker core 保持 `no_std` 且不依赖 heap；Windows/Unix closed staged source 是可选
  `std` backing adapter，不进入格式解析或 binding 算法；
- 无分配的 `check_post_emission_bundle` 与字段私有的
  `PostEmissionCheckedBundle`。

`laneflow-compiler` 必须：

- 从一个 `CompilationOutput` 产生 LFCA/LFSM/LFSD；
- 在 emitter 中逐对象调用 `preflight_object_values`，并在任何 LFCP 构造或共享静态构建前
  调用 bundle 后发射检查；
- 在 compiler-private 代码中计算 `NetworkRevisionId`；
- 用 `PortablePublicationCandidate` 拥有三份不可变 staged object source 及 expected base binding；
- 让 checker 与共享静态构建直接复用同一 file-backed backing，不复制完整对象；
- 从 checked capability 构造 LFCP v2 exact bytes 和 binding，持久化与认证由宿主负责。

## 3. 包与职责

```text
laneflow-compiler ───────────────┐
                                 ├──> laneflow-format
laneflow-static-network ─────────┘              │
                                                └──> laneflow-static-contract
```

| 包                         | 新增或保留职责                                                                                                                 | 明确不拥有                                 |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------ |
| `laneflow-static-contract` | 版本、格式硬上限、`NetworkRevisionId`、`Sha256Digest`、对象/字段登记                                                           | 读取字节、hash、发布                       |
| `laneflow-format`          | 单对象预检、bundle 后发射检查、对象摘要、revision 重算、跨对象 binding、来源绑定 capability；可选 `std` staged backing adapter | 来源/LIR、文件安装、manifest、完整路网语义 |
| `laneflow-compiler`        | 来源和 IR 语义、file-backed 发射、候选拥有、LFCP v2 exact bytes 与 binding                                                     | 第二套验证语义、对象内真实性、内容仓库事务 |
| `laneflow-static-network`  | 复用 public checked view 构造进程内 `SharedNetworkRevision`                                                                    | 反向依赖 compiler-private LIR/emitter      |

新增依赖：

```toml
# crates/laneflow-format/Cargo.toml
sha2 = { version = "0.11", default-features = false }
memmap2 = { version = "0.9.11", optional = true }
tempfile = { version = "3.27.0", optional = true }
```

`std` staged adapter 直接声明 `memmap2` 与 `tempfile`；checker/writer core 默认仍为
`no_std`。写侧使用安全的顺序 file sink 与定点回填，唯一 `unsafe` island 是 finish 后的
私有只读映射；`tempfile` 在 Unix 使用匿名或立即 unlink 的 backing，在 Windows 使用
`share_mode(0)` + delete-on-close 的 backing。该适配器只持有临时 backing，不提供安装或
发布事务。

## 4. 公共检查 API

公共 API 保持下列语义形状；精确 Rust 字段布局可以在不扩大能力的前提下调整。

```rust
pub enum ExpectedSemanticDiffBase {
    Genesis,
    Artifact {
        network_revision_derivation_version: u16,
        network_revision: NetworkRevisionId,
        digest: Sha256Digest,
        byte_length: ExactByteLength,
    },
}

mod private {
    pub trait SealedImmutableBacking {}
}

pub struct ClosedStagedObjectSource { /* 字段私有 */ }

pub trait BoundedReReadableObjectSource: private::SealedImmutableBacking {
    fn exact_byte_length(&self) -> ExactByteLength;
    fn read_exact_at(
        &self,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<(), ObjectSourceError>;
}

pub fn check_post_emission_bundle<L, M, D>(
    lfca: L,
    lfsm: M,
    lfsd: D,
    expected_base: ExpectedSemanticDiffBase,
    limits: FormatLimits,
) -> Result<PostEmissionCheckedBundle<L, M, D>, PostEmissionCheckError>
where
    L: BoundedReReadableObjectSource,
    M: BoundedReReadableObjectSource,
    D: BoundedReReadableObjectSource,
{
    // private implementation
}
```

该 sealed supertrait 是能力边界，不是仅靠文档约束任意调用方实现：safe downstream code
不能为路径、普通 `File`、可写映射、内部可变 buffer 或 callback 自行实现来源 trait。
`laneflow-format` 只登记调用期间没有可写别名的完整 slice/owned immutable bytes，以及已
关闭的暂存对象 `ClosedStagedObjectSource`。不得要求对象先持久化或安装才能参加 bundle
检查与共享静态构建。

`ClosedStagedObjectSource` 只能由字段私有的 staged writer 完成 `finish` 状态转换获得：

1. writer 在调用方提供且由 LaneFlow capability 独占的临时 backing 上写入；
2. `finish` 完成 flush，固定 file identity 与 exact length，并结束 staged writer 的写阶段；
3. backend 在能力生命周期内保留字段私有的 immutable capability/backing；底层可以仍由具写
   访问权的 `File` 支撑，但 finish 后没有 LaneFlow safe API 可达的写能力；
4. 路径、原始 `File`、writable mapping、writer token 与重新开启写权限的能力均不暴露给
   safe downstream；检查与共享静态构建只借用或消费该 immutable capability/backing；
5. identity、exact length 或 bytes 在检查期间发生漂移时失败关闭。宿主绕过 capability 直接
   修改其临时文件属于宿主错误，不要求 LaneFlow 建立操作系统级内容仓库隔离。

可选 `std` adapter 可以用平台文件 API 或只读映射实现字段私有 wrapper；所需平台细节不能
成为 downstream admission API，也不承担 rename、no-replace、目录持久化或 winner 竞争。

完整 slice 通过零复制 adapter 实现同一来源接口，不建立第二个检查入口。检查期间发生
任何 backing identity/length 漂移都是不可能由 safe API 表达的状态；若平台 capability
仍检测到漂移，必须失败关闭且不得返回受检 bundle。

`PostEmissionCheckedBundle<L, M, D>` 必须：

- 字段私有且没有 public/`unsafe` 构造器；
- 保存三个不可变受检来源句柄、SHA-256、`ExactByteLength` 与版本/登记表检查结果；
- 保存重算且已与 LFCA claim 比较的 `NetworkRevisionId`；
- 保存已从 LFSM 读取并完成跨对象比较的 compiler/source binding；
- 通过只读 accessor 暴露上述值；
- 不实现序列化、签名、trust 或 publication 状态转换。

来源在检查生命周期内必须保持 exact length 与 bytes 不变，并支持 checker 按目录、chunk
与绑定需求重复顺序扫描。emitter 可以逐 chunk 写入 staged writer，并在每个对象完成后转成
`ClosedStagedObjectSource`，不要求 LFCA、LFSM、LFSD 三份百万级完整对象同时驻留。
slice adapter 与其它来源必须
产生相同 digest、exact length、`NetworkRevisionId`、binding 与 first error。能力只保存
后续 LFCP 构造/共享静态构建所需的受检来源句柄或借用，不复制三份对象，也不暴露未验证
chunk。

capability 只在三个来源句柄都可克隆时实现 `Clone`，不要求 `Copy`；不可伪造来自字段
私有性和构造入口，而不是一次性消费技巧。

后发射检查只接纳 LFCA/LFSM/LFSD 对象版本 `5/4/4`；
`canonicalArtifactFormatVersion` 必须与所绑 LFCA 一致，Genesis target 合同行须与 LFCA
一致。
`NetworkRevisionId` 仍按
[`portable-canonical-artifact.md` §3.8](portable-canonical-artifact.md#38-路网修订标识) 的 v1 算法重算
（派生版本保持 `1`）。公开入口不带世代后缀。详见 ADR 0028。

LFSD 4 策略增量另由 compiler 的 `check_portable_policy_diff` 从实际 base/target
独立闭合，候选发射在返回前调用它；规则见路权实施合同 §4.3。该检查不把通用
`PostEmissionCheckedBundle` 提升为全部差异或全部来源的语义证明。LFSM 4 的
`check_portable_policy_sources` 在候选返回前按实施合同 §4.4 独立比较最终字节与
同次受检 source view；格式层已闭合策略及方向来源，两个正式前端的非空策略输入
仍由 W2 提供。不能从通用直接 binding 检查推断所有来源语义都已验证。

## 5. 检查顺序

检查顺序固定以下安全约束，但不把内部扫描遍数冻结为协议：

1. 用三个对象来源的已知 exact length 逐一检查调用方 `maxObjectBytes`；
2. 检查实际同时 retained/staged bytes；slice 适配器 checked-add 三个长度，流式来源按
   实际生命周期计量，不得仅因三个对象 exact length 之和超过旧常量而拒绝合法的百万级
   修订；
3. 分别运行 `preflight_object_values`；
4. 计算三对象 SHA-256 和精确长度；
5. 从 LFCA 六个语义节重算 `NetworkRevisionId`；
6. 比较 LFCA claim；
7. 比较 LFSM→LFCA binding 和 LFCA/LFSM 重复 provenance binding；
8. 比较 LFSD target→LFCA binding；
9. 比较 LFSD base→`ExpectedSemanticDiffBase` binding；
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

`ExpectedSemanticDiffBase::Genesis` 要求 LFSD 使用规范 Genesis 零绑定；
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
    ObjectSource { object: PortableObjectKind, error: ObjectSourceError },
    Format(FormatError),
    LimitExceeded { /* existing dimension/actual/limit */ },
    NetworkRevisionMismatch,
    SourceMapBindingMismatch,
    SemanticDiffBaseBindingMismatch,
    SemanticDiffTargetBindingMismatch,
    ArithmeticOverflow,
}
```

直接格式和值域错误继续保留 `FormatError` 的结构和 offset。来源读取失败、越界或 backing
漂移进入独立 `ObjectSource` 大类，不伪装成 wire 错误。跨对象错误不建立 check ID、结果
向量或策略插件；测试只依赖稳定大类和必要绑定值。

## 7. Compiler 候选与消费生命周期

`PortablePublicationCandidate` 拥有 LFCA/LFSM/LFSD 三个 sealed immutable-backing
object source；百万级生产路径使用 `ClosedStagedObjectSource`，并保存从实际
`PortableDiffBase` 计算的 expected base binding。完整
`Box<[u8]>` 可以通过零复制 adapter 支撑小对象，但不是百万级生产候选的存储形状。候选
不因 emitter 完成而可信。

emitter 只能同时借用同一个 `CompilationOutput` 的 canonical LIR 与
`ValidatedSourceMapInput`，并接收一份显式 `PortableEmissionProvenance` 和一个
`PortableDiffBase`。调用方不能分别构造、重新配对或覆盖 LFCA/LFSM/LFSD binding。
完整规范输入相同则所有支持平台产生相同 bytes；limits 与 worker 数只控制资源，不进入
bytes。显式改变 build/source provenance 可以改变 artifact/LFSM/LFCP digest，但在规范语义
未变时不得改变 `NetworkRevisionId`。

```text
CompilationOutput
    │
    ├─ emit LFCA/LFSM/LFSD
    ▼
PortablePublicationCandidate (owns staged object sources; unpublished)
    │
    ├─ check_post_emission_bundle
    ▼
PostEmissionCheckedBundle<L, M, D> (checked sources; process-local only)
    │
    ├─ build SharedNetworkRevision
    └─ build LFCP v2 exact bytes/bindings when requested
```

LFCP builder 接受 checked capability，不从 candidate 缓存字段读取 digest/revision；object
key 从 capability 的 digest 派生。宿主、CI 或打包工具可以复制、流式读取或持久化受检 exact
bytes，并用 LFCP binding 建立自身认证清单；LaneFlow 不把某种文件事务包装成发布能力。加载
时仍从宿主交付的 LFCP/LFCA/LFSM bytes 重新核对认证 binding、digest、length 与 revision。

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
[`LFCP-MIN-BINDINGS`](../../crates/laneflow-compiler/tests/fixtures/portable/lfcp-min-bindings/README.md)
固定向量覆盖。

### 8.1 v1 处置

生产实现直接把 `CANONICAL_PUBLICATION_DESCRIPTOR_VERSION` 提升为 `2`，registry 只
接受 v2。不保留 v1 parser/schema/branch。

现行树不保留 `CanonicalPublicationReceiptViewV1`、receipt subject/binding、receipt
validation/install/error 分支或 v1 生产兼容测试。旧 fixture 与实现由 Git 历史保存，不得
继续作为当前 wire。若发现已公开的 LFCP v1 消费者，必须重新进入兼容决策。

## 9. 宿主持久化与原子边界

LaneFlow 不拥有 LFCA/LFSM/LFSD/LFCP 的内容仓库或文件安装事务。宿主、CI、打包工具或发布
服务选择是否以及如何持久化 exact bytes，并负责其并发、崩溃一致性、目录耐久、签名与认证
manifest。未来若 LaneFlow 自建并发内容寻址仓库，必须单开设计与 Issue，不把 OS/文件系统
事务塞回 compiler、format 或 Runtime 核心合同。

本设计保留三种互不混同的原子性：

1. bundle 检查失败时不返回 `PostEmissionCheckedBundle`，也不触发 LFCP 构造或共享静态构建；
2. `SharedNetworkRevision` 构建失败时不返回部分 Traffic/Identity/Hints/Spatial 根；
3. Runtime 激活时整份活动修订一次替换，不能独立切换 Traffic 与 Spatial；该提交点由 #302
   定义，不属于文件安装。

LFCP v2 只绑定 checked LFCA/LFSM exact bytes 与 publication provenance。生成 LFCP 不表示宿主
已经持久化、认证、发布或激活任何对象；加载方仍必须根据受认证宿主描述符重新验证收到的
bytes。

## 10. 性能与资源

硬约束：

- `laneflow-format` checker core 继续 `no_std`；可选 `std` staged adapter 只建立/读取
  backing capability，不改变 checker 算法；
- checker 自身零 heap allocation；
- 零完整对象复制；
- 不建立第二份路网对象图；
- 单线程、确定性；
- 总成本 O(total exact bytes)；
- 每个候选在 LFCP 构造或共享静态构建前完成一次完整 bundle check。

checker 内每对象 digest 最多计算一次，revision 最多计算一次。宿主持久化或加载时的额外
hash 属于宿主交付完整性边界，不计入 checker 门槛。

现行资源证据在单线程 release 配置下覆盖 `10000` / `100000` / `1000000` 个现实混合稳定静态
实体，并逐阶段记录 source build、compile、file-backed emit、checker 与共享静态构建的
墙钟、对象 exact bytes、compiler-controlled peak、staged backing bytes 和 retained bytes。
代码与类型路径审计证明 staged 路径不构造完整对象 heap buffer；每档至少执行一次完整端到端
路径，证明百万级路径可达、返回候选不常驻完整 heap bytes、阶段峰值有记录且 checker 零 heap
allocation。单次资源样本不独立证明 emit 过程从未短暂分配完整 buffer，也不是统计性能结论或
Product Pass；仓库不要求不存在的专用 benchmark/harness。

## 11. 验证矩阵

最小测试集合：

| 类别       | 必需证据                                                                                         |
| ---------- | ------------------------------------------------------------------------------------------------ |
| 成功       | Genesis 与 Artifact base 的完整 file-backed emit→check→SharedNetworkRevision；按需构造 LFCP v2   |
| 单对象     | LFCA/LFSM/LFSD 截断、追加、错误 kind/version、caller limit                                       |
| revision   | LFCA declared revision 单独篡改后稳定失败                                                        |
| source map | LFCA digest/length/revision/compiler/source binding 任一错配                                     |
| diff       | Genesis 非零 base；Artifact base 或 target 的派生版本/revision/digest/length 任一错配            |
| 原子性     | 任一 check/build failure 不返回受检 bundle或部分共享根                                           |
| LFCP v2    | 固定 exact bytes/digest/offset；receipt 字段不存在；只从 checked capability 构造                 |
| backing    | safe code 不能为可变来源实现 sealed trait；slice 零复制；staged finish 后无 LaneFlow 写能力      |
| staging    | 同一 backing 发射、重复检查和共享静态构建；identity/length/bytes 漂移失败；无完整对象级复制      |
| 资源       | 10k/100k/1m 的 file-backed 路径、checker allocation 与 compile/emit/check/build 分阶段内存和耗时 |
| 平台       | Windows/Ubuntu 对 LFCA/LFSM/LFSD 与 LFCP v2 fixed vector 一致，不包含文件安装事务                |

不新增独立 fuzz service、证明 oracle 或测试 DSL。现有格式 mutation/property 测试可以
继续复用，但不成为新的产品层。

## 12. 实现范围

实际修改集中在：

- `crates/laneflow-format`：bundle checker、hash、capability、错误和测试；
- `crates/laneflow-static-contract`：LFCP 当前版本与 v2 registry；
- `crates/laneflow-compiler`：file-backed emission、expected base binding、LFCP v2、删除 receipt；
- `crates/laneflow-static-network`：直接消费 checked backing 并构造不借输入的共享根；
- portable exact-byte fixtures/workflow；
- #282/#283/#541 的直接消费证据与 #302 原子切换边界说明。

本合同不要求新建 crate，不修改 Runtime/Adapter 动态 API，也不实现修订切换；受检
file-backed LFCA 必须可直接构造现有 `SharedNetworkRevision`。

## 13. 重新进入设计的条件

出现下列任一情况必须重新进入设计：

- 无法在不分配/复制完整对象的前提下完成已接受检查；
- checker 资源门槛失败且局部优化不能关闭；
- LFCP v1 已存在未登记的公开消费者；
- 共享静态构建必须依赖新的语义能力而非本设计的 checked view；
- 实现需要完整身份、图语义或 LFSD 重建才能满足验收；
- 需要改变 LFCA 5 / LFSM 4 / LFSD 4 wire，而不仅是 LFCP 2。
