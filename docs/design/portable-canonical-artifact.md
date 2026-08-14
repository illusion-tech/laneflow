# 可移植规范制品与辅助制品格式

**文档状态**: Draft；#298 G1 可审阅候选，不构成 G1 Pass 或 G2 开工授权<br>
**最后更新**: 2026-08-14<br>
**适用范围**: `laneflow-format`、`laneflow-static-contract`、
`laneflow-compiler` 的可移植规范制品（Portable Canonical Artifact）、源映射封套
（Source Map Envelope）、语义差异封套（Semantic Diff Envelope）、规范发布描述符
（Canonical Publication Descriptor）与原子发布边界<br>
**实现状态**: 尚未实现；当前生产编译器只原子返回配对的
`ValidatedCanonicalLir` 与 `ValidatedSourceMapInput`<br>
**关联 Issue**: #298；依赖 #292、#296；阻断 #299、#300<br>
**关联文档**: `network-compiler.md`、`compiler-foundation.md`、
`numeric-representation.md`、
`../adr/0020-compiler-owned-static-network-and-static-image.md`、
`../reference/v0.10-portable-artifact-validation.md`、
`../reference/glossary.md`

## 1. 状态、目标与非目标

本文把 `network-compiler.md` 第 8 节和 ADR 0020 已接受的长期边界收窄为 #298 的
实现级格式候选。本文的 Draft 状态表示字段、常量和限制值仍在 G1 审阅中；任何实现
PR 都必须等待 #298 发布正式 `## G1 设计判断` 且结论为 Pass。

#298 必须同时闭合四类对象：

1. 可移植规范制品：完整、目标无关的规范静态路网语义；
2. 源映射封套：同次成功编译的来源位置和来源沿袭；
3. 语义差异封套：一份基线制品与目标制品之间的结构化变化；
4. 规范发布描述符：位于上述对象字节之外的摘要、精确长度、修订和验证收据绑定。

本文不实现或授予下列权威：

- 独立验证器、验证收据签发或语义信任属于 #299；
- 目标静态镜像、镜像完整性清单和运行时加载属于后继切片；
- 可信路网切换描述符与迁移授权属于 #300；
- 编译器产生的语义差异只供审阅和诊断，不能自行授权运行时迁移；
- `compilerBuildId`、来源位置和发布元数据不进入路网修订语义摘要；
- 本格式不复用当前 Traffic/Spatial JSON、内部 FlatBuffers B1 来源格式、Rust 内存布局
  或当前编译器的 `semantic_fingerprint`。

## 2. 分层与所有权

### 2.1 包边界

依赖方向保持：

```text
laneflow-format ---------> laneflow-static-contract
laneflow-compiler -------> laneflow-format
laneflow-compiler -------> laneflow-static-contract
laneflow-validator ------> laneflow-format          # #299
laneflow-validator ------> laneflow-static-contract # #299
```

职责冻结为：

| 包                           | 拥有职责                                                                                    | 禁止拥有                                           |
| ---------------------------- | ------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| `laneflow-static-contract`   | 版本轴、摘要/长度值、`NetworkRevisionId`、实体/字段/记录种类登记、有类型序号和描述符值类型  | 文件系统、序列化器、编译器语义遍、Runtime、Spatial |
| `laneflow-format`            | 四类对象的精确线格式、受限写入器、结构预检、只读受检视图、格式错误                          | 编译器语义闭包、独立语义验证、Runtime/Spatial 构造 |
| `laneflow-compiler`          | 从同一个 `CompilationOutput` 发射制品/源映射/差异候选，建立来源沿袭，执行失败原子的暂存事务 | 独立验证权威、可信发布签名、运行时迁移授权         |
| `laneflow-validator`（#299） | 不复用编译器语义实现的身份、关系、规则、修订、源映射和语义差异验证，以及验证收据            | 调用 compiler emitter 重建“验证”结果               |

`laneflow-format` 的结构预检只证明字节边界、版本、封闭种类、排序、计数、偏移和基本值域安全；
它不能把 `CheckedPortableArtifactView` 命名为 trusted/validated，也不能证明实体语义闭合。

### 2.2 编码方案裁决候选

v1 候选采用 LaneFlow 自有的封闭、节目录二进制格式，不采用 Serde/bincode、Rust
归档内存布局、Protocol Buffers 或内部 FlatBuffers B1 作为公共存档格式。

