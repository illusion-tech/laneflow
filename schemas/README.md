# LaneFlow Schema Source

本目录保存道路编辑前端使用的 schema source。它们不是对外发布的公共契约。
[ADR 0026](../docs/adr/0026-merge-governance-rebuild.md) 已取消公共 retrieval、永久 URL
与历史 schema 保留义务。

## 当前内部 source

Traffic v0.10、Spatial v0.1 与 Scenario Manifest v0.1 JSON Schema 已随 #301 删除，
不再作为仓库内部 loader、测试或 authoring 契约。

| Family           | Current source                                                                         | 状态                                     |
| ---------------- | -------------------------------------------------------------------------------------- | ---------------------------------------- |
| Road Editing     | [`road-editing/v3/road-editing.fbs`](road-editing/v3/road-editing.fbs)                 | 生产 compiler 来源；`format_version = 3` |
| Runtime Snapshot | [`runtime-snapshot/v1/runtime-snapshot.fbs`](runtime-snapshot/v1/runtime-snapshot.fbs) | 生产快照容器；`format_version = 1`       |

字段级领域语义见各家族 README。两个家族都使用 size-prefixed FlatBuffers，不是
JSON Schema。Road Editing 用 `LFRE` file identifier，现行 `format_version = 3`；
其它版本的 LFRE buffer 由读器失败关闭，历史 schema 以 git 为准，不作为
生产入口。Runtime Snapshot 用 `LFRS` file identifier，现行 `format_version = 1`。

Traffic v0.2–v0.10 历史 schema 已从当前树删除。当时验证流水账只保留在 git 历史
与对应 Issue 中，不构成现行发布义务。

## Runtime 边界

Runtime、Adapter 与 hermetic runtime tests 不联网解析 `$id` / `$schema`。current
JSON 不再安装可运行交通世界，也不再作为仓库内部 schema source。
