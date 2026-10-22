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
cargo +1.98.1 run -p laneflow-lust-converter -- verify-source --source-dir <LuSTScenario根目录>

cargo +1.98.1 run -p laneflow-lust-converter -- convert --config <toml>
```

可选全量集成（默认 `cargo test` 跳过）：

```text
set LUST_SOURCE_DIR=<LuSTScenario根目录>
cargo +1.98.1 test -p laneflow-lust-converter --locked --lib -- --ignored --skip regenerate
```

`--skip regenerate` 不可省略：`--ignored` 会并发跑全部 ignored 测试，其中
`regenerate_lust_stub_weld_candidates_manifest` 覆写 `evidence/` 的候选 manifest
基线——与比对测试并行竞争，漂移基线会被覆写后门检失效。regenerate 只在
更新基线时显式单独运行（见 `evidence/README.md` 更新流程）。

### 精确十进制渲染回归与局部测量

`sumo::decimal::tests::pinned_lust_decimal_rendering_matches_reference` 先通过
固定源集校验，再逐项比较参考实现与当前实现的字符串及 binary64 位模式。
输入覆盖全网 lane 的 length/speed/shape、精确投影与重定中心、相邻点和相对首点
差分、信号时长、车辆参数及全部 DUE 出发时间；普通测试另覆盖 i128 边界与
重复求中点产生的高精度小数。

设置 `LUST_SOURCE_DIR` 后，上面的全量集成命令包含这项回归。局部性能测量还需
设置 `LUST_DECIMAL_BENCH=1`，然后单独运行：

```text
cargo +1.98.1 test --release --locked -p laneflow-lust-converter --lib sumo::decimal::tests::pinned_lust_decimal_rendering_matches_reference -- --exact --ignored --nocapture
```

测量先验证等价性、预热两条路径，再交替先后顺序各取 10 个样本；每个样本遍历
同一输入集两遍，分别输出纯渲染和渲染后解析 binary64 的毫秒数组。源校验、XML
解析和输入集构造不计时。这是固定真实输入分布下的十进制局部微基准，不能据此
宣称全量转换或 Traffic Runtime 的端到端加速。完成后清除 `LUST_DECIMAL_BENCH`，
避免普通 ignored 回归意外进入 release 专用测量。

### 转换配置

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
