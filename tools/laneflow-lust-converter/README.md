# laneflow-lust-converter

LuST Scenario v2.0 source/static converter for Issue #253.

规范契约：[`docs/design/real-road-workloads.md`][real-road-workloads]。

## 当前范围（A 部分）

- `verify-source`：按 §2.2 对固定 commit / 文件做 size + SHA-256 fail-closed 校验。
- `convert`：校验 source 后生成（G1 验收重划口径，诊断模式输出集合）：
  - `issue253-infeasible-survey.md`：确定性 fail-closed 诊断清单（全网不可行
    普查，当前交付物；验收以固定 SHA-256 摘要锁定，全量正本已随 #1386 退役，
    追溯见退役前提交的 `evidence/lust-infeasible-survey.md`）
  - `routes.toml`：**实现中**——DUE 展开 route catalog + 精确一万 population
    record；车道级展开语义待修订（`can_complete` 禁止边内换道，v2 人口入选
    10,000 中 9,344 条途径不可展开）——诊断模式不产出
    （#253 的诊断层 L1 绕过展开——与 LF-SYNTH 五轴验收的 L1 输入拓扑核查（#1271，
    `LaneChange` 归一为 `L1`）是两个不同概念），fail-fast 路径在展开成功后产出
  - `manifest.toml` / `lust-conversion-report.json`
  - deterministic `lust-source.tar`（static bundle 与 `network.lfca` 依赖
    「路口级 maneuver 几何合成」新设计，落地前不交付，诊断模式不产出
    `lust-static.tar`）
  - `lust-semantic-provenance.json` / `lust-build-provenance.json`
  - `LICENSE.md`（upstream）、`ODbL-1.0.txt`、`NOTICE`
- **不**交付 TOPO/DEMAND plan（#254 / #255）。
- **不**把 LuST 大体量 source/static 提交进 Git。

