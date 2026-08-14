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
- 目标静态镜像、镜像完整性清单和有界结构校验器属于 #300；
- Traffic Runtime / Spatial 共享镜像消费路径属于 #301；
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
laneflow-validator ------> laneflow-format          # #299
laneflow-validator ------> laneflow-static-contract # #299
```

职责冻结为：

| 包                           | 拥有职责                                                                                    | 禁止拥有                                           |
| ---------------------------- | ------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| `laneflow-static-contract`   | 版本轴、摘要/长度值、`NetworkRevisionId`、实体/字段/记录种类登记、有类型序号和描述符值类型  | 文件系统、序列化器、编译器语义遍、Runtime、Spatial |
| `laneflow-format`            | 四类对象的精确线格式、受限写入器、结构预检、只读受检视图、格式错误                          | 编译器语义闭包、独立语义验证、Runtime/Spatial 构造 |
| `laneflow-compiler`          | 从同一个 `CompilationOutput` 和显式规范 provenance 发射制品/源映射/差异候选，建立来源沿袭，执行失败原子的暂存事务 | 独立验证权威、可信发布签名、运行时迁移授权         |
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

G1 必须让审阅者只依据本文的字段登记、偏移公式和少量已知向量人工重建关键字节与摘要；
若做不到，必须在 G1 内补齐规范，不能把某个 Rust 库版本、生成器或自举脚本提升为隐式
事实源。production emitter 的逐字节实现与 #299 独立验证器属于 G2 及后继议题，不为 G1
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
`ArtifactClaims.declaredNetworkRevisionId`。该声明始终是不受信任输入；#299 独立
验证器必须从六个语义节独立重算并逐字节比较。相同修订对应不同规范语义载荷时以
`NetworkRevisionDigestCollision` 失败关闭；不得追加随机数、ordinal 或 suffix。

完整 artifact exact bytes 另由 SHA-256 得到 `canonicalArtifactDigest`，长度是同一字节
序列的 `u64` 精确长度。摘要与长度不嵌回自身字节；二者由外部描述符绑定。

## 5. 源映射封套 `LFSM` v1

`magic = "LFSM"`，`sourceMapFormatVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                     | 内容                                                                   |
| ------------- | ------------------------ | ---------------------------------------------------------------------- |
| `0x0001`      | `SourceMapBindings`      | 修订派生版本/值、artifact digest/length、compiler build 与来源集合摘要 |
| `0x0002`      | `SourceModules`          | 依赖优先模块、来源文档、闭合位置池、frontend/import provenance         |
| `0x0003`      | `StableEntitySources`    | `(entityKind, StableId128, typedOrdinal)` 与 owning/contributing 位置  |
| `0x0004`      | `OwnerLocalSources`      | owner StableId128、typed role、`localIndex`、来源位置与空间点范围     |
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

记录先按来源模块/文档规范顺序，再按实体种类、StableId128、typed ordinal、role 与
localIndex 排序；位置破同值先按 `Text=0/RoadEditing=1`，`Text` 比较起止行列，
`RoadEditing` 比较 subject kind、稳定 key bytes 或 owner-local occurrence、property step
序列与 canvas selection 的实际语义值。RoadEditing context 的内部分配 ordinal、物理
vector index 和 byte offset 不得成为成功 LFSM 的规范顺序。

`sourceMapDigest` 是完整 `LFSM` exact bytes 的 SHA-256；`sourceMapByteLength` 是相同
字节的精确 `u64` 长度。封套不嵌入自身摘要。来源位置或来源沿袭变化必须改变 LFSM
exact bytes，但在规范语义未变时不得改变 `NetworkRevisionId`。

## 6. 语义差异封套 `LFSD` v1

`magic = "LFSD"`，`semanticDiffFormatVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                     | 内容                                                       |
| ------------- | ------------------------ | ---------------------------------------------------------- |
| `0x0001`      | `SemanticDiffBindings`   | base/genesis 与 target 的 revision、artifact digest/length |
| `0x0002`      | `EntityChanges`          | 稳定实体 add/remove 与规范字段语义变化                     |
| `0x0003`      | `RelationChanges`        | owner/member、topology reconnect、出现项和 localIndex 变化 |
| `0x0004`      | `GeometryChanges`        | 规范几何、长度和容差显著变化                               |
| `0x0005`      | `StaticRuleChanges`      | Gate/Waiting/Signal/Access 等行为变化                      |
| `0x0006`      | `IdentityClosureChanges` | 稳定标识改变及其父锚/字段原因                              |
| `0x0007`      | `SpatialConfigurationChanges` | headless/spatial presence 与闭合几何配置档变化        |

LFSD v1 的组合视图如下。该图只展开已由 §3.3 定义的公共前导、目录和变长节，不引入
第二种容器；第一节 wire offset 固定为 `0x00c8`（`32 + 7 * 24`）。

```text
    +---------------------------------------------------------------+
    |           LFSD ObjectPreambleV1 (32 bytes; SC == 7)           |
    +---------------------------------------------------------------+
    |            SectionDirectoryEntryV1[7] (168 bytes)             |
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
    :        IdentityClosureChanges (0x0006; variable bytes)        :
    :                                                               |
    +---------------------------------------------------------------+
    |                                                               :
    :      SpatialConfigurationChanges (0x0007; variable bytes)     :
    :                                                               |
    +---------------------------------------------------------------+
