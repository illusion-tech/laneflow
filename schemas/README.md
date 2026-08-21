# LaneFlow Schema Source

本目录保存当前内部 loader、测试与道路编辑前端使用的 schema source。它们不是对外
发布的公共契约。ADR 0011 的公共发布义务已被
[ADR 0026](../docs/adr/0026-merge-governance-rebuild.md) 取代。

## 当前内部 source

| Family            | Current source                                                                               | 说明                        |
| ----------------- | -------------------------------------------------------------------------------------------- | --------------------------- |
| Traffic           | [`laneflow-data-v0.10.schema.json`](laneflow-data-v0.10.schema.json)                         | 当前 JSON loader；#294 删除 |
| Spatial           | [`laneflow-spatial-v0.1.schema.json`](laneflow-spatial-v0.1.schema.json)                     | 当前 JSON loader；#294 删除 |
| Scenario Manifest | [`laneflow-scenario-manifest-v0.1.schema.json`](laneflow-scenario-manifest-v0.1.schema.json) | 当前 JSON loader；#294 删除 |

## G1 候选 source

| Family       | Candidate source                                                       | 状态                             |
| ------------ | ---------------------------------------------------------------------- | -------------------------------- |
| Road Editing | [`road-editing/v1/road-editing.fbs`](road-editing/v1/road-editing.fbs) | #296 G1 冻结候选；未实现、未发布 |

字段级领域语义见 [`road-editing/v1/README.md`](road-editing/v1/README.md)。它使用
size-prefixed FlatBuffers 和 `LFRE` file identifier，不是 JSON Schema。

Traffic v0.2–v0.9 历史 schema 已从当前树删除。当时的 closure review 按当时语义保留
在 Git 历史与对应 Issue 中，不构成现行发布义务。

## Runtime 边界

Core、production loader、Adapter 与 hermetic runtime tests 不联网解析 `$id` /
`$schema`。调用方如果自行下载 schema，自行负责输入大小、内容验证、缓存和网络失败。
