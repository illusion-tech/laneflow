# #216 补充：真实 step 的 CPU 采样归因

## 1. 结论

2026-09-08 使用 Windows Performance Recorder（WPR）录制优化构建，WPA 导出
完整调用栈，再用相同 PDB 的 LLVM 符号解析器展开内联函数和源码位置。
这不是查询回放，也没有在生产函数中加入逐调用计时器。

**仅 occupancy 重建、leader 查询和查询 horizon 三个不重叠子树，就占真实 `step`
CPU 采样权重的 18.52%（10k / 256）和 21.43%（10k / 16）。**
原来的约 6% 重建阶段墙钟不能代表 #216 整条 exact path 的成本。
旧的单次查询计时仍因时钟底噪而作废；这份新证据补充实际执行路径的统计归因，
不把旧计时结果重新解释成可靠数据。

生产算法没有修改，原单一测试候选的负结果也没有改变。
**定位热点不等于热点已优化，更不等于 100k 城市性能问题已解决。**

## 2. 采样范围与质量

- 生产基线为 `6fefd582364685c07ed83af79b2bec80f5223fc5`；研究 HEAD 为
  `2806ce4ab83c4084a1b610854d7b31d6ae50e2fd` 加 CPU integration-test 入口。
  精确工作树 blob、EXE/PDB SHA-256、PDB GUID/age 见
  [`provenance.json`](evidence-cpu/provenance.json)。不能把该 HEAD 单独当作采样源码标识。
- Rust 1.98.0，MSVC release，`CARGO_PROFILE_RELEASE_DEBUG=2`、
  `CARGO_PROFILE_RELEASE_STRIP=none`，独立 `target/issue-216-cpu-symbols`。
  PDB GUID/age 与 EXE RSDS 匹配，没有改变生产函数的内联设置。
- integration test 链接生产库；生产库不带 `cfg(test)`、`StatsAlloc` 或现有研究探针。
  只在最外层 `observed_cpu_window` 保留栈锚点。
- 每轮重新构造同一输入，预热 32 + 8 tick，再观察 64 个 100 ms tick；
  校验、快照、构造和预热均在栈锚点之外。每轮检查固定 digest。
  individual、active、intent 均等于场景规模，presented、aggregate 均为 0。
- 每个场景录制一次；128/1,024 是 trace 内重复窗口次数，**不是独立机器试验轮次**。
  本次不是 A/B 加速比或产品延迟认证，未测 ETW 相对不开 ETW 的开销差值。
- 初始机器状态为 AMD Ryzen 9 9955HX、16 核 32 线程、Windows 11 Insider 29648、
  平衡电源方案、接电。没有绑核、锁频或声称整个操作系统空闲。
  录制期间没有同时运行 Cargo、其他 LaneFlow 性能程序或符号导出。
- 仅录制辅助进程经用户 UAC 授权提权；被测 EXE 以普通权限运行。
  三组正式 trace 与短 smoke trace 均正常保存，录制器正常退出，WPR 已停止。

| 场景      | 重复窗口 | 观测 step | step CPU 样本 | step 采样权重 | 观测窗口墙钟合计 |
| --------- | -------: | --------: | ------------: | ------------: | ---------------: |
| 1k / 256  |    1,024 |    65,536 |        12,662 | 12,570.698 ms |    13,172.930 ms |
| 10k / 256 |      128 |     8,192 |        17,310 | 17,184.110 ms |    18,003.079 ms |
| 10k / 16  |      128 |     8,192 |        17,803 | 17,668.511 ms |    18,499.155 ms |

三个正式 trace 的 `Lost Events` 和 `Lost Buffers` 均为 0。
先按被测 PID，再按 `observed_cpu_window → TrafficWorld::step` 子树筛选；
三个场景的锚点样本数均等于 `step` 子树样本数，各自只含一个测试工作线程。
进程内其他工作不进入下列百分比分母。

