# LaneFlow JSON Schema Publication

<!-- schema-publication-contract: public-retrieval -->
<!-- schema-publication-catalog: schemas/publication.json -->
<!-- schema-publication-current: 0.6 -->

本目录保存 LaneFlow 已公开的 versioned JSON Schema。机器可读的版本、canonical URL 与 provenance 见 [`publication.json`](publication.json)。

## 公共契约

- 每个 schema `$id` 是 canonical identifier，也是受支持的 HTTPS retrieval URL。
- 已发布 version 永久保留且不可原地修改；修正格式契约必须发布新的 `formatVersion`。
- canonical URL 的响应体必须与 catalog 固定 source revision 中的 schema 逐字节一致。
- v0.2/v0.3 保留历史 Raw GitHub `$id`；v0.4 起使用 organisation-owned GitHub Pages URL。
- Pages 的 `/schema/` 同时提供全部 published versions 与 catalog，方便发现。

当前 active loader/schema contract 是：

- [`laneflow-data-v0.6.schema.json`](laneflow-data-v0.6.schema.json)
- <https://illusion-tech.github.io/laneflow/schema/laneflow-data-v0.6.schema.json>

v0.2-v0.5 只作为不可变发布制品保留，不进入当前生产加载器、固定样例或兼容矩阵。

## Runtime 边界

Core、production loader、Adapter 与 hermetic runtime tests 不联网解析 `$id` / `$schema`。公共发布是 distribution/CD concern；调用方主动下载 schema 时负责输入大小、内容验证、缓存和网络失败处理。

## 验证与构建

```powershell
cargo +1.96.0 run --locked -p xtask -- check-schema-publication-contract
cargo +1.96.0 run --locked -p xtask -- build-schema-publication target/schema-publication-site
```

GitHub Pages deployment 与每日 live monitor 分别由 `.github/workflows/schema-publication.yml` 和 `.github/workflows/schema-publication-monitor.yml` 执行。详细决策见 [ADR 0011](../docs/adr/0011-schema-identifier-and-publication-contract.md)。
