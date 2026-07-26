# laneflow-lust-converter

LuST Scenario v2.0 source/static converter for Issue #253.

权威契约：[`docs/design/real-road-workloads.md`](../../docs/design/real-road-workloads.md)。

## 当前范围（切片 A）

- `verify-source`：按 §2.2 对固定 commit / 文件做 size + SHA-256 fail-closed 校验。
- `convert`：校验 source 后生成：
  - Traffic v0.8 / Spatial v0.1 / ScenarioManifest v0.1
  - DUE lane-level routes + 共享 10k `lust-population.json`（harness 输入，不进 Manifest）
  - `lust-conversion-report.json`
  - deterministic `lust-source.tar` / `lust-static.tar`
  - `lust-semantic-provenance.json` / `lust-build-provenance.json`
  - `LICENSE.md`（upstream）、`ODbL-1.0.txt`、`NOTICE`
- **不**交付 TOPO/DEMAND plan（#254 / #255）。
- **不**把 LuST 大体量 source/static 提交进 Git。

## 用法

```text
# 本地需先 checkout 精确 commit c4bd5bd3751d426d42a9a1749c815e47ea188549
cargo +1.96.0 run -p laneflow-lust-converter -- verify-source --source-dir <LuSTScenario根目录>

cargo +1.96.0 run -p laneflow-lust-converter -- convert --config <toml>
```

可选全量集成（默认 `cargo test` 跳过）：

```text
set LUST_SOURCE_DIR=<LuSTScenario根目录>
cargo +1.96.0 test -p laneflow-lust-converter --locked -- --ignored
```

配置示例：

```toml
source_dir = "E:/data/LuSTScenario"
output_dir = "E:/data/laneflow-lust-out"
converter_commit = "<本仓库 commit>"
# 也可改用环境变量 LANEFLOW_CONVERTER_COMMIT
# 发布后填入 Release asset URL（权威仍是 size + SHA-256）：
# source_bundle_url = "https://github.com/illusion-tech/laneflow/releases/download/.../lust-source.tar"
# static_bundle_url = "https://github.com/illusion-tech/laneflow/releases/download/.../lust-static.tar"
```

## 许可边界

上游 LuST MIT + OSM 派生 ODbL 1.0。Release 制品与 NOTICE 要求见 `real-road-workloads.md` §8。
converter 自带 `licenses/NOTICE` 与 `licenses/ODbL-1.0.txt`；upstream `LICENSE.md` 从 pinned source 复制。
