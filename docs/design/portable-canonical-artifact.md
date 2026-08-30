# 可移植规范制品与辅助制品格式

**文档状态**: Accepted<br>
**最后更新**: 2026-08-30<br>
**适用范围**: LFCA、LFSM、LFSD、LFCP 的权威格式、规范排序、跨对象绑定、
失败关闭与格式安全天花板<br>
**关联决策与设计**:

- [`../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`](../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md)
- [`../adr/0024-compiler-post-emission-check-and-minimal-publication-closure.md`](../adr/0024-compiler-post-emission-check-and-minimal-publication-closure.md)
- [`../adr/0025-checked-canonical-network-and-shared-static-network.md`](../adr/0025-checked-canonical-network-and-shared-static-network.md)
- [`../adr/0028-integer-millimeter-traffic-geometry.md`](../adr/0028-integer-millimeter-traffic-geometry.md)
- [`../adr/0029-retire-precompiled-static-route.md`](../adr/0029-retire-precompiled-static-route.md)
- [`parking-system.md`](parking-system.md)
- [`network-compiler.md`](network-compiler.md)
- [`compiler-post-emission-check-and-minimal-publication-closure.md`](compiler-post-emission-check-and-minimal-publication-closure.md)
- [`shared-static-network.md`](shared-static-network.md)

## 1. 权威结论

对象集合只有四种。LFCA、LFSM 与 LFSD 使用同一套分块节格式；LFCP 继续使用
非分块 singleton 节：

| 对象           | magic  | 对象版本 | 节数 | 逻辑表种类 | 作用                                               |
| -------------- | ------ | -------: | ---: | ---------: | -------------------------------------------------- |
| 可移植规范制品 | `LFCA` |        4 |    8 |         33 | 目标无关的规范静态路网语义与编译 provenance        |
| 来源映射       | `LFSM` |        3 |    5 |          8 | LFCA 实体、owner-local occurrence 与来源位置的映射 |
| 语义差异       | `LFSD` |        3 |    6 |          6 | 两个受绑定 LFCA 修订之间的可重算语义差异           |
| 规范发布描述符 | `LFCP` |        2 |    3 |          3 | LFCA/LFSM exact bytes 与发布 provenance 的最小闭合 |

格式轴固定为：

```text
canonicalFormatVersion                = 4
identityEncodingVersion               = 1
identityRegistryRevision              = 3
networkRevisionDerivationVersion      = 1
constraintContractVersion             = 2
staticExecutionContractVersion        = 4
sourceMapFormatVersion                = 3
semanticDiffFormatVersion             = 3
canonicalPublicationDescriptorVersion = 2
```

四类对象采用 LaneFlow 自有的封闭节目录格式，而不是 Serde/bincode、Rust 内存归档、
Protocol Buffers 或来源侧 FlatBuffers。这里需要的是跨语言可人工重建的 exact bytes、
封闭的 unknown-field 策略、稳定的规范排序和不依赖某个 Rust 库版本的摘要输入；来源侧
FlatBuffers 只承担未发布的 Road Editing 编制输入，不是长期规范制品编码。

本格式不授予完整语义信任、运行时迁移权限或 Adapter 行为。单对象预检证明 framing、
登记和值域，bundle 后发射检查证明摘要、修订与跨对象 binding；来源真实性仍依赖受信
compiler/发布链，LFCA 到 `SharedNetworkRevision` 的完整语义构建另有合同。LFSD 只描述
可重算变化，不能自行授权 cutover。compiler/source/publisher provenance、worker 数和资源
budget 不进入 `NetworkRevisionId`，也不得用内部 `semantic_fingerprint` 替代本格式摘要。

LFCA 4 同时原子冻结停车设施与冲突静态领域：停车实体只有 `ParkingFacility` 与
`ParkingSpace`，冲突静态实体为 `ConflictZone` 与 `ParticipantStream`，并允许与同一
修订配对的 `ConflictZoneRegion`。`ParkingArea` 不是别名、兼容入口或可接受 wire 名称。
一个设施可以同时拥有显式泊位与虚拟容量，并可以拥有多个虚拟入口和多个虚拟出口；
容量不展开成泊位、内部 LaneEdge 或容量等长记录。冲突行为只来自显式 passage，空间
region 不反向生成行为关系。

## 2. 统一线格式

### 2.1 基本规则

- 整数固定宽度、小端序；不使用 `usize`、指针或可变长整数；
- `i32` 使用 4-byte 二进制补码小端序；
- `StableId128` 是 16 个原始字节，SHA-256/`NetworkRevisionId` 是 32 个原始字节；
- `f32`/`f64` 使用 IEEE 754 little-endian bits；写入前拒绝 NaN/无穷并把负零规范为正零，
  读取时发现非有限值或负零位模式失败关闭；
- UTF-8 不做 Unicode 归一化；字段自己的来源准入规则继续约束字符和值域；
- 所有计数、乘法、`offset + length` 与宿主地址宽度转换都必须 checked；
- 不允许隐式对齐、填充、共享指针、节间空洞或尾随字节。

### 2.2 对象、节、表、行与字段

```text
ObjectPreambleV1 (32 bytes) :=
  magic[4]
  formatVersion:u16
  headerByteLength:u16 (=32)
  flags:u32 (=0)
  sectionCount:u32
  sectionDirectoryOffset:u64 (=32)
  objectByteLength:u64

SectionDirectoryEntryV1 (24 bytes) :=
  sectionKind:u16
  sectionFormatVersion:u16
  flags:u32 (=0)
  byteOffset:u64
  byteLength:u64

ChunkedSectionPreambleV1 (16 bytes) :=
  chunkCount:u32
  directoryEntryByteLength:u16 (=72)
  flags:u16 (=0)
  directoryByteLength:u64 (=16 + chunkCount * 72)

TableChunkDirectoryEntryV1 (72 bytes) :=
  tableKind:u16
  tableSchemaVersion:u16 (=1)
  chunkIndex:u32
  firstLogicalRow:u32
  rowCount:u32
  flags:u32 (=0)
  reserved:u32 (=0)
  byteOffset:u64
  byteLength:u64
  chunkDigest:Sha256

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

LFCA 4、LFSM 3 与 LFSD 3 的每个 section 使用 `sectionFormatVersion = 2`，section
exact bytes 为一个 `ChunkedSectionPreambleV1`、紧随其后的 chunk directory 和所有
`TableV1` chunk。LFCP 2 继续使用 `sectionFormatVersion = 1`，每节直接保存一张
singleton `TableV1`，不增加空洞或兼容分支。

chunk directory 按 `(tableKind, chunkIndex)` 严格递增。每个逻辑表的 `chunkIndex` 必须从
0 连续，`firstLogicalRow` 必须等于此前同 table kind 的累计 `rowCount`；第一块从 0
开始。目录中的 `rowCount`、`tableSchemaVersion` 和 `byteLength` 必须与目标 `TableV1`
头逐值相等，`chunkDigest` 是该 `TableV1` 完整 exact bytes 的 SHA-256。`byteOffset` 相对
section 起点；第一块紧随 directory，后续块紧随前块，最后一块结束位置必须精确等于
section byte length。

逻辑表为空时不发射 chunk；singleton 表必须恰有一块一行。其余表先形成唯一规范逻辑
行序，再从首行起贪心装入当前块：加入下一行后若会超过 65,536 行或 16,777,216 exact
bytes，则在该行前结束当前块。单行本身超出单块 byte 上限时失败。该算法只依赖规范行
bytes，不依赖 source module、worker 数、hash iteration、运行时分区或空间 cell。

七个基础结构的规范性 ASCII 图如下。图内从左到右、从上到下表示递增 wire byte offset；
顶部 `0..31` 是每个 4-byte 行内的 wire bit slot，多字节数值仍按小端序编码。图与随后
完整 offset/constraint 表必须一致；任一不一致都是设计 blocker，不能由实现任选其一。

```text
An ObjectPreambleV1 is formatted as follows:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                             Magic                             |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |      Format Version (FV)      |  Header Byte Length (HL)      |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                             Flags                             |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      Section Count (SC)                       |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                Section Directory Offset (SDO)                 +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                   Object Byte Length (OBL)                    +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

 A SectionDirectoryEntryV1 is formatted as follows:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |       Section Kind (SK)       | Section Format Version (SFV)  |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                             Flags                             |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                       Byte Offset (BO)                        +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                       Byte Length (BL)                        +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

 A ChunkedSectionPreambleV1 is formatted as follows:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                       Chunk Count (CC)                        |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    | Dir Entry Byte Length (DEBL)  |           Flags (F)           |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                 Directory Byte Length (DBL)                   +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

 A TableChunkDirectoryEntryV1 is formatted as follows:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |        Table Kind (TK)        |  Table Schema Version (TSV)   |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                       Chunk Index (CI)                        |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                  First Logical Row (FLR)                      |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                        Row Count (RC)                         |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                           Flags (F)                           |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                         Reserved (R)                          |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                       Byte Offset (BO)                        +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                       Byte Length (BL)                        +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                                                               +
    |                                                               |
    +                                                               +
    |                                                               |
    +                                                               +
    |                                                               |
    +                       Chunk Digest (CD)                       +
    |                                                               |
    +                                                               +
    |                                                               |
    +                                                               +
    |                                                               |
    +                                                               +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

 A TableV1 is formatted as follows:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |        Table Kind (TK)        |  Table Schema Version (TSV)   |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                        Row Count (RC)                         |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                    Rows Byte Length (RSBL)                    +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               :
    :                            [Rows]                             :
    :                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

 A RowV1 is formatted as follows:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                     Row Byte Length (RBL)                     +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                       Field Count (FC)                        |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                           Reserved                            |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               :
    :                           [Fields]                            :
    :                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

 A FieldV1 is formatted as follows:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |        Field Tag (FT)         |Field Type (T) |     Flags     |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    +                    Value Byte Length (VBL)                    +
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               :
    :                             Value                             :
    :                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

完整 offset/constraint 登记如下；offset 均相对当前结构起点：

| 结构                         | offset |   宽度 | 字段                       | 约束                                   |
| ---------------------------- | -----: | -----: | -------------------------- | -------------------------------------- |
| `ObjectPreambleV1`           | `0x00` |      4 | `magic`                    | 对象专用 ASCII magic                   |
|                              | `0x04` |      2 | `formatVersion`            | LFCA=4，LFSM/LFSD=3，LFCP=2            |
|                              | `0x06` |      2 | `headerByteLength`         | `32`                                   |
|                              | `0x08` |      4 | `flags`                    | `0`                                    |
|                              | `0x0c` |      4 | `sectionCount`             | 对象登记节数                           |
|                              | `0x10` |      8 | `sectionDirectoryOffset`   | `32`                                   |
|                              | `0x18` |      8 | `objectByteLength`         | 外部受限 reader 观察到的 exact bytes   |
| `SectionDirectoryEntryV1`    | `0x00` |      2 | `sectionKind`              | 对象专用封闭枚举，严格递增             |
|                              | `0x02` |      2 | `sectionFormatVersion`     | LFCA/LFSM/LFSD=`2`；LFCP=`1`           |
|                              | `0x04` |      4 | `flags`                    | `0`                                    |
|                              | `0x08` |      8 | `byteOffset`               | 第一节紧随目录，后续节紧随前节         |
|                              | `0x10` |      8 | `byteLength`               | checked、不得越界                      |
| `ChunkedSectionPreambleV1`   | `0x00` |      4 | `chunkCount`               | section 内全部物理 chunk 数            |
|                              | `0x04` |      2 | `directoryEntryByteLength` | `72`                                   |
|                              | `0x06` |      2 | `flags`                    | `0`                                    |
|                              | `0x08` |      8 | `directoryByteLength`      | `16 + chunkCount * 72`                 |
| `TableChunkDirectoryEntryV1` | `0x00` |      2 | `tableKind`                | 所属 section 的封闭逻辑表登记          |
|                              | `0x02` |      2 | `tableSchemaVersion`       | 与目标 `TableV1` 相等                  |
|                              | `0x04` |      4 | `chunkIndex`               | 同 table kind 从 0 连续                |
|                              | `0x08` |      4 | `firstLogicalRow`          | 同 table kind 的累计此前行数           |
|                              | `0x0c` |      4 | `rowCount`                 | 与目标 `TableV1` 相等                  |
|                              | `0x10` |      4 | `flags`                    | `0`                                    |
|                              | `0x14` |      4 | `reserved`                 | `0`                                    |
|                              | `0x18` |      8 | `byteOffset`               | section-relative，连续无空洞           |
|                              | `0x20` |      8 | `byteLength`               | 完整 `TableV1` exact bytes             |
|                              | `0x28` |     32 | `chunkDigest`              | 目标 `TableV1` exact SHA-256           |
| `TableV1`                    | `0x00` |      2 | `tableKind`                | 所属 section 的封闭登记                |
|                              | `0x02` |      2 | `tableSchemaVersion`       | `1`                                    |
|                              | `0x04` |      4 | `rowCount`                 | 完整 `RowV1` 数量                      |
|                              | `0x08` |      8 | `rowsByteLength`           | 所有完整 `RowV1` 字节长度之和          |
|                              | `0x10` | `RSBL` | `rows`                     | 连续无填充；总长 `16 + RSBL`           |
| `RowV1`                      | `0x00` |      8 | `rowByteLength`            | `16 + sum(FieldV1 exact bytes)`        |
|                              | `0x08` |      4 | `fieldCount`               | 完整 `FieldV1` 数量                    |
|                              | `0x0c` |      4 | `reserved`                 | `0`                                    |
|                              | `0x10` |   可变 | `fields`                   | tag 严格递增；总长等于 `rowByteLength` |
| `FieldV1`                    | `0x00` |      2 | `fieldTag`                 | 所属 row registry 的封闭登记，严格递增 |
|                              | `0x02` |      1 | `fieldType`                | 必须匹配登记类型                       |
|                              | `0x03` |      1 | `flags`                    | `0`                                    |
|                              | `0x04` |      8 | `valueByteLength`          | 必须匹配固定宽度或变长值约束           |
|                              | `0x0c` |  `VBL` | `value`                    | 连续无填充；总长 `12 + VBL`            |

对象目录紧随前导。第一节偏移精确等于 `32 + sectionCount * 24`；节按 kind 严格递增、
无重复、无间隙覆盖到 `objectByteLength`。chunk entry、逻辑行和字段分别按上述
chunk/table 行键/tag 严格递增。未知、额外或重复的节、逻辑表种类、chunk、必需字段
和字段类型，以及缺失的 required singleton 全部失败关闭；登记为可空的逻辑表以零
chunk 表达，不伪造空 `TableV1`。

字段类型登记为：`1=u8`、`2=u16`、`3=u32`、`4=u64`、`5=f32`、`6=f64`、
`7=StableId128`、`8=Sha256`、`9=Utf8`、`10=Bytes`、`11=OrdinalVectorU32`、
`12=RecordVector`、`13=i32`。`OrdinalVectorU32` 的 value 为
`count:u32 || item:u32[count]`；`RecordVector` 为 `count:u32 || RowV1[count]`，只允许
登记表明确声明的一层内嵌行。

## 3. LFCA 4

### 3.1 节登记