| 候选                          | 独立实现与逐字节规范         | 未知字段策略          | 原生 ABI 风险 | v1 判断                                    |
| ----------------------------- | ---------------------------- | --------------------- | ------------- | ------------------------------------------ |
| 自有封闭节目录 + 规范记录编码 | 由本文完整定义               | 未知/重复种类失败关闭 | 无            | **候选选择**；攻击面最小，规范排序显式     |
| FlatBuffers                   | 需要 schema/codegen          | 通常允许向前演进      | 无            | 保留为 authoring B1，不复用为长期制品格式  |
| Protocol Buffers              | 字段顺序和未知字段需附加规范 | 默认保留/忽略未知字段 | 无            | 不选；closed-shape 与 exact bytes 约束更重 |
| bincode/rkyv/原生归档         | 易受库版本或 Rust 布局影响   | 非公共协议优先        | 高/中         | 不选                                       |

G1 必须由第二实现或独立脚本证明本文字节规则足以生成相同 known vectors；若做不到，必须在
G1 内补齐规范，不能把某个 Rust 库版本提升为隐式事实源。

## 3. 统一线格式规则

### 3.1 基本类型

所有对象遵守下列逐字节规则：

- 所有无符号整数采用固定宽度小端序；线格式不使用 `usize`、原生指针或可变长整数；
- `bool` 只允许单字节 `0` 或 `1`；其他值失败关闭；
- 封闭枚举使用登记宽度的无符号整数，未知判别值失败关闭；
- `StableId128` 是 16 个原始字节，保持身份编码计算结果顺序，不转换 UUID 字节序；
- SHA-256 与 `NetworkRevisionId` 是 32 个摘要原始字节，不在线格式中保存十六进制文本；
- 固定布局中的字符串是 `u32 byteLength` 后接严格 UTF-8 字节；不执行 Unicode
  归一化，字段自身的来源准入规则继续决定允许字符和值域；`FieldV1` 中则由外层
  `valueByteLength` 给出长度，value 只保存 UTF-8 原始字节；
- 固定布局中的字节串是 `u64 byteLength` 后接原始字节；`FieldV1` 中同样只保存由
  外层长度界定的原始字节；具体字段仍受调用方限制；
- `f32`/`f64` 保存 IEEE 754 binary32/binary64 little-endian bits；编译器必须先拒绝
  NaN/无穷并把 `-0.0` 规范化为 `+0.0`。读取器发现非有限值或负零位模式即失败关闭，
  不在读取时静默修复；
- 所有计数、`offset + length`、`count * stride` 和宿主地址宽度转换必须 checked；
- 线格式没有隐式对齐、填充、尾随字节或对象间共享指针。

### 3.2 对象前导与节目录

四类对象共用 32 字节 `ObjectPreambleV1`：

| 偏移   | 宽度 | 字段                     | v1 约束                                    |
| ------ | ---- | ------------------------ | ------------------------------------------ |
| `0x00` | 4    | `magic`                  | 对象专用 ASCII magic                       |
| `0x04` | 2    | `formatVersion`          | 对象专用版本；v1 为 `1`                    |
| `0x06` | 2    | `headerByteLength`       | v1 固定为 `32`                             |
| `0x08` | 4    | `flags`                  | v1 固定为 `0`，未知 bit 失败关闭           |
| `0x0c` | 4    | `sectionCount`           | 必须等于对象 v1 的封闭节数                 |
| `0x10` | 8    | `sectionDirectoryOffset` | v1 固定为 `32`                             |
| `0x18` | 8    | `objectByteLength`       | 必须等于外部受限读取器观察到的精确字节长度 |

每个目录项是 24 字节 `SectionDirectoryEntryV1`：

| 偏移（相对目录项） | 宽度 | 字段                   | v1 约束                              |
| ------------------ | ---- | ---------------------- | ------------------------------------ |
| `0x00`             | 2    | `sectionKind`          | 对象专用封闭枚举，严格递增且不得重复 |
| `0x02`             | 2    | `sectionFormatVersion` | v1 各节为 `1`                        |
| `0x04`             | 4    | `flags`                | v1 固定为 `0`                        |
| `0x08`             | 8    | `byteOffset`           | 第一节紧随目录，后续节紧随前一节     |
| `0x10`             | 8    | `byteLength`           | 允许对象专用规则定义的空节；不得越界 |

