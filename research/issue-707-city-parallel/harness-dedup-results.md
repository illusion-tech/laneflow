# #707 H1 完整观测去重结果

2026-09-22。Refs #707。承接 [H1 计划](harness-dedup-plan.md)与
[B1 WPR 诊断](b1-wpr-findings.md)。**H1 保留为后续研究候选：完整观测不降频，
100k 短 ABBA 中 observation 均值下降 13.61%，iteration 下降 9.29%。Core 没有
明确改善，16ms p95 目标仍未达到。** 本切片完成，没有扩大到 C1、H2 或红灯 ETA 修正。

## 1. 实现范围

独立源码位于 `target/issue707-h1-source`，继承 B1 的 `4de40e04` 加研究补丁，
本次增量仅涉及 Harness 的 7 个文件。Runtime 算法、公开 API、静态格式、Adapter
接口未改；父 B1 的 79 个已登记源码文件与补丁摘要再次核验通过。

H1 在拍后 `state` 的车辆遍历中，同时完成原 `counts` 的身份/生命周期计数和
`parking_invariants` 的停车绑定聚合。每辆 live 车辆读取一次状态、一次停车绑定，
局部值供三个消费者复用，最后核验总量与停车设施容量。

- 保留所有生命周期状态；逐拍摘要编码、SHA-256、质量累计、车身区间排序、冲突
  owner、Waiting 与停车不变量的检查内容及频率均保留。
- 拍前 `step_before`、红灯等待观测和事件处理不参与合并。
- 没有整世界投影、按车辆缓存或跨拍缓存。停车聚合表原本就存在；H1 将它的构建
  提前，因此与 state 临时缓冲的存活期重叠，不能先验宣称零峰值内存变化。
- `LF707_OBSERVE=reference/fused/audit` 是有限研究对照。`reference` 保留原始
  直接读取路径；`audit` 在同一个已提交状态分别运行两条路径，比较摘要、计数、
  逐拍质量并执行双方检查。后续使用候选需显式设置 `fused`。
- 多个错误同时存在时，融合路径可能先报告另一项错误；本切片要求错误仍被拒绝，
  不要求错误先后顺序相同。它不恢复旧版 Core 逐拍轨迹等价门槛。

实现、二进制及数据仍在本地研究目录，未合入主线，也未提交或推送。

## 2. 工作量确实减少

独立 `observation-work` 构建在实际 Host 读取调用与遍历点计数，范围只包括拍后
state/counts/parking 检查；不统计 Core、拍前观测或事件处理。此构建不用于性能结论。

100k、256 拍诊断中的每拍工作量如下；全部拍的比率通过分析器核验。

| 工作 | H0 | H1 |
| --- | ---: | ---: |
| 车辆状态读取 | 200000 | 100000 |
| 停车绑定读取 | 300000 | 100000 |
| 三项检查的主要 individual 遍历 | 300000 | 100000 |
| live 计数的额外 identity 扫描 | 100000 | 0 |
| 停车目标聚合 HashMap 最大 capacity | 3584 | 3584 |

因此车辆读取减少 50%，停车绑定读取减少 66.67%；包含 identity 扫描的 individual
访问从每拍 400000 降到 100000。摘要处理字节与空间排序工作没有减少。
这组数值来自逻辑调用计数，不是把 WPR 的采样 hits 当作调用次数。

## 3. 独立观测审计

4 workers、B1 固定为 `approx/both/combined/boundary=on/decision=every`：

| 输入 | 每臂拍数 | 臂 |
| --- | ---: | --- |
| junction_balanced-10k | 512 | H0 / H1 / 同状态双路径审计 |
| junction_long_tail-10k | 512 | H0 / H1 / 同状态双路径审计 |
| movement_mixed-10k | 768 | H0 / H1 / 同状态双路径审计 |
| junction_long_tail-100k | 256 | H0 / H1 / 同状态双路径审计 |

12 臂共 6144 拍全部完成。每组的 `ticks.jsonl`、`commands.jsonl`、`events.jsonl`、
`quality.csv`、`individual-quality.csv`、entry/boundary/decision 工作量文件均逐字节
相同，初始与最终 checkpoint 相同。扩展质量字段也相同，唯一排除的是明确记录
观测耗时的 `extra_observation_ns`。后面的 4 个性能臂也通过相同检查。

新增 3 个测试覆盖七类 UrbanCase 的 1792 拍同状态审计，以及错误身份映射、
birth/removal 守恒、缺失 individual、Parked 绑定缺失和聚合数据损坏的拒绝。
完整 Harness release 库测试为 **23 passed / 0 failed / 3 既有 ignored**；新增
测试单独执行也全部通过。既有多 gap 集成测试通过；默认配置 `cargo check`、
诊断配置 `cargo check`、Harness `cargo fmt --check` 和 `git diff --check` 均通过。