| sectionKind | 名称                         | 进入规范语义载荷 | 内容                                                   |
| ----------- | ---------------------------- | ---------------- | ------------------------------------------------------ |
| `0x0001`    | `ContractVersions`           | 是               | 格式、identity、修订派生、constraint 与 execution 版本 |
| `0x0002`    | `CanonicalIdentityTable`     | 是               | 完整 identity 前像、`StableId128` 与 typed ordinal     |
| `0x0003`    | `CanonicalEntityTables`      | 是               | 23 种可构造静态实体；kind `1..=23` 连续                |
| `0x0004`    | `CanonicalRelationTables`    | 是               | 不能由实体字段直接表达的规范关系                       |
| `0x0005`    | `CanonicalSpatialTables`     | 是               | 空间存在、规范折线与派生采样                           |
| `0x0006`    | `StaticExecutionConstraints` | 是               | worker-count-neutral 的静态执行约束                    |
| `0x0007`    | `CompilerProvenance`         | 否               | compiler/source/options/emitter provenance             |
| `0x0008`    | `ArtifactClaims`             | 否               | 声明的 `NetworkRevisionId`，仅供独立重算比较           |

八节必须全部存在。headless 用 `SpatialPresence.spatialPresent = 0` 表达，不删除空间节。

LFCA 4 的对象组合图如下；第一节 wire offset 固定为 `0x00e0`（`32 + 8 * 24`）：

```text
    +---------------------------------------------------------------+
    |           LFCA ObjectPreambleV1 (32 bytes; SC == 8)           |
    +---------------------------------------------------------------+
    |            SectionDirectoryEntryV1[8] (192 bytes)             |
    +===============================================================+
    |                                                               :
    :           ContractVersions (0x0001; variable bytes)           :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :        CanonicalIdentityTable (0x0002; variable bytes)        :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :        CanonicalEntityTables (0x0003; variable bytes)         :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :       CanonicalRelationTables (0x0004; variable bytes)        :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :        CanonicalSpatialTables (0x0005; variable bytes)        :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :      StaticExecutionConstraints (0x0006; variable bytes)      :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :          CompilerProvenance (0x0007; variable bytes)          :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :             ArtifactClaims (0x0008; variable bytes)           :
    :                                                               |
    +---------------------------------------------------------------+
```

### 3.2 合同版本表

`ContractVersions(0x0001)` 只有一张 `ContractVersions(0x0001)` singleton：

```text
1:canonicalFormatVersion:u16:R                 (=4)
2:identityEncodingVersion:u16:R                (=1)
3:identityRegistryRevision:u16:R               (=3)
4:networkRevisionDerivationVersion:u16:R       (=1)
5:constraintContractVersion:u16:R              (=2)
6:staticExecutionContractVersion:u16:R         (=4)
```

### 3.3 Identity v1 / registry revision 3

`CanonicalIdentity(0x0001)` 行键为 `(entityKind, typedOrdinal)`：

```text
1:entityKind:u16:R
2:typedOrdinal:u32:R
3:stableId:StableId128:R
4:identityFields:RecordVector:R
```

`identityFields` 内嵌行为
`1:identityFieldTag:u16:R, 2:value:Bytes:R`，并按该实体登记的 tag 顺序保存：

```text
identityCanonicalBytesV1 :=
  "LFID"
  || identityEncodingVersion:u16 (=1)
  || entityKind:u16
  || identityFieldCount:u16
  || for each registered identity field:
       identityFieldTag:u16
       valueByteLength:u32
       valueBytes[valueByteLength]

StableId128 := first-16-bytes(
  BLAKE3("laneflow.stable-id.v1\0" || identityCanonicalBytesV1)
)
```

实体 kind 登记：

```text
 1 RoadCorridor       2 RoadSection        3 AuthoringLane
 4 LaneEdge           5 Junction           6 Movement
 7 ManeuverPath       8 ManeuverGate       9 WaitingZone
10 StopLine          11 SignalGroup       12 SignalController
13 SignalPhase       14 ParkingFacility   15 ParkingSpace
16 LaneGroup         17 FacilityBand      18 ParticipantClass
19 AccessRule        20 VehicleProfile    21 ConflictZone
22 CanonicalFrame    23 ParticipantStream
```

revision 3 的 identity field tag 连续登记为 `1..=34`；tag 22 的名称为
`parkingFacilityKey`，tag 23/30 分别为 `conflictZoneKey` / `participantStreamKey`。
主要字段为：

```text
 1 authoringNamespaceId  2 corridorKey       3 sectionKey
 4 laneKey               5 laneEdgeKey       6 junctionKey
 7 pathKey               8 movementKey       9 directedEntryApproachKey
10 directedExitApproachKey                   11 movementStableId
12 entryEdgeStableId     13 exitEdgeStableId 14 maneuverPathStableId
15 gateKey               16 waitingZoneKey   17 stopLineKey
18 signalGroupKey        19 signalControllerKey
20 signalControllerStableId                  21 phaseKey
22 parkingFacilityKey    23 conflictZoneKey  24 parkingSpaceKey
25 laneGroupKey          26 facilityBandKey  27 participantClassKey
28 accessRuleKey         29 vehicleProfileKey
30 participantStreamKey  31 canonicalFrameKey
32 roadSectionStableId   33 roadCorridorStableId
34 junctionStableId
```

全部可构造 kind 的 required tag sequence 为：

```text
 1 RoadCorridor      [1,2]             2 RoadSection       [1,3,33]
 3 AuthoringLane     [1,4,32]          4 LaneEdge          [1,5]
 5 Junction          [1,6]             6 Movement          [1,8,9,10,34]
 7 ManeuverPath      [1,7,11,12,13]    8 ManeuverGate      [1,14,15]
 9 WaitingZone       [1,14,16]        10 StopLine          [1,17]
11 SignalGroup       [1,18]           12 SignalController  [1,19]
13 SignalPhase       [1,20,21]        14 ParkingFacility   [1,22]
15 ParkingSpace      [1,24]           16 LaneGroup         [1,25,32]
17 FacilityBand      [1,26,33]        18 ParticipantClass  [1,27]
19 AccessRule        [1,28]           20 VehicleProfile    [1,29]
21 ConflictZone      [1,23,34]        22 CanonicalFrame    [1,31]
23 ParticipantStream [1,30,34]
```

kind `1..=23` 与 identity tag `1..=34` 都是连续的现行登记；历史 `StaticRoute` / `routeKey`
不再保留编号墓碑，也不得由名字、旧字段形状或旧对象版本复活。上表中的方括号数字全部是
identity field tag。

新增 kind 必须提升 `identityRegistryRevision`，但不得改变既有 kind 的 identity canonical
bytes 或 `StableId128`。只有修改既有 kind 的 required field、tag 含义或编码时才提升
`identityEncodingVersion`。`ParticipantStream.maneuverPath` 不进入身份前像；重新归属
Junction 必须产生新 identity。

`Ascii` identity value 必须为 1..=53 bytes；首 byte 属于 `[A-Za-z0-9]`，其余只属于
`[A-Za-z0-9._:/-]`。对每个 kind，发射器按完整 identity bytes 的无符号逐字节字典序
排序并从 0 连续分配 typed ordinal。全部可构造 kind 的 `StableId128` 必须全局唯一；
重复前像和 BLAKE3-128 截断碰撞都失败关闭。

`ParkingFacility` 固定使用 kind 14 / tag 22；数值和相同 namespace+key 的前像字节不变。
LFCA 不接受 `ParkingArea` wire 名称或 alias；设施语义变化会产生不同的
`NetworkRevisionId`。

### 3.4 实体表登记

`CanonicalEntityTables(0x0003)` 精确包含下列 23 个连续逻辑表种类。
每行共同以
`1:typedOrdinal:u32:R, 2:stableId:StableId128:R` 开始：

| tableKind | 表名              | tag 3 起的字段                                                                                                                                                                                                                                                                                                     |
| --------- | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `0x0001`  | RoadCorridor      | `3:referenceSection:u32:R, 4:elements:RecordVector:R`                                                                                                                                                                                                                                                              |
| `0x0002`  | RoadSection       | `3:roadCorridor:u32:R, 4:kindId:Utf8:R, 5:lanes:OrdinalVectorU32:R`                                                                                                                                                                                                                                                |
| `0x0003`  | AuthoringLane     | `3:roadSection:u32:R, 4:edgeChain:OrdinalVectorU32:R, 5:laneGroup:u32:O`                                                                                                                                                                                                                                           |
| `0x0004`  | LaneEdge          | `3:lengthMillimetres:u32:R, 4:speedLimitMillimetresPerSecond:u32:R, 5:successors:OrdinalVectorU32:R`                                                                                                                                                                                                               |
| `0x0005`  | Junction          | `3:movements:OrdinalVectorU32:R`                                                                                                                                                                                                                                                                                   |
| `0x0006`  | Movement          | `3:junction:u32:R, 4:directedEntryApproachKey:Utf8:R, 5:directedExitApproachKey:Utf8:R, 6:maneuverPaths:OrdinalVectorU32:R`                                                                                                                                                                                        |
| `0x0007`  | ManeuverPath      | `3:movement:u32:R, 4:edges:OrdinalVectorU32:R, 5:maneuverGates:OrdinalVectorU32:R, 6:waitingZones:OrdinalVectorU32:R`                                                                                                                                                                                              |
| `0x0008`  | ManeuverGate      | `3:maneuverPath:u32:R, 4:transitionIndex:u32:R, 5:stopLine:u32:R, 6:signalControlKind:u8:R, 7:signalGroup:u32:O`                                                                                                                                                                                                   |
| `0x0009`  | WaitingZone       | `3:maneuverPath:u32:R, 4:entryGate:u32:R, 5:releaseGate:u32:R, 6:maxOccupancy:u32:R`                                                                                                                                                                                                                               |
| `0x000a`  | StopLine          | `3:laneEdge:u32:R, 4:maneuverGates:OrdinalVectorU32:R`                                                                                                                                                                                                                                                             |
| `0x000b`  | SignalGroup       | `3:controller:u32:R, 4:maneuverGates:OrdinalVectorU32:R`                                                                                                                                                                                                                                                           |
| `0x000c`  | SignalController  | `3:offsetMs:u64:R, 4:cycleDurationMs:u64:R, 5:signalGroups:OrdinalVectorU32:R, 6:phases:OrdinalVectorU32:R`                                                                                                                                                                                                        |
| `0x000d`  | SignalPhase       | `3:controller:u32:R, 4:durationMs:u64:R, 5:states:RecordVector:R`                                                                                                                                                                                                                                                  |
| `0x000e`  | ParkingFacility   | `3:parkingSpaces:OrdinalVectorU32:R, 4:virtualCapacity:u32:R, 5:virtualEntries:RecordVector:R, 6:virtualExits:RecordVector:R`                                                                                                                                                                                      |
| `0x000f`  | ParkingSpace      | `3:parkingFacility:u32:O, 4:entryLaneEdge:u32:R, 5:entryProgressMillimetres:u32:R, 6:exitLaneEdge:u32:R, 7:exitProgressMillimetres:u32:R, 8:lateralOffsetMillimetres:i32:R, 9:headingOffsetRadians:f32:R, 10:lengthMillimetres:u32:R, 11:widthMillimetres:u32:R`                                                   |
| `0x0010`  | LaneGroup         | `3:roadSection:u32:R, 4:members:OrdinalVectorU32:R`                                                                                                                                                                                                                                                                |
| `0x0011`  | FacilityBand      | `3:roadCorridor:u32:R, 4:kindId:Utf8:R`                                                                                                                                                                                                                                                                            |
| `0x0012`  | ParticipantClass  | `3:parent:u32:O, 4:depth:u32:R, 5:subtreeEnter:u32:R, 6:subtreeExit:u32:R`                                                                                                                                                                                                                                         |
| `0x0013`  | AccessRule        | `3:targetKind:u8:R, 4:targetOrdinal:u32:R, 5:effect:u8:R, 6:participantClasses:OrdinalVectorU32:R, 7:regulation:RecordVector:O, 8:priority:i32:R`                                                                                                                                                                  |
| `0x0014`  | VehicleProfile    | `3:participantClass:u32:R, 4:lengthMillimetres:u32:R, 5:desiredSpeedMillimetresPerSecond:u32:R, 6:minGapMillimetres:u32:R, 7:timeHeadwaySeconds:f32:R, 8:maxAccelerationMetersPerSecondSquared:f32:R, 9:comfortableDecelerationMetersPerSecondSquared:f32:R, 10:emergencyDecelerationMetersPerSecondSquared:f32:R` |
| `0x0015`  | ConflictZone      | `3:junction:u32:R`                                                                                                                                                                                                                                                                                                 |
| `0x0016`  | CanonicalFrame    | 无额外字段                                                                                                                                                                                                                                                                                                         |
| `0x0017`  | ParticipantStream | `3:junction:u32:R, 4:maneuverPath:u32:R, 5:passages:RecordVector:R`                                                                                                                                                                                                                                                |

内嵌行：

```text
RoadCorridor.elements:
  1:elementKind:u8:R, 2:ordinal:u32:R

SignalPhase.states:
  1:signalGroup:u32:R, 2:aspect:u8:R

ParkingFacility.virtualEntries / virtualExits:
  1:laneEdge:u32:R, 2:progressMillimetres:u32:R

ParticipantStream.passages:
  1:conflictZone:u32:R,
  2:entryKind:u8:R, 3:entryReference:u32:R, 4:entryProgressMillimetres:u32:O,
  5:exitKind:u8:R, 6:exitReference:u32:R, 7:exitProgressMillimetres:u32:O

AccessRule.regulation:
  1:jurisdiction:Utf8:R, 2:version:Utf8:R, 3:source:Utf8:O

LaneEdgeGeometry.points / FacilityBandGeometry.points:
  1:x:f32:R, 2:y:f32:R, 3:z:f32:R

ConflictZoneRegion.ringXZ:
  1:x:f32:R, 2:z:f32:R

LaneEdgeGeometry.segments:
  1:lengthMeters:f32:R, 2:cumulativeEndMeters:f32:R,
  3:tangentX:f32:R, 4:tangentY:f32:R, 5:tangentZ:f32:R,
  6:upX:f32:R, 7:upY:f32:R, 8:upZ:f32:R
```

`AccessRule.regulation` 的 tag 7 缺失表示没有 regulation；tag 存在时其
`RecordVector` 必须恰有一行。零行、两行以上或从多行中任选一行都失败关闭。

`ParticipantStream.passages` 的 anchor kind 固定为 `0=Gate, 1=EdgeBoundary, 2=Interior`。
Gate reference 是同一 stream ManeuverPath 的 `ManeuverGate` typed ordinal；EdgeBoundary
reference 是 `0..=pathEdgeCount` 的 boundary index；Interior reference 是
`0..<pathEdgeCount` 的 path edge index，且对应 progress tag 必需并满足
`0 < progressMillimetres < edgeLengthMillimetres`。非 Interior 禁止 progress tag。

三种 variant 先映射到同一规范路径位置键，再比较 entry/exit 和排序 passage：

```text
Gate(transitionIndex=t)            -> (pathEdgeIndex=t+1, progressMillimetres=0)
EdgeBoundary(boundaryIndex=b)      -> (pathEdgeIndex=b,   progressMillimetres=0)
Interior(pathEdgeIndex=i, progress=p) -> (pathEdgeIndex=i, progressMillimetres=p)
```

