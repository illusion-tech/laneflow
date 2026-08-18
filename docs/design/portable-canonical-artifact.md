# 可移植规范制品与辅助制品格式

> **后继覆盖（2026-08-18）**：Accepted ADR 0024 / #299 G2 已交付且不再交付独立
> `laneflow-validator` 或验证收据，并以 `laneflow-format` 的后发射检查和最小发布
> 闭合替代。生产代码现只支持不含 receipt 的 LFCP v2。本文 LFCP v1、
> `ValidationReceiptBinding` 与 receipt 安装步骤只保留为 #298 历史设计、实现和验证
> 证据，LFCP v2、API、检查深度和性能边界改以
> `compiler-post-emission-check-and-minimal-publication-closure.md` 为准。无论后继实现是否
> 生效，本文 LFCA/LFSM/LFSD v1 wire、已完成 emitter/format/安装证据及对象外信任锚
> 均保持有效。

**文档状态**: Accepted（#298 G1 Pass；G4 已完成，动态记录以 Issue Gate Ledger 为准）<br>
**最后更新**: 2026-08-18<br>
**适用范围**: `laneflow-format`、`laneflow-static-contract`、
`laneflow-compiler` 的可移植规范制品（Portable Canonical Artifact）、源映射封套
（Source Map Envelope）、语义差异封套（Semantic Diff Envelope）、规范发布描述符
（Canonical Publication Descriptor）与原子发布边界<br>
**实现状态**: G4 已完成；当前已建立共享 magic/version/field type/硬上限值、
framing、Table/Row/Field 通用结构预检、附录 A registry 零拷贝有类型遍历、对象内直接
值域检查，以及先完整计量、再写调用方提供的精确长度缓冲区且失败不改变输出的无分配
受限写入器。`laneflow-compiler` 已从同一个 `CompilationOutput` 原子发射 LFCA/LFSM 以及
Genesis 或 checked-base LFSD 的内存候选，并关闭 exact bytes、digest、length、object key、
revision 和跨对象 binding；`LFCA-V1-FULL-SPATIAL` LFCA/LFSM/Genesis LFSD、复用同一 LFCA
作为 base/target 的 `LFSD-V1-NOOP`，以及闭合实体 add/remove/modify、关系 reconnect、
geometry add、静态规则 modify 和全局空间 modify 的 `LFSD-V1-CHANGE-SET` 固定对象已提交，
最小 headless 锚点与 `PROVENANCE-ONLY`、`CLAIM-MISMATCH`、`REORDER-EQUIVALENT`、
`SIGNED-ZERO` LFCA 变体包也已提交，并由只读 exact-byte 测试约束。#298-owned
ART/MAP/DIFF 与 `SEC-001..015` 已闭合；本地 `LocalPortableObjectInstaller` 已实现同文件系统 staging、
flush/sync/close、digest key、hard-link atomic no-replace、并发 winner exact-byte 复用/冲突和
unsupported fail-closed。#298 的 LFCP v1/receipt 发布路径只保留为历史证据；#299 G2 已以
最终字节后发射检查、LFCP v2 与外部认证 manifest 单提交 adapter 取代该生产路径。
Windows/Ubuntu exact-byte CI 已对独立进程输出完成集中逐字节比较；
#292 合法 production workload 上的 P100 emitter 时延、候选暂存和三类输出大小也已登记。
当前检查只闭合发布所需的 digest/length/revision 与 LFSM/LFSD binding，不重跑完整路网语义；
候选或已安装对象仍不得解释为 trusted/published artifact<br>
**关联 Issue**: #298（历史主议题）、#299（后继闭合）；依赖 #292、#296<br>
**关联文档**: `network-compiler.md`、`compiler-foundation.md`、
`numeric-representation.md`、
`../adr/0020-compiler-owned-static-network-and-static-image.md`、
`../adr/0025-checked-canonical-network-and-shared-static-network.md`、
`../reference/v0.10-portable-artifact-validation.md`、
`../reference/glossary.md`

## 1. 状态、目标与非目标

本文把 `network-compiler.md` 第 8 节和 ADR 0020 已接受的长期边界收窄为 #298 已重新接受的
实现级格式输入。此前 G1 曾因 facility-only spatial output 无法表示、跨修订身份前像碰撞和混合
RoadEditing/Synthetic geometry 的方向配置档适用性而重开；这些缺口现已闭合，并由 #298 Gate
Ledger 中新的正式 `## G1 设计判断` 重新接受。当时的 G1 Pass 只授权准备独立 G2 开工判断，
不表示格式已经实现、通过验证或可供产品发布；此后 #298 已依次通过 G2、G3 与 G4，精确记录
以其 Gate Ledger 为准。

#298 必须同时闭合四类对象：

1. 可移植规范制品：完整、目标无关的规范静态路网语义；
2. 源映射封套：同次成功编译的来源位置和来源沿袭；
3. 语义差异封套：一份基线制品与目标制品之间的结构化变化；
4. 规范发布描述符：#298 历史版本绑定验证收据；当前 LFCP v2 只绑定 LFCA/LFSM 与发布
   provenance。

本文不实现或授予下列权威：

- 本文不授权完整语义信任；#299 当前后发射检查也不承担第二套完整语义验证；
- 受检 LFCA → `SharedNetworkRevision` 的构建闭合与性能布局属于 #300；
- Traffic Runtime / Spatial 共享静态路网消费路径属于 #301；
- 可信路网切换描述符、运行时快照、迁移授权与在线切换属于 #302；
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
```

职责冻结为：

| 包                         | 拥有职责                                                                                   | 禁止拥有                                                |
| -------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------- |
| `laneflow-static-contract` | 版本轴、摘要/长度值、`NetworkRevisionId`、实体/字段/记录种类登记、有类型序号和描述符值类型 | 文件系统、序列化器、完整编译器语义遍、Runtime、Spatial  |
| `laneflow-format`          | 四类对象的精确线格式、受限写入器、结构/直接值域预检，以及最终字节的最小后发射闭合检查      | 完整路网语义重建、LFSD 完备性验证、Runtime/Spatial 构造 |
| `laneflow-compiler`        | 发射 LFCA/LFSM/LFSD，调用后发射检查，构造 LFCP v2，并执行失败原子的安装与 manifest 单提交  | 第二套语义后端、可信发布签名、运行时迁移授权            |

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

G1 必须让审阅者只依据本文的字段登记、偏移公式和少量已知向量人工重建关键字节与摘要；
若做不到，必须在 G1 内补齐规范，不能把某个 Rust 库版本、生成器或自举脚本提升为隐式
事实源。production emitter 的逐字节实现属于 G2；#299 不再建立独立验证器，也不为 G1
另建 JSON registry、JSON Schema、代码生成器或第三套通用 oracle。

## 3. 统一线格式规则

### 3.1 基本类型

所有对象遵守下列逐字节规则：

- 所有无符号整数采用固定宽度小端序；线格式不使用 `usize`、原生指针或可变长整数；
- `i32` 采用固定 4-byte 二进制补码小端序；v1 只在 `AccessRule.priority` 使用；
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

### 3.2 ASCII 线格式图约定

本文的线格式图参考 [RFC 2360 第 3.1 节][rfc2360-packet-diagrams] 与
[Augmented Packet Header Diagrams draft-13][augmented-packet-diagrams]。采用规则如下：

- 固定宽字段使用每行 32 bit、顶部标出 bit slot 的横向图；强字节对齐的变长字段使用
  破边框 `:`，重复子结构使用方括号；
- 结构使用唯一英文名并由 “A/An ... is formatted as follows:” 引入，图后以
  “where:” 给出与图中完全相同的字段全名、短名、宽度与值约束；
- 图内从左到右、从上到下表示递增 wire byte offset。RFC 2360 示例采用网络字节序，
  但 LaneFlow v1 按 §3.1 的既定裁决使用**小端序**；顶部 `0..31` 是 4-byte 行内
  wire bit slot，不表示多字节整数的数值高低位；
- 跨两行的 64-bit field 是一个连续 8-byte little-endian value，中间的 `+` 只表示
  同一字段继续，不是字段边界；
- 图是结构化可读视图；紧随的 `where:`、offset/constraint 表与附录 A 是规范约束。
  字段名、顺序、宽度必须一致；如图与规范约束不一致，该不一致是 G1 blocker，不能由
  实现者任选其一，也不能通过解析 Markdown 图生成代码来裁决；
- 本文只参考 draft-13 的可读、结构化绘图约定，不声明该已过期 Internet-Draft 是
  LaneFlow 的外部协议依赖，也不声称当前 Markdown 可由其原型工具直接生成 parser。

### 3.3 对象前导与节目录

四类对象共用 32 字节 `ObjectPreambleV1`。

An ObjectPreambleV1 is formatted as follows:

```text
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
```

where:

Magic: 4 bytes.

Format Version (FV): 2 bytes; FV == 1. 每类对象拥有独立版本轴。

Header Byte Length (HL): 2 bytes; HL == 32.

Flags: 4 bytes; Flags == 0.

Section Count (SC): 4 bytes. 必须等于该对象 v1 的封闭节数。

Section Directory Offset (SDO): 8 bytes; SDO == 32.

Object Byte Length (OBL): 8 bytes. 必须等于受限读取器观察到的 exact byte length。

约束索引如下：

| 偏移   | 宽度 | 字段                     | v1 约束                                    |
| ------ | ---- | ------------------------ | ------------------------------------------ |
| `0x00` | 4    | `magic`                  | 对象专用 ASCII magic                       |
| `0x04` | 2    | `formatVersion`          | 对象专用版本；v1 为 `1`                    |
| `0x06` | 2    | `headerByteLength`       | v1 固定为 `32`                             |
| `0x08` | 4    | `flags`                  | v1 固定为 `0`，未知 bit 失败关闭           |
| `0x0c` | 4    | `sectionCount`           | 必须等于对象 v1 的封闭节数                 |
| `0x10` | 8    | `sectionDirectoryOffset` | v1 固定为 `32`                             |
| `0x18` | 8    | `objectByteLength`       | 必须等于外部受限读取器观察到的精确字节长度 |

每个目录项是 24 字节 `SectionDirectoryEntryV1`。

A SectionDirectoryEntryV1 is formatted as follows:

```text
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
```

where:

Section Kind (SK): 2 bytes. 对象专用封闭枚举，按 wire order 严格递增且不得重复。

Section Format Version (SFV): 2 bytes; SFV == 1.

Flags: 4 bytes; Flags == 0.

Byte Offset (BO): 8 bytes. 第一节紧随目录，后续节紧随前一节。

Byte Length (BL): 8 bytes. 允许对象专用规则定义的空节，但不得越界。

约束索引如下：

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

### 3.4 规范记录编码

实体、关系、来源和差异表使用同一显式记录框架。

A TableV1 is formatted as follows:

```text
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
```

where:

Table Kind (TK): 2 bytes.

Table Schema Version (TSV): 2 bytes; TSV == 1.

Row Count (RC): 4 bytes.

Rows Byte Length (RSBL): 8 bytes. 必须等于 `Rows` 的 exact byte length。

Rows: [RowV1]; count(Rows) == RC. 这是由 RC 和 RSBL 双重约束的连续 RowV1 序列。

偏移/约束索引如下：

| 偏移（相对 TableV1） | 宽度   | 字段                 | v1 约束                                              |
| -------------------- | ------ | -------------------- | ---------------------------------------------------- |
| `0x00`               | 2      | `tableKind`          | 由对象专用 registry 登记                             |
| `0x02`               | 2      | `tableSchemaVersion` | v1 固定为 `1`                                        |
| `0x04`               | 4      | `rowCount`           | 必须等于完整 RowV1 数量                              |
| `0x08`               | 8      | `rowsByteLength`     | 必须等于全部完整 RowV1 字节长度之和                  |
| `0x10`               | `RSBL` | `rows`               | 连续、无填充；TableV1 总长度必须精确等于 `16 + RSBL` |

A RowV1 is formatted as follows:

```text
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
```

where:

Row Byte Length (RBL): 8 bytes. 包含 16-byte Row header 与全部 Fields。

Field Count (FC): 4 bytes.

Reserved: 4 bytes; Reserved == 0.

Fields: [FieldV1]; count(Fields) == FC. Field Tag 必须严格递增且不得重复。

偏移/约束索引如下：

| 偏移（相对 RowV1） | 宽度 | 字段            | v1 约束                                          |
| ------------------ | ---- | --------------- | ------------------------------------------------ |
| `0x00`             | 8    | `rowByteLength` | 包含 16-byte header 和全部完整 FieldV1           |
| `0x08`             | 4    | `fieldCount`    | 必须等于完整 FieldV1 数量                        |
| `0x0c`             | 4    | `reserved`      | v1 固定为 `0`                                    |
| `0x10`             | 可变 | `fields`        | 连续、无填充；总长度必须精确等于 `rowByteLength` |

A FieldV1 is formatted as follows:

```text
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

where:

Field Tag (FT): 2 bytes.

Field Type (T): 1 byte. 必须是下文登记的封闭枚举。

Flags: 1 byte; Flags == 0.

Value Byte Length (VBL): 8 bytes. 只计算紧随 12-byte Field header 的 Value。

Value: VBL bytes. 编码由 Field Type 和 table/field registry 共同约束。

偏移/约束索引如下：

| 偏移（相对 FieldV1） | 宽度  | 字段              | v1 约束                                             |
| -------------------- | ----- | ----------------- | --------------------------------------------------- |
| `0x00`               | 2     | `fieldTag`        | 在所属 table registry 中登记并严格递增、不得重复    |
| `0x02`               | 1     | `fieldType`       | 必须是封闭枚举并与 registry 登记类型一致            |
| `0x03`               | 1     | `flags`           | v1 固定为 `0`                                       |
| `0x04`               | 8     | `valueByteLength` | 必须与固定宽度类型或登记的变长值约束一致            |
| `0x0c`               | `VBL` | `value`           | 连续、无填充；FieldV1 总长度必须精确等于 `12 + VBL` |

下面的等价伪码用于核对组合关系，不建立第二套字段名或宽度：

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
`10=Bytes`、`11=OrdinalVectorU32`、`12=RecordVector`、`13=i32`。固定宽度类型的
`valueByteLength` 必须精确匹配。`OrdinalVectorU32` 的 value 精确为
`count:u32 || item:u32[count]`，因此 VBL 必须等于 `4 + count * 4`；`RecordVector` 的
value 精确为 `count:u32 || RowV1[count]`，VBL 必须等于 `4 + sum(rowByteLength)`。
`rowByteLength` 包含 16 字节 Row header 与全部 Field；`valueByteLength` 只计算紧随
12 字节 Field header 的 value；`rowsByteLength` 必须等于所有完整 Row 字节长度之和。
`RecordVector` 只允许 registry 明确登记的一层内嵌 Row，内嵌 Row 不得再含
`RecordVector`。任何冗余长度、计数或深度不一致都失败关闭。

对象节先保存 `tableCount:u32`，随后保存按 `tableKind` 严格递增的 Table。每个
`tableKind` 的字段登记、必需字段、字段类型、行 key 和排序键必须进入
`laneflow-static-contract` 的 append-only registry，并由本文附录 A 冻结。`tableKind`
只在 `(object magic, sectionKind)` 内解释；附录 A 的 `R` 表示必需字段、`O` 表示省略即
`None` 的可选字段，v1 不用 sentinel 或零值表达缺失。未知表、
未知字段、缺失必需字段、类型错配、非最小长度或非规范排序均失败关闭。

[rfc2360-packet-diagrams]: https://www.rfc-editor.org/rfc/rfc2360.html#section-3.1
[augmented-packet-diagrams]: https://www.ietf.org/archive/id/draft-mcquistin-augmented-ascii-diagrams-13.html
[g1-evidence-lanes-correction]: https://github.com/illusion-tech/laneflow/issues/298#issuecomment-5322639700

## 4. 可移植规范制品 `LFCA` v1

### 4.1 封闭节

`magic = "LFCA"`，`formatVersion = canonicalFormatVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                         | 是否进入规范语义载荷 | 内容                                                                           |
| ------------- | ---------------------------- | -------------------- | ------------------------------------------------------------------------------ |
| `0x0001`      | `ContractVersions`           | 是                   | identity、registry、`NetworkRevisionId` 派生算法、constraint、execution 版本轴 |
| `0x0002`      | `CanonicalIdentityTable`     | 是                   | 完整身份前像、声明 StableId128、有类型 ordinal                                 |
| `0x0003`      | `CanonicalEntityTables`      | 是                   | 22 种稳定实体和目标无关规范静态值                                              |
| `0x0004`      | `CanonicalRelationTables`    | 是                   | 拓扑、成员、出现项、索引和静态规则关系                                         |
| `0x0005`      | `CanonicalSpatialTables`     | 是                   | 空间存在标记、规范 f32 折线与派生采样；headless 时为规范空表                   |
| `0x0006`      | `StaticExecutionConstraints` | 是                   | worker 数无关的静态执行约束                                                    |
| `0x0007`      | `CompilerProvenance`         | 否                   | compiler build、来源集合摘要、显式编译选项与发射器版本                         |
| `0x0008`      | `ArtifactClaims`             | 否                   | 制品声明的 `NetworkRevisionId`；只供独立重算比较，不自证可信                   |

LFCA v1 的组合视图如下。该图只展开已由 §3.3 定义的公共前导、目录和变长节，不引入
第二种容器；第一节 wire offset 固定为 `0x00e0`（`32 + 8 * 24`）。

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

所有节都必须存在；“无空间”用 `CanonicalSpatialTables` 内登记的显式 `spatialPresent=0`
表示，不能通过删节表示。`CompilerProvenance` 与 `ArtifactClaims` 都会改变 artifact
exact bytes 和 `canonicalArtifactDigest`，但二者都不进入规范路网语义载荷，因而不得
改变 `NetworkRevisionId`。

### 4.2 规范路网语义载荷与修订标识

规范路网语义载荷不是完整 artifact bytes。它只按
`sectionKind=0x0001..0x0006` 顺序连接标记为语义节的 exact bytes：

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

`ContractVersions` 只保存 `networkRevisionDerivationVersion` 等派生契约版本，不保存
摘要结果自身。编译器从六个语义节计算 `NetworkRevisionId`，再把结果写入非语义的
`ArtifactClaims.declaredNetworkRevisionId`。该声明始终是不受信任输入；#299 后发射检查从
最终 LFCA 的六个语义节重算并逐字节比较。当前检查不另建身份、拓扑或规则语义后端。
相同修订对应不同规范语义载荷时不得追加随机数、ordinal 或 suffix。

完整 artifact exact bytes 另由 SHA-256 得到 `canonicalArtifactDigest`，长度是同一字节
序列的 `u64` 精确长度。摘要与长度不嵌回自身字节；二者由外部描述符绑定。

## 5. 源映射封套 `LFSM` v1

`magic = "LFSM"`，`sourceMapFormatVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                     | 内容                                                                   |
| ------------- | ------------------------ | ---------------------------------------------------------------------- |
| `0x0001`      | `SourceMapBindings`      | 修订派生版本/值、artifact digest/length、compiler build 与来源集合摘要 |
| `0x0002`      | `SourceModules`          | 依赖优先模块、来源文档、闭合位置池、frontend/import provenance         |
| `0x0003`      | `StableEntitySources`    | `(entityKind, StableId128, typedOrdinal)` 与 owning/contributing 位置  |
| `0x0004`      | `OwnerLocalSources`      | owner StableId128、typed role、`localIndex`、来源位置与空间点范围      |
| `0x0005`      | `DerivedRelationSources` | generated relation 推导链、pass/constraint version                     |

LFSM v1 的组合视图如下。该图只展开已由 §3.3 定义的公共前导、目录和变长节，不引入
第二种容器；第一节 wire offset 固定为 `0x0098`（`32 + 5 * 24`）。

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

LFSM v1 的来源位置必须无损投影共同编译器已经冻结的闭合和类型 `SourceLocation`，不能
把不同前端降级成统一 UTF-8 字节范围：

