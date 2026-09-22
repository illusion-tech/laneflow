# #707 B1 暖机后 WPR 热点诊断

2026-09-22。Refs #707。用户授权的本切片为采集与诊断；已完成一次短窗口采集、
符号核对、线程活动与调用栈分析。未改 Runtime/Harness 算法，未提交或推送。

**结论：当前整轮耗时首先受主线程串行工作限制。Harness 观测是最大的可明确归属成本；
Core 内仍应优先研究运动约束查询与车辆状态迁移，不宜继续围绕 B1 边界游标做微调。**
本次只定位热点，不证明任何新优化收益，也不替代 [B1/B2 结果](boundary-results.md)
中的平衡复测与交通质量边界。旧版 [L3 诊断](l3-findings.md)的数值不能移植到 B1。

## 1. 采集身份与分析窗口

- 源码：`target/issue707-boundary-source`，基线 `4de40e04` 加研究补丁；79 个源码文件
  按上一轮 provenance 逐项核验，采集后再次核验。补丁 SHA-256：
  `c3a5c2719451380ae3f5941018977464857a5070793ab9d072a7e425c9308b50`。
- 输入：`junction_long_tail-100k` 的既有 input 与 plan，未重新生成。
  `approx + both + combined + boundary=on + decision=every`，4 workers，768 拍。
  无内部 stage 插桩、无 stop-trace；原有逐拍质量观测保留。
- 单独构建：Rust `1.98.0`、release、`--locked --offline`、`entry-frontier`，
  `CARGO_INCREMENTAL=0`、`CARGO_PROFILE_RELEASE_DEBUG=2`。
  EXE SHA-256：`6fd81d45beb297e1858b8cb913c19ea1251139011147842a7c5ccdd46ae1f193`。
  EXE CodeView 与 PDB 的 GUID `dd49130a-fa40-4683-84cc-cbac867c0ea3`、age 1 相符。
  这是带完整符号的诊断构建，不与前次无采样计时混作 A/B。
- Windows build 29671，32 逻辑处理器，平衡电源方案。普通权限 Harness PID 22568；
  提权辅助进程仅执行 WPR。录制前无已有 WPR 会话，录制后已停止。
- 在观察到第 128 拍完成后启动内置 `CPU.Verbose.File`，启动返回后约 45 秒请求停止。
  ETL 为 1,311,768,576 字节，采样周期 1ms；lost events / lost buffers 均为 0。
- ETL 起点为日本时间 **00:59:54.2520924**；统一分析相对 **[5s, 40s)**，即连续
  35 秒，早于 46.546s 开始的 rundown。全 ETL 长 69.729s，包含尾部处理，不能
  当作有效样本窗。WPR stop/merge 另耗约 117.5s，不能当作仿真运行耗时。
- Harness 正常完成 768 拍、退出码 0。129–768 拍的 Active 为 61630–73174；
  这是整个后暖机运行的范围，不是 ETL 窗口的精确逐拍映射。场景仍在演变。

采样 profile 权重共 43.597483 CPU 秒，调用栈共 43,328 hits；两者统计口径有
约 0.62% 差异，分别归一化，不能混用分母。应用模块占加权 CPU 样本的 87.84%，
该模块没有 `Unknown` 函数；其余系统、运行库和驱动模块未下载符号，保留模块级
未细分成本。已知函数名不代表所有内联函数和源码行都独立可见。

首次 stack 导出误用 `SampledProfile`，得到空表；改用 `Profile` 后相同时间范围
正常导出。空表和重试记录保留，报告仅使用 `stacks-profile-5-40.html`。

## 2. 主线程连续满载，辅助线程间歇执行

以下来自 context-switch 活动区间，不是全机任务管理器百分比。

| 线程 | 35 秒内 Running | 相对一个逻辑处理器的占用 |
| --- | ---: | ---: |
| 主线程 16764 | 34.779s | 99.37% |
| 辅助线程 26420 | 2.955s | 8.44% |
| 辅助线程 9712 | 2.959s | 8.46% |
| 辅助线程 5704 | 2.926s | 8.36% |

