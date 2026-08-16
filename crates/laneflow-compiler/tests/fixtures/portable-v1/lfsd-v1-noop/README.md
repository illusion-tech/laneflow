# `LFSD-V1-NOOP` 固定对象

本目录固定 #298 G2 的 `LFSD-V1-NOOP` 已知向量。它复用相邻
[`LFCA-V1-FULL-SPATIAL`](../lfca-v1-full-spatial/README.md) 的完整空间语义输入、
compiler provenance 与 `expected.lfca`，把同一不可变 LFCA 同时作为 Artifact base 和
target。该向量对应验证矩阵 `DIFF-002`：变化集合为空，但 base/target binding 必须完整。

## 输入与边界

- base：`../lfca-v1-full-spatial/expected.lfca`；
- target：用同一 full-spatial 语义输入和
  `laneflow-fixture-298-full-spatial-v1` provenance 重新发射；
- base kind：`Artifact(1)`，不是 Genesis；
- LFCA/LFSM 必须继续逐字节等于相邻 fixture，只有 LFSD 从 Genesis 形状变为 Artifact
  no-op 形状；
- 本向量不声明独立验证、可信发布或迁移授权。

## 固定对象

| 文件            | exact length | SHA-256 / object key                                                      |
| --------------- | -----------: | ------------------------------------------------------------------------- |
| `expected.lfsd` |          569 | `sha256/5d72d97e935aa2ecddf2cc1c3cc6af033b7c115166d78de9e95526bc78d7f818` |

base 与 target 共用：

- NetworkRevisionId：
  `dc1f3d54438d8ae4921dc045fd8a0c0d1a1b54363e415160445b27d427dce901`；
- LFCA digest：
  `87e1789dd94f664e2506c3a1f0faac1a86c647c14c3ccdafb536777d273e3a50`；
- LFCA exact length：`14,649` bytes；
- network revision derivation version：`1`。

## 目录与变化表复核

公共前导为 `magic=LFSD`、`formatVersion=1`、`headerByteLength=32`、`flags=0`、
`sectionCount=6`、`sectionDirectoryOffset=32`、`objectByteLength=569`。

| section kind | byte offset | byte length | change row count |
| ------------ | ----------: | ----------: | ---------------: |
| `0x0001`     |         176 |         293 |              N/A |
| `0x0002`     |         469 |          20 |                0 |
| `0x0003`     |         489 |          20 |                0 |
| `0x0004`     |         509 |          20 |                0 |
| `0x0005`     |         529 |          20 |                0 |
| `0x0006`     |         549 |          20 |                0 |

`SemanticDiffBindings` 的 tag `1..9` 依次固定为 Artifact、base version/revision/digest/
length、target version/revision/digest/length；两端 revision、digest 和 length 逐值相等。
Entity、Relation、Geometry、StaticRule 与 SpatialConfiguration 五张 change table 均为
规范空表，不省略 section/table/binding。

## 复核与更新规则

固定字节由 production emitter 一次性 materialize，提交态不保留写文件入口。随后使用只读
PowerShell byte reader 直接检查前导、目录、FieldV1 binding 和 change table rowCount，并以
`Get-FileHash` 独立计算摘要；该 reader 没有调用 production emitter，也不作为仓库工具保留。

测试只通过 `include_bytes!` 读取固定对象，不会在测试运行时生成或覆盖它。更新本文件或
`expected.lfsd` 时，PR 必须同时说明语义输入变化、格式版本判断、旧向量处置和新的独立复核
结果；禁止增加 `expected.json`、JSON Schema、codegen、专用 `xtask` 或第二套永久 emitter。