```text
SourceLocationV1 :=
  Text {
    sourceModuleOrdinal:u32
    sourceDocumentOrdinal:u32
    startLine:u32
    startColumn:u32
    endLine:u32
    endColumn:u32
  }
  | RoadEditing {
    sourceModuleOrdinal:u32
    sourceDocumentOrdinal:u32
    subject:
      ModuleHeader
      | RoadAlignment(address)
      | Declaration(address)
      | OwnerLocal(owner, relation, occurrence)
    propertyPath: None | Some(1..=4 RoadEditingPropertyStep)
    canvasSelection: None | Some(Utf8)
  }
```

`sourceLocationKind` 是 `u8` 封闭判别值：`0=Text`、`1=RoadEditing`。每个位置作为
registry 登记的 RowV1 编码，公共 ordinal 与相应变体字段都是必需字段；另一变体字段、
未知字段或未知判别值必须失败关闭。`roadEditingSubjectKind` 冻结为
`0=ModuleHeader`、`1=RoadAlignment`、`2=Declaration`、`3=OwnerLocal`；地址、owner、
relation、occurrence、property step 与 canvas selection 的字段 tag、类型、必需性和
封闭判别值见附录 A，不依赖 Rust 枚举布局。

两个 ordinal 必须解析到 `SourceModules` 中的规范 module/document 记录；不能替代该记录
保存的稳定来源键。`Text` 保留受检 `SourceSpan` 的真实起止行列；`RoadEditing` 保留稳定
声明或 owner-local subject、闭合的 table field / struct member / union variant property
step，以及可选 interned canvas selection。FlatBuffers 结构损坏时使用的已证明输入内
byte range / physical vector fallback 只属于失败诊断；结构预检失败不会产生 LFCA/LFSM，
因此不得把该 fallback 编入成功候选的 LFSM。

模块必须按确定性 Kahn 拓扑序排列：每轮只把全部 import 已发射的模块放入 ready set，以完整
`authoringNamespaceId` UTF-8 bytes 严格递增选择下一项；命名空间必须全局唯一。发射后从
`0` 连续分配 `sourceModuleOrdinal`，循环或 unresolved import 失败关闭。每个模块内的文档按
完整 `sourceDocumentKey` UTF-8 bytes 严格递增，模块依次拼接后从 `0` 连续分配
`sourceDocumentOrdinal`。调用方加入顺序、拓扑遍历队列和 hash iteration 不得参与两个
ordinal。

发射器随后收集所有被 LFSM 行引用的位置，按完整位置语义值排序和去重，再从 `0` 连续分配
`sourceLocationOrdinal`；位置池禁止重复值和没有任何引用方的额外行。完整比较键先比较
`sourceLocationKind`、规范 module/document ordinal；`Text` 随后按 tag 5..8 的四个整数值
比较，`RoadEditing` 随后按 tag 9..21 比较。整数按无符号数值序，UTF-8/稳定 key 按无符号
bytes 字典序，optional 按缺失先于存在，`propertySteps` 按行序及每行三个整数的数值 tuple
字典序，`canvasSelection` 比较 actual UTF-8 bytes；缺失始终不等于空值。所有引用必须在
重编号后投影到该唯一池。RoadEditing context 的内部分配 ordinal、物理 vector index、byte
offset 和第一次访问顺序不得成为成功 LFSM 的规范顺序。

除 `SourceModule/SourceDocument/SourceLocation` 外，每张 LFSM 表只按附录 A.2 登记的完整
行键严格排序；来源模块/文档顺序只参与位置池语义键，不得作为
`StableEntitySource/OwnerLocalSource/DerivedRelationSource` 的隐含前缀。所有
`primary/contributing/sourceLocations` 向量都使用上述位置语义序，而不是未冻结的池插入
顺序。

`sourceMapDigest` 是完整 `LFSM` exact bytes 的 SHA-256；`sourceMapByteLength` 是相同
字节的精确 `u64` 长度。封套不嵌入自身摘要。来源位置或来源沿袭变化必须改变 LFSM
exact bytes，但在规范语义未变时不得改变 `NetworkRevisionId`。

## 6. 语义差异封套 `LFSD` v1

`magic = "LFSD"`，`semanticDiffFormatVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                          | 内容                                                       |
| ------------- | ----------------------------- | ---------------------------------------------------------- |
| `0x0001`      | `SemanticDiffBindings`        | base/genesis 与 target 的 revision、artifact digest/length |
| `0x0002`      | `EntityChanges`               | 稳定实体 add/remove 与规范字段语义变化                     |
| `0x0003`      | `RelationChanges`             | owner/member、topology reconnect、出现项和 localIndex 变化 |
| `0x0004`      | `GeometryChanges`             | 规范几何、长度和容差显著变化                               |
| `0x0005`      | `StaticRuleChanges`           | Gate/Waiting/Signal/Access 等行为变化                      |
| `0x0006`      | `SpatialConfigurationChanges` | headless/spatial presence 与闭合最终方向配置档变化         |

LFSD v1 的组合视图如下。该图只展开已由 §3.3 定义的公共前导、目录和变长节，不引入
第二种容器；第一节 wire offset 固定为 `0x00b0`（`32 + 6 * 24`）。

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

`baseBindingKind` 是封闭枚举：`0=Genesis`、`1=Artifact`。Genesis 必须把所有 base
版本、修订、digest 和 length 字节规范为零，并把目标的所有稳定实体、owner-local 序列和
LaneEdge/FacilityBand geometry 行报告为新增，同时生成一条空间配置初始化记录；`Artifact`
禁止任何零占位。target 永远必须是具体 artifact。

`networkRevisionDerivationVersion + networkRevision + canonicalArtifactDigest +
canonicalArtifactByteLength` 四元组称为修订—制品绑定（`RevisionArtifactBindingV1`）；
只比较其中任一子集都不是完整绑定。

`SemanticDiffBindings` 的唯一一行按下列顺序和精确类型绑定两端，字段不得省略或重排：

```text
baseBindingKind:u8
baseNetworkRevisionDerivationVersion:u16
baseNetworkRevision:Sha256
baseCanonicalArtifactDigest:Sha256
baseCanonicalArtifactByteLength:u64
targetNetworkRevisionDerivationVersion:u16
targetNetworkRevision:Sha256
targetCanonicalArtifactDigest:Sha256
targetCanonicalArtifactByteLength:u64
```

`Genesis` 时四个 base 值分别为 `0`、32 个零字节、32 个零字节、`0`；`Artifact` 时
derivation version 与 byte length 必须非零，两个 32-byte 值必须是实际绑定值而非零
占位。target derivation version 与 byte length 必须非零，两个 32-byte 值必须是实际
绑定值。结构预检先验证该闭合形状；#299 后发射检查对当前 target LFCA 重算
revision/digest/length，并把 LFSD target binding 与调用方从实际 `PortableDiffBase` 保存的
显式 base binding 逐项比较。它不加载或重验一份外部 base artifact，也不验证 change set
完备性。

`baseBindingKind=Artifact` 时，base 与 target 的 `ContractVersions` singleton 和
`ExecutionContract` singleton 必须分别逐字段相等；任一语义 contract/version 轴变化都以
`UnsupportedSemanticContractTransition` 拒绝，且不得产生 LFSD 候选。v1 不用空变化集或
任一实体 change class 隐藏无法表示的 contract 迁移；Genesis 没有 base，因此只要求
target 的两行是 v1 支持值。

通过上述 contract 兼容性检查后、执行任何 retained/change 分类前，Artifact diff 必须从
两端已经独立通过 A.1 的 `CanonicalIdentity` 重建以 `StableId128` 为键的全局身份表。每个
两端共有的 ID 必须具有相同 `entityKind` 和逐字节相同的完整
`identityCanonicalBytesV1`；否则以 `CrossRevisionStableIdCollision` 拒绝整个 base/target
对，且不得产生 LFSD 候选。该检查不能因实体 Row、关系或 geometry 投影相同而省略，也不能
把截断碰撞降级成空变化集、`Modify` 或 `Remove` + `Add`。

四张 entity-scoped change table 分别按附录 A.3 的每-kind 规范键严格排序；该键只使用该
kind 必需字段，不定义“缺失 optional 值如何排序”。完全相同的键失败关闭。重复关系值由
required before/after `localIndex` 进入相应键来破同值；全局空间配置由独立 singleton
change table 表达。相同 base/target 允许产生合法的空变化集合，但仍保留完整绑定。目标
共享静态路网内部 layout-only 变化不进入 LFSD，也不得
伪装成语义变化；这不包括已经进入 LFCA 语义的最终方向配置档。

LFSD v1 只表达能够从两份 artifact 独立重算的集合和字段差异。`StableId128` 只存在于
base 时必须表达为 `Entity Remove`，只存在于 target 时必须表达为 `Entity Add`；即使编译器
内部知道两项来自同一编辑对象，也不得在规范 bytes 中声明、猜测或按排序强配 old/new
对应。重命名提示属于非权威展示，真实跨修订状态连续性必须由未来受认证的 cutover 映射
显式授权；来源沿袭不能补充 LFSD 的 artifact 语义。

编译器可以从受结构/直接值域预检的 base view 生成诊断性差异。#299 只闭合上述显式
base/target binding，不重算或逐项验证 LFSD change set；如 #302 需要以 LFSD 授权切换，必须
在其自身设计中重新冻结更强的迁移输入与信任边界。LFSD 自身永远不授予迁移权限。

`semanticDiffDigest` 是完整 LFSD exact bytes 的 SHA-256，`semanticDiffByteLength` 是同一
字节序列的精确 `u64` 长度；LFSD 不嵌入自身摘要。候选关闭 LFSD 后还必须唯一派生
`semanticDiffObjectKey = "sha256/" || hexLower(semanticDiffDigest)`，总长 71-byte ASCII。
这三个值是 `PortablePublicationCandidate` 的计算绑定，不进入 LFCP；#302 必须逐值绑定它们，
不得从 base/target artifact binding 猜测 diff 对象或接受调用方自报 locator。

## 7. 规范发布描述符 `LFCP` v1（#298 历史实现；已由 ADR 0024 取代）

`magic = "LFCP"`，`canonicalPublicationDescriptorVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                       | 内容                                                                        |
| ------------- | -------------------------- | --------------------------------------------------------------------------- |
| `0x0001`      | `CanonicalArtifactBinding` | format/revision 版本、revision、artifact digest/length                      |
| `0x0002`      | `SourceMapBinding`         | source-map version、digest/length、compiler build、来源集合摘要             |
| `0x0003`      | `ValidationReceiptBinding` | receipt format、`canonical-publication-v1`、validator build、digest/length  |
| `0x0004`      | `PublicationProvenance`    | publisher kind/build、content-addressed object keys、受控时间/CI provenance |

LFCP v1 的组合视图如下。该图只展开已由 §3.3 定义的公共前导、目录和变长节，不引入
第二种容器；第一节 wire offset 固定为 `0x0080`（`32 + 4 * 24`）。

```text
    +---------------------------------------------------------------+
    |           LFCP ObjectPreambleV1 (32 bytes; SC == 4)           |
    +---------------------------------------------------------------+
    |             SectionDirectoryEntryV1[4] (96 bytes)             |
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
    :       ValidationReceiptBinding (0x0003; variable bytes)       :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :        PublicationProvenance (0x0004; variable bytes)         :
    :                                                               |
    +---------------------------------------------------------------+
```

描述符不得包含自身摘要或签名。真实性来自描述符 exact bytes 之外的签名 publication
manifest、宿主已认证 asset/package manifest 或 pinned digest。对象内自报的修订、摘要、
长度、compiler/validator build 或 provenance 都不能自证可信。

#298 冻结 receipt binding 的槽位和 fail-closed 版本行为；#299 冻结收据内部 wire/check
results 并签发 `canonical-publication-v1`。在 #299 收据存在前，编译器只能产生未受信
`PortablePublicationCandidate`，不能伪造 LFCP 中的空摘要、空收据或 trusted 标记。

LFSD 不进入 LFCP，因为 canonical publication 不授予迁移；LFSD 的 exact binding 只
进入 #302 的 `NetworkRevisionCutoverDescriptor`。同一次发射事务仍必须原子地产生
artifact、source map 和 genesis/base diff 候选，避免后续错误配对。

## 8. 编译与原子发布事务

### 8.1 唯一输入与候选结果

发射器只能同时借用同一个 `CompilationOutput` 的 LIR 和来源映射输入，并接收一份显式、
已规范化的 `PortableEmissionProvenanceV1`。后者是 exact-byte 确定性输入的一部分，不能由
发射器读取工作目录、目标平台或时钟临时拼装：

```text
    +----------------------------------+     +------------------------------+
    | CompilationOutput                |     | PortableEmissionProvenanceV1 |
    | ValidatedCanonicalLir            |     | canonical compilerBuildId    |
    | + ValidatedSourceMapInput        |     +---------------+--------------+
    +----------------+-----------------+                     |
                     | one atomic borrow                      | canonical input
                     +-------------------+--------------------+
                                         |
    +--------------------------+     +----v----------------------------------+
    | optional checked base    |---->| emitPortableCandidate(output,        |
    | artifact view            |     |   provenance, limits, base)          |
    +--------------------------+     +-------------------+-------------------+
                                                        |
                                        +---------------+---------------+
                                        |               |               |
                                        v               v               v
                                   +---------+      +---------+      +---------+
                                   |  LFCA   |      |  LFSM   |      |  LFSD   |
                                   |  bytes  |      |  bytes  |      |  bytes  |
                                   +----+----+      +----+----+      +----+----+
                                        |               |               |
                                        +---------------+---------------+
                                                        |
                                                        v
                                   +---------------------------------------+
                                   | close -> digest/length -> structural  |
                                   | preflight -> internal binding checks  |
                                   +-------------------+-------------------+
                                                       |
                                                       v
                                   +---------------------------------------+
                                   | PortablePublicationCandidate          |
                                   | bytes + digest/length/object keys     |
                                   +-------------------+-------------------+
                                                       |
                                                       v
                                   +---------------------------------------+
                                   | no-replace object installation        |
                                   | LFCA + LFSM + LFSD; durable winner    |
                                   +-------------------+-------------------+
                                                       |
                                                       +------------------------------------------+
                                                       |                                          |
                                                       v                                          v
                                   +---------------------------------------+  +---------------------------------------+
                                   | #299 independent LFCA/LFSM validation |  | LFSD installed diagnostic object      |
                                   | canonical-publication-v1 receipt      |  | NOT authenticated / NOT published     |
                                   +-------------------+-------------------+  | future #302 binding only              |
                                                       |                      +---------------------------------------+
                                                       v
                                   +---------------------------------------+
                                   | receipt close -> digest/length ->     |
                                   | preflight -> immutable installation   |
                                   +-------------------+-------------------+
                                                       |
                                                       v
                                   +---------------------------------------+
                                   | LFCP exact bytes + installed bindings |
                                   +-------------------+-------------------+
                                                       |
                                                       v
                                   +---------------------------------------+
                                   | authenticated manifest/pointer commit |
                                   |              PUBLISHED                |
                                   +---------------------------------------+

    any failure before the final commit
        -> discard staging references
        -> no LFCP/manifest commit
        -> no partial publication
```

调用方不能分别构造或重新配对 LIR/source-map input。一次确定性比较的完整规范输入是
`CompilationOutput + PortableEmissionProvenanceV1 + base binding`；`limits` 和 worker 数只
控制资源，不进入 bytes。相同完整规范输入在所有支持平台必须产生相同 bytes；显式改变
build provenance 可以改变 LFCA/LFSM/LFCP binding 和 artifact digest，但不得改变规范语义
未变时的 `NetworkRevisionId`。`laneflow-format` 提供写入器与结构视图；对私有编译器 LIR
的字段投影和差异生成继续在 `laneflow-compiler`。G2 API 不得暴露“写 artifact 成功、
source map 失败后仍可取出 artifact”的中间成功状态。

`PortablePublicationCandidate` 原子拥有 LFCA/LFSM/LFSD 三份 exact bytes，以及分别从这些
bytes 重算的 `Sha256Digest`、`ExactByteLength` 和 `sha256/<64 lowercase hex>` object key，
并以 `NetworkRevisionId` 暴露修订绑定；这些值只在线格式边界转换为原始 32-byte/`u64`。
同一强类型边界延续到内容寻址安装结果和 receipt subject projection；只有 LFCP 字段编码等
明确 wire writer 才解包为原始值。调用方
不能覆盖、遗漏或重新配对任一计算绑定；只有三份对象全部关闭、结构预检和内部 binding
检查成功后才能取得候选。LFSD binding 保留给 #302，不因 LFCP 当前不引用 diff 而丢弃。

### 8.2 发布提交点

发布协议不依赖普通覆盖式 rename 或边写边暴露最终 digest 路径恰好原子：

1. 调用方必须预配置并持久化目标发布根及其祖先目录；发布实现不得递归创建或接管该
   外部目录树，只在该根下创建、验证并持久化固定的对象与 staging 直接子目录，再在同一
   文件系统内创建唯一暂存目录；
2. 以受限写入器在唯一暂存文件中完成三个对象，对文件数据执行平台持久化 flush，关闭
   写入并从每份最终 exact bytes 计算 digest/length，再按上一节为 LFCA/LFSM/LFSD 分别派生
   唯一 content-addressed object key；
3. 重新执行结构预检和候选内部 binding 核对，并在安装前完成暂存文件长度/bytes 复核；
4. 使用同一文件系统、具有原子可见和 no-replace 保证的平台安装原语，把已经完成且关闭的
   三份暂存文件分别一次性安装到候选中刚重算的 `sha256/<lowercase-hex>` 逻辑键；LFCA/LFSM
   两个键还必须分别等于附录 A.4 的 `artifactObjectKey/sourceMapObjectKey`，LFSD 键必须等于
   §6 的 `semanticDiffObjectKey`。这些键只能安全映射到已配置发布根下。禁止调用方覆盖 key，
   也禁止先创建最终文件再复制、流式写入或截断。平台 adapter 无法证明该语义时必须返回
   `AtomicInstallUnsupported`，不得退化；
5. 安装原语报告目标已存在时，只能读取已经由同一协议完成安装的 winner，并核对精确长度
   和 bytes；相同则复用，不同则以 collision/mismatch 失败，始终不能覆盖。`Installed` 与
   `Reused` 必须经过同一对象目录耐久屏障后才能返回安装 capability；另一发布者已经让目录项
   可见不能替代本次调用在 manifest 提交前建立持久化顺序。平台只能证明原子可见而不能证明
   目录元数据持久化时必须返回 `AtomicInstallUnsupported`；
6. #299 独立验证成功后，把 receipt exact bytes 写入新的唯一暂存文件，执行与前三个对象
   相同的受限写入、持久化 flush、close、digest/length 重算和结构预检，再以附录 A.4 的
   `receiptObjectKey` 执行同样的 atomic + no-replace 安装、winner exact bytes 比较和目录
   元数据持久化；receipt 只在这一安装完成后才可被 LFCP 引用；
7. 只有已安装的 receipt winner 存在，且 LFCA/LFSM/receipt 的 digest、exact length 与三个
   object key 全部按附录 A.4 重算、解析并指向相同 winner 时，才构造 LFCP exact bytes，
   再由外部发布链认证；
8. 只有外部认证 manifest/指针的单次提交才使 LFCP 实际引用的 descriptor、artifact、source
   map 与 receipt 绑定变为“已发布”；暂存对象、LFSD 或其他未引用的 content-addressed
   objects 不构成发布，current committed capability 也不得把 LFSD 安装结果提升为受认证绑定；
9. 任一步失败都删除暂存引用，不产生 LFCP/manifest 提交点。已经存在的完整内容寻址共享对象
   可以保留，但不得被当作部分发布成功。

同一逻辑对象键不得覆盖不同 bytes。协议参与者不得覆盖、截断或原地修改最终对象；新
compiler provenance 即使语义相同也产生新的 artifact exact digest，但可以保持同一
`NetworkRevisionId`。

该协议不把本地安装器提升为操作系统安全边界。调用方必须独占管理并信任预配置发布根；
直接拥有该根文件系统写权限的进程、ACL、账户隔离、只读挂载和 WORM 策略属于部署责任，
不由 `laneflow-compiler` 配置或证明。消费者从磁盘、网络或宿主包取得对象后仍须根据认证
binding 重新计算摘要，不匹配时失败关闭。`LocalPortableObjectInstaller` 只服务本节冻结的
LaneFlow 发布事务，不提供任意 bytes 公共写入、对象枚举/删除、GC、远程 backend、压缩、
加密、配额或通用存储生命周期。

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