35 个一秒区间均有目标活动；主线程每秒 Running 最少 985.829ms。
四线程合计 43.619 CPU 秒，平均使用约 **1.246 个逻辑处理器**。4 workers 的
实现本来就包含调用线程与 3 个辅助线程（`execution.rs:277`）。本次的辅助线程
调用栈确实包含 Rayon 的 Motion/候选任务，不能说“没有并行”；但整轮仍有大量
主线程独占工作。仅增加 worker 数不直接消除这些串行成本。

活动导出仅保留全 trace 累计 Running ≥100ms 的线程；这里不宣称不存在更短的
临时线程。主线程窗口内约 0.221s 未运行，不能仅凭这份汇总区分阻塞与 ready 等待。

## 3. Harness：摘要、遍历和重复查询

下面百分比的分母都是**目标进程全部 CPU 调用栈样本**，不是 wall time，也不是
32 核机器总容量。inclusive 包含子调用；不同层级不能相加。

| 观测入口 | Inclusive hits | 占目标 CPU 样本 |
| --- | ---: | ---: |
| `observe::state` | 12889 | 29.75% |
| `observe::counts` | 3369 | 7.78% |
| `observe::events` | 1999 | 4.61% |
| `observe::parking_invariants` | 1349 | 3.11% |
| `observe::red_waiters` | 439 | 1.01% |

五项是 `Harness::advance` 的独立调用，合计 **46.26%**；其中前四项为 45.25%。
这还没有包含整个观测阶段中的拍前快照构造、部分内联记录整理等成本，因此不是
完整 Harness 百分比。源码在 `runner.rs:1315–1384` 确认了这些串行调用。

具体成本与可操作方向：

1. **SHA-256 已是首位独占热点。** `sha256::x86_sha::compress` self 4888 hits，
   11.28%；其 inclusive 4899 hits 中 4867（99.35%）来自 `observe::Hash::flush`。
   现有 `observe.rs:13–51` 已合批 256 字节，下一步应先判断逐拍全量摘要的必要
   频率，不能把再次“小批量合并”当作未做过的优化。
2. **停车绑定被多轮重复读取。** `TrafficWorld::parking_binding` inclusive
   3993 hits，9.22%；调用者为 counts 1932、state 1199、parking_invariants 852、
   execute 10。前三项共 99.75%。它的 HashMap 查询是重要子成本，可考虑在同一
   拍的观测中共享一次读取或合并遍历。
3. **停车绑定 HashMap 自身还服务 Core。** 该 `HashMap::get` self 为 4171 hits，
   9.63%；调用关系同时包含上述 facade 和 `vehicle_motion_outcome`，后者有
   942 inclusive hits。不能把 9.63% 全部记为 Harness，也不能与 facade 再相加。
4. **state 不只生成摘要。** 它也累计行驶距离、速度、停驶等质量信息，检查冲突
   owner，并排序车身区间以检查重叠（`observe.rs:731` 起）。直接跳过整个函数会
   改变质量观测。应先拆开廉价逐拍指标、完整摘要、全量不变量检查，再明确各自
   频率；不能通过删除验收数据来宣称质量未退化。

`state` 下的车身区间排序有 1418 inclusive hits（进程样本 3.27%）。这个成本属于
质量检查，不应与摘要打包后全部称为“可无代价删除”。

## 4. Core：运动约束和状态迁移仍是重点

| Core 函数 | Inclusive | Self | 含义 |
| --- | ---: | ---: | --- |
| `TrafficWorld::step` | 21.69% | 0.00% | 只覆盖主线程调用栈，漏掉辅助线程计算 |
| `MotionTaskView::vehicle_motion_outcome` | 19.08% | 4.26% | 汇合主线程与辅助线程运动样本 |
| `StepWorkspace::stage_vehicle_transitions` | 16.83% | 4.68% | 主线程多阶段入口，包含内联准备/收尾代码 |
| `StepReadView::calculate_active_vehicle_motion` | 13.07% | 2.79% | Motion 的约束、跟车与运动推进 |
| `si_comfort_travel` | 3.27% | 3.11% | SI 运动模型计算 |
| `OccupancyIndex::leader_relation` | 2.32% | 2.31% | 前车关系查询 |
| `ConflictRead::owner_at` | 1.97% | 1.96% | 资源 owner 查询，多调用路径共享 |