位置键按两个无符号整数的字典序比较。`boundaryIndex == pathEdgeCount` 是路径末端哨兵；
Interior 已排除 0 和 edge length，因此不会与边界重合。若 boundary `b` 同时存在唯一
`transitionIndex=b-1` 的 ManeuverGate，该位置只能编码为 `Gate`；只有该 boundary 没有
Gate 时才允许 `EdgeBoundary`。官方来源准入必须规范化为这一表示，LFCA reader 对另一种
等价 variant 失败关闭；不得让 Gate ordinal 决定路径顺序。admission Gate 必须从规范化
entry 位置唯一派生，不进入 LFCA passage row。

上表连同 `CanonicalIdentity.identityFields` 已穷举 LFCA 4 的全部 `RecordVector` 行布局；
这些内嵌行均不得再含 `RecordVector`。任何未登记的内嵌字段、额外 tag 或第二层嵌套失败关闭。

#### 3.4.1 身份、ordinal 与所有权闭合

每个可构造 kind 的 `CanonicalIdentity` 逻辑行必须与同 kind 实体逻辑表形成严格双射。
`typedOrdinal` 从 0 连续，identity 行与实体行的 `(kind, typedOrdinal, stableId)` 必须逐值
相等；所有 scalar/vector ordinal 必须先按登记的目标 kind 解析，不能只检查它小于某张
表的行数。分块只改变物理位置，不改变这组全局 ordinal，也不得让同一逻辑表在 chunk
边界重新从 0 编号。

Identity 前像中重复表达的所有权或边界语义必须与实体行严格相等：

| entity            | Identity 字段                         | 必须等于的 LFCA 投影                      |
| ----------------- | ------------------------------------- | ----------------------------------------- |
| RoadSection       | `roadCorridorStableId`                | tag 3 `roadCorridor` 解析出的 StableId    |
| AuthoringLane     | `roadSectionStableId`                 | tag 3 `roadSection` 解析出的 StableId     |
| Movement          | `junctionStableId`                    | tag 3 `junction` 解析出的 StableId        |
| Movement          | `directedEntry/ExitApproachKey`       | tag 4/5 的 exact ASCII bytes              |
| ManeuverPath      | `movement/entryEdge/exitEdgeStableId` | tag 3 与 tag 4 首项/末项解析出的 StableId |
| ManeuverGate      | `maneuverPathStableId`                | tag 3 `maneuverPath` 解析出的 StableId    |
| WaitingZone       | `maneuverPathStableId`                | tag 3 `maneuverPath` 解析出的 StableId    |
| SignalPhase       | `signalControllerStableId`            | tag 3 `controller` 解析出的 StableId      |
| LaneGroup         | `roadSectionStableId`                 | tag 3 `roadSection` 解析出的 StableId     |
| FacilityBand      | `roadCorridorStableId`                | tag 3 `roadCorridor` 解析出的 StableId    |
| ConflictZone      | `junctionStableId`                    | tag 3 `junction` 解析出的 StableId        |
| ParticipantStream | `junctionStableId`                    | tag 3 `junction` 解析出的 StableId        |

规范 vector/record-vector 的顺序不是 writer 选择：

| 字段                            | 规范顺序与空值规则                                                                  |
| ------------------------------- | ----------------------------------------------------------------------------------- |
| `RoadCorridor.elements`         | 领域顺序；非空、无重复                                                              |
| `RoadSection.lanes`             | 区段横向领域顺序；非空、无重复                                                      |
| `AuthoringLane.edgeChain`       | 行驶方向领域顺序；非空、无重复                                                      |
| `LaneEdge.successors`           | target ordinal 严格递增集合；可空                                                   |
| `Junction.movements`            | member ordinal 严格递增集合；非空                                                   |
| `Movement.maneuverPaths`        | member ordinal 严格递增集合；非空                                                   |
| `ManeuverPath.edges`            | 完整 entry/internal/exit occurrence 领域顺序；至少两项，允许同 edge 重复 occurrence |
| `ManeuverPath.maneuverGates`    | 按 `transitionIndex` 严格递增；可空                                                 |
| `ManeuverPath.waitingZones`     | 按 entry/release transition tuple 严格递增；可空                                    |
| `StopLine.maneuverGates`        | member ordinal 严格递增集合；非空                                                   |
| `SignalGroup.maneuverGates`     | member ordinal 严格递增集合；非空                                                   |
| `SignalController.signalGroups` | member ordinal 严格递增集合；非空                                                   |
| `SignalController.phases`       | 固定时制程序领域顺序；非空、无重复                                                  |
| `SignalPhase.states`            | `signalGroup` ordinal 严格递增集合，精确覆盖 controller 的全部 groups               |
| `ParkingFacility.parkingSpaces` | member ordinal 严格递增集合；可空，完整规则见 §3.5                                  |
| `LaneGroup.members`             | 保持所属 `RoadSection.lanes` 的相对顺序；非空、无重复                               |
| `AccessRule.participantClasses` | member ordinal 严格递增集合；非空                                                   |

重复所有权必须双向互证，而不是只让两侧分别可解析：

| owner 侧                               | child/back-reference 侧                      | 必需不变量                                                               |
| -------------------------------------- | -------------------------------------------- | ------------------------------------------------------------------------ |
| `RoadCorridor.elements`                | `RoadSection/FacilityBand.roadCorridor`      | child 全集相等，每个 child 恰有一个 corridor owner                       |
| `RoadCorridor.referenceSection`        | 同一 `elements` 的 RoadSection               | reference 必须是该 corridor 的 section member                            |
| `RoadSection.lanes`                    | `AuthoringLane.roadSection`                  | 双向全集相等，每条 lane 恰有一个 section owner                           |
| `LaneGroup.members`                    | `AuthoringLane.laneGroup`                    | optional back-reference 当且仅当 membership 存在，且双方属于同一 section |
| `Junction.movements`                   | `Movement.junction`                          | 双向全集相等，每个 movement 恰有一个 junction owner                      |
| `Movement.maneuverPaths`               | `ManeuverPath.movement`                      | 双向全集相等，每个 path 恰有一个 movement owner                          |
| `ManeuverPath.maneuverGates`           | `ManeuverGate.maneuverPath/transitionIndex`  | 双向全集相等，vector 顺序与 transitionIndex 一致                         |
| `ManeuverPath.waitingZones`            | `WaitingZone.maneuverPath`                   | 双向全集相等，每个 zone 恰有一个 path owner                              |
| `StopLine.maneuverGates`               | `ManeuverGate.stopLine`                      | 双向全集相等，每个 gate 恰引用一个 stop line                             |
| `SignalController.signalGroups/phases` | `SignalGroup/SignalPhase.controller`         | 两个 child 集合分别双向全集相等                                          |
| `SignalGroup.maneuverGates`            | `ManeuverGate.signalControlKind/signalGroup` | Group 时恰在该 group；None 时不在任何 group                              |

任一遗漏 child、额外 member、多 owner、错误 owner，或 optional back-reference 与 membership
不一致都在建立语义视图前失败关闭。

#### 3.4.2 标量、拓扑与派生缓存闭合

内嵌闭合枚举为：`RoadCorridor.elementKind = 0 RoadSection / 1 FacilityBand`；
`signalControlKind = 0 None / 1 Group`，且 group 字段当且仅当值为 1 时存在；
`SignalPhase.aspect = 0 Red / 1 Yellow / 2 Green`；
`AccessRule.targetKind = 0 LaneEdge / 1 LaneGroup / 2 RoadSection / 3 ManeuverPath`；
`AccessRule.effect = 0 Deny / 1 Allow`。任何其他 discriminant 失败关闭。

`kindId` 的完整 bytes 必须是非空 ASCII，并匹配
`[A-Za-z0-9][A-Za-z0-9._:/-]*`；扩展前缀后的 suffix 也必须独立匹配同一语法，不能只有
前缀。`RoadSection.kindId` 只允许 `motorLane`、`nonMotorLane` 或 `x-lane-` 加非空后缀；
`FacilityBand.kindId` 只允许 `sidewalk`、`median`、`plantingStrip`、`facilityStrip`、
`shoulder` 或 `x-` 加非空后缀，但 `x-lane-` 仍属于 RoadSection。两者都先通过非空 ASCII
token 语法。整数毫米、停车、VehicleProfile 与信号计时的逐字段闭包以
[`traffic-runtime-integer-geometry.md`](traffic-runtime-integer-geometry.md) 和
[`numeric-representation.md`](numeric-representation.md) 的现行表为权威，LFCA reader
必须执行同一闭包；这些引用不是只约束编译输入。特别地：边长 `100..=10000000 mm`、
限速 `1..=100000 mm/s`、停车 progress 相对提交后边长两端各留 1 mm、存储 `+π` 非法、
所有信号 duration/cycle checked sum 与 offset 均受 `9007199254740991 ms` 上限约束。

ManeuverPath 全部中间 edge occurrence 的去重集合必须与 `JunctionInternalEdge` 精确闭合；
boundary edge 不得同时是 internal edge，internal edge 不得归属不同 Junction。
`LaneEdge.successors` 是过滤掉任一端为 internal edge 的唯一 source-model-neutral 投影；
其余 AuthoringLane 邻接必须由 successor 或至少一条 ManeuverPath 相邻 occurrence 覆盖。
每条 LaneEdge 至多属于一个 `AuthoringLane.edgeChain`；完整 ManeuverPath edge StableId
occurrence 序列在整个修订中唯一，序列内允许重复 edge，但两条 path 不能拥有完全相同序列。

每个 ManeuverGate 满足
`transitionIndex < maneuverPath.edges.count - 1`，且 StopLine edge 等于对应 transition
前项；同一路径 transitionIndex 不重复。每条 LaneEdge 至多一条 StopLine；每条 StopLine
至少关联一个 gate，且所在 edge 必须有显式 successor 或属于 ManeuverPath transition。
若 StopLine 存在 transitionIndex=0 的 entry gate，从该 edge 出发的每个显式 successor 必须
由唯一完整 ManeuverPath 覆盖，且每条该 path 都有自己的 transitionIndex=0 gate。
WaitingZone 的 `maxOccupancy > 0`；entry/release gate 必须属于同一路径且 entry index
严格小于 release index；同一路径半开区间 `[entry, release)` 不重叠。SignalGroup 至少控制
一个 Group gate。

`SignalController.cycleDurationMs` 必须等于按 phase 领域顺序 checked-sum 的全部
`durationMs`，`offsetMs < cycleDurationMs`。`ParticipantClass` 必须形成无环单继承森林：
根 depth 为 0，child depth 是 parent+1；按 typed ordinal 递增的根和同父 children 做确定性
DFS 前序，`subtreeEnter/subtreeExit` 必须逐值等于重算结果。interval 只是派生缓存，不能成为
第二份层级权威。AccessRule 在解析 target/class StableId 后，同一
`(accessPlane, targetKind, targetStableId, participantClassStableId, priority)` 分组不得同时
出现 Allow 与 Deny；LaneEdge/LaneGroup/RoadSection 属于 Edge plane，ManeuverPath 属于
ManeuverPath plane，同组允许多条相同 effect。regulation 的 jurisdiction/version 与 optional
source 各含 1..=128 个 Unicode scalar，全对象 regulation 的 `(jurisdiction, version)` 必须
一致，但各行 source 可以不同。

### 3.5 停车设施闭合

对每个 `ParkingFacility`：

1. `parkingSpaces.len + virtualCapacity > 0`；
2. `virtualCapacity == 0` 时，`virtualEntries` 与 `virtualExits` 都为空；
3. `virtualCapacity > 0` 时，两组 anchor 都非空；
4. 两组分别按 `(LaneEdge StableId128, progressMillimetres)` 严格递增且无重复；
5. anchor edge 必须存在，且相对提交后边长 `L` 满足
   `1 <= progressMillimetres <= L - 1`（两端各留 `1 mm`）；
6. `parkingSpaces` 按 member typed ordinal 严格递增，与每个
   `ParkingSpace.parkingFacility` 的可选正向引用全集互证；
7. 总容量只以 `u64::from(parkingSpaces.len) + u64::from(virtualCapacity)` 派生，不保存
   第二份 `totalCapacity`；
8. virtual capacity 不产生 `ParkingSpace`、几何、空 slot、静态 Route 或设施内部路网。

显式泊位与虚拟容量是同一设施内两个独立 admission pool；占用一个显式泊位不消耗
virtual capacity。显式泊位仍以自己的 entry/exit/geometry 为权威，设施 anchors 只服务
virtual pool。

### 3.6 冲突静态闭合

`ConflictZone` 与 `ParticipantStream` 是稳定实体；`ConflictPassage` 是
`ParticipantStream` owner-local row，不获得独立 `StableId128`。行为 authority 只来自
`ParticipantStream.passages`：

1. stream、其 ManeuverPath、每个 passage zone 与 identity 前像中的 Junction 必须一致；
2. 每个 stream 至少一条 passage；同一 `(ParticipantStream, ConflictZone)` 至多一条；
3. passage entry 必须严格早于 exit，且不得跨越 admission Gate 之后的下一 Gate；
4. passage 按 entry anchor、exit anchor、ConflictZone StableId 严格递增；该 local index
   是唯一 owner-local 地址；
5. 每个 ConflictZone 必须被至少两个不同 stream passage 引用；反向 stream 集合只从
   passages 派生，不进入 zone wire；
6. WaitingZone interior 与 ConflictPassage interior 不得重叠，精确共享边界合法；
7. geometry、2D 投影、mesh、collider 与 Adapter overlap 不得增加、删除或合并 passage。

`ParticipantStream.passages` 不保存 admission Gate；compiler、checker 与
`SharedNetworkRevision` 必须从 entry anchor 和同一路径 Gate sequence 独立导出并逐值
一致。共享根可以保存派生 Gate coverage 与 zone-stream CSR，但这些不是第二份 LFCA
authority。

### 3.7 其余 LFCA 表

`CanonicalRelationTables(0x0004)`：

```text
JunctionInternalEdge(0x0001):
  1:laneEdge:u32:R, 2:junction:u32:R
```

`CanonicalSpatialTables(0x0005)`：

```text
SpatialPresence(0x0001; singleton):
  1:spatialPresent:u8:R, 2:geometryDirectionProfile:u8:R

LaneEdgeGeometry(0x0002):
  1:laneEdge:u32:R, 2:canonicalFrame:u32:R, 3:arcLengthMeters:f32:R,
  4:points:RecordVector:R, 5:segments:RecordVector:R,
  6:directionProfileApplies:u8:R

FacilityBandGeometry(0x0003):
  1:facilityBand:u32:R, 2:canonicalFrame:u32:R,
  3:points:RecordVector:R, 4:directionProfileApplies:u8:R

ConflictZoneRegion(0x0004):
  1:conflictZone:u32:R, 2:canonicalFrame:u32:R,
  3:minY:f32:R, 4:maxY:f32:R, 5:ringXZ:RecordVector:R
```

虚拟停车设施没有 geometry/pose 表；显式泊位几何继续由停车领域与 Spatial 受检投影拥有。

