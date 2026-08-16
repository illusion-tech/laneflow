# `LFSD-V1-CHANGE-SET` 固定对象

本目录固定 #298 G2 的 `LFSD-V1-CHANGE-SET` 已知向量。base 与 target 使用同一
`city/portable-change-set` namespace、`portable-change-set.document` source document 和
`laneflow-fixture-298-change-set-v1` compiler provenance；LFSD 使用 `Artifact(1)` base，不是
Genesis。该向量对应验证矩阵 `DIFF-003`，并锚定 `DIFF-012` 使用的无配对身份变化规则；
`DIFF-012` 的双旧/双新身份与来源顺序扰动仍由后续矩阵切片完整闭合。

## 语义输入

- retained LaneEdge 的 speed limit 从 `10.0` 改为 `12.0`，产生一条 Entity Modify；
- LaneEdge key 从 `identity-old` 改为 `identity-new`，完整 Identity v1 前像和 StableId 随之
  改变，只产生一条 Entity Remove 和一条 Entity Add，不记录旧、新 ID 配对；
- retained ParticipantClass 的 parent 从 `parent-a` 改为 `parent-b`，产生 role `24` 的唯一
  Relation Reconnect；
- base 为 headless，target 增加 CanonicalFrame，并为两条 target LaneEdge 增加规范 geometry，
  产生两条 Geometry Add 和一条 SpatialConfiguration Modify；
- retained AccessRule 的 effect 从 Allow 改为 Deny，产生 field tag `5` 的 StaticRule Modify；
- 本向量不声明独立验证、可信发布或迁移授权。

## base/target 绑定

| 侧     | LFCA exact length | LFCA SHA-256 / object key                                                 | NetworkRevisionId                                                  |
| ------ | ----------------: | ------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| base   |             3,220 | `sha256/9006828d6105e970a1c98246baf7a4008aba39bf12ecd83e3dd6c013ef4569b0` | `876972850ffdf9fc6c230deaecb2c950a0aa9074d3640ecfeb4de066c08a5542` |
| target |             4,250 | `sha256/58995d30d5be9fe0d440dd90e1d73b531514ee2e8eb50bfdb0c8dcc80899626d` | `e553065685e30a651c9b2e0485a92691210c9d11a122e1cfbf05119125fc3436` |

两端 network revision derivation version 均为 `1`。测试从实际 LFCA exact bytes 重算并逐值
比较 LFSD binding 的 revision、digest 和 length，不接受调用方自报值。

## 固定对象

| 文件            | exact length | SHA-256 / object key                                                      |
| --------------- | -----------: | ------------------------------------------------------------------------- |
| `expected.lfsd` |        2,479 | `sha256/9839be5aa5c2a37535769ddd49d6822f5a14798c484d6276df85903e102a4f72` |

公共前导为 `magic=LFSD`、`formatVersion=1`、`headerByteLength=32`、`flags=0`、
`sectionCount=6`、`sectionDirectoryOffset=32`、`objectByteLength=2479`。

| section kind | byte offset | byte length | change row count |
| ------------ | ----------: | ----------: | ---------------: |
| `0x0001`     |         176 |         293 |              N/A |
| `0x0002`     |         469 |         686 |                4 |
| `0x0003`     |       1,155 |         192 |                1 |
| `0x0004`     |       1,347 |         844 |                2 |
| `0x0005`     |       2,191 |         131 |                1 |
| `0x0006`     |       2,322 |         157 |                1 |

变化表精确闭合为：

- Entity：两条 Add、一条 Remove、一条 Modify，按 change kind、entity kind、StableId/field
  tag 规范排序；Identity 变化的 Add/Remove StableId 不同，且两行没有配对字段；
- Relation：一条 Reconnect，before/after local index 均为 `0`，before/after target 不同；
- Geometry：两条 Add，payload 均为 `CanonicalGeometryValueV1`，不嵌入完整 geometry RowV1；
- StaticRule：一条 AccessRule effect Modify，before/after bytes 分别为 `01`、`00`；
- SpatialConfiguration：一条 Modify，before/after 为不同的完整 SpatialPresence RowV1。

## 复核与更新规则

固定字节由 production emitter 一次性 materialize，提交态不保留写文件入口。随后使用只读
PowerShell byte reader 直接检查前导、六项目录、每节单表和 row count，并以 `Get-FileHash`
独立计算摘要；该 reader 没有调用 production emitter，也不作为仓库工具保留。

合成前端把 `#[track_caller]` 来源位置编码进 source record；因此移动本测试的语义输入构造会
合法改变 source provenance、LFCA digest 和 LFSD artifact binding，但在规范语义不变时不得
改变 NetworkRevisionId。更新本文件或 `expected.lfsd` 时，PR 必须同时说明语义输入变化、
格式版本判断、旧向量处置和新的独立复核结果。

测试只通过 `include_bytes!` 读取固定对象，不会在测试运行时生成或覆盖它。禁止增加
`expected.json`、JSON Schema、codegen、专用 `xtask` 或第二套永久 emitter。