```

`baseBindingKind` 是封闭枚举：`0=Genesis`、`1=Artifact`。Genesis 必须把所有 base
版本、修订、digest 和 length 字节规范为零，并把目标的所有稳定实体和 owner-local
序列报告为新增，同时生成一条空间配置初始化记录；`Artifact` 禁止任何零占位。target
永远必须是具体 artifact。

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
绑定值。结构预检先验证该闭合形状；#299 再从 base/target 六个语义节重算 revision，
并对 exact artifact bytes 重算 digest/length 后逐项比较。

`baseBindingKind=Artifact` 时，base 与 target 的 `ContractVersions` singleton 和
`ExecutionContract` singleton 必须分别逐字段相等；任一语义 contract/version 轴变化都以
`UnsupportedSemanticContractTransition` 拒绝，且不得产生 LFSD 候选。v1 不用空变化集或
任一实体 change class 隐藏无法表示的 contract 迁移；Genesis 没有 base，因此只要求
target 的两行是 v1 支持值。

五张 entity-scoped change table 分别按附录 A.3 的每-kind 规范键严格排序；该键只使用该
kind 必需字段，不定义“缺失 optional 值如何排序”。完全相同的键失败关闭。重复关系值由
required before/after `localIndex` 进入相应键来破同值；全局空间配置由独立 singleton
change table 表达。相同 base/target 允许产生合法的空变化集合，但仍保留完整绑定。目标
静态镜像的 `staticImageLayoutVersion/staticImageProfileId`-only 变化不进入 LFSD，也不得
伪装成语义变化；这不包括已经进入 LFCA 语义的几何配置档。

编译器可以从受结构预检的 base view 生成诊断性差异，但 #299 必须从两份独立通过语义
验证的 artifact 重算或逐项验证 LFSD；#302 的可信切换描述符再绑定 LFSD digest/length。
LFSD 自身永远不授予迁移权限。

## 7. 规范发布描述符 `LFCP` v1

`magic = "LFCP"`，`canonicalPublicationDescriptorVersion = 1`。v1 精确包含：

| `sectionKind` | 名称                       | 内容                                                                       |
| ------------- | -------------------------- | -------------------------------------------------------------------------- |
| `0x0001`      | `CanonicalArtifactBinding` | format/revision 版本、revision、artifact digest/length                     |
| `0x0002`      | `SourceMapBinding`         | source-map version、digest/length、compiler build、来源集合摘要            |
| `0x0003`      | `ValidationReceiptBinding` | receipt format、`canonical-publication-v1`、validator build、digest/length |
| `0x0004`      | `PublicationProvenance`    | publisher kind/build、immutable object keys、受控时间/CI provenance        |

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
                                   | exact bytes + computed bindings       |
                                   +-------------------+-------------------+
                                                       |
                                                       v
                                   +---------------------------------------+
                                   | immutable object installation         |
                                   | create_new + existing-byte comparison |
                                   +-------------------+-------------------+
                                                       |
                                                       v
                                   +---------------------------------------+
                                   | #299 independent validation           |
                                   | canonical-publication-v1 receipt      |
                                   +-------------------+-------------------+
                                                       |
                                                       v
                                   +---------------------------------------+
                                   | LFCP exact bytes                      |
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

#298 把限制分成三层，不能让实现测量结果在 G2 反向定义格式契约：

1. 格式硬上限是 v1 固定的安全天花板，见下表；它来自固定宽字段可表达范围、每个对象
   的封闭节/表形状和已接受 `LF-COMP-P100-INITIAL-v2` 的资源维度，不是从 emitter
   benchmark 拟合出的性能结论；
2. 调用方运行上限必须显式提供，可以低于格式硬上限，但不得提高、禁用或用更大值绕过
   格式硬上限；默认发布路径从同次编译使用的 `CompileLimits` 推导更小预算，运行上限
   不改变 wire version；
3. 性能验收门槛与格式安全上限分别登记。G2 测量 production emitter 的成本并与既有
   产品预算比较；测量结果不能修改同一 wire version 的安全上限。

| v1 格式硬上限                            | 精确值                       | 适用检查点                                    |
| ---------------------------------------- | ---------------------------- | --------------------------------------------- |
| 单对象 exact bytes                       | `16,777,216` bytes           | transport/hash/read 前                        |
| 单节或单表 exact bytes                   | `16,777,216` bytes           | offset/length checked 后、建立 slice 前       |
| 单对象 TableV1 总数                      | LFCA `35`、LFSM `8`、LFSD `7`、LFCP `4` | 读取任一 TableV1 前；必须精确等于对象登记形状 |
| 单 TableV1 RowV1 数                      | `65,536`                     | `count * 16` 检查前                           |
| 单 RowV1 FieldV1 数                      | `17`                         | `count * 12` 检查前；具体 row registry 更严格 |
| 单 UTF-8 field bytes                     | `1,048,576` bytes            | UTF-8 验证和分配前                            |
| 单对象全部 UTF-8 value 累计 bytes        | `8,388,608` bytes            | checked 累加、驻留/复制前                     |
| 单向量 item 数                           | `65,536`                     | 内部 count 与 VBL 核对前                      |
| 单对象全部 vector value 累计 bytes       | `8,388,608` bytes            | checked 累加、分配前                          |
| `RecordVector` 内嵌深度                  | `1`                          | 读取 nested field type 前                     |
| 单 LFSM 来源位置记录数                   | `65,536`                     | 建立位置索引前                                |
| 单次 LFCA+LFSM+LFSD 候选暂存 exact bytes | `50,331,648` bytes（48 MiB） | 开始写入前保留总预算；每次增长前 checked 累加 |

固定节数同时给出精确形状：LFCA `8`、LFSM `5`、LFSD `7`、LFCP `4`。Table 总数也是
按附录 A 求和得到的精确形状，不是可由未知表填满的通用容量；`17` 是 RoadEditing
OwnerLocal SourceLocation 同时携带完整 address、三层 owner key、property 与 canvas 时可
达到的 v1 最大字段数，具体表仍必须满足自身 tag/presence matrix。一个对象中的
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

| 类别        | 向量/锚点 ID                 | 证明内容                                                           |
| ----------- | ---------------------------- | ------------------------------------------------------------------ |
| G1 摘要向量 | `REV-V1-MIN-HEADLESS`        | 六个合法最小语义节的 framing、domain separation 与 SHA-256         |
| G1 摘要向量 | `REV-V1-MIN-SPATIAL-EMPTY`   | 启用空间并写入闭合 profile code 时 revision 必须变化               |
| G1 结构锚点 | `LFCA-V1-MIN-HEADLESS`       | 最小合法无空间 artifact 的前导、八节目录和首节 offset 推导         |
| G1 结构锚点 | `LFSM-V1-MIXED-LOCATION`     | Text、无 property 的 Declaration、OwnerLocal 和可选 canvas 形状    |
| G1 结构锚点 | `LFSD-V1-GENESIS-BINDING`    | Genesis 四个 base 零值和完整 target binding                        |
| G1 结构锚点 | `LFCP-V1-MIN-BINDINGS`       | artifact/source-map/receipt 的外部 digest+exact length 绑定        |
| G2 固定对象 | `LFCA-V1-FULL-SPATIAL`       | 22 种实体、关系、规则、规范 f32/f64 与空间表                       |
| G2 固定对象 | `LFCA-V1-PROVENANCE-ONLY`    | 同语义不同来源沿袭：revision 相同，artifact digest 不同            |
| G2 固定对象 | `LFCA-V1-CLAIM-MISMATCH`     | 只篡改非语义 revision claim：结构预检成功、独立 revision 比较失败  |
| G2 固定对象 | `LFCA-V1-REORDER-EQUIVALENT` | 声明/集合/hash iteration 重排仍产生完全相同 bytes                  |
| G2 固定对象 | `LFCA-V1-SIGNED-ZERO`        | 合法输入 `-0.0` 在编译边界变为 `+0.0`；负零 wire 被读取器拒绝      |
| G2 固定对象 | `LFSD-V1-CHANGE-SET`         | add/remove/reconnect/geometry/global spatial/rule/identity closure |
| G2 固定对象 | `LFSD-V1-NOOP`               | 相同 base/target 的空记录但完整 binding                            |

两个 G1 revision 向量使用 §4.2 的 exact framing 和附录 A 的 section/table/row/field
编码。`REV-V1-MIN-HEADLESS` 的六节依次是：所有版本值为 `1` 的 ContractVersions；
空 CanonicalIdentity；22 张空实体表；5 张空关系表；`spatialPresent=0`、两个 profile code
均为 `0` 且两张空几何表；两个版本值为 `1` 的 ExecutionContract。各节 exact length 与
SHA-256 为：

| sectionKind | exact bytes | section SHA-256                                                    |
| ----------- | ----------- | ------------------------------------------------------------------ |
| `0x0001`    | `120`       | `8682b46d765cdc7cf4e880dbf1dcd8d046d6ca82990d57cf3abc2a3568220869` |
| `0x0002`    | `20`        | `3a85cd4b4d295cdd6cfe6ea3cb119b7c59f1addcc36faf58c33809f958191c7e` |
| `0x0003`    | `356`       | `54975e3435099f8ac2f6b6ec53e3bf68104d236da4a840318e9d0486a46e0f6e` |
| `0x0004`    | `84`        | `041fb436600f0bd293d9a9a78bb1367144e03e51ecfafca712bbc4dedb67dc19` |
| `0x0005`    | `107`       | `a7a3e939e0b7b1f32c5746beee7ae7c2e5fcbab23252cc16ae4568dc8b168eff` |
| `0x0006`    | `64`        | `79e8acf6943d876fd8ee1f45f6856c3b8285562f0c30e4d9de559317316f025f` |

加入六个 12-byte section frame 后，semantic payload 为 `823` bytes；复核算法精确为
`SHA-256("laneflow.network-revision.v1\0" || payload)`，字符串末尾包含一个 NUL byte，
得到 `2ae15313569a65ee1ed9db1b69434a34bfbf92e41884ae7cf8ba17e6dc8fbb4d`。

`REV-V1-MIN-SPATIAL-EMPTY` 保持两张几何表为空，但把 semantic payload 零基 offset
`688` 的 `spatialPresent` 从 `00` 改为 `01`，并把 offset `701` 的
`geometryAccuracyProfile` 与 offset `714` 的 `geometryDirectionProfile` 从 `00` 改为
`02`（两个 Balanced 档）。第 `0x0005` 节 SHA-256 变为
`79d6c423ffda12dc180fe721cdac0a1a7a2d055e366ee1c22d0c4ceaed555200`，payload 仍为
`823` bytes，NetworkRevisionId 变为
`e28eb7535bc8ac9a86f64f1209c39598acef0ee827079c80f2e36397d2944241`。

`LFCA-V1-MIN-HEADLESS` 的公共布局人工锚点为：前导 `32` bytes，
八项目录 `192` bytes，第一节 offset `0x00e0`；LFSM/LFSD/LFCP 的对应锚点分别为
`0x0098`、`0x00c8`、`0x0080`。完整对象 fixture 在 G2 从附录 A materialize 后固定，
不把整段二进制十六进制复制进规范正文。

确定性矩阵至少覆盖：

- Windows `x86_64-pc-windows-msvc` 本机与 Ubuntu `x86_64-unknown-linux-gnu` CI；
- single-thread 和编译器支持的所有 worker 数；
- clean process、重复运行、不同 hash seed/分配地址；
- production emitter 与 #299 独立验证器；G1 人工推导值作为不调用 emitter 的固定期望；
- 截断每个边界、单 bit 损坏、未知版本/节/表/字段、重复/乱序、gap/overlap、超限、
  length/digest/revision/source-map/base-target 错配；
- 暂存写入、flush/close、对象安装和 manifest 提交各失败点的无部分发布测试；
- 最小、P100 正式最高级和代表性大输入的发射时延、峰值暂存内存与输出大小；格式硬
  上限边界只属于安全/正确性矩阵，不作为性能 workload。

完整计划和 G2 证据占位见
`../reference/v0.10-portable-artifact-validation.md`。

## 11. G1 封闭条件与当前阻断项

本文只有在下列条件全部满足后才可转为 Accepted 并发布 #298 `G1 = Pass`：

- [ ] 附录 A 登记全部 artifact/source-map/diff/descriptor table kind、field tag、类型、
      必需性、排序键与 closed enum；
- [ ] 冻结四类对象的 magic、版本、节集合、目录、数值/文本/浮点编码和错误分类；
- [ ] 冻结 NetworkRevisionId exact payload、非语义 `ArtifactClaims` 比较规则和至少两个
      可人工复核的已知摘要；
- [ ] 冻结 Text/RoadEditing 来源位置的判别值、字段 registry、规范排序和混合来源结构
      审阅锚点；
- [ ] 冻结 artifact/source-map/receipt 与 base/target 的摘要+精确长度绑定；
- [ ] 冻结 `CompilationOutput + PortableEmissionProvenanceV1` 完整输入、候选对象不可拆分
      成功和发布提交点；
- [ ] 冻结 pre-hash 上限、结构计数上限、硬格式上限和失败原子性；
- [ ] 由非作者审阅者仅依据本文人工重建两个 revision 向量和最小对象关键 offset；
- [ ] 记录编码库/自有格式选择的安全与维护证据；
- [ ] #298 Gate Ledger 绑定当前精确提交并取得职责中立的外部 clean review，且
      Project/Issue 元数据完整。

当前 Draft 表示上述内容尚未在绑定精确提交的 #298 Gate Ledger 上全部通过职责中立审阅，
不是把字段 tag、限制值、排序破同值或 publication 原子性留给实现者选择。任一 checklist
项目未取得证据时都不得进入 G2。

## 附录 A：线格式登记表（规范）

本附录是 v1 table/field registry 的 Markdown 单一事实源。记法
`tag:name:type:presence` 中 `presence` 为 `R`（必需）或 `O`（可选且缺失即 `None`）；
未列出的 tag 不合法。每个表按“行键”严格递增，重复键失败关闭。`OrdinalVectorU32`
保持领域顺序时不得排序；标为集合时按数值严格递增且不得重复。所有 ordinal 必须解析到
同一对象相应有类型表。稳定实体的 ordinal 不仅必须从零连续，还必须按下述完整 Identity
v1 前像 bytes 排序得到；一致重编号全部引用不能把非规范顺序变成合法编码。

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
`(entityKind, typedOrdinal, stableId)` 一一对应；独立验证器必须重建前像、重算 ID、重排并
核对该映射。前像相同、排序重复、映射缺失或引用解析到不同 StableId 均失败关闭。

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

`Ascii` value 必须是 Identity v1 已准入的原始 ASCII bytes，不含长度；`StableId128` 必须
恰为 16 bytes。`entityKind 1..22` 与下列实体表同序；每种实体要求的 tag 序列精确为：

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

内嵌记录精确为：`RoadCorridor.elements = 1:elementKind:u8:R, 2:ordinal:u32:R`，其中
`0=RoadSection, 1=FacilityBand`；`SignalPhase.states = 1:signalGroup:u32:R,
2:aspect:u8:R`；`AccessRule.regulation` 必须恰有一行
`1:jurisdiction:Utf8:R, 2:version:Utf8:R, 3:source:Utf8:O`；
`StaticRoute.transitionGates` 的行数必须为 `max(edges.count-1, 0)`，每行只有
`1:maneuverGate:u32:O`。`signalControlKind` 为 `0=None, 1=Group`，且 tag 7 当且仅当值为
`1` 时存在。`targetKind` 为 `0=LaneEdge, 1=LaneGroup, 2=RoadSection, 3=ManeuverPath`；
`effect` 为 `0=Deny, 1=Allow`。`aspect` 为 `0=Red, 1=Yellow, 2=Green`，未知代码失败
关闭。

`CanonicalRelationTables(0x0004)` 精确包含：

| tableKind | 表名                       | 字段                                                                                                                                                                                                                                                    | 行键                                       |
| --------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| `0x0001`  | JunctionInternalEdge       | `1:laneEdge:u32:R, 2:junction:u32:R`                                                                                                                                                                                                                    | `laneEdge`                                 |
| `0x0002`  | RouteManeuverOccurrence    | `1:staticRoute:u32:R, 2:occurrenceIndex:u32:R, 3:maneuverPath:u32:R, 4:entryRouteEdgeIndex:u32:R, 5:exitRouteEdgeIndex:u32:R, 6:gateOccurrenceStart:u32:R, 7:gateOccurrenceCount:u32:R, 8:waitingOccurrenceStart:u32:R, 9:waitingOccurrenceCount:u32:R` | `(staticRoute, occurrenceIndex)`           |
| `0x0003`  | RouteGateOccurrence        | `1:staticRoute:u32:R, 2:occurrenceIndex:u32:R, 3:maneuverGate:u32:R, 4:maneuverOccurrenceIndex:u32:R, 5:fromRouteEdgeIndex:u32:R, 6:nextGateOccurrenceIndex:u32:O, 7:nextBoundaryRouteEdgeIndex:u32:R, 8:waitingZoneOccurrenceIndex:u32:O`              | `(staticRoute, occurrenceIndex)`           |
| `0x0004`  | RouteWaitingZoneOccurrence | `1:staticRoute:u32:R, 2:occurrenceIndex:u32:R, 3:waitingZone:u32:R, 4:maneuverOccurrenceIndex:u32:R, 5:entryGateOccurrenceIndex:u32:R, 6:releaseGateOccurrenceIndex:u32:R, 7:entryRouteEdgeIndex:u32:R, 8:releaseRouteEdgeIndex:u32:R`                  | `(staticRoute, occurrenceIndex)`           |
| `0x0005`  | StableRouteReverseIndex    | `1:entityKind:u16:R, 2:typedOrdinal:u32:R, 3:staticRoute:u32:R, 4:occurrenceIndex:u32:R`                                                                                                                                                                | `(entityKind, typedOrdinal, route, index)` |

反向索引只允许 `LaneEdge/ManeuverPath/ManeuverGate/WaitingZone` 四种 `entityKind`，且必须
与前三张 occurrence 表及 `StaticRoute.edges` 双向完全一致。

`CanonicalSpatialTables(0x0005)` 精确包含：

| tableKind | 表名                 | 字段                                                                                                                    | 行键           |
| --------- | -------------------- | ----------------------------------------------------------------------------------------------------------------------- | -------------- |
| `0x0001`  | SpatialPresence      | `1:spatialPresent:u8:R, 2:geometryAccuracyProfile:u8:R, 3:geometryDirectionProfile:u8:R`                               | singleton      |
| `0x0002`  | LaneEdgeGeometry     | `1:laneEdge:u32:R, 2:canonicalFrame:u32:R, 3:arcLengthMeters:f32:R, 4:points:RecordVector:R, 5:segments:RecordVector:R` | `laneEdge`     |
| `0x0003`  | FacilityBandGeometry | `1:facilityBand:u32:R, 2:canonicalFrame:u32:R, 3:points:RecordVector:R`                                                 | `facilityBand` |

`spatialPresent` 只允许 `0/1`。为 `0` 时两个 profile code 必须均为 `0=None`，且后两表
必须为空；为 `1` 时两个 code 都必须非零，LaneEdgeGeometry 必须与 LaneEdge 表同基数同
ordinal。`geometryAccuracyProfile` 的闭合代码为 `1=Fine2Cm, 2=Balanced5Cm,
3=Compact10Cm`；`geometryDirectionProfile` 的闭合代码为 `1=Smooth1Deg,
2=Balanced2Deg, 3=Compact5Deg`。两者必须逐值等于同次编译 LIR 的冻结配置，独立验证器
必须用它们核对空间约束；即使两种配置偶然产生相同 points/segments bytes，profile code
变化仍是规范语义变化。`points` 内嵌行为 `1:x:f32:R, 2:y:f32:R, 3:z:f32:R`；
`segments` 内嵌行为 `1:lengthMeters:f32:R, 2:cumulativeEndMeters:f32:R,
3:tangentX:f32:R, 4:tangentY:f32:R, 5:tangentZ:f32:R, 6:upX:f32:R, 7:upY:f32:R,
8:upZ:f32:R`。

`StaticExecutionConstraints(0x0006)` 只有 `ExecutionContract(0x0001)` singleton：
`1:staticExecutionContractVersion:u16:R, 2:constraintContractVersion:u16:R`。具体约束已由
前述实体、关系和空间表的规范值表达；该行禁止另存 worker 数、目标布局或运行时状态。

`CompilerProvenance(0x0007)` 只有 `CompilerProvenance(0x0001)` singleton：
`1:compilerBuildId:Utf8:R, 2:sourceCollectionDigestVersion:u16:R,
3:sourceCollectionDigest:Sha256:R, 4:compileOptionsDigest:Sha256:R, 5:emitterVersion:u16:R`。
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
- v1 没有会改变 portable bytes 的外部编译选项，因而
  `compileOptionsDigest = SHA-256("laneflow.portable-compile-options.v1\0" ||
  optionCount:u32=0)`，其中 `u32` 为小端；固定结果为
  `322682f455d06b36e9e3719f341db38f3ecda61d52c53d9d6fe3dca540eef445`。几何配置档
  属于规范 LIR/LFCA 语义，base 只影响 LFSD，limits/worker 数只控制资源，均不得另外
  塞入该摘要。

LFCA `CompilerProvenance`、LFSM `SourceMapBindings` 和 LFCP `SourceMapBinding` 中重复的
`compilerBuildId/sourceCollectionDigestVersion/sourceCollectionDigest` 必须逐字节相等；
LFCP 通过 `canonicalArtifactDigest` 间接绑定 LFCA 独有的
`compileOptionsDigest/emitterVersion`。任一不一致都是 binding mismatch；不得把这些字段
当作对象内信任锚。

### A.2 LFSM table registry

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
`1:authoringNamespaceId:Utf8:R`，按 namespace UTF-8 bytes 严格递增；模块行本身按依赖优先
顺序编号，循环或无法解析的 import 在编译阶段已经失败，不产生 LFSM。`primaryLocation`
必须解析到 SourceLocation，且其 module ordinal 必须等于当前行。SourceDocument key 必须
全局唯一，并在同一模块内按 UTF-8 bytes 严格递增；SyntheticDsl 的 `displaySource` 必须
缺失，RoadEditingSource 可以缺失或保存调用方的未认证显示来源。

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

`OwnerLocalSources(0x0004)` 精确包含：

| tableKind | 表名                       | 字段                                                                                                                                                                                                                                                                            | 行键                                                                           |
| --------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `0x0001`  | OwnerLocalSource           | `1:ownerEntityKind:u16:R, 2:ownerStableId:StableId128:R, 3:sourceRelationRole:u8:R, 4:localIndex:u32:R, 5:primaryLocation:u32:R, 6:contributingLocations:OrdinalVectorU32:R`                                                                                                    | `(ownerEntityKind, ownerStableId, sourceRelationRole, localIndex)`              |
| `0x0002`  | SpatialGeometrySourceRange | `1:ownerEntityKind:u16:R, 2:ownerStableId:StableId128:R, 3:sourceRelationRole:u8:R, 4:localIndex:u32:R, 5:pointStart:u32:R, 6:pointEndExclusive:u32:R, 7:sourceSegmentOrdinal:u32:R, 8:sourceLocation:u32:R` | `(ownerEntityKind, ownerStableId, sourceRelationRole, localIndex, pointStart)` |

`SpatialGeometrySourceRange` 只允许 `ownerEntityKind=CanonicalFrame` 以及
`sourceRelationRole=28(CanonicalFrameLaneEdgeGeometry)` 或
`29(CanonicalFrameFacilityBandGeometry)`。每行必须有同 owner/role/localIndex 的
`OwnerLocalSource` 父行；`sourceLocation` 必须解析到 SourceLocation。对每个父行，范围必须
从 `pointStart=0` 开始，按行键严格相邻、非空且无重叠，最后一个 `pointEndExclusive` 必须
等于对应 LFCA geometry `points` 的 item 数；localIndex 在该 frame/role 下按相应 LFCA
geometry 表行顺序从零编号。父行 `contributingLocations` 必须逐字节等于所有范围
`sourceLocation` 按位置语义值排序去重后的 ordinal 投影；`sourceSegmentOrdinal` 保留 authoring
segment 的原始 ordinal，不得由 point range 次序替代或重编号。这样 LFSM 能无损恢复每段
规范点区间的来源，而不是只保存扁平位置集合。

`DerivedRelationSources(0x0005)` 只有 `DerivedRelationSource(0x0001)`：
`1:ownerEntityKind:u16:R, 2:ownerStableId:StableId128:R, 3:sourceRelationRole:u8:R,
4:localIndex:u32:R, 5:derivationPassVersion:u16:R, 6:constraintVersion:u16:R,
7:sourceLocations:OrdinalVectorU32:R`，行键与 OwnerLocalSource 相同。所有位置 ordinal
必须解析到 `SourceLocation`；contributing/source vectors 是按位置语义值排序去重的集合。

### A.3 LFSD table registry

`SemanticDiffBindings(0x0001)` 只有 `SemanticDiffBindings(0x0001)` singleton，tag `1..9`
依次对应 §6 列出的九个字段，类型依次为
`u8,u16,Sha256,Sha256,u64,u16,Sha256,Sha256,u64`，全部必需。

其余五节各只有同名 `tableKind=0x0001` 的 change table。所有变化行共同以
`1:changeKind:u8:R, 2:entityKind:u16:R, 3:ownerStableId:StableId128:O,
4:subjectStableId:StableId128:O, 5:sourceRelationRole:u8:O, 6:fieldTag:u16:O,
7:beforeLocalIndex:u32:O, 8:afterLocalIndex:u32:O` 开始；各节追加字段如下：

| sectionKind | 表名                  | 追加字段                                                                                                      | `changeKind`                           |
| ----------- | --------------------- | ------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| `0x0002`    | EntityChange          | `9:beforeValue:Bytes:O, 10:afterValue:Bytes:O`                                                                | `0=Add, 1=Remove, 2=Modify`            |
| `0x0003`    | RelationChange        | `9:beforeTarget:StableId128:O, 10:afterTarget:StableId128:O`                                                  | `0=Add, 1=Remove, 2=Move, 3=Reconnect` |
| `0x0004`    | GeometryChange        | `9:beforeCanonicalValue:Bytes:O, 10:afterCanonicalValue:Bytes:O`                                              | `0=Add, 1=Remove, 2=Modify`            |
| `0x0005`    | StaticRuleChange      | `9:beforeCanonicalValue:Bytes:O, 10:afterCanonicalValue:Bytes:O`                                              | `0=Add, 1=Remove, 2=Modify`            |
| `0x0006`    | IdentityClosureChange | `9:beforeStableId:StableId128:O, 10:afterStableId:StableId128:O, 11:reasonKind:u8:R, 12:causalFieldTag:u16:O` | `0=Changed, 1=TransitivelyChanged`     |

下面是 tag 3..12 的完整存在性矩阵；`R` 表示该 change kind 必须存在，`F` 表示必须缺失。
未在相应行列出的 optional tag 同样视为 `F`，不存在实现者自选字段：

| 表/change kind                | 必需 common tags                                  | 禁止 common tags                         | payload 必需/禁止                                                |
| ----------------------------- | ------------------------------------------------- | ---------------------------------------- | ---------------------------------------------------------------- |
| Entity `Add`                  | `subjectStableId`                                 | `owner, role, fieldTag, before/afterIndex` | `afterValue:R, beforeValue:F`（完整目标 RowV1）                 |
| Entity `Remove`               | `subjectStableId`                                 | `owner, role, fieldTag, before/afterIndex` | `beforeValue:R, afterValue:F`（完整 base RowV1）                 |
| Entity `Modify`               | `subjectStableId, fieldTag`                       | `owner, role, before/afterIndex`          | `beforeValue/afterValue` 至少一个存在                            |
| Relation `Add`                | `ownerStableId, subjectStableId, role, afterIndex` | `fieldTag, beforeIndex`                   | `beforeTarget:F, afterTarget:F`                                  |
| Relation `Remove`             | `ownerStableId, subjectStableId, role, beforeIndex` | `fieldTag, afterIndex`                    | `beforeTarget:F, afterTarget:F`                                  |
| Relation `Move`               | `ownerStableId, subjectStableId, role, before/afterIndex` | `fieldTag`                         | `beforeTarget:F, afterTarget:F`                                  |
| Relation `Reconnect`          | `ownerStableId, role, before/afterIndex`          | `subjectStableId, fieldTag`               | `beforeTarget:R, afterTarget:R`                                  |
| Geometry `Add`                | `subjectStableId`                                 | `owner, role, fieldTag, before/afterIndex` | `afterCanonicalValue:R, beforeCanonicalValue:F`                 |
| Geometry `Remove`             | `subjectStableId`                                 | `owner, role, fieldTag, before/afterIndex` | `beforeCanonicalValue:R, afterCanonicalValue:F`                 |
| Geometry `Modify`             | `subjectStableId`                                 | `owner, role, fieldTag, before/afterIndex` | `beforeCanonicalValue:R, afterCanonicalValue:R`                 |
| StaticRule `Add`              | `subjectStableId`                                 | `owner, role, fieldTag, before/afterIndex` | `afterCanonicalValue:R, beforeCanonicalValue:F`（完整目标 RowV1） |
| StaticRule `Remove`           | `subjectStableId`                                 | `owner, role, fieldTag, before/afterIndex` | `beforeCanonicalValue:R, afterCanonicalValue:F`（完整 base RowV1） |
| StaticRule `Modify`           | `subjectStableId, fieldTag`                       | `owner, role, before/afterIndex`          | `beforeCanonicalValue/afterCanonicalValue` 至少一个存在          |
| Identity `Changed/TransitivelyChanged` | 无 tag 3..8                             | `owner, subject, role, fieldTag, before/afterIndex` | `beforeStableId:R, afterStableId:R, reasonKind:R, causalFieldTag:R` |

这里 `owner`、`role`、`beforeIndex`、`afterIndex` 是对应完整字段名的表内短写。所有行仍要求
公共 tag 1 `changeKind` 和 tag 2 `entityKind`；Relation 行的 `entityKind` 是 owner kind，其他
实体级行是 subject kind，且都只允许 LFCA `1..22`。Entity/StaticRule 的字段级 `Modify` 中，
`before*` 当且仅当 base field 存在，`after*` 当且仅当 target field 存在；二者至少一个存在，
若都存在则 bytes 必须不同。
这唯一表达 optional field 的出现/消失。Geometry `Modify` 两端都必须存在且不同；Relation
`Move` 的两个 index 必须不同，`Reconnect` 的两个 target 必须不同，但其 index 可以相等。
Entity `Modify.fieldTag` 只允许对应实体表 tag `3+` 的规范语义字段；`typedOrdinal/stableId`
不得伪装成普通字段变化，身份变化必须由 add/remove 与 `IdentityClosureChange` 闭合表达。
StaticRule `Modify.fieldTag` 必须是相应规则 RowV1 登记的语义字段。

字段级 Bytes 是附录 A 对应 LFCA field type 的 exact value bytes，不含 12-byte FieldV1
header；整行 Add/Remove 的 Bytes 是完整 LFCA RowV1；Geometry 实体 Bytes 是完整相应
geometry RowV1。所有 before bytes 必须逐字节等于 base 中的值，after bytes
必须逐字节等于 target 中的值。`reasonKind` 为
`0=IdentityFieldChanged, 1=ParentAnchorChanged,
2=ReferencedIdentityChanged`，`causalFieldTag` 必须是该 entity kind 的 Identity v1 required
tag。

规范排序键如下；StableId128/Bytes 使用无符号逐字节字典序，整数使用无符号数值序。表中
斜线分支由 change kind 唯一选择，因此没有 absent-value ordering：

| 表/change kind              | tag 1 `changeKind` 之后的规范排序键                                                                                     |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| Entity Add/Remove           | `(entityKind, subjectStableId)`                                                                                          |
| Entity Modify               | `(entityKind, subjectStableId, fieldTag)`                                                                                |
| Relation Add                | `(entityKind, ownerStableId, role, afterLocalIndex, subjectStableId)`                                                    |
| Relation Remove             | `(entityKind, ownerStableId, role, beforeLocalIndex, subjectStableId)`                                                   |
| Relation Move               | `(entityKind, ownerStableId, role, beforeLocalIndex, afterLocalIndex, subjectStableId)`                                  |
| Relation Reconnect          | `(entityKind, ownerStableId, role, beforeLocalIndex, afterLocalIndex, beforeTarget, afterTarget)`                        |
| Geometry Add/Remove/Modify  | `(entityKind, subjectStableId)`                                                                                          |
| StaticRule Add/Remove       | `(entityKind, subjectStableId)`                                                                                          |
| StaticRule Modify           | `(entityKind, subjectStableId, fieldTag)`                                                                                |
| Identity 两种 kind          | `(entityKind, beforeStableId, afterStableId, reasonKind, causalFieldTag)`                                                |

每张 change table 先按 `changeKind` 数值，再按本表对应 tuple 严格递增；完全相同的键、矩阵外
字段、错误 payload 形状或不能与两端 LFCA 独立重算结果一一对应的行都失败关闭。

`SpatialConfigurationChanges(0x0007)` 只有
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
headless/spatial presence 与两个闭合 geometry profile code 的全局变化。

### A.4 LFCP table registry

四个节各只有一张 `tableKind=0x0001`、一行 singleton 表：

| sectionKind | 表名                     | 字段                                                                                                                                                                                                    |
| ----------- | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0x0001`    | CanonicalArtifactBinding | `1:canonicalArtifactFormatVersion:u16:R, 2:networkRevisionDerivationVersion:u16:R, 3:networkRevision:Sha256:R, 4:canonicalArtifactDigest:Sha256:R, 5:canonicalArtifactByteLength:u64:R`                 |
| `0x0002`    | SourceMapBinding         | `1:sourceMapFormatVersion:u16:R, 2:sourceMapDigest:Sha256:R, 3:sourceMapByteLength:u64:R, 4:compilerBuildId:Utf8:R, 5:sourceCollectionDigestVersion:u16:R, 6:sourceCollectionDigest:Sha256:R`           |
| `0x0003`    | ValidationReceiptBinding | `1:validationReceiptFormatVersion:u16:R, 2:receiptKind:Utf8:R, 3:validatorBuildId:Utf8:R, 4:validationReceiptDigest:Sha256:R, 5:validationReceiptByteLength:u64:R`                                      |
| `0x0004`    | PublicationProvenance    | `1:publisherKind:u8:R, 2:publisherBuildId:Utf8:R, 3:artifactObjectKey:Utf8:R, 4:sourceMapObjectKey:Utf8:R, 5:receiptObjectKey:Utf8:R, 6:controlledBuildProvenance:Utf8:O, 7:controlledTimestamp:Utf8:O` |