被选子树内没有应用程序 PDB 缺失；仍有 154 / 230 / 238 个未解析函数名样本，
主要为未下载系统 PDB 的系统模块，**保留在总权重中，没有丢弃后重新归一化**。
PDB 能给出某个源码位置的权重比例为 98.72% / 98.63% / 98.64%；
编译器合成的 `lib.rs:1` 等位置不视为精确业务行归因。

CPU 权重是统计采样，不是每次调用的精确墙钟；不可与原探针阶段占比相减。
采样及 inclusive 解释见
[Microsoft CPU Analysis](https://learn.microsoft.com/en-us/windows-hardware/test/wpt/cpu-analysis)。

## 3. #216 实际路径：重建之外还有查询成本

分母统一为各场景的 `step` CPU 采样权重。前三行包含各自子调用；
逐栈检查确认三者互不嵌套，因此可以相加，但不覆盖所有 route/profile 输入准备。

| 不重叠子树                   | 1k / 256 | 10k / 256 | 10k / 16 |
| ---------------------------- | -------: | --------: | -------: |
| `rebuild_occupancy_index`    |    7.94% |     7.47% |    6.99% |
| `OccupancyIndex::leader_gap` |    3.51% |     9.24% |   12.32% |
| `leader_query_horizon`       |    2.62% |     1.81% |    2.13% |
| 三项合计                     |   14.06% |    18.52% |   21.43% |

内联展开把 `leader_gap` 中的重要部分定位到
[`OccupancyIndex::nearest_ahead`](../../crates/laneflow-runtime/src/kernel/occupancy.rs)：
`records[start..end].partition_point(...)`（采样源码第 462 行）。
这一行分别占 **1.26% / 5.51% / 10.64%**；整个内联 `nearest_ahead`
源码函数在两个 10k 场景约占 5.94% / 10.81%。
随着桶密度增加，二分查找在实际交错执行中的份额明显增加。
未采集 cache-miss、分支预测或 IBS 硬件事件，不能认定某一种微架构瓶颈。

## 4. 运动、输出与提交的函数及内联边界

以下为 10k / 256 的主要**非内联符号 self**：包含编译进该符号的内联体，
排除单独的被调函数。因此 `stage_vehicle_transitions` 不等于运动算法自身。
完整 self/inclusive 排名见 [`summary.json`](evidence-cpu/summary.json)。

| 非内联函数                                               | self 权重 |
| -------------------------------------------------------- | --------: |
| `StepWorkspace::stage_vehicle_transitions`               |    22.91% |
| `StepReadView::advance_active_vehicle_with_waiting_stop` |    16.03% |
| `StepWorkspace::finalize_waiting_outputs`                |    12.92% |
| `OccupancyIndex::leader_gap`                             |     9.24% |
| `si_comfort_travel`                                      |     8.06% |
| `CommittedStateMut::commit`                              |     5.01% |

`advance_active_vehicle_with_waiting_stop` 含子调用共占 **38.65%**，其中已含 leader、
horizon、controller 等，不能再与这些子项相加。
`si_comfort_travel` 含子调用为 8.10%，不是整个运动循环。
`finalize_waiting_outputs` 含子调用为 14.45%；它还处理全量扫描和 transition events，
不能解释为有 14.45% 的车辆正在 Waiting。
内联源码归因中的 `visit_transition_events` 自身约占 5.05%。

PDB 与热点指令还指出几处需要区分调用边界的位置：

| 优化后源码位置 | 10k / 256 权重 | 实际边界                                                    |
| -------------- | -------------: | ----------------------------------------------------------- |
| `tick.rs:846`  |          8.79% | `waiting_stop_for(state)?` 调用点，热点包含参数及返回值搬运 |
| `tick.rs:855`  |          4.35% | 推进返回后 `Option<VehicleState>::ok_or` 与状态展开         |
| `tick.rs:446`  |          5.96% | profile 取值后的值物化/搬运位置，不等于索引查找算法本身     |

RVA `0x3fef00`、`0x3fef96`、`0x3ff45d` 分别对应内存 `movups`/`movq` 指令，
`0x3fb7c8` 对应栈上的 `movupd`。
这是源码/指令热点证据，**不是“复制占用这些精确比例”或 cache miss 根因证明**：
优化后的行映射、采样 skid、相邻指令和调用约定都影响解释。
#217 可据此分别判断状态传递、返回值物化、profile 输入和 controller/advance，
无需预先承诺 SIMD、SoA 或整套接口重做。

## 5. 后继结论与边界

- #216：补充真实路径归因；旧单一重建候选仍无稳定整步收益，不采纳生产实现。
  新数据提示 leader 桶内查找值得单独决策，没有自动授权第二候选或生产改动。
- #217：已有函数、源码位置和调用边界输入；不要把旧运动阶段整体都记作 controller，
  也不能用查询回放的 ns/query 直接相减。
- #219：输出收尾包含全量 transition 扫描，commit 与输出边界分别可见。
  本次不替它实施重构或更新旧 Issue 范围。
- 不外推到 #544 的 100k 城市、Waiting/Conflict 密集场景、日志开启或多边路线。
  本轮未覆盖这些 CPU 输入，没有合并、关闭 Issue 或声称性能优化完成。

## 6. 证据与复现

- [`provenance.json`](evidence-cpu/provenance.json)：构建、源码、哈希、PID、采集时间。
- [`summary.json`](evidence-cpu/summary.json)：样本数、权重、self/inclusive 全排名。
- [`source-summary.json`](evidence-cpu/source-summary.json)：内联源码行与函数归因。
- 三个 `*-stacks.folded`：仅从 `TrafficWorld::step` 起的栈聚合，末列为纳秒权重。
  权重和均校验为对应 `step` 总权重，毫秒权重转换使用十进制整数运算。
- 原始 ETL、逐样本 CSV、所有 RVA 的 PDB 内联记录保留在 provenance 标识的本地
  `target/issue-216-cpu-profile/capture-20260908-193114`。
  ETL 含机器级其他进程事件，**不纳入公开证据目录**。

编译参数（仅在当前终端进程设置）：

```powershell
$env:CARGO_PROFILE_RELEASE_DEBUG = '2'
$env:CARGO_PROFILE_RELEASE_STRIP = 'none'
cargo +1.98.0 test --release --locked --offline -p laneflow-runtime --test runtime_profile_evidence --target-dir target/issue-216-cpu-symbols --no-run
```

用生成的 EXE 分别运行 `cpu_sampling::runtime_cpu_sampling_10k_256`、
`cpu_sampling::runtime_cpu_sampling_10k_16`、`cpu_sampling::runtime_cpu_sampling_1k_256`，
加 `--exact --ignored --nocapture --test-threads=1`；先构建再录制，三个输入串行。
WPR 使用系统 `CPU` profile、`-filemode`、独占 `-instancename`，保存时
`-skipPdbGen -compress`。仅管理员录制器提权，普通权限运行被测 EXE。

[`record-cpu.ps1`](record-cpu.ps1) 只消费四个固定场景的开始/结束标记；
[`capture-cpu-window.ps1`](capture-cpu-window.ps1) 负责普通权限工作负载及摘要；
[`export-cpu.ps1`](export-cpu.ps1) 导出 trace 质量、镜像地址、函数及完整样本栈。
[`cpu-samples.wpaProfile`](cpu-samples.wpaProfile) 保持无分组 raw rows，
避免只导出折叠的进程汇总行。

[`analyze-cpu.py`](analyze-cpu.py) 校验丢事件、PID、raw rows、线程及栈锚点后汇总；
[`symbolize-cpu.py`](symbolize-cpu.py) 由各 trace 的镜像装载地址恢复 RVA，
校验 EXE/PDB 哈希，再展开内联及源码行。分析器拒绝覆盖既有输出目录。
导出机制见 [Microsoft Exporter](https://learn.microsoft.com/en-us/windows-hardware/test/wpt/exporter)。

新增入口已通过三组完整采样、固定 digest 校验及 integration test 的两个默认测试；
`cargo fmt --all --check` 通过。这里只陈述本轮实际运行的验证，不冒充重跑整个 workspace。
