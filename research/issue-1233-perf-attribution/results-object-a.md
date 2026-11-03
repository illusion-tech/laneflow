# Issue #1233 对象 a：lifecycle 秒级突发分段测量（W4）

## 口径与配置
- **实现提交**：`e2bafbac329aefd3d058b925f42078a146c03c5a`（分支 `research/1233-perf-attribution`，release 构建）。
- **输入/比例**：`10000-W4-16-a`（表现比例 100%）、`100000-W4-33-a`（表现比例 10%）；各 **3 个全新进程轮**。
- **oracle**：同一实现提交的 release 语义运行生成（checkpoints/state_digest），测量端逐检查点核对通过。
- **环境**：`environment.json`（`source_clean=true`、rustc 1.99.1、AMD Ryzen 9 9955HX、Windows 11）；安静窗口 2026-11-04 02:20–02:48 JST（无并发矩阵/测试）。
- **计时口径**：整数纳秒；`command_execution_ns`（runtime 命令 API 调用）+ `harness_other_ns`（harness 记账）与 `lifecycle_ns` **守恒**，`residual_ns` 显式列示；checkpoint 校验独立计时（不计入 lifecycle 区间）。

## 结果

| 配置 | 轮 | lifecycle_ns | command_execution_ns | harness_other_ns | residual_ns | command 占比 | conserved |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| 10000-W4-16-p100 | r1 | 146,742,600 | 143,550,800 | 3,191,800 | 0 | 97.825% | true |
| 10000-W4-16-p100 | r2 | 137,267,200 | 134,151,300 | 3,115,900 | 0 | 97.730% | true |
| 10000-W4-16-p100 | r3 | 156,658,900 | 153,518,400 | 3,140,500 | 0 | 97.995% | true |
| 100000-W4-33-p10 | r1 | 15,621,564,000 | 15,616,511,000 | 5,053,000 | 0 | 99.968% | true |
| 100000-W4-33-p10 | r2 | 14,416,847,300 | 14,411,693,500 | 5,153,800 | 0 | 99.964% | true |
| 100000-W4-33-p10 | r3 | 14,701,991,200 | 14,697,067,100 | 4,924,100 | 0 | 99.967% | true |

### 三轮中位 / 最坏（max）

| 配置 | lifecycle 中位 | lifecycle 最坏 | command 中位 | command 最坏 | harness_other 中位 | harness_other 最坏 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10000-W4-16-p100 | 146,742,600 | 156,658,900 | 143,550,800 | 153,518,400 | 3,140,500 | 3,191,800 |
| 100000-W4-33-p10 | 14,701,991,200 | 15,621,564,000 | 14,697,067,100 | 15,616,511,000 | 5,053,000 | 5,153,800 |

**checkpoint 校验独立计时**（validate_population + state_digest + oracle 对比）：

| 配置 | r1 | r2 | r3 |
| --- | ---: | ---: | ---: |
| 10000-W4-16-p100 | 21,977,600 | 18,926,500 | 23,871,700 |
| 100000-W4-33-p10 | 284,030,700 | 216,669,200 | 253,165,700 |

## 守恒核对
- 每轮 `command_execution_ns + harness_other_ns + residual_ns == lifecycle_ns`（`conserved=true`），残差恒为 0；分段和与 #1170 的整段 lifecycle 区间口径一致。

## 结论
- lifecycle 区间 **约 97.7%–98.0%（10000）/ 99.96%–99.97%（100000）** 为 runtime **命令 API 执行段**；harness 记账段约 2.0%–2.3% / 0.03%。
- 突发随规模放大时，命令执行段占比进一步升高（harness 记账绝对量基本不变，~3–5 ms）。
- checkpoint 校验成本独立：10000 约 19–24 ms，100000 约 217–284 ms。

## 局限与后续
- **准入检查 / 索引维护 / 实体映射**位于 runtime 命令 API 内部，本迭代未单独分段（并入 `command_execution_ns`）；如需按方案稿 §4.1 六维拆分，需在 runtime 命令路径增加 `execution-evidence` 阶段探针（feature 内、零快路径影响），列为后续。
- 本测量未启用 `allocation` feature（`instrumented=false`）；allocation 诊断单列，未计入上表。

## 原始数据
- 测量目录 `lf1233/a-*`（6 个）、oracle `lf1233/oracle-*`；日志与报告生成于分支研究文档同目录。
