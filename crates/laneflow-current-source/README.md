# laneflow_current_source

LaneFlow current Traffic v0.10、SpatialPackage v0.1 与 ScenarioManifest v0.1 的 wire 校验、版本闸口、SHA-256 摘要与 Manifest 配对 source crate。

本 crate 只暴露两条 production-compatible 能力：

- `validate_traffic_compatible`：只接受 Traffic `formatVersion: "0.10"`，先执行头部版本闸口再解析 closed wire DTO；Traffic-only 入口不虚构 Manifest 或 Spatial；
- `validate_scenario_compatible`：按冻结顺序完成 Manifest syntax/版本/shape、Traffic/Spatial descriptor、conflicting/provided `artifactRef`、Traffic size→digest、Spatial size→digest 与 Traffic/Spatial wire 解析；额外唯一制品只检查非空与唯一，不哈希、不解析、不复制。

两条能力成功时返回不可拆的验证能力（capability），跨包消费固定为 capability 上的借用 accessor 与消费型 `into_parts(self)` 视图；"无 Serde"的承诺只适用于 capability 与 parts——它们不提供 `Clone`、`Default`、Serde 或裸构造器。wire DTO record 字段保持私有，不提供 `Clone`、`Default`、公开字段或裸构造器；其 `Deserialize` derive 只是本未发布 crate 的隐藏实现面，仅供 crate 内部解析使用，不属于对外能力承诺。

错误面固定为至少含一项 issue 的 `CurrentSourceError` bundle；每项 issue 携带 document、context、规范 `$` path、category 与 owned `serde_json::Error`，payload 只含 production 可达 variant 并提供稳定字符串码。`CurrentSourceIssueParts::into_components` 是调用方取走不可 Clone `serde_json::Error` 的唯一 owned bridge。

本 crate 不读取文件、不联网、不做 Core/Spatial 规范化，也不建立位置表、strict profile 或资源账本（归属 #297 后续切片）。

依赖方向固定为：

```text
laneflow-data -> laneflow-current-source
laneflow-current-source -X-> laneflow-core / laneflow-spatial / laneflow-compiler
```

本 crate 只依赖 Rust 标准库、Serde/serde_json、serde_path_to_error、SHA-256 与 thiserror，不依赖任何 LaneFlow crate。
