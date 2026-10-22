# laneflow-lust-converter

LuST Scenario v2.0 source/static converter for Issue #253.

权威契约：[`docs/design/real-road-workloads.md`](../../docs/design/real-road-workloads.md)。

## 当前范围（切片 A）

- `verify-source`：按 §2.2 对固定 commit / 文件做 size + SHA-256 fail-closed 校验。
- `convert`：校验 source 后生成（G1 验收重划口径，诊断模式输出集合）：
  - `issue253-infeasible-survey.md`：确定性 fail-closed 诊断清单（全网不可行
    普查，当前交付物；同基线与
    `evidence/lust-infeasible-survey.md` 逐字节一致）
  - `routes.toml`：**实现中**——DUE 展开 route catalog + 精确一万 population
    record；车道级展开语义待修订（`can_complete` 禁止边内换道，pinned 入选
    10,000 中 9,350 条途径不可展开，另立 issue 跟踪）——诊断模式不产出
    （#253 L1 绕过展开），fail-fast 路径在展开成功后产出
  - `manifest.toml` / `lust-conversion-report.json`
  - deterministic `lust-source.tar`（static bundle 与 `network.lfca` 依赖
    「路口级 maneuver 几何合成」新设计，落地前不交付，诊断模式不产出
    `lust-static.tar`）
  - `lust-semantic-provenance.json` / `lust-build-provenance.json`
  - `LICENSE.md`（upstream）、`ODbL-1.0.txt`、`NOTICE`
- **不**交付 TOPO/DEMAND plan（#254 / #255）。
- **不**把 LuST 大体量 source/static 提交进 Git。

## 用法

```text
# 本地需先 checkout 精确 commit c4bd5bd3751d426d42a9a1749c815e47ea188549
cargo +1.98.0 run -p laneflow-lust-converter -- verify-source --source-dir <LuSTScenario根目录>

cargo +1.98.0 run -p laneflow-lust-converter -- convert --config <toml>
```

可选全量集成（默认 `cargo test` 跳过）：

```text
set LUST_SOURCE_DIR=<LuSTScenario根目录>
cargo +1.98.0 test -p laneflow-lust-converter --locked --lib -- --ignored --skip regenerate
```

`--skip regenerate` 不可省略：`--ignored` 会并发跑全部 ignored 测试，其中
`regenerate_lust_stub_weld_candidates_manifest` 覆写 `evidence/` 的候选 manifest
基线——与比对测试并行竞争，漂移基线会被覆写后门检失效。regenerate 只在
更新基线时显式单独运行（见 `evidence/README.md` 更新流程）。

配置示例：

```toml
source_dir = "E:/data/LuSTScenario"
output_dir = "E:/data/laneflow-lust-out"
# converter_commit 可省略：记录值锚定 converter 源码 checkout 的实时 HEAD；
# 显式给出（或设环境变量 LANEFLOW_CONVERTER_COMMIT）时仅作一致性校验，
# 与 HEAD 不符即 fail-closed（防笔误/陈旧 SHA 混入 provenance）。
# 发布后填入 Release asset URL（权威仍是 size + SHA-256）：
# source_bundle_url = "https://github.com/illusion-tech/laneflow/releases/download/.../lust-source.tar"
# static_bundle_url = "https://github.com/illusion-tech/laneflow/releases/download/.../lust-static.tar"
```

## 许可边界

上游 LuST MIT + OSM 派生 ODbL 1.0。Release 制品与 NOTICE 要求见 `real-road-workloads.md` §8。
converter 自带 `licenses/NOTICE` 与 `licenses/ODbL-1.0.txt`；upstream `LICENSE.md` 从 pinned source 复制。
