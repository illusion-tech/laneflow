# LaneFlow JSON Schema Source and Publication

<!-- schema-publication-contract: public-retrieval -->
<!-- schema-publication-catalog: schemas/publication.json -->
<!-- schema-source-current: traffic=0.7;spatial=0.1;scenarioManifest=0.1 -->

本目录同时保存当前 schema source 与已经公开的 immutable schema artifacts。机器可读的 family、current source、canonical URL 与 publication provenance 见 [`publication.json`](publication.json)。

## 当前 Source Contract

| Family            | Current source                                                                               | Publication 状态 |
| ----------------- | -------------------------------------------------------------------------------------------- | ---------------- |
| Traffic           | [`laneflow-data-v0.7.schema.json`](laneflow-data-v0.7.schema.json)                           | 已发布           |
| Spatial           | [`laneflow-spatial-v0.1.schema.json`](laneflow-spatial-v0.1.schema.json)                     | 已发布           |
| Scenario Manifest | [`laneflow-scenario-manifest-v0.1.schema.json`](laneflow-scenario-manifest-v0.1.schema.json) | 已发布           |

`currentFormatVersion` 表示 repository 中由 loader/tests 使用的当前 source；只有列入对应 `publishedSchemas` 且具有固定 `sourceRevision` / `sourceBlobOid` 的文件才属于公共发布集合。新 family 可以先合入 source，再由后续 publication PR 固定 `main` revision 并发布，避免以可变分支提交伪造不可变 provenance。

## 公共契约

- 每个 published schema 的 `$id` 是 canonical identifier，也是受支持的 HTTPS retrieval URL。
- 已发布 version 永久保留且不可原地修改；修正格式契约必须发布新的 `formatVersion`。
- canonical URL 的响应体必须与 catalog 固定 source revision 中的 schema 逐字节一致。
- v0.2/v0.3 Traffic 保留历史 Raw GitHub `$id`；后续 schema 使用 organisation-owned GitHub Pages URL。
- Pages `/schema/` 只包含 `publishedSchemas` 与 machine-readable publication index；source-only schema 不会被误部署。

当前 Traffic production source 是 v0.7，其固定 `main` provenance 已登记到公共发布集合。
当前 schema contract 是：

- [`laneflow-data-v0.7.schema.json`](laneflow-data-v0.7.schema.json)
- <https://illusion-tech.github.io/laneflow/schema/laneflow-data-v0.7.schema.json>
- [`laneflow-spatial-v0.1.schema.json`](laneflow-spatial-v0.1.schema.json)
- <https://illusion-tech.github.io/laneflow/schema/laneflow-spatial-v0.1.schema.json>
- [`laneflow-scenario-manifest-v0.1.schema.json`](laneflow-scenario-manifest-v0.1.schema.json)
- <https://illusion-tech.github.io/laneflow/schema/laneflow-scenario-manifest-v0.1.schema.json>

Traffic v0.2-v0.5 只作为 immutable publication artifacts 保留，不进入 current production loader、fixture 或 compatibility matrix。

## Runtime 边界

Core、production loader、Adapter 与 hermetic runtime tests 不联网解析 `$id` / `$schema`。公共发布是 distribution/CD concern；调用方主动下载 schema 时负责输入大小、内容验证、缓存和网络失败处理。

## 验证与构建

```powershell
cargo +1.96.0 run --locked -p xtask -- check-schema-publication-contract
cargo +1.96.0 run --locked -p xtask -- build-schema-publication target/schema-publication-site
```

GitHub Pages deployment 与每日 live monitor 分别由 `.github/workflows/schema-publication.yml` 和 `.github/workflows/schema-publication-monitor.yml` 执行。详细决策见 [ADR 0011](../docs/adr/0011-schema-identifier-and-publication-contract.md)。