共享人口按 §4.1 使用 `LF-REAL-LUST-POPULATION-v2`、XXH3-64 seed 0 与原始
UTF-8 ID 次键。转换报告为 `formatVersion=0.3`，明确携带 `population.derivationId`；
来源、制品与 provenance 仍以 SHA-256 校验。人口迁移与诊断证据见
固定提交中的 [#1053 研究记录][lust-population-v2-readme]。

## 用法

```text
# 本地需先 checkout 精确 commit c4bd5bd3751d426d42a9a1749c815e47ea188549
cargo +1.99.1 run -p laneflow-lust-converter -- verify-source --source-dir <LuSTScenario根目录>

cargo +1.99.1 run -p laneflow-lust-converter -- convert --config <toml>
```

可选全量集成（默认 `cargo test` 跳过；均需 pinned `c4bd5bd3` 的
`LUST_SOURCE_DIR`，**按组用精确测试名运行**——多个测试共享输出目录变量且各自
要求 fresh 目录，全量 `--ignored` 并发会互相踩目录， regenerate 也会写
`target/`）：

真实源回归（只需 `LUST_SOURCE_DIR`；诊断清单落
`target/issue253-infeasible-survey.md`，人口回归可用
`LUST_POPULATION_REGRESSION_OUTPUT_DIR` 改输出目录）：

```text
set LUST_SOURCE_DIR=<LuSTScenario根目录>
cargo +1.99.1 test -p laneflow-lust-converter --locked --lib --release -- --ignored full_lust_net_topology_matches_external_lane_anchor
cargo +1.99.1 test -p laneflow-lust-converter --locked --lib --release -- --ignored full_lust_convert_cli_end_to_end
cargo +1.99.1 test -p laneflow-lust-converter --locked --lib --release -- --ignored full_lust_source_verification_precedes_diagnostic_convert
cargo +1.99.1 test -p laneflow-lust-converter --locked --lib --release -- --ignored verified_record_with_tampered_bytes_fails_closed
cargo +1.99.1 test -p laneflow-lust-converter --locked --lib --release -- --ignored lust_stub_weld_candidates_match_manifest
cargo +1.99.1 test -p laneflow-lust-converter --locked --lib --release -- --ignored pinned_lust_population_source_regression
```

S0F 邻接普查（另需**不存在**的 `S0F_OUTPUT_DIR`）：

```text
set S0F_OUTPUT_DIR=<尚不存在的目录>
cargo +1.99.1 test -p laneflow-lust-converter --locked --lib --release -- --ignored pinned_lust_path_adjacency_survey
```

局部诊断（各自需**不存在**的 `GRID_CALIBRATION_OUTPUT_DIR` 或
`CURVE_DESIGN_OUTPUT_DIR`，每个入口独立分配路径）：

```text
set GRID_CALIBRATION_OUTPUT_DIR=<尚不存在的目录>
cargo +1.99.1 test -p laneflow-lust-converter --locked --lib --release -- --ignored fixed_signal_seam_feasibility
:: 其余同组入口：fixed_signal_guided_recovery / fixed_signal_endpoint_feasibility /
:: fixed_grid_calibration / fixed_signal_span_probe —— 每个换一个新目录
set CURVE_DESIGN_OUTPUT_DIR=<尚不存在的目录>
cargo +1.99.1 test -p laneflow-lust-converter --locked --lib --release -- --ignored fixed_stream_a_curve_design
:: 其余同组入口：fixed_c12_curve_design / fixed_c17_bilateral_design /
:: fixed_single_edge_design_reuse / fixed_failed_curve_design —— 每个换一个新目录
```

两个 stub 相关 ignored 测试已随 #1386 分离：`regenerate` 只把完整候选诊断
manifest 写到 `target/issue253-stub-weld-candidates.json`（不覆写任何入库
文件，可与防漂移比对并发）；生产批准集合是 `src/convert/approved_stub_weld.json`
（net digest + 84 身份三元组，含 4 个 rejected-shared-entry），防漂移测试对
pinned 源重扫比对身份集合——批准域变更须先回 #253 走 G1 裁定并手工更新该
文件，不以 regenerate 覆写。

历史 pinned 重放入口与 `PINNED_RESEARCH_ROOT` 外部数据依赖已随 #1385 退役；
仍有效的最小回归输入已按文档边界归位到 `tests/fixtures/`（#1411 起信号闭合
局部输入统一复用 `issue-1113-controls.json`，历史迁入件已退役），源码注释与
输出来源字段中的
`research/...` 路径以含数据冻结提交 checkout 为根，正本在该提交可查。

### 精确十进制渲染回归与局部测量

`sumo::decimal::tests::pinned_lust_decimal_rendering_matches_reference` 先通过
固定源集校验，再逐项比较参考实现与当前实现的字符串及 binary64 位模式。
输入覆盖全网 lane 的 length/speed/shape、精确投影与重定中心、相邻点和相对首点
差分、信号时长、车辆参数及全部 DUE 出发时间；普通测试另覆盖 i128 边界与
重复求中点产生的高精度小数。

设置 `LUST_SOURCE_DIR` 后，上面的全量集成命令包含这项回归。局部性能测量还需
设置 `LUST_DECIMAL_BENCH=1`，然后单独运行：

```text
cargo +1.99.1 test --release --locked -p laneflow-lust-converter --lib sumo::decimal::tests::pinned_lust_decimal_rendering_matches_reference -- --exact --ignored --nocapture
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
# 发布后填入 Release asset URL（正本仍是 size + SHA-256）：
# source_bundle_url = "https://github.com/illusion-tech/laneflow/releases/download/.../lust-source.tar"
# static_bundle_url = "https://github.com/illusion-tech/laneflow/releases/download/.../lust-static.tar"
```

## 许可边界

上游 LuST MIT + OSM 派生 ODbL 1.0。Release 制品与 NOTICE 要求见 `real-road-workloads.md` §8。
converter 自带 `licenses/NOTICE` 与 `licenses/ODbL-1.0.txt`；upstream `LICENSE.md` 从 pinned source 复制。

[real-road-workloads]: ../../docs/design/real-road-workloads.md
[lust-population-v2-readme]: https://github.com/illusion-tech/laneflow/blob/e267d4c9eead22d14f7111555a70c70637cd9f04/research/issue-1053-lust-population-v2/README.md