初次混合输入尝试将 prefix 设为 1024，等于原计划终点，被既有入口检查拒绝，
没有开始仿真。改在新目录运行 768 拍；失败预检记录保留且不进入统计。

## 4. 无插桩 100k ABBA

同一 release 二进制，无 WPR、无工作量插桩、无 stage 插桩、无 stop-trace。
四臂串行运行，期间没有编译或分析任务。100k 是 individuals 规模；统计窗
**tick 65–512**，每臂 448 拍，Active 为 **65577–74392**，不是十万车辆全部 Active。
分位数按 `(n−1)×p` 线性插值，保留检查/命令尖峰，没有剔除昂贵拍。

| 臂 | observation mean ms | iteration mean ms | Core mean ms | Core p95 ms | Core p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| H0，第一臂 | 81.365 | 120.597 | 33.009 | 37.285 | 38.732 |
| H1，第二臂 | 70.364 | 109.822 | 33.270 | 37.756 | 40.485 |
| H1，第三臂 | 71.580 | 111.220 | 33.300 | 38.561 | 41.653 |
| H0，第四臂 | 82.947 | 123.081 | 33.786 | 38.972 | 40.833 |

两臂均值的对比：

| 指标 | H0 | H1 | 变化 |
| --- | ---: | ---: | ---: |
| observation mean | 82.156ms | 70.972ms | **−13.61%** |
| iteration mean | 121.839ms | 110.521ms | **−9.29%** |
| Core mean | 33.398ms | 33.285ms | −0.34%，不认定明确收益 |
| 完整进程墙钟 | 66.586s | 60.605s | −8.98% |

H1 两臂的 observation 和 iteration 都低于 H0 两臂，收益方向一致，因此本切片
无需为确定方向额外扩大测量。没有据此认证长期稳定性能。

Core 尾部没有明确改善：H1 的 p99 还高于对应相邻 H0；样本不足以将这种差异归因
于观测重排，但足以拒绝“Core 也明显提速”的说法。优化观测可能改变缓存和执行
节奏，后续 Core 候选应在固定观测模式下重新计时。

iteration 包含 advance 与缓冲语义写出，不包含独立扩展质量记录、最终 flush 与
checkpoint；完整进程墙钟包含这些工作以及安装。16 个实验臂共 8192 拍。

## 5. 内存与质量边界

通过每 250ms 读取进程高水位计数观察内存，可能漏掉退出前尾部峰值。性能臂
观察到的 working-set 峰值 H0 为 451.895/448.039MiB，H1 为 448.160/448.043MiB；
paged-memory 高水位约 524.5–525.2MiB。没有明显的新增大缓存成本，也不把这点
差异宣传成内存优化。首个 10k 诊断批次在退出后读取内存得到 null，未拿它作比较。

运行前存在两个闲置 Python 进程；采集的预检与四个性能臂期间其 CPU 增量均为 0。
未终止用户其他进程，也未改变电源设置。环境保持平衡电源方案。

100k 性能臂都观测到 10269 个完成、1153 次 reservation 释放；未观测到 same-zone
多 owner、unmatched clear 或 completed owner 仍持有资源，且这些结果与 H0 相同。
这只证明本切片未减少既有观测、已测数据一致，不是连续碰撞认证或完整交通质量
通过。既有红灯 ETA=0 错误阻塞、长等待问题未修复。

## 6. 结论与复核入口

**采用 H1 作为后续研究的快速完整观测候选，本切片收口。** 不继续扩展 Harness
重构，也不顺带实施降频。下一条性能主线是 C1：测运动约束的真实重复查询与
同一不可变视图内的复用成本；红灯 ETA 修正独立记账。

原目标仍为 100k 规模、33ms 仿真步长、Core p95≤16ms，当前没有达标。

本机根：`E:/projects/laneflow/target/issue707-h1-results/`。

- `summary.json`：每臂 mean/p95/p99/max、Active、内存与观测一致性结果。
- `audit-*`、`balanced-100k`：原始输出、工作量与进程元数据；失败预检目录单独排除。
- `analyze.py`：离线重新验证输出身份、连续 tick、观测相同和工作量比率。
- `run.ps1`：有限串行运行，拒绝覆盖既有目录，记录 WPR 状态和环境。
- `experiment.patch`、`provenance.json`、`verification.json`：完整研究补丁、身份清单、
  父 B1 源码与输入及新制品的独立摘要复核。

性能二进制 SHA-256：
`5525f9aea127793c69ab08d4891fd05e3049dd79585199e00272b5d050391080`。
工作量二进制 SHA-256：
`68f32c5db10399f5850000bfe48fdbaab9725b28971ec0658d4881985b7f6966`。
两者均 Rust 1.98.0、release、locked/offline、incremental=0；仅诊断构建额外开启
`observation-work`，两者均包含 `entry-frontier`。