`spatialPresent` 只允许 `0/1`。从 wire 重建：`hasProfile` 当且仅当
`geometryDirectionProfile != 0`，`hasFrame` 当且仅当 CanonicalFrame 实体表非空，另外三项
分别表示 LaneEdgeGeometry/FacilityBandGeometry/ConflictZoneRegion 表非空；
`spatialPresent` 必须精确等于五项逻辑或。闭合矩阵为：

| 条件               | `geometryDirectionProfile`                         | CanonicalFrame | LaneEdgeGeometry                                    | FacilityBandGeometry                               | ConflictZoneRegion                                 |
| ------------------ | -------------------------------------------------- | -------------- | --------------------------------------------------- | -------------------------------------------------- | -------------------------------------------------- |
| `spatialPresent=0` | 必须 `0=None`                                      | 必须空         | 必须空                                              | 必须空                                             | 必须空                                             |
| `spatialPresent=1` | `0..=3`；至少 profile/frame/三张 geometry 之一存在 | 可空           | 可空；非空时必须完整覆盖全部 LaneEdge typed ordinal | 可空；非空时按 FacilityBand ordinal 为唯一规范子集 | 可空；非空时按 ConflictZone ordinal 为唯一规范子集 |

profile code 固定为 `0=None, 1=Smooth1Deg, 2=Balanced2Deg, 3=Compact5Deg`。
`directionProfileApplies` 只允许 `0/1`：全局 code 为 0 时所有 geometry 行必须为 0；任一
行值为 1 时全局 code 必须非零；全局 code 非零而某行值为 0 是合法混合来源状态。
`CompilerProvenance.geometryAccuracyProfile` 与全局 direction profile 的零/非零 presence
必须一致。这样 headless semantic bytes 不会携带被 SharedNetwork 丢弃的空间事实。

`StaticExecutionConstraints(0x0006)`、`CompilerProvenance(0x0007)` 与
`ArtifactClaims(0x0008)` 分别只有一张 singleton：

```text
ExecutionContract(0x0001):
  1:staticExecutionContractVersion:u16:R,
  2:constraintContractVersion:u16:R

CompilerProvenance(0x0001):
  1:compilerBuildId:Utf8:R,
  2:sourceCollectionDigestVersion:u16:R,
  3:sourceCollectionDigest:Sha256:R,
  4:compileOptionsDigest:Sha256:R,
  5:emitterVersion:u16:R,
  6:geometryAccuracyProfile:u8:R

ArtifactClaims(0x0001):
  1:declaredNetworkRevisionId:Sha256:R
```

`ExecutionContract` tag 1/2 必须分别逐值等于 `ContractVersions` 的
`staticExecutionContractVersion/constraintContractVersion`；任一副本不一致都失败关闭，不能由
reader 选择其中一份作为权威。

`CompilerProvenance` 的直接值域同样是格式合同：

- `compilerBuildId` 是 1..=128-byte ASCII，匹配
  `[A-Za-z0-9][A-Za-z0-9._+@-]{0,127}`；路径、target triple、时间、worker 数、机器或
  随机 nonce 不得进入；
- `sourceCollectionDigestVersion=1`，digest 必须按 §4.1 的 exact framing 从同次 LFSM
  来源模块重算，调用方不能覆盖；
- LFCA 4 的 `emitterVersion=2`；这表示分块 emitter 合同，不是 compiler build 号；
- `geometryAccuracyProfile = 0 None / 1 Fine2Cm / 2 Balanced5Cm / 3 Compact10Cm`，并与
  `geometryDirectionProfile` 保持零/非零 presence 一致；
- `compileOptionsDigest = SHA-256("laneflow.portable-compile-options.v1\0" ||
  optionCount:u32=0)`，固定为
  `322682f455d06b36e9e3719f341db38f3ecda61d52c53d9d6fe3dca540eef445`。资源 budget、
  worker 数与 LFSD base 不进入该摘要。

LFCA provenance、LFSM bindings 与 LFCP source-map binding 中重复的
`compilerBuildId/sourceCollectionDigestVersion/sourceCollectionDigest` 必须逐字节相等；
这些副本只用于绑定，不是对象内 trust anchor。

`ConflictZoneRegion` 每个 ConflictZone 至多一行；ring 至少三个不同点，wire 不重复首点，
从 `+Y` 观察必须逆时针，首点是 `(x,z)` 词典序最小点，ring 无自交且面积为正，
`minY < maxY`。min/max Y 与 ring 坐标必须有限、使用 bit-exact 正零并位于
`[-16384, 16384] m`。region 只服务验证、调试与表现，缺失时 headless 冲突行为保持完整。

#### 3.7.1 规范几何数值闭合

预计算 segment/basis 不能成为点表之外的第二权威。`LaneEdgeGeometry.points` 至少两项，
`segments.count = points.count - 1`。所有坐标有限、位于 `[-16384, 16384] m`，零必须是
bit-exact `+0.0f32`。对相邻点按字段顺序做 binary32 减法并规范零，随后定义：

```text
HypotRteF32(a,b) = RN32(sqrt(Exact(a)^2 + Exact(b)^2))
length           = HypotRteF32(HypotRteF32(delta.x, delta.y), delta.z)
```

`Exact` 把 binary32 无损提升为精确实数，`RN32` 只在末尾执行 IEEE 754
roundTiesToEven。三维长度按上式左结合，不能用精度未冻结的平台 `hypotf`/`f32::hypot`、
FMA、重结合或额外精度改变位模式。每段 length 必须严格大于 `0.1f32 m`；
`cumulativeEndMeters` 从正零开始逐段做 binary32 加法，有限且严格递增，末项与
`arcLengthMeters` 位模式相同。令
`declared = f64(lengthMillimetres) / 1000`、`arc = f64(arcLengthMeters)`，必须满足：

```text
abs(declared - arc) <= max(0.01, 1.0e-6 * max(declared, arc))
```

每段 basis 只能从同一 delta 按以下顺序重算；每个具名普通运算都舍入到 binary32，零规范
为正零：

```text
normalize(v):
  scale = max(abs(v.x), abs(v.y), abs(v.z))
  q = v / scale
  return q / HypotRteF32(HypotRteF32(q.x, q.y), q.z)

tangent     = normalize(delta)
projectedUp = HypotRteF32(tangent.x, tangent.z)   // must be >= 0.008726535f32
left        = normalize([tangent.z, 0, -tangent.x])
up          = normalize([
  tangent.y * left.z,
  tangent.z * left.x - tangent.x * left.z,
  -tangent.y * left.x
])
```

存储的 length/cumulative/tangent/up 必须与重算结果逐 bit 相等。每个由
`LaneEdge.successors` 或任一 ManeuverPath 相邻 occurrence 导出的有向连接对只检查一次；
两端 geometry 必须使用同一 CanonicalFrame。predecessor 末点与 successor 首点按同一
binary32 delta/Hypot 规则得到的 gap 必须 `<= 0.005f32`（bits `0x3ba3d70a`），不得吸附、
焊接或移动端点。

方向谓词只在行 `directionProfileApplies=1` 时约束行内相邻弦；跨 edge 连接在任一端标记
为 1 时约束末弦/首弦。把弦的 binary32 点无损提升为 binary64，分别用最大绝对分量缩放，
再按写出顺序逐运算舍入到 binary64：

```text
dot  = (x*x' + y*y') + z*z'
norm = (x*x + y*y) + z*z
lhs  = dot * dot
rhs  = (C * norm(left)) * norm(right)
accept iff dot > 0 && lhs >= rhs
```

Smooth/Balanced/Compact 的 `C` binary64 bits 依次为 `0x3feffd813c5f82b4`、
`0x3feff605b8b87ffc`、`0x3fefc1c5c6408e0c`。禁止 FMA、重结合、`acos/cos` 或自选
epsilon。标记为 0 只跳过方向谓词，不跳过 frame、gap、点表、segment 或 arc-length
闭合。`FacilityBandGeometry.points` 至少两项，同样执行坐标、逐弦长度与适用的方向谓词，
但不发射 segments/arc-length。

`ConflictZoneRegion.ringXZ` 的规范判断不使用平台浮点几何库：把每个有限 binary32 坐标
无损提升为精确二进制有理数；词典序、orientation、非相邻边相交和 shoelace 有向面积都在
该精确域计算。首点必须是唯一词典序最小点，有向面积必须严格为正；任何重复点、非相邻边
相交/接触、共线重叠，或相邻边除共同端点外再相交都失败关闭。这样逆时针、自交与面积结论
不依赖平台 epsilon。

### 3.8 路网修订标识

规范语义载荷只连接 section `0x0001..=0x0006`：

```text
canonicalNetworkSemanticPayloadV1 :=
  for each semantic section in sectionKind order:
    sectionKind:u16
    sectionFormatVersion:u16
    sectionByteLength:u64
    sectionExactBytes[sectionByteLength]

NetworkRevisionIdV1 := SHA-256(
  "laneflow.network-revision.v1\0"
  || canonicalNetworkSemanticPayloadV1
)
```

`ArtifactClaims.declaredNetworkRevisionId` 不自证可信；后发射检查必须从最终六个语义节
重算并逐字节比较。完整 LFCA exact bytes 另做 SHA-256 得到
`canonicalArtifactDigest`，摘要和 exact `u64` length 由外部对象绑定。

## 4. LFSM 3

`magic = "LFSM"`，`sourceMapFormatVersion = 3`。LFSM 3 精确包含：

| sectionKind | 名称                     | tables                                             |
| ----------- | ------------------------ | -------------------------------------------------- |
| `0x0001`    | `SourceMapBindings`      | `SourceMapBindings` singleton                      |
| `0x0002`    | `SourceModules`          | `SourceModule`、`SourceDocument`、`SourceLocation` |
| `0x0003`    | `StableEntitySources`    | `StableEntitySource`                               |
| `0x0004`    | `OwnerLocalSources`      | `OwnerLocalSource`、`SpatialGeometrySourceRange`   |
| `0x0005`    | `DerivedRelationSources` | `DerivedRelationSource`                            |

第一节 wire offset 固定为 `0x0098`（`32 + 5 * 24`）：

```text
    +---------------------------------------------------------------+
    |           LFSM ObjectPreambleV1 (32 bytes; SC == 5)           |
    +---------------------------------------------------------------+
    |            SectionDirectoryEntryV1[5] (120 bytes)             |
    +===============================================================+
    |                                                               :
    :          SourceMapBindings (0x0001; variable bytes)           :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :            SourceModules (0x0002; variable bytes)             :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :         StableEntitySources (0x0003; variable bytes)          :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :          OwnerLocalSources (0x0004; variable bytes)           :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :        DerivedRelationSources (0x0005; variable bytes)        :
    :                                                               |
    +---------------------------------------------------------------+
```

### 4.1 Bindings 与来源池登记

`SourceMapBindings(0x0001)` 只有同名 `tableKind=0x0001` singleton：

```text
1:networkRevisionDerivationVersion:u16:R (=1)
2:networkRevision:Sha256:R
3:canonicalArtifactFormatVersion:u16:R (=4)
4:canonicalArtifactDigest:Sha256:R
5:canonicalArtifactByteLength:u64:R
6:compilerBuildId:Utf8:R
7:sourceCollectionDigestVersion:u16:R
8:sourceCollectionDigest:Sha256:R
```

LFSM 接受必须先绑定 LFCA，不能先暴露来源行：以 tag 5 约束 LFCA exact bytes，重算 tag 4
digest，结构和值域预检 LFCA 后，再核对 tag 3 format、tag 1 revision derivation version 与
tag 2 `NetworkRevisionId`。其中任一版本、摘要、长度或修订不等都以 source-map binding
mismatch 失败关闭，不能用 LFSM 自报版本选择 LFCA decoder。

`sourceCollectionDigestVersion=1`，tag 8 精确为：

```text
SHA-256(
  "laneflow.source-collection.v1\0"
  || sourceModuleCount:u32
  || for each SourceModule in sourceModuleOrdinal order:
       authoringNamespaceByteLength:u32
       authoringNamespaceUtf8Bytes
       sourceDocumentSetDigestVersion:u32
       sourceDocumentSetDigest:Sha256
)
```

所有整数小端。该摘要绑定完整来源集合，不替代逐文档 digest/length，也不进入
`NetworkRevisionId`。

`SourceModules(0x0002)` 的完整表登记为：

| tableKind | 表名             | 字段                                                                                                                                                                                                                                                                                                                                                                                         | 行键                    |
| --------- | ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- |
| `0x0001`  | `SourceModule`   | `1:sourceModuleOrdinal:u32:R, 2:authoringNamespaceId:Utf8:R, 3:sourceLanguage:u16:R, 4:sourceDocumentSetDigest:Sha256:R, 5:sourceDocumentSetDigestVersion:u32:R, 6:frontendVersion:u32:R, 7:frontendOptionsDigest:Sha256:R, 8:generatorBuildId:Utf8:R, 9:parametersAndInputsDigest:Sha256:R, 10:randomSeed:u64:O, 11:provenance:Utf8:R, 12:imports:RecordVector:R, 13:primaryLocation:u32:R` | `sourceModuleOrdinal`   |
| `0x0002`  | `SourceDocument` | `1:sourceDocumentOrdinal:u32:R, 2:sourceModuleOrdinal:u32:R, 3:sourceDocumentKey:Utf8:R, 4:sourceContentDigest:Sha256:R, 5:sourceRecordByteLength:u32:R, 6:displaySource:Utf8:O`                                                                                                                                                                                                             | `sourceDocumentOrdinal` |
| `0x0003`  | `SourceLocation` | tag `1..21`，见下表                                                                                                                                                                                                                                                                                                                                                                          | `sourceLocationOrdinal` |

`SourceModule.imports` 的唯一内嵌行是
`1:authoringNamespaceId:Utf8:R`。`SourceLocation.propertySteps` 的唯一内嵌行是
`1:stepKind:u8:R, 2:containerCode:u16:R, 3:memberCode:u16:R`。两者均不得继续嵌套。

`SourceLocation` 的完整字段和两种闭合变体为：

| tag | 字段                      | 类型           | `Text(0)` | `RoadEditing(1)` |
| --: | ------------------------- | -------------- | --------- | ---------------- |
|   1 | `sourceLocationOrdinal`   | `u32`          | R         | R                |
|   2 | `sourceLocationKind`      | `u8`           | R         | R                |
|   3 | `sourceModuleOrdinal`     | `u32`          | R         | R                |
|   4 | `sourceDocumentOrdinal`   | `u32`          | R         | R                |
|   5 | `startLine`               | `u32`          | R         | F                |
|   6 | `startColumn`             | `u32`          | R         | F                |
|   7 | `endLine`                 | `u32`          | R         | F                |
|   8 | `endColumn`               | `u32`          | R         | F                |
|   9 | `roadEditingSubjectKind`  | `u8`           | F         | R                |
|  10 | `moduleNamespace`         | `Utf8`         | F         | O                |
|  11 | `entityKind`              | `u16`          | F         | O                |
|  12 | `ownerLocalKey0`          | `Utf8`         | F         | O                |
|  13 | `ownerLocalKey1`          | `Utf8`         | F         | O                |
|  14 | `ownerLocalKey2`          | `Utf8`         | F         | O                |
|  15 | `localKey`                | `Utf8`         | F         | O                |
|  16 | `ownerKind`               | `u8`           | F         | O                |
|  17 | `roadEditingRelationKind` | `u8`           | F         | O                |
|  18 | `occurrenceKind`          | `u8`           | F         | O                |
|  19 | `occurrenceOrdinal`       | `u32`          | F         | O                |
|  20 | `propertySteps`           | `RecordVector` | F         | O                |
|  21 | `canvasSelection`         | `Utf8`         | F         | O                |

