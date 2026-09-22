# #707 L3 同进程早晚窗口离线诊断

2026-09-21。Refs #707。本文件是诊断摘要，不是正式性能认证，不改预算。
未新增仿真运行、编译或 Rust 测试；原 ETL、冻结源码、二进制和计划未修改。

## 1. 来源与有效范围

- 源码：`4de40e045398e4b010b2aa36522afc02a4094c4d`。
- 诊断程序：`target/diag-release/release/laneflow-urban-harness.exe`，SHA-256
  `960fe18ca04396fdcf7ded9455456b27b9434760562f3596b4a68ec704e80b95`。
- 证据目录：`E:/projects/laneflow-evidence/issue-707/4de40e04/diagnostics/wpr/`。
- 同一 PID 11984、workers=4、原始 100k 正式计划，从初态演进。
- 元数据：`l3-w4-meta.txt`；采集：`l3-w4-early.etl`、`l3-w4-late.etl`。
- 实际轮询端点：早窗 tick 66–536；晚窗 tick 21527–22531。端点不是
  ETW 事件与 Core step 的逐拍对齐标记，不能直接作为下面裁剪区间的拍数。
- 两份 ETL 均报告 Lost Buffers=0、Lost Events=0。脚本结束后已确认程序退出。

| 项目 | 早窗 | 晚窗 |
| --- | --- | --- |
| ETL 总时长 | 104.916s | 262.056s |
| WPR rundown 起点（相对 trace） | 81.285s | 236.740s |
| 本次比较采用的区间 | [1s, 81s)，80s | [44s, 236s)，192s |

脚本 `wpr -stop` 返回时才记停止时间，包含合并落盘等待；不能把终端的约
12 分钟除以晚窗拍数作为运行速度。程序在合并期间继续演进，最后才被脚本终止。

晚窗开头约 42 秒没有目标线程 Running 记录；系统级 activityintervals 和 CPU
采样报告也存在开头缺口。因此不能把整份 ETL 时长作为线程占用率分母，更不能
把缺口解释为 LaneFlow 等待。脚本使用 `wpr -start CPU`，未指定 `-filemode`；
内存模式的保留边界是待查解释，尚未独立证明具体缺口原因。零丢失计数并不证明
每种事件覆盖全部请求窗口。本次采用完整活动开始后的 44s，并去掉 rundown。

