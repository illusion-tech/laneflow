# LaneFlow Schema Source

本目录保存道路编辑前端使用的 schema source。它们不是对外发布的公共契约。ADR 0011
的公共发布义务已被
[ADR 0026](../docs/adr/0026-merge-governance-rebuild.md) 取代。

## 当前内部 source

Traffic v0.10、Spatial v0.1 与 Scenario Manifest v0.1 JSON Schema 已随 #301 删除，
不再作为仓库内部 loader、测试或 authoring 契约。

| Family       | Current source                                                         | 状态                            |
| ------------ | ---------------------------------------------------------------------- | ------------------------------- |
| Road Editing | [`road-editing/v2/road-editing.fbs`](road-editing/v2/road-editing.fbs) | 生产 compiler 来源；`format_version = 2` |

字段级领域语义见 [`road-editing/v2/README.md`](road-editing/v2/README.md)。它使用
size-prefixed FlatBuffers 和 `LFRE` file identifier，不是 JSON Schema。历史 v1 见
[`road-editing/v1/`](road-editing/v1/)，读器拒绝。

Traffic v0.2–v0.10 历史 schema 已从当前树删除。当时验证流水账只保留在 git 历史
与对应 Issue 中，不构成现行发布义务。

## Runtime 边界

Runtime、Adapter 与 hermetic runtime tests 不联网解析 `$id` / `$schema`。current
JSON 不再安装可运行交通世界，也不再作为仓库内部 schema source。
