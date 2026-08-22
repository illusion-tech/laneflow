# laneflow_data

current JSON 运行时入口已由 #301 拆除。本 crate 不再把 Traffic/Spatial/Manifest JSON
正规化为可运行世界，也不创建 `TrafficWorld`。

它只再导出 `laneflow-current-source` 的格式版本与媒体类型常量，供 authoring 工具与
schema 测试使用。wire 校验仍由 `laneflow-current-source` 拥有；可运行交通世界只从
`SharedNetworkRevision` 安装。

```text
laneflow-data -> laneflow-current-source
laneflow-data -X-> laneflow-runtime
laneflow-data -X-> laneflow-spatial
```