`R` 表示必需，`O` 表示受 subject 形状约束的可选字段，`F` 表示禁止出现。
`roadEditingSubjectKind` 固定为 `0=ModuleHeader, 1=RoadAlignment, 2=Declaration,
3=OwnerLocal`；`occurrenceKind` 固定为 `0=OrderedProductOrdinal,
1=CanonicalSetOrdinal`；`stepKind` 固定为 `0=TableField, 1=StructMember,
2=UnionVariant`。Road Editing 的 container/member 闭合集以 Road Editing v3 schema 和
[`road-editing-source-and-geometry-frontend.md`](road-editing-source-and-geometry-frontend.md)
冻结的 property-path 登记为准，LFSM 不接受未登记的自由组合。

`ownerKind = 0 ModuleHeader / 1 Address`。`roadEditingRelationKind` 是 LFSM 自己的封闭代码，
不继承 Rust enum 判别值：

```text
 0 Import                         1 CurveSegment
 2 CorridorElement               3 RoadSectionAuthoringLane
 4 LaneEdgeSuccessor             5 JunctionApproachEdge
 6 JunctionInternalEdge          7 ManeuverPathInternalEdge
 8 SignalControllerGroup         9 SignalControllerPhase
10 SignalPhaseState             11 AccessRuleParticipantClass
12 ParkingFacilityVirtualEntry  13 ParkingFacilityVirtualExit
14 ParticipantStreamPassage     15 ConflictZoneRegion
```

code 12/13 要求 Address owner=ParkingFacility 与 CanonicalSet occurrence；14 要求
Address owner=ParticipantStream 与 OrderedProduct occurrence；15 要求 ModuleHeader owner 与
按 `(ConflictZone StableId, CanonicalFrame StableId)`
规范化的 CanonicalSet occurrence。未知代码、错误 owner/occurrence 或把这些代码按数值强转
为 `sourceRelationRole` 都失败关闭。

`propertySteps.containerCode` 同样是 LFSM 3 的封闭登记，不读取 Rust enum 判别值。table
container code 与可用 field id 为：

```text
 0 RoadEditingSource[0..28]       1 ModuleHeader[0..3]
 2 Provenance[0..5]               3 LineSegment[0]
 4 CubicBezierSegment[0..2]       5 CurveSegment[0..2]
 6 CurveProgram[0..1]             7 RoadAlignment[0..3]
 8 CorridorElement[0..1]          9 RoadCorridor[0..8]
10 RoadSection[0..4]             11 AuthoringLane[0..6]
12 LaneEdge[0..4]                13 Junction[0..3]
14 Movement[0..4]                15 ManeuverPath[0..5]
16 ManeuverGate[0..6]            17 WaitingZone[0..5]
18 StopLine[0..2]                19 SignalGroup[0..1]
20 SignalController[0..4]        21 SignalPhaseState[0..1]
22 SignalPhase[0..4]             23 ParkingFacility[0..4]
24 ParkingLaneAnchor[0..1]       25 ParkingSpaceGeometry[0..3]
26 ParkingSpace[0..5]            27 LaneGroup[0..2]
28 FacilityBand[0..4]            29 ParticipantClass[0..2]
30 AccessRegulation[0..2]        31 AccessRule[0..7]
32 IidmVehicleProfile[0..6]      33 VehicleProfile[0..3]
34 ConflictZoneRegion[0..5]      35 CanonicalFrame[0..1]
36 ConflictZone[0..2]            37 PathAnchor[0..4]
38 ConflictPassage[0..2]         39 ParticipantStream[0..4]
```

code 23 是 `ParkingFacility`，不接受 `ParkingArea` 名称；新增 source table 的 field id
语义固定为：

```text
ConflictZone:       0 key, 1 junction, 2 canvas_selection
PathAnchor:         0 kind, 1 gate, 2 boundary_index, 3 path_edge_index, 4 progress_meters
ConflictPassage:    0 conflict_zone, 1 entry, 2 exit
ParticipantStream:  0 key, 1 junction, 2 maneuver_path, 3 passages, 4 canvas_selection
ConflictZoneRegion: 0 conflict_zone, 1 canonical_frame, 2 min_y, 3 max_y,
                    4 ring_xz, 5 canvas_selection
```

struct container code 为 `0 Digest256[0] / 1 OptionalU64[0] / 2 Vec3F64[0..2] /
3 LinearWidthProfile[0..1] / 4 Vec2F64[0..1]`；union container 只有
`0 CurveSegmentGeometry`，合法 discriminant 为 `1 LineSegment / 2 CubicBezierSegment`。

单步 `TableField` 只要命中上表已登记 field 就是完整 path。多步 path 只能沿下列 v3 schema
边继续，任何前缀、跨表拼接或未列边失败关闭：

- table→struct：`Provenance.{2,3}->Digest256`、`Provenance.4->OptionalU64`、
  `CurveProgram.0/LineSegment.0/CubicBezierSegment.{0,1,2}->Vec3F64`、
  `AuthoringLane.3/FacilityBand.2->LinearWidthProfile`、
  `ConflictZoneRegion.4->Vec2F64`；
- table→table：`ModuleHeader.3->Provenance`、`RoadAlignment.2/LaneEdge.3->CurveProgram`、
  `RoadCorridor.7->CorridorElement`、`SignalPhase.2->SignalPhaseState`、
  `ParkingFacility.{3,4}/ParkingSpace.{2,3}->ParkingLaneAnchor`、
  `ParkingSpace.4->ParkingSpaceGeometry`、`AccessRule.5->AccessRegulation`、
  `VehicleProfile.2->IidmVehicleProfile`、`ParticipantStream.3->ConflictPassage`、
  `ConflictPassage.{1,2}->PathAnchor`、`RoadEditingSource.28->ConflictZoneRegion`；
- union：`CurveSegment.1 -> CurveSegmentGeometry(1) -> LineSegment.0 -> Vec3F64 member`，
  或 discriminant 2 后接 `CubicBezierSegment.{0,1,2} -> Vec3F64 member`。

多步总长仍为 `1..=4`。第一步必须从 SourceLocation subject 的 root container 可达；
Declaration root 是其 entity kind 同名 table，RoadAlignment root 是 RoadAlignment，
OwnerLocal root 由 relation kind/owner 决定。因每一步单独合法而接受不可达组合仍是格式错误。

RoadEditing 变体的 optional 字段还受以下闭合矩阵约束：