目录紧随前导。第一节偏移必须等于 `32 + sectionCount * 24`；所有节按 kind 升序无间隙、
无重叠、无填充地覆盖到 `objectByteLength`。因此不同写入器不能通过改变对齐或目录顺序
制造另一份合法字节。未知/缺失/额外/重复节、非最小偏移或尾随字节一律失败关闭。

### 3.3 规范记录编码

实体、关系、来源和差异表使用同一显式记录框架：

```text
TableV1 :=
  tableKind:u16
  tableSchemaVersion:u16 (=1)
  rowCount:u32
  rowsByteLength:u64
  RowV1[rowCount]

RowV1 :=
  rowByteLength:u64
  fieldCount:u32
  reserved:u32 (=0)
  FieldV1[fieldCount]

FieldV1 :=
  fieldTag:u16
  fieldType:u8
  flags:u8 (=0)
  valueByteLength:u64
  valueBytes[valueByteLength]
```

`fieldTag` 严格递增且不得重复。`fieldType` 是封闭枚举：`1=u8`、`2=u16`、`3=u32`、
`4=u64`、`5=f32`、`6=f64`、`7=StableId128`、`8=Sha256`、`9=Utf8`、
`10=Bytes`、`11=OrdinalVectorU32`、`12=RecordVector`。固定宽度类型的
`valueByteLength` 必须精确匹配；向量内部使用 `count:u32` 和相同类型的连续规范值。
`rowByteLength` 包含 16 字节 Row header 与全部 Field；`valueByteLength` 只计算紧随
12 字节 Field header 的 value；`rowsByteLength` 必须等于所有完整 Row 字节长度之和。
`RecordVector` 只允许 registry 明确登记的一层内嵌 Row，内嵌 Row 不得再含
`RecordVector`。任何冗余长度、计数或深度不一致都失败关闭。

对象节先保存 `tableCount:u32`，随后保存按 `tableKind` 严格递增的 Table。每个
`tableKind` 的字段登记、必需字段、字段类型、行 key 和排序键必须进入
`laneflow-static-contract` 的 append-only registry，并由本文附录 A 冻结。未知表、
未知字段、缺失必需字段、类型错配、非最小长度或非规范排序均失败关闭。

> **当前 G1 阻断项**：附录 A 仍须从 `ValidatedCanonicalLir` 的完整 22 种稳定实体、
> owner-local 关系、静态规则、几何和执行约束逐表登记精确 `tableKind/fieldTag`，并由
> 独立 oracle known vectors 证明。该项完成前本文不能转为 Accepted。

## 4. 可移植规范制品 `LFCA` v1

### 4.1 封闭节

`magic = "LFCA"`，`formatVersion = canonicalFormatVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                         | 是否进入规范语义载荷 | 内容                                                         |
| ------------- | ---------------------------- | -------------------- | ------------------------------------------------------------ |
| `0x0001`      | `ContractVersions`           | 是                   | identity、registry、revision、constraint、execution 版本轴   |
| `0x0002`      | `CanonicalIdentityTable`     | 是                   | 完整身份前像、声明 StableId128、有类型 ordinal               |
| `0x0003`      | `CanonicalEntityTables`      | 是                   | 22 种稳定实体和目标无关规范静态值                            |
| `0x0004`      | `CanonicalRelationTables`    | 是                   | 拓扑、成员、出现项、索引和静态规则关系                       |
| `0x0005`      | `CanonicalSpatialTables`     | 是                   | 空间存在标记、规范 f32 折线与派生采样；headless 时为规范空表 |
| `0x0006`      | `StaticExecutionConstraints` | 是                   | worker 数无关的静态执行约束                                  |
| `0x0007`      | `CompilerProvenance`         | 否                   | compiler build、来源集合摘要、显式编译选项与发射器版本       |

所有节都必须存在；“无空间”用 `CanonicalSpatialTables` 内登记的显式 `spatialPresent=0`
表示，不能通过删节表示。`CompilerProvenance` 会改变 artifact exact bytes 和
`canonicalArtifactDigest`，但不得改变 `NetworkRevisionId`。

### 4.2 规范路网语义载荷与修订标识

规范路网语义载荷不是完整 artifact bytes。它按 `sectionKind=0x0001..0x0006` 顺序连接：

```text
canonicalNetworkSemanticPayloadV1 :=
  for each semantic section:
    sectionKind:u16
    sectionFormatVersion:u16
    sectionByteLength:u64
    sectionExactBytes[sectionByteLength]
