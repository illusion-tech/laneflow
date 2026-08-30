# 可移植规范制品与辅助制品格式

**文档状态**: Review（#549 G1；统一 LFCA 4 / LFSM 3 / LFSD 3 / LFCP 2）<br>
**最后更新**: 2026-08-30<br>
**适用范围**: LFCA、LFSM、LFSD、LFCP 的当前目标线格式、规范排序、跨对象绑定、
失败关闭与格式安全天花板<br>
**实现状态**: 本文只定义一套目标合同。生产实现仍只接受 LFCA 3；#549 G1 接受后必须
另立一个原子实现切片，统一切换到本文的 LFCA 4、LFSM 3、LFSD 3 与 Road Editing v3。
旧格式的设计和逐轮证据只从 Git/GitHub 历史追溯，不在当前权威正文保留第二套说明。<br>
**关联决策与设计**:

- [`../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`](../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md)
- [`../adr/0024-compiler-post-emission-check-and-minimal-publication-closure.md`](../adr/0024-compiler-post-emission-check-and-minimal-publication-closure.md)
- [`../adr/0025-checked-canonical-network-and-shared-static-network.md`](../adr/0025-checked-canonical-network-and-shared-static-network.md)
- [`../adr/0028-integer-millimeter-traffic-geometry.md`](../adr/0028-integer-millimeter-traffic-geometry.md)
- [`../adr/0029-retire-precompiled-static-route.md`](../adr/0029-retire-precompiled-static-route.md)
- [`parking-system.md`](parking-system.md)
- [Issue #540：虚拟停车设施与不可见停驻生命周期](https://github.com/illusion-tech/laneflow/issues/540)
- [Issue #283：冲突静态与空间配对](https://github.com/illusion-tech/laneflow/issues/283)
- [`network-compiler.md`](network-compiler.md)
- [`compiler-post-emission-check-and-minimal-publication-closure.md`](compiler-post-emission-check-and-minimal-publication-closure.md)
- [`shared-static-network.md`](shared-static-network.md)

## 1. 权威结论

当前目标对象集合只有四种。LFCA、LFSM 与 LFSD 使用同一套分块节格式；LFCP 继续使用
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
sourceMapFormatVersion                 = 3
semanticDiffFormatVersion             = 3
canonicalPublicationDescriptorVersion = 2
```

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

五个既有基础结构的规范性 ASCII 图如下；新增的两个 chunk-directory 结构以随后完整
offset 表为权威。图内从左到右、从上到下表示递增 wire byte offset；顶部 `0..31` 是
每个 4-byte 行内的 wire bit slot，多字节数值仍按小端序编码。

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
| `0x0003`    | `CanonicalEntityTables`      | 是               | 23 种可构造静态实体；kind 21 保留空位                  |
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
4:networkRevisionDerivationVersion:u16:R        (=1)
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
19 AccessRule        20 VehicleProfile    21 RESERVED
22 CanonicalFrame    23 ConflictZone      24 ParticipantStream
```

revision 3 的 identity field tag 沿用 v1 数值；tag 22 的名称为
`parkingFacilityKey`。tag 23 与不再可构造的 `routeKey` tag 30 保留空位。主要字段为：

```text
 1 authoringNamespaceId  2 corridorKey       3 sectionKey
 4 laneKey               5 laneEdgeKey       6 junctionKey
 7 pathKey               8 movementKey       9 directedEntryApproachKey
10 directedExitApproachKey                  11 movementStableId
12 entryEdgeStableId     13 exitEdgeStableId 14 maneuverPathStableId
15 gateKey               16 waitingZoneKey   17 stopLineKey
18 signalGroupKey        19 signalControllerKey
20 signalControllerStableId                 21 phaseKey
22 parkingFacilityKey    23 RESERVED         24 parkingSpaceKey
25 laneGroupKey          26 facilityBandKey  27 participantClassKey
28 accessRuleKey         29 vehicleProfileKey 30 RESERVED
31 canonicalFrameKey     32 roadSectionStableId
33 roadCorridorStableId  34 junctionStableId
35 conflictZoneKey       36 participantStreamKey
```

revision 3 新增 kind 的 required tag sequence 为：

```text
23 ConflictZone      [1 authoringNamespaceId, 34 junctionStableId, 35 conflictZoneKey]
24 ParticipantStream [1 authoringNamespaceId, 34 junctionStableId, 36 participantStreamKey]
```

新增 kind 必须提升 `identityRegistryRevision`，但不得改变既有 kind 的 identity canonical
bytes 或 `StableId128`。只有修改既有 kind 的 required field、tag 含义或编码时才提升
`identityEncodingVersion`。`ParticipantStream.maneuverPath` 不进入身份前像；重新归属
Junction 必须产生新 identity。

`Ascii` identity value 必须为 1..=53 bytes；首 byte 属于 `[A-Za-z0-9]`，其余只属于
`[A-Za-z0-9._:/-]`。对每个 kind，发射器按完整 identity bytes 的无符号逐字节字典序
排序并从 0 连续分配 typed ordinal。全部可构造 kind 的 `StableId128` 必须全局唯一；
重复前像和 BLAKE3-128 截断碰撞都失败关闭。

kind 14 / tag 22 只原子改名，数值和相同 namespace+key 的前像字节不变；因此同一现实
设施的 `StableId128` 不因 `ParkingArea -> ParkingFacility` 改名而变化。LFCA 格式与设施
语义仍变化，所以新旧 artifact 的 `NetworkRevisionId` 不会被判成同一修订。

### 3.4 实体表登记

`CanonicalEntityTables(0x0003)` 精确包含下列 23 个逻辑表种类；`0x0015` 禁止出现。
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
| `0x0016`  | CanonicalFrame    | 无额外字段                                                                                                                                                                                                                                                                                                         |
| `0x0017`  | ConflictZone      | `3:junction:u32:R`                                                                                                                                                                                                                                                                                                 |
| `0x0018`  | ParticipantStream | `3:junction:u32:R, 4:maneuverPath:u32:R, 5:passages:RecordVector:R`                                                                                                                                                                                                                                                |

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

`ParticipantStream.passages` 的 anchor kind 固定为 `0=Gate, 1=EdgeBoundary, 2=Interior`。
Gate reference 是同一 stream ManeuverPath 的 `ManeuverGate` typed ordinal；EdgeBoundary
reference 是 `0..=pathEdgeCount` 的 boundary index；Interior reference 是
`0..<pathEdgeCount` 的 path edge index，且对应 progress tag 必需并满足
`0 < progressMillimetres < edgeLengthMillimetres`。非 Interior 禁止 progress tag。
admission Gate 必须从 entry anchor 唯一派生，不进入 LFCA passage row。

上表连同 `CanonicalIdentity.identityFields` 已穷举 LFCA 4 的全部 `RecordVector` 行布局；
这些内嵌行均不得再含 `RecordVector`。任何未登记的内嵌字段、额外 tag 或第二层嵌套失败关闭。

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

`ConflictZoneRegion` 每个 ConflictZone 至多一行；ring 至少三个不同点，wire 不重复首点，
从 `+Y` 观察必须逆时针，首点是 `(x,z)` 词典序最小点，ring 无自交且面积为正，
`minY < maxY`。region 只服务验证、调试与表现，缺失时 headless 冲突行为保持完整。

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

RoadEditing 变体的 optional 字段还受以下闭合矩阵约束：

| subject            | 必需                                                                                          | 禁止/条件                                                                                                                                                                    |
| ------------------ | --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ModuleHeader(0)`  | 无额外字段                                                                                    | tag 10..20 禁止；tag 21 可选                                                                                                                                                 |
| `RoadAlignment(1)` | tag 10 `moduleNamespace`、tag 15 `localKey`                                                   | tag 11 禁止；tag 12..14/16..19 禁止；tag 20/21 可选                                                                                                                          |
| `Declaration(2)`   | tag 10 `moduleNamespace`、tag 11 `entityKind`、tag 15 `localKey`                              | owner-local tag 16..19 禁止；tag 12..14 按实体 owner 深度连续出现；tag 20/21 可选                                                                                            |
| `OwnerLocal(3)`    | tag 16 `ownerKind`、17 `roadEditingRelationKind`、18 `occurrenceKind`、19 `occurrenceOrdinal` | `ownerKind=0(ModuleHeader)` 禁止 tag 10..15；`ownerKind=1(Address)` 要求 tag 10/15，tag 11 仅 Declaration owner 存在；tag 12..14 按 owner 深度连续；tag 20 必需，tag 21 可选 |

`propertySteps` 必须有 `1..=4` 行并构成 Road Editing v3 登记的一条完整可达路径，不能只因
各 step 单独合法就拼接。`sourceLanguage` 只允许 `1=SyntheticDsl` 与
`3=RoadEditingSource`；LFSM 3 分别要求 `frontendVersion=4` 与 `frontendVersion=3`。
前者只允许 Text，后者只允许 RoadEditing。

模块按依赖优先 Kahn 顺序排列；ready set 以完整 namespace UTF-8 bytes 取最小项。文档在
模块内按完整 `sourceDocumentKey` bytes 排序。位置池按完整位置语义值排序去重并从 `0`
连续编号；位置引用向量按位置语义值排序，不按哈希遍历、插入顺序或来源 vector 顺序。

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
28/29/36，并且必须有相同行键的 `OwnerLocalSource` 父行。`DerivedRelationSource` 当前只允许
role 9；保留 role 13..16 禁止出现。

### 4.3 完整 `sourceRelationRole` 登记

`localIndex` 一律是 owner 内局部下标；`vector` 是相应 LFCA vector/RecordVector 的零基位置，
`scalar` 在字段存在时固定为 `0`，`filtered row` 是按表规范行键过滤 owner 后的零基位置。
`set` 的位置只服务 wire 排序和来源定位，不产生 LFSD Move；`domain` 的位置属于语义顺序。

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
| `13..16 RESERVED`                       | —                 | —                             | 禁止出现                                    | —                   | —                        |
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
| `30 ParkingFacilityVirtualEntry`        | ParkingFacility   | LaneEdge                      | `ParkingFacility.virtualEntries[].laneEdge` | vector / set        | Relation + field payload |
| `31 ParkingFacilityVirtualExit`         | ParkingFacility   | LaneEdge                      | `ParkingFacility.virtualExits[].laneEdge`   | vector / set        | Relation + field payload |
| `32 JunctionConflictZone`               | Junction          | ConflictZone                  | 按 `ConflictZone.junction` 过滤实体行       | filtered row / set  | Relation                 |
| `33 JunctionParticipantStream`          | Junction          | ParticipantStream             | 按 `ParticipantStream.junction` 过滤实体行  | filtered row / set  | Relation                 |
| `34 ParticipantStreamManeuverPath`      | ParticipantStream | ManeuverPath                  | `ParticipantStream.maneuverPath`            | scalar              | Relation                 |
| `35 ParticipantStreamConflictPassage`   | ParticipantStream | ConflictZone                  | `ParticipantStream.passages[].conflictZone` | vector / domain     | Relation + field payload |
| `36 CanonicalFrameConflictZoneRegion`   | CanonicalFrame    | ConflictZone                  | `ConflictZoneRegion` owner rows             | filtered row / set  | Geometry only            |

role 21 的名称原子改为 `ParkingSpaceFacility`，数值不变；role 30/31 是追加项，与 identity
field tag 是不同编号空间，不复活 `StaticRoute`。设施声明、`virtualCapacity` 和每个 anchor
都必须回指 exact Road Editing v3 property path。anchor 先按
`(LaneEdge StableId128, progressMillimetres)` 规范排序，因此来源 vector 顺序不能改变
canonical localIndex。

新增 role 的 Road Editing v3 primary-source projection 固定为：

| role | primary declaration / owner-local path                                                                      |
| ---: | ----------------------------------------------------------------------------------------------------------- |
|   30 | `RoadEditingSource.parking_facilities[].virtual_entries[]`                                                  |
|   31 | `RoadEditingSource.parking_facilities[].virtual_exits[]`                                                    |
|   32 | `RoadEditingSource.conflict_zones[].junction`                                                               |
|   33 | `RoadEditingSource.participant_streams[].junction`                                                          |
|   34 | `RoadEditingSource.participant_streams[].maneuver_path`                                                     |
|   35 | `RoadEditingSource.participant_streams[].passages[]`；完整 passage element 是 owner-local subject           |
|   36 | `RoadEditingSource.conflict_zone_regions[].canonical_frame`；ring/high range 共用该 region primary location |

role 35 的 local index 是 passage 规范领域顺序，不是来源 vector 位置；role 30/31/32/33/36
是 set，只在 relation tuple 集合变化时生成 LFSD relation change。role 36 的
`SpatialGeometrySourceRange` 覆盖 ring point range；role 35 的 entry/exit property steps
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
`StableId128` 的 `entityKind` 和完整 identity 前像逐字节相同；否则整份 diff 失败。

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
| Entity `Modify`      | `subjectStableId, fieldTag`                                               | before/after 至少一个存在；都存在时必须不同                                |
| Relation `Add`       | `ownerStableId, subjectStableId, role, afterLocalIndex`                   | tag 9/10 禁止                                                              |
| Relation `Remove`    | `ownerStableId, subjectStableId, role, beforeLocalIndex`                  | tag 9/10 禁止                                                              |
| Relation `Move`      | `ownerStableId, subjectStableId, role, beforeLocalIndex, afterLocalIndex` | tag 9/10 禁止，两个 index 必须不同                                         |
| Relation `Reconnect` | `ownerStableId, role, beforeLocalIndex, afterLocalIndex`                  | `beforeTarget:R, afterTarget:R`，`subjectStableId:F`，两个 target 必须不同 |
| Geometry `Add`       | `subjectStableId`                                                         | `afterCanonicalValue:R`，before 禁止                                       |
| Geometry `Remove`    | `subjectStableId`                                                         | `beforeCanonicalValue:R`，after 禁止                                       |
| Geometry `Modify`    | `subjectStableId`                                                         | before/after 都必需且不同                                                  |
| StaticRule `Modify`  | `subjectStableId, fieldTag`                                               | before/after 至少一个存在；都存在时必须不同                                |

Relation 行的 `entityKind` 是 owner kind，其余表是 subject kind；实体 kind 21 禁止出现。
Relation 只允许 role `1..12, 17..18, 20..27, 30..35`。role 13..16 保留，role 19 只投影
StaticRule，role 28/29/36 只投影 Geometry。

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

`ParkingFacility.virtualEntries/virtualExits` 必须双重闭合，而不是重复表达同一信息：

- role 30/31 的 RelationChange 表达 owner、LaneEdge subject 和规范 localIndex；
- tag 5/6 的 Entity Modify 表达完整 anchor payload，确保同一 LaneEdge 上只有
  `progressMillimetres` 变化时仍可观察；
- tag 5/6 的 `SemanticFieldValueV1` 精确为
  `count:u32 || (LaneEdge StableRefV1 || progressMillimetres:u32)[count]`，保持 LFCA 规范顺序；
- 任一 anchor 语义 vector 变化都必须产生 tag 5/6 字段投影；只有
  `(owner, role, LaneEdge subject, canonical localIndex)` relation tuple 集合实际变化时才
  另外产生 RelationChange。仅 progress 改变而 LaneEdge/localIndex 不变时不得伪造
  Add/Remove/Move/Reconnect；
- `virtualCapacity` 只由 tag 4 StaticRule Modify 表达；显式成员只由 tag 3/role 21 闭合。

这项双投影是必要的现实边界：anchor 没有全局 StableId，而仅凭 relation 的 LaneEdge
StableId 无法区分同一条边上的不同 progress。

`ParticipantStream.passages` 使用同样的“relation + field payload”闭合：role 35 表达
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
生成 role 35 Add/Remove/Move。admission Gate 始终重新派生，不进入 LFSD。Genesis 和
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
只允许 Artifact 两端不同时，要求 tag 1/2/3。相同时该表为空。

Genesis 对目标每个实体、合法 relation tuple 和 geometry 分别产生 Add，禁止
Remove/Modify/Move/Reconnect，StaticRule 为空，并恰有一条空间 Initialize。Artifact diff
由两份受绑定 LFCA 独立重算。LFCA 3 到 LFCA 4 是不支持的语义合同转换，不生成跨格式 LFSD。
`semanticDiffDigest` 是完整 LFSD exact bytes 的 SHA-256；LFSD 自身不授予迁移权限。

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

发布顺序固定为：关闭 LFCA/LFSM/LFSD 候选 → 对最终 bytes 做结构和值域预检与最小
后发射闭合 → 安装 content-addressed objects → 构造并安装 LFCP 2 → 外部认证 manifest
恰好一次提交。任何失败都不得返回可发布 capability，也不得让部分安装对象变成已认证
发布。

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

reader/checker 必须先验证对象 exact length、section directory、chunk directory 的 checked
长度和调用方预算，再为 directory、行、字符串或向量分配。writer、digest 和 checker 必须
按 chunk 流式工作；不允许为了保留旧 48 MiB staging 模型而把完整百万级对象复制三份。
调用方预算可以更严格并返回明确的 budget error，但不得被写入 LFCA bytes、StableId、
`NetworkRevisionId` 或规范 chunk 边界。

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
各阶段时间、retained/scratch/peak bytes、digest、失败原子性和声明排列扰动。headless 与
完整 Spatial 分开报告。路线登记另外使用现实短/典型/长路线、密集 passage、重复/
循环 occurrence 和聚合注册量验证；本合同不要求荒谬的“单条路线穿过一百万个冲突区”，
也不把一百万静态实体写成一百万活动车辆 fixed-tick 认证。

## 8. 统一实现验收

#549 G1 接受后必须另立一次 clean-break 的公共格式实现切片，并同步完成：

1. Road Editing v3、Road Editing frontend 3、Synthetic frontend 4、Identity registry
   revision 3、LFCA 4、LFSM 3 与 LFSD 3；
2. schema clean regeneration，删除 `ParkingArea` reader/writer/public symbol，加入冲突
   静态声明，不保留 alias、双读、双写或迁移 façade；
3. `laneflow-static-contract` 机器登记表、chunk directory 与本文逐项一致；
4. `laneflow-format` 对 unknown/extra/missing/duplicate/chunk order/digest/limit 做失败关闭；
5. compiler、LFSM/LFSD、LFCP、SharedNetworkRevision、Runtime、Spatial/Adapter 一次贯通；
6. ParkingFacility 与 ConflictZone/ParticipantStream/ConflictZoneRegion 的 known vectors、
   来源投影、语义差异、规范排序扰动和 LFCA 3 rejection 定向反例；
7. `10000`/`100000`/`1000000` 单修订证据、真实 round-trip 与发布事务原子性验证。

领域 Runtime 行为仍由 #282/#283/#540 的后继实施拥有；公共格式实现不得顺带实现
Waiting admission、ConflictArbiter 或 Parking lifecycle。实现发现本文存在未定义或互相
冲突的 wire 选择时必须回到 #549 G1 修正文档，不能让实现细节成为第二套事实源。