`receiptKind` v1 必须逐字节等于 UTF-8 `canonical-publication-v1`。`publisherKind` 为
`0=LocalTool, 1=CI, 2=ReleaseService`。对象 key 和 provenance 不构成信任锚；真实性仍由
LFCP exact bytes 外部的认证 manifest/指针提供。

### A.5 RoadEditing 闭合代码

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

编译后来源关系使用另一张闭合表：`sourceRelationRole 1..29` 依次为
`LaneEdgeSuccessor, RoadCorridorElement, RoadSectionLane, AuthoringLaneEdge,
LaneGroupMember, JunctionMovement, MovementManeuverPath, ManeuverPathEdge,
JunctionInternalEdge, ManeuverPathGate, ManeuverPathWaitingZone, StopLineManeuverGate,
StaticRouteEdge, StaticRouteManeuverOccurrence, StaticRouteGateOccurrence,
StaticRouteWaitingZoneOccurrence, SignalControllerGroup, SignalControllerPhase,
SignalPhaseState, ManeuverGateSignalGroup, ParkingSpaceArea, ParkingSpaceEntry,
ParkingSpaceExit, ParticipantClassExtends, AccessRuleTarget, AccessRuleParticipantClass,
VehicleProfileParticipantClass, CanonicalFrameLaneEdgeGeometry,
CanonicalFrameFacilityBandGeometry`。RoadEditing relation 与 source relation role 不得互换、
按数值强制转换或共享未知值处理。

任何已有 Rust struct/view、测试 helper、serializer、JSON 文件或生成代码都不是本附录的
替代事实源。G2 可以把这些常量手工实现为有类型 Rust registry，但不得引入另一份需要与
本文双向同步的 schema；若实现发现本登记无法表达已接受 LIR，必须返回 #298 G1 修改本文。