```

随后精确计算：

```text
NetworkRevisionIdV1 =
  SHA-256(
    "laneflow.network-revision.v1\0"
    || canonicalNetworkSemanticPayloadV1
  )
```

`ContractVersions` 保存上述派生版本和制品声明的 `NetworkRevisionId`。编译器计算该值，
#299 独立验证器必须从六个语义节独立重算。相同修订对应不同规范语义载荷时以
`NetworkRevisionDigestCollision` 失败关闭；不得追加随机数、ordinal 或 suffix。

完整 artifact exact bytes 另由 SHA-256 得到 `canonicalArtifactDigest`，长度是同一字节
序列的 `u64` 精确长度。摘要与长度不嵌回自身字节；二者由外部描述符绑定。

## 5. 源映射封套 `LFSM` v1

`magic = "LFSM"`，`sourceMapFormatVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                     | 内容                                                                   |
| ------------- | ------------------------ | ---------------------------------------------------------------------- |
| `0x0001`      | `SourceMapBindings`      | 修订派生版本/值、artifact digest/length、compiler build 与来源集合摘要 |
| `0x0002`      | `SourceModules`          | 依赖优先模块、来源文档键、文档摘要、frontend/import provenance         |
| `0x0003`      | `StableEntitySources`    | `(entityKind, StableId128, typedOrdinal)` 与 owning/contributing 位置  |
| `0x0004`      | `OwnerLocalSources`      | owner StableId128、typed role、`localIndex` 与来源位置                 |
| `0x0005`      | `DerivedRelationSources` | generated relation 推导链、pass/constraint version                     |

来源位置使用来源文档键和 UTF-8 byte offset/length；line/column 只可作为显示冗余，不能
替代字节范围。记录排序先按来源模块/文档的规范顺序，再按实体种类、StableId128、typed
ordinal、role、localIndex 和位置字节范围破同值。

`sourceMapDigest` 是完整 `LFSM` exact bytes 的 SHA-256；`sourceMapByteLength` 是相同
字节的精确 `u64` 长度。封套不嵌入自身摘要。来源位置或来源沿袭变化必须改变 LFSM
exact bytes，但在规范语义未变时不得改变 `NetworkRevisionId`。

## 6. 语义差异封套 `LFSD` v1

`magic = "LFSD"`，`semanticDiffFormatVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                     | 内容                                                       |
| ------------- | ------------------------ | ---------------------------------------------------------- |
| `0x0001`      | `SemanticDiffBindings`   | base/genesis 与 target 的 revision、artifact digest/length |
| `0x0002`      | `EntityChanges`          | 稳定实体 add/remove、显示名变化与字段语义变化              |
| `0x0003`      | `RelationChanges`        | owner/member、topology reconnect、出现项和 localIndex 变化 |
| `0x0004`      | `GeometryChanges`        | 规范几何、长度和容差显著变化                               |
| `0x0005`      | `StaticRuleChanges`      | Gate/Waiting/Signal/Access 等行为变化                      |
| `0x0006`      | `IdentityClosureChanges` | 稳定标识改变及其父锚/字段原因                              |

`baseBindingKind` 是封闭枚举：`0=Genesis`、`1=Artifact`。Genesis 必须把所有 base
版本、修订、digest 和 length 字节规范为零，并把目标的所有稳定实体和 owner-local
序列报告为新增；`Artifact` 禁止任何零占位。target 永远必须是具体 artifact。

记录按 change class、entity kind、owner StableId128、subject StableId128、typed role、
field tag、before localIndex、after localIndex 严格排序。重复关系值按最低 before/after
`localIndex` 破同值。相同 base/target 允许产生合法的空变化集合，但仍保留完整绑定。
目标静态镜像的 layout/profile-only 变化不进入 LFSD，也不得伪装成语义变化。

编译器可以从受结构预检的 base view 生成诊断性差异，但 #299 必须从两份独立通过语义
验证的 artifact 重算或逐项验证 LFSD；#300 的可信切换描述符再绑定 LFSD digest/length。
LFSD 自身永远不授予迁移权限。

## 7. 规范发布描述符 `LFCP` v1

`magic = "LFCP"`，`canonicalPublicationDescriptorVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                       | 内容                                                                       |
| ------------- | -------------------------- | -------------------------------------------------------------------------- |
| `0x0001`      | `CanonicalArtifactBinding` | format/revision 版本、revision、artifact digest/length                     |
| `0x0002`      | `SourceMapBinding`         | source-map version、digest/length、compiler build、来源集合摘要            |
| `0x0003`      | `ValidationReceiptBinding` | receipt format、`canonical-publication-v1`、validator build、digest/length |
| `0x0004`      | `PublicationProvenance`    | publisher kind/build、immutable object keys、受控时间/CI provenance        |