#298 把限制分成三层，不能让实现测量结果在 G2 反向定义格式契约：

1. 格式硬上限是 v1 固定的安全天花板，见下表；它来自固定宽字段可表达范围、每个对象
   的封闭节/表形状和已接受 `LF-COMP-P100-INITIAL-v2` 的资源维度，不是从 emitter
   benchmark 拟合出的性能结论；
2. 调用方运行上限必须显式提供，可以低于格式硬上限，但不得提高、禁用或用更大值绕过
   格式硬上限；默认发布路径从同次编译使用的 `CompileLimits` 推导更小预算，运行上限
   不改变 wire version；
3. 性能验收门槛与格式安全上限分别登记。G2 测量 production emitter 的成本并与既有
   产品预算比较；测量结果不能修改同一 wire version 的安全上限。

| v1 格式硬上限                            | 精确值                                  | 适用检查点                                    |
| ---------------------------------------- | --------------------------------------- | --------------------------------------------- |
| 单对象 exact bytes                       | `16,777,216` bytes                      | transport/hash/read 前                        |
| 单节或单表 exact bytes                   | `16,777,216` bytes                      | offset/length checked 后、建立 slice 前       |
| 单对象 TableV1 总数                      | LFCA `35`、LFSM `8`、LFSD `6`、LFCP `4` | 读取任一 TableV1 前；必须精确等于对象登记形状 |
| 单 TableV1 RowV1 数                      | `65,536`                                | `count * 16` 检查前                           |
| 单 RowV1 FieldV1 数                      | `17`                                    | `count * 12` 检查前；具体 row registry 更严格 |
| 单 Identity v1 `Ascii` value bytes       | `53`                                    | token 文法检查和 StableId 重算前              |
| 单 UTF-8 field bytes                     | `1,048,576` bytes                       | UTF-8 验证和分配前                            |
| 单对象全部 UTF-8 value 累计 bytes        | `8,388,608` bytes                       | checked 累加、驻留/复制前                     |
| 单向量 item 数                           | `65,536`                                | 内部 count 与 VBL 核对前                      |
| 单对象全部 vector value 累计 bytes       | `8,388,608` bytes                       | checked 累加、分配前                          |
| `RecordVector` 内嵌深度                  | `1`                                     | 读取 nested field type 前                     |
| 单 LFSM 来源位置记录数                   | `65,536`                                | 建立位置索引前                                |
| 单次 LFCA+LFSM+LFSD 候选暂存 exact bytes | `50,331,648` bytes（48 MiB）            | 开始写入前保留总预算；每次增长前 checked 累加 |

固定节数同时给出精确形状：LFCA `8`、LFSM `5`、LFSD `6`、LFCP `4`。Table 总数也是
按附录 A 求和得到的精确形状，不是可由未知表填满的通用容量。`17` 是通用 RowV1 parser
的安全天花板，并可由通用 TableV1 达到；按 A.2 已冻结的 OwnerLocal owner/relation 矩阵，
需要 address 的 relation owner 最大深度为二，因此同时携带完整 address、property 与 canvas
且通过直接值域检查的 SourceLocation 当前最多 `16` 个字段。第 17 个字段只会构成不匹配的
三层 owner address 并失败关闭；具体表始终必须满足自身 tag/presence/value matrix。一个对象中的
`rowsByteLength`、全部
`rowByteLength`、全部 `valueByteLength` 仍必须受外层 exact length 逐层约束；上表不是允许
padding、截断或只验证其中一份冗余计数的理由。

“安全天花板”与“合法对象可达到的最大值”必须分开记录。单对象、节/表 bytes 和候选暂存
等通用资源天花板是拒绝上界，并不承诺附录 A 中存在恰好等于每个数值的合法对象；尤其节/
表 bytes 还受对象前导、目录和其他必需节约束。G2 必须对每个天花板验证 `limit+1` 在其值
被用于分配/hash/view 前失败，并在独立 reachability 表中验证该维度的最大可构造合法值
被接受；只有 reachability 表证明 `limit` 本身可构造时才要求 boundary 接受。精确节数、
精确 Table 总数、每表登记字段形状则必须直接验证合法精确值接受，任意 `+1/-1` 失败。

这些安全天花板有意高于当前 `LF-COMP-P100-INITIAL-v2` 的 `38,112` 条 LIR 记录、
`2,782,758` bytes 逻辑输出、`991,537` bytes 累计字符串和 `22,368` 个几何点，同时
把恶意预分配限制在固定数量级。后继若合法产品输入需要更大值，必须提升对象或相关节的
格式版本并重新审阅，不能原地改常量；这是一项有界协议决策，不触发新的性能研究议题。

G2 必须验证每个冻结维度的 `limit+1`、最大可达合法值、P100、算术溢出、截断和失败
恢复；可达边界另验证 `limit`。若 G2 证据证明 G1 限制不可实现或安全边界不充分，必须
暂停实现并返回 G1 修订 constraints；不能把该发现登记为“G2 后冻结”，也不能自动扩展为
新的 benchmark/schema/tooling 工作流。

## 10. 确定性、known vectors 与验收矩阵

G1 只冻结少量可人工复核的向量及推导；G2 再把完整对象 materialize 为固定 fixture，
提交输入、完整 expected bytes、SHA-256、长度和修订 ID，不允许测试在运行时用
production emitter 自己生成 expected：

| 类别        | 向量/锚点 ID                 | 证明内容                                                                         |
| ----------- | ---------------------------- | -------------------------------------------------------------------------------- |
| G1 摘要向量 | `REV-V1-MIN-HEADLESS`        | 六个合法最小语义节的 framing、domain separation 与 SHA-256                       |
| G1 摘要向量 | `REV-V1-MIN-SPATIAL-EMPTY`   | 启用空间并写入闭合 direction profile code 时 revision 必须变化                   |
| G1 结构锚点 | `LFCA-V1-MIN-HEADLESS`       | 最小合法无空间 artifact 的前导、八节目录和首节 offset 推导                       |
| G1 结构锚点 | `LFSM-V1-MIXED-LOCATION`     | Text、无 property 的 Declaration、OwnerLocal 和可选 canvas 形状                  |
| G1 结构锚点 | `LFSD-V1-GENESIS-BINDING`    | Genesis 四个 base 零值和完整 target binding                                      |
| G1 结构锚点 | `LFCP-V1-MIN-BINDINGS`       | artifact/source-map/receipt 的外部 digest+exact length 绑定                      |
| G2 固定对象 | `LFCA-V1-FULL-SPATIAL`       | 22 种实体、关系、规则、规范 f32/f64 与空间表                                     |
| G2 固定对象 | `LFCA-V1-PROVENANCE-ONLY`    | 同语义不同来源沿袭：revision 相同，artifact digest 不同                          |
| G2 固定对象 | `LFCA-V1-CLAIM-MISMATCH`     | 只篡改非语义 revision claim：结构预检成功、独立 revision 比较失败                |
| G2 固定对象 | `LFCA-V1-REORDER-EQUIVALENT` | 声明/集合/hash iteration 重排仍产生完全相同 bytes                                |
| G2 固定对象 | `LFCA-V1-SIGNED-ZERO`        | 合法输入 `-0.0` 在编译边界变为 `+0.0`；负零 wire 被读取器拒绝                    |
| G2 固定对象 | `LFSD-V1-CHANGE-SET`         | add/remove/reconnect/geometry/global spatial/rule；身份变化只含无配对 remove/add |
| G2 固定对象 | `LFSD-V1-NOOP`               | 相同 base/target 的空记录但完整 binding                                          |

两个 G1 revision 向量使用 §4.2 的 exact framing 和附录 A 的 section/table/row/field
编码。`REV-V1-MIN-HEADLESS` 的六节依次是：所有版本值为 `1` 的 ContractVersions；
空 CanonicalIdentity；22 张空实体表；5 张空关系表；`spatialPresent=0`、direction profile
code 为 `0` 且两张空几何表；两个版本值为 `1` 的 ExecutionContract。各节 exact length 与
SHA-256 为：

| sectionKind | exact bytes | section SHA-256                                                    |
| ----------- | ----------- | ------------------------------------------------------------------ |
| `0x0001`    | `120`       | `8682b46d765cdc7cf4e880dbf1dcd8d046d6ca82990d57cf3abc2a3568220869` |
| `0x0002`    | `20`        | `3a85cd4b4d295cdd6cfe6ea3cb119b7c59f1addcc36faf58c33809f958191c7e` |
| `0x0003`    | `356`       | `54975e3435099f8ac2f6b6ec53e3bf68104d236da4a840318e9d0486a46e0f6e` |
| `0x0004`    | `84`        | `041fb436600f0bd293d9a9a78bb1367144e03e51ecfafca712bbc4dedb67dc19` |
| `0x0005`    | `94`        | `1ac4a913965b92e3dec446f935384fc18f6947038140c2feb3b474cc854dc5ed` |
| `0x0006`    | `64`        | `79e8acf6943d876fd8ee1f45f6856c3b8285562f0c30e4d9de559317316f025f` |

加入六个 12-byte section frame 后，semantic payload 为 `810` bytes；复核算法精确为
`SHA-256("laneflow.network-revision.v1\0" || payload)`，字符串末尾包含一个 NUL byte，
得到 `4b61b28fca27bdecd0397f826cfae1ada0b2ea375b725ddc84ecd668960c1c89`。

`REV-V1-MIN-SPATIAL-EMPTY` 保持两张几何表为空，但把 semantic payload 零基 offset
`688` 的 `spatialPresent` 从 `00` 改为 `01`，并把 offset `701` 的
`geometryDirectionProfile` 从 `00` 改为 `02`（Balanced2Deg）。第 `0x0005` 节 SHA-256
变为 `8e8cceaa34daadc3b176aa5cca44c639c0edd9f884d65099830dab9014d06f76`，payload 仍为
`810` bytes，NetworkRevisionId 变为
`bc30ae4a4551ee9987a165ef8fa74bfc1bddf39d333fd5038c6bcadcc1b59f9b`。来源近似 accuracy
profile 只进入非语义 CompilerProvenance，不进入这两个 revision 向量。该向量由
`hasProfile=true` 单独使四项逻辑或为真，明确覆盖 profile-only spatial 状态。

`LFCA-V1-MIN-HEADLESS` 的公共布局人工锚点为：前导 `32` bytes，
八项目录 `192` bytes，第一节 offset `0x00e0`；LFSM/LFSD 的对应锚点分别为
`0x0098`、`0x00b0`。当前 LFCP v2 的锚点为 `0x0068`；历史 LFCP v1 锚点为 `0x0080`。
完整对象 fixture 在 G2 从附录 A materialize 后固定，
不把整段二进制十六进制复制进规范正文。

确定性矩阵至少覆盖：

- Windows `x86_64-pc-windows-msvc` 本机与 Ubuntu `x86_64-unknown-linux-gnu` CI；
- single-thread 和编译器支持的所有 worker 数；
- clean process、重复运行、不同 hash seed/分配地址；
- production emitter 与 #299 后发射检查；G1 人工推导值作为不调用 emitter 的固定期望；
- 截断每个边界、单 bit 损坏、未知版本/节/表/字段、重复/乱序、gap/overlap、超限、
  length/digest/revision/source-map/base-target 错配；
- 暂存写入、flush/close、对象安装和 manifest 提交各失败点的无部分发布测试；
- 最小、P100 正式最高级和代表性大输入的发射时延、峰值暂存内存与输出大小；格式硬
  上限边界只属于安全/正确性矩阵，不作为性能 workload。

