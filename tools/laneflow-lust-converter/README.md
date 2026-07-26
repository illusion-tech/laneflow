# laneflow-lust-converter

LuST Scenario v2.0 source/static converter for Issue #253.

权威契约：[`docs/design/real-road-workloads.md`](../../docs/design/real-road-workloads.md)。

## 当前范围（切片 A 增量）

- `verify-source`：按 §2.2 对固定 commit / 文件做 size + SHA-256 fail-closed 校验。
- 库 API：`parse_sumo_network_xml` / `parse_tll_static_xml` / `parse_vtypes_xml` / `convert_topology_from_xml_with_tll_and_vtypes` —— 转为 Traffic v0.8（§3.1 Junction/Movement/ManeuverPath、§3.3 Signals、§3.4 六个 passenger profiles）+ Spatial v0.1 + ScenarioManifest。
- 本增量仍 **空** routes；完整 static bundle（DUE routes / report / provenance / tar）后续同 draft PR 交付。
- `convert` CLI：仍先跑 source verify，随后返回 `StaticConversionNotImplemented`（待 routes/provenance/tar 齐备后再接线）。
- **不**交付 TOPO/DEMAND plan（#254 / #255）。
- **不**把 LuST 大体量 source/static 提交进 Git。

## 用法

```text
# 本地需先 checkout 精确 commit c4bd5bd3751d426d42a9a1749c815e47ea188549
cargo +1.96.0 run -p laneflow-lust-converter -- verify-source --source-dir <LuSTScenario根目录>

cargo +1.96.0 run -p laneflow-lust-converter -- convert --config <toml>
```

可选全量 net 集成（默认 `cargo test` 跳过）：

```text
set LUST_SOURCE_DIR=<LuSTScenario根目录>
cargo +1.96.0 test -p laneflow-lust-converter --locked -- --ignored
```

配置示例：

```toml
source_dir = "E:/data/LuSTScenario"
output_dir = "E:/data/laneflow-lust-out"
```

## 许可边界

上游 LuST MIT + OSM 派生 ODbL 1.0。Release 制品与 NOTICE 要求见 `real-road-workloads.md` §8。