描述符不得包含自身摘要或签名。真实性来自描述符 exact bytes 之外的签名 publication
manifest、宿主已认证 asset/package manifest 或 pinned digest。对象内自报的修订、摘要、
长度、compiler/validator build 或 provenance 都不能自证可信。

#298 冻结 receipt binding 的槽位和 fail-closed 版本行为；#299 冻结收据内部 wire/check
results 并签发 `canonical-publication-v1`。在 #299 收据存在前，编译器只能产生未受信
`PortablePublicationCandidate`，不能伪造 LFCP 中的空摘要、空收据或 trusted 标记。

LFSD 不进入 LFCP，因为 canonical publication 不授予迁移；LFSD 的 exact binding 只
进入 #300 的 `NetworkRevisionCutoverDescriptor`。同一次发射事务仍必须原子地产生
artifact、source map 和 genesis/base diff 候选，避免后续错误配对。

## 8. 编译与原子发布事务

### 8.1 唯一输入与候选结果

发射器只能同时借用同一个 `CompilationOutput` 的 LIR 和来源映射输入：

```text
CompilationOutput
  -> emitPortableCandidate(base: Optional<CheckedPortableArtifactView>, limits)
  -> PortablePublicationCandidate
       canonicalArtifactExactBytes
       sourceMapEnvelopeExactBytes
       semanticDiffEnvelopeExactBytes
       computedBindings
```

调用方不能分别构造或重新配对 LIR/source-map input。`laneflow-format` 提供写入器与
结构视图；对私有编译器 LIR 的字段投影和差异生成继续在 `laneflow-compiler`。G2 API
不得暴露“写 artifact 成功、source map 失败后仍可取出 artifact”的中间成功状态。

### 8.2 发布提交点

发布协议不依赖跨平台目录替换恰好原子：

1. 在目标发布根目录同一文件系统内创建唯一暂存目录；
2. 以受限写入器完成三个对象，关闭写入并从最终 exact bytes 计算 digest/length；
3. 重新执行结构预检和候选内部 binding 核对；
4. 使用 `create_new` 语义按 digest 写入不可变对象；若对象已存在，必须核对精确长度和
   bytes，不能覆盖；
5. #299 收据完成后构造 LFCP，并由外部发布链认证；
6. 只有外部认证 manifest/指针的单次提交才使候选变为“已发布”；暂存对象或未引用的
   content-addressed objects 不构成发布；
7. 任一步失败都删除暂存引用，不产生 LFCP/manifest 提交点。已经存在的不可变共享对象
   可以保留，但不得被当作部分发布成功。

同一逻辑对象键不得覆盖不同 bytes。发布后对象不可原地修改；新 compiler provenance
即使语义相同也产生新的 artifact exact digest，但可以保持同一 `NetworkRevisionId`。

## 9. 受限读取与安全失败

所有来自磁盘、网络或宿主包的对象在 hash、分配或线性解析前都必须取得调用方上限：

- 已知长度：先以 O(1) 比较 `declared/transport length <= maxObjectBytes`；
- 未知流：最多读取 `maxObjectBytes + 1`，读到第 `+1` 字节立即失败；
- 先验证 32 字节前导和目录总长度，再访问任何节；
- `objectByteLength`、外部 descriptor length 与实际受限字节长度必须三方相等；
- section/table/row/field count 各有独立上限；在验证 `count * minimumStride` 不越界前
  不按 count 分配；
- UTF-8、向量、一层内嵌记录总字节和来源位置记录数有独立累计预算；v1 不允许递归
  RecordVector；
- checked view 只在完整结构预检成功后建立，任何未验证 slice 不进入有类型 API；
- 格式错误必须稳定区分 unsupported version、limit exceeded、truncated、overflow、
  overlap/gap、unknown kind、non-canonical order/value 和 binding mismatch；错误文本不是
  稳定协议。

限制值由 #298 G2 在 P100 编译工作负载和损坏输入测试后冻结进版本化 constraints；
调用方可以选择更小上限，但不能用更大值绕过格式硬上限。

## 10. 确定性、known vectors 与验收矩阵