上述条目按风险职责组合证据，不形成所有轴的自动笛卡尔积。跨平台 exact-byte 通道固定
发射 `min headless` 与 `full spatial`，覆盖没有/具有空间语义的两个生产分支，并在 Windows、
Ubuntu 和两个 fresh process 之间比较三对象 exact bytes 与全部计算绑定。P100 正式最高级
继续只在具名 P100 参考机证明规模、时延和资源成本；格式 hard-limit/reachability 继续由
安全/正确性测试在支持 OS 上证明接受与失败点。后继 workload 必须先分类为新语义分支、规模
证据或安全边界；只有新语义分支默认扩展跨平台 exact-byte 集合，不能因新增一个规模或边界
样本而无序扩张整个矩阵。该职责分配由 [#298 G1 窄纠正][g1-evidence-lanes-correction] 接受。

完整计划和 G2 证据占位见
`../reference/v0.10-portable-artifact-validation.md`。

## 11. G1 再次重开内容闭合与重新接受结果

本文按下列条件闭合 G1 实现输入；动态 Gate、精确提交、外部审阅和 Project 状态仍以
#298 Gate Ledger 为准：

- [x] 附录 A 登记全部 artifact/source-map/diff/descriptor table kind、field tag、类型、
      必需性、排序键与 closed enum；
- [x] 冻结四类对象的 magic、版本、节集合、目录、数值/文本/浮点编码和错误分类；
- [x] 冻结 NetworkRevisionId exact payload、非语义 `ArtifactClaims` 比较规则和至少两个
      可人工复核的已知摘要；
- [x] 冻结 Text/RoadEditing 来源位置的判别值、字段 registry、规范排序和混合来源结构
      审阅锚点；
- [x] 冻结 artifact/source-map/receipt 与 base/target 的摘要+精确长度绑定；
- [x] 冻结 `CompilationOutput + PortableEmissionProvenanceV1` 完整输入、候选对象不可拆分
      成功和发布提交点；
- [x] 冻结 pre-hash 上限、结构计数上限、硬格式上限和失败原子性；
- [x] 由非作者审阅者仅依据本文人工重建两个 revision 向量和最小对象关键 offset；
- [x] 记录编码库/自有格式选择的安全与维护证据；
- [x] 冻结混合 RoadEditing/Synthetic 编译的逐 geometry direction profile 适用标记、连接端点
      OR 规则、LFSM range binding 与 LFSD GeometryChange 投影；
- [x] #298 Gate Ledger 已绑定本次再次重开的语义最终提交并取得职责中立的 exact-head clean
      review，且 Project/Issue 元数据完整。

此前的 G1 Pass 都只保留为历史记录，不能覆盖本次逐 geometry direction profile 适用性语义
变化。#298 Gate Ledger 已在上述新事实成立后重新记录正式 G1 判断；新的 G1 Pass 仍只冻结
实现输入，不声明 LFCA/LFSM/LFSD/LFCP 已实现、通过产品验证或获得独立语义授信。
G2 开工前仍须重新核验 GitHub 元数据、原生依赖、Accepted 文档和实现切片，并在 #298 追加独立
`## G2 开工判断`；任何字段 tag、限制值、排序破同值、publication 原子性或职责边界变化都必须
返回 G1。

## 附录 A：线格式登记表（规范）

本附录是 v1 table/field registry 的 Markdown 单一事实源。记法
`tag:name:type:presence` 中 `presence` 为 `R`（必需）或 `O`（可选且缺失即 `None`）；
未列出的 tag 不合法。每个表按“行键”严格递增，重复键失败关闭。`OrdinalVectorU32`
保持领域顺序时不得排序；标为集合时按数值严格递增且不得重复。所有 ordinal 必须解析到
同一对象相应有类型表。稳定实体的 ordinal 不仅必须从零连续，还必须按下述完整 Identity
v1 前像 bytes 排序得到；一致重编号全部引用不能把非规范顺序变成合法编码。

本附录定义的是 LFCA/LFSM/LFSD 可从对象自身独立重算的规范接受域，不是“某一 frontend
是否能生成该对象”的来源可达性证明。只有会改变线格式唯一性、目标无关静态语义或安全消费
前置条件，且能从本对象闭合字段重算的规则进入本 registry；只存在于某种来源声明、诊断
spelling、调用方 CompileLimits 或 frontend 能力中的准入规则继续由 compiler 拥有，不得暗中
继承为 wire 规则。官方 emitter 仍只接受 `CompilationOutput`。#298 曾把下文全部跨表语义
规则设想为独立 validator 的接受职责；Accepted ADR 0024 已取消该交付。#299 G2 只实现设计文档
明确列出的 revision/digest/length 与 LFSM/LFSD binding 闭合，不执行下文完整身份、拓扑、几何、
规则或 diff 双射复验。未来若要接收不受信任 LFCA/LFSM/LFSD 并提升为完整语义能力，必须另行
设计、回到 G1，不能把这些历史条款自动计入当前 checker 或发布前置条件。

### A.1 LFCA table registry

`ContractVersions(0x0001)` 节只有 `ContractVersions(0x0001)` 一行，行键为 singleton：

```text
1:canonicalFormatVersion:u16:R
2:identityEncodingVersion:u16:R
3:identityRegistryRevision:u16:R
4:networkRevisionDerivationVersion:u16:R
5:constraintContractVersion:u16:R
6:staticExecutionContractVersion:u16:R
```

`CanonicalIdentityTable(0x0002)` 节只有 `CanonicalIdentity(0x0001)`。行键为
`(entityKind, typedOrdinal)`：

```text
1:entityKind:u16:R
2:typedOrdinal:u32:R
3:stableId:StableId128:R
4:identityFields:RecordVector:R
```

`identityFields` 按 Identity v1 registry 顺序保存非空内嵌行
`1:identityFieldTag:u16:R, 2:value:Bytes:R`；不得排序或去重。`stableId` 必须等于该完整
Identity v1 前像按下式带域分离计算的 BLAKE3 摘要前 16 bytes：

```text
identityCanonicalBytesV1 :=
  "LFID"
  || identityEncodingVersion:u16 (=1)
  || entityKind:u16
  || identityFieldCount:u16
  || for each identity field in required-tag order:
       identityFieldTag:u16
       valueByteLength:u32
       valueBytes[valueByteLength]

StableId128 := first-16-bytes(
  BLAKE3("laneflow.stable-id.v1\0" || identityCanonicalBytesV1)
)
```

全部整数为小端。对每个 `entityKind`，发射器必须按完整
`identityCanonicalBytesV1` 的无符号逐字节字典序严格排序，再从 `0` 连续分配
`typedOrdinal`。`CanonicalIdentity` 行、对应实体表行与所有 ordinal 引用必须对
`(entityKind, typedOrdinal, stableId)` 一一对应。#298 历史完整接受域要求重建前像、重算 ID、
重排并核对该映射；该要求不属于 #299 当前后发射检查。

`identityFieldTag` 的闭合代码、名称与 value 编码为：

```text
 1 authoringNamespaceId Ascii        2 corridorKey Ascii
 3 sectionKey Ascii                  4 laneKey Ascii
 5 laneEdgeKey Ascii                 6 junctionKey Ascii
 7 pathKey Ascii                     8 movementKey Ascii
 9 directedEntryApproachKey Ascii   10 directedExitApproachKey Ascii
11 movementStableId StableId128     12 entryEdgeStableId StableId128
13 exitEdgeStableId StableId128     14 maneuverPathStableId StableId128
15 gateKey Ascii                    16 waitingZoneKey Ascii
17 stopLineKey Ascii                18 signalGroupKey Ascii
19 signalControllerKey Ascii        20 signalControllerStableId StableId128
21 phaseKey Ascii                   22 parkingAreaKey Ascii
23 RESERVED                         24 parkingSpaceKey Ascii
25 laneGroupKey Ascii               26 facilityBandKey Ascii
27 participantClassKey Ascii        28 accessRuleKey Ascii
29 vehicleProfileKey Ascii          30 routeKey Ascii
31 canonicalFrameKey Ascii          32 roadSectionStableId StableId128
33 roadCorridorStableId StableId128 34 junctionStableId StableId128
```

`Ascii` value 必须是 Identity v1 已准入的原始 ASCII bytes，不含长度，其 portable v1
接受域精确为 `1..=53` bytes、首 byte 属于 `[A-Za-z0-9]`、其余 byte 只属于
`[A-Za-z0-9._:/-]`。空值、控制字符、非 ASCII、标点开头、未登记标点或 54 bytes 及以上
一律在 StableId 重算前失败；读取器不得从调用方 limits 选择另一套 token 文法或放宽该
格式级上限。`StableId128` 必须恰为 16 bytes。`entityKind 1..22` 与下列实体表同序；每种
实体要求的 tag 序列精确为：

```text
RoadCorridor [1,2]                    RoadSection [1,3,33]
AuthoringLane [1,4,32]                LaneEdge [1,5]
Junction [1,6]                        Movement [1,8,9,10,34]
ManeuverPath [1,7,11,12,13]           ManeuverGate [1,14,15]
WaitingZone [1,14,16]                 StopLine [1,17]
SignalGroup [1,18]                    SignalController [1,19]
SignalPhase [1,20,21]                 ParkingArea [1,22]
ParkingSpace [1,24]                   LaneGroup [1,25,32]
FacilityBand [1,26,33]                ParticipantClass [1,27]
AccessRule [1,28]                     VehicleProfile [1,29]
StaticRoute [1,30]                    CanonicalFrame [1,31]
```

#298 历史完整接受域还要求在重算全部前像后，为 22 种实体的联合域建立一张全局
`StableId128` 唯一性表；该全量检查不属于 #299 当前后发射检查。
任何相同 `StableId128` 的第二行都失败关闭：完整前像相同表示重复实体，完整前像不同表示
BLAKE3-128 截断碰撞；二者都不得继续建立 ordinal、关系或辅助对象引用。该检查与逐行摘要
重算同属身份验证前置条件，不能因引用中另有 `entityKind` 而省略。

`CanonicalEntityTables(0x0003)` 精确包含下列 22 张表；即使无行也必须存在。每行共同以
`1:typedOrdinal:u32:R, 2:stableId:StableId128:R` 开始，余下字段如下：

| tableKind | 表名             | 字段（从 tag 3 开始）                                                                                                                                                                                                                                                                               | 行键           |
| --------- | ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------- |
| `0x0001`  | RoadCorridor     | `3:referenceSection:u32:R, 4:elements:RecordVector:R`                                                                                                                                                                                                                                               | `typedOrdinal` |
| `0x0002`  | RoadSection      | `3:roadCorridor:u32:R, 4:kindId:Utf8:R, 5:lanes:OrdinalVectorU32:R`                                                                                                                                                                                                                                 | `typedOrdinal` |
| `0x0003`  | AuthoringLane    | `3:roadSection:u32:R, 4:edgeChain:OrdinalVectorU32:R, 5:laneGroup:u32:O`                                                                                                                                                                                                                            | `typedOrdinal` |
| `0x0004`  | LaneEdge         | `3:lengthMeters:f64:R, 4:speedLimitMetersPerSecond:f64:R, 5:successors:OrdinalVectorU32:R`                                                                                                                                                                                                          | `typedOrdinal` |
| `0x0005`  | Junction         | `3:movements:OrdinalVectorU32:R`                                                                                                                                                                                                                                                                    | `typedOrdinal` |
| `0x0006`  | Movement         | `3:junction:u32:R, 4:directedEntryApproachKey:Utf8:R, 5:directedExitApproachKey:Utf8:R, 6:maneuverPaths:OrdinalVectorU32:R`                                                                                                                                                                         | `typedOrdinal` |
| `0x0007`  | ManeuverPath     | `3:movement:u32:R, 4:edges:OrdinalVectorU32:R, 5:maneuverGates:OrdinalVectorU32:R, 6:waitingZones:OrdinalVectorU32:R`                                                                                                                                                                               | `typedOrdinal` |
| `0x0008`  | ManeuverGate     | `3:maneuverPath:u32:R, 4:transitionIndex:u32:R, 5:stopLine:u32:R, 6:signalControlKind:u8:R, 7:signalGroup:u32:O`                                                                                                                                                                                    | `typedOrdinal` |
| `0x0009`  | WaitingZone      | `3:maneuverPath:u32:R, 4:entryGate:u32:R, 5:releaseGate:u32:R, 6:maxOccupancy:u32:R`                                                                                                                                                                                                                | `typedOrdinal` |
| `0x000a`  | StopLine         | `3:laneEdge:u32:R, 4:maneuverGates:OrdinalVectorU32:R`                                                                                                                                                                                                                                              | `typedOrdinal` |
| `0x000b`  | SignalGroup      | `3:controller:u32:R, 4:maneuverGates:OrdinalVectorU32:R`                                                                                                                                                                                                                                            | `typedOrdinal` |
| `0x000c`  | SignalController | `3:offsetMs:u64:R, 4:cycleDurationMs:u64:R, 5:signalGroups:OrdinalVectorU32:R, 6:phases:OrdinalVectorU32:R`                                                                                                                                                                                         | `typedOrdinal` |
| `0x000d`  | SignalPhase      | `3:controller:u32:R, 4:durationMs:u64:R, 5:states:RecordVector:R`                                                                                                                                                                                                                                   | `typedOrdinal` |
| `0x000e`  | ParkingArea      | `3:parkingSpaces:OrdinalVectorU32:R`                                                                                                                                                                                                                                                                | `typedOrdinal` |
| `0x000f`  | ParkingSpace     | `3:parkingArea:u32:O, 4:entryLaneEdge:u32:R, 5:entryProgressMeters:f64:R, 6:exitLaneEdge:u32:R, 7:exitProgressMeters:f64:R, 8:lateralOffsetMeters:f64:R, 9:headingOffsetRadians:f64:R, 10:lengthMeters:f64:R, 11:widthMeters:f64:R`                                                                 | `typedOrdinal` |
| `0x0010`  | LaneGroup        | `3:roadSection:u32:R, 4:members:OrdinalVectorU32:R`                                                                                                                                                                                                                                                 | `typedOrdinal` |
| `0x0011`  | FacilityBand     | `3:roadCorridor:u32:R, 4:kindId:Utf8:R`                                                                                                                                                                                                                                                             | `typedOrdinal` |
| `0x0012`  | ParticipantClass | `3:parent:u32:O, 4:depth:u32:R, 5:subtreeEnter:u32:R, 6:subtreeExit:u32:R`                                                                                                                                                                                                                          | `typedOrdinal` |
| `0x0013`  | AccessRule       | `3:targetKind:u8:R, 4:targetOrdinal:u32:R, 5:effect:u8:R, 6:participantClasses:OrdinalVectorU32:R, 7:regulation:RecordVector:O, 8:priority:i32:R`                                                                                                                                                   | `typedOrdinal` |
| `0x0014`  | VehicleProfile   | `3:participantClass:u32:R, 4:lengthMeters:f64:R, 5:desiredSpeedMetersPerSecond:f64:R, 6:minGapMeters:f64:R, 7:timeHeadwaySeconds:f64:R, 8:maxAccelerationMetersPerSecondSquared:f64:R, 9:comfortableDecelerationMetersPerSecondSquared:f64:R, 10:emergencyDecelerationMetersPerSecondSquared:f64:R` | `typedOrdinal` |
| `0x0015`  | StaticRoute      | `3:edges:OrdinalVectorU32:R, 4:transitionGates:RecordVector:R`                                                                                                                                                                                                                                      | `typedOrdinal` |
| `0x0016`  | CanonicalFrame   | 无额外字段                                                                                                                                                                                                                                                                                          | `typedOrdinal` |

Identity 前像中重复表达规范所有权或边界语义的字段必须与实体行严格等值，不能只各自解析成功：

| entity        | Identity field                               | 必须等于的 LFCA 语义投影                   |
| ------------- | -------------------------------------------- | ------------------------------------------ |
| RoadSection   | `33 roadCorridorStableId`                    | tag 3 `roadCorridor` 解析出的 StableId     |
| AuthoringLane | `32 roadSectionStableId`                     | tag 3 `roadSection` 解析出的 StableId      |
| Movement      | `34 junctionStableId`                        | tag 3 `junction` 解析出的 StableId         |
| Movement      | `9/10 directedEntry/ExitApproachKey`         | tag 4/5 的 exact ASCII bytes               |
| ManeuverPath  | `11 movementStableId`                        | tag 3 `movement` 解析出的 StableId         |
| ManeuverPath  | `12 entryEdgeStableId / 13 exitEdgeStableId` | tag 4 `edges` 第一项/末项解析出的 StableId |
| ManeuverGate  | `14 maneuverPathStableId`                    | tag 3 `maneuverPath` 解析出的 StableId     |
| WaitingZone   | `14 maneuverPathStableId`                    | tag 3 `maneuverPath` 解析出的 StableId     |
| SignalPhase   | `20 signalControllerStableId`                | tag 3 `controller` 解析出的 StableId       |
| LaneGroup     | `32 roadSectionStableId`                     | tag 3 `roadSection` 解析出的 StableId      |
| FacilityBand  | `33 roadCorridorStableId`                    | tag 3 `roadCorridor` 解析出的 StableId     |

表中任一不等都按 identity binding mismatch 在建立关系视图前失败；不得保留旧前像却把实体迁到
另一 owner。其余 `authoringNamespaceId/*Key` 只属于 Identity 前像，本版没有未登记的实体字段
副本；未来新增前像到实体投影必须先扩展本表。

每个 `OrdinalVectorU32` 字段的顺序、重复和空值策略不是实现者选择：

| 实体字段                        | 语义顺序                                             | 重复       | 空值             |
| ------------------------------- | ---------------------------------------------------- | ---------- | ---------------- |
| `RoadSection.lanes`             | 领域顺序；区段内已验证横向车道序                     | 禁止       | 禁止             |
| `AuthoringLane.edgeChain`       | 领域顺序；覆盖链行驶方向                             | 禁止       | 禁止             |
| `LaneEdge.successors`           | 集合；按 target typed ordinal 严格递增               | 禁止       | 允许             |
| `Junction.movements`            | 集合；按 member typed ordinal 严格递增               | 禁止       | 禁止             |
| `Movement.maneuverPaths`        | 集合；按 member typed ordinal 严格递增               | 禁止       | 禁止             |
| `ManeuverPath.edges`            | 领域顺序；完整 entry、internal、exit occurrence 序列 | 允许重复项 | 禁止，且至少两项 |
| `ManeuverPath.maneuverGates`    | 领域顺序；按 `transitionIndex` 严格递增              | 禁止       | 允许             |
| `ManeuverPath.waitingZones`     | 领域顺序；按 entry/release transition tuple 严格递增 | 禁止       | 允许             |
| `StopLine.maneuverGates`        | 集合；按 member typed ordinal 严格递增               | 禁止       | 禁止             |
| `SignalGroup.maneuverGates`     | 集合；按 member typed ordinal 严格递增               | 禁止       | 禁止             |
| `SignalController.signalGroups` | 集合；按 member typed ordinal 严格递增               | 禁止       | 禁止             |
| `SignalController.phases`       | 领域顺序；固定时制程序顺序                           | 禁止       | 禁止             |
| `ParkingArea.parkingSpaces`     | 集合；按 member typed ordinal 严格递增               | 禁止       | 禁止             |
| `LaneGroup.members`             | 领域顺序；保持所属 `RoadSection.lanes` 中的相对顺序  | 禁止       | 禁止             |
| `AccessRule.participantClasses` | 集合；按 member typed ordinal 严格递增               | 禁止       | 禁止             |
| `StaticRoute.edges`             | 领域顺序；路线出现序                                 | 允许重复项 | 禁止             |

领域顺序向量不得为了得到较小 ordinal 而重排；集合向量不得保留声明顺序。下列跨表关系是
#298 历史完整接受域对重复 ownership 的闭合规则，不是 #299 当前 checker 的逐字段复验项：

| owner 侧                        | member/back-reference 侧                         | 必需不变量                                                                              |
| ------------------------------- | ------------------------------------------------ | --------------------------------------------------------------------------------------- |
| `RoadCorridor.elements`         | `RoadSection/FacilityBand.roadCorridor`          | 每个 child 恰出现一次且指回同一 corridor；elements 非空、领域有序、无重复               |
| `RoadCorridor.referenceSection` | 同一 `RoadCorridor.elements` 的 `RoadSection` 项 | reference 必须恰为本 corridor 的一个 section member                                     |
| `RoadSection.lanes`             | `AuthoringLane.roadSection`                      | 双向全集相等，每个 lane 恰有一个 section owner                                          |
| `LaneGroup.members`             | `AuthoringLane.laneGroup`                        | optional back-reference 存在当且仅当 lane 在一个 group；group/lane 必须属于同一 section |
| `Junction.movements`            | `Movement.junction`                              | 双向全集相等，每个 movement 恰有一个 junction owner                                     |
| `Movement.maneuverPaths`        | `ManeuverPath.movement`                          | 双向全集相等，每个 path 恰有一个 movement owner                                         |
| `ManeuverPath.maneuverGates`    | `ManeuverGate.maneuverPath/transitionIndex`      | 双向全集相等；vector localIndex 顺序与严格递增 transitionIndex 一致                     |
| `ManeuverPath.waitingZones`     | `WaitingZone.maneuverPath`                       | 双向全集相等；每个 waiting zone 恰有一个 path owner                                     |
| `StopLine.maneuverGates`        | `ManeuverGate.stopLine`                          | 双向全集相等，每个 gate 恰引用一个 stop line                                            |
| `SignalController.signalGroups` | `SignalGroup.controller`                         | 双向全集相等，每个 group 恰有一个 controller                                            |
| `SignalController.phases`       | `SignalPhase.controller`                         | 双向全集相等，每个 phase 恰有一个 controller                                            |
| `SignalGroup.maneuverGates`     | `ManeuverGate.signalControlKind/signalGroup`     | `Group` 时 gate 恰在该 group；`None` 时不得出现在任一 group                             |
| `ParkingArea.parkingSpaces`     | `ParkingSpace.parkingArea`                       | optional back-reference 存在当且仅当 space 在一个 area，且不得出现在第二个 area         |

任一只验证一侧、遗漏 child、额外 member、owner 不一致或 optional back-reference 与 membership
不一致都失败关闭。`SignalPhase.states` 作为集合按 `signalGroup` ordinal 严格递增、不得重复，
并必须逐项覆盖其 controller 的全部 `signalGroups`；`StaticRoute.transitionGates` 保持路线
transition 领域顺序，行数精确为 `max(edges.count-1, 0)`。这些 RecordVector 约束与上表同样
属于独立语义验证，不由结构预检猜测。

内嵌记录精确为：`RoadCorridor.elements = 1:elementKind:u8:R, 2:ordinal:u32:R`，其中
`0=RoadSection, 1=FacilityBand`；`SignalPhase.states = 1:signalGroup:u32:R,
2:aspect:u8:R`；`AccessRule.regulation` 必须恰有一行
`1:jurisdiction:Utf8:R, 2:version:Utf8:R, 3:source:Utf8:O`；
`StaticRoute.transitionGates` 的行数必须为 `max(edges.count-1, 0)`，每行只有
`1:maneuverGate:u32:O`。`signalControlKind` 为 `0=None, 1=Group`，且 tag 7 当且仅当值为
`1` 时存在。`targetKind` 为 `0=LaneEdge, 1=LaneGroup, 2=RoadSection, 3=ManeuverPath`；
`effect` 为 `0=Deny, 1=Allow`。`aspect` 为 `0=Red, 1=Yellow, 2=Green`，未知代码失败
关闭。

`RoadSection.kindId` 与 `FacilityBand.kindId` 先满足非空 ASCII token：首字节为字母或数字，
后续只允许字母、数字、`.`、`_`、`:`、`/`、`-`。RoadSection 只允许 `motorLane`、`nonMotorLane` 或
`x-lane-` 加非空后缀；FacilityBand 只允许 `sidewalk`、`median`、`plantingStrip`、
`facilityStrip`、`shoulder` 或 `x-` 加非空后缀，但 `x-lane-` 前缀仍属于 RoadSection，不能
回退为 FacilityBand。该分类是 closed structural category，不授予未登记的交通能力。

下列规范标量必须在通用 finite/正零检查之外逐字段验证：

| 对象/字段                                                                 | 闭合约束                                                          |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `LaneEdge.lengthMeters`                                                   | `> 1.0e-9 m`                                                      |
| `LaneEdge.speedLimitMetersPerSecond`                                      | `> 0 m/s`                                                         |
| `WaitingZone.maxOccupancy`                                                | `> 0`                                                             |
| `ParkingSpace.entry/exitProgressMeters`                                   | 对所引 edge length `L`，`1.0e-9 < progress < L - 1.0e-9`          |
| `ParkingSpace.lateralOffsetMeters`                                        | `abs(value) > 1.0e-9 m`                                           |
| `ParkingSpace.headingOffsetRadians`                                       | `-π <= value < π`；`π` 的 binary64 bits 为 `0x400921fb54442d18`   |
| `ParkingSpace.lengthMeters/widthMeters`                                   | 各自 `> 1.0e-9 m`                                                 |
| `VehicleProfile.lengthMeters`                                             | `> 1.0e-9 m`                                                      |
| `VehicleProfile.desiredSpeedMetersPerSecond`                              | `> 0 m/s`                                                         |
| `VehicleProfile.minGapMeters`                                             | `>= 0 m`                                                          |
| `VehicleProfile.timeHeadwaySeconds` 与三项 acceleration/deceleration 标量 | 各自 `> 0`；且 `emergencyDeceleration >= comfortableDeceleration` |

ManeuverPath 的完整 edge StableId occurrence 序列必须全局唯一；同一
edge 可以在一个序列中重复出现，每个位置分别参与 transition、gate、diff 与来源映射。首末
occurrence 是 boundary，中间 occurrence 的去重集合必须与 `JunctionInternalEdge` 精确闭合。
boundary edge 不得同时是任何 internal edge；同一 internal edge 可在一条 path 中重复、或被
同一路口的多条 path 共享，但不得归属不同路口。记全部中间 occurrence 的去重集合为 `I`。

AuthoringLane 的每条 LaneEdge 最多属于一个 `edgeChain`。链内每对相邻 edge 若两端都不在
`I`，前项的规范 `successors` 必须显式包含后项；若任一端在 `I`，该有向 pair 必须作为相邻
occurrence 出现在至少一条 ManeuverPath 中，且涉及的 internal edge 必须归属该 path 的同一
路口。此时 edgeChain 只表达有序覆盖，ManeuverPath 仍是 internal transition 的 portable
拓扑权威。

LFCA 对 `LaneEdge.successors` 使用单一、source-model-neutral 的规范投影。发射器先从所有
ManeuverPath 的中间 occurrence 重建并核对 `I`，再把 validated LIR successor
集合中 `owner in I || target in I` 的 pair 全部过滤；剩余 pair 仍按 target typed ordinal 严格
递增写入。该 total projection 不读取 LFSM/source language，不因 Synthetic 或 RoadEditing
分叉，也不接受调用方 override。path 内相邻 edge 若都不在 `I`，过滤后的 LFCA successor 必须
仍包含该 pair；只要任一端在 `I`，完整 ManeuverPath occurrence 序列就是 portable 拓扑权威。
#298 历史完整接受域要求从 `JunctionInternalEdge` 重建 `I`，并拒绝 wire 中任一 owner 或
target 落入 `I` 的 successor；#299 当前 checker 不执行该拓扑重建。这样当前 Synthetic LIR 的显式 internal connectivity 可以无损归一为与 RoadEditing
一致的 path-only portable 语义，不会形成第二套 portable 拓扑权威或来源相关 revision。

每个 ManeuverGate 必须满足
`transitionIndex < maneuverPath.edges.count - 1`，其 StopLine 的 `laneEdge` 必须逐值等于
`maneuverPath.edges[transitionIndex]`；同一 path 的 transitionIndex 不得重复。每条 LaneEdge
最多有一条 StopLine；每条 StopLine 必须至少关联一个 gate，且所在 edge 必须有显式 successor
或至少属于一条 ManeuverPath transition。若该 StopLine 存在 transitionIndex=0 的 entry gate，
则从该 edge 出发的每个显式 successor 都必须有唯一完整 ManeuverPath 覆盖，且每条这样的 path
也必须有自身的 transitionIndex=0 gate。

WaitingZone 的 entry/release gate 必须都属于其 ManeuverPath，且
`entry.transitionIndex < release.transitionIndex`；同一 path 上按半开 transition 区间
`[entry, release)` 比较，任意两区间不得重叠，相邻边界允许。SignalGroup 除必须被一个
controller 拥有并被其每个 phase 覆盖外，还必须至少控制一个 `signalControlKind=Group` 的
ManeuverGate；ParkingArea 同样必须至少拥有一个 ParkingSpace。

AccessRule 的 regulation 存在时，`jurisdiction/version` 以及可选 `source` 各自必须包含
`1..=128` 个 Unicode scalar；全对象所有 regulation 必须使用同一组
`(jurisdiction, version)`，但 `source` 可以不同。把 target 解析为 StableId 后，按
`(accessPlane, targetKind, targetStableId, participantClassStableId, priority)` 分组，其中
LaneEdge/LaneGroup/RoadSection 属于 Edge plane，ManeuverPath 属于 ManeuverPath plane；同组
可以有多个相同 effect，但不得同时出现 Allow 与 Deny。

固定时制信号还必须满足下列闭合语义。每个 `SignalPhase.durationMs` 位于
`1..=9_007_199_254_740_991`；控制器按 `phases` 领域顺序做 `u64` checked sum，结果不得超过
同一上限并必须逐值等于 `cycleDurationMs`；`offsetMs` 不得超过该上限且必须严格小于
`cycleDurationMs`。任一零时长、溢出、累计值不等或 offset 越界都在 revision 接受前失败。

`ParticipantClass` 必须形成无环单继承森林。根的 `parent` 缺失且 `depth=0`，非根的
`depth=parent.depth+1`（checked）。森林以 `typedOrdinal` 递增排列根及同父 children，执行确定性
深度优先前序：访问节点时把从 `0` 开始的计数写入 `subtreeEnter` 后加一，全部后代完成时把
当前计数写入 `subtreeExit`。因此 enter 值精确覆盖 `0..count-1`，每行满足
`enter < exit <= count`，祖先关系当且仅当其半开区间包含后代 enter；非祖先区间不得重叠。
循环、错误 depth、gap/重复 enter、非规范 sibling 顺序或任一错误 interval 都失败关闭。

`CanonicalRelationTables(0x0004)` 精确包含：

| tableKind | 表名                       | 字段                                                                                                                                                                                                                                                    | 行键                                                       |
| --------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| `0x0001`  | JunctionInternalEdge       | `1:laneEdge:u32:R, 2:junction:u32:R`                                                                                                                                                                                                                    | `laneEdge`                                                 |
| `0x0002`  | RouteManeuverOccurrence    | `1:staticRoute:u32:R, 2:occurrenceIndex:u32:R, 3:maneuverPath:u32:R, 4:entryRouteEdgeIndex:u32:R, 5:exitRouteEdgeIndex:u32:R, 6:gateOccurrenceStart:u32:R, 7:gateOccurrenceCount:u32:R, 8:waitingOccurrenceStart:u32:R, 9:waitingOccurrenceCount:u32:R` | `(staticRoute, occurrenceIndex)`                           |
| `0x0003`  | RouteGateOccurrence        | `1:staticRoute:u32:R, 2:occurrenceIndex:u32:R, 3:maneuverGate:u32:R, 4:maneuverOccurrenceIndex:u32:R, 5:fromRouteEdgeIndex:u32:R, 6:nextGateOccurrenceIndex:u32:O, 7:nextBoundaryRouteEdgeIndex:u32:R, 8:waitingZoneOccurrenceIndex:u32:O`              | `(staticRoute, occurrenceIndex)`                           |
| `0x0004`  | RouteWaitingZoneOccurrence | `1:staticRoute:u32:R, 2:occurrenceIndex:u32:R, 3:waitingZone:u32:R, 4:maneuverOccurrenceIndex:u32:R, 5:entryGateOccurrenceIndex:u32:R, 6:releaseGateOccurrenceIndex:u32:R, 7:entryRouteEdgeIndex:u32:R, 8:releaseRouteEdgeIndex:u32:R`                  | `(staticRoute, occurrenceIndex)`                           |
| `0x0005`  | StableRouteReverseIndex    | `1:entityKind:u16:R, 2:typedOrdinal:u32:R, 3:staticRoute:u32:R, 4:occurrenceIndex:u32:R`                                                                                                                                                                | `(entityKind, typedOrdinal, staticRoute, occurrenceIndex)` |

反向索引只允许 `LaneEdge/ManeuverPath/ManeuverGate/WaitingZone` 四种 `entityKind`，且必须
与前三张 occurrence 表及 `StaticRoute.edges` 双向完全一致。

三张 occurrence 表物理上按 `(staticRoute, occurrenceIndex)` 全局排序，反向索引按其登记的
四元行键排序；但全部 occurrence/index 字段都是所属 `StaticRoute` 内的局部坐标，绝不是
全局扁平表下标。对每条 route，三类
occurrenceIndex 各自从 `0` 连续编号；`entry/exit/from/nextBoundary*RouteEdgeIndex` 只引用该
route 的 `edges`，`maneuverOccurrenceIndex` 只引用该 route 的 maneuver 表，
`next/entry/release*GateOccurrenceIndex` 只引用该 route 的 gate 表，
`waitingZoneOccurrenceIndex` 只引用该 route 的 waiting-zone 表。

每个 maneuver 行满足 `entry <= exit < edges.count`，其 route edge slice 必须逐项等于所引
`ManeuverPath.edges`；`gateOccurrenceStart/count` 与 `waitingOccurrenceStart/count` 是各自
route-local 表的半开区间。按 maneuver occurrence 顺序，这些区间必须从零开始、严格相邻并
完整分割相应表。每个 gate 行必须落在其 `maneuverOccurrenceIndex` 的 gate 区间，且
`fromRouteEdgeIndex = maneuver.entryRouteEdgeIndex + ManeuverGate.transitionIndex`；同一
maneuver 内除末项外 `nextGateOccurrenceIndex` 恰为下一 gate 行，末项必须缺失，
`nextBoundaryRouteEdgeIndex` 分别等于下一 gate 的 `fromRouteEdgeIndex` 或该 maneuver 的
`exitRouteEdgeIndex`。每个 waiting-zone 行必须落在其 maneuver 的 waiting 区间，两个 gate
index 必须落在同一 maneuver gate 区间并分别解析为该 `WaitingZone` 的 entry/release gate，
两个 route-edge index 必须逐值等于对应 gate 的 `fromRouteEdgeIndex`；gate 上的可选
`waitingZoneOccurrenceIndex` 与该 entry gate 关系必须双向完全一致。任何跨 route 引用、
区间 gap/overlap、范围外 index 或逆关系不一致都失败关闭。

这些 occurrence 不是可由 writer 自选的辅助缓存，而是 `StaticRoute.edges` 与全部
ManeuverPath 的完整唯一投影。route 不得以 JunctionInternalEdge 开始或结束，也不得终止在
StopLine 所在 edge。对每个相邻 route edge pair：若两项都不触及 internal edge，前项必须在
`LaneEdge.successors` 中显式引用后项；再以当前位置起始的完整 route slice 匹配所有首两项相同
的 ManeuverPath，零个候选允许无 maneuver，存在候选但无完整匹配时失败，完整匹配超过一个也
失败。唯一匹配产生且只产生一条 maneuver occurrence，并完整投影该 path 的 gate/waiting
occurrence；不同匹配的 internal route-edge 区间不得重叠，route 中每个 internal edge 又必须被
恰好一条匹配覆盖。

`StaticRoute.transitionGates[i]` 必须从上述唯一匹配重算：若覆盖 transition `i` 的 path 在
对应 path-local transition 有 ManeuverGate，则保存该 gate，否则缺失；writer 不得用全
`None` 掩盖已匹配 maneuver。三张 occurrence 表、反向索引和 transitionGates 任一遗漏、额外
或不能从同一算法重算都在 revision 接受前失败。

`CanonicalSpatialTables(0x0005)` 精确包含：

| tableKind | 表名                 | 字段                                                                                                                                                    | 行键           |
| --------- | -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------- |
| `0x0001`  | SpatialPresence      | `1:spatialPresent:u8:R, 2:geometryDirectionProfile:u8:R`                                                                                                | singleton      |
| `0x0002`  | LaneEdgeGeometry     | `1:laneEdge:u32:R, 2:canonicalFrame:u32:R, 3:arcLengthMeters:f32:R, 4:points:RecordVector:R, 5:segments:RecordVector:R, 6:directionProfileApplies:u8:R` | `laneEdge`     |
| `0x0003`  | FacilityBandGeometry | `1:facilityBand:u32:R, 2:canonicalFrame:u32:R, 3:points:RecordVector:R, 4:directionProfileApplies:u8:R`                                                 | `facilityBand` |

`spatialPresent` 只允许 `0/1`。发射器从同次 LIR 重建四个布尔量：`hasProfile` 当且仅当
`geometry_profiles` 为 `Some`，`hasFrame` 当且仅当 CanonicalFrame 实体表非空，另两个量分别
表示 LaneEdgeGeometry、FacilityBandGeometry 表非空；`spatialPresent` 必须精确等于这四项的
逻辑或。为 `0` 时 direction profile code 必须为 `0=None`，CanonicalFrame 与两张 geometry
表都必须为空。为 `1` 时 LaneEdgeGeometry 允许为空；若非空，则必须与 LaneEdge 表同基数同
ordinal，禁止部分覆盖。FacilityBandGeometry 按 `facilityBand` 行键保持唯一规范子集，不要求与
FacilityBand 表同基数；因此已有 LaneEdge 而只有 FacilityBand geometry 的成功输出可表示。
direction code 始终严格投影 `geometry_profiles`：`None` 写 `0=None`，`Some` 写对应非零值。
#298 历史完整接受域以 direction code 非零重建 wire `hasProfile`，再与其余三项执行同一逻辑或；下文
CompilerProvenance 的 accuracy code 必须与 direction code 具有相同的零/非零 presence。
`geometryDirectionProfile` 的闭合代码为 `0=None, 1=Smooth1Deg, 2=Balanced2Deg,
3=Compact5Deg`。每张 geometry 的 `directionProfileApplies` 只允许 `0/1`：`1` 表示该行的最终
折线受上述唯一全局 direction profile 约束，`0` 表示该行没有 frontend direction policy。
全局 code 为 `0` 时全部行的适用标记必须为 `0`；任一行标记为 `1` 时全局 code 必须非零。
全局 code 非零但某行标记为 `0` 是混合 RoadEditing/Synthetic 编译的合法状态，不得把该 code
无条件套到该行，也不得为它另选默认或逐行 profile。#298 历史完整接受域无论标记为何仍核对全部点、段、
frame 和 join gap，只按下文的适用规则核对最终折线和跨 edge full-angle。来源曲线到最终折线的
accuracy profile 无法从最终点表独立重算，
因此不在本语义表中，也不构成 LFCA 对 `2/5/10 cm` 最大误差的自证声明；它只按下文进入
非语义 CompilerProvenance，并由受认证的 compiler/receipt 链承担来源策略真实性。不得为
补回该不可验证声明而向 LFCA 增加 reference curve、误差证书或第二套 oracle。
`points` 内嵌行为 `1:x:f32:R, 2:y:f32:R, 3:z:f32:R`；
`segments` 内嵌行为 `1:lengthMeters:f32:R, 2:cumulativeEndMeters:f32:R,
3:tangentX:f32:R, 4:tangentY:f32:R, 5:tangentZ:f32:R, 6:upX:f32:R, 7:upY:f32:R,
8:upZ:f32:R`。

每张 `LaneEdgeGeometry` 的 points 至少两项，segments 必须精确为 `points.count-1`。所有点分量
有限、位于 `[-16_384, 16_384] m` 且零为 bit-exact `+0.0f32`。对每对相邻点按字段顺序执行
冻结的 binary32 运算：先计算并把有符号零规范为正零的 `delta=(next-current)`，再计算
`length = hypot(hypot(delta.x, delta.y), delta.z)`；length 必须严格大于 `0.1 m`。
`cumulativeEnd` 是从 `+0.0f32` 起逐段做 binary32 加法的结果，必须有限且严格递增；最后一项
必须与 `arcLengthMeters` 位模式相同。该值与同 ordinal `LaneEdge.lengthMeters` 的差还必须
满足 `abs(diff) <= max(0.01 m, 1.0e-6 * max(edgeLength, f64(arcLength))) + 0.0 m`；末项是
`numeric-representation.md` 冻结的 Core length quantization allowance，不得由实现省略或改写。

这里所有 `hypot(a,b)` 都精确表示 `HypotRteF32(a,b) = RN32(sqrt(Exact(a)^2 +
Exact(b)^2))`：`Exact` 把 binary32 操作数提升为精确实数，`sqrt` 取非负实根，`RN32` 只在
末尾按 IEEE 754 binary32 `roundTiesToEven` 舍入一次。三维长度继续按写出的左结合顺序调用
两次该二元原语。普通 `+ - * /` 每个具名中间值也分别 roundTiesToEven 到 binary32，禁止
FMA、运算重排或额外精度；结果为零时规范成 `+0.0f32`。平台 `hypotf`/Rust `f32::hypot`
精度未指定，不能作为该原语的规范实现；实现可以使用整数/多精度或已证明正确舍入的软件
例程，但必须对全部输入产生上述唯一位模式。

每段 basis 也只能从该 delta 重算：`normalize(v)` 先除以三个绝对分量的最大值，再以
`hypot(hypot(x,y),z)` 归一化并把零规范为正零；`tangent=normalize(delta)`，
`projectedUp=hypot(tangent.x,tangent.z)` 必须 `>=0.008_726_535`，
`left=normalize([tangent.z,0,-tangent.x])`，
`up=normalize([tangent.y*left.z, tangent.z*left.x-tangent.x*left.z,
-tangent.y*left.x])`。存储的 length/cumulative/tangent/up 必须与该顺序重算的 f32 位模式逐项
相同；该行 `directionProfileApplies=1` 时还必须使用非零 `geometryDirectionProfile` 继续通过
`numeric-representation.md`/ADR 0022 冻结的最终弦方向谓词，等于 `0` 时不得暗中选择默认
profile。不得把任一预计算字段当作点表之外的第二权威。

LaneEdgeGeometry 非空时还必须从对象自身建立有向连接对集合：先加入全部
`LaneEdge.successors`，再加入每条 ManeuverPath 的每对相邻 edge；重复 pair 只检查一次，
path-only transition 不能因未出现在 successors 中而跳过。每个 `(predecessor, successor)`
的两张 LaneEdgeGeometry 必须引用同一 CanonicalFrame。取 predecessor 最后一点与 successor
第一点，三个差分各按 binary32 roundTiesToEven 并规范零，再按
`HypotRteF32(HypotRteF32(dx,dy),dz)` 重算 gap；只接受
`gap <= 0.005f32`（bits `0x3ba3d70a`，等号有效），不得吸附、焊接或移动端点。

方向谓词的适用集合按 geometry 行闭合：一张 geometry 内的相邻弦对只在该行
`directionProfileApplies=1` 时受检；有向连接 pair 的 predecessor 末弦和 successor 首弦在
任一端适用标记为 `1` 时受检。所有受检弦对必须通过同一个非零
`geometryDirectionProfile` 谓词：先把两弦的 binary32 点分量无损提升为 binary64 后分别做
末点减首点，再各除以自身
绝对分量最大值；
按 `dot=(x*x'+y*y')+z*z'`、`norm2=(x*x+y*y)+z*z`、
`lhs=dot*dot`、`rhs=(C*norm2(left))*norm2(right)` 的写出顺序逐运算舍入到 binary64，只接受
`dot > 0 && lhs >= rhs`。`C` 的 binary64 bits 对 Smooth/Balanced/Compact 依次为
`0x3feffd813c5f82b4`、`0x3feff605b8b87ffc`、`0x3fefc1c5c6408e0c`；禁止 FMA、重结合、
`acos/cos`、额外精度或实现自选 epsilon。适用行必须覆盖其全部内部相邻弦对，适用连接必须覆盖
两端 join；任一 polyline 不能只检查首尾或只检查跨 edge join。行标记为 `0`、连接两端标记都为
`0`，或全局 code 为 `0=None` 时，只跳过对应方向谓词，不能跳过 frame、gap、点表、段表或
arc-length 验证。任何适用检查失败都在建立 spatial view 前失败关闭。

`FacilityBandGeometry.points` 同样必须至少两项，并逐点、逐弦执行上述分量范围、正零、
`length>0.1 m` 和有限严格递增累计；该行 `directionProfileApplies=1` 时再执行所选非零 profile
检查。它不保存 segments 或 arcLength，验证器只使用 points 重算，不得为其接受额外预计算值。

`StaticExecutionConstraints(0x0006)` 只有 `ExecutionContract(0x0001)` singleton：
`1:staticExecutionContractVersion:u16:R, 2:constraintContractVersion:u16:R`。具体约束已由
前述实体、关系和空间表的规范值表达；该行禁止另存 worker 数、目标布局或运行时状态。
LFCA v1 的 `ContractVersions` 六个字段都只接受值 `1`；`ExecutionContract` tag 1 必须逐值
等于 `ContractVersions.staticExecutionContractVersion`，tag 2 必须逐值等于
`ContractVersions.constraintContractVersion`。结构预检完成后必须先验证支持值和这两项对象
内相等关系，再执行依赖相应 contract 的实体语义验证；任一未知值或副本不一致都失败关闭，
不得由实现选择其中一份作为权威。

`CompilerProvenance(0x0007)` 只有 `CompilerProvenance(0x0001)` singleton：
`1:compilerBuildId:Utf8:R, 2:sourceCollectionDigestVersion:u16:R,
3:sourceCollectionDigest:Sha256:R, 4:compileOptionsDigest:Sha256:R, 5:emitterVersion:u16:R,
6:geometryAccuracyProfile:u8:R`。
`ArtifactClaims(0x0008)` 只有 `ArtifactClaims(0x0001)` singleton：
`1:declaredNetworkRevisionId:Sha256:R`。

`PortableEmissionProvenanceV1` 精确提供 `compilerBuildId`，其余字段由 v1 规则派生：

- `compilerBuildId` 是构建系统一次提供的 1..=128-byte ASCII 标识，必须匹配
  `[A-Za-z0-9][A-Za-z0-9._+@-]{0,127}`；同一 compiler build 的所有支持 target 必须提供
  完全相同的 bytes。路径、target triple、时间戳、worker 数、进程/机器标识、随机 nonce
  和环境变量展开结果不得进入该值；
- `sourceCollectionDigestVersion=1`，`sourceCollectionDigest` 必须由同一个
  `CompilationOutput.ValidatedSourceMapInput` 按 A.2 的精确前像重算；调用方不得覆盖；
- `emitterVersion=1`；
- `geometryAccuracyProfile` 严格投影同次编译 LIR 的 `geometry_profiles`：`None` 写
  `0=None`，`Some` 按闭合代码 `1=Fine2Cm, 2=Balanced5Cm, 3=Compact10Cm` 写入；因此
  `spatialPresent=0` 必须为 `0`，全局 profile 缺失的显式 Synthetic 或 facility-only geometry
  也必须为 `0`；混合编译中它可因其他 RoadEditing geometry 而非零。#298 历史完整接受域只核对该投影、
  枚举与 object binding，不得把它报告为已独立证明的最大位置误差；accuracy 与 semantic
  direction code 必须同时为零或同时非零，单边伪造 profile presence 失败关闭。逐 geometry
  `directionProfileApplies` 只控制方向谓词，不建立第二个 accuracy profile，也不改变该全局
  presence 一致性；
- v1 没有会改变 portable bytes 的外部编译选项，因而
  `compileOptionsDigest = SHA-256("laneflow.portable-compile-options.v1\0" ||
  optionCount:u32=0)`，其中 `u32` 为小端；固定结果为
  `322682f455d06b36e9e3719f341db38f3ecda61d52c53d9d6fe3dca540eef445`。最终 direction
  profile 属于规范 LIR/LFCA 语义，accuracy profile 已由本行显式保存为非语义来源沿袭；
  base 只影响 LFSD，limits/worker 数只控制资源，均不得另外塞入该摘要。

LFCA `CompilerProvenance`、LFSM `SourceMapBindings` 和 LFCP `SourceMapBinding` 中重复的
`compilerBuildId/sourceCollectionDigestVersion/sourceCollectionDigest` 必须逐字节相等；
LFCP 通过 `canonicalArtifactDigest` 间接绑定 LFCA 独有的
`compileOptionsDigest/emitterVersion/geometryAccuracyProfile`。任一不一致都是 binding
mismatch；不得把这些字段当作对象内信任锚。

### A.2 LFSM table registry

本节严格区分两种检查主体。凡规则引用 compiler-private `ValidatedSourceMapInput`、
`primary_source()`、`contributing_sources()` 或 `geometry_source_ranges()`，都只属于
`laneflow-compiler` 在返回 `PortablePublicationCandidate` 前的候选发射闭合；这些 API 不进入
LFSM wire。#298 历史完整接受域曾要求依据 LFSM exact bytes、digest-bound LFCA、受限外部对象
与本 registry 执行全量结构、排序、双射、地址和摘要复验；Accepted ADR 0024 已明确不把它
交付为 #299 独立 validator。当前后发射检查只核对 LFSM 的 LFCA/provenance binding，不重建
compiler-private view，也不证明来源沿袭与原始编译输入完全一致；后者依赖对象外的可信
compiler/emission 环境、来源资产链或认证发布清单。

`SourceMapBindings(0x0001)` 的 `SourceMapBindings(0x0001)` singleton 为：

```text
1:networkRevisionDerivationVersion:u16:R
2:networkRevision:Sha256:R
3:canonicalArtifactFormatVersion:u16:R
4:canonicalArtifactDigest:Sha256:R
5:canonicalArtifactByteLength:u64:R
6:compilerBuildId:Utf8:R
7:sourceCollectionDigestVersion:u16:R
8:sourceCollectionDigest:Sha256:R
```

LFSM 接受必须先绑定并验证 LFCA，不能先暴露任一来源行：以
`canonicalArtifactByteLength` 约束 exact bytes，重算 `canonicalArtifactDigest`，结构预检
LFCA 后，再要求 `canonicalArtifactFormatVersion` 同时等于 LFCA preamble `formatVersion` 与
`ContractVersions.canonicalFormatVersion`，`networkRevisionDerivationVersion` 等于
`ContractVersions.networkRevisionDerivationVersion`，`networkRevision` 等于从六个语义节
重算的值。版本、摘要、长度或修订任一不等都按 source-map artifact binding mismatch 失败，
不得用 LFSM 自报版本选择 LFCA decoder 或推迟到位置查询之后比较。

v1 中 `sourceCollectionDigestVersion=1`，并精确计算：

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

该摘要只绑定来源集合，不替代逐文档 digest/length，也不进入 NetworkRevisionId。

`SourceModules(0x0002)` 精确包含：

| tableKind | 表名           | 字段                                                                                                                                                                                                                                                                                                                                                                                         | 行键                    |
| --------- | -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- |
| `0x0001`  | SourceModule   | `1:sourceModuleOrdinal:u32:R, 2:authoringNamespaceId:Utf8:R, 3:sourceLanguage:u16:R, 4:sourceDocumentSetDigest:Sha256:R, 5:sourceDocumentSetDigestVersion:u32:R, 6:frontendVersion:u32:R, 7:frontendOptionsDigest:Sha256:R, 8:generatorBuildId:Utf8:R, 9:parametersAndInputsDigest:Sha256:R, 10:randomSeed:u64:O, 11:provenance:Utf8:R, 12:imports:RecordVector:R, 13:primaryLocation:u32:R` | `sourceModuleOrdinal`   |
| `0x0002`  | SourceDocument | `1:sourceDocumentOrdinal:u32:R, 2:sourceModuleOrdinal:u32:R, 3:sourceDocumentKey:Utf8:R, 4:sourceContentDigest:Sha256:R, 5:sourceRecordByteLength:u32:R, 6:displaySource:Utf8:O`                                                                                                                                                                                                             | `sourceDocumentOrdinal` |
| `0x0003`  | SourceLocation | 见下列闭合字段                                                                                                                                                                                                                                                                                                                                                                               | `sourceLocationOrdinal` |

`sourceLanguage` 只允许 `1=SyntheticDsl, 3=RoadEditingSource`；LFSM v1 分别只接受
`frontendVersion=2` 与 `frontendVersion=1`。`imports` 的每个内嵌行只有
`1:authoringNamespaceId:Utf8:R`，按 namespace UTF-8 bytes 严格递增；模块行必须按 §5 的
Kahn ready-set 最小 namespace 规则从 `0` 连续编号，任意 import 必须指向更小的 module
ordinal。循环、重复 namespace 或无法解析的 import 在编译阶段已经失败，不产生 LFSM。
SourceDocument 行按 module ordinal、模块内 document-key bytes 从 `0` 连续编号，位置池按
§5 完整语义值排序去重并从 `0` 连续编号；三个表都禁止 gap、重复值和一致重编号。
`primaryLocation`
必须解析到 SourceLocation，且其 module ordinal 必须等于当前行。SourceDocument key 必须
全局唯一，并在同一模块内按 UTF-8 bytes 严格递增；SyntheticDsl 的 `displaySource` 必须
缺失，RoadEditingSource 可以缺失或保存调用方的未认证显示来源。

每个 SourceModule 必须至少拥有一张 SourceDocument 行，且候选发射时与同次成功编译的
`ValidatedSourceMapInput` 文档描述符形成严格双射；模块不得遗漏未被位置引用的文档，也不得
添加输入中不存在的文档。`sourceDocumentSetDigestVersion` 只允许 `1`，并对该模块按
`sourceDocumentKey` bytes 严格递增的完整行集精确重算：

```text
SHA-256(
  "LFSOURCE-DOCUMENT-SET"
  || 1:u32 little-endian
  || documentCount:u32 little-endian
  || for each document:
       sourceDocumentKeyByteLength:u32 little-endian
       || sourceDocumentKeyUtf8Bytes
       || sourceRecordByteLength:u32 little-endian
       || sourceContentDigest:Sha256
)
```

候选发射时，SourceModule 行本身也必须与同一个
`ValidatedSourceMapInput.source_module_sources()` 形成按
iterator 位置的一一投影，而不是仅让文档集闭合。iterator 位置精确等于 tag 1；其
`descriptor()` 的 authoring namespace、source language、document-set digest/version、frontend
version/options digest、generator build、parameters-and-inputs digest、optional random seed、
provenance 与 imports 必须分别逐值等于 tag 2..12；tag 13 解引用后的完整 SourceLocation
语义值必须逐值等于同一 view 的 `primary_source()`。其中 source language 使用上述闭合 wire
code，imports 使用 descriptor 已冻结的 namespace bytes 顺序。候选 API 不接受这些字段或
primary location 的调用方 override；任何遗漏、额外 module、重新配对、只保持 module ordinal
相同却更换位置，或 descriptor 字段单独变化，都在计算 LFSM digest 和返回
`PortablePublicationCandidate` 前失败。

`sourceRecordByteLength/sourceContentDigest` 必须分别等于该文档完整来源记录的精确长度与
SHA-256；重算结果必须逐字节等于 module 行的 `sourceDocumentSetDigest`，再进入上层
`sourceCollectionDigest`。这些全量闭合属于 #298 历史接受域；当前 checker 只绑定已发射的
`sourceCollectionDigest`，不重新枚举外部来源。未认证的
LFSM 自身不能证明一个完全未出现的外部文档曾经存在，该真实性仍由 LFCP/发布清单的外部
trust anchor 承担。

`SourceLocation` 公共字段为 `1:sourceLocationOrdinal:u32:R, 2:sourceLocationKind:u8:R,
3:sourceModuleOrdinal:u32:R, 4:sourceDocumentOrdinal:u32:R`。`Text(0)` 还必须且只能包含
`5:startLine:u32:R, 6:startColumn:u32:R, 7:endLine:u32:R, 8:endColumn:u32:R`。
四个 Text 坐标均为一基非零值，且 `(startLine,startColumn)` 不得晚于
`(endLine,endColumn)`。
`RoadEditing(1)` 还使用：

```text
9:roadEditingSubjectKind:u8:R
10:moduleNamespace:Utf8:O
11:entityKind:u16:O
12:ownerLocalKey0:Utf8:O
13:ownerLocalKey1:Utf8:O
14:ownerLocalKey2:Utf8:O
15:localKey:Utf8:O
16:ownerKind:u8:O
17:roadEditingRelationKind:u8:O
18:occurrenceKind:u8:O
19:occurrenceOrdinal:u32:O
20:propertySteps:RecordVector:O
21:canvasSelection:Utf8:O
```

tag 3/4 必须解析到同一 SourceModule/SourceDocument 归属；RoadEditing address 的
`moduleNamespace` 必须逐字节等于该 SourceModule 的 `authoringNamespaceId`。
每个 SourceLocation 的种类还必须与所属模块语言一致：SyntheticDsl 只允许 Text，
RoadEditingSource 只允许 RoadEditing；不得把同一组数字字段换一种语言解释。

`ModuleHeader(0)` 禁止 tag 10..19；`RoadAlignment(1)` 要求完整 address（tag 10、15，
禁止 11）；`Declaration(2)` 要求完整 address（tag 10、11、15）；`OwnerLocal(3)` 要求
tag 16..19，`ownerKind=0(ModuleHeader)` 时禁止 address，`ownerKind=1(Address)` 时要求
tag 10、15 且 tag 11 只在 owner 是 Declaration 时存在。owner local key 必须从 tag 12
开始连续出现，最大深度三；RoadAlignment 深度为 0，RoadSection/Movement/FacilityBand/
SignalPhase 为 1，AuthoringLane/ManeuverPath/LaneGroup 为 2，ManeuverGate/WaitingZone 为
3，其余 Declaration 为 0。`occurrenceKind` 为 `0=OrderedProductOrdinal,
1=CanonicalSetOrdinal`。

`propertySteps` 若存在必须有 1..=4 行，每行
`1:stepKind:u8:R, 2:containerCode:u16:R, 3:memberCode:u16:R`；`stepKind` 为
`0=TableField, 1=StructMember, 2=UnionVariant`，后两种的 `containerCode/memberCode` 高
8 bit 必须为零。`memberCode` 必须是该 RoadEditing source format version 对相应 container
声明的 field/member/discriminant，未知值失败关闭；table/struct/union 与 relation 的
精确代码按 A.5；`canvasSelection` 缺失
与空 UTF-8 是不同语义值。成功 LFSM 禁止 `Wire` subject 和 byte range。

`StableEntitySources(0x0003)` 只有 `StableEntitySource(0x0001)`：
`1:entityKind:u16:R, 2:stableId:StableId128:R, 3:typedOrdinal:u32:R,
4:primaryLocation:u32:R, 5:contributingLocations:OrdinalVectorU32:R`，行键为
`(entityKind, stableId, typedOrdinal)`。

该表必须与绑定 LFCA 的 `CanonicalIdentity` 和 22 张实体表形成严格双射：每个
`(entityKind, typedOrdinal, stableId)` 恰有一行且逐值相等，LFSM 不得遗漏实体、添加没有
绑定实体的来源行或把 ordinal/stableId 重新配对。`primaryLocation` 必须解析；
`contributingLocations` 是允许为空、按完整位置语义值严格递增且去重的集合。候选发射时定义
`C(view)` 为把同一 `ValidatedSourceMapInput` view 的 `contributing_sources()` 按完整位置语义
值排序、去重后映射成最终 SourceLocation ordinal 的唯一向量。每行 `primaryLocation` 必须
逐值等于对应 stable-entity view 的 `primary_source()`，`contributingLocations` 必须逐字节
等于 `C(view)`；当前 22 类 stable-entity view 的贡献迭代器都为空，因此 v1 该字段必须为空，
writer 不得添加另一合法位置作为“补充 provenance”。#298 历史完整接受域曾核对 artifact
binding、上述双射、位置解析、v1 空贡献字段和下列 identity/address 线格式投影；#299 当前
checker 不执行该全量复验。一个内部自洽但由不可信 emitter 替换的合法 Synthetic Text 位置
不由 LFSM 自证为原始 `primary_source()`。

双射还必须绑定“实体是谁”与“主位置声明了谁”。每行 primaryLocation 所属 SourceModule 的
`authoringNamespaceId` 必须逐字节等于绑定 CanonicalIdentity 的 tag 1。RoadEditingSource
主位置必须是 `Declaration`，其 address `entityKind` 等于该行 entityKind、`localKey` 等于
下列 entity-local key tag 的 ASCII bytes，且 owner-local key tuple 等于下列 parent anchor
递归解析出的 root-to-direct-parent key tuple：

```text
local-key tag:
  RoadCorridor=2 RoadSection=3 AuthoringLane=4 LaneEdge=5 Junction=6
  Movement=8 ManeuverPath=7 ManeuverGate=15 WaitingZone=16 StopLine=17
  SignalGroup=18 SignalController=19 SignalPhase=21 ParkingArea=22
  ParkingSpace=24 LaneGroup=25 FacilityBand=26 ParticipantClass=27
  AccessRule=28 VehicleProfile=29 StaticRoute=30 CanonicalFrame=31
parent StableId anchor:
  RoadSection=33 AuthoringLane=32 Movement=34 ManeuverPath=11
  ManeuverGate=14 WaitingZone=14 SignalPhase=20 LaneGroup=32 FacilityBand=33
  all other entity kinds=None
```

递归时每个 parent StableId 必须先解析到全局唯一 CanonicalIdentity，再取该 parent 的
entity-local key；不得直接把 StableId hex、raw ordinal 或来源 address 自报值当作 owner key。
SyntheticDsl 没有结构化 declaration address，只执行 module namespace 绑定。任一 entity
kind、namespace、local key、owner 深度或 owner key 不等都在建立来源视图前失败关闭；
候选发射器还要求 contributingLocations 是前述 `C(view)` 的精确投影；独立 wire 接受对
StableEntitySource v1 直接要求该向量为空，不要求贡献位置伪装成主声明。

`OwnerLocalSources(0x0004)` 精确包含：

| tableKind | 表名                       | 字段                                                                                                                                                                                                         | 行键                                                                           |
| --------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------ |
| `0x0001`  | OwnerLocalSource           | `1:ownerEntityKind:u16:R, 2:ownerStableId:StableId128:R, 3:sourceRelationRole:u8:R, 4:localIndex:u32:R, 5:primaryLocation:u32:R, 6:contributingLocations:OrdinalVectorU32:R`                                 | `(ownerEntityKind, ownerStableId, sourceRelationRole, localIndex)`             |
| `0x0002`  | SpatialGeometrySourceRange | `1:ownerEntityKind:u16:R, 2:ownerStableId:StableId128:R, 3:sourceRelationRole:u8:R, 4:localIndex:u32:R, 5:pointStart:u32:R, 6:pointEndExclusive:u32:R, 7:sourceSegmentOrdinal:u32:R, 8:sourceLocation:u32:R` | `(ownerEntityKind, ownerStableId, sourceRelationRole, localIndex, pointStart)` |

附录 A.5 是全部 `sourceRelationRole 1..29` 的 owner、subject、LFCA 投影与 localIndex 唯一
registry。对绑定 LFCA 按该表投影出的每个 tuple，LFSM 必须恰有一行相同
`(ownerEntityKind, ownerStableId, role, localIndex)` 的 `OwnerLocalSource`；optional scalar
缺失时恰无行。任何遗漏、额外行、错误 owner/role/index，或同一键不能反解到表中唯一 LFCA
tuple 都失败关闭。候选发射时，每个 tuple 必须选择同一 `ValidatedSourceMapInput` 中由 A.5 role 对应的
owner-local view；`primaryLocation` 必须逐值等于其 `primary_source()`，
`contributingLocations` 必须逐字节等于 `C(view)`，不能只是任意合法且有序的位置集合。
当前普通显式关系贡献集为空，role 14..16 的 route-derived view 则精确保留各自
ManeuverPath/ManeuverGate/WaitingZone 声明贡献位置。OwnerLocalSource 不重复保存 subject，
#298 历史完整接受域不复演该私有 view 比较，而是从绑定 LFCA 和 A.5 一一反解 subject，核对位置引用、
集合排序、DerivedRelationSources 联合关系和下列 RoadEditing 地址投影；#299 当前 checker
不执行该全量来源视图复验。若
`primaryLocation` 属于
RoadEditingSource，还必须按 A.5 的 `RoadEditing primary-source projection` 把该位置的
Declaration/OwnerLocal subject、identity address、property path 和 occurrence 逐值绑定回同一
LFCA tuple；只证明位置在全局上合法，或把另一关系行的合法位置互换过来，都必须失败关闭。

`SpatialGeometrySourceRange` 只允许 `ownerEntityKind=CanonicalFrame` 以及
`sourceRelationRole=28(CanonicalFrameLaneEdgeGeometry)` 或
`29(CanonicalFrameFacilityBandGeometry)`。每行必须有同 owner/role/localIndex 的
`OwnerLocalSource` 父行；`sourceLocation` 必须解析到 SourceLocation。候选发射时，每个父行
先与对应 spatial relation view 的 `geometry_source_ranges()` 做精确投影：每个迭代项的 point range、
`sourceSegmentOrdinal` 和 source location 必须逐值生成恰好一行，不能遗漏、添加、重排或
合成 segment ordinal。候选发射器还必须把该迭代器是否非空精确投影为对应 LFCA geometry 行的
`directionProfileApplies`：非空写 `1`，空写 `0`。若迭代器为空，则
`SpatialGeometrySourceRange` 必须恰无子行，父行 `contributingLocations` 也必须为空；这是显式
Synthetic `CanonicalFrame` geometry 的合法 profile-free/source-range-free 状态，不要求伪造
segment。

#298 历史完整接受域不持有该迭代器：候选中范围为空时只核对父行贡献集为空；范围非空时，范围必须从
`pointStart=0` 开始，按行键严格相邻、非空且无重叠，最后一个
`pointEndExclusive` 必须等于对应 LFCA geometry `points` 的 item 数；localIndex 在该
frame/role 下按相应 LFCA geometry 表行顺序从零编号；非空状态的父行
`contributingLocations` 必须等于所有子行 `sourceLocation` 按位置语义值排序去重后的 ordinal
投影。#298 历史 LFCA/LFSM 交叉接受规则还要求每个父行的范围非空性与绑定 LFCA geometry 行的
`directionProfileApplies` 精确相等；只改 flag、只增删范围或把另一 geometry 的合法范围换入都
失败关闭。每个非空范围的 `sourceLocation` 还必须是 RoadEditing `OwnerLocal`：全部同父行范围的
module/document 与 Address owner 必须完全相同；relation 必须为 `CurveSegment`；occurrence
必须为 `OrderedProductOrdinal(sourceSegmentOrdinal)`；property path 必须精确为单步
`TableField(CurveSegment, 1)`，并按 A.5 的 OwnerLocal 可达性规则属于该 owner。Text、
Declaration、另一 owner、ordinal/occurrence 不等或错误 property path 都失败关闭。

候选发射器在两种状态下另外要求父行贡献集逐字节等于 `C(view)`，并把上述共同 owner、segment
ordinal 与完整 source location 逐项纳入私有迭代器 exact projection；因此协调替换为另一条
合法 segment 也不能形成候选。#298 历史完整接受域只覆盖 wire 内
owner/occurrence/property 闭合，不读取未认证来源文档，也不把该闭合提升为来源真实性。
这些全量规则不属于 #299 当前 checker。

`DerivedRelationSources(0x0005)` 只有 `DerivedRelationSource(0x0001)`：
`1:ownerEntityKind:u16:R, 2:ownerStableId:StableId128:R, 3:sourceRelationRole:u8:R,
4:localIndex:u32:R, 5:derivationPassVersion:u16:R, 6:constraintVersion:u16:R,
7:sourceLocations:OrdinalVectorU32:R`，行键与 OwnerLocalSource 相同。所有位置 ordinal
必须解析到 `SourceLocation`；contributing/source vectors 是按位置语义值排序去重的集合。
v1 只允许下列四类派生行：

| sourceRelationRole                    | owner kind    | 必需覆盖的绑定 LFCA 行                                                         | derivationPassVersion | constraintVersion                     |
| ------------------------------------- | ------------- | ------------------------------------------------------------------------------ | --------------------- | ------------------------------------- |
| `9=JunctionInternalEdge`              | `Junction`    | 该 junction 的 internal-edge 行；localIndex 是按 laneEdge 行键过滤后的零基位置 | `1`                   | LFCA `constraintContractVersion`      |
| `14=StaticRouteManeuverOccurrence`    | `StaticRoute` | 同 route 的 `RouteManeuverOccurrence`；localIndex 等于 occurrenceIndex         | `1`                   | LFCA `staticExecutionContractVersion` |
| `15=StaticRouteGateOccurrence`        | `StaticRoute` | 同 route 的 `RouteGateOccurrence`；localIndex 等于 occurrenceIndex             | `1`                   | LFCA `staticExecutionContractVersion` |
| `16=StaticRouteWaitingZoneOccurrence` | `StaticRoute` | 同 route 的 `RouteWaitingZoneOccurrence`；localIndex 等于 occurrenceIndex      | `1`                   | LFCA `staticExecutionContractVersion` |

每个被上表覆盖的 LFCA 行必须恰有一个 `DerivedRelationSource`，并恰有一个相同行键的
`OwnerLocalSource`；其 `sourceLocations` 必须逐字节等于该 owner-local 行的
`primaryLocation + contributingLocations` 按完整位置语义值排序去重后的集合。其他 role、owner
kind、版本值、遗漏/额外行或不能与绑定 LFCA 行一一对应的 localIndex 都失败关闭。两个版本值
由本 registry 和绑定 LFCA contract 派生，不要求 `ValidatedSourceMapInput` 另存一份可漂移
副本。

### A.3 LFSD table registry

`SemanticDiffBindings(0x0001)` 只有 `SemanticDiffBindings(0x0001)` singleton，tag `1..9`
依次对应 §6 列出的九个字段，类型依次为
`u8,u16,Sha256,Sha256,u64,u16,Sha256,Sha256,u64`，全部必需。

`sectionKind=0x0002..0x0005` 各只有同名 `tableKind=0x0001` 的 change table。所有变化行共同以
`1:changeKind:u8:R, 2:entityKind:u16:R, 3:ownerStableId:StableId128:O,
4:subjectStableId:StableId128:O, 5:sourceRelationRole:u8:O, 6:fieldTag:u16:O,
7:beforeLocalIndex:u32:O, 8:afterLocalIndex:u32:O` 开始；各节追加字段如下：

| sectionKind | 表名             | 追加字段                                                         | `changeKind`                           |
| ----------- | ---------------- | ---------------------------------------------------------------- | -------------------------------------- |
| `0x0002`    | EntityChange     | `9:beforeValue:Bytes:O, 10:afterValue:Bytes:O`                   | `0=Add, 1=Remove, 2=Modify`            |
| `0x0003`    | RelationChange   | `9:beforeTarget:StableId128:O, 10:afterTarget:StableId128:O`     | `0=Add, 1=Remove, 2=Move, 3=Reconnect` |
| `0x0004`    | GeometryChange   | `9:beforeCanonicalValue:Bytes:O, 10:afterCanonicalValue:Bytes:O` | `0=Add, 1=Remove, 2=Modify`            |
| `0x0005`    | StaticRuleChange | `9:beforeCanonicalValue:Bytes:O, 10:afterCanonicalValue:Bytes:O` | `0=Modify`                             |

下面是 tag 3..10 的完整存在性矩阵；`R` 表示该 change kind 必须存在，`F` 表示必须缺失。
未在相应行列出的 optional tag 同样视为 `F`，不存在实现者自选字段：

| 表/change kind       | 必需 common tags                                          | 禁止 common tags                           | payload 必需/禁止                                       |
| -------------------- | --------------------------------------------------------- | ------------------------------------------ | ------------------------------------------------------- |
| Entity `Add`         | `subjectStableId`                                         | `owner, role, fieldTag, before/afterIndex` | `afterValue:R, beforeValue:F`（完整目标 RowV1）         |
| Entity `Remove`      | `subjectStableId`                                         | `owner, role, fieldTag, before/afterIndex` | `beforeValue:R, afterValue:F`（完整 base RowV1）        |
| Entity `Modify`      | `subjectStableId, fieldTag`                               | `owner, role, before/afterIndex`           | `beforeValue/afterValue` 至少一个存在                   |
| Relation `Add`       | `ownerStableId, subjectStableId, role, afterIndex`        | `fieldTag, beforeIndex`                    | `beforeTarget:F, afterTarget:F`                         |
| Relation `Remove`    | `ownerStableId, subjectStableId, role, beforeIndex`       | `fieldTag, afterIndex`                     | `beforeTarget:F, afterTarget:F`                         |
| Relation `Move`      | `ownerStableId, subjectStableId, role, before/afterIndex` | `fieldTag`                                 | `beforeTarget:F, afterTarget:F`                         |
| Relation `Reconnect` | `ownerStableId, role, before/afterIndex`                  | `subjectStableId, fieldTag`                | `beforeTarget:R, afterTarget:R`                         |
| Geometry `Add`       | `subjectStableId`                                         | `owner, role, fieldTag, before/afterIndex` | `afterCanonicalValue:R, beforeCanonicalValue:F`         |
| Geometry `Remove`    | `subjectStableId`                                         | `owner, role, fieldTag, before/afterIndex` | `beforeCanonicalValue:R, afterCanonicalValue:F`         |
| Geometry `Modify`    | `subjectStableId`                                         | `owner, role, fieldTag, before/afterIndex` | `beforeCanonicalValue:R, afterCanonicalValue:R`         |
| StaticRule `Modify`  | `subjectStableId, fieldTag`                               | `owner, role, before/afterIndex`           | `beforeCanonicalValue/afterCanonicalValue` 至少一个存在 |

这里 `owner`、`role`、`beforeIndex`、`afterIndex` 是对应完整字段名的表内短写。所有行仍要求
公共 tag 1 `changeKind` 和 tag 2 `entityKind`；Relation 行的 `entityKind` 是 owner kind，其他
实体级行是 subject kind，且都只允许 LFCA `1..22`。Entity/StaticRule 的字段级 `Modify` 中，
`before*` 当且仅当 base field 存在，`after*` 当且仅当 target field 存在；二者至少一个存在，
若都存在则 bytes 必须不同。
这唯一表达 optional field 的出现/消失。Geometry `Modify` 两端都必须存在且不同；Relation
`Move` 的两个 index 必须不同，`Reconnect` 的两个 target 必须不同，但其 index 可以相等。

Artifact diff 在读取任何 change table 前，必须从两端 `CanonicalIdentity` 重建以
`StableId128` 为键的全局身份表；每个交集项的 `entityKind` 和完整
`identityCanonicalBytesV1` 必须逐字节相同。任一不等都是
`CrossRevisionStableIdCollision`，必须在 retained、字段、关系和 geometry 分类前拒绝整个
base/target 对，不产生 LFSD 候选。只有通过该检查后，相同 `(entityKind, StableId128)` 才是
retained identity，其字段差异的 change class 由下表排他决定；`—` 表示该类没有合法字段，
范围和集合以 LFCA 字段 tag 表示：

| entity table     | Entity `Modify` | 只投影 Relation，不生成字段 `Modify` | StaticRule `Modify` | Identity 语义锚 / derived cache；不生成 `Modify` |
| ---------------- | --------------- | ------------------------------------ | ------------------- | ------------------------------------------------ |
| RoadCorridor     | `3`             | `4`                                  | —                   | —                                                |
| RoadSection      | `4`             | `5`                                  | —                   | `3`                                              |
| AuthoringLane    | —               | `4,5`                                | —                   | `3`                                              |
| LaneEdge         | `3,4`           | `5`                                  | —                   | —                                                |
| Junction         | —               | `3`                                  | —                   | —                                                |
| Movement         | —               | `6`                                  | —                   | `3..5`                                           |
| ManeuverPath     | —               | `4..6`                               | —                   | `3`                                              |
| ManeuverGate     | `4`             | `5,7`                                | `6`                 | `3`                                              |
| WaitingZone      | —               | —                                    | `4..6`              | `3`                                              |
| StopLine         | `3`             | `4`                                  | —                   | —                                                |
| SignalGroup      | —               | `3,4`                                | —                   | —                                                |
| SignalController | —               | `5,6`                                | `3,4`               | —                                                |
| SignalPhase      | —               | —                                    | `4,5`               | `3`                                              |
| ParkingArea      | —               | `3`                                  | —                   | —                                                |
| ParkingSpace     | `5,7..11`       | `3,4,6`                              | —                   | —                                                |
| LaneGroup        | —               | `4`                                  | —                   | `3`                                              |
| FacilityBand     | `4`             | —                                    | —                   | `3`                                              |
| ParticipantClass | `4`             | `3`                                  | —                   | `5,6`（derived）                                 |
| AccessRule       | —               | `3,4,6`                              | `5,7,8`             | —                                                |
| VehicleProfile   | `4..10`         | `3`                                  | —                   | —                                                |
| StaticRoute      | —               | `3,4`                                | —                   | —                                                |
| CanonicalFrame   | —               | —                                    | —                   | —                                                |

Relation 不是同一字段的第二种 `Modify` 编码，而是把表中 Relation 列和
`CanonicalRelationTables` 独立规范化为 A.5 登记的 stable-identity tuple，再严格执行 A.5 的
set/scalar/domain/occurrence 配对算法。LFSD v1 只允许 role `1..18,20..27`；其中 role
9、14、15、16 直接来自四张规范关系表，其余来自表中 Relation 列。一个复合关系的多个字段
必须原子投影，例如 AccessRule `targetKind/targetOrdinal`；`StaticRoute.transitionGates` 通过
role 14..16 occurrence 闭合。`SignalPhase.states` 整个 tag 5 只属于 StaticRule，不另发 role
19 RelationChange；role 28/29 只由 GeometryChange 覆盖。没有 LFSD role 的
`RoadCorridor.referenceSection` 和 `StopLine.laneEdge` 只能按
`SemanticFieldValueV1::StableRefV1` 报告，不得伪造未知 role 或比较 raw ordinal。

`ManeuverGate.transitionIndex`（tag 4）是字段级规范标量，不是 role 10 的 localIndex：它变化时
必须产生一个 Entity `Modify`，即使该 gate 在 `ManeuverPath.maneuverGates` 中的零基位置没有
改变。role 10 仍只投影 vector membership/order；只有规范 vector 位置变化才另外产生
Relation `Move`，不得用该位置替代或吞掉 transitionIndex 的 before/after 值。

本表末列中的 Identity 语义锚项必须解析为同一 StableId 已验证前像中的相同语义值；
artifact-local ordinal
的数值可以因其他实体插入而重编号，不能单独构成身份变化或字段 `Modify`。ManeuverPath tag 4
的 entry/exit 元素也必须继续解析到已验证前像中的相同 StableId，但不妨碍中间 edge 变化
投影为 Relation。任一 Identity v1 前像变化且正常得到不同 StableId 时，只允许旧实体完整
RowV1 的 Entity `Remove` 与新实体完整 RowV1 的 Entity `Add`，不建立旧、新 ID 配对；如果
变化后的前像碰撞到相同截断 StableId，则必须由上述跨修订检查拒绝，不能按 retained 或
Remove/Add 处理。相应规范关系 tuple 的 Add/Remove 仍由关系集合差独立且唯一地产生。

字段级 before/after `Bytes` 使用闭合的 `SemanticFieldValueV1`，不得直接比较会因表内插入而
漂移的 raw ordinal。`StableRefV1` 精确为
`referencedEntityKind:u16 little-endian || referencedStableId:16 bytes`。投影规则只有：

| entity.field                        | `SemanticFieldValueV1`                                                     |
| ----------------------------------- | -------------------------------------------------------------------------- |
| `RoadCorridor.referenceSection`     | 所引 `RoadSection` 的 `StableRefV1`                                        |
| `StopLine.laneEdge`                 | 所引 `LaneEdge` 的 `StableRefV1`                                           |
| `WaitingZone.entryGate/releaseGate` | 各自所引 `ManeuverGate` 的 `StableRefV1`                                   |
| `SignalPhase.states`                | `count:u32`，随后每行 `SignalGroup StableRefV1 || aspect:u8`，保持规范行序 |
| 其他允许字段 `Modify`               | 附录 A 对应 LFCA field value 的 exact bytes                                |

表中 ordinal-bearing 字段之外的 ordinal 全部属于 Relation 列或 Identity 语义锚，不进入字段
`Modify`。未来新增 ordinal-bearing 字段必须先扩展本闭合表；不得默认退化为 raw `u32` bytes。
因此仅 ordinal 重编号时 before/after 投影相同且不产生记录，真实目标、状态集合或 aspect 变化
仍能从两端 LFCA 独立重算。

`ParticipantClass.subtreeEnter/subtreeExit`（tag 5/6）是从 role 24 parent forest 和
typedOrdinal 顺序唯一重算的验证缓存，不生成字段 `Modify`；插入无关类别导致 interval 整体
平移时不得制造伪差异。父关系变化由 role 24 表达，depth（tag 4）变化按 Entity Modify 表达，
两端 interval 仍必须分别通过 A.1 的层级闭包验证。

StaticRule 只有 `Modify`；实体出现/消失始终只由 Entity Add/Remove 表达，Geometry
Add/Remove 只用于两张 geometry 表。除上述 tag 5/6 derived cache 外，相同 StableId 的每个
变化字段必须按上表恰好报告一次；Relation tuple 集合也必须完整投影。把字段放入另一
class、重复字段记录、遗漏字段记录或遗漏/额外关系 tuple 都失败关闭。

Genesis 把 base 的实体、关系和 geometry 集合都视为空：`EntityChanges` 必须对目标每个稳定
实体恰有一行 `Add`，`RelationChanges` 必须对目标每个可 diff 的 A.5 relation tuple 恰有一行
`Add`，`GeometryChanges` 必须对目标每个 LaneEdgeGeometry 和 FacilityBandGeometry 恰有一行
`Add`。Genesis 禁止任何 Remove/Modify/Move/Reconnect，`StaticRuleChanges` 必须为空，因为完整
目标 entity RowV1 已携带其规则字段；`SpatialConfigurationChanges` 仍按下文恰有一行
`Initialize`。#298 历史完整接受域要求从 target LFCA 重建这三个目标集合并逐行双射；#299
当前 checker 明确不执行该 diff 完备性复验。headless target 的两张 geometry 表和
`GeometryChanges` 才同时为空。

GeometryChange 的 before/after `Bytes` 使用下列 `CanonicalGeometryValueV1`，不能放入完整
LFCA geometry RowV1：

```text
canonicalFrame:StableRefV1
projectedFieldCount:u16 little-endian
for each projected field in required-tag order:
  fieldTag:u16 little-endian
  valueByteLength:u32 little-endian
  valueBytes[valueByteLength]
```

LaneEdgeGeometry 的 projected fields 精确为 tag `3,4,5,6`，FacilityBandGeometry 精确为 tag
`3,4`；valueBytes 是对应 LFCA field value 的 exact bytes。GeometryChange 已在共同字段携带
subject kind/StableId，因此 geometry row tag 1 subject ordinal 必须排除；tag 2 frame ordinal
必须替换为所引 CanonicalFrame 的 `StableRefV1`。这样 subject/frame 仅因表内重编号而变化时
投影保持相同，真实 frame 或几何值变化仍唯一可见。Add/Remove/Modify 三种 change kind 都按
所在一侧从 LFCA 重算同一种投影。

字段级 Bytes 是上述 `SemanticFieldValueV1`，不含 12-byte FieldV1 header；整行 Add/Remove
的 Bytes 是完整 LFCA entity RowV1；Geometry 实体 Bytes 是上述
`CanonicalGeometryValueV1`。所有 before bytes 必须逐字节等于从 base 重算的投影，after
bytes 必须逐字节等于从 target 重算的投影。
Entity Add/Remove 的 `subjectStableId`、完整 RowV1 与两端 `CanonicalIdentity` 必须一致；任何
编译器内部 lineage、source location、raw ordinal 或相似度都不得改变 base-only/target-only
集合差结果。

规范排序键如下；StableId128/Bytes 使用无符号逐字节字典序，整数使用无符号数值序。表中
斜线分支由 change kind 唯一选择，因此没有 absent-value ordering：

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

每张 change table 先按 `changeKind` 数值，再按本表对应 tuple 严格递增；完全相同的键、矩阵外
字段、错误 payload 形状或不能与两端 LFCA 独立重算结果一一对应的行都失败关闭。

`SpatialConfigurationChanges(0x0006)` 只有
`SpatialConfigurationChange(tableKind=0x0001)`，字段为
`1:changeKind:u8:R, 2:beforeSpatialPresence:Bytes:O,
3:afterSpatialPresence:Bytes:R`。`changeKind` 是闭合枚举：`0=Initialize, 1=Modify`：

- Genesis 必须且只能有一行 `Initialize`，before 缺失，after 是目标 LFCA 的完整
  `SpatialPresence` RowV1；
- Artifact 两端 `SpatialPresence` RowV1 相同则该表必须为空；不同时必须且只能有一行
  `Modify`，before/after 分别是 base/target 的完整 RowV1 且逐字节不同；
- 其他 change kind、before presence 组合、额外字段、重复行或与绑定 artifact 独立重算
  不一致都失败关闭。

该表的排序键是 singleton，不使用或伪造 `entityKind`；它唯一表达
headless/spatial presence 与闭合 direction profile code 的全局变化。逐 geometry
`directionProfileApplies` 的变化由 `GeometryChange` 表达；accuracy profile 只改变非语义
CompilerProvenance 和 artifact exact binding，不得产生空间语义变化记录。

### A.4 LFCP v1 table registry（#298 历史实现；已由 ADR 0024 取代）

四个节各只有一张 `tableKind=0x0001`、一行 singleton 表：

| sectionKind | 表名                     | 字段                                                                                                                                                                                                    |
| ----------- | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0x0001`    | CanonicalArtifactBinding | `1:canonicalArtifactFormatVersion:u16:R, 2:networkRevisionDerivationVersion:u16:R, 3:networkRevision:Sha256:R, 4:canonicalArtifactDigest:Sha256:R, 5:canonicalArtifactByteLength:u64:R`                 |
| `0x0002`    | SourceMapBinding         | `1:sourceMapFormatVersion:u16:R, 2:sourceMapDigest:Sha256:R, 3:sourceMapByteLength:u64:R, 4:compilerBuildId:Utf8:R, 5:sourceCollectionDigestVersion:u16:R, 6:sourceCollectionDigest:Sha256:R`           |
| `0x0003`    | ValidationReceiptBinding | `1:validationReceiptFormatVersion:u16:R, 2:receiptKind:Utf8:R, 3:validatorBuildId:Utf8:R, 4:validationReceiptDigest:Sha256:R, 5:validationReceiptByteLength:u64:R`                                      |
| `0x0004`    | PublicationProvenance    | `1:publisherKind:u8:R, 2:publisherBuildId:Utf8:R, 3:artifactObjectKey:Utf8:R, 4:sourceMapObjectKey:Utf8:R, 5:receiptObjectKey:Utf8:R, 6:controlledBuildProvenance:Utf8:O, 7:controlledTimestamp:Utf8:O` |

`receiptKind` v1 必须逐字节等于 UTF-8 `canonical-publication-v1`。`publisherKind` 为
`0=LocalTool, 1=CI, 2=ReleaseService`。

LFCP 接受前必须读取三个 digest-bound 对象并逐份核对，不能只验证 singleton 形状：

- CanonicalArtifactBinding 的 format/derivation/revision/digest/length 必须分别等于 LFCA
  preamble、ContractVersions、六节重算 revision 与 exact bytes；
- SourceMapBinding 的 format/digest/length 必须等于 LFSM preamble 与 exact bytes，三个
  compiler/source-collection 字段必须等于 LFSM SourceMapBindings；该 LFSM 又必须按 A.2
  绑定同一 LFCA；
- ValidationReceiptBinding 的 format/digest/length、receiptKind 与 validatorBuildId 必须
  等于 #299 收据格式冻结后从 exact receipt 独立读取的值。若 kind 为
  `canonical-publication-v1`，还必须解析其 `subjectBindings` 并要求恰有一个 artifact subject
  和一个 source-map subject：前者的 format/derivation version、network revision、digest 与
  exact length 必须逐值等于 CanonicalArtifactBinding，后者的 format version、digest 与 exact
  length 必须逐值等于 SourceMapBinding；不得携带 base/target、image、diff 或另一对象的 subject。
  这些是跨对象语义比较项；#299 仍独自冻结 receipt 内部 wire tag、排序与 checkResults。在
  subjectBindings 全部匹配之前不得构造 LFCP。

三个 object key 都是内容寻址逻辑键，不是调用方路径或不透明 locator，精确为 ASCII
`sha256/` 后接相应 binding digest 的 64 个 lowercase hex 字符，总长 71 bytes：

```text
artifactObjectKey  = "sha256/" || hexLower(canonicalArtifactDigest)
sourceMapObjectKey = "sha256/" || hexLower(sourceMapDigest)
receiptObjectKey   = "sha256/" || hexLower(validationReceiptDigest)
```

空值、大写 hex、额外 separator、`..`、绝对路径、另一摘要或解析后 length/bytes 不等都在
认证 manifest 提交前失败。本地安装器只把该逻辑键映射到已配置发布根下的
content-addressed no-replace winner，不得让 key 选择根目录或重解释路径。对象 key 和
provenance 仍不构成信任锚；
真实性由 LFCP exact bytes 外部的认证 manifest/指针提供。

### A.5 RoadEditing 与来源关系闭合代码

以下代码只服务 LFSM v1，不继承 Rust enum 判别值：

- `roadEditingRelationKind 0..12` 依次为 `Import, CurveSegment, CorridorElement,
  RoadSectionAuthoringLane, LaneEdgeSuccessor, JunctionApproachEdge,
  JunctionInternalEdge, ManeuverPathInternalEdge, SignalControllerGroup,
  SignalControllerPhase, SignalPhaseState, AccessRuleParticipantClass, StaticRouteEdge`；
- `structKind 0..3` 依次为 `Digest256, OptionalU64, Vec3F64, LinearWidthProfile`；
- `unionKind 0` 为 `CurveSegmentGeometry`；
- `tableKind 0..35` 依次为 `RoadEditingSource, ModuleHeader, Provenance, LineSegment,
  CubicBezierSegment, CurveSegment, CurveProgram, RoadAlignment, CorridorElement,
  RoadCorridor, RoadSection, AuthoringLane, LaneEdge, Junction, Movement, ManeuverPath,
  ManeuverGate, WaitingZone, StopLine, SignalGroup, SignalController, SignalPhaseState,
  SignalPhase, ParkingArea, ParkingLaneAnchor, ParkingSpaceGeometry, ParkingSpace,
  LaneGroup, FacilityBand, ParticipantClass, AccessRegulation, AccessRule,
  IidmVehicleProfile, VehicleProfile, StaticRoute, CanonicalFrame`。

每个 table container 的合法 `TableField.memberCode` 是下列闭区间；区间外的值失败关闭：

```text
 0 RoadEditingSource       0..26    18 StopLine             0..2
 1 ModuleHeader            0..3     19 SignalGroup          0..1
 2 Provenance              0..5     20 SignalController     0..4
 3 LineSegment             0..0     21 SignalPhaseState     0..1
 4 CubicBezierSegment      0..2     22 SignalPhase          0..4
 5 CurveSegment            0..2     23 ParkingArea          0..1
 6 CurveProgram            0..1     24 ParkingLaneAnchor    0..1
 7 RoadAlignment           0..3     25 ParkingSpaceGeometry 0..3
 8 CorridorElement         0..1     26 ParkingSpace         0..5
 9 RoadCorridor            0..8     27 LaneGroup            0..2
10 RoadSection             0..4     28 FacilityBand         0..4
11 AuthoringLane           0..6     29 ParticipantClass     0..2
12 LaneEdge                0..4     30 AccessRegulation     0..2
13 Junction                0..3     31 AccessRule           0..7
14 Movement                0..4     32 IidmVehicleProfile   0..6
15 ManeuverPath            0..5     33 VehicleProfile       0..3
16 ManeuverGate            0..6     34 StaticRoute          0..2
17 WaitingZone             0..5     35 CanonicalFrame       0..1
```

`StructMember` 的闭合成员表为：`Digest256(0): member 0`、
`OptionalU64(1): member 0`、`Vec3F64(2): members 0..2`、
`LinearWidthProfile(3): members 0..1`。`UnionVariant` 只有
`CurveSegmentGeometry(0)` 的 `1=LineSegment` 与 `2=CubicBezierSegment`；判别值 `0` 或其他
值不构成 property step。

合法 property path 不是任意合法 step 的笛卡尔积，而精确由下列闭合形状构成：

1. 任一上表登记的单步 `TableField(table, field)`；
2. 下列直接 table-to-struct 边，后接该 struct 的任一登记成员：
   `Provenance.2->Digest256`、`Provenance.3->Digest256`、
   `Provenance.4->OptionalU64`、`CurveProgram.0->Vec3F64`、
   `LineSegment.0->Vec3F64`、`AuthoringLane.3->LinearWidthProfile`、
   `FacilityBand.2->LinearWidthProfile`，以及
   `CubicBezierSegment.{0,1,2}->Vec3F64`；
3. 下列 table-to-table 边，后接目标 table 的任一登记 field：
   `ModuleHeader.3->Provenance`、`RoadAlignment.2->CurveProgram`、
   `LaneEdge.3->CurveProgram`、`RoadCorridor.7->CorridorElement`、
   `SignalPhase.2->SignalPhaseState`、
   `ParkingSpace.{2,3}->ParkingLaneAnchor`、
   `ParkingSpace.4->ParkingSpaceGeometry`、
   `AccessRule.5->AccessRegulation`、`VehicleProfile.2->IidmVehicleProfile`；
4. 第 3 项中 `RoadAlignment/LaneEdge -> CurveProgram.0` 可以再接
   `Vec3F64` 任一成员；`ModuleHeader -> Provenance.{2,3,4}` 可以再接其第 2 项登记的
   struct member；
5. 唯一四步形状为 `CurveSegment.1 ->
   UnionVariant(CurveSegmentGeometry, 1) -> LineSegment.0 -> Vec3F64.member`，或把判别值
   换成 `2`，再接 `CubicBezierSegment.{0,1,2} -> Vec3F64.member`。

除上述 1..=4 步完整序列外，任何未被条目自身登记为完整序列的前缀、拼接、
container/member 不匹配或未知值都失败关闭；验证只依赖本登记，不读取 FlatBuffers schema、
生成代码或 Rust 枚举布局。

路径形状合法还不代表它属于该 SourceLocation。每个 property path 的第一步必须是
`TableField`，并从位置 subject 的闭合 root container 可达：ModuleHeader subject 禁止
property；RoadAlignment subject 的 root 是 RoadAlignment；Declaration subject 的 root 是其
`entityKind` 同名 table。OwnerLocal 必须存在 property，并精确满足下表；Address owner 还要
按 StableEntitySource 的同一 identity/address 算法核对 namespace、kind、owner keys 与 local
key，occurrence ordinal 必须等于该位置声明的值：

| roadEditingRelationKind    | 必需 owner                | occurrence            | root container   |
| -------------------------- | ------------------------- | --------------------- | ---------------- |
| Import                     | ModuleHeader              | CanonicalSetOrdinal   | ModuleHeader     |
| CurveSegment               | RoadAlignment 或 LaneEdge | OrderedProductOrdinal | CurveSegment     |
| CorridorElement            | RoadCorridor              | OrderedProductOrdinal | RoadCorridor     |
| RoadSectionAuthoringLane   | RoadSection               | OrderedProductOrdinal | RoadSection      |
| LaneEdgeSuccessor          | LaneEdge                  | CanonicalSetOrdinal   | LaneEdge         |
| JunctionApproachEdge       | Junction                  | CanonicalSetOrdinal   | Junction         |
| JunctionInternalEdge       | Junction                  | CanonicalSetOrdinal   | Junction         |
| ManeuverPathInternalEdge   | ManeuverPath              | OrderedProductOrdinal | ManeuverPath     |
| SignalControllerGroup      | SignalController          | CanonicalSetOrdinal   | SignalController |
| SignalControllerPhase      | SignalController          | OrderedProductOrdinal | SignalController |
| SignalPhaseState           | SignalPhase               | CanonicalSetOrdinal   | SignalPhase      |
| AccessRuleParticipantClass | AccessRule                | CanonicalSetOrdinal   | AccessRule       |
| StaticRouteEdge            | StaticRoute               | OrderedProductOrdinal | StaticRoute      |

第一步 table 必须逐值等于 root container；后续步骤再按上述闭合形状逐边验证。禁止 LaneEdge
subject 携带 AccessRule root、关系 kind 借用另一 owner 的 table，或只因每一步分别出现在全局
registry 就接受不可达拼接。

编译后来源关系使用下列另一张闭合表。`localIndex` 一律是 owner 内局部下标；`vector`
表示对应 LFCA vector/RecordVector 的零基位置，`scalar` 表示字段存在时固定为 `0`，
`filtered row` 表示按相应 LFCA 表规范行键过滤 owner 后的零基位置。`set` 的位置只服务 wire
排序与来源定位，不是 LFSD Move 语义；`domain/occurrence` 的位置属于语义顺序。

| role / 名称                             | owner kind       | subject kind                  | 绑定 LFCA 唯一投影                      | localIndex / 序策略 | LFSD 投影          |
| --------------------------------------- | ---------------- | ----------------------------- | --------------------------------------- | ------------------- | ------------------ |
| `1 LaneEdgeSuccessor`                   | LaneEdge         | LaneEdge                      | `LaneEdge.successors`                   | vector / set        | Relation           |
| `2 RoadCorridorElement`                 | RoadCorridor     | RoadSection 或 FacilityBand   | `RoadCorridor.elements`（kind+ordinal） | vector / domain     | Relation           |
| `3 RoadSectionLane`                     | RoadSection      | AuthoringLane                 | `RoadSection.lanes`                     | vector / domain     | Relation           |
| `4 AuthoringLaneEdge`                   | AuthoringLane    | LaneEdge                      | `AuthoringLane.edgeChain`               | vector / domain     | Relation           |
| `5 LaneGroupMember`                     | LaneGroup        | AuthoringLane                 | `LaneGroup.members`                     | vector / domain     | Relation           |
| `6 JunctionMovement`                    | Junction         | Movement                      | `Junction.movements`                    | vector / set        | Relation           |
| `7 MovementManeuverPath`                | Movement         | ManeuverPath                  | `Movement.maneuverPaths`                | vector / set        | Relation           |
| `8 ManeuverPathEdge`                    | ManeuverPath     | LaneEdge                      | `ManeuverPath.edges`                    | vector / domain     | Relation           |
| `9 JunctionInternalEdge`                | Junction         | LaneEdge                      | `JunctionInternalEdge` owner rows       | filtered row / set  | Relation + derived |
| `10 ManeuverPathGate`                   | ManeuverPath     | ManeuverGate                  | `ManeuverPath.maneuverGates`            | vector / domain     | Relation           |
| `11 ManeuverPathWaitingZone`            | ManeuverPath     | WaitingZone                   | `ManeuverPath.waitingZones`             | vector / domain     | Relation           |
| `12 StopLineManeuverGate`               | StopLine         | ManeuverGate                  | `StopLine.maneuverGates`                | vector / set        | Relation           |
| `13 StaticRouteEdge`                    | StaticRoute      | LaneEdge                      | `StaticRoute.edges`                     | vector / occurrence | Relation           |
| `14 StaticRouteManeuverOccurrence`      | StaticRoute      | ManeuverPath                  | `RouteManeuverOccurrence` owner rows    | occurrenceIndex     | Relation + derived |
| `15 StaticRouteGateOccurrence`          | StaticRoute      | ManeuverGate                  | `RouteGateOccurrence` owner rows        | occurrenceIndex     | Relation + derived |
| `16 StaticRouteWaitingZoneOccurrence`   | StaticRoute      | WaitingZone                   | `RouteWaitingZoneOccurrence` owner rows | occurrenceIndex     | Relation + derived |
| `17 SignalControllerGroup`              | SignalController | SignalGroup                   | `SignalController.signalGroups`         | vector / set        | Relation           |
| `18 SignalControllerPhase`              | SignalController | SignalPhase                   | `SignalController.phases`               | vector / domain     | Relation           |
| `19 SignalPhaseState`                   | SignalPhase      | SignalGroup                   | `SignalPhase.states.signalGroup`        | vector / set        | StaticRule only    |
| `20 ManeuverGateSignalGroup`            | ManeuverGate     | SignalGroup                   | Group 时的 `ManeuverGate.signalGroup`   | scalar              | Relation           |
| `21 ParkingSpaceArea`                   | ParkingSpace     | ParkingArea                   | 可选 `ParkingSpace.parkingArea`         | scalar              | Relation           |
| `22 ParkingSpaceEntry`                  | ParkingSpace     | LaneEdge                      | `ParkingSpace.entryLaneEdge`            | scalar              | Relation           |
| `23 ParkingSpaceExit`                   | ParkingSpace     | LaneEdge                      | `ParkingSpace.exitLaneEdge`             | scalar              | Relation           |
| `24 ParticipantClassExtends`            | ParticipantClass | ParticipantClass              | 可选 `ParticipantClass.parent`          | scalar              | Relation           |
| `25 AccessRuleTarget`                   | AccessRule       | targetKind 指定的四种实体之一 | `AccessRule.(targetKind,targetOrdinal)` | scalar              | Relation           |
| `26 AccessRuleParticipantClass`         | AccessRule       | ParticipantClass              | `AccessRule.participantClasses`         | vector / set        | Relation           |
| `27 VehicleProfileParticipantClass`     | VehicleProfile   | ParticipantClass              | `VehicleProfile.participantClass`       | scalar              | Relation           |
| `28 CanonicalFrameLaneEdgeGeometry`     | CanonicalFrame   | LaneEdge                      | `LaneEdgeGeometry` owner rows           | filtered row / set  | Geometry only      |
| `29 CanonicalFrameFacilityBandGeometry` | CanonicalFrame   | FacilityBand                  | `FacilityBandGeometry` owner rows       | filtered row / set  | Geometry only      |

role 1 只投影 A.1 过滤后的 `LaneEdge.successors`；compiler-private source view 中对应被过滤
internal-touching pair 的来源行不得进入 LFSM，也不得占用 localIndex。保留项在过滤后按 target
typed ordinal 重新从零编号并与 LFCA tuple 一致；两种 frontend 都不得为被过滤 pair 产生
role 1 行，但其余合法来源位置仍可按 LFSM 语义不同。

RoadEditing primary-source projection 使用以下两个闭合算子，不解析或复制 frontend schema：

- `D(E, path)` 要求 `Declaration` subject；它的 namespace、entity kind、owner key chain 与
  local key 必须用 A.2 `StableEntitySource` 的同一 CanonicalIdentity/address 算法精确绑定到
  tuple 中的实体 `E`，property path 必须逐步等于 `path`；`-` 表示 property 缺失；
- `O(E, relation, occurrence, path)` 要求 `OwnerLocal` subject；其 Address owner 必须用同一
  算法绑定到 `E`，relation、occurrence kind/value 与 property path 必须逐值相等。下式中的
  `owner`、`subject` 都指当前 A.5 tuple 反解出的实体，不接受 LFSM 自报的另一实体。

```text
RoadEditing primary-source projection by sourceRelationRole:
 1 O(owner, LaneEdgeSuccessor, CanonicalSetOrdinal(localIndex), LaneEdge.2)
 2 O(owner, CorridorElement, OrderedProductOrdinal(localIndex),
     RoadCorridor.7/CorridorElement.1)
 3 D(subject, -)
 4 D(owner, AuthoringLane.1)
 5 D(subject, AuthoringLane.4)
 6 D(subject, Movement.1)
 7 D(subject, ManeuverPath.1)
 8 localIndex == 0:
       D(owner, ManeuverPath.2)
   0 < localIndex < edgeCount(owner) - 1:
       O(owner, ManeuverPathInternalEdge,
         OrderedProductOrdinal(localIndex - 1), ManeuverPath.3)
   localIndex == edgeCount(owner) - 1:
       D(owner, ManeuverPath.4)
 9 let P be the lowest-StableId ManeuverPath under owner whose internal-edge
     sequence contains subject, and j be subject's first zero-based index in that
     internal-edge sequence when the same edge repeats:
       O(P, ManeuverPathInternalEdge, OrderedProductOrdinal(j), ManeuverPath.3)
10 D(subject, ManeuverGate.1)
11 D(subject, WaitingZone.1)
12 D(subject, ManeuverGate.3)
13 O(owner, StaticRouteEdge, OrderedProductOrdinal(localIndex), StaticRoute.1)
14 O(owner, StaticRouteEdge,
     OrderedProductOrdinal(RouteManeuverOccurrence.entryRouteEdgeIndex), StaticRoute.1)
15 O(owner, StaticRouteEdge,
     OrderedProductOrdinal(RouteGateOccurrence.fromRouteEdgeIndex), StaticRoute.1)
16 O(owner, StaticRouteEdge,
     OrderedProductOrdinal(RouteWaitingZoneOccurrence.entryRouteEdgeIndex), StaticRoute.1)
17 O(owner, SignalControllerGroup, CanonicalSetOrdinal(localIndex), SignalController.2)
18 O(owner, SignalControllerPhase, OrderedProductOrdinal(localIndex), SignalController.3)
19 O(owner, SignalPhaseState, CanonicalSetOrdinal(localIndex),
     SignalPhase.2/SignalPhaseState.0)
20 D(owner, ManeuverGate.5)
21 D(owner, ParkingSpace.1)
22 D(owner, ParkingSpace.2/ParkingLaneAnchor.0)
23 D(owner, ParkingSpace.3/ParkingLaneAnchor.0)
24 D(owner, ParticipantClass.1)
25 D(owner, AccessRule.2)
26 O(owner, AccessRuleParticipantClass, CanonicalSetOrdinal(localIndex), AccessRule.4)
27 D(owner, VehicleProfile.1)
28 D(subject, -)
29 D(subject, -)
```

role 9 的 `P` 和 `j` 只从绑定 LFCA 的 Junction/Movement/ManeuverPath/edge 关系及稳定身份
重算；若不存在唯一最低 StableId 路径或 subject 不是其 internal edge，则失败。role 14..16
使用同一绑定 LFCA occurrence 行中的 edge index，不使用 LFSM 自报 index。SyntheticDsl 的
Text 位置没有结构化 declaration address，因此只执行 A.2 的模块/文档/行列闭合；不得把它
伪装为可通过上述投影的 RoadEditing 位置。

每个投影 tuple 精确为
`(ownerEntityKind, ownerStableId, role, localIndex, subjectEntityKind, subjectStableId)`；subject
ordinal 必须先经绑定 LFCA `CanonicalIdentity` 解析为全局唯一 StableId，不能进入 tuple。LFSM
按全部 29 role 执行 A.2 的 OwnerLocal 双射；role 9/14/15/16 再执行 derived 行双射，role
28/29 再执行 source-range exact projection，且仅在迭代器非空时执行 point-range 全覆盖。
LFSD 只接受表中标为 Relation 的 role：

- `set` 按 `(subjectKind, subjectStableId)` 比较成员；保留成员仅因另一成员插入导致 canonical
  position 改变时不产生 Move，Add/Remove 仍携带所在一侧重算出的 localIndex；
- `scalar` 缺失/出现产生 Remove/Add，两端都存在但 subject 改变时产生 Reconnect，index 恒为
  `0`；
- `domain/occurrence` 对相同 subject 按各自 localIndex 递增分配零基同值 occurrence rank，并
  只配对两端相同 rank；配对项 index 改变产生 Move，未配对项产生 Remove/Add。这样重复
  `StaticRoute.edges` 或 route occurrence 也只有一种配对；
- role 19 的 group StableId 与 aspect 一起进入 `SignalPhase.states` 的
  `SemanticFieldValueV1`，role 28/29 只进入 GeometryChange，不得重复生成 RelationChange。

RoadEditing relation 与 source relation role 不得互换、按数值强制转换或共享未知值处理。

任何已有 Rust struct/view、测试 helper、serializer、JSON 文件或生成代码都不是本附录的
替代事实源。G2 可以把这些常量手工实现为有类型 Rust registry，但不得引入另一份需要与
本文双向同步的 schema；若实现发现本登记无法表达已接受 LIR，必须返回 #298 G1 修改本文。