| subject            | 必需                                                                                          | 禁止/条件                                                                                                                                                                    |
| ------------------ | --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ModuleHeader(0)`  | 无额外字段                                                                                    | tag 10..20 禁止；tag 21 可选                                                                                                                                                 |
| `RoadAlignment(1)` | tag 10 `moduleNamespace`、tag 15 `localKey`                                                   | tag 11 禁止；tag 12..14/16..19 禁止；tag 20/21 可选                                                                                                                          |
| `Declaration(2)`   | tag 10 `moduleNamespace`、tag 11 `entityKind`、tag 15 `localKey`                              | owner-local tag 16..19 禁止；tag 12..14 按实体 owner 深度连续出现；tag 20/21 可选                                                                                            |
| `OwnerLocal(3)`    | tag 16 `ownerKind`、17 `roadEditingRelationKind`、18 `occurrenceKind`、19 `occurrenceOrdinal` | `ownerKind=0(ModuleHeader)` 禁止 tag 10..15；`ownerKind=1(Address)` 要求 tag 10/15，tag 11 仅 Declaration owner 存在；tag 12..14 按 owner 深度连续；tag 20 必需，tag 21 可选 |

`propertySteps` 必须有 `1..=4` 行并构成 Road Editing v3 登记的一条完整可达路径，不能只因
各 step 单独合法就拼接。`sourceLanguage` 只允许 `1=SyntheticDsl` 与
`2=RoadEditingSource`；LFSM 3 分别要求 `frontendVersion=4` 与 `frontendVersion=3`。
前者只允许 Text 且 `SourceDocument.displaySource` 必须缺失，后者只允许 RoadEditing 且
display source 可选。optional `canvasSelection` 缺失与存在但为空是不同语义值，writer
不得互换。

模块按依赖优先 Kahn 顺序排列；ready set 以完整 namespace UTF-8 bytes 取最小项。文档在
模块内按完整 `sourceDocumentKey` bytes 排序。位置池按完整位置语义值排序去重并从 `0`
连续编号；该池必须恰好等于全部 primary/contributing/derived/range 引用位置的集合，禁止
未被任何来源行引用的额外位置。位置引用向量按位置语义值排序，不按哈希遍历、插入顺序或
来源 vector 顺序。

Text 位置的 line/column 都是一基非零值，且 start 不晚于 end。每个位置的 module/document
必须互相归属；RoadEditing 的 `moduleNamespace` 必须逐字节等于所属模块 namespace。
`SourceModule.primaryLocation` 必须解析到本模块位置。每个模块至少拥有一个 document；
document key 在全对象唯一，document ordinal 按 `(module ordinal, document-key bytes)` 从 0
连续编号。imports 按 namespace bytes 严格递增、不得重复，且只能指向更小的 module
ordinal。

每个 `SourceDocument` 的 `sourceRecordByteLength/sourceContentDigest` 分别等于完整来源记录
的 exact length 与 SHA-256。`sourceDocumentSetDigestVersion=1`，模块摘要精确为：

```text
SHA-256(
  "LFSOURCE-DOCUMENT-SET"
  || 1:u32
  || documentCount:u32
  || for each document in sourceDocumentKey byte order:
       sourceDocumentKeyByteLength:u32
       sourceDocumentKeyUtf8Bytes
       sourceRecordByteLength:u32
       sourceContentDigest:Sha256
)
```

候选发射必须从同一个 `CompilationOutput.ValidatedSourceMapInput` 原子投影，不接受调用方
分别拼装或覆盖来源字段：`source_module_sources()` 的 iterator 位置等于 module ordinal，
descriptor 的 namespace、language、document-set digest/version、frontend version/options、
generator build、parameters-and-inputs digest、optional random seed、provenance 与 imports
分别逐值等于 tag 2..12；tag 13 解引用后的完整位置值等于该 view 的
`primary_source()`。模块与文档必须和输入形成严格双射，包括没有被任一实体位置引用的
合法文档。只有在 LFCA、LFSM、LFSD 三份对象及这些投影全部成功后才能返回一个未发布候选。

### 4.2 实体、owner-local 与派生来源登记

其余五张表的完整字段和规范行键为：

| section/table                                          | 字段                                                                                                                                                                                                         | 规范行键                                                                       |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------ |
| `StableEntitySources/StableEntitySource(0x0001)`       | `1:entityKind:u16:R, 2:stableId:StableId128:R, 3:typedOrdinal:u32:R, 4:primaryLocation:u32:R, 5:contributingLocations:OrdinalVectorU32:R`                                                                    | `(entityKind, stableId, typedOrdinal)`                                         |
| `OwnerLocalSources/OwnerLocalSource(0x0001)`           | `1:ownerEntityKind:u16:R, 2:ownerStableId:StableId128:R, 3:sourceRelationRole:u8:R, 4:localIndex:u32:R, 5:primaryLocation:u32:R, 6:contributingLocations:OrdinalVectorU32:R`                                 | `(ownerEntityKind, ownerStableId, sourceRelationRole, localIndex)`             |
| `OwnerLocalSources/SpatialGeometrySourceRange(0x0002)` | `1:ownerEntityKind:u16:R, 2:ownerStableId:StableId128:R, 3:sourceRelationRole:u8:R, 4:localIndex:u32:R, 5:pointStart:u32:R, 6:pointEndExclusive:u32:R, 7:sourceSegmentOrdinal:u32:R, 8:sourceLocation:u32:R` | `(ownerEntityKind, ownerStableId, sourceRelationRole, localIndex, pointStart)` |
| `DerivedRelationSources/DerivedRelationSource(0x0001)` | `1:ownerEntityKind:u16:R, 2:ownerStableId:StableId128:R, 3:sourceRelationRole:u8:R, 4:localIndex:u32:R, 5:derivationPassVersion:u16:R, 6:constraintVersion:u16:R, 7:sourceLocations:OrdinalVectorU32:R`      | `(ownerEntityKind, ownerStableId, sourceRelationRole, localIndex)`             |

`StableEntitySource` 必须与绑定 LFCA 的 identity/实体形成严格双射。typed ordinal 与
source-location ordinal 都是完整逻辑表中的全局 `u32`，不得改写成
`(chunkIndex, rowInChunk)` 或其它物理地址。`OwnerLocalSource` 必须
与下节的全部合法关系 tuple 形成严格双射；可选 scalar 缺失时恰无行。几何范围只允许 role
28/29/32，并且必须有相同行键的 `OwnerLocalSource` 父行。`DerivedRelationSource` 当前只允许
role 9。

对任一 compiler-private source view，令 `C(view)` 为把
`contributing_sources()` 按完整位置语义值排序、去重后映射成最终 location ordinal 的唯一
向量。每个 `StableEntitySource` 的 primary/contributing 必须分别逐值等于对应 view 的
`primary_source()` 与 `C(view)`；每个 `OwnerLocalSource` 同理对应由 role 唯一选择的
owner-local view。一个任意但语法合法的位置、另一关系的合法位置，或仅保持 ordinal 不变的
重新配对都不能形成候选。

Stable entity 的来源还必须绑定“实体是谁”：primary 所属模块 namespace 等于 Identity tag 1。
RoadEditing primary 必须是对应 Declaration，其 `entityKind`、`localKey` 与 owner key chain
分别等于 identity 的 entity-local key 和递归 parent anchor。entity-local key tag 为：

```text
RoadCorridor=2 RoadSection=3 AuthoringLane=4 LaneEdge=5 Junction=6
Movement=8 ManeuverPath=7 ManeuverGate=15 WaitingZone=16 StopLine=17
SignalGroup=18 SignalController=19 SignalPhase=21 ParkingFacility=22
ParkingSpace=24 LaneGroup=25 FacilityBand=26 ParticipantClass=27
AccessRule=28 VehicleProfile=29 CanonicalFrame=31 ConflictZone=23
ParticipantStream=30
```

parent anchor 为 RoadSection/AuthoringLane/LaneGroup 的 33/32/32，
Movement/ConflictZone/ParticipantStream 的 34，ManeuverPath 的 11，ManeuverGate/WaitingZone
的 14，SignalPhase 的 20，FacilityBand 的 33；其余没有 parent anchor。递归时必须先用
全局唯一 Identity 解析 parent StableId 再取 parent local key，不能用 raw ordinal、StableId
hex 或 LFSM 自报地址替代。

`SpatialGeometrySourceRange` 必须逐项等于对应 source view 的
`geometry_source_ranges()`：同一父行按 `pointStart` 严格相邻、非空、无重叠，从 0 覆盖到
对应 geometry point count；`sourceSegmentOrdinal` 和 location 逐值投影，不得合并、拆分或
换入另一条合法 segment。role 28/29 的 range 非空性必须与 LFCA
`directionProfileApplies` 相等；role 32 覆盖 ConflictZoneRegion ring，不能把 min/max height
或另一 region 的位置伪装为 ring range。父行 contributing locations 必须等于子行 locations
按完整语义值排序去重后的投影。范围为空时恰无子行，不伪造来源 segment。

role 28/29 的非空 range location 必须是同一 Address owner 下的 RoadEditing
`CurveSegment` OwnerLocal，occurrence 为
`OrderedProductOrdinal(sourceSegmentOrdinal)`，property path 精确落到
`CurveSegment.geometry`；全部 ranges 的 module/document/owner 必须相同。Synthetic Text
geometry 没有 curve range。role 32 的 range location 必须落到同一
`ConflictZoneRegion.ring_xz` owner-local 记录，不能借用 `min_y/max_y` 或 zone declaration
位置。只证明 location 在全局语法上合法不足以通过这些闭合。

`DerivedRelationSource` 只为每个 role 9 `JunctionInternalEdge` tuple 恰好生成一行，并与
相同行键的 `OwnerLocalSource` 双射；`derivationPassVersion=1`，`constraintVersion` 等于
绑定 LFCA constraint contract，`sourceLocations` 等于 owner-local primary+contributing
按位置语义值排序去重。其它 role、版本、遗漏/额外行或不能反解到唯一 LFCA tuple 的行
失败关闭。

### 4.3 完整 `sourceRelationRole` 登记

`localIndex` 一律是 owner 内局部下标；`vector` 是相应 LFCA vector/RecordVector 的零基位置，
`scalar` 在字段存在时固定为 `0`，`filtered row` 是按表规范行键过滤 owner 后的零基位置。
`set` 的位置只服务 wire 排序和来源定位，不产生 LFSD Move；`multiset`（重集）另以同
subject occurrence rank 保留重复基数；`domain` 的位置属于语义顺序。

| role / 名称                             | owner kind        | subject kind                  | 绑定 LFCA 唯一投影                          | localIndex / 序策略 | LFSD 投影                |
| --------------------------------------- | ----------------- | ----------------------------- | ------------------------------------------- | ------------------- | ------------------------ |
| `1 LaneEdgeSuccessor`                   | LaneEdge          | LaneEdge                      | `LaneEdge.successors`                       | vector / set        | Relation                 |
| `2 RoadCorridorElement`                 | RoadCorridor      | RoadSection 或 FacilityBand   | `RoadCorridor.elements`                     | vector / domain     | Relation                 |
| `3 RoadSectionLane`                     | RoadSection       | AuthoringLane                 | `RoadSection.lanes`                         | vector / domain     | Relation                 |
| `4 AuthoringLaneEdge`                   | AuthoringLane     | LaneEdge                      | `AuthoringLane.edgeChain`                   | vector / domain     | Relation                 |
| `5 LaneGroupMember`                     | LaneGroup         | AuthoringLane                 | `LaneGroup.members`                         | vector / domain     | Relation                 |
| `6 JunctionMovement`                    | Junction          | Movement                      | `Junction.movements`                        | vector / set        | Relation                 |
| `7 MovementManeuverPath`                | Movement          | ManeuverPath                  | `Movement.maneuverPaths`                    | vector / set        | Relation                 |
| `8 ManeuverPathEdge`                    | ManeuverPath      | LaneEdge                      | `ManeuverPath.edges`                        | vector / domain     | Relation                 |
| `9 JunctionInternalEdge`                | Junction          | LaneEdge                      | `JunctionInternalEdge` owner rows           | filtered row / set  | Relation + derived       |
| `10 ManeuverPathGate`                   | ManeuverPath      | ManeuverGate                  | `ManeuverPath.maneuverGates`                | vector / domain     | Relation                 |
| `11 ManeuverPathWaitingZone`            | ManeuverPath      | WaitingZone                   | `ManeuverPath.waitingZones`                 | vector / domain     | Relation                 |
| `12 StopLineManeuverGate`               | StopLine          | ManeuverGate                  | `StopLine.maneuverGates`                    | vector / set        | Relation                 |
| `13 ParkingFacilityVirtualEntry`        | ParkingFacility   | LaneEdge                      | `ParkingFacility.virtualEntries[].laneEdge` | vector / multiset   | Relation + field payload |
| `14 ParkingFacilityVirtualExit`         | ParkingFacility   | LaneEdge                      | `ParkingFacility.virtualExits[].laneEdge`   | vector / multiset   | Relation + field payload |
| `15 JunctionConflictZone`               | Junction          | ConflictZone                  | 按 `ConflictZone.junction` 过滤实体行       | filtered row / set  | Relation                 |
| `16 JunctionParticipantStream`          | Junction          | ParticipantStream             | 按 `ParticipantStream.junction` 过滤实体行  | filtered row / set  | Relation                 |
| `17 SignalControllerGroup`              | SignalController  | SignalGroup                   | `SignalController.signalGroups`             | vector / set        | Relation                 |
| `18 SignalControllerPhase`              | SignalController  | SignalPhase                   | `SignalController.phases`                   | vector / domain     | Relation                 |
| `19 SignalPhaseState`                   | SignalPhase       | SignalGroup                   | `SignalPhase.states.signalGroup`            | vector / set        | StaticRule only          |
| `20 ManeuverGateSignalGroup`            | ManeuverGate      | SignalGroup                   | Group 时的 `ManeuverGate.signalGroup`       | scalar              | Relation                 |
| `21 ParkingSpaceFacility`               | ParkingSpace      | ParkingFacility               | 可选 `ParkingSpace.parkingFacility`         | scalar              | Relation                 |
| `22 ParkingSpaceEntry`                  | ParkingSpace      | LaneEdge                      | `ParkingSpace.entryLaneEdge`                | scalar              | Relation                 |
| `23 ParkingSpaceExit`                   | ParkingSpace      | LaneEdge                      | `ParkingSpace.exitLaneEdge`                 | scalar              | Relation                 |
| `24 ParticipantClassExtends`            | ParticipantClass  | ParticipantClass              | 可选 `ParticipantClass.parent`              | scalar              | Relation                 |
| `25 AccessRuleTarget`                   | AccessRule        | targetKind 指定的四种实体之一 | `AccessRule.(targetKind,targetOrdinal)`     | scalar              | Relation                 |
| `26 AccessRuleParticipantClass`         | AccessRule        | ParticipantClass              | `AccessRule.participantClasses`             | vector / set        | Relation                 |
| `27 VehicleProfileParticipantClass`     | VehicleProfile    | ParticipantClass              | `VehicleProfile.participantClass`           | scalar              | Relation                 |
| `28 CanonicalFrameLaneEdgeGeometry`     | CanonicalFrame    | LaneEdge                      | `LaneEdgeGeometry` owner rows               | filtered row / set  | Geometry only            |
| `29 CanonicalFrameFacilityBandGeometry` | CanonicalFrame    | FacilityBand                  | `FacilityBandGeometry` owner rows           | filtered row / set  | Geometry only            |
| `30 ParticipantStreamManeuverPath`      | ParticipantStream | ManeuverPath                  | `ParticipantStream.maneuverPath`            | scalar              | Relation                 |
| `31 ParticipantStreamConflictPassage`   | ParticipantStream | ConflictZone                  | `ParticipantStream.passages[].conflictZone` | vector / domain     | Relation + field payload |
| `32 CanonicalFrameConflictZoneRegion`   | CanonicalFrame    | ConflictZone                  | `ConflictZoneRegion` owner rows             | filtered row / set  | Geometry only            |

role 21 的名称固定为 `ParkingSpaceFacility`。设施声明、`virtualCapacity` 和每个 anchor
都必须回指 exact Road Editing v3 property path。anchor 先按
`(LaneEdge StableId128, progressMillimetres)` 规范排序，因此来源 vector 顺序不能改变
canonical localIndex。

RoadEditing primary-source projection 不是“任一合法 property path”。下表冻结 role 1–29
的唯一来源语义；`Declaration` 表示目标 declaration 的位置，`OwnerLocal` 表示相应 owner
关系 occurrence 的位置，set/domain 的 occurrence kind 必须与本节 role 表一致：

| role | Road Editing v3 primary source projection                                                                |
| ---: | -------------------------------------------------------------------------------------------------------- |
|    1 | LaneEdge owner 的 `successors[canonical localIndex]` OwnerLocal                                          |
|    2 | RoadCorridor owner 的 `elements[localIndex]` OwnerLocal                                                  |
|    3 | AuthoringLane subject Declaration                                                                        |
|    4 | AuthoringLane owner 的 `lane_edge` declaration property                                                  |
|    5 | AuthoringLane subject 的 `lane_group` property                                                           |
|    6 | Movement subject 的 `junction` property                                                                  |
|    7 | ManeuverPath subject 的 `movement` property                                                              |
|    8 | 首项为 ManeuverPath `entry_edge`；末项为 `exit_edge`；中间项为 `internal_edges[localIndex-1]` OwnerLocal |
|    9 | 见下文从 Junction 内部边反解出的 ManeuverPath `internal_edges` OwnerLocal                                |
|   10 | ManeuverGate subject 的 `maneuver_path` property                                                         |
|   11 | WaitingZone subject 的 `maneuver_path` property                                                          |
|   12 | ManeuverGate subject 的 `stop_line` property                                                             |
|   13 | `RoadEditingSource.parking_facilities[].virtual_entries[]`                                               |
|   14 | `RoadEditingSource.parking_facilities[].virtual_exits[]`                                                 |
|   15 | `RoadEditingSource.conflict_zones[].junction`                                                            |
|   16 | `RoadEditingSource.participant_streams[].junction`                                                       |
|   17 | SignalController owner 的 `signal_groups[canonical localIndex]` OwnerLocal                               |
|   18 | SignalController owner 的 `signal_phases[localIndex]` OwnerLocal                                         |
|   19 | SignalPhase owner 的 `states[canonical localIndex].signal_group` OwnerLocal                              |
|   20 | ManeuverGate owner 的 `signal_group` property                                                            |
|   21 | ParkingSpace owner 的 `parking_facility` property                                                        |
|   22 | ParkingSpace owner 的 `entry.lane_edge` property                                                         |
|   23 | ParkingSpace owner 的 `exit.lane_edge` property                                                          |
|   24 | ParticipantClass owner 的 `extends` property                                                             |
|   25 | AccessRule owner 的 `target_reference` property                                                          |
|   26 | AccessRule owner 的 `participant_classes[canonical localIndex]` OwnerLocal                               |
|   27 | VehicleProfile owner 的 `participant_class` property                                                     |
|   28 | LaneEdge subject Declaration；可选 point ranges 另投影其曲线 segment OwnerLocal                          |
|   29 | FacilityBand subject Declaration；可选 point ranges 另投影其曲线 segment OwnerLocal                      |

role 9 对给定 Junction 先按 `JunctionInternalEdge` owner row 的规范行键过滤；关系 tuple
的 `localIndex` 必须是该过滤结果的零基行位置。primary source 另行从绑定 LFCA 中选择
StableId 最小、且 internal-edge occurrence 序列包含该 subject edge 的 ManeuverPath，再取
该 edge 在所选 internal sequence 的第一次零基 occurrence，映射到对应
`internal_edges[occurrence]` OwnerLocal。这个 source occurrence 只定位 primary，绝不
替代或改变 filtered-row `localIndex`；不存在候选或无法得到唯一投影时失败关闭。选择只从
绑定 LFCA 稳定身份和关系重算，不能使用 LFSM 自报路径打破平局。

role 30–32 的 Road Editing v3 primary-source projection 固定为：

| role | primary declaration / owner-local path                                                                      |
| ---: | ----------------------------------------------------------------------------------------------------------- |
|   30 | `RoadEditingSource.participant_streams[].maneuver_path`                                                     |
|   31 | `RoadEditingSource.participant_streams[].passages[]`；完整 passage element 是 owner-local subject           |
|   32 | `RoadEditingSource.conflict_zone_regions[].canonical_frame`；ring/high range 共用该 region primary location |

role 13/14/31/32 的 OwnerLocal location 必须分别携带 relation kind 12/13/14/15；role 15/16/30
使用稳定实体 Declaration 的对应 scalar property，不伪造 OwnerLocal occurrence。
role 31 的 local index 是 passage 规范领域顺序，不是来源 vector 位置；role 13/14 是按
同一 LaneEdge subject 的 occurrence rank 保留 multiplicity 的 multiset；role 15/16/32 是
set。它们只在相应 relation tuple 集合或重集变化时生成 LFSD relation change。role 32 的
`SpatialGeometrySourceRange` 覆盖 ring point range；role 31 的 entry/exit property steps
必须落到同一个 passage owner-local row。独立 writer/checker 必须从绑定 LFCA 与这些
路径一一反解，不能仅凭 role 数值猜测 projection。

LFSM 接受前必须先用 tag 3/4/5 绑定 LFCA 4 exact bytes，再暴露任一来源行。
`sourceMapDigest` 是完整 LFSM exact bytes 的 SHA-256；`sourceMapByteLength` 是同一字节序列的
精确 `u64` 长度，二者不嵌回 LFSM。

## 5. LFSD 3

`magic = "LFSD"`，`semanticDiffFormatVersion = 3`。六节和各自唯一的表为：

| sectionKind | 名称                          | tableKind `0x0001`                       |
| ----------- | ----------------------------- | ---------------------------------------- |
| `0x0001`    | `SemanticDiffBindings`        | `SemanticDiffBindings` singleton         |
| `0x0002`    | `EntityChanges`               | `EntityChange`                           |
| `0x0003`    | `RelationChanges`             | `RelationChange`                         |
| `0x0004`    | `GeometryChanges`             | `GeometryChange`                         |
| `0x0005`    | `StaticRuleChanges`           | `StaticRuleChange`                       |
| `0x0006`    | `SpatialConfigurationChanges` | `SpatialConfigurationChange`（至多一行） |

第一节 wire offset 固定为 `0x00b0`（`32 + 6 * 24`）：

```text
    +---------------------------------------------------------------+
    |           LFSD ObjectPreambleV1 (32 bytes; SC == 6)           |
    +---------------------------------------------------------------+
    |            SectionDirectoryEntryV1[6] (144 bytes)             |
    +===============================================================+
    |                                                               :
    :         SemanticDiffBindings (0x0001; variable bytes)         :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :            EntityChanges (0x0002; variable bytes)             :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :           RelationChanges (0x0003; variable bytes)            :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :           GeometryChanges (0x0004; variable bytes)            :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :          StaticRuleChanges (0x0005; variable bytes)           :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :      SpatialConfigurationChanges (0x0006; variable bytes)     :
    :                                                               |
    +---------------------------------------------------------------+