G1 必须冻结以下 fixture 名称和语义；G2 必须提交输入、完整 expected bytes、SHA-256、
长度和修订 ID，不允许测试在运行时用 production emitter 自己生成 expected：

| 向量 ID                         | 证明内容                                                           |
| ------------------------------- | ------------------------------------------------------------------ |
| `LFCA-V1-MIN-HEADLESS`          | 最小合法无空间 artifact、目录和空表规范化                          |
| `LFCA-V1-FULL-SPATIAL`          | 22 种实体、关系、规则、规范 f32/f64 与空间表                       |
| `LFCA-V1-PROVENANCE-ONLY`       | 同语义不同来源沿袭：revision 相同，artifact digest 不同            |
| `LFCA-V1-REORDER-EQUIVALENT`    | 声明/集合/hash iteration 重排仍产生完全相同 bytes                  |
| `LFCA-V1-SIGNED-ZERO`           | 合法输入 `-0.0` 在编译边界变为 `+0.0`；负零 wire 被读取器拒绝      |
| `LFSM-V1-MULTI-MODULE`          | 模块/文档/实体/owner-local/派生来源排序与 artifact binding         |
| `LFSD-V1-GENESIS`               | 明确 genesis 零基线与全部新增                                      |
| `LFSD-V1-CHANGE-SET`            | add/remove/reconnect/geometry/rule/identity closure 和重复值破同值 |
| `LFSD-V1-NOOP`                  | 相同 base/target 的空记录但完整 binding                            |
| `LFCP-V1-CANONICAL-PUBLICATION` | artifact/source-map/receipt/provenance 的外部精确绑定              |

确定性矩阵至少覆盖：

- Windows `x86_64-pc-windows-msvc` 本机与 Ubuntu `x86_64-unknown-linux-gnu` CI；
- single-thread 和编译器支持的所有 worker 数；
- clean process、重复运行、不同 hash seed/分配地址；
- production emitter、独立 Rust oracle 和至少一个非 Rust/脚本 oracle；
- 截断每个边界、单 bit 损坏、未知版本/节/表/字段、重复/乱序、gap/overlap、超限、
  length/digest/revision/source-map/base-target 错配；
- 暂存写入、flush/close、对象安装和 manifest 提交各失败点的无部分发布测试；
- 最小、P100 正式最高级和硬上限附近输入的发射时延、峰值暂存内存与输出大小。

完整计划和 G2 证据占位见
`../reference/v0.10-portable-artifact-validation.md`。

## 11. G1 封闭条件与当前阻断项

本文只有在下列条件全部满足后才可转为 Accepted 并发布 #298 `G1 = Pass`：

- [ ] 附录 A 登记全部 artifact/source-map/diff/descriptor table kind、field tag、类型、
      必需性、排序键与 closed enum；
- [ ] 冻结四类对象的 magic、版本、节集合、目录、数值/文本/浮点编码和错误分类；
- [ ] 冻结 NetworkRevisionId exact payload 和至少两个可人工复核的已知摘要；
- [ ] 冻结 artifact/source-map/receipt 与 base/target 的摘要+精确长度绑定；
- [ ] 冻结 `CompilationOutput` 单一输入、候选对象不可拆分成功和发布提交点；
- [ ] 冻结 pre-hash 上限、结构计数上限、硬格式上限和失败原子性；
- [ ] 用独立 oracle 证明最小向量与至少一个完整向量可仅依据规范重建；
- [ ] 记录编码库/自有格式选择的安全与维护证据；
- [ ] G1 Related PR 当前精确头取得外部 clean review，且 Project/Issue 元数据完整。

当前 Draft 故意把未闭合事项留为显式阻断项，不允许实现者自行选择字段 tag、限制值、
排序破同值或 publication 原子性语义后直接进入 G2。

## 附录 A：线格式登记表（G1 待补齐）

附录 A 将成为 `laneflow-static-contract` registry 的文档事实源，至少覆盖：

- 22 种稳定实体的规范表和完整字段；
- `CanonicalIdentityTable` 的 entity kind、typed ordinal、StableId128 与身份字段前像；
- topology、owner/member、route/path/gate/waiting occurrence、signal、parking、access、
  geometry、反向索引和静态执行约束；
- 来源模块/文档、稳定实体、owner-local relation 与推导链；
- 六类 semantic diff change record；
- LFCP 的 artifact/source-map/receipt/publication provenance 字段。

任何已有 Rust struct/view、测试 helper 或 serializer 都不是附录 A 的替代事实源。