分析使用本机 Windows Performance Toolkit xperf、匹配的本地 PDB；系统 DLL
有未解析符号。CPU Sampled 用于定位函数采样热点；CSwitch Running 用于线程
执行时间，二者不是阶段墙钟。方法参考
[Microsoft CPU Analysis](https://learn.microsoft.com/en-us/windows-hardware/test/wpt/cpu-analysis)。

## 2. 主线程仍持续执行，辅助线程未持续占满 CPU

下面为裁剪区间内逐秒 Running 时间累加，百分比以一个逻辑处理器为 100%。

| 线程 | 早窗 Running / 区间 | 晚窗 Running / 区间 |
| --- | --- | --- |
| 主线程 11496 | 79.098 / 80s = 98.87% | 188.438 / 192s = 98.14% |
| 辅助 45248 | 7.126 / 80s = 8.91% | 13.296 / 192s = 6.92% |
| 辅助 11648 | 7.003 / 80s = 8.75% | 12.809 / 192s = 6.67% |
| 辅助 2136 | 6.969 / 80s = 8.71% | 13.177 / 192s = 6.86% |

主线程最差完整 1s 桶：早窗 97.53%，晚窗 95.71%。本窗口未见主线程
长期得不到调度，或辅助线程持续占满 CPU 的现象。晚窗主线程非 Running
余量约 1.86%，不足以独自解释很大的整窗退化。

这不证明缓存、内存带宽、频率或短暂等待不存在；Running 也可能包含自旋。
后台 node、桌面合成等有实际 CPU 消耗，不能把本次当作独占机器上的性能认证。
ETL 头部 CPU Speed 字段不是窗口内实测频率。本次没有完整 Ready/Wait 原因
分解、硬件计数器或 w1 同状态对照。

三个辅助线程的占用率以整个 Harness 窗口为分母，其中包括大量命令与观测工作，
不能据此声称 Core 并行比例只有约 20%，也不能直接套 Amdahl 计算 Core 上限。
但它明确支持优先拆解主线程工作和分发边界，而非直接增加 worker 数。

## 3. 函数热点及晚窗新增线索

分母为上述各自裁剪区间内目标进程全部 CPU 采样权重，包含所有线程和模块。
这是函数 IP 的采样分布，受内联归属影响；不是调用树累计时间，也不是 Core p95。

| 符号 | 早窗占比 | 晚窗占比 | 归因边界 |
| --- | --- | --- | --- |
| `sha2::sha256::x86_sha::compress` | 8.64% | 7.32% | Harness 摘要；不能计作 Core |
| `TrafficWorld::parking_binding` | 5.78% | 5.37% | 需区分观测调用与 Runtime 内部 binding 路径 |
| `StepWorkspace::stage_vehicle_transitions` | 5.70% | 5.83% | 涵盖/内联多个子阶段，需插桩拆分 |
| `observe::state` | 4.25% | 4.36% | Harness 状态遍历与摘要 |
| `WorldState::replace_completed_vehicle` | 0.57% | 4.29% | 公共命令路径，晚窗优先新增计时 |
| `MotionTaskView::vehicle_motion_outcome` | 3.97% | 2.98% | 运动计算，百分比下降不等于单次变快 |
| `StepWorkspace::finalize_waiting_outputs` | 1.84% | 2.50% | Waiting 输出整理 |

哈希、冲突 owner/ETA、occupancy leader 查询也持续出现。不能直接把多个
哈希符号相加后归入某个阶段。新增替换热点也不能单独解释 D2 的 w4/w1 增量：
L3 只有 w4，早晚车辆状态和命令组合不同。

源码确认替换命令存在以下成本（冻结源码 `kernel/world.rs`）：

1. 第 1251–1259 行通过 `live_order.iter().position(...)` 定位旧车辆。这发生在
   入口 `overlap_blocker` 检查之前，因此已经 Completed 但入口被占的重试也支付
   该扫描成本。一次最坏访问整个 live 序，不能用 Active 数代替。
2. 第 1411–1412 行成功替换后修改 live 句柄并调用 `rebuild_active_order`；
   后者第 1833 行起扫描整个 live 序，同时使位置索引失效。成功替换是另一条成本。
3. `kernel/active_order.rs` 已有派生 `LiveOrderIndex`；候选应先评估其复用及失效
   成本，不能重复设计一套独立权威。不能把 `position` 简单换成二分查找：live 序
   有稳定更新顺序，替换保留原位置，未证明按句柄排序。
4. Harness `runner.rs:605` 起在旧车辆未 Completed 时调用者直接延期，根本不进
   公共替换 API；必须将这类延期与 Runtime `Blocked` 重试分开计数。

待测最小切片：替换调用/成功/Blocked/调用者延期次数，live 定位总访问量，
定位、输入与入口预检、成功提交及 Active 重建的独立总时间。用包含该命令组合
的定向短用例筛选优化；只跑最初 512 拍不能证明晚窗命令覆盖。
必须保留句柄 generation、live 顺序、错误优先级、失败原子性、容量失败回退与
迁移日志语义。当前证据不足以认定替换函数的 4.29% 全来自线性定位。

## 4. 后续诊断决策

- 不继续补数小时重复测量来再次证明超预算；这次 L3 已提供后段机制证据。
- 先实施[短窗口计时草案](short-profile-plan.md)的分层计时，覆盖 Core、命令、
  Harness 和 writer；加入替换命令分解及 Waiting 输出整理。
- Core 优先测 `stage_vehicle_transitions` 内部的准备、分发、规范消费，以及
  preflight/occupancy 等串行项；不能直接删除检查、摘要或更改随机/排序语义。
- 替换定位与 Active 重建是新增候选；P3 投影计数快路径仍只是待测候选。
- 有明确可消除成本且短对照语义一致后，再选择必要的后段区间及正式认证。

本次没有证明 w4 后段增量的唯一根因，也没有优化收益数值。它将“调度长期
饿死/辅助线程持续空转”的优先级降低，并给出了可落地的命令与阶段诊断边界。

## 5. 本地复核产物

派生报告位于 `E:/projects/laneflow/target/issue707-l3-analysis-20260921/`，属于
Git 忽略的可再生分析目录，不替代证据根的原文件：

- `export.ps1`、`export-ranges.ps1`：本次实际 xperf 导出入口。
- `early/late-stats.txt`、`*-rundown.txt`：完整性和裁剪边界。
- `*-activity.txt`、`*-harness-timeline.csv`：逐秒 Running 明细。
- `*-range-symbols.txt`：表中采用的裁剪区间函数分布。
- `*-symbols.txt`、`*-top-symbols.csv`：整份 trace 分布，只作辅助，不作为本表分母。
- `*-util.txt`、`*-frequency.txt`：采样覆盖与采样周期；不是 CPU 运行频率。

原始 ETL 字节数分别为 1154482176、3331325952。后续 #734 收尾时可将本摘要及
来源索引纳入交付；本轮未 commit、push 或修改 Issue/PR 状态。