```

### 5.1 两端绑定

`SemanticDiffBindings` 的九个字段全部必需：

```text
1:baseBindingKind:u8:R
2:baseNetworkRevisionDerivationVersion:u16:R
3:baseNetworkRevision:Sha256:R
4:baseCanonicalArtifactDigest:Sha256:R
5:baseCanonicalArtifactByteLength:u64:R
6:targetNetworkRevisionDerivationVersion:u16:R
7:targetNetworkRevision:Sha256:R
8:targetCanonicalArtifactDigest:Sha256:R
9:targetCanonicalArtifactByteLength:u64:R
```

`baseBindingKind` 固定为 `0=Genesis, 1=Artifact`。Genesis 的四个 base 值精确为
`0, zero[32], zero[32], 0`；Artifact 禁止零占位。target 永远是具体 LFCA 4。Artifact diff
要求两端 LFCA 的 identity/constraint/execution 合同轴一致，并在变化分类前验证所有共有
`StableId128` 的 `entityKind` 和完整 identity 前像逐字节相同；合同轴不一致返回
`UnsupportedSemanticContractTransition`，共有 StableId 的 kind/前像不一致返回
`CrossRevisionStableIdCollision`，两者都在读取 change table 前拒绝整份 diff。

### 5.2 Change table 字段与存在性矩阵

四张 entity-scoped change table 的 tag `1..8` 完全相同：

```text
1:changeKind:u8:R
2:entityKind:u16:R
3:ownerStableId:StableId128:O
4:subjectStableId:StableId128:O
5:sourceRelationRole:u8:O
6:fieldTag:u16:O
7:beforeLocalIndex:u32:O
8:afterLocalIndex:u32:O
```

各表追加字段和封闭 `changeKind` 为：

| 表                 | tag 9/10                                                         | `changeKind`                           |
| ------------------ | ---------------------------------------------------------------- | -------------------------------------- |
| `EntityChange`     | `9:beforeValue:Bytes:O, 10:afterValue:Bytes:O`                   | `0=Add, 1=Remove, 2=Modify`            |
| `RelationChange`   | `9:beforeTarget:StableId128:O, 10:afterTarget:StableId128:O`     | `0=Add, 1=Remove, 2=Move, 3=Reconnect` |
| `GeometryChange`   | `9:beforeCanonicalValue:Bytes:O, 10:afterCanonicalValue:Bytes:O` | `0=Add, 1=Remove, 2=Modify`            |
| `StaticRuleChange` | `9:beforeCanonicalValue:Bytes:O, 10:afterCanonicalValue:Bytes:O` | `0=Modify`                             |

完整存在性矩阵如下；未列入“必需”的 optional tag 全部禁止：

| 表/change kind       | 必需 common tags                                                          | payload                                                                    |
| -------------------- | ------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| Entity `Add`         | `subjectStableId`                                                         | `afterValue:R`（完整目标 RowV1），`beforeValue:F`                          |
| Entity `Remove`      | `subjectStableId`                                                         | `beforeValue:R`（完整 base RowV1），`afterValue:F`                         |
| Entity `Modify`      | `subjectStableId, fieldTag`                                               | payload 必须与 base/target 字段存在性精确一致，见下文                      |
| Relation `Add`       | `ownerStableId, subjectStableId, role, afterLocalIndex`                   | tag 9/10 禁止                                                              |
| Relation `Remove`    | `ownerStableId, subjectStableId, role, beforeLocalIndex`                  | tag 9/10 禁止                                                              |
| Relation `Move`      | `ownerStableId, subjectStableId, role, beforeLocalIndex, afterLocalIndex` | tag 9/10 禁止，两个 index 必须不同                                         |
| Relation `Reconnect` | `ownerStableId, role, beforeLocalIndex, afterLocalIndex`                  | `beforeTarget:R, afterTarget:R`，`subjectStableId:F`，两个 target 必须不同 |
| Geometry `Add`       | `subjectStableId`                                                         | `afterCanonicalValue:R`，before 禁止                                       |
| Geometry `Remove`    | `subjectStableId`                                                         | `beforeCanonicalValue:R`，after 禁止                                       |
| Geometry `Modify`    | `subjectStableId`                                                         | before/after 都必需且不同                                                  |
| StaticRule `Modify`  | `subjectStableId, fieldTag`                                               | payload 必须与 base/target 字段存在性精确一致，见下文                      |

Relation 行的 `entityKind` 是 owner kind，其余表是 subject kind。Relation 只允许 role
`1..18, 20..27, 30..31`；role 19 只投影 StaticRule，role 28/29/32 只投影 Geometry。

`Entity Modify` 与 `StaticRule Modify` 的 before/after payload 不能任选“至少一侧”。checker
必须先从绑定的 base/target LFCA 确认该 `fieldTag` 的实际存在性，再按下表要求唯一编码；`R/F`
表示必需/禁止，两个字段都存在时其规范 bytes 必须不同：

| base 字段 | target 字段 | before payload | after payload | 结果                |
| --------- | ----------- | -------------- | ------------- | ------------------- |
| 存在      | 存在        | R              | R             | 两值不同的 `Modify` |
| 缺失      | 存在        | F              | R             | optional 字段新增   |
| 存在      | 缺失        | R              | F             | optional 字段删除   |
| 缺失      | 缺失        | F              | F             | 禁止生成 change row |

required 字段在两端都必须存在，因此其 `Modify` 永远携带 before/after 两侧完整 payload。
payload 缺侧、放错侧、两侧相同、或为两端都缺失的字段生成行都使整份 LFSD 失败关闭。该规则
同时约束 emitter、LFSD reader 与双重闭合检查，不能从 change row 自报的 payload 反推字段存在性。

### 5.3 字段变化分类

对 retained identity，每个变化字段必须按下表排他归类。Identity/derived 列只参与两端
闭合或由其他规范关系派生，不生成字段 `Modify`：

| entity table      | Entity `Modify` | Relation 投影 | StaticRule `Modify` | Identity / derived |
| ----------------- | --------------- | ------------- | ------------------- | ------------------ |
| RoadCorridor      | `3`             | `4`           | —                   | —                  |
| RoadSection       | `4`             | `5`           | —                   | `3`                |
| AuthoringLane     | —               | `4,5`         | —                   | `3`                |
| LaneEdge          | `3,4`           | `5`           | —                   | —                  |
| Junction          | —               | `3`           | —                   | —                  |
| Movement          | —               | `6`           | —                   | `3..5`             |
| ManeuverPath      | —               | `4..6`        | —                   | `3`                |
| ManeuverGate      | `4`             | `5,7`         | `6`                 | `3`                |
| WaitingZone       | —               | —             | `4..6`              | `3`                |
| StopLine          | `3`             | `4`           | —                   | —                  |
| SignalGroup       | —               | `3,4`         | —                   | —                  |
| SignalController  | —               | `5,6`         | `3,4`               | —                  |
| SignalPhase       | —               | —             | `4,5`               | `3`                |
| ParkingFacility   | `5,6`           | `3,5,6`       | `4`                 | —                  |
| ParkingSpace      | `5,7..11`       | `3,4,6`       | —                   | —                  |
| LaneGroup         | —               | `4`           | —                   | `3`                |
| FacilityBand      | `4`             | —             | —                   | `3`                |
| ParticipantClass  | `4`             | `3`           | —                   | `5,6`              |
| AccessRule        | —               | `3,4,6`       | `5,7,8`             | —                  |
| VehicleProfile    | `4..10`         | `3`           | —                   | —                  |
| CanonicalFrame    | —               | —             | —                   | —                  |
| ConflictZone      | —               | —             | —                   | `3`                |
| ParticipantStream | `5`             | `4,5`         | —                   | `3`                |

字段级 before/after `Bytes` 只保存 `SemanticFieldValueV1`，不包含 12-byte `FieldV1`
header；Entity Add/Remove 保存所在一侧完整 LFCA entity `RowV1`。所有 before/after payload
必须逐字节等于从受绑定 base/target LFCA 独立重算的投影。除表中明确标记为 derived 的缓存外，
每个变化字段、关系 tuple 与 geometry 必须在其唯一 change class 中恰好出现一次；换类、重复、
遗漏或额外记录都失败关闭。

`ParkingFacility.virtualEntries/virtualExits` 必须双重闭合，而不是重复表达同一信息：

- role 13/14 的 RelationChange 表达 owner、LaneEdge subject、同 subject occurrence rank
  和所在一侧规范 localIndex；
- tag 5/6 的 Entity Modify 表达完整 anchor payload，确保同一 LaneEdge 上只有
  `progressMillimetres` 变化时仍可观察；
- tag 5/6 的 `SemanticFieldValueV1` 精确为
  `count:u32 || (LaneEdge StableRefV1 || progressMillimetres:u32)[count]`，保持 LFCA 规范顺序；
- 任一 anchor 语义 vector 变化都必须产生 tag 5/6 字段投影；只有
  `(owner, role, LaneEdge subject, per-subject occurrence rank)` relation multiset 实际变化时才
  另外产生 RelationChange。仅 progress 改变且同 LaneEdge 的 anchor 数量不变时，即使
  规范 localIndex 因重新排序而变化，也不得伪造 Add/Remove/Move/Reconnect；
- `virtualCapacity` 只由 tag 4 StaticRule Modify 表达；显式成员只由 tag 3/role 21 闭合。

这项双投影是必要的现实边界：anchor 没有全局 StableId，而仅凭 relation 的 LaneEdge
StableId 无法区分同一条边上的不同 progress。

`ParticipantStream.passages` 使用同样的“relation + field payload”闭合：role 31 表达
`(ParticipantStream, ConflictZone, canonical localIndex)`；tag 5 `Entity Modify` 表达
完整 passage 值。稳定值不允许携带 artifact-local ordinal：

```text
PathAnchorStableValueV1 :=
  anchorKind:u8
  case Gate(0):          ManeuverGate StableRefV1
  case EdgeBoundary(1):  boundaryIndex:u32
  case Interior(2):      pathEdgeIndex:u32 || progressMillimetres:u32

ConflictPassageStableValueV1 :=
  ConflictZone StableRefV1
  || entry:PathAnchorStableValueV1
  || exit:PathAnchorStableValueV1

ParticipantStream.passages SemanticFieldValueV1 :=
  count:u32 || ConflictPassageStableValueV1[count]
```

anchor payload 变化必须产生 tag 5 Modify；只有 zone/localIndex relation tuple 变化时才另外
生成 role 31 Add/Remove/Move。admission Gate 始终重新派生，不进入 LFSD。Genesis 和
Artifact diff 都必须覆盖全部 passage，不得因它没有独立 StableId 而省略。

其他 ordinal-bearing 字段不得直接比较 artifact-local `u32`。`StableRefV1` 精确为
`referencedEntityKind:u16 || referencedStableId[16]`。以下语义字段使用稳定投影：

| entity.field                                  | `SemanticFieldValueV1`                                       |
| --------------------------------------------- | ------------------------------------------------------------ |
| `RoadCorridor.referenceSection`               | 所引 `RoadSection` 的 `StableRefV1`                          |
| `StopLine.laneEdge`                           | 所引 `LaneEdge` 的 `StableRefV1`                             |
| `WaitingZone.entryGate/releaseGate`           | 各自所引 `ManeuverGate` 的 `StableRefV1`                     |
| `SignalPhase.states`                          | `count:u32 || (SignalGroup StableRefV1 || aspect:u8)[count]` |
| `ParkingFacility.virtualEntries/virtualExits` | 上述 anchor vector 稳定投影                                  |
| `ParticipantStream.maneuverPath`              | 所引 `ManeuverPath` 的 `StableRefV1`                         |
| `ParticipantStream.passages`                  | 上述 passage vector 稳定投影                                 |
| 其他允许字段                                  | 对应 LFCA field value exact bytes                            |

Geometry before/after 使用：

```text
CanonicalGeometryValueV1 :=
  canonicalFrame:StableRefV1
  projectedFieldCount:u16
  for each projected field in required-tag order:
    fieldTag:u16
    valueByteLength:u32
    valueBytes[valueByteLength]
```

LaneEdgeGeometry 的 projected fields 为 tag `3,4,5,6`，FacilityBandGeometry 为 `3,4`，
ConflictZoneRegion 为 `3,4,5`；subject ordinal 排除，frame ordinal 替换为
`StableRefV1`。ConflictZoneRegion 的 subject 是对应 `ConflictZone StableId`，tag 5 ring
使用规范 `(x:f32 bits, z:f32 bits)` 序列，不比较 artifact-local row/chunk 地址。

### 5.4 规范排序与空间配置

Artifact diff 必须先以 `StableId128` 建立两端全局身份表。共有 StableId 的 kind 和完整
identity canonical bytes 必须逐字节相同，否则整个 base/target 对失败；不能把截断碰撞当成
retained identity。身份前像改变并正常得到新 StableId 时，只产生旧实体完整 Remove 与新实体
完整 Add，不使用 compiler lineage、来源位置、相似度或 raw ordinal 猜测配对。

relation tuple 的两端配对算法由 §4.3 的序策略唯一决定：

- `set` 以 `(subjectKind, subjectStableId)` 比较成员；已有成员仅因另一成员插入导致规范位置
  改变时不产生 Move，Add/Remove 仍携带所在一侧重算的 localIndex；
- `multiset` 先按 `(subjectKind, subjectStableId)` 分组，再按各自规范 localIndex 为同 subject
  分配零基 occurrence rank；两端只配对相同 subject 与 rank，数量减少/增加分别产生
  Remove/Add，记录携带所在一侧重算的 localIndex。其他 subject 插入或同一 anchor 的
  progress payload 改变不产生 Move；payload 变化由对应 Entity Modify 完整表达；
- `scalar` 缺失/出现产生 Remove/Add；两端都存在而 target 改变时产生 Reconnect，index 恒为 0；
- `domain` 对同一 subject 按各自 localIndex 递增分配零基 occurrence rank，只配对两端相同
  rank；配对项 index 改变产生 Move，未配对项产生 Remove/Add。因此同一 edge 在
  ManeuverPath 中重复 occurrence 不会被错误折叠；
- role 19 只进入 `SignalPhase.states` StaticRule，role 28/29/32 只进入 Geometry；不得再
  生成 RelationChange。role 13/14 的 multiset tuple 与 anchor field payload、role 31 的
  domain tuple 与 passage field payload 双重闭合。

`ManeuverGate.transitionIndex` 是 entity tag 4 的规范标量，不是 role 10 localIndex；它变化
必须产生 Entity Modify，即使 gate 在 path vector 的位置未变。只有 vector 位置变化才另产生
role 10 Move。`ParticipantClass.subtreeEnter/subtreeExit` 从 parent forest 与规范 DFS 重算，
只验证两端闭合，不产生伪 Modify。

| 表/change kind             | tag 1 `changeKind` 之后的规范排序键                                                               |
| -------------------------- | ------------------------------------------------------------------------------------------------- |
| Entity Add/Remove          | `(entityKind, subjectStableId)`                                                                   |
| Entity Modify              | `(entityKind, subjectStableId, fieldTag)`                                                         |
| Relation Add               | `(entityKind, ownerStableId, role, afterLocalIndex, subjectStableId)`                             |
| Relation Remove            | `(entityKind, ownerStableId, role, beforeLocalIndex, subjectStableId)`                            |
| Relation Move              | `(entityKind, ownerStableId, role, beforeLocalIndex, afterLocalIndex, subjectStableId)`           |
| Relation Reconnect         | `(entityKind, ownerStableId, role, beforeLocalIndex, afterLocalIndex, beforeTarget, afterTarget)` |
| Geometry Add/Remove/Modify | `(entityKind, subjectStableId)`                                                                   |
| StaticRule Modify          | `(entityKind, subjectStableId, fieldTag)`                                                         |

整数按无符号数值序，StableId/Bytes 按无符号逐字节字典序。每张表先按 `changeKind`，再按
上表 tuple 严格递增；重复键失败关闭。

`SpatialConfigurationChange` 的字段是
`1:changeKind:u8:R, 2:beforeSpatialPresence:Bytes:O,
3:afterSpatialPresence:Bytes:R`。`0=Initialize` 只允许 Genesis，要求 tag 1/3；`1=Modify`
只允许 Artifact 两端不同时，要求 tag 1/2/3。before/after `Bytes` 分别是所在一侧完整
`SpatialPresence` RowV1；Artifact 两端相同时该表为空。

Genesis 对目标每个实体、合法 relation tuple 和 geometry 分别产生 Add，禁止
Remove/Modify/Move/Reconnect，StaticRule 为空，并恰有一条空间 Initialize。Artifact diff
由两份受绑定 LFCA 独立重算。LFCA 3 到 LFCA 4 是不支持的语义合同转换，不生成跨格式 LFSD。
`semanticDiffDigest` 是完整 LFSD exact bytes 的 SHA-256，`semanticDiffByteLength` 是同一
字节序列的精确 `u64` 长度；两者不嵌回 LFSD。内容寻址 key 唯一为
`"sha256/" || hexLower(semanticDiffDigest)`，调用方不能覆盖。该绑定供候选、宿主资产绑定与
后继切换合同使用，不进入 LFCP；LFSD 自身不授予迁移权限。

## 6. LFCP 2 与发布闭合

`magic = "LFCP"`，`canonicalPublicationDescriptorVersion = 2`。LFCP 2 精确有三节，
每节只有一张 `tableKind=0x0001` singleton：

| sectionKind | 名称                       | 行数 |
| ----------- | -------------------------- | ---: |
| `0x0001`    | `CanonicalArtifactBinding` |    1 |
| `0x0002`    | `SourceMapBinding`         |    1 |
| `0x0003`    | `PublicationProvenance`    |    1 |

第一节 wire offset 固定为 `0x0068`（`32 + 3 * 24`）：

```text
    +---------------------------------------------------------------+
    |           LFCP ObjectPreambleV1 (32 bytes; SC == 3)           |
    +---------------------------------------------------------------+
    |             SectionDirectoryEntryV1[3] (72 bytes)             |
    +===============================================================+
    |                                                               :
    :       CanonicalArtifactBinding (0x0001; variable bytes)       :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :           SourceMapBinding (0x0002; variable bytes)           :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :        PublicationProvenance (0x0003; variable bytes)         :
    :                                                               |
    +---------------------------------------------------------------+
