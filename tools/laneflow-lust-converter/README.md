# laneflow-lust-converter

LuST Scenario v2.0 source/static converter for Issue #253.

权威契约：[`docs/design/real-road-workloads.md`](../../docs/design/real-road-workloads.md)。

## 当前范围（切片 A 首批）

- `verify-source`：按 §2.2 对固定 commit / 文件做 size + SHA-256 fail-closed 校验。
- `convert`：先跑同一校验，随后返回 `StaticConversionNotImplemented`（static 转换后续增量交付）。
- **不**交付 TOPO/DEMAND plan（#254 / #255）。
- **不**把 LuST 大体量 source/static 提交进 Git。

## 用法

```text
# 本地需先 checkout 精确 commit c4bd5bd3751d426d42a9a1749c815e47ea188549
cargo +1.96.0 run -p laneflow-lust-converter -- verify-source --source-dir <LuSTScenario根目录>

cargo +1.96.0 run -p laneflow-lust-converter -- convert --config <toml>
```

配置示例：

```toml
source_dir = "E:/data/LuSTScenario"
output_dir = "E:/data/laneflow-lust-out"
```

## 许可边界

上游 LuST MIT + OSM 派生 ODbL 1.0。Release 制品与 NOTICE 要求见 `real-road-workloads.md` §8。