**这些行相互重叠，不能求和为 Core 占比。** 尤其不能把 `TrafficWorld::step` 的
21.69% 当作 Core 全部成本；异步线程的调用栈不会包含调用线程的 step 帧。

`calculate_active_vehicle_motion` 的直接子调用包括 `si_comfort_travel` 1416、
`leader_relation` 988、`leader_query_horizon` 765、`gate_policy_decision_with_signals`
538、`hop_permitted` 398 hits。下一轮 Core 原型应关注这些重复约束查询、静态/拍内
共享结果和数据访问，分别计时验证。B2“只复用加速度”已经失败，不能因本次看到
运动热点就把原候选重新描述为有效降频方案。

`stage_vehicle_transitions` 不能直接等同于某个单独阶段：`tick.rs:2633` 起包含
conflict 准备、Motion dispatch、Waiting/Conflict 收尾等；内联会把一些成本归到
这个较大函数。仅凭这次采样不能再精确宣称每个阶段各花多少毫秒。

`boundary` 命名函数的 self 合计仅 282 hits（**0.65%**）。这不是含内联代码的
全部边界成本，但当前证据不支持继续把它列为首要热点。

## 5. 下一切片建议与完成边界

建议先做一个**独立的 Harness 观测成本实验**：保持 B1 交通算法，拆分逐拍指标与
全量摘要/检查频率，合并重复停车绑定和身份读取。用短 ABBA 同时报 iteration、
observation、Core wall time，记录检查降频后的可见性变化，保留完整观测审计臂。
这能缩短后续实验周转，但收益应记到 Harness/整轮，不能算作 Core 算法提速。

Core 的下一条独立研究线仍是运动约束查询；红灯停止线 ETA=0 阻挡绿灯的质量问题
继续按 [B1/B2 结果](boundary-results.md)单独处理。本次没有修复该问题。

本次构建的 129–768 拍混合了有/无 WPR 的时段，其均值 Core 34.556ms、observation
77.068ms、iteration 119.737ms 只作运行背景，**不与旧轮作收益比较**。35 秒采样、
演变中的 Active、系统与记录开销都不支持新的 33ms/p95 或生产认证结论。

## 6. 复核入口

本机证据根：`E:/projects/laneflow/target/issue707-b1-wpr-20260922/`。
这是本地诊断制品目录，不是已提交到仓库的证据包。

- `identity.json`、`capture.json`、`recording-*.json`：源码/输入/二进制身份与采集时刻。
- `b1.etl`、匹配的 `binaries/*.pdb`：原始 trace 与应用符号。
- `trace-stats.txt`、`frequency.txt`、`rundown.txt`：丢失、频率和停止边界。
- `symbols-5-40.txt`、`stacks-profile-5-40.html`、`activity.txt`：原始离线导出。
- `analysis.json`、`hotspots-*.csv`、`caller-callee.csv`、`thread-activity.csv`：归一化结果。
- `export.ps1` 使用相同 [5s,40s) 范围；`analyze.py` 可仅从已有导出重新分析，包含
  PID、非空表、丢失、频率、连续窗口、样本总和与已知独立入口的断言。
- `provenance.json`、`verification.json`：选定制品与外部源码/输入摘要，以及独立复核结果。

无须重新采集即可运行 `python analyze.py`。`run-capture.ps1` 会拒绝覆盖已有 run。
原始 recorder 临时文件、符号缓存与失败预检记录保留；provenance 只索引所需制品，
不把可重建缓存和合并中间 ETL 全部再做一份归档。