```

三张 singleton 的完整字段登记为：

```text
CanonicalArtifactBinding(0x0001):
  1:canonicalArtifactFormatVersion:u16:R (=4)
  2:networkRevisionDerivationVersion:u16:R (=1)
  3:networkRevision:Sha256:R
  4:canonicalArtifactDigest:Sha256:R
  5:canonicalArtifactByteLength:u64:R

SourceMapBinding(0x0001):
  1:sourceMapFormatVersion:u16:R (=3)
  2:sourceMapDigest:Sha256:R
  3:sourceMapByteLength:u64:R
  4:compilerBuildId:Utf8:R
  5:sourceCollectionDigestVersion:u16:R
  6:sourceCollectionDigest:Sha256:R

PublicationProvenance(0x0001):
  1:publisherKind:u8:R
  2:publisherBuildId:Utf8:R
  3:artifactObjectKey:Utf8:R
  4:sourceMapObjectKey:Utf8:R
  5:controlledBuildProvenance:Utf8:O
  6:controlledTimestamp:Utf8:O
```

object key 精确为 `sha256/<64 lowercase hex>`。LFCP 不包含验证收据，也不绑定 LFSD；
LFSD 是供修订切换认证使用的诊断/迁移制品。真实性由 LFCP exact bytes 之外的外部认证
manifest/指针提供。

LaneFlow 顺序固定为：关闭 LFCA/LFSM/LFSD 候选 → 对最终 bytes 做结构和值域预检与 bundle
后发射闭合 → 按需从 checked capability 构造 LFCP 2。宿主、CI 或打包工具决定是否以及如何
持久化与认证这些 exact bytes；内容仓库、atomic no-replace、winner 和目录持久化不属于本
wire 或 compiler/format 核心合同。加载方仍须按受认证 LFCP/宿主描述符重新验证收到的 bytes。
职责边界见
[`compiler-post-emission-check-and-minimal-publication-closure.md` §9](compiler-post-emission-check-and-minimal-publication-closure.md#9-宿主持久化与原子边界)。

## 7. 格式安全天花板

格式 hard limit 与接收方 budget 是不同层。hard limit 保证单个 wire 原语可安全预检；
接收方 budget 按设备、工具或部署约束整个对象，但正式产品 profile 不得低于本节容量
合同。

| 限制                              | 值 / 规则                                                                           |
| --------------------------------- | ----------------------------------------------------------------------------------- |
| 对象 exact bytes                  | wire 为 checked `u64`；读取任一 section directory 前必须比较调用方 `maxObjectBytes` |
| section chunk 数                  | wire 为 `u32`；分配 directory 前必须比较调用方 `maxChunksPerSection`                |
| 单 `TableV1` chunk exact bytes    | `16,777,216`                                                                        |
| 单 chunk 行数                     | `65,536`                                                                            |
| 单逻辑表累计行数                  | `firstLogicalRow + rowCount` checked 且不得超过 `u32::MAX`                          |
| 单行字段数                        | `17`                                                                                |
| Identity ASCII value              | `53 bytes`                                                                          |
| 单 UTF-8 Field value              | `1,048,576 bytes`                                                                   |
| 单 chunk 累计 UTF-8 value         | `8,388,608 bytes`                                                                   |
| 单 vector item 数                 | `65,536`                                                                            |
| 单 chunk 累计 vector bytes        | `8,388,608 bytes`                                                                   |
| `RecordVector` 深度               | `1`                                                                                 |
| 单 LFSM SourceLocation chunk 行数 | `65,536`；完整 location ordinal 空间仍为全局 `u32`                                  |
| 同时 staged LFCA+LFSM+LFSD chunk  | `50,331,648 bytes`；不得据此缓存三个完整对象                                        |

`16,777,216` 只表示单 `TableV1` chunk ceiling，`50,331,648` 只表示三个同时 staged chunk
的内存 ceiling；两者都不是完整对象或完整三对象候选上限。完整对象与 bundle 由调用方
具名配置档中的受检 `u64` 预算约束，不得重新引入 16 MiB 完整对象或 48 MiB 完整候选限制。

reader/checker 必须先验证对象 exact length、section directory、chunk directory 的 checked
长度和调用方预算，再为 directory、行、字符串或向量分配。writer、digest 和 checker 必须
按 chunk 流式工作；不允许为了保留旧 48 MiB staging 模型而把完整百万级对象复制三份。
调用方预算可以更严格并返回明确的 budget error，但不得被写入 LFCA bytes、StableId、
`NetworkRevisionId` 或规范 chunk 边界。

安全天花板是拒绝上界，不自动宣称每个值都存在恰好达到上限的合法对象。每个维度必须在
值用于分配、hash 或 view 前证明 `limit+1` 失败；另以 reachability 表记录该维度最大可构造
合法值，只有证明 hard limit 本身可达时才要求接受 `limit`。对象登记的精确节/逻辑表种类、
singleton 行数、row 字段 shape 与 chunk directory 连续性不是资源天花板，必须直接验证精确值
接受、任意 `+1/-1` 失败。

已知长度来源在 hash 前比较 `exactLength <= caller maxObjectBytes`；未知长度 transport 最多
读取 `maxObjectBytes + 1`，观察到第 `+1` byte 立即失败。错误至少稳定区分 unsupported
version、budget/limit、truncated、arithmetic overflow、gap/overlap、unknown/duplicate kind、
non-canonical order/value、chunk digest 与 cross-object binding mismatch；错误文本不是协议。

### 7.1 单路网产品容量

一份 LFCA / 一个 `SharedNetworkRevision` 必须至少接受 `1000000` 个现实混合稳定静态
实体。这个数字统计 `CanonicalIdentity` 完整逻辑行，不是道路条数、单一 kind 行数、
活动车辆数或多个 world 的汇总。正式证据至少包括道路/车道/路口/信号/停车/准入/
CanonicalFrame，并在对应领域交付后包括 ConflictZone/ParticipantStream；不得用一百万
个无关系空壳行代替现实闭合。

验证阶梯固定为：

- `10000`：日常定向回归；
- `100000`：常规规模和近线性趋势；
- `1000000`：单逻辑修订容量门禁。

三档都报告 LFCA/LFSM/LFSD exact bytes、chunk 数与分布、compile/emission/check/load/build
各阶段时间、retained/scratch/peak bytes 和 digest。失败原子性与排列扰动由日常定向回归、
固定向量和规模证据按风险组合，不在十万/百万档重复已经与规模无关的同一分支；headless
与完整 Spatial 分开报告。路线登记另外使用现实短/典型/长路线、密集 passage、重复/
循环 occurrence 和聚合注册量验证；本合同不要求荒谬的“单条路线穿过一百万个冲突区”，
也不把一百万静态实体写成一百万活动车辆 fixed-tick 认证。

排列扰动必须区分两条边界：只改变模块准入顺序、内部集合或 hash 迭代顺序而不改变
official source exact bytes 时，三对象 exact bytes 必须相同；若直接重排来源文档中的声明，
`sourceDocumentDigest` 与 `sourceCollectionDigest` 按定义必须变化，此时要求语义节与
`NetworkRevisionId` 不变，而 provenance、LFSM 来源位置及其跨对象 binding 所在的 exact
bytes 和对象摘要相应变化。不得为了制造伪 exact-byte 相等而丢弃真实来源绑定。

三档 official source 编译都必须显式选择
`LF-COMP-SINGLE-NETWORK-1M-v2`，其精确有限向量见
[`compiler-foundation.md` §5.3](compiler-foundation.md#53-编译资源上限)。一百万档必须在
`max_stable_entity_count = 1000000` 内完成真实 compiler/emitter；手工拼装 LFCA、只运行
reader/builder、修改私有 limits 或使用 unlimited 测试入口均不构成该容量证据。大型配置档
只控制失败关闭资源上限，emitter 的 LFCA/LFSM/LFSD 必须落入 sealed closed staged file；
不得把 `max_portable_bundle_bytes` 解释为同等 RAM 预留或性能 SLA。

## 8. 实现验收

公共格式实现必须原子满足：

1. Road Editing v3、Road Editing frontend 3、Synthetic frontend 4、Identity registry
   revision 3、LFCA 4、LFSM 3 与 LFSD 3；
2. schema clean regeneration，删除 `ParkingArea` reader/writer/public symbol，加入冲突
   静态声明，不保留 alias、双读、双写或迁移 façade；
3. `laneflow-static-contract` 机器登记表、chunk directory 与本文逐项一致；
4. `laneflow-format` 对 unknown/extra/missing/duplicate/chunk order/digest/limit 做失败关闭；
5. compiler、LFSM/LFSD、LFCP、SharedNetworkRevision、Runtime、Spatial/Adapter 一次贯通；
6. ParkingFacility 与 ConflictZone/ParticipantStream/ConflictZoneRegion 的 known vectors、
   来源投影、语义差异、规范排序扰动和旧 LFCA 版本 rejection 定向反例；
7. `10000`/`100000`/`1000000` 单修订证据、真实 round-trip、bundle 失败零受检能力与
   SharedNetworkRevision 构建失败零部分根验证。
   三档均使用 `LF-COMP-SINGLE-NETWORK-1M-v2`，一百万档不得在 emission 前由较小 P100
   profile 拒绝；

固定向量不得在测试运行时用 production emitter 自己生成 expected。目标实现至少冻结并
检入输入、完整 expected bytes、SHA-256、exact length、object key 与 revision/binding：

| 向量族                          | 必须证明                                                                                            |
| ------------------------------- | --------------------------------------------------------------------------------------------------- |
| `min-headless`                  | 最小合法 LFCA 4 的八节、空逻辑表表达、首 chunk/section offset 与 revision framing                   |
| `lfca-full-spatial`             | 23 种可构造实体、所有合法 relation role、停车/冲突静态、规范 f32、三类 geometry 与 LFSM/LFSD 配对   |
| `lfsm-mixed-location`           | Synthetic Text 与 Road Editing Declaration/OwnerLocal/property/canvas、文档与来源集合摘要的完整绑定 |
| `provenance-*`                  | 语义相同而来源/build provenance 不同时 revision 相同、artifact digest 不同                          |
| `claim-mismatch`                | 只篡改 declared revision 时对象结构可读但 bundle binding 稳定失败                                   |
| `reorder-equivalent`            | declaration、集合与 hash iteration 扰动后 exact bytes 完全相同                                      |
| `signed-zero`                   | 编译边界把 `-0.0` 规范为 `+0.0`，负零 wire 失败                                                     |
| `lfsd-change-set` / `lfsd-noop` | add/remove/modify/reconnect/move、空间配置及空差异的完整两端绑定                                    |
| `lfcp-min-bindings`             | LFCP 2 的三节、`0x0068` 首节 offset、object keys 与无 receipt/LFSD binding                          |
| `path-anchor-boundary`          | Gate/EdgeBoundary/Interior 统一位置键、边界唯一 variant、entry/exit/order 与 admission Gate 派生    |
| `lfsm-role9-filtered-row`       | path occurrence 顺序/重复变化不改 filtered-row localIndex，primary 仍定位到选中来源 occurrence      |
| `parking-anchor-multiset`       | 同一 LaneEdge 多 anchor 删除/新增一个时保留基数，progress-only 变化只走完整字段 payload             |
| `closed-value-rejection`        | regulation tag 缺失或恰一行成功，零/多行与非法 `x-lane-*` / `x-*` ASCII token 稳定失败              |
| `lfsd-field-presence`           | required/optional 字段的四种 base/target 存在性只接受唯一 before/after payload 形状                 |

这些向量必须原子保持 `4/3/3/2`；不得把旧版本 bytes 当作当前成功向量，
也不得为保留旧 fixture 增加双读分支。

确定性矩阵至少覆盖 Windows x86-64 与 Ubuntu x86-64、single-thread 与 compiler 支持的全部
worker 数、两个 fresh process、不同 hash seed/分配地址，以及无 Spatial/完整 Spatial 两条
生产分支。安全矩阵覆盖每个 preamble/directory/chunk/table/row/field 边界的截断与追加、
单 bit 损坏、未知/重复/乱序、gap/overlap、chunk digest、length/digest/revision/source-map/
base-target 错配和所有 hard-limit `+1`。staging 矩阵在 write、patch、flush、finish、不可变映射
与 backing identity/length/bytes 漂移的失败点证明不返回受检 bundle；共享静态构建失败时不
返回部分根。规模证据与 exact-byte 分支证据按风险组合，不建立无意义的全轴笛卡尔积。

领域 Runtime 行为不属于本公共格式合同；实现不得顺带引入 Waiting admission、
ConflictArbiter 或 Parking lifecycle。未定义或互相冲突的 wire 选择必须先修订权威设计，
不能让实现细节成为第二套事实源。
