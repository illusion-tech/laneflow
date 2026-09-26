# P3 候选筛选接入复测（#759）

这是 `laneflow-runtime` 内部优化的有界验证。研究原型与更详细质量工具由 #757 / #758
独立交付；本报告不宣称 #707 城市性能认证完成。

## 输入与执行

- 基线：`46fdfaf47ae0c000ddc63420c8a71443baa8fb04`。
- 候选：`37b441a85d0866232c515f4d4e8dd0be494c0f47`；只修改 P3 发现与筛选、测试和设计说明。
- 两份独立干净 checkout，Rust 1.98.0，`cargo build -p laneflow-urban-harness --release --locked --offline`，`CARGO_INCREMENTAL=0`。
- 同一份 #707 `4de40e04` 输入；10k / 100k 的原 `*-smoke.toml`，MIXED-PEAK，seed 544。
- workers=4，每次 256 拍，包含初始阶段，不设暖机；每规模按 A/B/B/A/A/B 交错，A=基线、B=候选，共 12 个成功进程。
- 每次记录源码提交/tree、干净状态、二进制与计划摘要、输入清单摘要，运行后核验 HEAD、二进制、计划不变。
- Windows x86_64，32 逻辑处理器，AMD Family 26 Model 68 Stepping 0，平衡电源方案。

复现入口（从对应干净 checkout 执行其构建二进制，输出目录必须不存在）：

```powershell
laneflow-urban-harness.exe run <inputs/urban-10k> <plans/10k-smoke.toml> <new-output> --workers 4
laneflow-urban-harness.exe run <inputs/urban-100k> <plans/100k-smoke.toml> <new-output> --workers 4
laneflow-urban-harness.exe compare <baseline-output> <candidate-output> <new-comparison.json>
```

`probe-results.json` 保存每次运行诊断、身份、检查点、交通日志摘要和原始包文件索引。
外部原始包位于本工作树的 `target/p3-perf-final/`，二进制位于 `target/p3-binaries/`；
机器绝对路径是历史采集信息。Git 不包含原始大文件或可执行文件，外部复核需携带该包
及对应冻结输入，不能仅凭摘要声称复现。另保留 `target/p3-perf/` 的一次准备运行：
被测进程成功，但采集脚本读错诊断文件名后停止；修正脚本后重新开始完整矩阵，未混入下表。

## 结果

表中每项是三次运行各自分位数的中位数，不是合并全部样本后的总体分位数。

| 规模 | 基线 p50 (ms) | 候选 p50 (ms) | 降幅 | 基线 p95 (ms) | 候选 p95 (ms) | 降幅 |
| ---- | ------------- | ------------- | ---- | ------------- | ------------- | ---- |
| 10k | 3.6418 | 3.2889 | 9.7% | 4.4383 | 4.2202 | 4.9% |
| 100k | 48.1415 | 42.6668 | 11.4% | 56.8963 | 51.8105 | 8.9% |

全部运行均 `probe-complete`、256 拍、退出码 0。每规模六次运行的 `ticks.jsonl`、
`commands.jsonl`、`events.jsonl` 摘要、初末检查点及 result 语义字段完全相同；
原生结果中的文件大小和 SHA-256 全部核验。官方 compare 对两规模均给出 `probe-match`。

开发机探针没有隔离全部系统后台活动，窗口较短且包含入口阶段，10k p95 波动较大；
结论是未观察到回归且方向支持接入，不将这些比例当作长期稳定收益、全天质量结论
或 #707 正式性能达标证明。

## 正确性与边界

Runtime release lib：512 项通过，15 项既有手动测试忽略；release/all-targets Clippy
以 `-D warnings` 通过，格式与 diff 检查通过。新增筛选测试覆盖：

- 近门、远门、跨边零点回看、无后续 Gate、未知运动域保守保留。
- 混合远近车辆下 Active 缓存位与候选位不同，同槽新代次、筛选后末位失败、失败原子性与重试。
- workers 1/2/4/8/16 与测试专用完整求值 oracle 逐拍输出、快照及资源决策一致。
- Conflict 资源取得、跨拍持有和清空与完整求值一致；结束条件是见证一次真实释放，仿真上限 16.384 秒。
- 既有恢复/切换、首错次序、可选分配回退和组合矩阵继续通过；密集近门场景确认真实多线程参与。

完整求值 oracle 只在 `cfg(test)` 编译，用于筛选对拍及底层槽位/分配协议回归；
正式 crate 无环境变量开关、额外计数或第二条领域算法。已有 reservation 持有与释放
仍由原路径负责，P3 筛选不承担资源回收。

内部合入条件是上述必要验证、审阅和 CI。没有已识别的未解风险要求追加 44544 拍
100k 全计划；最终性能认证留在 #707 的适当阶段。
